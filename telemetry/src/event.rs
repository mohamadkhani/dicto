//! Event schema for Dicto telemetry.
//!
//! Events are an enum so event names are compile-time-checked: a typo like
//! `"ap_started"` is impossible. The string payload (`props` / `systemProps`
//! in Aptabase's wire format) is built inside [`crate::aptabase`].

/// An analytics event. Built ergonomically and validated by construction —
/// call sites pass `Event::LookupPerformed`, not a string key.
///
/// Counters that the dashboard might want (e.g. "total lookups per
/// installation") are derived server-side by counting events; nothing about
/// the *content* of a lookup (the word, the dictionary) is ever recorded.
pub enum Event {
    /// App launched and the GPUI process started.
    AppStarted,
    /// A dictionary window was opened (at startup, or "Show" from the tray).
    WindowOpened,
    /// The dictionary window was closed/hidden by the user.
    WindowClosed,
    /// The user submitted a word lookup. No word text is attached — privacy.
    /// The `source` property distinguishes passive auto-preview (the debounced
    /// first-hit preview while typing) from explicit user intent (click or
    /// keyboard navigation).
    LookupPerformed { source: LookupSource },
    /// The user clicked the pronunciation play button (intent).
    PronunciationPlayed,
    /// A pronunciation playback actually failed.
    PronunciationPlaybackFailed {
        reason: PlaybackFailureReason,
    },
    /// A dictionary was imported. `count` is the total number of enabled
    /// dictionaries after the import — never the dictionary name/path.
    /// `duration_ms` is the wall-clock time of the whole import batch
    /// (copy + index, across every file in the batch), in milliseconds.
    /// `size_mb` is the total bytes copied, rounded to the nearest MB —
    /// rounded so the precise size (a weak proxy for *which* dictionary)
    /// doesn't leak, while preserving the perf-correlation use case.
    DictionaryImported {
        count: usize,
        duration_ms: u64,
        size_mb: u64,
    },
    /// User saved changes via the settings dialog. `change` records which
    /// category of settings was saved. The telemetry opt-in toggle does
    /// NOT fire this (consent changes are not a "settings change" in the
    /// dashboard sense).
    SettingsChanged { change: SettingsChange },
    /// The settings dialog was opened. `source` records how the user got
    /// there (gear button vs. tray menu).
    SettingsOpened { source: SettingsSource },
    /// The user selected a tab in the settings dialog. The tab name is
    /// compile-time-checked via [`SettingsTab`].
    SettingsTabSelected { tab: SettingsTab },
    /// An error the user would notice (failed indexing or failed import).
    /// `message` is truncated to 180 chars and path prefixes stripped by the
    /// client before sending.
    ErrorOccurred {
        kind: ErrorKind,
        message: String,
    },
    /// The user triggered quick translate via hotkey or tray menu.
    /// `source` records how it was triggered; no text content is recorded.
    QuickTranslateTriggered { source: QuickTranslateSource },
    /// A quick translate request completed successfully or failed.
    /// `provider` is the LLM provider name; no translated text is recorded.
    QuickTranslateCompleted {
        provider: &'static str,
        success: bool,
        duration_ms: u64,
    },
}

/// How the quick-translate feature was triggered.
pub enum QuickTranslateSource {
    /// Global hotkey press.
    Hotkey,
    /// Tray menu item.
    TrayMenu,
}

impl QuickTranslateSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hotkey => "hotkey",
            Self::TrayMenu => "tray_menu",
        }
    }
}

/// Why a pronunciation playback failed. Actionable and non-sensitive.
pub enum PlaybackFailureReason {
    /// No default audio output device available.
    NoDevice,
    /// rodio couldn't create a sink.
    SinkFailed,
    /// The requested audio resource wasn't found in the dictionary.
    ResourceNotFound,
    /// `ffmpeg` isn't installed (needed for Speex clips).
    FfmpegMissing,
    /// `ffmpeg` ran but exited non-zero.
    FfmpegFailed,
    /// rodio rejected the decoded/cached buffer.
    DecoderRejected,
}

impl PlaybackFailureReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NoDevice => "no_device",
            Self::SinkFailed => "sink_failed",
            Self::ResourceNotFound => "resource_not_found",
            Self::FfmpegMissing => "ffmpeg_missing",
            Self::FfmpegFailed => "ffmpeg_failed",
            Self::DecoderRejected => "decoder_rejected",
        }
    }
}

/// Category of a user-visible error. Kept narrow on purpose: only
/// [`ErrorKind::Indexing`] and [`ErrorKind::Import`] are tracked, since those
/// are the actionable "my dictionary didn't load" failures. Audio failures
/// have their own [`Event::PronunciationPlaybackFailed`] event and generic
/// I/O errors are intentionally excluded.
pub enum ErrorKind {
    Indexing,
    Import,
}

/// How a `LookupPerformed` was triggered. Distinguishes passive interest
/// (the debounced first-hit preview while typing) from explicit user intent
/// (mouse click or keyboard navigation in the suggestion list).
pub enum LookupSource {
    /// Debounced auto-preview of the first suggestion while typing (3+ chars).
    /// No user submit — the definition pane was populated passively.
    AutoPreview,
    /// User clicked a suggestion in the word list.
    Click,
    /// User navigated with Up/Down arrow keys and confirmed the selection.
    Keyboard,
}

impl LookupSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AutoPreview => "auto_preview",
            Self::Click => "click",
            Self::Keyboard => "keyboard",
        }
    }
}

