use std::path::PathBuf;

use mdict_rs::settings::{DictEntry, QuickTranslateSettings};

use crate::catalog::DictCatalogEntry;
use crate::html::Block;
use crate::quick_translate::QuickTranslateEngine;

#[derive(Debug, Clone)]
pub struct DictResult {
    /// Short name for tab labels.
    pub short_name: String,
    pub blocks: Vec<Block>,
}

pub struct ImportFile {
    pub path: PathBuf,
    pub name: String,
    pub status: ImportStatus,
}

pub enum ImportStatus {
    Pending,
    Copying,
    Indexing,
    Done,
    Error(String),
}

#[derive(Debug, Clone)]
pub enum CatalogState {
    Idle,
    Loading,
    Loaded {
        base_url: String,
        entries: Vec<DictCatalogEntry>,
    },
    Error(String),
}

#[derive(Debug, Clone)]
pub enum DictDownloadStatus {
    Idle,
    Downloading {
        progress: f32,
        speed: String,
        current_file: String,
    },
    Done,
    Error(String),
}

pub struct DictState {
    /// Scroll handle for the word list panel, used to auto-scroll to the
    /// selected item during keyboard navigation.
    pub word_list_scroll: gpui::ScrollHandle,
    /// One entry per dictionary that had a hit for the current word, in
    /// settings order. Parsed blocks are cached so the detail panel
    /// never re-parses HTML on render.
    pub results: Vec<DictResult>,
    pub active_result: usize,

    pub result_word: Option<String>,
    pub is_searching: bool,
    pub suggestions: Vec<String>,
    pub selected_suggestion: Option<usize>,

    /// Working copy of the dictionary list used by the settings dialog.
    /// Edits mutate this in place; Save persists it, Cancel reloads from disk.
    pub dictionaries: Vec<DictEntry>,

    /// Background-indexing progress. `indexing_total == 0` means idle.
    pub indexing_total: usize,
    pub indexing_done: usize,
    pub indexing_current: Option<String>,

    /// True when the dicts directory was empty at startup — shows import modal.
    pub show_import_modal: bool,
    /// Files being imported via the init/settings modal.
    pub import_files: Vec<ImportFile>,

    /// Active tab in the settings dialog: 0 = Dictionaries, 1 = Import, 2 = Download.
    pub settings_active_tab: usize,

    pub catalog: CatalogState,
    pub download_status: DictDownloadStatus,
    pub download_active_id: Option<String>,
    pub import_modal_tab: usize,

    /// Quick Translate settings snapshot (loaded from disk, editable in UI).
    pub quick_translate: QuickTranslateSettings,

    /// Hotkey backend identifier — "x11", "tray_menu", or "none".
    pub hotkey_backend: String,

    /// Quick Translate engine (hotkey + translator + popup state).
    ///
    /// Stored as an option so we can lazily initialize or replace it
    /// when settings change.
    pub quick_translate_engine: Option<QuickTranslateEngine>,

    /// TTS playback controllers — one per Speak slot so the source and the
    /// translation can play independently without their controls/state mixing.
    /// Each owns its rodio stream + sink so clips can be paused/seeked/replayed
    /// without re-synthesizing. State is observable via `snapshot()`; the popup
    /// polls `poll_progress()` on a timer to drive each seek bar.
    pub playback_source: crate::playback::PlaybackController,
    pub playback_translation: crate::playback::PlaybackController,

    /// Editable text-input states for the Quick Translate settings fields.
    /// Lazily created on first render of the settings tab and persisted so
    /// focus/cursor survives re-renders. `None` until the tab is first shown.
    pub qt_api_key_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    pub qt_base_url_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    pub qt_model_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    pub qt_target_lang_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    /// TTS settings input fields (lazily created, same pattern as above).
    pub qt_tts_api_key_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    #[allow(dead_code)]
    pub qt_tts_base_url_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    #[allow(dead_code)]
    pub qt_tts_model_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    #[allow(dead_code)]
    pub qt_tts_voice_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    /// True once we've seeded the input states from loaded settings, so we
    /// don't clobber the user's in-progress edits on every re-render.
    pub qt_inputs_seeded: bool,

