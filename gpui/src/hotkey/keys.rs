//! Platform-independent hotkey-string parsing.
//!
//! Turns user-facing hotkey strings like `"Ctrl+Alt+D"` into the
//! `Modifiers` + `Code` pair the `global-hotkey` crate expects. Shared by
//! every backend that wraps `global-hotkey` (currently the unified
//! `global_hotkey` backend on X11 and Windows).

use global_hotkey::hotkey::{Code, Modifiers};

use crate::hotkey::HotkeyError;

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
