//! X11 global hotkey backend using the `global-hotkey` crate.
//!
//! Works on X11 and XWayland. Registers a system-wide hotkey and delivers
//! events via an mpsc channel polled with `try_recv`.
//!
//! The `global-hotkey` crate delivers events through a global receiver,
//! so we spawn a background thread that forwards events to our channel.

use std::sync::{Arc, Mutex};

use global_hotkey::{
    GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use tracing::{debug, error, info};

use crate::hotkey::{HotkeyError, HotkeyManager};

/// X11 global hotkey manager.
pub struct X11HotkeyManager {
    inner: Arc<Mutex<Option<GlobalHotKeyManager>>>,
    registered: Arc<Mutex<Vec<HotKey>>>,
    events: Arc<Mutex<Vec<String>>>,
}

impl X11HotkeyManager {
    pub fn new() -> Result<Self, HotkeyError> {
        let manager = GlobalHotKeyManager::new().map_err(|e| {
            HotkeyError::Unavailable(format!("failed to create hotkey manager: {e}"))
        })?;

        let events = Arc::new(Mutex::new(Vec::<String>::new()));
        let events_clone = events.clone();

        // Spawn a background thread that polls the global hotkey event receiver.
        std::thread::Builder::new()
            .name("hotkey-listener".into())
            .spawn(move || {
                let receiver = global_hotkey::GlobalHotKeyEvent::receiver();
                loop {
                    match receiver.recv() {
                        Ok(event) => {
                            if event.state == HotKeyState::Pressed {
                                debug!(id = event.id, "hotkey pressed");
                                // We map hotkey id 0 → "quick_translate"
                                if event.id == 0 {
                                    let mut buf = events_clone.lock().unwrap();
                                    buf.push("quick_translate".to_string());
                                }
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "hotkey event receiver disconnected");
                            break;
                        }
                    }
                }
            })
            .map_err(|e| {
                HotkeyError::Unavailable(format!("failed to spawn listener thread: {e}"))
            })?;

        Ok(Self {
            inner: Arc::new(Mutex::new(Some(manager))),
            registered: Arc::new(Mutex::new(Vec::new())),
            events,
        })
    }
}

impl HotkeyManager for X11HotkeyManager {
    fn register(&self, _id: &str, hotkey: &str) -> Result<(), HotkeyError> {
        let parsed = parse_hotkey(hotkey)?;
        let manager = self.inner.lock().unwrap();
        let manager = manager
            .as_ref()
            .ok_or_else(|| HotkeyError::Unavailable("manager has been dropped".into()))?;

        let hotkey_obj = HotKey::new(Some(parsed.modifiers), parsed.key);

        // Unregister any existing hotkeys first
        {
            let mut registered = self.registered.lock().unwrap();
            if !registered.is_empty() {
                let _ = manager.unregister_all(&registered);
                registered.clear();
            }
        }

        manager
            .register(hotkey_obj.clone())
            .map_err(|e| HotkeyError::RegistrationFailed(format!("{e}")))?;

        {
            let mut registered = self.registered.lock().unwrap();
            registered.push(hotkey_obj);
        }

        info!("registered X11 hotkey");
        Ok(())
    }

    fn unregister(&self, _id: &str) -> Result<(), HotkeyError> {
        let manager = self.inner.lock().unwrap();
        if let Some(manager) = manager.as_ref() {
            let mut registered = self.registered.lock().unwrap();
            if !registered.is_empty() {
                let _ = manager.unregister_all(&registered);
                registered.clear();
                info!("unregistered all X11 hotkeys");
            }
        }
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
        "x11"
    }
}

/// Parsed hotkey components.
#[derive(Debug)]
pub struct ParsedHotkey {
    pub modifiers: Modifiers,
    pub key: Code,
}

/// Parse a hotkey string like "Ctrl+Alt+D" into modifiers + key code.
pub fn parse_hotkey(s: &str) -> Result<ParsedHotkey, HotkeyError> {
    let mut modifiers = Modifiers::empty();
    let mut key_code: Option<Code> = None;

    for part in s.split('+') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "alt" | "opt" | "option" => modifiers |= Modifiers::ALT,
            "shift" => modifiers |= Modifiers::SHIFT,
            "super" | "win" | "cmd" | "command" | "meta" => modifiers |= Modifiers::META,
            _ => {
                let code = key_name_to_code(part)?;
                key_code = Some(code);
            }
        }
    }

    let key =
        key_code.ok_or_else(|| HotkeyError::InvalidHotkey(format!("no key found in '{s}'")))?;

    Ok(ParsedHotkey { modifiers, key })
}

