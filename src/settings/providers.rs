//! Built-in provider presets for the Quick Translate setup card.
//!
//! A new user should not have to know what a base URL is. Each preset carries
//! everything one provider needs — endpoint, where to get a key, the model to
//! start on, and whether it can also speak — so choosing a row fills the
//! settings in one click and leaves only "paste your key" to the user.
//!
//! The list is deliberately small and global: OpenRouter, AvalAI, GitHub
//! Models, and a local server. It is not a country list. Which rows appear
//! first is decided at runtime by [`crate::settings::free`]-style reachability
//! probing in the UI, not by asking the user where they live.
//!
//! The model ids here are *starting points*; the pickers reconcile them
//! against the provider's live `/models` response (see `qt_catalog`), so a
//! retired id does not strand anyone.

use super::{QuickTranslateSettings, free};

/// One provider the setup card can configure.
#[derive(Debug, Clone, Copy)]
pub struct ProviderPreset {
    /// Stable key used by the UI for element ids and probe bookkeeping.
    pub id: &'static str,
    pub name: &'static str,
    /// One line under the name, in the card's provider list.
    pub tagline: &'static str,
    /// OpenAI-compatible API root.
    pub base_url: &'static str,
    /// Page where the user creates a key. `None` for keyless providers.
    pub key_url: Option<&'static str>,
    /// Label for the key link, e.g. "Open AI keys page".
    pub key_label: &'static str,
    /// Where the key lives in that dashboard, in one short sentence.
    pub key_hint: &'static str,
    /// The dashboard walkthrough, one short sentence per step. Shown as a
    /// numbered list so "get a key" is a checklist, not a hunt.
    pub steps: &'static [&'static str],
    /// What signing up costs, when it is not obvious.
    pub signup_note: &'static str,
    /// Translation model to select after choosing this provider.
    pub text_model: &'static str,
    /// Voice model; empty when the provider cannot speak.
    pub tts_model: &'static str,
    /// Default voice name; empty means "the model's own default".
    pub voice: &'static str,
    /// True for a server on this machine: its probe result means "not
    /// running" rather than "blocked by the network".
    pub local: bool,
}

impl ProviderPreset {
    pub fn keyed(&self) -> bool {
        self.key_url.is_some()
    }

    /// Whether this provider can read text aloud through the AI voice.
    pub fn speaks(&self) -> bool {
        !self.tts_model.is_empty()
    }

    /// Host shown in the card when space is tight (`api.avalai.ir`).
    pub fn host(&self) -> &'static str {
        self.base_url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or(self.base_url)
    }
}

/// The presets the setup card offers, in fallback order.
///
/// This order is only the tie-breaker: the card re-sorts by reachability so a
/// service that answers from the user's network is never buried under one
/// that does not.
pub const PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        id: "openrouter",
        name: "OpenRouter",
        tagline: "Many models behind one key, including free ones.",
        base_url: free::OPENROUTER_BASE_URL,
        key_url: Some("https://openrouter.ai/settings/keys"),
        key_label: "Open AI keys page",
        key_hint: "Settings → Keys → Create key",
        steps: &[
            "Sign in with Google, GitHub, or email.",
            "Open Settings → Keys and choose Create key.",
            "Copy it now — the full key is shown only once.",
        ],
        signup_note: "Free models need no card. Free models allow 50 requests a day.",
        text_model: free::FREE_TEXT_MODELS[0],
        tts_model: free::FREE_TTS_MODELS[0],
        voice: "",
        local: false,
    },
    ProviderPreset {
        id: "avalai",
        name: "AvalAI",
        tagline: "No VPN, rial payment, Persian voice.",
        base_url: "https://api.avalai.ir/v1",
        key_url: Some("https://chat.avalai.ir/platform/api-keys"),
        key_label: "Open AvalAI API keys",
        key_hint: "API Keys → Create key",
        steps: &[
            "Create an account. Verifying a phone number adds credit.",
            "Open API Keys and choose Create key.",
            "Copy it now — the full key is shown only once.",
        ],
        signup_note: "Email signup works; verifying a phone number adds credit.",
        text_model: "gemini-2.5-flash-lite",
        tts_model: "gemini-3.8-flash-lite-tts",
        voice: "",
        local: false,
    },
    ProviderPreset {
        id: "github",
        name: "GitHub Models",
        tagline: "A free daily quota with the GitHub account you have.",
        base_url: "https://models.github.ai/inference",
        key_url: Some("https://github.com/settings/tokens"),
        key_label: "Open GitHub token settings",
        key_hint: "Fine-grained token with Models: read",
        steps: &[
            "Create a fine-grained token with the Models: read permission.",
            "Copy the token — it starts with github_pat_.",
            "Paste it below.",
        ],
        signup_note: "No payment. Rate-limited per day.",
        text_model: "openai/gpt-4.1-mini",
        // GitHub Models serves text only; Speak keeps the system voice.
        tts_model: "",
        voice: "",
        local: false,
    },
    ProviderPreset {
        id: "local",
        name: "On this computer",
        tagline: "Ollama or LM Studio. No account, nothing leaves the machine.",
        base_url: "http://localhost:11434/v1",
        key_url: None,
        key_label: "",
        key_hint: "No key needed",
        steps: &[
            "Install Ollama and run ollama pull qwen3:4b.",
            "Keep Ollama running. No key is needed.",
            "Optionally install a Piper voice for offline speech.",
        ],
        signup_note: "Free. Needs disk space and some RAM.",
        text_model: "qwen3:4b",
        tts_model: "",
        voice: "",
        local: true,
    },
];

