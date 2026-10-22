//! Sentence chunking + word-timing estimation for TTS word highlighting.
//!
//! The playback controller speaks the clip text in SENTENCE CHUNKS: each chunk
//! is synthesized and played separately (first word of a long text starts after
//! the first sentence's synthesis, not the whole text). Highlighting a spoken
//! word needs a time→word mapping; we do not get word timestamps from any
//! OpenAI-compatible TTS endpoint, so within a chunk we ESTIMATE: speech rate
//! is roughly uniform over a short sentence, so a word's duration is
//! proportional to its character count, plus a pause weight after punctuation.
//! Because chunk boundaries are exact (a chunk boundary is a real audio
//! boundary), estimate error cannot accumulate past one chunk.
//!
//! Pure logic — no gpui types — so it is unit-testable (see the tests below).

use std::ops::Range;
use std::time::Duration;

/// One word inside a chunk: its byte range in the FULL clip text plus the
/// relative duration weight used to divide the chunk's audio between words.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WordSpan {
    pub(crate) range: Range<usize>,
    pub(crate) weight: f32,
}

/// Estimated word layout of one sentence chunk.
#[derive(Clone, Debug, Default)]
pub(crate) struct ChunkTiming {
    /// Byte range of the chunk in the full clip text.
    pub(crate) range: Range<usize>,
    /// Words in reading order (logical order — the editor's paint maps byte
    /// ranges to visual positions itself, RTL included).
    pub(crate) words: Vec<WordSpan>,
    /// Sum of all word weights (the chunk's duration divides by this).
    pub(crate) total_weight: f32,
}

impl ChunkTiming {
    /// The byte range of the word spoken at `elapsed` into a chunk of
    /// `duration`. Weights map linearly onto time: cumulative weight /
    /// total weight == elapsed fraction. Returns the last word when `elapsed`
    /// lands in the trailing pause of a final punctuation mark.
    pub(crate) fn word_at(&self, elapsed: Duration, duration: Duration) -> Option<Range<usize>> {
        if self.words.is_empty() || duration.is_zero() {
            return None;
        }
        let frac = (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0);
        let target = frac * self.total_weight;
        let mut cum = 0.0;
        let last = self.words.len() - 1;
        for (i, word) in self.words.iter().enumerate() {
            cum += word.weight;
            // The last word also owns the trailing pause of a final
            // punctuation mark, so it is the fallback at the end of the clip.
            if target < cum || i == last {
                return Some(word.range.clone());
            }
        }
        None
    }
}

/// Split `text` into sentence-chunk byte ranges (logical order). Splits after
/// terminal punctuation (`.`, `!`, `?`, `؟`, `…`, `؛`, `;`) and at newlines.
/// Chunks are trimmed and non-empty. An imperfect split is harmless — a chunk
/// is just the unit of synthesis + timing reset — so this stays deliberately
/// simple; the one guard is single-letter abbreviations (`J.`, `A.`) which
/// must not split.
pub(crate) fn split_sentences(text: &str) -> Vec<Range<usize>> {
    let mut chunks: Vec<Range<usize>> = Vec::new();
    let mut start = 0usize;
    for (i, c) in text.char_indices() {
        if is_terminal(c) && !is_abbreviation(text, i, c) {
            chunks.push(start..i + c.len_utf8());
            start = i + c.len_utf8();
        } else if c == '\n' {
            // A paragraph break is a natural chunk boundary (the newline
            // itself belongs to neither chunk after trimming).
            chunks.push(start..i);
            start = i + c.len_utf8();
        }
    }
    if start < text.len() {
        chunks.push(start..text.len());
    }
    chunks.iter_mut().for_each(|r| trim_range(text, r));
    chunks.retain(|r| !r.is_empty());
    chunks
}

/// The punctuation characters that end a TTS chunk.
fn is_terminal(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '؟' | '…' | '؛' | ';')
}

/// Whether the terminal punctuation at byte `i` is part of an abbreviation
/// (or an ellipsis of one) rather than a sentence end: the previous "word" is
/// a single letter (`J.`, `A.`). Multi-letter abbreviations (`Dr.`) still
/// split — harmless.
fn is_abbreviation(text: &str, i: usize, c: char) -> bool {
    if c != '.' {
        return false;
    }
    let before = &text[..i];
    let prev = before.chars().next_back();
    match prev {
        Some(p) if p.is_alphabetic() => {
            let earlier = before[..before.len() - p.len_utf8()].chars().next_back();
            earlier.is_none_or(|e| e.is_whitespace())
        }
        // "…" already matched as one char; a '.' after '…' is not a boundary.
        Some('…') => true,
        _ => false,
    }
}

