//! Fallback hotkey manager that has no global hotkey support.
//!
//! Used when no platform hotkey backend is available (e.g. on Wayland
//! without the XDG GlobalShortcuts portal). The user can still trigger
//! quick translate via the tray menu.

use std::sync::{Arc, Mutex};

use crate::hotkey::{HotkeyError, HotkeyManager};

/// No-op hotkey manager.
///
/// Register/unregister succeed silently (no-op). `try_recv` always returns
/// `None`. The user triggers quick translate via the tray menu instead.
pub struct FallbackHotkeyManager {
    registered: Arc<Mutex<Vec<String>>>,
}

impl FallbackHotkeyManager {
    pub fn new() -> Self {
        Self {
            registered: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Default for FallbackHotkeyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl HotkeyManager for FallbackHotkeyManager {
    fn register(&self, id: &str, _hotkey: &str) -> Result<(), HotkeyError> {
        let mut registered = self.registered.lock().unwrap();
        if !registered.iter().any(|s| s == id) {
            registered.push(id.to_string());
        }
        Ok(())
    }

    fn unregister(&self, id: &str) -> Result<(), HotkeyError> {
        let mut registered = self.registered.lock().unwrap();
        registered.retain(|s| s != id);
        Ok(())
    }

    fn try_recv(&self) -> Option<String> {
        None
    }

    fn backend_name(&self) -> &'static str {
        "tray_menu"
    }
}
