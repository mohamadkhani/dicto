//! Font fallback wiring for Persian / Arabic-script text.
//!
//! The theme's primary family (`.SystemUIFont`) carries no Persian glyphs,
//! so GPUI's per-glyph fallback would otherwise drop Persian text to
//! whatever fontconfig ranks first for the script — a poor match at UI
//! sizes. Every window root cascades [`PERSIAN_FALLBACK`] as the first
//! fallback family so Persian renders in Vazirmatn while Latin keeps the
//! theme font.

use gpui::{Font, FontFallbacks, Window};

/// Preferred family for Arabic-script (Persian) glyphs.
pub const PERSIAN_FALLBACK: &str = "Vazirmatn";

/// The window's resolved text-style font with [`PERSIAN_FALLBACK`]
/// prepended to its fallback chain. Cascade it from a window's root
/// element so it reaches every descendant text style.
pub fn persian_fallback_font(window: &Window) -> Font {
    let style = window.text_style();
    let mut fallbacks = style
        .font_fallbacks
        .clone()
        .map(|f| f.fallback_list().to_vec())
        .unwrap_or_default();
    if !fallbacks.iter().any(|f| f == PERSIAN_FALLBACK) {
        fallbacks.insert(0, PERSIAN_FALLBACK.to_string());
    }
    Font {
        family: style.font_family.clone(),
        features: style.font_features.clone(),
        fallbacks: Some(FontFallbacks::from_fonts(fallbacks)),
        weight: style.font_weight,
        style: style.font_style,
    }
}
