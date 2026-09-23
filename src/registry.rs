use std::sync::{LazyLock, RwLock};

use tracing::warn;

use crate::dictionary::{DictHit, Dictionary};
use crate::formats::detect;
use crate::settings::enabled_mdx;

// ── DictionaryRegistry ────────────────────────────────────────────────────────

pub struct DictionaryRegistry {
    dictionaries: Vec<Box<dyn Dictionary>>,
}

impl DictionaryRegistry {
    pub fn from_settings() -> Self {
        let dicts = enabled_mdx()
            .into_iter()
            .filter_map(|path| {
                let d = detect(&path)?;
                if !d.index_ready() {
                    warn!("registry: index not ready for {path}, skipping");
                    return None;
                }
                Some(d)
            })
            .collect();
        DictionaryRegistry {
            dictionaries: dicts,
        }
    }

    /// Build (or rebuild) indexes for all configured MDX files, then reload.
    pub fn ensure_indexed(&self, force: bool) {
        for d in &self.dictionaries {
            if let Err(e) = d.build_index(force) {
                warn!("registry: indexing failed for {}: {e}", d.name());
            }
        }
    }

    pub fn query_all(&self, word: &str) -> Vec<DictHit> {
        self.dictionaries
            .iter()
            .filter_map(|d| {
                d.lookup(word).map(|def| DictHit {
                    name: d.name().to_string(),
                    short_name: d.short_name().to_string(),
                    stem: d.stem().to_string(),
                    definition: def,
                })
            })
            .collect()
    }

    pub fn suggestions(&self, prefix: &str, limit: usize) -> Vec<String> {
        let p = prefix.to_lowercase();
        let mut exact: Vec<String> = Vec::new();
        let mut rest: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Consult every dictionary before stopping: an exact match in a
        // later dictionary must outrank fuzzy matches from earlier ones.
        // Index keys are lowercased at build time, so `word == p` is the
        // exact-match test.
        for d in &self.dictionaries {
            for word in d.suggestions(prefix, limit) {
                if seen.insert(word.clone()) {
                    if word == p {
                        exact.push(word);
                    } else {
                        rest.push(word);
                    }
                }
            }
        }

        exact.extend(rest);
        exact.truncate(limit);
        exact
    }

    pub fn lookup_resource(&self, path: &str) -> Option<Vec<u8>> {
        self.dictionaries.iter().find_map(|d| d.resource(path))
    }

    pub fn css_for_dict(&self, name: &str) -> Vec<(String, String)> {
        self.dictionaries
            .iter()
            .find(|d| d.name() == name)
            .map(|d| d.css_resources())
            .unwrap_or_default()
    }

    pub fn all_css(&self) -> Vec<(String, Vec<(String, String)>)> {
        self.dictionaries
            .iter()
            .map(|d| (d.name().to_string(), d.css_resources()))
            .collect()
    }
}

// ── global registry ───────────────────────────────────────────────────────────

static REGISTRY: LazyLock<RwLock<DictionaryRegistry>> = LazyLock::new(|| {
    RwLock::new(DictionaryRegistry {
        dictionaries: vec![],
    })
});

/// Initialize or reload the registry from current settings.
/// Must be called after indexing completes.
pub fn reload() {
    *REGISTRY.write().unwrap() = DictionaryRegistry::from_settings();
}

pub fn query_all(word: &str) -> Vec<DictHit> {
    REGISTRY.read().unwrap().query_all(word)
}

pub fn suggestions(prefix: &str, limit: usize) -> Vec<String> {
    REGISTRY.read().unwrap().suggestions(prefix, limit)
}

pub fn lookup_resource(path: &str) -> Option<Vec<u8>> {
    REGISTRY.read().unwrap().lookup_resource(path)
}

pub fn css_for_dict(name: &str) -> Vec<(String, String)> {
    REGISTRY.read().unwrap().css_for_dict(name)
}

pub fn all_css() -> Vec<(String, Vec<(String, String)>)> {
    REGISTRY.read().unwrap().all_css()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockDict {
        words: Vec<&'static str>,
    }

    impl Dictionary for MockDict {
        fn name(&self) -> &str {
            "mock"
        }
        fn short_name(&self) -> &str {
            "mock"
        }
        fn stem(&self) -> &str {
            "mock"
        }
        fn info(&self) -> crate::dictionary::DictInfo {
            unimplemented!()
        }
        fn lookup(&self, _word: &str) -> Option<String> {
            unimplemented!()
        }
        fn suggestions(&self, prefix: &str, limit: usize) -> Vec<String> {
            let p = prefix.to_lowercase();
            self.words
                .iter()
                .filter(|w| w.starts_with(&p))
                .take(limit)
                .map(|w| w.to_string())
                .collect()
        }
        fn resource(&self, _path: &str) -> Option<Vec<u8>> {
            unimplemented!()
        }
        fn css_resources(&self) -> Vec<(String, String)> {
            unimplemented!()
        }
        fn build_index(&self, _force: bool) -> anyhow::Result<()> {
            unimplemented!()
        }
        fn index_ready(&self) -> bool {
            true
        }
    }

    fn registry(dicts: Vec<MockDict>) -> DictionaryRegistry {
        DictionaryRegistry {
            dictionaries: dicts
                .into_iter()
                .map(|d| Box::new(d) as Box<dyn Dictionary>)
                .collect(),
        }
    }

    #[test]
    fn exact_match_in_later_dict_ranks_first() {
        let r = registry(vec![
            MockDict {
                words: vec!["runnable", "running"],
            },
            MockDict {
                words: vec!["runner", "run"],
            },
        ]);
        let got = r.suggestions("run", 50);
        assert_eq!(got.first().map(String::as_str), Some("run"));
    }

    #[test]
    fn exact_match_survives_when_earlier_dict_fills_limit() {
        let mut words: Vec<&'static str> = (0..50)
            .map(|i| Box::leak(format!("run{i}").into_boxed_str()) as &'static str)
            .collect();
        words.push("run");
        let r = registry(vec![MockDict { words }, MockDict { words: vec!["run"] }]);
        let got = r.suggestions("run", 50);
        assert_eq!(got.first().map(String::as_str), Some("run"));
        assert_eq!(got.len(), 50);
    }

    #[test]
    fn no_exact_match_keeps_dictionary_order() {
        let r = registry(vec![
            MockDict {
                words: vec!["catapult", "catch"],
            },
            MockDict {
                words: vec!["catalog"],
            },
        ]);
        let got = r.suggestions("cat", 50);
        assert_eq!(got, vec!["catapult", "catch", "catalog"]);
    }
}
