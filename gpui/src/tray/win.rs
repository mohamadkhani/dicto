//! Windows system tray via the `tray-icon` crate.
//!
//! The tray (icon + menu) lives on a dedicated thread that pumps Win32
//! messages (see `crate::win32`) — the tray and menu window procedures run
//! during message dispatch. `muda` menu items are thread-affine, so menu
//! rebuilds (visibility flips) run on that same thread: a NULL-hwnd timer
//! wakes the pump to service the refresh channel — the same pattern as the
//! hotkey pump in `crate::hotkey::global_hotkey`. Menu clicks arrive on
//! `tray-icon`'s process-global channel; a small poller thread forwards them
//! to the action channel the GPUI main loop polls.

use std::sync::{Arc, Mutex, mpsc};

use tracing::warn;
use tray_icon::menu::accelerator::Accelerator;
use tray_icon::menu::{ContextMenu, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::SetTimer;

use super::{MenuRefresh, SharedToken, SharedVisibility, TrayAction};

pub(super) fn spawn(
    visibility: SharedVisibility,
) -> (mpsc::Receiver<TrayAction>, SharedToken, MenuRefresh) {
    let (action_tx, action_rx) = mpsc::channel::<TrayAction>();
    // Wayland-only concept — never populated on Windows.
    let token: SharedToken = Arc::new(Mutex::new(None));
    // Menu items cannot cross threads, so a refresh only pokes the tray
    // thread's pump (see `run_tray`).
    let (refresh_tx, refresh_rx) = mpsc::channel::<()>();
    let refresh: MenuRefresh = Arc::new(move || {
        let _ = refresh_tx.send(());
    });

    let spawn_result = std::thread::Builder::new()
        .name("tray-win".into())
        .spawn(move || run_tray(action_tx, refresh_rx, visibility));
    if let Err(e) = spawn_result {
        warn!("tray: failed to spawn tray thread: {e}");
    }

    (action_rx, token, refresh)
}

/// Create the tray icon + menu, then pump messages until quit, rebuilding
/// the menu whenever a refresh request arrives.
fn run_tray(
    action_tx: mpsc::Sender<TrayAction>,
    refresh_rx: mpsc::Receiver<()>,
    visibility: SharedVisibility,
) {
    // A NULL-hwnd timer wakes GetMessageW every 250 ms so refresh requests
    // are serviced even while nothing else is dispatched.
    let timer = unsafe { SetTimer(HWND::default(), 0, 250, None) };
    if timer == 0 {
        warn!("tray: SetTimer failed — menu rebuilds wait for other messages");
    }

    // Keep the icon alive: dropping the last `TrayIcon` handle removes it
    // from the notification area, so it lives here for the process lifetime.
    let tray = match build_tray(action_tx, &visibility) {
        Ok(tray) => tray,
        Err(e) => {
            warn!("tray: failed to create Windows tray icon: {e}");
            return;
        }
    };

    loop {
        if !crate::win32::wait_and_dispatch_one() {
            break;
        }
        // Drain first: a burst of requests rebuilds the menu once.
        let mut rebuild = false;
        while refresh_rx.try_recv().is_ok() {
            rebuild = true;
        }
        if rebuild {
            match build_menu(&visibility) {
                Ok(menu) => {
                    let menu: Box<dyn ContextMenu> = Box::new(menu);
                    tray.set_menu(Some(menu));
                }
                Err(e) => warn!("tray: failed to rebuild tray menu: {e}"),
            }
        }
    }
}

/// Create the tray icon + menu + click forwarder. Identical menu to the
/// Linux ksni backend.
fn build_tray(
    action_tx: mpsc::Sender<TrayAction>,
    visibility: &SharedVisibility,
) -> Result<TrayIcon, String> {
    let icon = Icon::from_rgba(super::icon::rgba(), super::icon::SIZE, super::icon::SIZE)
        .map_err(|e| format!("bad tray icon: {e}"))?;

    let menu = build_menu(visibility)?;

    let tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_tooltip("Dicto — Dictionary & quick translate")
        .build()
        .map_err(|e| format!("failed to create tray icon: {e}"))?;

    // Forward menu clicks to the action channel.
    std::thread::Builder::new()
        .name("tray-menu-events".into())
        .spawn(move || {
            let receiver = MenuEvent::receiver();
            while let Ok(event) = receiver.recv() {
                let action = match event.id.0.as_str() {
                    "show" => TrayAction::Show { token: None },
                    "quick_translate" => TrayAction::QuickTranslate,
                    "quick_lookup" => TrayAction::QuickLookup,
                    "quit" => TrayAction::Quit,
                    _ => continue,
                };
                let _ = action_tx.send(action);
            }
        })
        .ok();

    Ok(tray)
}

/// Build the menu from scratch — the quick-translate / lookup items appear
/// only while their feature is enabled (see `sync_menu_visibility`), and
/// show the feature's configured hotkey as the menu-item accelerator
/// (Windows renders it right-aligned in the menu; the hotkey backend is
/// what actually fires the action).
fn build_menu(visibility: &SharedVisibility) -> Result<Menu, String> {
    let show = MenuItem::with_id("show", "Show Dictionary", true, None);
    let quick_translate = MenuItem::with_id(
        "quick_translate",
        "Quick Translate",
        true,
        parse_accel(&visibility.translate_hotkey()),
    );
    let quick_lookup = MenuItem::with_id(
        "quick_lookup",
        "Look Up Word",
        true,
        parse_accel(&visibility.lookup_hotkey()),
    );
    let quit = MenuItem::with_id("quit", "Quit", true, None);

    let mut items: Vec<&dyn IsMenuItem> = vec![&show];
    if visibility.quick_translate() {
        items.push(&quick_translate);
    }
    if visibility.quick_lookup() {
        items.push(&quick_lookup);
    }
    let separator = PredefinedMenuItem::separator();
    items.push(&separator);
    items.push(&quit);

    let menu = Menu::new();
    menu.append_items(&items)
        .map_err(|e| format!("failed to build tray menu: {e}"))?;
    Ok(menu)
}

/// Parse a `"Ctrl+Alt+D"`-style hotkey into a muda accelerator. muda's
/// parser accepts the same `Mod+Mod+Key` syntax the settings store, so this
/// only has to fold parse failures (unset/unsupported hotkeys) into `None`.
fn parse_accel(hotkey: &str) -> Option<Accelerator> {
    if hotkey.trim().is_empty() {
        return None;
    }
    match hotkey.parse::<Accelerator>() {
        Ok(accel) => Some(accel),
        Err(e) => {
            warn!("tray: cannot display hotkey '{hotkey}' as accelerator: {e}");
            None
        }
    }
}
