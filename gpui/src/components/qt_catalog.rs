//! Hardcoded, verified option catalogs for the Quick Translate settings UI.
//!
//! Every model ID, base URL, and voice listed here has been confirmed working
//! by hitting the provider's API (curl). Keeping these curated — instead of
//! free-text inputs — prevents invalid configurations (e.g. an OpenAI voice on
//! a Grok model, or a non-existent model ID) that previously caused silent
//! fallbacks with no user-visible guidance.
//!
//! The persisted settings (`QuickTranslateSettings` / `TtsSettings`) stay as
//! plain strings; this module only constrains *which* strings the UI offers.

use mdict_rs::settings::LlmProvider;

// ---------------------------------------------------------------------------
// Translation: provider → models
// ---------------------------------------------------------------------------

/// A model entry: `(model_id, human_label)`.
pub type ModelEntry = (&'static str, &'static str);

/// Models offered when the translation provider is Anthropic.
pub const ANTHROPIC_MODELS: &[ModelEntry] = &[
    ("claude-sonnet-4-6", "Sonnet 4.6 (balanced)"),
    ("claude-opus-4-7", "Opus 4.7 (most capable)"),
    ("claude-haiku-4-5", "Haiku 4.5 (fast)"),
];

/// Models offered when the translation provider is OpenAI-compatible.
///
/// These work with any OpenAI-compatible `/chat/completions` endpoint
/// (z.ai, OpenAI, OpenRouter, local servers). The model ID must match what
/// the chosen base URL serves.
pub const OPENAI_COMPATIBLE_MODELS: &[ModelEntry] = &[
    ("GLM-5.3-flash", "GLM-5.3-flash (z.ai)"),
    ("gpt-4o-mini", "GPT-4o mini"),
    ("gpt-5", "GPT-5"),
    ("llama-3.1-70b", "Llama 3.1 70B"),
    ("mistral-large", "Mistral Large"),
];

/// Return the model list for the given provider.
pub fn models_for(provider: LlmProvider) -> &'static [ModelEntry] {
    match provider {
        LlmProvider::Anthropic => ANTHROPIC_MODELS,
        LlmProvider::OpenAiCompatible => OPENAI_COMPATIBLE_MODELS,
    }
}

// ---------------------------------------------------------------------------
// TTS: provider → (model + base_url) → voices
// ---------------------------------------------------------------------------

/// A TTS preset: display label plus the model/base_url it commits and the
/// voices valid for that model. All entries are curl-verified.
pub struct TtsPreset {
    pub label: &'static str,
    pub model: &'static str,
    pub base_url: &'static str,
    pub voices: &'static [&'static str],
}

pub const TTS_PRESETS: &[TtsPreset] = &[
    TtsPreset {
        label: "Grok Voice (OpenRouter)",
        model: "x-ai/grok-voice-tts-1.0",
        base_url: "https://openrouter.ai/api/v1",
        // 5 fixed voices; accent is baked in per voice (no British/Am switch).
        voices: &["Eve", "Ara", "Rex", "Sal", "Leo"],
    },
    TtsPreset {
        label: "Kokoro (OpenRouter)",
        model: "hexgrad/kokoro-82m",
        base_url: "https://openrouter.ai/api/v1",
        // bf_* = British female, bm_* = British male,
        // af_* = American female, am_* = American male.
        voices: &["bf_emma", "bf_alice", "bm_george", "af_sky", "am_adam"],
    },
    TtsPreset {
        label: "OpenAI",
        model: "gpt-4o-mini-tts",
        base_url: "https://api.openai.com/v1",
        voices: &["alloy", "nova", "shimmer", "echo", "fable", "onyx"],
    },
];

/// Find the preset index whose model+base_url match the given settings.
///
/// Returns `None` when the user has a hand-edited value outside the catalog
/// (the UI then shows it as a "Custom" selection so nothing is silently lost).
pub fn find_tts_preset(model: &str, base_url: &str) -> Option<usize> {
    TTS_PRESETS
        .iter()
        .position(|p| p.model == model && p.base_url == base_url)
}

// ---------------------------------------------------------------------------
// Target languages
// ---------------------------------------------------------------------------

/// Common target languages for translation. The popup sends these to the LLM
/// as the `target_lang`, so full names work better than codes.
pub const TARGET_LANGS: &[&str] = &[
    "English", "Persian", "Arabic", "French", "German", "Spanish", "Italian", "Russian", "Chinese",
    "Japanese", "Korean", "Turkish", "Hindi",
];
