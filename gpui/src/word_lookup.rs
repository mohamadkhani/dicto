//! Word Lookup feature orchestrator.
//!
//! The twin of [`crate::quick_translate`]: instead of an LLM translation it
//! queries the local MDict dictionaries and shows the definition in the same
//! popup window. Wires together:
//! - Global hotkey manager (second registration, `QUICK_LOOKUP_ID`)
//! - Selection reader (clipboard / primary selection)
//! - Dictionary registry (`mdict_rs::query::query_all` + HTML parsing)
//! - The shared popup window (state rendered by `translate_popup`)
//!
//! Lookups run fully locally — no network, no API key.

use mdict_rs::settings::WordLookupSettings;
use tracing::{info, warn};

use crate::hotkey::{HotkeyError, HotkeyManager, QUICK_LOOKUP_ID, create_hotkey_manager};
use crate::selection::{SelectionError, SelectionSource, read_selected_text};
use crate::state::DictResult;

/// Current state of the word-lookup popup.
#[derive(Debug, Clone)]
pub enum LookupStatus {
    /// No popup is shown.
    Hidden,
    /// Popup is visible with the given state.
    Visible(LookupState),
}

/// State of the word-lookup popup. Mirrors the `PopupState` translate
/// variants; the popup maps these onto its lookup UI.
#[derive(Debug, Clone)]
pub enum LookupState {
    /// Querying the dictionaries in the background.
    Loading { word: String },
    /// At least one dictionary had a hit. `active` is the selected tab.
    Ready {
        word: String,
        results: Vec<DictResult>,
        active: usize,
        /// Fuzzy near-matches shown as horizontally scrollable chips.
        related: Vec<String>,
    },
    /// No dictionary had a hit — the popup offers "Translate with AI"
    /// plus the near-miss suggestions.
    NotFound {
        word: String,
        /// Fuzzy near-matches shown as horizontally scrollable chips.
        related: Vec<String>,
    },
    /// The feature is turned off — the popup offers a one-click Enable.
    Disabled,
    /// The selection could not be read (same hint as the translate popup).
    Error { message: String },
}

impl LookupState {
    /// The looked-up word — present only in the word-carrying variants.
    pub fn word(&self) -> Option<&str> {
        match self {
            LookupState::Loading { word }
            | LookupState::Ready { word, .. }
            | LookupState::NotFound { word, .. } => Some(word),
            LookupState::Disabled | LookupState::Error { .. } => None,
        }
    }
}

/// Word Lookup engine.
///
/// Owns the lookup hotkey registration and the popup state. Created once at
/// app startup when the feature is enabled; the hotkey is re-registered when
/// settings change.
pub struct WordLookupEngine {
    hotkey_manager: Option<Box<dyn HotkeyManager>>,
    settings: WordLookupSettings,
    status: LookupStatus,
}

impl WordLookupEngine {
    /// Create a new engine with the given settings.
    pub fn new(settings: WordLookupSettings) -> Self {
        let hotkey_manager = if settings.enabled {
            match create_and_register(&settings) {
                Ok(mgr) => Some(mgr),
                Err(e) => {
                    warn!(error = %e, "word lookup: failed to register hotkey");
                    None
                }
            }
        } else {
            None
        };

        Self {
            hotkey_manager,
            settings,
            status: LookupStatus::Hidden,
        }
    }

    /// Update settings and re-register the hotkey when it changed.
    pub fn update_settings(&mut self, new_settings: WordLookupSettings) {
        let old_enabled = self.settings.enabled;
        let old_hotkey = self.settings.hotkey.clone();
        self.settings = new_settings;

        if old_enabled != self.settings.enabled || old_hotkey != self.settings.hotkey {
            // Drop existing manager first (unregisters on drop).
            self.hotkey_manager = None;
            if self.settings.enabled {
                match create_and_register(&self.settings) {
                    Ok(mgr) => self.hotkey_manager = Some(mgr),
                    Err(e) => {
                        warn!(error = %e, "word lookup: failed to register hotkey after settings change");
                    }
                }
            }
        }
    }

    /// Poll for hotkey events. Returns `true` if the lookup hotkey fired.
    /// Drains only — the popup opens via [`trigger_lookup`](Self::trigger_lookup).
    pub fn poll(&mut self) -> bool {
        let mut fired = false;
        if let Some(manager) = self.hotkey_manager.as_ref() {
            while let Some(id) = manager.try_recv() {
                if id == QUICK_LOOKUP_ID {
                    fired = true;
                }
            }
        }
        fired
    }

