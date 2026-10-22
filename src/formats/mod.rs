pub mod mdict;

use std::path::Path;

use mdict::MdxDictionary;

use crate::dictionary::Dictionary;

/// Detect the format of `path` and return the appropriate Dictionary implementation.
/// Returns `None` if the format is not recognised or the file does not exist.
pub fn detect(path: &str) -> Option<Box<dyn Dictionary>> {
    let ext = Path::new(path).extension()?.to_ascii_lowercase();
    match ext.to_string_lossy().as_ref() {
        "mdx" => Some(Box::new(MdxDictionary::new(path))),
        _ => None,
    }
}
