//! The tray icon pixels, shared by both platform backends.
//!
//! The icon is the inner dictionary card (front card + bold "A" +
//! definition lines) pre-rendered from `assets/tray-icon.svg` by librsvg at
//! 128×128 — see `assets/gen-tray-icon.sh`. Embedding a real SVG render
//! instead of rasterizing geometry by hand gives pixel-exact gradients and
//! smooth 8-bit anti-aliased edges at any tray scale.

/// Edge length of the pre-rendered icon, in pixels.
pub(super) const SIZE: u32 = 128;

/// The embedded 128×128 RGBA bytes (`assets/tray-icon-128.raw`).
fn raw_rgba() -> &'static [u8] {
    include_bytes!("../../../assets/tray-icon-128.raw")
}

/// Icon bytes in plain RGBA — what `tray-icon`'s `Icon::from_rgba` wants.
#[cfg(target_os = "windows")]
pub fn rgba() -> Vec<u8> {
    raw_rgba().to_vec()
}

/// Icon bytes converted to ARGB32 (network byte order) — what the SNI
/// `IconPixmap` field wants (the same `rotate_right(1)` the ksni docs show).
#[cfg(target_os = "linux")]
pub fn argb() -> Vec<u8> {
    to_argb32(raw_rgba())
}

/// Rotate every RGBA pixel into ARGB32 byte order.
#[cfg(any(target_os = "linux", test))]
fn to_argb32(rgba: &[u8]) -> Vec<u8> {
    let mut data = rgba.to_vec();
    for pixel in data.chunks_exact_mut(4) {
        pixel.rotate_right(1); // RGBA → ARGB32
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_covers_the_canvas() {
        let data = to_argb32(raw_rgba());
        assert_eq!(raw_rgba().len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(data.len(), (SIZE * SIZE * 4) as usize);

        // Byte order: ARGB32. The canvas corners are transparent padding —
        // all four bytes must be zero there.
        let size = SIZE as i32;
        for (x, y) in [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)] {
            let i = ((y * size + x) * 4) as usize;
            assert_eq!(
                &data[i..i + 4],
                &[0, 0, 0, 0],
                "corner ({x},{y}) must be transparent"
            );
        }

        // The card is scaled to fill the canvas (height-limited), so the
        // opaque area covers most of it.
        let opaque = data.chunks_exact(4).filter(|p| p[0] > 0).count();
        let frac = opaque as f32 / (SIZE * SIZE) as f32;
        assert!(
            frac > 0.5,
            "tray icon only covers {:.1}% of the canvas — it looks small",
            frac * 100.0
        );

        // The "A" mark renders as near-black (#1a1b26) on the card.
        let dark = data
            .chunks_exact(4)
            .filter(|p| p[0] == 255 && p[1] < 0x40 && p[2] < 0x40 && p[3] < 0x40)
            .count();
        assert!(dark > 10, "no dark 'A' mark rendered");
    }
}
