#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod alsa_default;
mod app;
mod audio;
mod autostart;
mod bidi;
mod catalog;
mod colors;
mod components;
mod download;
mod hotkey;
mod html;
mod indexing;
mod karaoke;
mod playback;
mod quick_translate;
mod selection;
mod state;
mod tray;
mod tts;
#[cfg(target_os = "windows")]
mod win32;
mod window_move;
mod word_lookup;

use std::borrow::Cow;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use gpui::{
    App, AppContext as _, AssetSource, AsyncApp, Bounds, Entity, QuitMode, SharedString,
    WindowBounds, WindowDecorations, WindowOptions, px, size,
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

/// Global flag set by the tray menu "Look Up Word" item.
/// The main app loop polls this and triggers a word lookup when set.
static TRAY_LOOKUP_TRIGGERED: AtomicBool = AtomicBool::new(false);

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

/// Compositor-minted xdg-activation token forwarded by a `dicto --lookup` /
/// `dicto --translate` IPC client. GNOME custom keyboard shortcuts launch
/// the command WITH `XDG_ACTIVATION_TOKEN` in the environment (Mutter mints
/// one per keybinding press); the short-lived CLI client forwards it over
/// the socket so the MAIN instance can raise the popup through the
/// activation protocol's own grant. It is the deterministic raise path;
/// without it the popup relies on the window-calls extension's `Activate`,
/// which Mutter may decline under focus-stealing prevention.
static IPC_ACTIVATION_TOKEN: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
    std::sync::OnceLock::new();

fn ipc_activation_token() -> &'static std::sync::Mutex<Option<String>> {
    IPC_ACTIVATION_TOKEN.get_or_init(|| std::sync::Mutex::new(None))
}

/// Stash the activation token sent by an IPC client.
pub fn set_ipc_activation_token(token: Option<String>) {
    if let Ok(mut g) = ipc_activation_token().lock() {
        *g = token;
    }
}

/// Take the stashed IPC activation token (last writer wins, one-shot).
pub fn take_ipc_activation_token() -> Option<String> {
    ipc_activation_token()
        .lock()
        .ok()
        .and_then(|mut g| g.take())
}

/// Global flag set by the `dicto-main.sock` "show" IPC trigger: a second
/// plain launch asks this instance to show + raise the dictionary window.
static IPC_SHOW_TRIGGERED: AtomicBool = AtomicBool::new(false);

/// Activation token forwarded by the "show" IPC client. Separate slot from
/// `IPC_ACTIVATION_TOKEN` so a window activation can never clobber the
/// translate/lookup popup's token (and vice versa).
static IPC_SHOW_TOKEN: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
    std::sync::OnceLock::new();

/// Stash the activation token sent by the "show" IPC client.
fn set_ipc_show_token(token: Option<String>) {
    if let Ok(mut g) = IPC_SHOW_TOKEN
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
    {
        *g = token;
    }
}

/// Take the stashed "show" token (last writer wins, one-shot).
fn take_ipc_show_token() -> Option<String> {
    IPC_SHOW_TOKEN
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .ok()
        .and_then(|mut g| g.take())
}

/// The dictionary (main) window. Tray "Show Dictionary" raises+focuses THIS
/// window (raising whatever window happens to exist would hand keyboard
/// focus to a popup) and reopens it after the user closes it — the app
/// stays alive in the tray (`QuitMode::Explicit`).
static DICT_WINDOW: std::sync::Mutex<Option<gpui::AnyWindowHandle>> = std::sync::Mutex::new(None);

/// Path to an IPC socket used by `dicto --translate` / `dicto --lookup` to
/// signal a running instance. Lives in the user's runtime directory.
#[cfg(target_os = "linux")]
fn ipc_socket_path(name: &str) -> std::path::PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"));
    base.join(name)
}

