use std::fmt;

use serde::{Deserialize, Serialize};

/// Errors that can occur during translation.
#[derive(Debug, thiserror::Error)]
pub enum TranslateError {
    #[error("translator not configured")]
    NotConfigured,
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error: {0}")]
    Api(String),
    #[error("failed to parse response: {0}")]
    Parse(String),
    #[error("text too long: {0} chars (max 10000)")]
    TextTooLong(usize),
    #[error("empty input text")]
    EmptyText,
}

/// A translation request.
#[derive(Debug, Clone)]
pub struct TranslationRequest {
    /// Text to translate.
    pub text: String,
    /// Source language, or None for auto-detection.
    pub source_lang: Option<String>,
    /// Target language (e.g. "English", "Persian", "Spanish").
    pub target_lang: String,
}

/// A translation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationResult {
    /// The translated text.
    pub translated_text: String,
    /// Detected source language, if auto-detected.
    pub detected_lang: Option<String>,
    /// Model that produced the translation.
    pub model: String,
    /// Provider name ("anthropic" / "openai_compatible").
    pub provider: String,
}

/// Which LLM provider to use.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LlmProvider {
    #[default]
    Anthropic,
    OpenAiCompatible,
}

impl fmt::Display for LlmProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmProvider::Anthropic => write!(f, "Anthropic"),
            LlmProvider::OpenAiCompatible => write!(f, "OpenAI-compatible"),
        }
    }
}

/// Trait for translation providers.
///
/// Mirrors the `Telemetry` trait pattern in dicto-telemetry: a trait with
/// a real implementation and a no-op `NullTranslator` fallback.
pub trait Translator: Send + Sync {
    /// Translate the given request.
    fn translate(&self, request: TranslationRequest) -> Result<TranslationResult, TranslateError>;

    /// Returns true if the translator is fully configured and ready to use.
    fn is_configured(&self) -> bool;

    /// Provider name for display / telemetry.
    fn provider_name(&self) -> &'static str;
}

/// Max input text length to prevent accidental huge API calls.
pub const MAX_TEXT_LENGTH: usize = 10_000;

pub mod anthropic;
pub mod null;
pub mod openai;

pub use null::NullTranslator;

/// Build a translator from settings.
///
/// Returns `NullTranslator` if the settings are incomplete or the feature
/// is disabled, so callers can always use `translator.translate()` without
/// checking configuration first.
pub fn translator_from_settings(
    enabled: bool,
    provider: LlmProvider,
    api_key: &str,
    api_base_url: &str,
    model: &str,
) -> Box<dyn Translator> {
    if !enabled {
        return Box::new(NullTranslator);
    }

    match provider {
        LlmProvider::Anthropic => {
            if api_key.is_empty() {
                tracing::debug!("translate: Anthropic selected but no API key, using NullTranslator");
                return Box::new(NullTranslator);
            }
            let client = anthropic::AnthropicTranslator::builder()
                .api_key(api_key.to_string())
                .model(if model.is_empty() {
                    anthropic::DEFAULT_MODEL.to_string()
                } else {
                    model.to_string()
                })
                .maybe_base_url(if api_base_url.is_empty() {
                    None
                } else {
                    Some(api_base_url.to_string())
                })
                .build();
            Box::new(client)
        }
        LlmProvider::OpenAiCompatible => {
            if api_key.is_empty() || api_base_url.is_empty() {
                tracing::debug!(
                    "translate: OpenAI-compatible selected but missing config, using NullTranslator"
                );
                return Box::new(NullTranslator);
            }
            let client = openai::OpenaiTranslator::builder()
                .api_key(api_key.to_string())
                .base_url(api_base_url.to_string())
                .model(if model.is_empty() {
                    openai::DEFAULT_MODEL.to_string()
                } else {
                    model.to_string()
                })
                .build();
            Box::new(client)
        }
    }
}
