#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod audio;
mod bidi;
mod catalog;
mod colors;
mod components;
mod download;
mod hotkey;
mod html;
mod indexing;
mod playback;
mod quick_translate;
mod selection;
mod state;
mod tray;
mod tts;
#[cfg(target_os = "windows")]
mod win32;

use std::borrow::Cow;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use gpui::{
    App, AppContext as _, AssetSource, Bounds, QuitMode, SharedString, WindowBounds,
    WindowDecorations, WindowOptions, px, size,
};
use gpui_component::{Root, Theme, ThemeMode};
use gpui_platform::application;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::app::DictApp;
use crate::state::DictState;
use crate::tray::{TrayAction, spawn_tray};

/// Global flag set by the tray menu "Quick Translate" item.
/// The main app loop polls this and triggers translation when set.
static TRAY_TRANSLATE_TRIGGERED: AtomicBool = AtomicBool::new(false);

/// The compositor-minted xdg-activation token captured by the tray (SNI
/// `ProvideXdgActivationToken`) immediately before a "Quick Translate" click.
/// Forwarded to the popup window's `activate_with_token` so GNOME/Mutter
/// authoritatively raises it instead of falling back to demand-attention.
/// Same pattern as LogiGuard (see apps/gpui/src/tray.rs).
static TRAY_TRANSLATE_TOKEN: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
    std::sync::OnceLock::new();

fn tray_translate_token() -> &'static std::sync::Mutex<Option<String>> {
    TRAY_TRANSLATE_TOKEN.get_or_init(|| std::sync::Mutex::new(None))
}

/// Stash the latest tray activation token for the next popup open.
pub fn set_tray_translate_token(token: Option<String>) {
    if let Ok(mut g) = tray_translate_token().lock() {
        *g = token;
    }
}

/// Drain the stashed tray activation token (returns it, leaving None behind).
pub fn take_tray_translate_token() -> Option<String> {
    tray_translate_token()
        .lock()
        .ok()
        .and_then(|mut g| g.take())
}

/// Path to the IPC socket used by `dicto --translate` to signal a running
/// instance. Lives in the user's runtime directory.
#[cfg(target_os = "linux")]
fn ipc_socket_path() -> std::path::PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"));
    base.join("dicto-translate.sock")
}

/// Send a translate trigger to the running instance via the IPC socket.
/// Returns an error if no instance is listening.
#[cfg(target_os = "linux")]
fn send_translate_trigger() -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let path = ipc_socket_path();
    let mut stream = UnixStream::connect(&path)?;
    stream.write_all(b"translate\n")?;
    Ok(())
}

/// Spawn the IPC server that listens for `dicto --translate` triggers.
/// Runs in a background thread; sets the global flag on each trigger.
#[cfg(target_os = "linux")]
fn spawn_ipc_server() {
    use std::os::unix::net::UnixListener;

    let path = ipc_socket_path();
    let _ = std::fs::remove_file(&path); // clear stale socket

    let listener = match UnixListener::bind(&path) {
        Ok(l) => {
            tracing::info!("ipc: listening on {}", path.display());
            l
        }
        Err(e) => {
            tracing::warn!("ipc: failed to bind socket at {}: {e}", path.display());
            return;
        }
    };

    std::thread::Builder::new()
        .name("dicto-ipc".into())
        .spawn(move || {
            use std::io::Read;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 32];
                if stream.read(&mut buf).unwrap_or(0) > 0 {
                    TRAY_TRANSLATE_TRIGGERED.store(true, Ordering::Release);
                }
            }
        })
        .ok();
}

struct AppAssets;

