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

/// Cut a model list down to free variants when that picker's "Free models
/// only" switch is on. Translation and voice each have their own switch.
///
/// The switch is a *filter*, not a policy: when the endpoint publishes no
/// free variant at all (OpenAI, a local server), the full list comes back
/// untouched so the picker never ends up empty.
pub fn filter_free(ids: &[String], free_only: bool) -> Vec<String> {
    if !free_only || !ids.iter().any(|id| mdict_rs::settings::free::is_free(id)) {
        return ids.to_vec();
    }
    ids.iter()
        .filter(|id| mdict_rs::settings::free::is_free(id))
        .cloned()
        .collect()
}

/// Secondary line under a model picker: what the switch is showing.
///
/// The count is the honest answer to "did the filter change anything?" —
/// with the switch on, the number of rows a user can open is the free
/// count, not the endpoint's total. `total == 0` means nothing is loaded
/// yet, so there is nothing to say.
pub(crate) fn free_hint(free_only: bool, total: usize, free_n: usize) -> Option<String> {
    if total == 0 {
        return None;
    }
    Some(if !free_only {
        format!("All {total} models")
    } else if free_n == 0 {
        // The switch is a filter, not a policy: an endpoint with no free
        // variant keeps its full list, and the line says why.
        format!("No free models here — showing all {total}")
    } else {
        format!("Showing {free_n} free of {total} models")
    })
}

/// The inputs a `/models` load depends on: the endpoint and the key.
///
/// The list changes when the endpoint changes, and an endpoint that
/// refuses a keyless request answers the same request once a key is
/// present — so both parts decide whether a cached list is still the
/// right list. Whitespace around a pasted URL is not a different
/// endpoint, hence the trim.
pub(crate) fn load_key(base_url: &str, api_key: &str) -> (String, String) {
    (base_url.trim().to_string(), api_key.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn filter_keeps_only_free_variants_when_on() {
        let all = ids(&["gpt-4o-mini", "google/gemma-4-26b-a4b-it:free"]);
        assert_eq!(
            filter_free(&all, true),
            ids(&["google/gemma-4-26b-a4b-it:free"])
        );
    }

    #[test]
    fn filter_is_a_no_op_when_off() {
        let all = ids(&["gpt-4o-mini", "google/gemma-4-26b-a4b-it:free"]);
        assert_eq!(filter_free(&all, false), all);
    }

    #[test]
    fn endpoint_without_free_models_keeps_its_list() {
        let all = ids(&["gpt-4o-mini", "tts-1"]);
        assert_eq!(filter_free(&all, true), all);
    }

    #[test]
    fn hint_says_nothing_before_a_load() {
        assert_eq!(free_hint(true, 0, 0), None);
    }

    #[test]
    fn hint_reports_the_filtered_count() {
        // 422 models loaded, 6 of them free: the pull-down offers 6.
        assert_eq!(
            free_hint(true, 422, 6).as_deref(),
            Some("Showing 6 free of 422 models")
        );
    }

    #[test]
    fn hint_explains_an_endpoint_without_free_variants() {
        assert_eq!(
            free_hint(true, 3, 0).as_deref(),
            Some("No free models here — showing all 3")
        );
    }

    #[test]
    fn hint_names_the_total_when_the_filter_is_off() {
        assert_eq!(free_hint(false, 422, 6).as_deref(), Some("All 422 models"));
    }

    #[test]
    fn load_key_ignores_whitespace_around_the_url() {
        // A pasted URL often carries a trailing space or newline; that is
        // the same endpoint, so it must not look like a change to the
        // automatic loader (and refetch on every render).
        assert_eq!(
            load_key(" https://openrouter.ai/api/v1\n", "sk-1"),
            load_key("https://openrouter.ai/api/v1", "sk-1")
        );
    }

    #[test]
    fn load_key_tracks_the_key() {
        // A keyed endpoint answers only after the key is entered.
        assert_ne!(
            load_key("https://api.openai.com/v1", ""),
            load_key("https://api.openai.com/v1", "sk-1")
        );
    }
}
