//! Quick Translate feature orchestrator.
//!
//! Wires together:
//! - Global hotkey manager (X11 / tray menu fallback)
//! - Selection reader (clipboard / primary selection)
//! - LLM translator (Anthropic / OpenAI-compatible)
//! - Popup window with result display

use std::sync::Arc;

use dicto_translate::{TranslationRequest, Translator, translator_from_settings};
use mdict_rs::settings::{LlmProvider, QuickTranslateSettings};
use tracing::{error, info, warn};

use crate::components::translate_popup::PopupState;
use crate::hotkey::{HotkeyError, HotkeyManager, QUICK_TRANSLATE_ID, create_hotkey_manager};
use crate::selection::{SelectionError, SelectionSource, read_selected_text};

/// Current state of the quick-translate popup.
#[derive(Debug, Clone)]
pub enum PopupStatus {
    /// No popup is shown.
    Hidden,
    /// Popup is visible with the given state.
    Visible(PopupState),
}

impl PopupStatus {
    pub fn is_visible(&self) -> bool {
        matches!(self, PopupStatus::Visible(_))
    }
}

/// Quick Translate engine.
///
/// Owns the hotkey manager, translator client, and popup state.
/// Created once at app startup; hotkey registration and translator
/// are reconfigured when settings change.
pub struct QuickTranslateEngine {
    hotkey_manager: Option<Box<dyn HotkeyManager>>,
    translator: Arc<dyn Translator>,
    settings: QuickTranslateSettings,
    popup_status: PopupStatus,
}

impl QuickTranslateEngine {
    /// Create a new engine with the given settings.
    pub fn new(settings: QuickTranslateSettings) -> Self {
        let translator = build_translator(&settings);
        let hotkey_manager = if settings.enabled {
            match create_and_register(&settings) {
                Ok(mgr) => Some(mgr),
                Err(e) => {
                    warn!(error = %e, "failed to register hotkey");
                    None
                }
            }
        } else {
            None
        };

        Self {
            hotkey_manager,
            translator,
            settings,
            popup_status: PopupStatus::Hidden,
        }
    }

    /// Update settings and reconfigure the hotkey/translator as needed.
    pub fn update_settings(&mut self, new_settings: QuickTranslateSettings) {
        let old_provider = self.settings.llm_provider;
        let old_api_key = self.settings.api_key.clone();
        let old_base_url = self.settings.api_base_url.clone();
        let old_model = self.settings.model.clone();
        let old_enabled = self.settings.enabled;
        let old_hotkey = self.settings.hotkey.clone();

        self.settings = new_settings;

        // Reconfigure translator if provider/key/url/model changed
        let provider_changed = old_provider != self.settings.llm_provider
            || old_api_key != self.settings.api_key
            || old_base_url != self.settings.api_base_url
            || old_model != self.settings.model;

        if provider_changed {
            self.translator = build_translator(&self.settings);
        }

        // Reconfigure hotkey if enabled state or hotkey string changed
        let hotkey_changed =
            old_enabled != self.settings.enabled || old_hotkey != self.settings.hotkey;

        if hotkey_changed {
            self.reconfigure_hotkey();
        }
    }

    fn reconfigure_hotkey(&mut self) {
        // Drop existing manager first (unregisters on drop)
        self.hotkey_manager = None;

        if self.settings.enabled {
            match create_and_register(&self.settings) {
                Ok(mgr) => {
                    self.hotkey_manager = Some(mgr);
                }
                Err(e) => {
                    warn!(error = %e, "failed to register hotkey after settings change");
                }
            }
        }
    }

    /// Poll for hotkey events. Returns `true` if the popup was (re)opened.
    ///
    /// On a hotkey activation this calls [`trigger_translate`], which reads the
    /// selection and shows it in an `Idle` state. No translation runs here —
    /// the popup's Translate button calls [`start_translation`].
    pub fn poll(&mut self) -> bool {
        // Check for hotkey events without holding a borrow on self.
        let mut should_trigger = false;
        if let Some(manager) = self.hotkey_manager.as_ref() {
            while let Some(id) = manager.try_recv() {
                if id == QUICK_TRANSLATE_ID {
                    should_trigger = true;
                }
            }
        }

        if should_trigger {
            self.trigger_translate()
        } else {
            false
        }
    }

