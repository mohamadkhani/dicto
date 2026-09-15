use crate::{TranslateError, Translator, TranslationRequest, TranslationResult};

/// No-op translator used when translation is disabled or not configured.
///
/// Always returns `TranslateError::NotConfigured`. This lets callers use
/// `translator.translate()` uniformly without checking `is_configured()`
/// first — they just handle the error.
pub struct NullTranslator;

impl Translator for NullTranslator {
    fn translate(&self, _request: TranslationRequest) -> Result<TranslationResult, TranslateError> {
        Err(TranslateError::NotConfigured)
    }

    fn is_configured(&self) -> bool {
        false
    }

    fn provider_name(&self) -> &'static str {
        "null"
    }
}
