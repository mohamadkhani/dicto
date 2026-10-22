//! Global hotkey backend wrapping the `global-hotkey` crate (tauri-apps).
//!
//! One backend serves both platforms that can use `global-hotkey`:
//! - Linux/X11 (and XWayland): the manager is created directly on this
//!   thread; the crate's X11 event source needs no message pump.
//! - Windows: `RegisterHotKey` posts `WM_HOTKEY` to the message queue of the
//!   thread that owns the manager's window, so the manager must live on a
//!   thread that pumps Win32 messages. The backend therefore lazily starts a
//!   **singleton pump thread** that owns one process-lifetime manager;
//!   `register`/`unregister` are sent to it as commands with reply channels
//!   so errors still surface synchronously as [`HotkeyError`]. The manager is
//!   never dropped or recreated — reconfiguring the hotkey is an Unregister +
//!   Register round-trip on the pump.
//!
//! Events arrive through the crate's process-global receiver on a shared
//! listener thread which maps `event.id` (the id of the registered `HotKey`)
//! back to the logical name given at registration time (`"quick_translate"`).

#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "windows")]
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use tracing::{debug, error, info, warn};

use crate::hotkey::{HotkeyError, HotkeyManager, keys};

/// Hotkey id → logical name (e.g. `"quick_translate"`), written by
/// `register`/`unregister`/`Drop` and read by the listener thread. Shared
/// across backend instances because the event receiver is process-global.
/// (A `Vec` because there are at most a couple of hotkeys and `Vec::new`
/// is `const`.)
static EVENT_NAMES: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());

/// Logical names of pressed hotkeys, waiting for [`HotkeyManager::try_recv`].
static PENDING_EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// X11 / Windows global hotkey manager.
pub struct GlobalHotkeyBackend {
    backend_name: &'static str,
    /// Hotkeys registered by *this* instance, so cleanup touches exactly what
    /// it registered (older instances may still be shutting down).
    registered: Mutex<Vec<HotKey>>,
    #[cfg(target_os = "linux")]
    manager: Arc<Mutex<Option<GlobalHotKeyManager>>>,
}

impl GlobalHotkeyBackend {
    pub fn new(backend_name: &'static str) -> Result<Self, HotkeyError> {
        ensure_event_listener();

        #[cfg(target_os = "linux")]
        let manager = {
            let manager = GlobalHotKeyManager::new().map_err(|e| {
                HotkeyError::Unavailable(format!("failed to create hotkey manager: {e}"))
            })?;
            Arc::new(Mutex::new(Some(manager)))
        };

        // Ensures the Windows pump thread is up (or fails `new` if it cannot
        // be created); the Linux manager is created directly above.
        #[cfg(target_os = "windows")]
        pump_tx()?;

        Ok(Self {
            backend_name,
            registered: Mutex::new(Vec::new()),
            #[cfg(target_os = "linux")]
            manager,
        })
    }

    /// Register `hotkey`, first clearing any hotkeys this backend still holds.
    fn register_hotkey(&self, hotkey: HotKey) -> Result<(), HotkeyError> {
        // Unregister any existing hotkeys first.
        self.clear_registered()?;

        #[cfg(target_os = "linux")]
        {
            let manager = self.manager.lock().unwrap();
            let manager = manager
                .as_ref()
                .ok_or_else(|| HotkeyError::Unavailable("manager has been dropped".into()))?;
            manager
                .register(hotkey)
                .map_err(|e| HotkeyError::RegistrationFailed(format!("{e}")))?;
        }

        #[cfg(target_os = "windows")]
        {
            let (cmd, reply) = Command::register(hotkey.clone());
            pump_call(cmd, reply)?;
        }

        self.registered.lock().unwrap().push(hotkey);
        info!("registered global hotkey");
        Ok(())
    }

    /// Unregister and forget every hotkey this instance registered.
    fn clear_registered(&self) -> Result<(), HotkeyError> {
        let hotkeys = {
            let mut registered = self.registered.lock().unwrap();
            std::mem::take(&mut *registered)
        };
        for hotkey in &hotkeys {
            #[cfg(target_os = "linux")]
            {
                let manager = self.manager.lock().unwrap();
                if let Some(manager) = manager.as_ref() {
                    let _ = manager.unregister_all(&[*hotkey]);
                }
            }
            #[cfg(target_os = "windows")]
            {
                let (cmd, reply) = Command::unregister(hotkey.clone());
                pump_call(cmd, reply)?;
            }
            EVENT_NAMES
                .lock()
                .unwrap()
                .retain(|(id, _)| *id != hotkey.id());
        }
        if !hotkeys.is_empty() {
            info!("unregistered all global hotkeys");
        }
        Ok(())
    }
}