    /// Open the popup with the currently-selected word (hotkey / tray /
    /// IPC trigger). When the feature is disabled the popup shows an
    /// enable hint instead of failing silently. Returns the job to run on
    /// the background executor, or `None` when no query is pending.
    pub fn trigger_lookup(&mut self) -> Option<LookupJob> {
        // Feature off: never read the selection or run a query — show how
        // to turn it on instead (the popup carries an Enable button).
        if !self.settings.enabled {
            info!("word lookup: triggered while disabled — showing hint popup");
            self.status = LookupStatus::Visible(LookupState::Disabled);
            return None;
        }

        let (text, source) = match read_selected_text() {
            Ok(t) => t,
            Err(SelectionError::TooLong(text)) => {
                self.status = LookupStatus::Visible(LookupState::Error {
                    message: format!(
                        "Selection is {} characters — the lookup limit is {}. \
                         Select a single word and try again.",
                        text.chars().count(),
                        crate::selection::MAX_SELECTION_LENGTH
                    ),
                });
                return None;
            }
            Err(e) => {
                warn!(error = %e, "word lookup: failed to read selection");
                self.status = LookupStatus::Visible(LookupState::Error {
                    message: format!(
                        "Could not read the selected text: {e}\n\n\
                         Tip: Select a word and copy it (Ctrl+C), then try again."
                    ),
                });
                return None;
            }
        };

        info!(
            source = match source {
                SelectionSource::Primary => "primary",
                SelectionSource::Clipboard => "clipboard",
            },
            "word lookup: got selection"
        );

        Some(self.start(text))
    }

    /// Start a lookup for already-captured text (smart dispatch: the
    /// translate hotkey fired and the selection turned out to be a single
    /// word). Shows `Loading` and returns the job to spawn.
    pub fn lookup_text(&mut self, text: String) -> LookupJob {
        self.start(text)
    }

    /// Shared path for both trigger routes: normalize, show `Loading`.
    /// Telemetry fires here — only real lookups (not disabled/failure
    /// popups) are counted.
    fn start(&mut self, text: String) -> LookupJob {
        dicto_telemetry::get().track(dicto_telemetry::Event::LookupPerformed {
            source: dicto_telemetry::LookupSource::QuickPopup,
        });
        let word = normalize_word(&text);
        info!(word = %word, "word lookup: triggered");
        self.status = LookupStatus::Visible(LookupState::Loading { word: word.clone() });
        LookupJob { word }
    }

    /// Apply the outcome of a background lookup, moving the popup from
    /// `Loading` to `Ready` or `NotFound`. Called on the main thread once
    /// the background executor finishes.
    pub fn apply_result(&mut self, outcome: LookupOutcome) {
        let LookupOutcome {
            word,
            results,
            related,
        } = outcome;
        if results.is_empty() {
            info!(word = %word, "word lookup: no dictionary hits");
            self.status = LookupStatus::Visible(LookupState::NotFound { word, related });
        } else {
            info!(word = %word, dicts = results.len(), "word lookup: completed");
            self.status = LookupStatus::Visible(LookupState::Ready {
                word,
                active: 0,
                results,
                related,
            });
        }
    }

    /// Switch the active dictionary tab in the `Ready` state.
    pub fn set_active(&mut self, index: usize) {
        if let LookupStatus::Visible(LookupState::Ready {
            active, results, ..
        }) = &mut self.status
        {
            *active = index.min(results.len().saturating_sub(1));
        }
    }

    /// Current popup status.
    pub fn status(&self) -> &LookupStatus {
        &self.status
    }

    /// Hide the popup.
    pub fn hide_popup(&mut self) {
        self.status = LookupStatus::Hidden;
    }

    /// Hand the parsed results and word to the caller ("Open in Dicto"):
    /// moves the results out of the engine so the main window's detail
    /// panel can own them, and hides the popup. Only a `Ready` state has
    /// results to hand over.
    pub fn take_results(&mut self) -> Option<(String, Vec<DictResult>)> {
        match std::mem::replace(&mut self.status, LookupStatus::Hidden) {
            LookupStatus::Visible(LookupState::Ready { word, results, .. }) => {
                Some((word, results))
            }
            _ => None,
        }
    }

    /// Get the hotkey backend name.
    pub fn backend_name(&self) -> &'static str {
        self.hotkey_manager
            .as_ref()
            .map(|m| m.backend_name())
            .unwrap_or("none")
    }

    /// Whether a popup is currently visible.
    pub fn is_visible(&self) -> bool {
        matches!(self.status, LookupStatus::Visible(_))
    }
}

