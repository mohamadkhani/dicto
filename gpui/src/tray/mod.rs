//! System tray support.
//!
//! Platform backends behind a plain facade (the OS is known at compile time;
//! only the hotkey layer needs runtime detection):
//! - Linux: `ksni` speaking SNI over D-Bus ([`ksni`])
//! - Windows: the `tray-icon` crate ([`win`])
//!
//! Both backends expose the same contract: they spawn the tray and return
//! the action channel the GPUI main loop polls plus the shared
//! activation-token slot. Actions are handled in `main.rs`; the "Quick
//! Translate" item shares the trigger path used by `dicto --translate` IPC
//! (see `app.rs`).

pub mod icon;

#[cfg(target_os = "linux")]
pub mod ksni;
#[cfg(target_os = "windows")]
pub mod win;

use std::sync::{Arc, Mutex, mpsc};

/// The last compositor-provided activation token, shared between the SNI
/// service thread (which receives it) and the menu-item click handlers.
///
/// Forwarded to the popup window's `Window::activate_with_token` (provided by
/// the mohamadkhani/zed gpui fork) for authoritative focus on GNOME/Wayland.
/// Always `None` on Windows (a Wayland-only concept).
pub type SharedToken = Arc<Mutex<Option<String>>>;

/// Actions the tray requests the GPUI main loop to perform.
#[derive(Debug)]
pub enum TrayAction {
    Show,
    QuickTranslate,
    Quit,
}

/// Spawn the platform tray. Returns the channel the GPUI main loop polls for
/// actions and the shared activation-token slot.
pub fn spawn_tray() -> (mpsc::Receiver<TrayAction>, SharedToken) {
    #[cfg(target_os = "linux")]
    return ksni::spawn();

    #[cfg(target_os = "windows")]
    return win::spawn();
}
