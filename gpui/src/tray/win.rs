//! Windows system tray via the `tray-icon` crate.
//!
//! The tray (icon + menu) is created on a dedicated thread that then pumps
//! Win32 messages (see `crate::win32`) — the tray and menu window procedures
//! run during message dispatch, so the pumping thread is what keeps the icon
//! alive and clickable. Menu clicks arrive on `tray-icon`'s process-global
//! channel; a small poller thread forwards them to the action channel the
//! GPUI main loop polls.

use std::sync::{Arc, Mutex, mpsc};

use tracing::warn;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder};

use super::{SharedToken, TrayAction};

pub(super) fn spawn() -> (mpsc::Receiver<TrayAction>, SharedToken) {
    let (action_tx, action_rx) = mpsc::channel::<TrayAction>();
    // Wayland-only concept — never populated on Windows.
    let token: SharedToken = Arc::new(Mutex::new(None));

    let setup_tx = action_tx.clone();
    let spawn_result = crate::win32::spawn_message_thread("tray-win", move || {
        if let Err(e) = build_tray(setup_tx) {
            warn!("tray: failed to create Windows tray icon: {e}");
        }
    });
    if let Err(e) = spawn_result {
        warn!("tray: failed to spawn tray thread: {e}");
    }

    (action_rx, token)
}

/// Create the tray icon + menu. Identical menu to the Linux ksni backend.
fn build_tray(action_tx: mpsc::Sender<TrayAction>) -> Result<(), String> {
    let icon = Icon::from_rgba(super::icon::rgba(), super::icon::SIZE, super::icon::SIZE)
        .map_err(|e| format!("bad tray icon: {e}"))?;

    let menu = Menu::new();
    let show = MenuItem::with_id("show", "Show Dictionary", true, None);
    let quick_translate = MenuItem::with_id("quick_translate", "Quick Translate", true, None);
    let quit = MenuItem::with_id("quit", "Quit", true, None);
    menu.append_items(&[
        &show,
        &quick_translate,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .map_err(|e| format!("failed to build tray menu: {e}"))?;

    let tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_tooltip("Dicto — Dictionary & quick translate")
        .build()
        .map_err(|e| format!("failed to create tray icon: {e}"))?;

    // Dropping the TrayIcon removes the icon from the notification area.
    // The message loop keeps it alive for the rest of the process lifetime.
    std::mem::forget(tray);

    // Forward menu clicks to the action channel.
    std::thread::Builder::new()
        .name("tray-menu-events".into())
        .spawn(move || {
            let receiver = MenuEvent::receiver();
            while let Ok(event) = receiver.recv() {
                let action = match event.id.0.as_str() {
                    "show" => TrayAction::Show,
                    "quick_translate" => TrayAction::QuickTranslate,
                    "quit" => TrayAction::Quit,
                    _ => continue,
                };
                let _ = action_tx.send(action);
            }
        })
        .ok();

    Ok(())
}