fn trim_range(text: &str, r: &mut Range<usize>) {
    while r.start < r.end && text[r.start..].starts_with(char::is_whitespace) {
        r.start += text[r.start..].chars().next().unwrap().len_utf8();
    }
    while r.end > r.start && text[..r.end].ends_with(char::is_whitespace) {
        r.end -= text[..r.end].chars().next_back().unwrap().len_utf8();
    }
}

/// Build the estimated word layout for one chunk (`range` into `text`).
///
/// Word weight = character count (min 1) + a pause bonus for punctuation at
/// the word's end: sentence-terminal +2.0, comma-class +1.0. The absolute
/// scale cancels out (weights map onto the chunk's real duration), so these
/// numbers only need to be proportionally right.
pub(crate) fn chunk_timing(text: &str, range: Range<usize>) -> ChunkTiming {
    let chunk = &text[range.clone()];
    let mut words = Vec::new();
    let mut total = 0.0;
    let mut word_start: Option<usize> = None;
    let mut weight = 0.0;
    for (i, c) in chunk.char_indices() {
        let abs = range.start + i;
        if c.is_whitespace() {
            if let Some(s) = word_start.take() {
                total += weight;
                words.push(WordSpan {
                    range: s..abs,
                    weight,
                });
                weight = 0.0;
            }
        } else {
            if word_start.is_none() {
                word_start = Some(abs);
            }
            weight += 1.0;
            if is_terminal(c) {
                weight += 2.0;
            } else if matches!(c, ',' | '،' | ':' | '(' | ')') {
                weight += 1.0;
            }
        }
    }
    if let Some(s) = word_start {
        total += weight;
        words.push(WordSpan {
            range: s..range.end,
            weight,
        });
    }
    ChunkTiming {
        range,
        words,
        total_weight: total,
    }
}

/// Estimated word timings for every chunk of `text`, parallel to
/// [`split_sentences`].
pub(crate) fn timings_for(text: &str) -> Vec<ChunkTiming> {
    split_sentences(text)
        .into_iter()
        .map(|range| chunk_timing(text, range))
        .collect()
}

// ---------------------------------------------------------------------------
// Pure chunk-timeline math (unit-tested; the controller applies it to sinks)
// ---------------------------------------------------------------------------

/// Start time of each chunk on the clip timeline. Known durations are used
/// as-is; chunks not yet decoded are estimated from the average
/// seconds-per-weight of the known chunks (falling back to a default rate).
/// Returns one entry per chunk (len == durations.len()).
pub(crate) fn chunk_starts(durations: &[Option<Duration>], weights: &[f32]) -> Vec<Duration> {
    let est = estimate_secs_per_weight(durations, weights);
    let mut starts = Vec::with_capacity(durations.len());
    let mut t = Duration::ZERO;
    for (d, w) in durations.iter().zip(weights) {
        starts.push(t);
        t += d.unwrap_or_else(|| Duration::from_secs_f32(w * est));
    }
    starts
}

/// Total timeline length: Σ known durations + estimates for the rest.
pub(crate) fn total_duration(durations: &[Option<Duration>], weights: &[f32]) -> Duration {
    let est = estimate_secs_per_weight(durations, weights);
    durations
        .iter()
        .zip(weights)
        .map(|(d, w)| d.unwrap_or_else(|| Duration::from_secs_f32(w * est)))
        .sum()
}

/// Average seconds per timing-weight across DECODED chunks; a neutral default
/// (~15 chars/sec is a natural speech rate) when nothing is decoded yet.
fn estimate_secs_per_weight(durations: &[Option<Duration>], weights: &[f32]) -> f32 {
    let mut secs = 0.0;
    let mut weight = 0.0;
    for (d, w) in durations.iter().zip(weights) {
        if let Some(d) = d {
            secs += d.as_secs_f32();
            weight += w;
        }
    }
    if weight > 0.0 { secs / weight } else { 0.065 }
}

