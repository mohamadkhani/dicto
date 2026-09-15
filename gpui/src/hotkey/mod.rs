//! Global hotkey registration with platform backends.
//!
//! Provides a unified [`HotkeyManager`] trait with multiple backends:
//! - X11: `global-hotkey` crate (tauri-apps) — full support
//! - Wayland: XDG GlobalShortcuts portal (`ashpd`) — GNOME/KDE support
//! - Fallback: tray-menu-only mode when no global hotkey backend is available
//!
//! Hotkey events are polled via [`HotkeyManager::try_recv`] so the GPUI
//! event loop can check them on each tick without blocking.

use thiserror::Error;
use tracing::{info, warn};

/// Errors from hotkey registration.
#[derive(Debug, Error)]
pub enum HotkeyError {
    #[error("hotkey already registered: {0}")]
    AlreadyRegistered(String),
    #[error("invalid hotkey string: {0}")]
    InvalidHotkey(String),
    #[error("hotkey backend unavailable: {0}")]
    Unavailable(String),
    #[error("registration failed: {0}")]
    RegistrationFailed(String),
}

/// Identifier for the quick-translate hotkey.
pub const QUICK_TRANSLATE_ID: &str = "quick_translate";

/// A platform-agnostic hotkey manager.
///
/// Call [`try_recv`](Self::try_recv) periodically (e.g. in the GPUI event
/// loop or a polling timer) to check for hotkey presses.
pub trait HotkeyManager: Send + Sync {
    /// Register a hotkey with the given id and key combination string.
    ///
    /// The hotkey string format is "Mod+Mod+Key", e.g. "Ctrl+Alt+D".
    fn register(&self, id: &str, hotkey: &str) -> Result<(), HotkeyError>;

    /// Unregister a previously registered hotkey.
    fn unregister(&self, id: &str) -> Result<(), HotkeyError>;

    /// Try to receive the next hotkey event without blocking.
    ///
    /// Returns `Some(id)` if a hotkey was pressed since the last call,
    /// or `None` if no new events are available.
    fn try_recv(&self) -> Option<String>;

    /// Name of the active backend, for display in settings.
    fn backend_name(&self) -> &'static str;
}

/// Detect the best available hotkey backend at runtime and create a manager.
///
/// On Wayland: tries the XDG GlobalShortcuts portal, falls back to tray menu.
/// On X11: uses the `global-hotkey` crate.
///
/// Note: `XDG_SESSION_TYPE` takes priority over the presence of `DISPLAY`,
/// because Wayland compositors run XWayland (so `DISPLAY` is usually set
/// even on Wayland). If we checked `DISPLAY` first, we'd wrongly pick the
/// X11 backend on GNOME/KDE Wayland — and X11 global hotkeys only fire
/// inside XWayland apps, never native Wayland ones.
pub fn create_hotkey_manager() -> Box<dyn HotkeyManager> {
    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    info!(session_type = %session_type, "hotkey: detecting backend");

    if session_type == "wayland" || std::env::var("WAYLAND_DISPLAY").is_ok() {
        match PortalHotkeyManager::new() {
            Ok(manager) => {
                info!("hotkey: using XDG Portal backend (Wayland)");
                return Box::new(manager);
            }
            Err(e) => {
                warn!(error = %e, "hotkey: XDG Portal backend failed, falling back to tray menu");
            }
        }
    }

    if session_type == "x11" || std::env::var("DISPLAY").is_ok() {
        match X11HotkeyManager::new() {
            Ok(manager) => {
                info!("hotkey: using X11 backend");
                return Box::new(manager);
            }
            Err(e) => {
                warn!(error = %e, "hotkey: X11 backend failed");
            }
        }
    }

    warn!("hotkey: no global hotkey backend available, using tray menu fallback");
    Box::new(FallbackHotkeyManager::new())
}

// --- Backends ---

pub mod fallback;
pub use fallback::FallbackHotkeyManager;

pub mod portal;
pub use portal::PortalHotkeyManager;

pub mod x11;
pub use x11::X11HotkeyManager;
