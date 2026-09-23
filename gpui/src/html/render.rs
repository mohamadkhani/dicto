//! Convert a parsed HTML block list into a GPUI element tree.
//!
//! Inline runs lay out via wrapping flex rows; one row per paragraph,
//! one wrapping segment per styled span. Sound links become clickable
//! pill buttons that play audio via the MDD resource lookup; images
//! are cached to a tmp directory and rendered via `gpui::img`.

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use gpui::{
    FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, img, px, svg,
};
use gpui_component::{h_flex, v_flex};
use tracing::{debug, warn};

use crate::audio;
use crate::colors;
use crate::html::parser::{Block, BlockLayout, Inline, Link, Style};

/// Render a pre-parsed block list. Callers parse once on lookup and
/// cache the result so we don't re-parse on every frame. Iteration is
/// by reference so we avoid cloning Blocks (and their inner Vecs) on
/// every render pass; SharedString text fields make leaf clones cheap.
pub fn render_blocks(blocks: &[Block]) -> gpui::AnyElement {
    v_flex()
        .w_full()
        .gap(px(8.))
        .children(
            blocks
                .iter()
                .enumerate()
                .map(|(idx, b)| render_block(idx, b)),
        )
        .into_any_element()
}

fn render_block(idx: usize, block: &Block) -> gpui::AnyElement {
    match block {
        Block::Paragraph { runs, layout } => with_layout(paragraph(idx, runs, 14.0, None), layout),
        Block::Heading {
            level,
            runs,
            layout,
        } => with_layout(heading(idx, *level, runs), layout),
        Block::ListItem {
            ordered,
            depth,
            content,
        } => list_item(idx, *ordered, *depth, content),
        Block::Divider => divider(),
        Block::Image(src) => image_block(idx, src.as_ref()),
    }
}

fn heading(idx: usize, level: u8, inlines: &[Inline]) -> gpui::AnyElement {
    let size = match level {
        1 => 22.0,
        2 => 19.0,
        3 => 17.0,
        _ => 15.5,
    };
    paragraph(idx, inlines, size, Some(FontWeight::BOLD))
}

/// Wrap a block element with CSS margins, padding, border-radius and
/// background. Margins are clamped so a single misbehaving rule can't
/// push content off-screen.
fn with_layout(inner: gpui::AnyElement, layout: &BlockLayout) -> gpui::AnyElement {
    let mt = layout.margin_top_px.clamp(0.0, 80.0);
    let mb = layout.margin_bottom_px.clamp(0.0, 80.0);
    let ml = layout.margin_left_px.clamp(0.0, 80.0);
    let pt = layout.padding_top_px.clamp(0.0, 80.0);
    let pb = layout.padding_bottom_px.clamp(0.0, 80.0);
    let pl = layout.padding_left_px.clamp(0.0, 80.0);
    let pr = layout.padding_right_px.clamp(0.0, 80.0);
    let br = layout.border_radius_px.clamp(0.0, 40.0);
    let bg = layout.bg_color.as_ref().and_then(|c| parse_bg_color(c));
    if mt == 0.0
        && mb == 0.0
        && ml == 0.0
        && pt == 0.0
        && pb == 0.0
        && pl == 0.0
        && pr == 0.0
        && br == 0.0
        && bg.is_none()
    {
        return inner;
    }
    let mut el = div().w_full().pt(px(mt)).pb(px(mb)).pl(px(ml));
    if pt > 0.0 {
        el = el.pt(px(pt));
    }
    if pb > 0.0 {
        el = el.pb(px(pb));
    }
    if pl > 0.0 {
        el = el.pl(px(pl));
    }
    if pr > 0.0 {
        el = el.pr(px(pr));
    }
    if br > 0.0 {
        el = el.rounded(px(br));
    }
    if let Some(bg) = bg {
        el = el.bg(bg);
    }
    el.child(inner).into_any_element()
}

fn list_item(idx: usize, _ordered: bool, depth: u8, content: &[Inline]) -> gpui::AnyElement {
    let indent = px(16.0 * depth as f32);
    h_flex()
        .w_full()
        .pl(indent)
        .gap(px(8.))
        .items_start()
        .child(
            div()
                .text_size(px(14.))
                .text_color(colors::text_secondary())
                .child("•"),
        )
        .child(paragraph(idx, content, 14.0, None))
        .into_any_element()
}

