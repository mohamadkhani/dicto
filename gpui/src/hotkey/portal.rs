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
//! The whole portal flow runs on a dedicated thread with its own multi-threaded
//! tokio runtime (ashpd is async-only). Shortcut activations are pushed into a
//! shared `Arc<Mutex<Vec<String>>>` that `try_recv` drains from the GPUI loop.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tracing::{error, info, warn};

use crate::hotkey::{HotkeyError, HotkeyManager};

const APP_ID: &str = "com.mohamad.dicto";

/// Shortcut id the portal reports back when our binding fires.
const SHORTCUT_ID: &str = "quick_translate";
/// Preferred trigger, GTK accelerator syntax (the "shortcuts" XDG spec).
const PREFERRED_TRIGGER: &str = "<Control><Alt>d";

/// Wayland XDG GlobalShortcuts portal backend (via ashpd).
pub struct PortalHotkeyManager {
    events: Arc<Mutex<Vec<String>>>,
    _handle: std::thread::JoinHandle<()>,
}

impl PortalHotkeyManager {
    pub fn new() -> Result<Self, HotkeyError> {
        let events = Arc::new(Mutex::new(Vec::<String>::new()));
        let events_clone = events.clone();

        let handle = std::thread::Builder::new()
            .name("portal-hotkey".into())
            .spawn(move || run_portal(events_clone))
            .map_err(|e| HotkeyError::Unavailable(format!("failed to spawn portal thread: {e}")))?;

        Ok(Self {
            events,
            _handle: handle,
        })
    }
}

impl HotkeyManager for PortalHotkeyManager {
    fn register(&self, _id: &str, _hotkey: &str) -> Result<(), HotkeyError> {
        info!("portal: register called (binding happens at session creation)");
        Ok(())
    }

    fn unregister(&self, _id: &str) -> Result<(), HotkeyError> {
        info!("portal: unregister called (no-op)");
        Ok(())
    }

    fn try_recv(&self) -> Option<String> {
        let mut events = self.events.lock().unwrap();
        if events.is_empty() {
            None
        } else {
            Some(events.remove(0))
        }
    }

    fn backend_name(&self) -> &'static str {
        "xdg-portal"
    }
}

/// Run the full portal flow on a dedicated tokio runtime.
fn run_portal(events: Arc<Mutex<Vec<String>>>) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            error!(error = %e, "portal: failed to build tokio runtime");
            return;
        }
    };
    if let Err(e) = runtime.block_on(portal_loop(events)) {
        error!(error = %e, "portal hotkey loop failed");
    }
}

/// The async portal flow: register app_id → create session → bind shortcut →
/// listen for activations forever.
async fn portal_loop(
    events: Arc<Mutex<Vec<String>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use ashpd::desktop::CreateSessionOptions;
    use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};

    // 1. Open one session-bus connection and register our app_id on it.
    //    GNOME requires this before CreateSession, and it MUST be the same
    //    connection the portal calls are made on (Registry.Register is
    //    per-connection).
    let conn = zbus::Connection::session().await?;
    register_app_id(&conn).await?;

    // 2. Create a GlobalShortcuts session on that registered connection.
    let global_shortcuts = GlobalShortcuts::with_connection(conn).await?;
    let session = global_shortcuts
        .create_session(CreateSessionOptions::default())
        .await?;
    info!("portal: session created");

    // 3. Bind our shortcut.
    let shortcut = NewShortcut::new(SHORTCUT_ID, "Translate selected text")
        .preferred_trigger(PREFERRED_TRIGGER);
    global_shortcuts
        .bind_shortcuts(&session, &[shortcut], None, BindShortcutsOptions::default())
        .await?;
    info!("portal: shortcut bound");

    // 4. Listen for Activated signals until the process exits.
    use futures_util::StreamExt;
    let mut activated = global_shortcuts.receive_activated().await?;
    info!("portal: listening for activations...");
    while let Some(event) = activated.next().await {
        let id = event.shortcut_id().to_string();
        info!(shortcut_id = %id, "portal: shortcut activated");
        if let Ok(mut buf) = events.lock() {
            buf.push(id);
        }
    }

    warn!("portal: activation stream ended");
    Ok(())
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
