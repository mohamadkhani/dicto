use crate::dictionary::DictHit;
use crate::registry;

/// Query all enabled dictionaries. Each hit carries the dictionary name and its definition.
pub fn query_all(word: &str) -> Vec<DictHit> {
    registry::query_all(word)
}

/// Convenience: return only the first definition, or "not found".
pub fn query(word: String) -> String {
    query_all(&word)
        .into_iter()
        .next()
        .map(|h| h.definition)
        .unwrap_or_else(|| "not found".to_string())
}

/// Prefix search across all enabled dictionaries, deduplicated, up to `limit` results.
pub fn search_suggestions(prefix: &str, limit: usize) -> Vec<String> {
    registry::suggestions(prefix, limit)
}

/// Fuzzy near-matches for `word` across all enabled dictionaries,
/// deduplicated, excluding the word itself. Backs the popup's horizontal
/// "related words" row.
pub fn related_words(word: &str, limit: usize) -> Vec<String> {
    registry::related_words(word, limit)
}

/// Look up a binary resource (image, audio, etc.) across all enabled dictionaries.
pub fn lookup_resource(path: &str) -> Option<Vec<u8>> {
    registry::lookup_resource(path)
}

/// CSS stylesheets from the dictionary named `dict_name`.
pub fn css_for_dict(dict_name: &str) -> Vec<(String, String)> {
    registry::css_for_dict(dict_name)
}