fn divider() -> gpui::AnyElement {
    div()
        .h(px(1.))
        .w_full()
        .bg(colors::border())
        .into_any_element()
}

/// Render one paragraph as a wrapping row of styled spans.
fn paragraph(
    idx: usize,
    inlines: &[Inline],
    base_size: f32,
    weight: Option<FontWeight>,
) -> gpui::AnyElement {
    // Single-run paragraphs (the common case for long body text) skip
    // the wrapping h_flex entirely; wrap in a w_full div so the text
    // knows its container width and can wrap properly!
    if inlines.len() == 1 {
        let child = if is_grouped_link(&inlines[0]) {
            render_link_group(idx, 0, &inlines[0..1], base_size, weight)
        } else {
            render_run(idx, 0, &inlines[0], base_size, weight)
        };
        return div().w_full().child(child).into_any_element();
    }

    let mut row = h_flex().w_full().flex_wrap().gap_x(px(0.)).gap_y(px(2.));

    let mut i = 0;
    while i < inlines.len() {
        let end = grouped_link_end(inlines, i);
        if end > i + 1 || is_grouped_link(&inlines[i]) {
            row = row.child(render_link_group(
                idx,
                i,
                &inlines[i..end],
                base_size,
                weight,
            ));
            i = end;
        } else {
            row = row.child(render_run(idx, i, &inlines[i], base_size, weight));
            i += 1;
        }
    }

    row.into_any_element()
}

fn grouped_link_end(inlines: &[Inline], start: usize) -> usize {
    if !is_grouped_link(&inlines[start]) {
        return start + 1;
    }
    let mut end = start + 1;
    while end < inlines.len()
        && same_grouped_link(inlines[start].link.as_ref(), inlines[end].link.as_ref())
    {
        end += 1;
    }
    end
}

fn is_grouped_link(run: &Inline) -> bool {
    matches!(run.link, Some(Link::Entry(_)) | Some(Link::External(_)))
        && (run.style.bg_color.is_some()
            || run.style.padding_top_px > 0.0
            || run.style.padding_right_px > 0.0
            || run.style.padding_bottom_px > 0.0
            || run.style.padding_left_px > 0.0
            || run.style.margin_right_px > 0.0
            || run.style.border_radius_px > 0.0)
}

fn same_grouped_link(a: Option<&Link>, b: Option<&Link>) -> bool {
    match (a, b) {
        (Some(Link::Entry(a)), Some(Link::Entry(b))) => a == b,
        (Some(Link::External(a)), Some(Link::External(b))) => a == b,
        _ => false,
    }
}

fn render_link_group(
    block_idx: usize,
    run_idx: usize,
    runs: &[Inline],
    base_size: f32,
    weight: Option<FontWeight>,
) -> gpui::AnyElement {
    let first = &runs[0];
    let link = first.link.as_ref();
    let target_word = match link {
        Some(Link::Entry(word)) => Some(word.clone()),
        _ => None,
    };

    let mut group = h_flex()
        .id(SharedString::from(format!("grp-{block_idx}-{run_idx}")))
        .items_center()
        .gap(px(0.))
        .flex_shrink(0.)
        .min_w(px(0.));

    if let Some(bg) = first
        .style
        .bg_color
        .as_ref()
        .and_then(|c| parse_bg_color(c))
    {
        group = group.bg(bg);
    }
    if first.style.padding_top_px > 0.0 {
        group = group.pt(px(first.style.padding_top_px));
    }
    if first.style.padding_right_px > 0.0 {
        group = group.pr(px(first.style.padding_right_px));
    }
    if first.style.padding_bottom_px > 0.0 {
        group = group.pb(px(first.style.padding_bottom_px));
    }
    if first.style.padding_left_px > 0.0 {
        group = group.pl(px(first.style.padding_left_px));
    }
    if first.style.margin_right_px > 0.0 {
        group = group.mr(px(first.style.margin_right_px));
    }
    if first.style.border_radius_px > 0.0 {
        group = group.rounded(px(first.style.border_radius_px));
    }

    let group = if let Some(word) = target_word {
        let target = word.clone();
        group.cursor_pointer().on_click(move |_, _, _cx| {
            debug!("entry link clicked: {target}");
        })
    } else {
        group
    };

    let group = runs.iter().enumerate().fold(group, |group, (offset, run)| {
        let mut style = run.style.clone();
        style.bg_color = None;
        style.padding_top_px = 0.0;
        style.padding_right_px = 0.0;
        style.padding_bottom_px = 0.0;
        style.padding_left_px = 0.0;
        style.margin_right_px = 0.0;
        style.border_radius_px = 0.0;
        group.child(styled_span(
            block_idx,
            run_idx + offset,
            run.text.clone(),
            &style,
            base_size,
            weight,
            None,
        ))
    });

    group.into_any_element()
}

