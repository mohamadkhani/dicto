//! RTL-aware text preparation for GPUI's text pipeline.
//!
//! On Linux, GPUI shapes text with cosmic-text, which implements the Unicode
//! Bidirectional Algorithm correctly for a SINGLE line: glyphs come out in
//! visual order with correct joined letter forms and ligatures. Two things
//! remain broken for RTL content, and both are fixed WITHOUT touching the
//! character order (any manual reordering would double-apply bidi and corrupt
//! letter forms):
//!
//! 1. GPUI's line wrapping (`compute_wrap_boundaries`) walks glyphs in visual
//!    order but reasons about byte indices as if visual order == logical
//!    order. For RTL lines the two orders are inverted, so wrapped fragments
//!    are cut and stacked incorrectly (scrambled paragraphs).
//!    → We wrap ourselves: break ONLY at spaces (never inside a word), using
//!    real shaped token widths ([`wrap_line_segments`]). The editor paints the
//!    resulting segments as hard lines and never lets GPUI re-wrap them.
//! 2. Every line is painted flush LEFT, but an RTL paragraph must hug the
//!    RIGHT edge. The paragraph direction comes from its first strong
//!    character (UAX #9 P2/P3).
//!    → [`first_paragraph_is_rtl`] reports the direction; the editor
//!    ([`crate::components::text_editor`]) paints RTL lines with a per-line
//!    x offset that pushes them against the right edge.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{Font, Pixels, TextRun, WindowTextSystem, black, px};
use unicode_bidi::BidiInfo;

/// True if `text` contains characters from right-to-left scripts (Arabic —
/// including Persian — Hebrew, Thaana, N'Ko, Syriac, Samaritan, Mandaic,
/// Adlam, and their presentation forms).
pub(crate) fn contains_rtl(text: &str) -> bool {
    text.chars().any(is_rtl_script_char)
}

pub(crate) fn is_rtl_script_char(c: char) -> bool {
    matches!(c as u32,
        0x0590..=0x05FF      // Hebrew
        | 0x0600..=0x06FF    // Arabic (incl. Persian)
        | 0x0700..=0x074F    // Syriac
        | 0x0750..=0x077F    // Arabic Supplement
        | 0x0780..=0x07BF    // Thaana
        | 0x07C0..=0x07FF    // N'Ko
        | 0x0800..=0x083E    // Samaritan
        | 0x0840..=0x085F    // Mandaic
        | 0x08A0..=0x08FF    // Arabic Extended-A
        | 0xFB1D..=0xFDFF    // Hebrew & Arabic Presentation Forms-A
        | 0xFE70..=0xFEFF    // Arabic Presentation Forms-B
        | 0x1E900..=0x1E95F  // Adlam
    )
}

/// Direction of `text`'s FIRST paragraph (UAX #9 P2/P3): true when its first
/// strong character is RTL. Empty text and whitespace-only text are LTR.
pub(crate) fn first_paragraph_is_rtl(text: &str) -> bool {
    // Cheap pre-check: UAX #9 assigns LTR to any text without an RTL char,
    // and running BidiInfo per keystroke is not free.
    contains_rtl(text)
        && BidiInfo::new(text, None)
            .paragraphs
            .first()
            .is_some_and(|para| para.level.is_rtl())
}

/// Wrap `text` into EDITING segments: the byte range of each wrapped line,
/// breaking ONLY at spaces (never inside a word) with real shaped widths,
/// spanning all hard `\n` paragraphs, and WITHOUT the display-only
/// right-alignment padding — padding spaces would pollute the edited string.
///
/// Byte ranges are global (into the whole `text`). Spaces at a wrap point
/// belong to no segment (they are invisible there); empty hard lines yield an
/// empty segment so every cursor position still maps to a line.
pub(crate) fn wrap_line_segments(
    text_system: &Arc<WindowTextSystem>,
    text: &str,
    font: Font,
    font_size: Pixels,
    wrap_width: Pixels,
) -> Vec<std::ops::Range<usize>> {
    let mut measurer = ShapedWidthMeasurer::new(text_system.clone(), font, font_size);
    wrap_segments_with_measurer(text, wrap_width, &mut |token| measurer.measure(token))
}

/// Pure segment wrapping, decoupled from the text system so tests can inject
/// synthetic token widths.
fn wrap_segments_with_measurer(
    text: &str,
    wrap_width: Pixels,
    measure: &mut dyn FnMut(&str) -> Pixels,
) -> Vec<std::ops::Range<usize>> {
    let mut segments = Vec::new();
    let mut offset = 0usize;
    for hard_line in text.split('\n') {
        let line_segments = wrap_at_spaces(hard_line, wrap_width, measure);
        if line_segments.is_empty() {
            segments.push(offset..offset);
        } else {
            for segment in line_segments {
                segments.push(offset + segment.start..offset + segment.end);
            }
        }
        offset += hard_line.len() + 1; // +1 for the '\n'
    }
    segments
}

