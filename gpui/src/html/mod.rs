//! Lightweight HTML rendering for MDX definition content.
//!
//! MDX definitions arrive as small fragments of HTML. We tokenize them
//! into a flat stream of structural events, fold those into a list of
//! block + inline nodes, then turn the nodes into a GPUI element tree.
//! Inline images and sounds are pulled out of the companion MDD file.

pub mod css;
pub mod parser;
pub mod render;

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

pub use css::Stylesheet;
pub use parser::{Block, first_sound_path, parse_with_styles};
pub use render::render_blocks;

/// Per-dictionary stylesheets, keyed by dictionary name (file stem).
/// Loaded at startup; each entry holds the merged CSS for the
/// directory containing that dictionary's `.mdx` file. This keeps
/// rules from different dictionaries from cross-contaminating each
/// other — e.g. an LSC4 rule like `.hw { display: none }` would
/// otherwise wipe out the Cambridge headword class with the same
/// name but different semantics.
pub static STYLESHEETS: LazyLock<RwLock<HashMap<String, Stylesheet>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Parse HTML against the stylesheet registered for `dict_name`.
/// Falls back to an empty stylesheet if none is registered.
pub fn parse_styled(html: &str, dict_name: &str) -> Vec<Block> {
    let map = STYLESHEETS.read().unwrap();
    let empty = Stylesheet::default();
    let sheet = map.get(dict_name).unwrap_or(&empty);
    parse_with_styles(html, sheet)
}

pub fn set_dict_styles(map: HashMap<String, Stylesheet>) {
    *STYLESHEETS.write().unwrap() = map;
}
