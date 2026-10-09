//! XDG Desktop Portal `GlobalShortcuts` backend (Wayland / GNOME / KDE).
//!
//! Uses the `ashpd` crate (the maintained Rust XDG portal client) to drive the
//! portal's async API correctly — `ashpd` serializes the `Session` handle as a
//! D-Bus object path (`o`) and the shortcuts list as `a(sa{sv})`, which is the
//! type quirk that defeats a hand-rolled `zbus` `call_method`.
//!
//! GNOME's `xdg-desktop-portal-gnome` additionally requires non-sandboxed
//! apps to declare their app_id via `Registry.Register` *before*
//! `CreateSession`, else it rejects with "An app id is required". We do that
//! registration with a single low-level `zbus` call (ashpd doesn't expose it).
//!
//! ## Process-wide session
//!
//! All portal shortcuts live in ONE session owned by a singleton
//! [`PortalShared`]. Every feature engine gets its own lightweight
//! [`PortalHotkeyManager`] handle; `register()` updates the shared
//! desired-bindings set and bumps a generation counter, and the portal thread
//! rebinds the session whenever the generation changes. This lets both Quick
//! Translate and Word Lookup share one desktop confirmation instead of
//! fighting over two sessions.
//!
//! Events carry the portal shortcut id; each handle only surfaces the id it
//! registered, so two engines can poll the same event log independently.
//!
//! The whole portal flow runs on a dedicated thread with its own multi-threaded
//! tokio runtime (ashpd is async-only).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use std::collections::HashMap;

use ashpd::desktop::global_shortcuts::GlobalShortcuts;
use tracing::{error, info, warn};

use crate::hotkey::{HotkeyError, HotkeyManager, QUICK_LOOKUP_ID, QUICK_TRANSLATE_ID, keys};

const APP_ID: &str = "com.mohamad.dicto";

/// How long `portal_handle` waits for the first bind attempt to settle before
/// assuming the portal works anyway (KDE may sit in a confirmation dialog).
const BIND_SETTLE_TIMEOUT: Duration = Duration::from_secs(3);

/// Delay before retrying after a failed bind (e.g. GNOME without allowlist —
/// may succeed once the admin allowlists us, or the portal comes up late).
const BIND_RETRY_DELAY: Duration = Duration::from_secs(10);

/// How often the portal loop checks for generation changes / shutdown.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Human-facing description for a portal shortcut id. Shown in the desktop's
/// shortcut configuration UI.
fn description_for(id: &str) -> &'static str {
    match id {
        QUICK_TRANSLATE_ID => "Translate selected text",
        QUICK_LOOKUP_ID => "Look up the selected word",
        _ => "Dicto action",
    }
}

/// A shortcut we want bound: portal shortcut id + preferred trigger in GTK
/// accelerator syntax (`"<Control><Alt>d"`).
#[derive(Debug, Clone)]
struct DesiredBinding {
    id: String,
    trigger: String,
}

/// Bind state of the portal thread, read by `portal_handle` to decide whether
/// the portal backend is actually usable (GNOME rejects non-allowlisted apps
/// at bind time — that must fall back to the tray/gsettings path, not silently
/// dead-end in a portal session that never fires).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortalState {
    /// Thread started, first bind attempt still in flight.
    Connecting,
    /// At least one successful bind since startup.
    Bound,
    /// The portal rejected our bind (GNOME allowlist, no portal backend…).
    Failed,
}

/// One activation event, tagged with a process-wide sequence number so each
/// engine handle can track its own read position.
#[derive(Debug)]
struct PortalEvent {
    seq: u64,
    id: String,
}

/// Shared state between the portal thread and every engine handle.
pub struct PortalShared {
    /// Shortcuts we want bound right now.
    desired: Mutex<Vec<DesiredBinding>>,
    /// Bumped on every desired-set change; the portal thread rebinds when it
    /// sees a generation it hasn't bound yet.
    generation: AtomicU64,
    /// Activation event log. Each handle holds its own cursor; events stay
    /// until every interested handle has passed them (bounded defensively).
    events: Mutex<Vec<PortalEvent>>,
    next_seq: AtomicU64,
    state: Mutex<PortalState>,
}

impl PortalShared {
    fn set_state(&self, state: PortalState) {
        *self.state.lock().unwrap() = state;
    }