/// Send a trigger to the running instance via the named IPC socket.
/// `trigger` is the first line ("trigger" for the translate/lookup popups,
/// "show" for the main-window activation socket). Returns a human-readable
/// error when no live instance is listening — including the stale-socket
/// case (a previous instance died and left the socket file behind;
/// connecting to it fails with ECONNREFUSED).
#[cfg(target_os = "linux")]
fn send_ipc_trigger(socket_name: &str, trigger: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let path = ipc_socket_path(socket_name);
    match UnixStream::connect(&path) {
        Ok(mut stream) => {
            // Line 1: the trigger. Line 2 (optional): the compositor-minted
            // activation token from our environment — present when GNOME
            // launched us from a keyboard shortcut. Old servers ignore the
            // extra line; new servers use it to raise the popup reliably.
            let mut msg = format!("{trigger}\n");
            if let Ok(token) = std::env::var("XDG_ACTIVATION_TOKEN")
                && !token.is_empty()
            {
                msg.push_str("token ");
                msg.push_str(&token);
                msg.push('\n');
            }
            stream.write_all(msg.as_bytes()).map_err(|e| e.to_string())
        }
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => Err(format!(
            "stale IPC socket at {} — a previous Dicto instance exited \
             without cleanup. Quit and restart Dicto to fix it. ({e})",
            path.display()
        )),
        Err(e) => Err(e.to_string()),
    }
}

/// Connect-only probe: `true` when a live instance is listening on the named
/// IPC socket. Writes nothing, so the server never treats it as a trigger.
#[cfg(target_os = "linux")]
fn ipc_instance_running(socket_name: &str) -> bool {
    use std::os::unix::net::UnixStream;

    UnixStream::connect(ipc_socket_path(socket_name)).is_ok()
}

/// Win32 single-instance guard: creates a named mutex kept owned for the
/// process lifetime. Returns `true` when another instance already owns it.
#[cfg(target_os = "windows")]
fn win32_already_running() -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::w;

    let mutex = unsafe { CreateMutexW(None, false, w!("Local\\Dicto")) };
    // Must be read immediately — any later Win32 call overwrites it.
    let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    match mutex {
        // Leaking the handle keeps the mutex owned; a closed handle would
        // release it and let a third instance in.
        Ok(handle) => std::mem::forget(handle),
        Err(e) => tracing::warn!("single-instance mutex: {e}"),
    }
    already
}

