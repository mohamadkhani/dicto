use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, warn};

use crate::{MAX_TEXT_LENGTH, TranslateError, Translator, TranslationRequest, TranslationResult};

/// Default Anthropic model.
pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

/// Default API base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// Maximum number of tokens to generate for a translation.
const MAX_TOKENS: u32 = 2048;

/// Anthropic Messages API client.
pub struct AnthropicTranslator {
    client: reqwest::blocking::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl AnthropicTranslator {
    pub fn builder() -> AnthropicTranslatorBuilder {
        AnthropicTranslatorBuilder::default()
    }
}

#[derive(Default)]
pub struct AnthropicTranslatorBuilder {
    api_key: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
}

impl AnthropicTranslatorBuilder {
    pub fn api_key(mut self, api_key: String) -> Self {
        self.api_key = Some(api_key);
        self
    }

    pub fn model(mut self, model: String) -> Self {
        self.model = Some(model);
        self
    }

    pub fn maybe_base_url(mut self, base_url: Option<String>) -> Self {
        self.base_url = base_url;
        self
    }

    pub fn build(self) -> AnthropicTranslator {
        let api_key = self.api_key.unwrap_or_default();
        let model = self.model.unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let base_url = self.base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("dicto-translate/0.1")
            .build()
            .expect("failed to build reqwest client");

        AnthropicTranslator {
            client,
            api_key,
            model,
            base_url,
        }
    }
}

// --- Request / Response types ---

#[derive(Serialize)]
struct MessageRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<Message<'a>>,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct MessageResponse {
    content: Vec<ContentBlock>,
    model: String,
    #[allow(dead_code)]
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
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

impl Translator for AnthropicTranslator {
    #[instrument(skip(self, request), fields(provider = "anthropic", model = %self.model))]
    fn translate(&self, request: TranslationRequest) -> Result<TranslationResult, TranslateError> {
        let text = request.text.trim();
        if text.is_empty() {
            return Err(TranslateError::EmptyText);
        }
        if text.len() > MAX_TEXT_LENGTH {
            return Err(TranslateError::TextTooLong(text.len()));
        }

        let system = build_system_prompt(&request.target_lang, request.source_lang.as_deref());

        let body = MessageRequest {
            model: &self.model,
            max_tokens: MAX_TOKENS,
            system: &system,
            messages: vec![Message {
                role: "user",
                content: text,
            }],
        };

        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        debug!(url = %url, "sending Anthropic translation request");

        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            warn!(status = %status, body = %body, "Anthropic API error");
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

        let parsed: MessageResponse = response.json().map_err(|e| {
            TranslateError::Parse(format!("failed to parse Anthropic response: {e}"))
        })?;

        let translated_text = parsed
            .content
            .into_iter()
            .filter(|b| b.kind == "text")
            .filter_map(|b| b.text)
            .collect::<Vec<_>>()
            .join("")
            .trim()
            .to_string();

        if translated_text.is_empty() {
            return Err(TranslateError::Parse(
                "Anthropic response contained no text content".to_string(),
            ));
        }

        Ok(TranslationResult {
            translated_text,
            detected_lang: None, // Anthropic doesn't return detected language
            model: parsed.model,
            provider: "anthropic".to_string(),
        })
    }

    fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    fn provider_name(&self) -> &'static str {
        "anthropic"
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
    fn test_null_translator() {
        let t = crate::NullTranslator;
        assert!(!t.is_configured());
        assert_eq!(t.provider_name(), "null");
        let err = t
            .translate(TranslationRequest {
                text: "hello".into(),
                source_lang: None,
                target_lang: "French".into(),
            })
            .unwrap_err();
        assert!(matches!(err, TranslateError::NotConfigured));
    }

    #[test]
    fn test_empty_text_error() {
        let t = AnthropicTranslator::builder()
            .api_key("test".into())
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
    fn test_text_too_long() {
        let t = AnthropicTranslator::builder()
            .api_key("test".into())
            .build();
        let err = t
            .translate(TranslationRequest {
                text: "x".repeat(MAX_TEXT_LENGTH + 1),
                source_lang: None,
                target_lang: "French".into(),
            })
            .unwrap_err();
        assert!(matches!(err, TranslateError::TextTooLong(_)));
    }
}
