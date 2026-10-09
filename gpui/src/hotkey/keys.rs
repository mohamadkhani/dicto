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

/// Convert a user-facing hotkey string like `"Ctrl+Alt+D"` into a GTK
/// accelerator — the format both the XDG GlobalShortcuts portal
/// (`preferred_trigger`) and GNOME's custom-keybinding `binding` field
/// expect, e.g. `"<Control><Alt>d"`.
///
/// Returns `Err` for the same inputs [`parse_hotkey`] rejects.
pub fn to_gtk_accel(s: &str) -> Result<String, HotkeyError> {
    let mut out = String::new();
    let mut key: Option<String> = None;

    for part in s.split('+') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let modifier = match part.to_lowercase().as_str() {
            "ctrl" | "control" => Some("<Control>"),
            "alt" | "opt" | "option" => Some("<Alt>"),
            "shift" => Some("<Shift>"),
            "super" | "win" | "cmd" | "command" | "meta" => Some("<Super>"),
            _ => None,
        };
        match modifier {
            Some(tag) => {
                if key.is_some() {
                    // Modifier after the key: "Ctrl+D+Alt" is malformed.
                    return Err(HotkeyError::InvalidHotkey(format!(
                        "modifier '{part}' after key in '{s}'"
                    )));
                }
                out.push_str(tag);
            }
            None => {
                if key.is_some() {
                    return Err(HotkeyError::InvalidHotkey(format!(
                        "multiple keys in '{s}'"
                    )));
                }
                key = Some(accel_key_name(part, s)?);
            }
        }
    }

    let key = key.ok_or_else(|| HotkeyError::InvalidHotkey(format!("no key found in '{s}'")))?;
    out.push_str(&key);
    Ok(out)
}

/// Normalize the key part of an accelerator: single characters stay as-is
/// (lowercased, per GTK convention), named keys get their canonical spelling.
fn accel_key_name(name: &str, original: &str) -> Result<String, HotkeyError> {
    let lower = name.to_lowercase();
    let mut chars = lower.chars();
    if let (Some(first), None) = (chars.next(), chars.next()) {
        // Single character: letters/digits/punctuation are valid GTK keys.
        return Ok(first.to_string());
    }
    match lower.as_str() {
        "space" => Ok("space".into()),
        "enter" | "return" => Ok("Return".into()),
        "escape" | "esc" => Ok("Escape".into()),
        "tab" => Ok("Tab".into()),
        f if f.len() >= 2 && f.starts_with('f') && f[1..].chars().all(|c| c.is_ascii_digit()) => {
            Ok(format!("F{}", &f[1..]))
        }
        _ => Err(HotkeyError::InvalidHotkey(format!(
            "unsupported key: '{name}' in '{original}'"
        ))),
    }
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

    #[test]
    fn test_accel_ctrl_alt_d() {
        assert_eq!(to_gtk_accel("Ctrl+Alt+D").unwrap(), "<Control><Alt>d");
    }

    #[test]
    fn test_accel_order_and_super() {
        assert_eq!(
            to_gtk_accel("Super+Shift+Space").unwrap(),
            "<Super><Shift>space"
        );
        assert_eq!(to_gtk_accel("Ctrl+Alt+F5").unwrap(), "<Control><Alt>F5");
        assert_eq!(to_gtk_accel("Alt+1").unwrap(), "<Alt>1");
    }

    #[test]
    fn test_accel_invalid() {
        assert!(to_gtk_accel("Ctrl+Alt+").is_err());
        assert!(to_gtk_accel("just_a_key").is_err());
        assert!(to_gtk_accel("Ctrl+D+Alt").is_err());
    }
}