/// Map a timeline position to (chunk index, elapsed within the chunk).
pub(crate) fn locate(
    position: Duration,
    durations: &[Option<Duration>],
    weights: &[f32],
) -> Option<(usize, Duration)> {
    let starts = chunk_starts(durations, weights);
    let mut result = None;
    for (i, start) in starts.iter().enumerate() {
        if position >= *start {
            result = Some((i, position - *start));
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(text: &str) -> Vec<(usize, usize)> {
        split_sentences(text)
            .into_iter()
            .map(|r| (r.start, r.end))
            .collect()
    }

    fn slice<'a>(text: &'a str, r: &(usize, usize)) -> &'a str {
        &text[r.0..r.1]
    }

    #[test]
    fn splits_english_sentences() {
        let text = "Hello world. How are you?";
        assert_eq!(
            ranges(text)
                .into_iter()
                .map(|r| slice(text, &r).to_string())
                .collect::<Vec<_>>(),
            vec!["Hello world.", "How are you?"]
        );
    }

    #[test]
    fn splits_persian_sentences() {
        let text = "سلام دنیا. حال شما چطور است؟ خوبم";
        assert_eq!(
            ranges(text)
                .into_iter()
                .map(|r| slice(text, &r).to_string())
                .collect::<Vec<_>>(),
            vec!["سلام دنیا.", "حال شما چطور است؟", "خوبم"]
        );
    }

    #[test]
    fn single_letter_abbreviation_does_not_split() {
        let text = "John F. Kennedy spoke.";
        assert_eq!(
            ranges(text)
                .into_iter()
                .map(|r| slice(text, &r).to_string())
                .collect::<Vec<_>>(),
            vec![text]
        );
    }

    #[test]
    fn newline_is_a_boundary() {
        let text = "first line\nsecond line";
        assert_eq!(
            ranges(text)
                .into_iter()
                .map(|r| slice(text, &r).to_string())
                .collect::<Vec<_>>(),
            vec!["first line", "second line"]
        );
    }

    #[test]
    fn empty_and_whitespace_text_yield_no_chunks() {
        assert!(split_sentences("").is_empty());
        assert!(split_sentences("  \n ").is_empty());
    }

    #[test]
    fn word_ranges_cover_words_not_spaces() {
        let text = "سلام دنیا. خوبم";
        let timings = timings_for(text);
        assert_eq!(timings.len(), 2);
        let words: Vec<&str> = timings[0]
            .words
            .iter()
            .map(|w| &text[w.range.clone()])
            .collect();
        assert_eq!(words, vec!["سلام", "دنیا."]);
    }

    #[test]
    fn word_at_maps_fraction_to_words_in_order() {
        let text = "aaa bbb ccc ddd.";
        let timing = timings_for(text).remove(0);
        // weights: 3,3,3,3+2(pause) = 14 total
        let dur = Duration::from_secs_f32(14.0);
        assert_eq!(timing.word_at(Duration::ZERO, dur), Some(0..3));
        assert_eq!(
            timing.word_at(Duration::from_secs_f32(4.0), dur),
            Some(4..7)
        );
        assert_eq!(
            timing.word_at(Duration::from_secs_f32(13.9), dur),
            Some(12..16)
        );
        // At the very end the last word stays highlighted.
        assert_eq!(timing.word_at(dur, dur), Some(12..16));
    }

    #[test]
    fn word_at_handles_multibyte_offsets() {
        // Each Persian letter is 2 bytes: سلام = 0..8, دنیا = 9..17.
        let text = "سلام دنیا";
        let timing = timings_for(text).remove(0);
        let dur = Duration::from_secs_f32(2.0);
        assert_eq!(timing.word_at(Duration::ZERO, dur), Some(0..8));
        assert_eq!(
            timing.word_at(Duration::from_secs_f32(1.5), dur),
            Some(9..17)
        );
    }

    #[test]
    fn chunk_starts_estimates_unknown_durations_proportionally() {
        let d = Duration::from_secs_f32;
        // 3 chunks, first two decoded (2s, 4s), third unknown.
        let durations = vec![Some(d(2.0)), Some(d(4.0)), None];
        let weights = vec![10.0, 20.0, 10.0];
        let starts = chunk_starts(&durations, &weights);
        // Known rate: 6s / 30 weight = 0.2 s/weight → third chunk ≈ 2s.
        assert_eq!(starts[0], Duration::ZERO);
        assert_eq!(starts[1], d(2.0));
        assert_eq!(starts[2], d(6.0));
        let total = total_duration(&durations, &weights);
        assert_eq!(total, d(8.0));
    }

    #[test]
    fn locate_finds_chunk_and_elapsed() {
        let d = Duration::from_secs_f32;
        let durations = vec![Some(d(2.0)), Some(d(3.0))];
        let weights = vec![10.0, 10.0];
        assert_eq!(locate(d(0.5), &durations, &weights), Some((0, d(0.5))));
        assert_eq!(locate(d(2.5), &durations, &weights), Some((1, d(0.5))));
        assert_eq!(locate(d(5.1), &durations, &weights), Some((1, d(3.1))));
        assert_eq!(
            locate(Duration::ZERO, &durations, &weights),
            Some((0, Duration::ZERO))
        );
        assert_eq!(locate(d(1.0), &[], &[]), None);
    }
}