/// How the settings dialog was opened.
pub enum SettingsSource {
    /// The gear button in the search bar.
    GearButton,
    /// A "Settings" item in the tray menu (if wired).
    TrayMenu,
}

impl SettingsSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GearButton => "gear_button",
            Self::TrayMenu => "tray_menu",
        }
    }
}

/// A tab in the settings dialog. Compile-time-checked so a tab rename
/// surfaces at the call site rather than silently dropping the event.
pub enum SettingsTab {
    Dictionaries,
    Import,
    Download,
    QuickTranslate,
    Telemetry,
    About,
}

impl SettingsTab {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dictionaries => "dictionaries",
            Self::Import => "import",
            Self::Download => "download",
            Self::QuickTranslate => "quick_translate",
            Self::Telemetry => "telemetry",
            Self::About => "about",
        }
    }
}

/// Which category of settings was saved. Today only the dictionary list
/// save (the gear-button "Save") fires [`Event::SettingsChanged`]; the
/// enum is here so future settings categories (theme, font, shortcuts…)
/// add a variant rather than overloading a string.
pub enum SettingsChange {
    /// The dictionary list (enables/toggles, reordering, short-names).
    DictionaryList,
}

impl SettingsChange {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DictionaryList => "dictionary_list",
        }
    }
}

impl ErrorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Indexing => "indexing",
            Self::Import => "import",
        }
    }
}

/// Maximum length of an error message before truncation. Aptabase's ingestion
/// truncates string property values at 180 chars server-side; we do it
/// client-side too so the limit is visible and predictable.
pub const MAX_MESSAGE_LEN: usize = 180;

/// Sanitize an error message for sending: truncate to [`MAX_MESSAGE_LEN`]
/// chars and strip any absolute path prefixes that could leak usernames or
/// filesystem layout (e.g. `/home/mohamad/...`, `C:\Users\mohamad\...`).
///
/// Examples:
/// - `"/home/mohamad/Downloads/foo.mdx: bad header"` → `": bad header"`
/// - `"C:\\Users\\mohamad\\foo.mdx corrupt"` → `"corrupt"`
pub fn sanitize_message(raw: &str) -> String {
    let trimmed = strip_path_prefixes(raw).trim_start().to_string();
    truncate_chars(&trimmed, MAX_MESSAGE_LEN)
}

/// Replace any leading run of path-like prefix segments with nothing. Matches
/// both POSIX (`/...`) and Windows (`C:\...`, `C:/...`) absolute paths, plus
/// a tilde home (`~/...`), greedily up to and including the final path
/// separator. Anything that isn't part of an absolute path prefix is left
/// untouched.
fn strip_path_prefixes(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return String::new();
    }

    // POSIX absolute or home-relative path: starts with '/' or '~'.
    if bytes[0] == b'/' || bytes[0] == b'~' {
        return strip_to_last_sep(s);
    }

    // Windows absolute path: drive letter + ':' + ('\' or '/').
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return strip_to_last_sep(s);
    }

    s.to_string()
}

/// Given a string known to start with an absolute path, drop everything up to
/// and including the last of the given separators, then drop the filename
/// portion as well, leaving only the error message (if any).
///
/// Patterns handled:
/// - "/path/to/file.mdx: error message" → ": error message"
/// - "/path/to/file.mdx corrupt" → "corrupt"
fn strip_to_last_sep(s: &str) -> String {
    let bytes = s.as_bytes();

    // Step 1: Find where the path+filename portion ends
    // If there's a space, treat everything before it as the path+filename
    let first_space = bytes.iter().position(|&b| b == b' ');

    // Step 2: Look for ": " (colon-space) pattern which typically separates
    // filename from error message
    let colon_space = s.find(": ");

    // Step 3: If we have a colon-space, return everything from there
    if let Some(i) = colon_space {
        return s[i..].to_string();
    }

    // Step 4: No colon-space, but there's a space — return from first space
    if let Some(i) = first_space {
        return s[i..].to_string();
    }

    // Step 5: No space, no colon-space — the entire string is path/filename,
    // return empty
    String::new()
}

/// Truncate to at most `max` Unicode scalar values (not bytes), so we never
/// split a multibyte character.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max).collect();
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_posix_home_path() {
        assert_eq!(
            sanitize_message("/home/mohamad/Downloads/foo.mdx: bad header"),
            ": bad header"
        );
    }

    #[test]
    fn strips_posix_root_path() {
        assert_eq!(
            sanitize_message("/usr/share/dict/oed.mdx corrupt"),
            "corrupt"
        );
    }

    #[test]
    fn strips_tilde_path() {
        assert_eq!(sanitize_message("~/dicts/x.mdx failed"), "failed");
    }

    #[test]
    fn strips_windows_backslash_path() {
        assert_eq!(
            sanitize_message("C:\\Users\\mohamad\\foo.mdx corrupt"),
            "corrupt"
        );
    }

    #[test]
    fn strips_windows_forward_slash_path() {
        assert_eq!(
            sanitize_message("D:/dicts/oed.mdx: invalid header"),
            ": invalid header"
        );
    }

    #[test]
    fn leaves_relative_text_untouched() {
        assert_eq!(sanitize_message("bad header in file"), "bad header in file");
    }

    #[test]
    fn truncates_long_message_by_chars() {
        let long: String = "é".repeat(200); // multibyte, 200 chars
        let out = sanitize_message(&long);
        assert_eq!(out.chars().count(), MAX_MESSAGE_LEN);
    }

    #[test]
    fn empty_message_stays_empty() {
        assert_eq!(sanitize_message(""), "");
    }
}
