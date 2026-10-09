use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, warn};

use crate::{MAX_TEXT_LENGTH, TranslateError, TranslationRequest, TranslationResult, Translator};

/// Default OpenAI model.
pub const DEFAULT_MODEL: &str = "gpt-4o-mini";

/// Default API base URL (official OpenAI API).
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// OpenAI-compatible chat completions client.
///
/// Works with any API that implements the OpenAI chat completions format:
/// - OpenAI itself
/// - Ollama (http://localhost:11434/v1)
/// - Groq, OpenRouter, Together, etc.
/// - Local models served via llama.cpp server, vLLM, etc.
pub struct OpenaiTranslator {
    client: reqwest::blocking::Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenaiTranslator {
    pub fn builder() -> OpenaiTranslatorBuilder {
        OpenaiTranslatorBuilder::default()
    }
}

#[derive(Default)]
pub struct OpenaiTranslatorBuilder {
    api_key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
}

impl OpenaiTranslatorBuilder {
    pub fn api_key(mut self, api_key: String) -> Self {
        self.api_key = Some(api_key);
        self
    }

    pub fn base_url(mut self, base_url: String) -> Self {
        self.base_url = Some(base_url);
        self
    }

    pub fn model(mut self, model: String) -> Self {
        self.model = Some(model);
        self
    }

    pub fn build(self) -> OpenaiTranslator {
        let api_key = self.api_key.unwrap_or_default();
        let base_url = self
            .base_url
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let model = self.model.unwrap_or_else(|| DEFAULT_MODEL.to_string());

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("dicto-translate/0.1")
            .build()
            .expect("failed to build reqwest client");

        OpenaiTranslator {
            client,
            api_key,
            base_url,
            model,
        }
    }
}

// --- Request / Response types ---

/// Model-id fragments marking non-chat models. The /models endpoint lists
/// everything the key can access (embeddings, rerankers, audio, image,
/// video, moderation…); the model picker only wants chat-capable models.
/// Substring match, lowercase — verified against the common OpenAI,
/// OpenRouter, and z.ai naming schemes.
const NON_CHAT_FRAGMENTS: &[&str] = &[
    "embedding",
    "embed",
    "rerank",
    "whisper",
    "transcribe",
    "tts",
    "audio",
    "dall-e",
    "dalle",
    "image",
    "sora",
    "video",
    "flux",
    "stable-diffusion",
    "sd3",
    "moderation",
    "guard",
    "shield",
    "clip",
    "bge-",
];

/// Model-id fragments marking TTS-capable models (the inverse concern of
/// [`NON_CHAT_FRAGMENTS`]): the /models list usually doesn't tag speech
/// models, so we match known speech families by name.
const TTS_FRAGMENTS: &[&str] = &[
    "tts",
    "voice",
    "speech",
    "speak",
    "kokoro",
    "piper",
    "xtts",
    "orpheus",
    "chatterbox",
];

/// Whether a /models id looks like a text-to-speech model.
fn is_tts_model(id: &str) -> bool {
    let lower = id.to_lowercase();
    TTS_FRAGMENTS.iter().any(|frag| lower.contains(frag))
}

/// Whether a /models id looks chat-capable (translate-able).
fn is_chat_model(id: &str) -> bool {
    let lower = id.to_lowercase();
    !NON_CHAT_FRAGMENTS.iter().any(|frag| lower.contains(frag))
}

/// Fetch the model list from an OpenAI-compatible `GET {base_url}/models`
/// endpoint, filtered to chat-capable models. Blocking (same pattern as
/// translation); callers run it on a background executor. Returns sorted,
/// deduped model ids.
pub fn list_models(api_key: &str, base_url: &str) -> Result<Vec<String>, TranslateError> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("dicto-translate/0.1")
        .build()?;
    // The key is optional: several endpoints (e.g. OpenRouter) expose
    // /models publicly.
    let mut req = client.get(&url);
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }
    let resp = req.send()?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(TranslateError::Api(format!(
            "models API error {status}: {body}"
        )));
    }
    #[derive(Deserialize)]
    struct ModelsResponse {
        #[serde(default)]
        data: Vec<ModelEntry>,
    }
    #[derive(Deserialize)]
    struct ModelEntry {
        id: String,
    }
    let parsed: ModelsResponse = resp.json()?;
    let mut ids: Vec<String> = parsed
        .data
        .into_iter()
        .map(|m| m.id)
        .filter(|id| is_chat_model(id))
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// A TTS-capable model plus the voices the endpoint publishes for it
/// (`supported_voices`; may be empty — some models take arbitrary voice
/// ids).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TtsModel {
    pub id: String,
    pub voices: Vec<String>,
}