    fn set_desired(&self, id: &str, trigger: String) {
        {
            let mut desired = self.desired.lock().unwrap();
            match desired.iter_mut().find(|b| b.id == id) {
                // No change — don't force a pointless session rebind.
                Some(b) if b.trigger == trigger => return,
                Some(b) => b.trigger = trigger,
                None => desired.push(DesiredBinding {
                    id: id.to_string(),
                    trigger,
                }),
            }
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    fn remove_desired(&self, id: &str) {
        {
            let mut desired = self.desired.lock().unwrap();
            let before = desired.len();
            desired.retain(|b| b.id != id);
            if desired.len() == before {
                return;
            }
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    fn push_event(&self, id: String) {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        self.events.lock().unwrap().push(PortalEvent { seq, id });
    }
}

static PORTAL_SHARED: OnceLock<Option<Arc<PortalShared>>> = OnceLock::new();

/// Set once the optimistic "assume the portal works" decision has been made
/// for this process — later handles must not each burn another settle
/// timeout while a KDE confirmation dialog is still open.
static OPTIMISTIC: AtomicBool = AtomicBool::new(false);

/// Get (starting it on first use) the process-wide portal manager, or `None`
/// when the portal is unusable — the caller should fall back to another
/// backend. Waits up to [`BIND_SETTLE_TIMEOUT`] for the first bind attempt so
/// a GNOME allowlist rejection is detected here, not discovered as dead
/// hotkeys later.
pub fn portal_handle() -> Option<PortalHotkeyManager> {
    let shared = PORTAL_SHARED
        .get_or_init(|| {
            let shared = Arc::new(PortalShared {
                desired: Mutex::new(Vec::new()),
                generation: AtomicU64::new(0),
                events: Mutex::new(Vec::new()),
                next_seq: AtomicU64::new(0),
                state: Mutex::new(PortalState::Connecting),
            });
            let thread_shared = shared.clone();
            match std::thread::Builder::new()
                .name("portal-hotkey".into())
                .spawn(move || run_portal(thread_shared))
            {
                Ok(_) => Some(shared),
                Err(e) => {
                    warn!(error = %e, "portal: failed to spawn portal thread");
                    None
                }
            }
        })
        .as_ref()?
        .clone();

    let deadline = Instant::now() + BIND_SETTLE_TIMEOUT;
    loop {
        let state = *shared.state.lock().unwrap();
        match state {
            PortalState::Failed => return None,
            PortalState::Bound => return Some(PortalHotkeyManager::new(shared.clone())),
            PortalState::Connecting => {
                if OPTIMISTIC.load(Ordering::Acquire) {
                    return Some(PortalHotkeyManager::new(shared.clone()));
                }
                if Instant::now() >= deadline {
                    // Still negotiating (KDE dialog awaiting user consent).
                    // Optimistically stay on the portal; a late failure is
                    // no worse than the old always-succeed behavior.
                    OPTIMISTIC.store(true, Ordering::Release);
                    return Some(PortalHotkeyManager::new(shared.clone()));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// Per-engine handle. Cheap to create; all real state lives in
/// [`PortalShared`].
pub struct PortalHotkeyManager {
    shared: Arc<PortalShared>,
    /// The logical id this handle registered — `try_recv` only surfaces
    /// events for it, and dropping the handle unbinds it.
    filter: Mutex<Option<String>>,
    /// Sequence number of the last event this handle examined.
    cursor: AtomicU64,
}

impl PortalHotkeyManager {
    fn new(shared: Arc<PortalShared>) -> Self {
        Self {
            shared,
            filter: Mutex::new(None),
            cursor: AtomicU64::new(0),
        }
    }
}

impl HotkeyManager for PortalHotkeyManager {
    fn register(&self, id: &str, hotkey: &str) -> Result<(), HotkeyError> {
        let trigger = keys::to_gtk_accel(hotkey)?;
        info!(id, %trigger, "portal: desired binding set");
        self.shared.set_desired(id, trigger);
        *self.filter.lock().unwrap() = Some(id.to_string());
        Ok(())
    }

    fn unregister(&self, id: &str) -> Result<(), HotkeyError> {
        self.shared.remove_desired(id);
        Ok(())
    }

    fn try_recv(&self) -> Option<String> {
        let filter = self.filter.lock().unwrap().clone()?;
        let mut events = self.shared.events.lock().unwrap();
        let cursor = self.cursor.load(Ordering::Acquire);
        let mut taken: Option<usize> = None;
        for (i, event) in events.iter().enumerate() {
            if event.seq <= cursor {
                continue; // already consumed by this handle
            }
            if event.id == filter {
                taken = Some(i);
                break;
            }
            // Not ours — advance past it but keep it for the other engine.
            self.cursor.store(event.seq, Ordering::Release);
        }
        let id = taken.map(|i| events.remove(i).id);
        // Defensive bound: both engines poll on every UI tick, so the log
        // stays tiny; this only guards against a stalled consumer.
        let len = events.len();
        if len > 1024 {
            events.drain(0..len - 1024);
        }
        id
    }

    fn backend_name(&self) -> &'static str {
        "xdg-portal"
    }
}

impl Drop for PortalHotkeyManager {
    fn drop(&mut self) {
        if let Some(id) = self.filter.lock().unwrap().take() {
            self.shared.remove_desired(&id);
        }
    }
}

/// Run the portal flow on a dedicated tokio runtime.
fn run_portal(shared: Arc<PortalShared>) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            error!(error = %e, "portal: failed to build tokio runtime");
            shared.set_state(PortalState::Failed);
            return;
        }
    };
    if let Err(e) = runtime.block_on(portal_loop(shared.clone())) {
        error!(error = %e, "portal hotkey loop failed");
        shared.set_state(PortalState::Failed);
    }
}

/// The async portal flow: register app_id → connect → bind the desired
/// shortcuts → listen for activations, rebinding whenever the desired set
/// changes (settings edit, feature toggle, engine reconfigure).
async fn portal_loop(
    shared: Arc<PortalShared>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use ashpd::desktop::Session;

    // 1. Open one session-bus connection and register our app_id on it.
    //    GNOME requires this before CreateSession, and it MUST be the same
    //    connection the portal calls are made on (Registry.Register is
    //    per-connection).
    let conn = zbus::Connection::session().await?;
    register_app_id(&conn).await?;

    let global_shortcuts = GlobalShortcuts::with_connection(conn).await?;
    info!("portal: connected");

    // Generation we last bound; `None` forces a (re)bind on the next pass.
    let mut bound_gen: Option<u64> = None;
    // Kept alive so the current session stays valid; dropping it unbinds.
    // Kept alive so the session stays valid; dropping it unbinds.
    let mut _session: Option<Session<GlobalShortcuts>> = None;
    let mut activated: Option<_> = None;

    loop {
        let generation = shared.generation.load(Ordering::Acquire);
        if bound_gen != Some(generation) {
            let desired = shared.desired.lock().unwrap().clone();
            // Drop the old session + stream before rebinding.
            _session = None;
            activated = None;

            if desired.is_empty() {
                // Nothing wanted — stay unbound until someone registers.
                bound_gen = Some(generation);
            } else {
                let count = desired.len();
                match bind_cycle(&global_shortcuts, desired).await {
                    Ok((new_session, stream)) => {
                        info!(count, "portal: shortcuts bound");
                        _session = Some(new_session);
                        activated = Some(stream);
                        bound_gen = Some(generation);
                        shared.set_state(PortalState::Bound);
                    }
                    Err(e) => {
                        warn!(error = %e, "portal: bind failed; will retry");
                        shared.set_state(PortalState::Failed);
                        bound_gen = None;
                        tokio::time::sleep(BIND_RETRY_DELAY).await;
                        continue;
                    }
                }
            }
        }

        match activated.as_mut() {
            Some(stream) => {
                tokio::select! {
                    event = futures_util::StreamExt::next(stream) => match event {
                        Some(activation) => {
                            let id = activation.shortcut_id().to_string();
                            info!(shortcut_id = %id, "portal: shortcut activated");
                            shared.push_event(id);
                        }
                        None => {
                            // Signal stream ended — force a fresh session.
                            warn!("portal: activation stream ended; rebinding");
                            bound_gen = None;
                            activated = None;
                            _session = None;
                        }
                    },
                    _ = tokio::time::sleep(POLL_INTERVAL) => {}
                }
            }
            None => {
                // Idle (nothing desired yet): just watch for registrations.
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
    }
}

/// Create a session, bind `desired`, and return the session handle (must be
/// kept alive by the caller) plus the activation stream.
async fn bind_cycle(
    global_shortcuts: &GlobalShortcuts,
    desired: Vec<DesiredBinding>,
) -> Result<
    (
        ashpd::desktop::Session<GlobalShortcuts>,
        impl futures_util::Stream<Item = ashpd::desktop::global_shortcuts::Activated>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    use ashpd::desktop::CreateSessionOptions;
    use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, NewShortcut};

    // Create a GlobalShortcuts session on the registered connection.
    let session = global_shortcuts
        .create_session(CreateSessionOptions::default())
        .await?;
    info!("portal: session created");

    // Bind the desired shortcuts with their preferred triggers. The
    // preferred trigger is a hint — desktops may prompt the user to confirm
    // or adjust (KDE shows a one-time dialog per app).
    let shortcuts: Vec<NewShortcut> = desired
        .iter()
        .map(|binding| {
            NewShortcut::new(&binding.id, description_for(&binding.id))
                .preferred_trigger(binding.trigger.as_str())
        })
        .collect();
    global_shortcuts
        .bind_shortcuts(&session, &shortcuts, None, BindShortcutsOptions::default())
        .await?;

    // Listen for Activated signals until the session is replaced.
    let activated = global_shortcuts.receive_activated().await?;
    Ok((session, activated))
}

/// Typed proxy for the portal's host-side Registry interface.
#[zbus::proxy(
    interface = "org.freedesktop.host.portal.Registry",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait Registry {
    /// Register the caller's app_id. `options` is currently empty.
    fn register(
        &self,
        app_id: &str,
        options: HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<()>;
}

/// Call `Registry.Register(app_id, {})` so GNOME's portal accepts subsequent
/// app-id-gated calls from this native binary. Must be called on the same
/// connection that issues CreateSession.
async fn register_app_id(
    conn: &zbus::Connection,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let registry = RegistryProxy::new(conn).await?;
    let empty_opts: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
    registry.register(APP_ID, empty_opts).await?;
    info!(app_id = APP_ID, "portal: registered app_id with portal");
    Ok(())
}