impl HotkeyManager for GlobalHotkeyBackend {
    fn register(&self, id: &str, hotkey: &str) -> Result<(), HotkeyError> {
        let parsed = keys::parse_hotkey(hotkey)?;
        let hotkey_obj = HotKey::new(Some(parsed.modifiers), parsed.key);

        self.register_hotkey(hotkey_obj)?;

        // Events carry the registered HotKey's numeric id — map it back to
        // the logical name the engine polls for.
        let hotkey_id = hotkey_obj.id();
        let mut names = EVENT_NAMES.lock().unwrap();
        names.retain(|(id, _)| *id != hotkey_id);
        names.push((hotkey_id, id.to_string()));

        Ok(())
    }

    fn unregister(&self, _id: &str) -> Result<(), HotkeyError> {
        self.clear_registered()
    }

    fn try_recv(&self) -> Option<String> {
        let mut events = PENDING_EVENTS.lock().unwrap();
        if events.is_empty() {
            None
        } else {
            Some(events.remove(0))
        }
    }

    fn backend_name(&self) -> &'static str {
        self.backend_name
    }
}

impl Drop for GlobalHotkeyBackend {
    fn drop(&mut self) {
        // Linux: dropping the manager unregisters at the OS level.
        #[cfg(target_os = "linux")]
        {
            if let Ok(mut manager) = self.manager.lock() {
                manager.take(); // GlobalHotKeyManager::Drop unregisters all
            }
        }

        // Windows: the pump's manager lives for the whole process, so
        // explicitly release our bindings.
        #[cfg(target_os = "windows")]
        {
            if let Ok(registered) = self.registered.lock() {
                for hotkey in registered.iter() {
                    let (cmd, reply) = Command::unregister(hotkey.clone());
                    let _ = pump_call(cmd, reply);
                }
            }
        }

        if let (Ok(registered), Ok(mut names)) = (self.registered.lock(), EVENT_NAMES.lock()) {
            for hotkey in registered.iter() {
                names.retain(|(id, _)| *id != hotkey.id());
            }
        }
    }
}

/// Map a `global-hotkey` error to our error type. `AlreadyRegistered` is
/// called out explicitly: it is the expected failure when the OS still holds
/// a stale binding for the same key combination.
#[cfg(target_os = "windows")]
fn map_platform_error(e: global_hotkey::Error) -> HotkeyError {
    match e {
        global_hotkey::Error::AlreadyRegistered(hotkey) => {
            HotkeyError::AlreadyRegistered(format!("{hotkey:?}"))
        }
        other => HotkeyError::RegistrationFailed(other.to_string()),
    }
}

/// Spawn (once) the thread that forwards pressed-hotkey events from the
/// process-global receiver into [`PENDING_EVENTS`], resolving each event's
/// numeric id to its logical name.
fn ensure_event_listener() {
    static LISTENER: OnceLock<()> = OnceLock::new();
    LISTENER.get_or_init(|| {
        let spawned = std::thread::Builder::new()
            .name("hotkey-listener".into())
            .spawn(|| {
                let receiver = GlobalHotKeyEvent::receiver();
                loop {
                    match receiver.recv() {
                        Ok(event) => {
                            if event.state != HotKeyState::Pressed {
                                continue;
                            }
                            debug!(id = event.id, "hotkey pressed");
                            let name = EVENT_NAMES.lock().ok().and_then(|names| {
                                names
                                    .iter()
                                    .find(|(id, _)| *id == event.id)
                                    .map(|(_, name)| name.clone())
                            });
                            if let Some(name) = name
                                && let Ok(mut pending) = PENDING_EVENTS.lock()
                            {
                                pending.push(name);
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "hotkey event receiver disconnected");
                            break;
                        }
                    }
                }
            });
        if let Err(e) = spawned {
            warn!(error = %e, "failed to spawn hotkey listener thread");
        }
    });
}

// --- Windows pump thread ---