const WINDOW_CLOSE_SVG: &[u8] = br##"<svg width="24" height="24" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path fill="#000" d="M6.7 5.3 12 10.6l5.3-5.3 1.4 1.4-5.3 5.3 5.3 5.3-1.4 1.4-5.3-5.3-5.3 5.3-1.4-1.4 5.3-5.3-5.3-5.3z"/></svg>"##;
const WINDOW_MAXIMIZE_SVG: &[u8] = br##"<svg width="24" height="24" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path fill="#000" d="M5 5h14v14H5zm2 2v10h10V7z"/></svg>"##;
const WINDOW_MINIMIZE_SVG: &[u8] = br##"<svg width="24" height="24" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path fill="#000" d="M5 11h14v2H5z"/></svg>"##;
const WINDOW_RESTORE_SVG: &[u8] = br##"<svg width="24" height="24" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path fill="#000" d="M8 5h11v11h-2V7H8z"/><path fill="#000" d="M5 8h11v11H5zm2 2v7h7v-7z"/></svg>"##;
const PENCIL_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/></svg>"##;
const APP_ICON_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128"><defs><linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stop-color="#7aa2f7"/><stop offset="100%" stop-color="#414868"/></linearGradient><linearGradient id="card" x1="0%" y1="0%" x2="0%" y2="100%"><stop offset="0%" stop-color="#fafbff"/><stop offset="100%" stop-color="#dde3ff"/></linearGradient></defs><rect width="128" height="128" rx="28" fill="url(#bg)"/><rect x="34" y="16" width="68" height="84" rx="6" fill="#7aa2f7" opacity=".35"/><rect x="30" y="20" width="70" height="84" rx="6" fill="#7aa2f7" opacity=".55"/><rect x="26" y="24" width="72" height="84" rx="6" fill="url(#card)"/><path d="M 38 84 L 52 38 L 62 38 L 76 84 L 67 84 L 64 72 L 50 72 L 47 84 Z M 52 64 L 62 64 L 57 48 Z" fill="#1a1b26" fill-rule="evenodd"/><line x1="38" y1="94" x2="86" y2="94" stroke="#7aa2f7" stroke-width="3" stroke-linecap="round"/><line x1="38" y1="102" x2="70" y2="102" stroke="#7aa2f7" stroke-width="3" stroke-linecap="round" opacity=".55"/></svg>"##;
impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        eprintln!("[assets] load: {path}");
        let local = match path {
            "icons/window-close.svg" => Some(WINDOW_CLOSE_SVG),
            "icons/window-maximize.svg" => Some(WINDOW_MAXIMIZE_SVG),
            "icons/window-minimize.svg" => Some(WINDOW_MINIMIZE_SVG),
            "icons/window-restore.svg" => Some(WINDOW_RESTORE_SVG),
            "icons/app-icon.svg" => Some(APP_ICON_SVG),
            "icons/pencil.svg" => Some(PENCIL_SVG),
            _ => None,
        };

        if let Some(bytes) = local {
            return Ok(Some(Cow::Borrowed(bytes)));
        }

        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let mut assets = gpui_component_assets::Assets.list(path)?;
        for extra in [
            "icons/window-close.svg",
            "icons/window-maximize.svg",
            "icons/window-minimize.svg",
            "icons/window-restore.svg",
            "icons/app-icon.svg",
            "icons/pencil.svg",
        ] {
            if extra.starts_with(path) && !assets.iter().any(|item| item.as_ref() == extra) {
                assets.push(extra.into());
            }
        }
        Ok(assets)
    }
}

fn main() {
    // Handle the `--translate` CLI flag FIRST, before any GUI init: a second
    // invocation with this flag signals the already-running instance to
    // trigger quick translate. This is the GNOME Wayland workaround for
    // global hotkeys — the user binds a custom keyboard shortcut to
    // `dicto --translate` in GNOME Settings → Keyboard → Custom Shortcuts.
    if std::env::args().any(|a| a == "--translate" || a == "-t") {
        #[cfg(target_os = "linux")]
        {
            match send_translate_trigger() {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    eprintln!(
                        "dicto: could not reach a running instance.\n\
                         Start Dicto first, then press the shortcut.\n\
                         Error: {e}"
                    );
                    std::process::exit(1);
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            eprintln!("dicto: --translate IPC is only supported on Linux");
            std::process::exit(1);
        }
    }

    #[cfg(target_os = "linux")]
    gtk::init().expect("failed to init GTK");

    // symphonia (rodio's underlying demuxer) prints a WARN for every
    // byte it can't make sense of when handed a non-mp3 stream — for
    // Speex clips that's hundreds of lines per click. Silence its
    // crates here; the audio module logs a single line on failure.
    //
    // arboard logs a WARN on every clipboard read under GNOME/KDE
    // Wayland ("wayland data control not supported, falling back to
    // X11"). The fallback is expected and harmless there — the X11
    // read succeeds — so keep it out of the default log; RUST_LOG can
    // still surface it.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "info,symphonia_bundle_mp3=error,symphonia_core=error,symphonia_format_ogg=error,arboard=error",
        )
    });
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load any indexes that already exist so the UI is usable immediately
    // for cached dictionaries. New/unindexed dicts are built in the background
    // (see `indexing::spawn`) after the window opens.
    mdict_rs::registry::reload();
    indexing::load_stylesheets();

    // Prepare telemetry consent + installation id *before* entering the GPUI
    // runtime. We want to fail these only to warnings, not block startup.
    let install_id = match mdict_rs::settings::ensure_installation_id() {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!("telemetry: failed to ensure installation id: {e}");
            String::new()
        }
    };
    let settings = mdict_rs::settings::current();
    let opted_in = matches!(
        settings.telemetry_consent,
        mdict_rs::settings::TelemetryConsent::OptedIn
    );
    let app_version = env!("APP_VERSION").to_string();

    // Initialize telemetry once before GPUI runs. Opted-out users get NullTelemetry.
    dicto_telemetry::init(opted_in, install_id.clone(), app_version);
    if opted_in {
        dicto_telemetry::get().track(dicto_telemetry::Event::AppStarted);
    }

    let app = application();
    app.with_assets(AppAssets)
        // The tray must survive closing the dictionary window. Default
        // QuitMode quits the app when the last window closes, which would tear
        // down the tray. Quit only on the explicit "Quit" tray action.
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);

            // Start the IPC server so `dicto --translate` (e.g. from a GNOME
            // custom keyboard shortcut) can trigger quick translate.
            #[cfg(target_os = "linux")]
            spawn_ipc_server();

            // Spawn the system tray; poll its action channel from the main
            // loop.
            {
                let (tray_rx, tray_token) = spawn_tray();
                poll_tray_actions(cx, tray_rx, tray_token);
            }

            open_dictionary_window(cx);

            cx.activate(true);
        });
}