fn render_run(
    block_idx: usize,
    run_idx: usize,
    run: &Inline,
    base_size: f32,
    weight: Option<FontWeight>,
) -> gpui::AnyElement {
    if let Some(Link::Sound(path)) = &run.link {
        return sound_button(
            block_idx,
            run_idx,
            run.text.as_ref(),
            &run.style,
            run.image.as_ref(),
            path.clone(),
        );
    }
    styled_span(
        block_idx,
        run_idx,
        run.text.clone(),
        &run.style,
        base_size,
        weight,
        run.link.as_ref(),
    )
}

fn styled_span(
    block_idx: usize,
    run_idx: usize,
    text: SharedString,
    style: &Style,
    base_size: f32,
    weight: Option<FontWeight>,
    link: Option<&Link>,
) -> gpui::AnyElement {
    let color = if let Some(c) = style.color.as_ref().and_then(|c| parse_text_color(c)) {
        c
    } else if matches!(link, Some(Link::Entry(_)) | Some(Link::External(_))) {
        colors::primary()
    } else {
        colors::text()
    };

    let bg = style.bg_color.as_ref().and_then(|c| parse_bg_color(c));

    let font_weight = if style.bold || weight == Some(FontWeight::BOLD) {
        FontWeight::BOLD
    } else {
        weight.unwrap_or(FontWeight::NORMAL)
    };

    let mut size_px = style.font_size_px.unwrap_or(base_size);
    if style.superscript || style.subscript {
        size_px *= 0.7;
    }

    if let Some(Link::Entry(word)) = link {
        let target = word.clone();
        let mut el = div()
            .id(SharedString::from(format!("r-{block_idx}-{run_idx}")))
            .text_size(px(size_px))
            .font_weight(font_weight)
            .text_color(color)
            .cursor_pointer()
            .flex_shrink(0.)
            .min_w(px(0.))
            .on_click(move |_, _, _cx| {
                debug!("entry link clicked: {target}");
            });
        if let Some(bg) = bg {
            el = el.bg(bg);
        }
        if style.italic {
            el = el.italic();
        }
        if style.underline {
            el = el.underline();
        }
        return el.child(text).into_any_element();
    }

    let mut el = div()
        .text_size(px(size_px))
        .font_weight(font_weight)
        .text_color(color)
        .flex_shrink(0.)
        .min_w(px(0.));
    if let Some(bg) = bg {
        el = el.bg(bg);
        // Non-link spans with a background are "chips" (e.g. WordNet
        // <span class="pos">): apply CSS box-model properties so they
        // get padding, rounding and spacing between adjacent chips.
        if style.border_radius_px > 0.0 {
            el = el.rounded(px(style.border_radius_px));
        }
        if style.padding_left_px > 0.0 {
            el = el.pl(px(style.padding_left_px));
        }
        if style.padding_right_px > 0.0 {
            el = el.pr(px(style.padding_right_px));
        }
        if style.padding_top_px > 0.0 {
            el = el.pt(px(style.padding_top_px));
        }
        if style.padding_bottom_px > 0.0 {
            el = el.pb(px(style.padding_bottom_px));
        }
        if style.margin_left_px > 0.0 {
            el = el.ml(px(style.margin_left_px));
        }
        if style.margin_right_px > 0.0 {
            el = el.mr(px(style.margin_right_px));
        }
    }
    if style.italic {
        el = el.italic();
    }
    if style.underline {
        el = el.underline();
    }
    el.child(text).into_any_element()
}