/// A self-contained lookup request ready to run on a background thread.
/// Carries only the word — everything it needs is local.
pub struct LookupJob {
    word: String,
}

impl LookupJob {
    /// Run the blocking dictionary query + HTML parsing and package the
    /// result for the main thread. Mirrors the main window's background
    /// search (`DictApp::lookup_word`).
    pub fn run(self) -> LookupOutcome {
        let results = mdict_rs::query::query_all(&self.word)
            .into_iter()
            .map(|hit| {
                let blocks = crate::html::parse_styled(&hit.definition, &hit.stem);
                let audio = crate::html::first_sound_path(&blocks);
                DictResult {
                    short_name: hit.short_name,
                    blocks,
                    audio,
                }
            })
            .collect();
        // Fuzzy near-matches for the horizontal chips row — useful both on
        // a miss ("did you mean") and to hop between word forms.
        let related = mdict_rs::query::related_words(&self.word, RELATED_LIMIT);
        LookupOutcome {
            word: self.word,
            results,
            related,
        }
    }
}

/// How many fuzzy near-match chips the popup offers.
pub const RELATED_LIMIT: usize = 14;

/// The completed lookup: the word plus one parsed entry per dictionary
/// that had a hit (empty = not found), and the fuzzy near-matches for the
/// selectable chips row.
pub struct LookupOutcome {
    pub word: String,
    pub results: Vec<DictResult>,
    pub related: Vec<String>,
}

/// Clean up a selection so it hits the dictionary FST: trim whitespace and
/// surrounding punctuation / quotes / brackets. Internal whitespace is kept
/// — multi-word phrase entries are legitimate MDict headwords.
fn normalize_word(text: &str) -> String {
    const STRIP: &[char] = &[
        // ASCII punctuation commonly glued to words in prose
        '.', ',', ';', ':', '!', '?', '"', '\'', '(', ')', '[', ']', '{', '}', '<', '>', '*', '_',
        '…', '—', '–', '«', '»', '“', '”', '‘', '’', '¡', '¿', '،', '؛',
    ];
    let trimmed = text.trim().trim_matches(STRIP).trim();
    if trimmed.is_empty() {
        // Pathological (e.g. selection was only quotes) — use raw text.
        text.trim().to_string()
    } else {
        trimmed.to_string()
    }
}