    /// Handle to the currently-open Quick Translate popup window, if any.
    /// Kept so we can close it before opening a new one on a fresh trigger.
    /// The popup's root view is a `gpui_component::Root` wrapping the
    /// `TranslatePopupView` — the Original text editor (`Input`) requires a
    /// `Root` at the window root (it registers itself as the focused input).
    pub qt_popup_window: Option<gpui::WindowHandle<gpui_component::Root>>,
    /// Set while a tokenless re-trigger is replacing the popup window: the
    /// window-closed observer must NOT hide the popup engine for this
    /// programmatic close (the poll loop reopens the window next tick).
    pub qt_replace_pending: bool,
}

impl DictState {
    pub fn new() -> Self {
        let settings = mdict_rs::settings::current();
        let qt_settings = settings.quick_translate.clone();
        let engine = if qt_settings.enabled {
            Some(QuickTranslateEngine::new(qt_settings.clone()))
        } else {
            None
        };
        let backend = engine
            .as_ref()
            .map(|e| e.backend_name().to_string())
            .unwrap_or_else(|| "none".to_string());

        Self {
            word_list_scroll: gpui::ScrollHandle::new(),
            results: Vec::new(),
            active_result: 0,
            result_word: None,
            is_searching: false,
            suggestions: Vec::new(),
            selected_suggestion: None,
            dictionaries: settings.dictionaries,
            indexing_total: 0,
            indexing_done: 0,
            indexing_current: None,
            show_import_modal: mdict_rs::config::discover_mdx_files().is_empty(),
            import_files: Vec::new(),
            settings_active_tab: 0,
            catalog: CatalogState::Idle,
            download_status: DictDownloadStatus::Idle,
            download_active_id: None,
            import_modal_tab: 0,
            quick_translate: qt_settings,
            hotkey_backend: backend,
            quick_translate_engine: engine,
            qt_api_key_input: None,
            qt_base_url_input: None,
            qt_model_input: None,
            qt_target_lang_input: None,
            qt_tts_api_key_input: None,
            qt_tts_base_url_input: None,
            qt_tts_model_input: None,
            qt_tts_voice_input: None,
            qt_inputs_seeded: false,
            qt_popup_window: None,
            qt_replace_pending: false,
            playback_source: crate::playback::PlaybackController::default(),
            playback_translation: crate::playback::PlaybackController::default(),
        }
    }

    /// Reload the hotkey registration after settings change.
    pub fn reload_hotkey(&mut self, _cx: &mut gpui::App) {
        let settings = self.quick_translate.clone();

        if let Some(engine) = self.quick_translate_engine.as_mut() {
            engine.update_settings(settings);
            self.hotkey_backend = engine.backend_name().to_string();
        } else if settings.enabled {
            let engine = QuickTranslateEngine::new(settings);
            self.hotkey_backend = engine.backend_name().to_string();
            self.quick_translate_engine = Some(engine);
        }
    }

    /// Reload the translator after settings change.
    pub fn reload_translator(&mut self, _cx: &mut gpui::App) {
        let settings = self.quick_translate.clone();

        if let Some(engine) = self.quick_translate_engine.as_mut() {
            engine.update_settings(settings);
        } else if settings.enabled {
            self.quick_translate_engine = Some(QuickTranslateEngine::new(settings));
        }
    }

    /// Persist quick_translate settings to disk along with the full settings.
    pub fn save_settings(&mut self, _cx: &mut gpui::App) {
        let mut current = mdict_rs::settings::current();
        current.dictionaries = self.dictionaries.clone();
        current.quick_translate = self.quick_translate.clone();
        let _ = mdict_rs::settings::save(&current);
    }

    /// Stop any active TTS clip that no longer matches the popup's current
    /// content: a different text (new selection / new translation) OR a
    /// different TTS key (options changed). Ended/Loading clips are left
    /// alone (their stale affordance is already shown by the popup).
    ///
    /// `text` / `tts_key` are compared independently — pass `None` to skip
    /// a comparison. The two slots invalidate against the same inputs:
    /// source matches the popup's original, translation matches its
    /// translation. Callers reading the current text out of `popup_status`
    /// pass it in directly; the popup passes the full tts key from
    /// settings.
    pub fn invalidate_tts_clips(&mut self, text: Option<&str>, tts_key: Option<&str>) {
        self.playback_source.invalidate_on_change(text, tts_key);
        self.playback_translation
            .invalidate_on_change(text, tts_key);
    }

    /// Convenience: invalidate using the current `settings.tts` key (for
    /// settings-driven invalidation from the Options panel).
    pub fn invalidate_tts_clips_for_settings(&mut self, text: Option<&str>) {
        let key = crate::playback::tts_key(&self.quick_translate.tts);
        self.invalidate_tts_clips(text, Some(&key));
    }
}