fn sound_button(
    block_idx: usize,
    run_idx: usize,
    label: &str,
    style: &Style,
    image_src: Option<&SharedString>,
    path: SharedString,
) -> gpui::AnyElement {
    let icon_only = image_src.is_none() && label.trim().is_empty();
    let mut btn = div()
        .id(SharedString::from(format!("snd-{block_idx}-{run_idx}")))
        .cursor_pointer()
        .flex_shrink(0.)
        .min_w(px(0.));

    if style.margin_left_px > 0.0 {
        btn = btn.ml(px(style.margin_left_px));
    }
    if style.margin_right_px > 0.0 {
        btn = btn.mr(px(style.margin_right_px));
    }

    // If we have a speaker icon image (e.g. img/spkr_r.png), render it.
    // Otherwise fall back to the ▶ text pill.
    if let Some(src) = image_src {
        btn = btn
            .px(px(4.))
            .py(px(1.))
            .rounded(px(4.))
            .hover(|s| s.bg(colors::border()));
        if let Some(bytes) = mdict_rs::query::lookup_resource(src.as_ref()) {
            if let Some(cached) = cache_image(src.as_ref(), &bytes) {
                btn = btn.child(img(cached).h(px(16.)).w(px(16.)));
            } else {
                btn = btn.child(SharedString::from("▶"));
            }
        } else {
            btn = btn.child(SharedString::from("▶"));
        }
    } else if icon_only {
        let color = style
            .color
            .as_ref()
            .and_then(|c| parse_text_color(c))
            .unwrap_or(colors::primary());
        let size_px = style.font_size_px.unwrap_or(16.0);
        btn = btn.px(px(2.)).hover(|s| s.bg(colors::border())).child(
            svg()
                .path("icons/play.svg")
                .text_color(color)
                .size(px(size_px)),
        );
    } else {
        let display: SharedString = if label.trim().is_empty() {
            SharedString::from("▶")
        } else {
            SharedString::from(format!("▶ {}", label.trim()))
        };
        btn = btn
            .px(px(4.))
            .py(px(1.))
            .mx(px(1.))
            .rounded(px(4.))
            .hover(|s| s.bg(colors::border()))
            .bg(colors::surface())
            .border_1()
            .border_color(colors::border())
            .text_size(px(13.))
            .text_color(colors::primary())
            .child(display);
    }

    btn.on_click(move |_, _, _cx| {
        dicto_telemetry::get().track(dicto_telemetry::Event::PronunciationPlayed);
        audio::play_resource(path.as_ref());
    })
    .into_any_element()
}

fn image_block(idx: usize, src: &str) -> gpui::AnyElement {
    let bytes = match mdict_rs::query::lookup_resource(src) {
        Some(b) => b,
        None => {
            return div()
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(format!("[image: {src}]")))
                .into_any_element();
        }
    };

    let Some(path) = cache_image(src, &bytes) else {
        return div()
            .text_size(px(12.))
            .text_color(colors::text_secondary())
            .child(SharedString::from(format!("[image: {src}]")))
            .into_any_element();
    };

    // `img(SharedString)` routes through the embedded asset loader and
    // fails on absolute filesystem paths; passing a PathBuf hits the
    // `Resource::Path` branch instead and reads the file directly.
    img(path)
        .id(SharedString::from(format!("img-{idx}")))
        .max_w(px(360.))
        .into_any_element()
}

/// Write image bytes to a per-session tmp cache so gpui can load them
/// by path. The filename keys off a hash of the source path so repeat
/// references reuse the same file.
fn cache_image(src: &str, bytes: &[u8]) -> Option<PathBuf> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut hasher);
    let hash = hasher.finish();

    let ext = std::path::Path::new(src)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("img");

    let mut dir = std::env::temp_dir();
    dir.push("mdict-rs-cache");
    if let Err(e) = fs::create_dir_all(&dir) {
        warn!("could not create image cache dir: {e}");
        return None;
    }

    let path = dir.join(format!("{hash:016x}.{ext}"));
    if !path.exists()
        && let Err(e) = fs::write(&path, bytes)
    {
        warn!("could not write image cache: {e}");
        return None;
    }
    Some(path)
}

/// Parse an HTML color value for foreground text on the app's dark
/// theme. Hue is preserved while dark colors are lifted so they stay
/// readable against the surface.
fn parse_text_color(s: &str) -> Option<gpui::Hsla> {
    let (r, g, b) = parse_rgb(s)?;
    Some(remap_text_color_for_dark_theme(r, g, b))
}