/// Create a hotkey manager and register the word-lookup hotkey.
fn create_and_register(
    settings: &WordLookupSettings,
) -> Result<Box<dyn HotkeyManager>, HotkeyError> {
    let manager = create_hotkey_manager();
    manager.register(QUICK_LOOKUP_ID, &settings.hotkey)?;
    Ok(manager)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_surrounding_punctuation() {
        assert_eq!(normalize_word("  “hello,”  "), "hello");
        assert_eq!(normalize_word("(world)"), "world");
        assert_eq!(normalize_word("don't…"), "don't");
        assert_eq!(normalize_word("«serendipity»"), "serendipity");
    }

    #[test]
    fn normalize_keeps_internal_spaces_and_punctuation() {
        assert_eq!(normalize_word("ice cream"), "ice cream");
        assert_eq!(normalize_word("give up on"), "give up on");
        assert_eq!(normalize_word("rock 'n' roll"), "rock 'n' roll");
    }

    #[test]
    fn normalize_falls_back_to_raw_text() {
        assert_eq!(normalize_word("\"\""), "\"\"");
    }

    /// Manual probe: PLAY a Speex-derived clip (ffmpeg transcode path) and
    /// then an MP3 clip (direct rodio path) back to back
    /// (`cargo test -p dicto probe_play_spx_vs_mp3 -- --ignored --nocapture`).
    /// If you hear only the MP3, the transcoded-WAV → rodio path is the
    /// silent one on this machine.
    #[test]
    #[ignore = "plays audio out loud; needs local dictionaries"]
    fn probe_play_spx_vs_mp3() {
        mdict_rs::registry::reload();
        let mut spx: Option<String> = None;
        let mut mp3: Option<String> = None;
        for hit in mdict_rs::query::query_all("wood") {
            let blocks = crate::html::parse_styled(&hit.definition, &hit.stem);
            if let Some(path) = crate::html::first_sound_path(&blocks) {
                if path.to_lowercase().ends_with(".spx") && spx.is_none() {
                    spx = Some(path.clone());
                }
                if path.to_lowercase().ends_with(".mp3") && mp3.is_none() {
                    mp3 = Some(path.clone());
                }
            }
        }
        for (kind, path) in [
            ("SPEEX (ffmpeg transcode)", spx.as_deref()),
            ("MP3 (direct rodio)", mp3.as_deref()),
        ] {
            let Some(path) = path else {
                println!("no {kind} clip found");
                continue;
            };
            println!("NOW PLAYING {kind}: {path}");
            crate::audio::play_resource(path);
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    }

    /// Manual probe: for each dictionary hit, print every CLICKABLE sound
    /// run IN ORDER with the text right before it
    /// (`cargo test -p dicto probe_pill_order -- --ignored --nocapture`).
    /// This shows which clip each pill plays and what it sits next to —
    /// a pill playing the WRONG word's clip means the entry data or the
    /// parser assigns links incorrectly.
    #[test]
    #[ignore = "needs local dictionaries and their indexes"]
    fn probe_pill_order() {
        mdict_rs::registry::reload();
        for word in ["wood", "woman"] {
            for hit in mdict_rs::query::query_all(word) {
                let blocks = crate::html::parse_styled(&hit.definition, &hit.stem);
                for (b, block) in blocks.iter().enumerate() {
                    let runs = match block {
                        crate::html::parser::Block::Paragraph { runs, .. }
                        | crate::html::parser::Block::Heading { runs, .. } => Some(runs),
                        crate::html::parser::Block::ListItem { content, .. } => Some(content),
                        _ => None,
                    };
                    let Some(runs) = runs else { continue };
                    for (r, run) in runs.iter().enumerate() {
                        if let Some(crate::html::parser::Link::Sound(path)) = &run.link {
                            let prev = runs
                                .iter()
                                .take(r)
                                .rev()
                                .find_map(|p| {
                                    if p.link.is_none() && !p.text.trim().is_empty() {
                                        Some(p.text.trim().to_string())
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or_default();
                            println!(
                                "{word} [{}] blk{b} run{r}: {}  (after: {:?})",
                                hit.short_name, path, prev
                            );
                        }
                    }
                }
            }
        }
    }

    /// Manual probe: for each dictionary hit, count CLICKABLE sound runs
    /// (what the renderer turns into speaker pills) vs plain image blocks
    /// (`cargo test -p dicto probe_pill_structure -- --ignored --nocapture`).
    /// Dictionaries that draw their speaker buttons as unlinked images
    /// produce `images` with no `sound_runs` — those "buttons" are
    /// decoration and can never play.
    #[test]
    #[ignore = "needs local dictionaries and their indexes"]
    fn probe_pill_structure() {
        mdict_rs::registry::reload();
        for word in ["wood", "word", "have", "previous", "and"] {
            for hit in mdict_rs::query::query_all(word) {
                let blocks = crate::html::parse_styled(&hit.definition, &hit.stem);
                let mut sound_runs = 0usize;
                let mut image_blocks = 0usize;
                let mut image_srcs: Vec<String> = Vec::new();
                for block in &blocks {
                    let runs = match block {
                        crate::html::parser::Block::Paragraph { runs, .. }
                        | crate::html::parser::Block::Heading { runs, .. } => Some(runs),
                        crate::html::parser::Block::ListItem { content, .. } => Some(content),
                        crate::html::parser::Block::Image(src) => {
                            image_blocks += 1;
                            image_srcs.push(src.to_string());
                            None
                        }
                        crate::html::parser::Block::Divider => None,
                    };
                    if let Some(runs) = runs {
                        for run in runs {
                            if matches!(run.link, Some(crate::html::parser::Link::Sound(_))) {
                                sound_runs += 1;
                            }
                        }
                    }
                }
                println!(
                    "{word} [{}] sound_runs={sound_runs} image_blocks={image_blocks} srcs={image_srcs:?}",
                    hit.short_name
                );
            }
        }
    }

    /// Manual probe: run against the local dictionaries and their indexes
    /// (`cargo test -p dicto probe_bundled_audio -- --ignored --nocapture`).
    /// Prints which dictionary hits carry a bundled `sound://` clip —
    /// evidence for the header dict-audio button (MDD playback).
    #[test]
    #[ignore = "needs local dictionaries and their indexes"]
    fn probe_bundled_audio_paths() {
        mdict_rs::registry::reload();
        for word in ["serendipity", "hello", "world", "test"] {
            for hit in mdict_rs::query::query_all(word) {
                let blocks = crate::html::parse_styled(&hit.definition, &hit.stem);
                if let Some(audio) = crate::html::first_sound_path(&blocks) {
                    println!("{word} [{}] audio={audio}", hit.short_name);
                }
            }
        }
    }
}