/// Best-effort raise of the already-running instance's main window (second
/// plain launch on Windows). Finds the window by its "Dicto" title and
/// restores + focuses it. Limitation: a different app's window titled
/// "Dicto" would match instead; nothing better is available cross-process
/// without extra plumbing (a hidden broadcast window).
#[cfg(target_os = "windows")]
fn show_existing_window() {
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    use windows::core::{PCWSTR, w};

    unsafe {
        if let Ok(hwnd) = FindWindowW(PCWSTR::null(), w!("Dicto")) {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// Spawn an IPC server that listens for trigger messages on the named
/// socket and sets the given global flag on each one. Runs in a background
/// thread; the poll loop drains the flag. `stash_token` receives the
/// activation token forwarded on line 2 (when present).
#[cfg(target_os = "linux")]
fn spawn_ipc_server(
    socket_name: &'static str,
    flag: &'static AtomicBool,
    stash_token: fn(Option<String>),
    thread_name: &'static str,
) {
    use std::os::unix::net::UnixListener;

    let path = ipc_socket_path(socket_name);
    let _ = std::fs::remove_file(&path); // clear stale socket

    let listener = match UnixListener::bind(&path) {
        Ok(l) => {
            tracing::info!(socket = %path.display(), "ipc: listening");
            l
        }
        Err(e) => {
            tracing::warn!("ipc: failed to bind socket at {}: {e}", path.display());
            return;
        }
    };

    std::thread::Builder::new()
        .name(thread_name.into())
        .spawn(move || {
            use std::io::Read;
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                // The client writes its lines and drops the connection, so
                // read to EOF (bounded) and parse. Any content is a trigger.
                let mut msg = String::new();
                if stream.take(4096).read_to_string(&mut msg).unwrap_or(0) > 0 {
                    if let Some(token) = msg
                        .lines()
                        .find_map(|l| l.strip_prefix("token "))
                        .map(str::to_string)
                        && !token.is_empty()
                    {
                        stash_token(Some(token));
                    }
                    flag.store(true, Ordering::Release);
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
    // Handle the `--translate` / `--lookup` CLI flags FIRST, before any GUI
    // init: a second invocation with one of these flags signals the
    // already-running instance. This is the GNOME Wayland path for global
    // hotkeys: the app registers GNOME custom shortcuts bound to
    // `dicto --translate` / `dicto --lookup` by itself (see
    // `hotkey::os_binding`), and each keybinding press launches one of
    // these short-lived clients, which forward the trigger (plus the
    // compositor-minted activation token) to the running instance over IPC.
    let trigger_flag = std::env::args().find_map(|a| match a.as_str() {
        "--translate" | "-t" => Some(("dicto-translate.sock", "dicto: --translate")),
        "--lookup" | "-l" => Some(("dicto-lookup.sock", "dicto: --lookup")),
        _ => None,
    });
    if let Some((socket, label)) = trigger_flag {
        #[cfg(target_os = "linux")]
        {
            match send_ipc_trigger(socket, "trigger") {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    eprintln!(
                        "{label}: could not reach a running instance.\n\
                         Start (or restart) Dicto first, then press the shortcut.\n\
                         Error: {e}"
                    );
                    std::process::exit(1);
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (socket, label);
            eprintln!("dicto: --translate/--lookup IPC is only supported on Linux");
            std::process::exit(1);
        }
    }

    // Start hidden in the tray (no dictionary window) when `--tray` is set —
    // the autostart entry uses this. First-run surfaces still win: the setup
    // guard below ignores the flag.
    let tray_requested = std::env::args().any(|a| a == "--tray");

    // Single-instance guard: a second full launch must NOT steal the IPC
    // sockets (and with them the global hotkeys) from the running instance.
    #[cfg(target_os = "linux")]
    {
        if tray_requested {
            // A second `--tray` launch (autostart racing a live session):
            // signal nothing, exit silently.
            if ipc_instance_running("dicto-main.sock") {
                std::process::exit(0);
            }
        } else {
            // A second plain launch shows + raises the running instance's
            // dictionary window, then exits. Falls through on error (no
            // instance, or a stale socket file) — this process becomes the
            // main instance and `spawn_ipc_server` clears the stale file.
            if send_ipc_trigger("dicto-main.sock", "show").is_ok() {
                std::process::exit(0);
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        if win32_already_running() {
            if !tray_requested {
                show_existing_window();
            }
            std::process::exit(0);
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let _ = tray_requested;

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

    // Must run before the first audio device open (see module docs): makes
    // ALSA's `default` PCM follow the desktop's default output device even
    // on systems whose ALSA plugin configs are missing from the load path.
    #[cfg(target_os = "linux")]
    alsa_default::ensure_server_routed_default();

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

            // Start the IPC servers so `dicto --translate` / `dicto --lookup`
            // (e.g. from a GNOME custom keyboard shortcut) can trigger the
            // popups in the running instance, and a second plain launch can
            // ask us to show + raise the dictionary window.
            #[cfg(target_os = "linux")]
            {
                spawn_ipc_server(
                    "dicto-translate.sock",
                    &TRAY_TRANSLATE_TRIGGERED,
                    set_ipc_activation_token,
                    "dicto-ipc-translate",
                );
                spawn_ipc_server(
                    "dicto-lookup.sock",
                    &TRAY_LOOKUP_TRIGGERED,
                    set_ipc_activation_token,
                    "dicto-ipc-lookup",
                );
                spawn_ipc_server(
                    "dicto-main.sock",
                    &IPC_SHOW_TRIGGERED,
                    set_ipc_show_token,
                    "dicto-ipc-main",
                );
            }

            // Spawn the system tray; poll its action channel from the main
            // loop.
            {
                let (tray_rx, tray_token) = spawn_tray();
                poll_tray_actions(cx, tray_rx, tray_token);
            }

            // With `--tray` (autostart) start hidden in the tray: no
            // dictionary window, no activation — but the tray, the global
            // hotkeys, and the popup triggers must still come up, so the
            // app machinery starts headless. First-run setup wins over
            // hiding: the telemetry consent prompt and the
            // empty-dictionaries import modal are blocking surfaces that
            // need the window.
            let setup_needed = matches!(
                mdict_rs::settings::current().telemetry_consent,
                mdict_rs::settings::TelemetryConsent::Undecided
            ) || mdict_rs::config::discover_mdx_files().is_empty();
            if tray_requested && !setup_needed {
                start_background_machinery(cx);
            } else {
                open_dictionary_window(cx);
                cx.activate(true);
            }
        });
}

/// Poll the tray action channel from a GPUI background task. Also drains the
/// `dicto-main.sock` "show" IPC flag: a second plain launch asks this
/// instance to raise the dictionary window.
///
/// - IPC "show" → raise + focus (or reopen) the dictionary window with the
///   activation token the second launch forwarded.
/// - `Show` → raise + focus (or reopen) the dictionary window with the
///   tray's xdg-activation token.
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
            // A second plain launch asked us to show + raise the dictionary
            // window ("show" trigger on dicto-main.sock).
            if IPC_SHOW_TRIGGERED.swap(false, Ordering::Acquire) {
                let token = take_ipc_show_token();
                cx.update(|cx| {
                    raise_dictionary_window(token, cx);
                });
            }
            while let Ok(action) = tray_rx.try_recv() {
                match action {
                    TrayAction::Show { token } => {
                        cx.update(|cx| {
                            raise_dictionary_window(token, cx);
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
                    TrayAction::QuickLookup => {
                        let token = tray_token.lock().ok().and_then(|mut g| g.take());
                        set_tray_translate_token(token);
                        TRAY_LOOKUP_TRIGGERED.store(true, Ordering::Release);
                    }
                    TrayAction::Quit => {
                        cx.update(|cx| cx.quit());
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

/// Raise + keyboard-focus the dictionary window; open a fresh one when it
/// has been closed (the app keeps running in the tray).
///
/// The tray captures the compositor-minted xdg-activation token
/// (`ProvideXdgActivationToken`) at click time; feeding it to
/// `activate_with_token` (mohamadkhani/zed gpui fork) makes Mutter
/// authoritatively grant the raise instead of leaving the window behind the
/// foreground app under focus-stealing prevention. Same pattern as netkeep's
/// "Settings…" tray item (apps/gpui/src/tray.rs). Without a token (Windows,
/// hosts without the SNI method) `activate_window` + `cx.activate` still do
/// the platform-level raise.
fn raise_dictionary_window(token: Option<String>, cx: &mut App) {
    let handle = DICT_WINDOW.lock().ok().and_then(|g| *g);
    let raised = handle
        .and_then(|handle| {
            handle
                .update(cx, |_, window, _cx| {
                    window.activate_window();
                    if let Some(tok) = token.as_deref() {
                        window.activate_with_token(tok);
                    }
                })
                .ok()
        })
        .is_some();
    if raised {
        cx.activate(true);
    } else {
        // `update` failed: the window was closed (or never opened). A fresh
        // map gets keyboard focus on GNOME/Mutter on its own (see the
        // tokenless-retrigger note in `app.rs`).
        open_dictionary_window(cx);
    }
}

/// Attach hotkey managers built off the UI thread to the state's
/// engines. The XDG portal backend blocks up to `BIND_SETTLE_TIMEOUT`
/// (3s) waiting for its first bind to settle, so callers run this after
/// the critical startup work (see the original inline comments below).
fn attach_hotkey_managers(state: &Entity<DictState>, cx: &mut App) {
    // Attach hotkey managers built OFF the UI thread: the XDG portal
    // backend blocks up to `BIND_SETTLE_TIMEOUT` (3s) waiting for its
    // first bind to settle. Doing that inside `DictState::new` delayed
    // the first frame by ~3s on every startup (GNOME/Wayland). The
    // window shows immediately; hotkeys land moments later.
    {
        let (wl, qt, attempt) = {
            let s = state.read(cx);
            (
                s.word_lookup.clone(),
                s.quick_translate.clone(),
                s.word_lookup_toggle_gen,
            )
        };
        let state_bg = state.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let wl_build = wl.clone();
            let qt_build = qt.clone();
            let (wl_mgr, qt_mgr) = cx
                .background_executor()
                .spawn(async move {
                    let wl_mgr = if wl_build.enabled {
                        crate::word_lookup::make_hotkey_manager(wl_build)
                    } else {
                        None
                    };
                    let qt_mgr = if qt_build.enabled {
                        crate::quick_translate::make_hotkey_manager(qt_build)
                    } else {
                        None
                    };
                    (wl_mgr, qt_mgr)
                })
                .await;

            cx.update(|cx| {
                cx.update_entity(&state_bg, |s, _cx| {
                    // A settings change or toggle during registration
                    // makes the manager stale — drop it instead of
                    // attaching (same guard as the toggle flow).
                    if let Some(mgr) = wl_mgr
                        && s.word_lookup_toggle_gen == attempt
                        && s.word_lookup.enabled == wl.enabled
                        && s.word_lookup.hotkey == wl.hotkey
                        && let Some(engine) = s.word_lookup_engine.as_mut()
                    {
                        engine.set_hotkey_manager(Some(mgr));
                    }
                    if let Some(mgr) = qt_mgr
                        && s.quick_translate.enabled == qt.enabled
                        && s.quick_translate.hotkey == qt.hotkey
                        && let Some(engine) = s.quick_translate_engine.as_mut()
                    {
                        engine.set_hotkey_manager(Some(mgr));
                    }
                });
            });
        })
        .detach();
    }
}

/// Keep popup state in sync when the popup window is destroyed by a
/// path that bypasses `close_popup` (window manager close, Alt+F4,
/// compositor kill).
fn watch_popup_window_close(state: &Entity<DictState>, cx: &mut App) {
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
                if let Some(engine) = s.quick_translate_engine.as_mut()
                    && !replace_pending
                {
                    engine.hide_popup();
                }
                if let Some(engine) = s.word_lookup_engine.as_mut()
                    && !replace_pending
                {
                    engine.hide_popup();
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
}

/// Start the app machinery without showing the main window (`--tray`).
/// Same wiring `open_dictionary_window` wraps around the window: state +
/// engines (which register the OS-level shortcuts in `DictState::new`),
/// hotkey-manager registration, the trigger poll loop, popup housekeeping,
/// and background indexing. The tray's "Show Dictionary" (or a second
/// plain launch) later opens the window via `raise_dictionary_window`.
fn start_background_machinery(cx: &mut App) {
    let state = cx.new(|_cx| DictState::new());
    attach_hotkey_managers(&state, cx);
    watch_popup_window_close(&state, cx);
    app::spawn_trigger_poll_loop(state.clone(), cx);
    indexing::spawn(state, cx);
}

fn open_dictionary_window(cx: &mut App) {
    dicto_telemetry::get().track(dicto_telemetry::Event::WindowOpened);
    let bounds = Bounds::centered(None, size(px(920.), px(680.)), cx);

    let state_for_indexing: std::cell::RefCell<Option<gpui::Entity<DictState>>> =
        std::cell::RefCell::new(None);

    let handle: gpui::AnyWindowHandle = cx
        .open_window(
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
        .expect("failed to open window")
        .into();
    // Track the handle for the tray's "Show Dictionary": it raises+focuses
    // this exact window (`cx.windows()` would also match popups).
    if let Ok(mut g) = DICT_WINDOW.lock() {
        *g = Some(handle);
    }

    if let Some(state) = state_for_indexing.into_inner() {
        attach_hotkey_managers(&state, cx);
        watch_popup_window_close(&state, cx);
        indexing::spawn(state, cx);
    }
}
