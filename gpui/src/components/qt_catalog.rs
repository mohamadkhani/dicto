//! Option catalogs for the Quick Translate settings UI.
//!
//! Translation and TTS models are NOT hardcoded here: both pickers load
//! their catalogs live from the endpoint's `/models` API
//! (`dicto_translate::openai::list_models` / `list_tts_models`), filtered
//! to chat-capable and speech-capable models respectively. A user's current
//! setting always stays visible in its picker as a "Custom: …" entry.
//!
//! The one exception is [`OPENAI_TTS_MODELS`]: a fallback list used when an
//! endpoint serves TTS models without listing them in /models (OpenRouter
//! does exactly that — verified 2026-10). The persisted settings
//! (`QuickTranslateSettings` / `TtsSettings`) stay as plain strings; this
//! module only constrains *which* strings the UI offers.

// ---------------------------------------------------------------------------
// TTS: fallback models
// ---------------------------------------------------------------------------

/// Fallback TTS models shown when the endpoint's /models load finds none.
///
/// Important: some endpoints (OpenRouter) serve TTS models but exclude
/// them from /models entirely (verified 2026-10: none of OpenRouter's
/// TTS-collection models appear in /api/v1/models). This list is scraped
/// from OpenRouter's own "Text-to-Speech Models" collection plus OpenAI's
/// classic TTS models, so those endpoints stay usable.
pub const OPENAI_TTS_MODELS: &[&str] = &[
    // OpenAI
    "gpt-4o-mini-tts",
    "tts-1",
    "tts-1-hd",
    // OpenRouter TTS collection (verified 2026-10-03)
    "bytedance-seed/seed-audio-1-0",
    "canopylabs/orpheus-3b-0.1-ft",
    "deepgram/aura-2",
    "deepgram/flux-tts",
    "fish-audio/s1",
    "fish-audio/s2-pro",
    "fish-audio/s2.1-pro",
    "fish-audio/s2.1-pro-free:free",
    "google/gemini-3.1-flash-tts-preview",
    "google/gemini-3.8-flash-lite-tts",
    "google/gemini-3.8-flash-tts",
    "google/gemini-3.8-flash-tts-20260922",
    "hexgrad/kokoro-82m",
    "microsoft/mai-voice-2",
    "microsoft/mai-voice-2-flash",
    "microsoft/mai-voice-2.1",
    "microsoft/mai-voice-2.1-flash",
    "microsoft/mai-voice-2.1-20261001",
    "microsoft/mai-voice-2.1-flash-20261001",
    "minimax/speech-2.8-hd",
    "minimax/speech-2.8-turbo",
    "mistralai/voxtral-mini-tts-2603",
    "qwen/qwen-audio-3.0-tts-flash",
    "qwen/qwen-audio-3.0-tts-plus",
    "sesame/csm-1b",
    "x-ai/grok-voice-tts-1.0",
    "zyphra/zonos-v0.1-hybrid",
    "zyphra/zonos-v0.1-transformer",
];

// ---------------------------------------------------------------------------
// Target languages
// ---------------------------------------------------------------------------

/// Common target languages for translation. The popup sends these to the LLM
/// as the `target_lang`, so full names work better than codes.
pub const TARGET_LANGS: &[&str] = &[
    "English", "Persian", "Arabic", "French", "German", "Spanish", "Italian", "Russian", "Chinese",
    "Japanese", "Korean", "Turkish", "Hindi",
];