/// Map a key name string to a `Code` enum variant.
fn key_name_to_code(name: &str) -> Result<Code, HotkeyError> {
    let upper = name.to_uppercase();
    match upper.as_str() {
        "A" => Ok(Code::KeyA),
        "B" => Ok(Code::KeyB),
        "C" => Ok(Code::KeyC),
        "D" => Ok(Code::KeyD),
        "E" => Ok(Code::KeyE),
        "F" => Ok(Code::KeyF),
        "G" => Ok(Code::KeyG),
        "H" => Ok(Code::KeyH),
        "I" => Ok(Code::KeyI),
        "J" => Ok(Code::KeyJ),
        "K" => Ok(Code::KeyK),
        "L" => Ok(Code::KeyL),
        "M" => Ok(Code::KeyM),
        "N" => Ok(Code::KeyN),
        "O" => Ok(Code::KeyO),
        "P" => Ok(Code::KeyP),
        "Q" => Ok(Code::KeyQ),
        "R" => Ok(Code::KeyR),
        "S" => Ok(Code::KeyS),
        "T" => Ok(Code::KeyT),
        "U" => Ok(Code::KeyU),
        "V" => Ok(Code::KeyV),
        "W" => Ok(Code::KeyW),
        "X" => Ok(Code::KeyX),
        "Y" => Ok(Code::KeyY),
        "Z" => Ok(Code::KeyZ),
        "0" => Ok(Code::Digit0),
        "1" => Ok(Code::Digit1),
        "2" => Ok(Code::Digit2),
        "3" => Ok(Code::Digit3),
        "4" => Ok(Code::Digit4),
        "5" => Ok(Code::Digit5),
        "6" => Ok(Code::Digit6),
        "7" => Ok(Code::Digit7),
        "8" => Ok(Code::Digit8),
        "9" => Ok(Code::Digit9),
        "SPACE" => Ok(Code::Space),
        "ENTER" | "RETURN" => Ok(Code::Enter),
        "ESCAPE" | "ESC" => Ok(Code::Escape),
        "TAB" => Ok(Code::Tab),
        "F1" => Ok(Code::F1),
        "F2" => Ok(Code::F2),
        "F3" => Ok(Code::F3),
        "F4" => Ok(Code::F4),
        "F5" => Ok(Code::F5),
        "F6" => Ok(Code::F6),
        "F7" => Ok(Code::F7),
        "F8" => Ok(Code::F8),
        "F9" => Ok(Code::F9),
        "F10" => Ok(Code::F10),
        "F11" => Ok(Code::F11),
        "F12" => Ok(Code::F12),
        _ => Err(HotkeyError::InvalidHotkey(format!(
            "unsupported key: '{name}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ctrl_alt_d() {
        let hk = parse_hotkey("Ctrl+Alt+D").unwrap();
        assert!(hk.modifiers.contains(Modifiers::CONTROL));
        assert!(hk.modifiers.contains(Modifiers::ALT));
        assert_eq!(hk.key, Code::KeyD);
    }

    #[test]
    fn test_parse_shift_f1() {
        let hk = parse_hotkey("Shift+F1").unwrap();
        assert!(hk.modifiers.contains(Modifiers::SHIFT));
        assert_eq!(hk.key, Code::F1);
    }

    #[test]
    fn test_parse_super_space() {
        let hk = parse_hotkey("Super+Space").unwrap();
        assert!(hk.modifiers.contains(Modifiers::META));
        assert_eq!(hk.key, Code::Space);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse_hotkey("Ctrl+Alt+").is_err());
        assert!(parse_hotkey("Ctrl+F5").is_ok());
        assert!(parse_hotkey("just_a_key").is_err());
    }
}