    /// Open the popup with the currently-selected text (hotkey / tray trigger).
    ///
    /// This does **not** translate anything — it just reads the selection and
    /// shows it in an `Idle` state with Translate / Speak buttons. The actual
    /// translation only starts when the user clicks Translate, via
    /// [`start_translation`]. Returns `true` if a popup is now visible.
    pub fn trigger_translate(&mut self) -> bool {
        dicto_telemetry::get().track(dicto_telemetry::Event::QuickTranslateTriggered {
            source: dicto_telemetry::QuickTranslateSource::Hotkey,
        });

        // Read selected text
        let (text, source) = match read_selected_text() {
            Ok(t) => t,
            Err(SelectionError::TooLong(text)) => {
                // Not a failure — the user just needs a smaller selection.
                self.popup_status = PopupStatus::Visible(PopupState::TooLong { original: text });
                return true;
            }
            Err(e) => {
                warn!(error = %e, "failed to read selection");
                self.popup_status = PopupStatus::Visible(PopupState::Error {
                    original: String::new(),
                    error: format!(
                        "Could not read selected text: {e}\n\n\
                         Tip: Select some text and copy it (Ctrl+C), then try again."
                    ),
                });
                return true;
            }
        };

        info!(
            source = match source {
                SelectionSource::Primary => "primary",
                SelectionSource::Clipboard => "clipboard",
            },
            len = text.len(),
            "got selection"
        );

        // Show the original text with Translate / Speak buttons — translation
        // only starts on explicit click.
        self.popup_status = PopupStatus::Visible(PopupState::Idle { original: text });
        true
    }

    /// Start the actual translation from the popup's current `Idle` state.
    ///
    /// Moves the popup to `Loading` and returns a `TranslationJob` whose
    /// `run()` performs the (blocking) HTTP request. The caller spawns `run()`
    /// on a background executor and feeds the outcome back into
    /// [`apply_translation_result`]. Returns `None` if the popup isn't `Idle`.
    pub fn start_translation(&mut self) -> Option<TranslationJob> {
        let original = match &self.popup_status {
            PopupStatus::Visible(PopupState::Idle { original }) => original.clone(),
            _ => return None,
        };

        self.popup_status = PopupStatus::Visible(PopupState::Loading {
            original: original.clone(),
        });

        Some(TranslationJob {
            request: TranslationRequest {
                text: original.clone(),
                source_lang: None,
                target_lang: self.settings.target_lang.clone(),
            },
            provider: provider_display_name(self.settings.llm_provider),
            model: self.settings.model.clone(),
            translator: self.translator.clone(),
            original,
        })
    }

    /// Apply the outcome of a background translation, moving the popup from
    /// `Loading` to `Ready` or `Error`. Called on the main thread once the
    /// background executor finishes the request.
    pub fn apply_translation_result(&mut self, outcome: TranslationOutcome) {
        let TranslationOutcome {
            original,
            provider,
            model,
            result,
        } = outcome;
        match result {
            Ok(translated) => {
                info!(provider = %provider, "translation completed");
                dicto_telemetry::get().track(dicto_telemetry::Event::QuickTranslateCompleted {
                    provider,
                    success: true,
                    duration_ms: 0,
                });
                self.popup_status = PopupStatus::Visible(PopupState::Ready {
                    original,
                    translation: translated,
                    provider: provider.to_string(),
                    model,
                });
            }
            Err(e) => {
                error!(error = %e, "translation failed");
                dicto_telemetry::get().track(dicto_telemetry::Event::QuickTranslateCompleted {
                    provider,
                    success: false,
                    duration_ms: 0,
                });
                self.popup_status = PopupStatus::Visible(PopupState::Error {
                    original,
                    error: e.to_string(),
                });
            }
        }
    }

