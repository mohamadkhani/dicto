//! Selection / clipboard reading for quick-translate.
//!
//! Strategy:
//! 1. Try PRIMARY selection (X11 PRIMARY / wlroots primary selection).
//!    This lets the user just select text and press the hotkey, no copy needed.
//! 2. Fall back to the regular CLIPBOARD (Ctrl+C) — works everywhere
//!    including GNOME/KDE Wayland where primary selection is unavailable.
//!
//! Uses the `arboard` crate which abstracts X11, Wayland (wl-clipboard /
//! wlr-data-control), macOS, and Windows.

use std::time::Instant;

use arboard::Clipboard;
#[cfg(target_os = "linux")]
use arboard::LinuxClipboardKind;
use tracing::debug;

/// Which source the selected text was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionSource {
    /// PRIMARY selection (X11 PRIMARY / wlroots primary selection).
    /// Text selected with the mouse — no explicit copy needed.
    Primary,
    /// Standard system clipboard (Ctrl+C / Ctrl+V buffer).
    Clipboard,
}

/// Errors from reading the selection.
#[derive(Debug, thiserror::Error)]
pub enum SelectionError {
    #[error("clipboard unavailable: {0}")]
    Unavailable(String),
    #[error("selection is empty")]
    Empty,
    #[error("selection too long: {} chars (max 10000)", .0.len())]
    TooLong(String),
}

/// Maximum selection length we'll process.
pub const MAX_SELECTION_LENGTH: usize = 10_000;

/// Read the currently selected text.
///
/// Tries primary selection first, then falls back to the regular clipboard.
/// Returns the text and which source it came from.
///
/// On GNOME/KDE Wayland, primary selection is not available — callers
/// should guide the user to copy text first (Ctrl+C).
pub fn read_selected_text() -> Result<(String, SelectionSource), SelectionError> {
    let start = Instant::now();

    // Try primary selection first.
    match read_primary() {
        Ok(text) if !text.trim().is_empty() => {
            let trimmed = text.trim().to_string();
            if trimmed.len() > MAX_SELECTION_LENGTH {
                return Err(SelectionError::TooLong(trimmed));
            }
            debug!(
                source = "primary",
                len = trimmed.len(),
                elapsed_ms = start.elapsed().as_millis(),
                "read selection"
            );
            return Ok((trimmed, SelectionSource::Primary));
        }
        Ok(_) => {
            debug!("primary selection is empty, trying clipboard");
        }
        Err(e) => {
            debug!(error = %e, "primary selection unavailable, trying clipboard");
        }
    }

    // Fall back to regular clipboard.
    let text = read_clipboard()?;
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Err(SelectionError::Empty);
    }
    if trimmed.len() > MAX_SELECTION_LENGTH {
        return Err(SelectionError::TooLong(trimmed));
    }

    debug!(
        source = "clipboard",
        len = trimmed.len(),
        elapsed_ms = start.elapsed().as_millis(),
        "read selection"
    );
    Ok((trimmed, SelectionSource::Clipboard))
}

/// Returns true if the text looks like a single word (no whitespace, not too long).
#[allow(dead_code)]
pub fn is_single_word(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && trimmed.len() <= 100 && !trimmed.contains(char::is_whitespace)
}

// --- Platform-specific helpers ---

#[cfg(target_os = "linux")]
fn read_primary() -> Result<String, SelectionError> {
    use arboard::GetExtLinux;

    let mut clipboard = Clipboard::new().map_err(|e| SelectionError::Unavailable(e.to_string()))?;

    // On Wayland with wlr-data-control, this reads PRIMARY.
    // On GNOME/KDE Wayland, this fails — we fall back to clipboard.
    clipboard
        .get()
        .clipboard(LinuxClipboardKind::Primary)
        .text()
        .map_err(|e| SelectionError::Unavailable(e.to_string()))
}

#[cfg(not(target_os = "linux"))]
fn read_primary() -> Result<String, SelectionError> {
    // On non-Linux platforms there's no concept of PRIMARY selection.
    Err(SelectionError::Unavailable(
        "primary selection not supported on this platform".to_string(),
    ))
}

fn read_clipboard() -> Result<String, SelectionError> {
    let mut clipboard = Clipboard::new().map_err(|e| SelectionError::Unavailable(e.to_string()))?;

    clipboard
        .get_text()
        .map_err(|e| SelectionError::Unavailable(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_single_word() {
        assert!(is_single_word("hello"));
        assert!(is_single_word("hello-world"));
        assert!(is_single_word("café"));
        assert!(is_single_word("123"));
        assert!(!is_single_word("hello world"));
        assert!(!is_single_word("hello\tworld"));
        assert!(!is_single_word("hello\nworld"));
        assert!(!is_single_word(""));
        assert!(!is_single_word("   "));
        // Very long single word is not considered a word
        let long = "a".repeat(101);
        assert!(!is_single_word(&long));
    }
}