/// Poll the tray action channel from a GPUI background task.
///
/// - `Show` → open (or re-activate) the dictionary window.
/// - `QuickTranslate` → set the same flag the `dicto --translate` IPC path
///   uses; the `app.rs` poll loop picks it up and runs `trigger_translate`.
///   On Linux this also stashes the tray's xdg-activation token so the popup
///   can raise+focus (the token slot stays `None` on Windows).
/// - `Quit` → quit the app.
fn poll_tray_actions(
    cx: &mut App,
    tray_rx: std::sync::mpsc::Receiver<TrayAction>,
    tray_token: crate::tray::SharedToken,
) {
    cx.spawn(async move |cx| {
        loop {
            while let Ok(action) = tray_rx.try_recv() {
                match action {
                    TrayAction::Show => {
                        let _ = cx.update(|cx| {
                            if cx.windows().is_empty() {
                                open_dictionary_window(cx);
                            } else {
                                cx.activate(true);
                            }
                        });
                    }
                    TrayAction::QuickTranslate => {
                        // Forward the compositor-minted activation token (if
                        // the host supports ProvideXdgActivationToken) so the
                        // popup raises+focuses on GNOME/Mutter. Then set the
                        // trigger flag the DictApp poll loop drains.
                        let token = tray_token.lock().ok().and_then(|mut g| g.take());
                        set_tray_translate_token(token);
                        TRAY_TRANSLATE_TRIGGERED.store(true, Ordering::Release);
                    }
                    TrayAction::Quit => {
                        let _ = cx.update(|cx| cx.quit());
                    }
                }
            }

            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
        }
    })
    .detach();
}

fn open_dictionary_window(cx: &mut App) {
    dicto_telemetry::get().track(dicto_telemetry::Event::WindowOpened);
    let bounds = Bounds::centered(None, size(px(920.), px(680.)), cx);

    let state_for_indexing: std::cell::RefCell<Option<gpui::Entity<DictState>>> =
        std::cell::RefCell::new(None);

    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_decorations: Some(WindowDecorations::Client),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Dicto".into()),
                appears_transparent: cfg!(target_os = "windows"),
                ..Default::default()
            }),
            window_min_size: Some(size(px(600.), px(400.))),
            is_resizable: true,
            app_id: Some("dicto".into()),
            ..Default::default()
        },
        |window, cx| {
            let state = cx.new(|_cx| DictState::new());
            *state_for_indexing.borrow_mut() = Some(state.clone());
            let view = cx.new(|cx| DictApp::new(state, window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        },
    )
    .expect("failed to open window");

    if let Some(state) = state_for_indexing.into_inner() {
        // Keep popup state in sync when the popup window is destroyed by a
        // path that bypasses `close_popup` (window manager close, Alt+F4,
        // compositor kill). Without this the stale handle lingers in
        // `qt_popup_window` and later hotkey/tray triggers silently do
        // nothing.
        let closed_state = state.clone();
        cx.on_window_closed(move |cx, closed_id| {
            let popup_id = closed_state
                .read(cx)
                .qt_popup_window
                .map(|handle| handle.window_id());
            if popup_id == Some(closed_id) {
                closed_state.update(cx, |s, cx| {
                    s.qt_popup_window = None;
                    // A tokenless re-trigger closes the window only to
                    // reopen a fresh one on the next poll tick — the popup
                    // state (and its new selection) must survive.
                    let replace_pending = s.qt_replace_pending;
                    if let Some(engine) = s.quick_translate_engine.as_mut() {
                        if !replace_pending {
                            engine.hide_popup();
                        }
                    }
                    // The popup window is gone (e.g. closed by the window
                    // manager) — stop speaking the source/translation
                    // clips rather than leaving orphaned audio playing.
                    s.playback_source.stop();
                    s.playback_translation.stop();
                    cx.notify();
                });
            }
        })
        .detach();

        indexing::spawn(state, cx);
    }
}