    /// Get the current popup status.
    pub fn popup_status(&self) -> &PopupStatus {
        &self.popup_status
    }

    /// The full quick-translate settings (provider, model, target lang, TTS).
    /// Used by the popup to render inline selectors.
    pub fn settings(&self) -> &QuickTranslateSettings {
        &self.settings
    }

    /// The configured target language tag (e.g. `"fa"`, `"en"`).
    pub fn target_lang(&self) -> &str {
        &self.settings.target_lang
    }

    /// The TTS settings (AI TTS config; falls back to platform TTS if unset).
    pub fn tts_settings(&self) -> &mdict_rs::settings::TtsSettings {
        &self.settings.tts
    }

    /// Seed an `Idle` popup with the given original text and immediately start
    /// translating it. Used by the popup's Translate button. Returns the job
    /// to spawn off-thread (if any).
    pub fn restart_translation(&mut self, original: String) -> Option<TranslationJob> {
        self.popup_status = PopupStatus::Visible(PopupState::Idle {
            original: original.clone(),
        });
        self.start_translation()
    }

    /// Replace the popup's original text with the user's edited version and
    /// move the popup back to `Idle` — the shown translation (if any) is now
    /// stale. Called from the popup's Original editor on every edit; also
    /// lets a trimmed TooLong selection become translatable.
    pub fn set_original(&mut self, original: String) {
        self.popup_status = PopupStatus::Visible(PopupState::Idle { original });
    }

    /// Hide the popup.
    pub fn hide_popup(&mut self) {
        self.popup_status = PopupStatus::Hidden;
    }

    /// Get the hotkey backend name.
    pub fn backend_name(&self) -> &'static str {
        self.hotkey_manager
            .as_ref()
            .map(|m| m.backend_name())
            .unwrap_or("none")
    }
}

/// A self-contained translation request ready to run on a background thread.
pub struct TranslationJob {
    pub request: TranslationRequest,
    pub provider: &'static str,
    pub model: String,
    pub translator: Arc<dyn Translator>,
    /// The original selected text, carried through so the popup's `original`
    /// field is populated even though the request owns its own copy.
    pub original: String,
}

impl TranslationJob {
    /// Run the blocking translation and package the result for the main thread.
    pub fn run(self) -> TranslationOutcome {
        let TranslationJob {
            request,
            provider,
            model,
            translator,
            original,
        } = self;
        let result = translator
            .translate(request)
            .map(|r| r.translated_text)
            .map_err(|e| e.to_string());
        TranslationOutcome {
            original,
            provider,
            model,
            result,
        }
    }
}

/// The completed (or failed) translation, handed back to the engine.
pub struct TranslationOutcome {
    pub original: String,
    pub provider: &'static str,
    pub model: String,
    pub result: Result<String, String>,
}

/// Build a translator based on settings using the shared factory.
fn build_translator(settings: &QuickTranslateSettings) -> Arc<dyn Translator> {
    Arc::from(translator_from_settings(
        settings.enabled,
        match settings.llm_provider {
            LlmProvider::Anthropic => dicto_translate::LlmProvider::Anthropic,
            LlmProvider::OpenAiCompatible => dicto_translate::LlmProvider::OpenAiCompatible,
        },
        &settings.api_key,
        &settings.api_base_url,
        &settings.model,
    ))
}

/// Create a hotkey manager and register the quick-translate hotkey.
fn create_and_register(
    settings: &QuickTranslateSettings,
) -> Result<Box<dyn HotkeyManager>, HotkeyError> {
    let manager = create_hotkey_manager();
    manager.register(QUICK_TRANSLATE_ID, &settings.hotkey)?;
    Ok(manager)
}

/// Display name for the LLM provider.
pub fn provider_display_name(provider: LlmProvider) -> &'static str {
    match provider {
        LlmProvider::Anthropic => "Anthropic",
        LlmProvider::OpenAiCompatible => "OpenAI-compatible",
    }
}