/// Fetch the model list filtered to text-to-speech models, **including the
/// voices the endpoint publishes for each model** (`supported_voices`).
/// Used by the TTS settings picker.
///
/// Queries `/models?output_modalities=speech` — the documented filter for
/// TTS models. Endpoints that ignore the parameter return their full chat
/// catalog instead; in that case the name-based [`is_tts_model`] filter
/// kicks in as a fallback.
pub fn list_tts_models(api_key: &str, base_url: &str) -> Result<Vec<TtsModel>, TranslateError> {
    let mut models = fetch_tts_models_raw(api_key, base_url)?;
    // If the endpoint honored the filter, every returned entry is TTS —
    // even those without a published voice list (e.g. fish-audio).
    // Otherwise it ignored the parameter and returned everything: keep
    // only name-matched TTS models.
    let honored = models.iter().any(|m| !m.voices.is_empty());
    if !honored {
        models.retain(|m| is_tts_model(&m.id));
    }
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    Ok(models)
}

/// GET `{base_url}/models[?output_modalities=speech]` and collect model ids
/// with their published voices.
///
/// The body is read as text first and parsed separately, so JSON problems
/// produce a *readable* error (with a response snippet) instead of
/// reqwest's opaque "error decoding response body". One retry absorbs
/// flaky connections / cold CDN cache on the first hit.
fn fetch_tts_models_raw(api_key: &str, base_url: &str) -> Result<Vec<TtsModel>, TranslateError> {
    let url = format!(
        "{}/models?output_modalities=speech",
        base_url.trim_end_matches('/')
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("dicto-translate/0.1")
        .build()?;

    let send = || -> Result<String, TranslateError> {
        let mut req = client.get(&url);
        if !api_key.is_empty() {
            req = req.bearer_auth(api_key);
        }
        let resp = req.send()?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(TranslateError::Api(format!(
                "models API error {status}: {}",
                truncate(&body, 200)
            )));
        }
        Ok(resp.text()?)
    };

    let body = match send() {
        Ok(body) => body,
        Err(e) => {
            tracing::warn!("models fetch failed ({e}); retrying once");
            std::thread::sleep(std::time::Duration::from_millis(500));
            send()?
        }
    };

    #[derive(Deserialize)]
    struct ModelsResponse {
        #[serde(default)]
        data: Vec<ModelEntry>,
    }
    #[derive(Deserialize)]
    struct ModelEntry {
        id: String,
        // `null` is common (models without a published voice list), so an
        // Option is required — `#[serde(default)]` alone only covers a
        // *missing* key, not an explicit null.
        #[serde(default)]
        supported_voices: Option<Vec<String>>,
    }
    let parsed: ModelsResponse = serde_json::from_str(&body).map_err(|e| {
        TranslateError::Parse(format!(
            "unexpected response from models API: {e}; body starts with: {}",
            truncate(&body, 120)
        ))
    })?;
    Ok(parsed
        .data
        .into_iter()
        .map(|m| TtsModel {
            id: m.id,
            voices: m.supported_voices.unwrap_or_default(),
        })
        .collect())
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    temperature: f32,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    model: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
    #[allow(dead_code)]
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

// --- Translation logic ---

fn build_system_prompt(target_lang: &str, source_lang: Option<&str>) -> String {
    match source_lang {
        Some(src) => format!(
            "You are a professional translator. Translate the following text from {src} to {target}. \
             Respond with ONLY the translation, no explanation, no commentary, no extra text.",
            src = src,
            target = target_lang,
        ),
        None => format!(
            "You are a professional translator. Translate the following text to {target}. \
             If the text is already in {target}, return it unchanged. \
             Respond with ONLY the translation, no explanation, no commentary, no extra text.",
            target = target_lang,
        ),
    }
}

impl Translator for OpenaiTranslator {
    #[instrument(skip(self, request), fields(provider = "openai_compatible", model = %self.model))]
    fn translate(&self, request: TranslationRequest) -> Result<TranslationResult, TranslateError> {
        let text = request.text.trim();
        if text.is_empty() {
            return Err(TranslateError::EmptyText);
        }
        if text.len() > MAX_TEXT_LENGTH {
            return Err(TranslateError::TextTooLong(text.len()));
        }

        let system = build_system_prompt(&request.target_lang, request.source_lang.as_deref());

        let body = ChatRequest {
            model: &self.model,
            temperature: 0.3,
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: &system,
                },
                ChatMessage {
                    role: "user",
                    content: text,
                },
            ],
        };

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        debug!(url = %url, "sending OpenAI-compatible translation request");

        let mut req = self
            .client
            .post(&url)
            .header("content-type", "application/json");

        // Ollama and some local servers don't require an API key.
        // We still send the Authorization header if a key is configured.
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }

        let response = req.json(&body).send()?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            warn!(status = %status, body = %body, "OpenAI-compatible API error");
            return Err(TranslateError::Api(format!(
                "HTTP {}: {}",
                status,
                if body.is_empty() {
                    "no response body".to_string()
                } else {
                    body
                }
            )));
        }

        let parsed: ChatResponse = response
            .json()
            .map_err(|e| TranslateError::Parse(format!("failed to parse OpenAI response: {e}")))?;

        let translated_text = parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content.trim().to_string())
            .ok_or_else(|| TranslateError::Parse("no choices in response".to_string()))?;

        if translated_text.is_empty() {
            return Err(TranslateError::Parse(
                "OpenAI response contained empty translation".to_string(),
            ));
        }

        Ok(TranslationResult {
            translated_text,
            detected_lang: None,
            model: parsed.model.unwrap_or_else(|| self.model.clone()),
            provider: "openai_compatible".to_string(),
        })
    }

    fn is_configured(&self) -> bool {
        // base_url is required; api_key is optional (some local servers don't need one)
        !self.base_url.is_empty()
    }

    fn provider_name(&self) -> &'static str {
        "openai_compatible"
    }
}

/// Cut a string to `max` chars on a char boundary, with an ellipsis.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}\u{2026}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_system_prompt_with_source() {
        let prompt = build_system_prompt("English", Some("French"));
        assert!(prompt.contains("from French to English"));
        assert!(prompt.contains("ONLY the translation"));
    }

    #[test]
    fn test_build_system_prompt_auto_detect() {
        let prompt = build_system_prompt("English", None);
        assert!(prompt.contains("to English"));
        assert!(prompt.contains("already in English, return it unchanged"));
    }

    #[test]
    fn test_empty_text_error() {
        let t = OpenaiTranslator::builder()
            .api_key("test".into())
            .base_url("http://localhost:11434/v1".into())
            .build();
        let err = t
            .translate(TranslationRequest {
                text: "   ".into(),
                source_lang: None,
                target_lang: "French".into(),
            })
            .unwrap_err();
        assert!(matches!(err, TranslateError::EmptyText));
    }

    #[test]
    fn test_is_configured_requires_base_url() {
        let t = OpenaiTranslator::builder().api_key("test".into()).build();
        // No base_url set → falls back to default → configured
        assert!(t.is_configured());

        let t = OpenaiTranslator::builder().build();
        // Also falls back to default base URL
        assert!(t.is_configured());
    }
}