/// Parse an HTML color value for backgrounds on the app's dark theme.
/// Unlike text colors, backgrounds should stay subdued so chips and
/// panels do not turn into bright glowing blocks.
fn parse_bg_color(s: &str) -> Option<gpui::Hsla> {
    let (r, g, b) = parse_rgb(s)?;
    Some(remap_bg_color_for_dark_theme(r, g, b))
}

fn parse_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim();
    if let Some(rgb) = named_color(&s.to_lowercase()) {
        return Some(rgb);
    }
    let hex = s.strip_prefix('#').unwrap_or(s);
    match hex.len() {
        6 => Some((
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
        )),
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
            Some((r * 17, g * 17, b * 17))
        }
        _ => None,
    }
}

fn named_color(s: &str) -> Option<(u8, u8, u8)> {
    Some(match s {
        "black" => (0, 0, 0),
        "white" => (255, 255, 255),
        "red" => (255, 0, 0),
        "green" => (0, 128, 0),
        "blue" => (0, 0, 255),
        "yellow" => (255, 255, 0),
        "cyan" | "aqua" => (0, 255, 255),
        "magenta" | "fuchsia" => (255, 0, 255),
        "gray" | "grey" => (128, 128, 128),
        "silver" => (192, 192, 192),
        "maroon" => (128, 0, 0),
        "navy" => (0, 0, 128),
        "olive" => (128, 128, 0),
        "purple" => (128, 0, 128),
        "teal" => (0, 128, 128),
        "lime" => (0, 255, 0),
        "orange" => (255, 165, 0),
        "pink" => (255, 192, 203),
        "brown" => (165, 42, 42),
        _ => return None,
    })
}

/// Remap a CSS text color for a dark background. We lift the lightness
/// so it reads against the surface and cap saturation so primary colors
/// don't burn on dark themes.
fn remap_text_color_for_dark_theme(r: u8, g: u8, b: u8) -> gpui::Hsla {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let (h, mut s, mut l) = rgb_to_hsl(r, g, b);
    const MIN_L: f32 = 0.65;
    const MAX_S: f32 = 0.75;
    if l < MIN_L {
        l = MIN_L;
    }
    if s > MAX_S {
        s = MAX_S;
    }
    let (r, g, b) = hsl_to_rgb(h, s, l);
    let r = (r * 255.0).round().clamp(0.0, 255.0) as u32;
    let g = (g * 255.0).round().clamp(0.0, 255.0) as u32;
    let b = (b * 255.0).round().clamp(0.0, 255.0) as u32;
    gpui::rgb((r << 16) | (g << 8) | b).into()
}

/// Remap a CSS background color for the app's dark theme. Neutral
/// backgrounds become dark panels; colored backgrounds keep their hue
/// but stay darker than text-oriented remapping so white text still
/// contrasts well.
fn remap_bg_color_for_dark_theme(r: u8, g: u8, b: u8) -> gpui::Hsla {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let (h, mut s, mut l) = rgb_to_hsl(r, g, b);

    if s < 0.08 {
        s = 0.0;
        l = l.clamp(0.14, 0.22);
    } else {
        const MIN_L: f32 = 0.24;
        const MAX_L: f32 = 0.40;
        const MAX_S: f32 = 0.60;
        l = l.clamp(MIN_L, MAX_L);
        if s > MAX_S {
            s = MAX_S;
        }
    }

    let (r, g, b) = hsl_to_rgb(h, s, l);
    let r = (r * 255.0).round().clamp(0.0, 255.0) as u32;
    let g = (g * 255.0).round().clamp(0.0, 255.0) as u32;
    let b = (b * 255.0).round().clamp(0.0, 255.0) as u32;
    gpui::rgb((r << 16) | (g << 8) | b).into()
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < 1e-6 {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < 1e-6 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s.abs() < 1e-6 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let h = h / 360.0;
    (
        hue_to_rgb(p, q, h + 1.0 / 3.0),
        hue_to_rgb(p, q, h),
        hue_to_rgb(p, q, h - 1.0 / 3.0),
    )
}

fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 0.5 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}
