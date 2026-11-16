//! Free-model defaults and the rule for picking a model the user can afford.
//!
//! The lists here are *preferences*, not facts: the provider's live
//! `/models` response decides what actually exists. [`pick_text_model`] and
//! [`pick_tts_model`] reconcile the two, so a retired or newly free model
//! changes the answer without a code change.
//!
//! The ids are OpenRouter ids. Every other provider leaves the model name to
//! the user, so these lists only apply when the base URL is OpenRouter.

/// OpenRouter's API root, used to recognise the provider from its base URL.
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Free chat models that translate well, best first. Gemma leads because it
/// is the strongest multilingual free model; dots-3 is a user-tested
/// fallback while its `preview` id is alive.
pub const FREE_TEXT_MODELS: &[&str] = &[
    "google/gemma-4-26b-a4b-it:free",
    "google/gemma-4-31b-it:free",
    "dots-studio/dots-3-note-preview:free",
    "baidu/ernie-5.5-flash-preview:free",
];

/// Free text-to-speech models, best English pronunciation first. Persian is
/// not covered by either of them; the UI falls back to the system voice for
/// that side.
pub const FREE_TTS_MODELS: &[&str] = &[
    "fish-audio/s2.1-pro-free:free",
    "heygen/heygen-voice-1-turbo:free",
];

/// Cheap paid models used once the key has credit. Chosen so the first
/// request does not wait behind a busy free model.
pub const PAID_TEXT_MODELS: &[&str] = &[
    "google/gemini-3.8-flash-lite",
    "google/gemini-2.5-flash-lite",
    "openai/gpt-4.1-mini",
];

/// OpenRouter marks free model variants with a `:free` suffix.
pub fn is_free(id: &str) -> bool {
    id.ends_with(":free")
}

/// Whether `base_url` points at OpenRouter. Covers the public host and
/// mirrors, which keep `openrouter.ai` in the host name.
pub fn is_openrouter(base_url: &str) -> bool {
    base_url.to_ascii_lowercase().contains("openrouter.ai")
}

/// The model to start from on a base URL that has no stored choice.
///
/// OpenRouter gets a free model, because a fresh key has no credit and a paid
/// model would fail with 402. Everywhere else keeps the OpenAI default.
pub fn default_text_model_for(base_url: &str) -> &'static str {
    if is_openrouter(base_url) {
        FREE_TEXT_MODELS[0]
    } else {
        "gpt-4o-mini"
    }
}

/// First preferred id that the endpoint actually offers.
pub fn first_available(preferred: &[&str], available: &[String]) -> Option<String> {
    preferred
        .iter()
        .find(|want| available.iter().any(|id| id == *want))
        .map(|id| (*id).to_string())
}

/// Pick a translation model from what the provider offers.
///
/// `free_only` is the user's "free models only" switch; `has_credit` comes
/// from the key endpoint. Without credit a paid model would answer 402, so
/// the free list wins even when `free_only` is off.
///
/// Returns `None` when nothing in `available` matches — the caller should
/// keep the stored model and let the user choose, rather than invent an id.
pub fn pick_text_model(free_only: bool, has_credit: bool, available: &[String]) -> Option<String> {
    if free_only || !has_credit {
        return first_available(FREE_TEXT_MODELS, available)
            .or_else(|| available.iter().find(|id| is_free(id)).cloned());
    }
    first_available(PAID_TEXT_MODELS, available)
        .or_else(|| available.iter().find(|id| !is_free(id)).cloned())
}

/// Pick a voice model from what the provider offers, with the same fallback
/// rule as [`pick_text_model`].
pub fn pick_tts_model(free_only: bool, has_credit: bool, available: &[String]) -> Option<String> {
    if free_only || !has_credit {
        return first_available(FREE_TTS_MODELS, available)
            .or_else(|| available.iter().find(|id| is_free(id)).cloned());
    }
    available.iter().find(|id| !is_free(id)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn default_is_free_on_openrouter_only() {
        assert_eq!(
            default_text_model_for(OPENROUTER_BASE_URL),
            "google/gemma-4-26b-a4b-it:free"
        );
        assert_eq!(
            default_text_model_for("https://openrouter.ai/api/v1/"),
            "google/gemma-4-26b-a4b-it:free"
        );
        assert_eq!(
            default_text_model_for("https://api.openai.com/v1"),
            "gpt-4o-mini"
        );
        assert_eq!(default_text_model_for(""), "gpt-4o-mini");
    }

    #[test]
    fn picks_the_best_preference_that_exists() {
        let available = ids(&[
            "baidu/ernie-5.5-flash-preview:free",
            "google/gemma-4-31b-it:free",
        ]);
        assert_eq!(
            pick_text_model(true, false, &available).as_deref(),
            Some("google/gemma-4-31b-it:free")
        );
    }

    #[test]
    fn no_credit_beats_the_free_only_switch() {
        // The user asked for paid models, but the key cannot pay yet.
        let available = ids(&[
            "google/gemma-4-26b-a4b-it:free",
            "google/gemini-3.8-flash-lite",
        ]);
        assert_eq!(
            pick_text_model(false, false, &available).as_deref(),
            Some("google/gemma-4-26b-a4b-it:free")
        );
        assert_eq!(
            pick_text_model(false, true, &available).as_deref(),
            Some("google/gemini-3.8-flash-lite")
        );
    }

    #[test]
    fn unknown_free_model_still_counts_as_free() {
        let available = ids(&["someone/new-model:free"]);
        assert_eq!(
            pick_text_model(true, true, &available).as_deref(),
            Some("someone/new-model:free")
        );
    }

    #[test]
    fn no_match_returns_none_instead_of_guessing() {
        let available = ids(&["gpt-4o-mini"]);
        assert_eq!(pick_text_model(true, false, &available), None);
    }

    #[test]
    fn tts_prefers_fish_when_free() {
        let available = ids(&[
            "hexgrad/kokoro-82m",
            "fish-audio/s2.1-pro-free:free",
            "heygen/heygen-voice-1-turbo:free",
        ]);
        assert_eq!(
            pick_tts_model(true, false, &available).as_deref(),
            Some("fish-audio/s2.1-pro-free:free")
        );
        assert_eq!(
            pick_tts_model(false, true, &available).as_deref(),
            Some("hexgrad/kokoro-82m")
        );
    }
}