/// The preset for a base URL, when it is one of the built-ins.
///
/// Matching ignores case, whitespace, and a trailing slash, so a hand-edited
/// URL still resolves to its preset instead of looking like a custom server.
pub fn detect(base_url: &str) -> Option<&'static ProviderPreset> {
    let want = normalize(base_url);
    if want.is_empty() {
        return None;
    }
    PRESETS
        .iter()
        .find(|p| normalize(p.base_url) == want)
        .or_else(|| {
            // A user may have pasted a deeper path on the same host
            // (`.../v1/`); the host is enough to recognise the provider.
            PRESETS.iter().find(|p| {
                let host = normalize(p.base_url);
                want == host
                    || want
                        .strip_prefix(&host)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
        })
}

fn normalize(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// Whether `base_url` points at a server on this machine.
///
/// A local server needs no key, so the "is this configured?" check and the
/// setup card both treat an empty key as fine for these addresses. The match
/// is on the host, not a provider name, because a user may run a local
/// server on any port or a LAN address.
pub fn is_local_url(base_url: &str) -> bool {
    let lower = normalize(base_url);
    let host = lower
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("");
    let host = host.split(':').next().unwrap_or(host);
    host == "localhost"
        || host == "127.0.0.1"
        || host == "0.0.0.0"
        || host == "[::1]"
        || host.ends_with(".local")
}

/// Write `preset` into the Quick Translate settings.
///
/// A key belongs to the provider it was issued for, so switching to a
/// different endpoint clears it — otherwise the first test would fail with
/// a rejection the user cannot explain. Re-applying the same preset keeps the
/// key, so clicking the current row twice is harmless.
///
/// One key covers both translation and the voice, which is the setup card's
/// promise; [`sync_shared_key`] keeps them in step afterwards.
pub fn apply(preset: &ProviderPreset, qt: &mut QuickTranslateSettings) {
    let switching = normalize(&qt.api_base_url) != normalize(preset.base_url);
    if switching {
        qt.api_key.clear();
        qt.tts.api_key.clear();
    }

    qt.api_base_url = preset.base_url.to_string();
    qt.model = preset.text_model.to_string();
    // A fresh provider starts on free models: a new key usually has no
    // credit, and a paid model would answer 402 before the user can relax.
    qt.free_only = true;

    qt.tts.api_base_url = preset.base_url.to_string();
    qt.tts.model = preset.tts_model.to_string();
    qt.tts.voice = preset.voice.to_string();
    qt.tts.enabled = preset.speaks();
    qt.tts.free_only = true;
}

/// Keep the voice endpoint in step with a just-changed translation endpoint.
///
/// Call this *after* the translation field was written, passing the value it
/// replaced. The TTS section deliberately allows a different key and
/// endpoint, so this only follows along while the voice side still holds the
/// old shared value (or nothing). Once the user types something of its own,
/// it is left alone.
pub fn sync_shared_base_url(qt: &mut QuickTranslateSettings, old_base_url: &str) {
    let follows = qt.tts.api_base_url.is_empty()
        || normalize(&qt.tts.api_base_url) == normalize(old_base_url);
    if follows {
        qt.tts.api_base_url = qt.api_base_url.clone();
    }
}

/// Same rule for the key, expressed against the value being replaced.
pub fn sync_shared_key(qt: &mut QuickTranslateSettings, old_key: &str) {
    let follows = qt.tts.api_key.is_empty() || qt.tts.api_key == old_key;
    if follows {
        qt.tts.api_key = qt.api_key.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with_key() -> QuickTranslateSettings {
        let mut qt = QuickTranslateSettings {
            api_base_url: "https://api.openai.com/v1".to_string(),
            api_key: "sk-old".to_string(),
            ..QuickTranslateSettings::default()
        };
        qt.tts.api_key = "sk-old".to_string();
        qt
    }

    #[test]
    fn every_preset_has_a_distinct_id_and_an_api_root() {
        let mut seen = std::collections::HashSet::new();
        for p in PRESETS {
            assert!(seen.insert(p.id), "duplicate preset id {}", p.id);
            assert!(
                p.base_url.starts_with("http"),
                "{} has no usable base URL",
                p.id
            );
            assert!(!p.name.is_empty());
            assert!(!p.steps.is_empty(), "{} has no key steps", p.id);
        }
    }

    #[test]
    fn local_servers_are_recognised_without_a_key() {
        for url in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:1234/v1",
            "  HTTP://LOCALHOST:8080/v1  ",
            "http://my-box.local:8000/v1",
        ] {
            assert!(is_local_url(url), "{url}");
        }
        for url in [
            "https://api.openrouter.ai/api/v1",
            "https://api.avalai.ir/v1",
            "",
        ] {
            assert!(!is_local_url(url), "{url}");
        }
    }

    #[test]
    fn openrouter_starts_on_a_free_model() {
        let p = detect(free::OPENROUTER_BASE_URL).expect("openrouter preset");
        assert!(free::is_free(p.text_model));
        assert_eq!(p.text_model, free::FREE_TEXT_MODELS[0]);
    }

    #[test]
    fn detect_ignores_case_whitespace_and_trailing_slash() {
        for url in [
            "https://openrouter.ai/api/v1",
            "https://openrouter.ai/api/v1/",
            "  HTTPS://OpenRouter.ai/api/v1  ",
        ] {
            assert_eq!(detect(url).map(|p| p.id), Some("openrouter"), "{url}");
        }
        assert_eq!(detect("https://api.openai.com/v1").map(|p| p.id), None);
        assert_eq!(detect("").map(|p| p.id), None);
    }

    #[test]
    fn applying_a_preset_wires_text_and_voice_to_one_endpoint() {
        let mut qt = settings_with_key();
        let p = detect("https://api.avalai.ir/v1").expect("avalai preset");
        apply(p, &mut qt);

        assert_eq!(qt.api_base_url, p.base_url);
        assert_eq!(qt.model, p.text_model);
        assert!(qt.free_only);
        assert_eq!(qt.tts.api_base_url, p.base_url);
        assert_eq!(qt.tts.model, p.tts_model);
        assert_eq!(qt.tts.enabled, p.speaks());
    }

    #[test]
    fn switching_provider_drops_a_key_issued_elsewhere() {
        let mut qt = settings_with_key();
        let p = detect("https://api.avalai.ir/v1").unwrap();
        apply(p, &mut qt);
        assert!(qt.api_key.is_empty());
        assert!(qt.tts.api_key.is_empty());
    }

    #[test]
    fn re_applying_the_same_preset_keeps_the_key() {
        let mut qt = settings_with_key();
        let p = detect("https://openrouter.ai/api/v1").unwrap();
        apply(p, &mut qt);
        qt.api_key = "sk-or".to_string();
        apply(p, &mut qt);
        assert_eq!(qt.api_key, "sk-or");
    }

    #[test]
    fn a_provider_without_voices_leaves_ai_speech_off() {
        let p = PRESETS
            .iter()
            .find(|p| p.id == "github")
            .expect("github preset");
        assert!(!p.speaks());
        let mut qt = QuickTranslateSettings::default();
        apply(p, &mut qt);
        assert!(!qt.tts.enabled);
    }

    #[test]
    fn the_shared_key_follows_only_while_it_is_still_shared() {
        let mut qt = settings_with_key();
        // The translation field was just written; the voice side still holds
        // the key that field used to have, so it follows along.
        qt.api_key = "sk-new".to_string();
        sync_shared_key(&mut qt, "sk-old");
        assert_eq!(qt.tts.api_key, "sk-new");

        // A voice key of its own is not overwritten.
        qt.tts.api_key = "sk-voice".to_string();
        qt.api_key = "sk-newer".to_string();
        sync_shared_key(&mut qt, "sk-new");
        assert_eq!(qt.tts.api_key, "sk-voice");
    }

    #[test]
    fn the_shared_base_url_follows_only_while_it_still_matches() {
        let mut qt = settings_with_key();
        qt.tts.api_base_url = "https://api.openai.com/v1".to_string();

        // The voice endpoint still follows the translation one, so it moves
        // with it.
        qt.api_base_url = "https://new.example/v1".to_string();
        sync_shared_base_url(&mut qt, "https://api.openai.com/v1");
        assert_eq!(qt.tts.api_base_url, "https://new.example/v1");

        // A voice endpoint of its own stays put.
        qt.tts.api_base_url = "https://voice.example/v1".to_string();
        qt.api_base_url = "https://newer.example/v1".to_string();
        sync_shared_base_url(&mut qt, "https://new.example/v1");
        assert_eq!(qt.tts.api_base_url, "https://voice.example/v1");
    }
}