/// Greedy logical wrapping of one hard line, breaking ONLY at spaces (an RTL
/// word must never be split). Token widths come from `measure` (whole tokens,
/// shaped, so fallback fonts are accounted for). Returns byte ranges into
/// `line`; trailing spaces at a break point are dropped.
fn wrap_at_spaces(
    line: &str,
    wrap_width: Pixels,
    measure: &mut dyn FnMut(&str) -> Pixels,
) -> Vec<std::ops::Range<usize>> {
    let bytes = line.as_bytes();
    let mut segments: Vec<std::ops::Range<usize>> = Vec::new();
    let mut line_start = 0usize;
    let mut line_end = 0usize;
    let mut line_width = px(0.);
    let mut has_word = false;
    let mut i = 0usize;

    while i < bytes.len() {
        let spaces_start = i;
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        let word_start = i;
        while i < bytes.len() && bytes[i] != b' ' {
            i += 1;
        }
        if word_start == i {
            break; // trailing spaces — nothing to add
        }
        let word_width = measure(&line[word_start..i]);
        let spaces_width = if has_word {
            measure(&line[spaces_start..word_start])
        } else {
            px(0.)
        };

        if has_word && line_width + spaces_width + word_width > wrap_width {
            segments.push(line_start..spaces_start);
            line_start = word_start;
            line_end = i;
            line_width = word_width;
        } else {
            line_width += spaces_width + word_width;
            line_end = i;
            has_word = true;
        }
    }

    if has_word {
        segments.push(line_start..line_end);
    } else if !line.is_empty() {
        // Whitespace-only hard line: keep verbatim.
        segments.push(0..line.len());
    }
    segments
}

/// Measures token widths with the REAL text-system shaping (`layout_line`),
/// so font fallback (a Latin UI font shaping Persian glyphs via a fallback
/// font) is reflected in the widths. Results are cached per token.
struct ShapedWidthMeasurer {
    text_system: Arc<WindowTextSystem>,
    font: Font,
    font_size: Pixels,
    cache: HashMap<String, Pixels>,
}

impl ShapedWidthMeasurer {
    fn new(text_system: Arc<WindowTextSystem>, font: Font, font_size: Pixels) -> Self {
        Self {
            text_system,
            font,
            font_size,
            cache: HashMap::new(),
        }
    }

    fn measure(&mut self, token: &str) -> Pixels {
        if let Some(width) = self.cache.get(token) {
            return *width;
        }
        // The run's color/decoration are irrelevant for width measurement.
        let run = TextRun {
            len: token.len(),
            font: self.font.clone(),
            color: black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let width = self
            .text_system
            .layout_line(token, self.font_size, &[run], None)
            .width;
        self.cache.insert(token.to_string(), width);
        width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1px per character — makes wrap thresholds trivial to reason about.
    fn per_char_measurer(width_per_char: f32) -> impl FnMut(&str) -> Pixels {
        move |token: &str| px(token.chars().count() as f32 * width_per_char)
    }

    #[test]
    fn contains_rtl_detects_arabic_and_hebrew() {
        assert!(contains_rtl("سلام"));
        assert!(contains_rtl("mixed שלום text"));
        assert!(!contains_rtl("plain English 123"));
        // ZWNJ alone (common inside Persian words) is not an RTL script char.
        assert!(!contains_rtl("\u{200C}"));
    }

    #[test]
    fn first_paragraph_direction_follows_first_strong_char() {
        assert!(first_paragraph_is_rtl("سلام"));
        assert!(first_paragraph_is_rtl("  سلام abc")); // whitespace is neutral
        assert!(!first_paragraph_is_rtl("abc سلام"));
        assert!(!first_paragraph_is_rtl(""));
        assert!(!first_paragraph_is_rtl("   "));
        // Direction is per first paragraph: a Latin first line stays LTR even
        // when a later paragraph is RTL.
        assert!(!first_paragraph_is_rtl("hello\nسلام"));
        assert!(first_paragraph_is_rtl("سلام\nhello"));
    }

    #[test]
    fn wrap_segments_split_at_newlines_with_global_offsets() {
        let mut m = per_char_measurer(1.);
        let text = "سلام\nدنیا";
        let segs = wrap_segments_with_measurer(text, px(100.), &mut m);
        assert_eq!(segs.len(), 2);
        assert_eq!(&text[segs[0].clone()], "سلام");
        // The second hard line starts after the '\n'.
        assert_eq!(segs[1].start, "سلام".len() + 1);
        assert_eq!(&text[segs[1].clone()], "دنیا");
    }

    #[test]
    fn wrap_segments_empty_hard_line_gets_empty_segment() {
        let mut m = per_char_measurer(1.);
        let segs = wrap_segments_with_measurer("a\n\nc", px(100.), &mut m);
        assert_eq!(segs, vec![0..1, 2..2, 3..4]);
    }

    #[test]
    fn wrap_segments_exclude_break_spaces() {
        // 1px/char: "aa" (2px) fits at width 2; the following space+word does
        // not, so the line wraps — and the break space belongs to no segment.
        let mut m = per_char_measurer(1.);
        let segs = wrap_segments_with_measurer("aa bb", px(2.), &mut m);
        assert_eq!(segs, vec![0..2, 3..5]);
    }

    #[test]
    fn wrap_segments_never_split_words() {
        // 2px/char: two 4-char words (8px each) can never share a 12px line.
        let mut m = per_char_measurer(2.);
        let text = "سلام دنیا";
        let segs = wrap_segments_with_measurer(text, px(12.), &mut m);
        assert_eq!(segs.len(), 2);
        assert_eq!(&text[segs[0].clone()], "سلام");
        assert_eq!(&text[segs[1].clone()], "دنیا");
    }
}