/// A register/unregister request for the pump thread. These must run on the
/// pump thread because `RegisterHotKey` binds the hotkey to the message
/// queue of the thread that owns the manager's window.
#[cfg(target_os = "windows")]
enum Command {
    Register {
        hotkey: HotKey,
        reply: mpsc::Sender<Result<(), HotkeyError>>,
    },
    Unregister {
        hotkey: HotKey,
        reply: mpsc::Sender<Result<(), HotkeyError>>,
    },
}

#[cfg(target_os = "windows")]
impl Command {
    /// (`Command`, reply receiver) pair for a synchronous round-trip.
    fn register(hotkey: HotKey) -> (Self, mpsc::Receiver<Result<(), HotkeyError>>) {
        let (reply_tx, reply_rx) = mpsc::channel();
        (
            Self::Register {
                hotkey,
                reply: reply_tx,
            },
            reply_rx,
        )
    }

    fn unregister(hotkey: HotKey) -> (Self, mpsc::Receiver<Result<(), HotkeyError>>) {
        let (reply_tx, reply_rx) = mpsc::channel();
        (
            Self::Unregister {
                hotkey,
                reply: reply_tx,
            },
            reply_rx,
        )
    }
}

/// The singleton pump thread's command channel. `Err` holds the init failure
/// (as a string — `HotkeyError` is not `Clone`) so repeated `new()` calls
/// fail the same way instead of retrying a broken pump.
#[cfg(target_os = "windows")]
static PUMP: OnceLock<Result<mpsc::Sender<Command>, String>> = OnceLock::new();
#[cfg(target_os = "windows")]
static PUMP_INIT: Mutex<()> = Mutex::new(());

/// Get (lazily creating) the command channel of the singleton pump thread.
#[cfg(target_os = "windows")]
fn pump_tx() -> Result<&'static mpsc::Sender<Command>, HotkeyError> {
    if PUMP.get().is_none() {
        let _guard = PUMP_INIT.lock().unwrap();
        PUMP.get_or_init(start_pump);
    }
    match PUMP.get().expect("initialized above") {
        Ok(tx) => Ok(tx),
        Err(e) => Err(HotkeyError::Unavailable(e.clone())),
    }
}

/// Send a command to the pump thread and wait for its reply.
#[cfg(target_os = "windows")]
fn pump_call(
    cmd: Command,
    reply: mpsc::Receiver<Result<(), HotkeyError>>,
) -> Result<(), HotkeyError> {
    pump_tx()?
        .send(cmd)
        .map_err(|_| HotkeyError::Unavailable("hotkey pump thread is gone".into()))?;
    reply
        .recv()
        .map_err(|_| HotkeyError::Unavailable("hotkey pump thread is gone".into()))?
}

/// Start the singleton pump thread and wait until its manager is ready.
#[cfg(target_os = "windows")]
fn start_pump() -> Result<mpsc::Sender<Command>, String> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), HotkeyError>>();

    std::thread::Builder::new()
        .name("hotkey-pump".into())
        .spawn(move || match GlobalHotKeyManager::new() {
            Ok(manager) => {
                let _ = ready_tx.send(Ok(()));
                pump_loop(manager, cmd_rx);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(HotkeyError::Unavailable(format!(
                    "failed to create hotkey manager: {e}"
                ))));
            }
        })
        .map_err(|e| format!("failed to spawn hotkey pump thread: {e}"))?;

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(cmd_tx),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("hotkey pump thread died".into()),
    }
}

/// The pump's main loop: dispatch Win32 messages (`WM_HOTKEY` arrives here)
/// and service register/unregister commands between dispatches.
#[cfg(target_os = "windows")]
fn pump_loop(manager: GlobalHotKeyManager, cmd_rx: mpsc::Receiver<Command>) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::SetTimer;

    // A NULL-hwnd timer wakes GetMessageW every 30 ms so commands are
    // handled promptly even with no hotkey traffic.
    let timer = unsafe { SetTimer(HWND::default(), 0, 30, None) };
    if timer == 0 {
        warn!("hotkey pump: SetTimer failed — commands run only when messages arrive");
    }

    loop {
        if !crate::win32::wait_and_dispatch_one() {
            break;
        }
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Command::Register { hotkey, reply } => {
                    debug!(id = hotkey.id(), "hotkey pump: register");
                    let _ = reply.send(manager.register(hotkey).map_err(map_platform_error));
                }
                Command::Unregister { hotkey, reply } => {
                    debug!(id = hotkey.id(), "hotkey pump: unregister");
                    let _ = reply.send(manager.unregister(hotkey).map_err(map_platform_error));
                }
            }
        }
    }
}
