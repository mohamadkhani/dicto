/// A single dictionary's hit for a word lookup.
#[derive(Debug, Clone)]
pub struct DictHit {
    /// Human-readable dictionary name (from MDX header Title, or file stem).
    pub name: String,
    /// Short name for tab titles (abbreviation or full title if short enough).
    pub short_name: String,
    /// File stem for stylesheet lookup and internal routing.
    pub stem: String,
    /// HTML or plain-text definition.
    pub definition: String,
}

/// Metadata about a dictionary, read from its header.
#[derive(Debug, Clone)]
pub struct DictInfo {
    /// Display title (e.g. "Merriam-Webster's Advanced Learner's English Dictionary").
    pub title: String,
    /// Description from the header (may contain HTML entities).
    pub description: String,
    /// File stem used for index paths and stylesheet lookup.
    pub stem: String,
    /// Full path to the MDX file.
    pub path: String,
    /// Text encoding declared in the header (e.g. "UTF-8").
    pub encoding: String,
    /// MDX engine version (1 or 2).
    pub version: u8,
}

/// Core interface every dictionary format must implement.
///
/// All methods that touch the index or source file are fallible.
/// A format that doesn't support resources (e.g. a plain word list)
/// simply returns `Ok(None)` from `resource` and an empty vec from
/// `css_resources`.
pub trait Dictionary: Send + Sync {
    /// Human-readable name shown in the UI (MDX header Title, or file stem).
    fn name(&self) -> &str;

    /// Short name for tab titles (abbreviation if title is long).
    fn short_name(&self) -> &str;

    /// File stem for index paths and stylesheet lookup.
    fn stem(&self) -> &str;

    /// Metadata read from the dictionary header.
    fn info(&self) -> DictInfo;

    /// Look up a word. Returns the HTML/text definition or `None` if not
    /// found. Redirect resolution (e.g. `@@@LINK=`) is handled internally.
    fn lookup(&self, word: &str) -> Option<String>;

    /// Prefix search. Returns up to `limit` matching headwords.
    fn suggestions(&self, prefix: &str, limit: usize) -> Vec<String>;

    /// Fuzzy near-matches: headwords within a small edit distance of
    /// `word`, excluding the word itself. Backs the "related words"
    /// suggestions shown when a lookup misses (or between near forms).
    fn related_words(&self, word: &str, limit: usize) -> Vec<String>;

    /// Fetch a binary resource (image, audio, font) by virtual path.
    /// Returns `None` if this dictionary has no such resource.
    fn resource(&self, path: &str) -> Option<Vec<u8>>;

    /// CSS stylesheets bundled with this dictionary as `(filename, body)` pairs.
    fn css_resources(&self) -> Vec<(String, String)>;

    /// Build (or rebuild) the on-disk search index for this dictionary.
    /// `force = true` removes and recreates an existing index.
    fn build_index(&self, force: bool) -> anyhow::Result<()>;

    /// True if a complete, usable index already exists on disk.
    fn index_ready(&self) -> bool;
}
