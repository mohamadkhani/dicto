//! Very small HTML parser tuned to MDX dictionary content.
//!
//! The output is a flat list of [`Block`] nodes; each text block holds
//! a list of styled [`Inline`] spans. This is deliberately not a
//! conformant HTML parser — it handles the tag subset that real-world
//! MDX dictionaries use (formatting, links, images, lists, headings)
//! and falls back to plain text for anything unrecognized.

use std::collections::HashMap;

use gpui::SharedString;

use crate::html::css::{ElementCtx, Stylesheet};

// Owned text fields use SharedString (Arc-backed): cloning is constant
// time, so the renderer can freely re-emit blocks on every frame
// without re-allocating the underlying strings.

#[derive(Debug, Clone, Default)]
pub struct BlockLayout {
    pub margin_top_px: f32,
    pub margin_bottom_px: f32,
    pub margin_left_px: f32,
    pub bg_color: Option<SharedString>,
    pub padding_top_px: f32,
    pub padding_bottom_px: f32,
    pub padding_left_px: f32,
    pub padding_right_px: f32,
    pub border_radius_px: f32,
}

#[derive(Debug, Clone)]
pub enum Block {
    Paragraph {
        runs: Vec<Inline>,
        layout: BlockLayout,
    },
    Heading {
        level: u8,
        runs: Vec<Inline>,
        layout: BlockLayout,
    },
    ListItem {
        ordered: bool,
        depth: u8,
        content: Vec<Inline>,
    },
    Divider,
    Image(SharedString),
}

#[derive(Debug, Clone, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub color: Option<SharedString>,
    /// Explicit background tint set on this element. This does not
    /// inherit; block elements consume it via `BlockLayout`, while
    /// inline elements render it directly on the span.
    pub bg_color: Option<SharedString>,
    /// Explicit font size override in px (CSS `font-size: 14px` / `1em`).
    pub font_size_px: Option<f32>,
    /// Inline box-model fields used by dictionaries that style links as
    /// chips/buttons (for example headword jump links).
    pub padding_top_px: f32,
    pub padding_right_px: f32,
    pub padding_bottom_px: f32,
    pub padding_left_px: f32,
    pub margin_right_px: f32,
    pub border_radius_px: f32,
    /// CSS margins, captured per-element. Block-level elements emit a
    /// Paragraph carrying these as `BlockLayout` at flush time.
    pub margin_top_px: f32,
    pub margin_bottom_px: f32,
    pub margin_left_px: f32,
}

#[derive(Debug, Clone)]
pub enum Link {
    Sound(SharedString),
    Entry(SharedString),
    #[allow(dead_code)]
    External(SharedString),
}

#[derive(Debug, Clone)]
pub struct Inline {
    pub text: SharedString,
    pub style: Style,
    pub link: Option<Link>,
    /// When this inline was produced from an <img> inside a sound link,
    /// this holds the image `src` so the renderer can show a speaker icon
    /// instead of a plain ▶ pill.
    pub image: Option<SharedString>,
}

#[derive(Debug)]
enum Event {
    Open(String, HashMap<String, String>),
    Close(String),
    SelfClose(String, HashMap<String, String>),
    Text(String),
}

pub fn parse_with_styles(html: &str, styles: &Stylesheet) -> Vec<Block> {
    let events = tokenize(html);
    build_blocks(events, styles)
}

fn tokenize(html: &str) -> Vec<Event> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut i = 0;

    while i < html.len() {
        let rest = &html[i..];
        let c = match rest.chars().next() {
            Some(c) => c,
            None => break,
        };
        let c_len = c.len_utf8();

        if c == '<' {
            if !buf.is_empty() {
                out.push(Event::Text(std::mem::take(&mut buf)));
            }
            if let Some(body) = rest.strip_prefix("<!--") {
                if let Some(end) = body.find("-->") {
                    i += 4 + end + 3;
                    continue;
                } else {
                    break;
                }
            }
            if rest.starts_with("<!") {
                if let Some(end) = rest.find('>') {
                    i += end + 1;
                    continue;
                } else {
                    break;
                }
            }

            // locate matching '>', honoring attribute quotes
            let mut in_quote: Option<char> = None;
            let mut close_rel: Option<usize> = None;
            for (off, ch) in rest[c_len..].char_indices() {
                match in_quote {
                    Some(q) if ch == q => in_quote = None,
                    None if ch == '"' || ch == '\'' => in_quote = Some(ch),
                    None if ch == '>' => {
                        close_rel = Some(c_len + off);
                        break;
                    }
                    _ => {}
                }
            }
            let Some(close) = close_rel else { break };
            let raw = &rest[c_len..close];
            i += close + 1;
            if let Some(ev) = parse_tag(raw) {
                out.push(ev);
            }
        } else {
            buf.push(c);
            i += c_len;
        }
    }
    if !buf.is_empty() {
        out.push(Event::Text(buf));
    }
    out
}

fn parse_tag(raw: &str) -> Option<Event> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(rest) = raw.strip_prefix('/') {
        let name = rest.split_whitespace().next().unwrap_or("").to_lowercase();
        if name.is_empty() {
            return None;
        }
        return Some(Event::Close(name));
    }
    let self_close = raw.ends_with('/');
    let body = if self_close {
        &raw[..raw.len() - 1]
    } else {
        raw
    };
    let mut parts = body.splitn(2, |c: char| c.is_whitespace());
    let name = parts.next().unwrap_or("").to_lowercase();
    let attrs_str = parts.next().unwrap_or("").trim();
    let attrs = parse_attrs(attrs_str);
    // void elements
    let void = matches!(
        name.as_str(),
        "br" | "img" | "hr" | "meta" | "link" | "input"
    );
    if self_close || void {
        Some(Event::SelfClose(name, attrs))
    } else {
        Some(Event::Open(name, attrs))
    }
}

fn parse_attrs(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let name_start = i;
        while i < bytes.len() {
            let c = bytes[i] as char;
            if c == '=' || c.is_whitespace() || c == '>' {
                break;
            }
            i += 1;
        }
        let name = s[name_start..i].to_lowercase();
        if name.is_empty() {
            break;
        }
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < bytes.len() && bytes[i] as char == '=' {
            i += 1;
            while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] as char == '"' || bytes[i] as char == '\'') {
                let q = bytes[i] as char;
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] as char != q {
                    i += 1;
                }
                value = s[start..i].to_string();
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                let start = i;
                while i < bytes.len() {
                    let c = bytes[i] as char;
                    if c.is_whitespace() || c == '>' {
                        break;
                    }
                    i += 1;
                }
                value = s[start..i].to_string();
            }
        }
        out.insert(name, value);
    }
    out
}

fn decode_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let rest = &input[i..];
        let c = match rest.chars().next() {
            Some(c) => c,
            None => break,
        };
        if c == '&'
            && let Some(end_rel) = rest.find(';')
        {
            let entity = &rest[1..end_rel];
            let replacement = match entity {
                "amp" => Some("&".to_string()),
                "lt" => Some("<".to_string()),
                "gt" => Some(">".to_string()),
                "quot" => Some("\"".to_string()),
                "apos" => Some("'".to_string()),
                "nbsp" => Some(" ".to_string()),
                _ => {
                    if let Some(num) = entity.strip_prefix('#') {
                        let (radix, digits) = if let Some(hex) = num.strip_prefix(['x', 'X']) {
                            (16, hex)
                        } else {
                            (10, num)
                        };
                        u32::from_str_radix(digits, radix)
                            .ok()
                            .and_then(char::from_u32)
                            .map(|c| c.to_string())
                    } else {
                        None
                    }
                }
            };
            if let Some(r) = replacement {
                out.push_str(&r);
                i += end_rel + 1;
                continue;
            }
        }
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn classify_link(href: &str) -> Link {
    let lower = href.to_lowercase();
    if lower.starts_with("sound://") {
        return Link::Sound(SharedString::from(href[8..].to_string()));
    }
    if lower.starts_with("sound:") {
        return Link::Sound(SharedString::from(href[6..].to_string()));
    }
    if lower.starts_with("entry://") {
        return Link::Entry(SharedString::from(href[8..].to_string()));
    }
    Link::External(SharedString::from(href.to_string()))
}

struct OpenElement {
    tag: String,
    classes: Vec<String>,
    id: Option<String>,
    /// Was the styled frame pushed for this element? Mirrors style_stack.
    style_pushed: bool,
    /// CSS-derived display: emit a paragraph flush around the element.
    block_display: bool,
    /// `display: none` — the element and descendants are dropped.
    hidden: bool,
}

struct Builder {
    blocks: Vec<Block>,
    inline_buf: Vec<Inline>,
    style_stack: Vec<Style>,
    link_stack: Vec<Link>,
    list_stack: Vec<bool>, // bool = ordered?
    element_stack: Vec<OpenElement>,
    /// > 0 while we're inside a `display: none` subtree.
    skip_depth: u32,
}

impl Builder {
    fn new() -> Self {
        Self {
            blocks: Vec::new(),
            inline_buf: Vec::new(),
            style_stack: Vec::new(),
            link_stack: Vec::new(),
            list_stack: Vec::new(),
            element_stack: Vec::new(),
            skip_depth: 0,
        }
    }

    fn current_style(&self) -> Style {
        self.style_stack.last().cloned().unwrap_or_default()
    }

    fn current_link(&self) -> Option<Link> {
        self.link_stack.last().cloned()
    }

    fn push_text(&mut self, mut text: String) {
        if text.is_empty() {
            return;
        }
        // Browsers collapse whitespace across adjacent inline elements.
        // Our parser emits one Inline per text node, so if the previous
        // run ended with a space (or we're at the start of a fresh
        // paragraph) we strip leading whitespace from this run.
        let trim_leading = match self.inline_buf.last() {
            None => true,
            Some(prev) => prev.text.ends_with(|c: char| c.is_whitespace()),
        };
        if trim_leading {
            let trimmed = text.trim_start();
            if trimmed.len() != text.len() {
                text = trimmed.to_string();
            }
            if text.is_empty() {
                return;
            }
        }
        self.inline_buf.push(Inline {
            text: SharedString::from(text),
            style: self.current_style(),
            link: self.current_link(),
            image: None,
        });
    }

    fn flush_paragraph(&mut self) {
        if self.inline_buf.is_empty() {
            return;
        }
        let runs = std::mem::take(&mut self.inline_buf);
        let layout = self.current_layout();
        if self.list_stack.last().is_some() {
            let ordered = *self.list_stack.last().unwrap();
            self.blocks.push(Block::ListItem {
                ordered,
                depth: self.list_stack.len() as u8,
                content: runs,
            });
        } else {
            self.blocks.push(Block::Paragraph { runs, layout });
        }
    }

    fn flush_heading(&mut self, level: u8) {
        if self.inline_buf.is_empty() {
            return;
        }
        let runs = std::mem::take(&mut self.inline_buf);
        let layout = self.current_layout();
        self.blocks.push(Block::Heading {
            level,
            runs,
            layout,
        });
    }

    /// Read margin properties off the current style frame as a
    /// [`BlockLayout`]. Returns the default (all zeros) when there's
    /// no active style frame.
    fn current_layout(&self) -> BlockLayout {
        match self.style_stack.last() {
            Some(s) => BlockLayout {
                margin_top_px: s.margin_top_px,
                margin_bottom_px: s.margin_bottom_px,
                margin_left_px: s.margin_left_px,
                bg_color: s.bg_color.clone(),
                padding_top_px: s.padding_top_px,
                padding_bottom_px: s.padding_bottom_px,
                padding_left_px: s.padding_left_px,
                padding_right_px: s.padding_right_px,
                border_radius_px: s.border_radius_px,
            },
            None => BlockLayout::default(),
        }
    }

    fn close_style(&mut self) {
        self.style_stack.pop();
    }
}

fn extract_classes(attrs: &HashMap<String, String>) -> Vec<String> {
    attrs
        .get("class")
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

fn extract_id(attrs: &HashMap<String, String>) -> Option<String> {
    attrs.get("id").cloned()
}

/// Apply a CSS declaration map onto an existing Style.
fn apply_decls(decls: &HashMap<String, String>, style: &mut Style) {
    for (k, v) in decls {
        match k.as_str() {
            "color" => style.color = Some(SharedString::from(v.clone())),
            "background-color" | "background" => {
                // Use only the first token of `background` shorthand —
                // we just want the color, not images/positions.
                let val = v.split_whitespace().next().unwrap_or("");
                if !val.is_empty() {
                    style.bg_color = Some(SharedString::from(val.to_string()));
                }
            }
            "font-weight" => {
                let lv = v.to_ascii_lowercase();
                if lv == "bold" || lv == "bolder" {
                    style.bold = true;
                } else if let Ok(n) = lv.parse::<u32>()
                    && n >= 600
                {
                    style.bold = true;
                }
            }
            "font-style" => {
                if v.eq_ignore_ascii_case("italic") || v.eq_ignore_ascii_case("oblique") {
                    style.italic = true;
                }
            }
            "text-decoration" | "text-decoration-line" => {
                if v.to_ascii_lowercase().contains("underline") {
                    style.underline = true;
                }
            }
            "font-size" => {
                if let Some(px) = parse_css_length(v, 14.0) {
                    style.font_size_px = Some(px);
                }
            }
            "margin-top" => {
                if let Some(px) = parse_css_length(v, 14.0) {
                    style.margin_top_px = px;
                }
            }
            "margin-bottom" => {
                if let Some(px) = parse_css_length(v, 14.0) {
                    style.margin_bottom_px = px;
                }
            }
            "margin-left" => {
                if let Some(px) = parse_css_length(v, 14.0) {
                    style.margin_left_px = px;
                }
            }
            "margin-right" => {
                if let Some(px) = parse_css_length(v, 14.0) {
                    style.margin_right_px = px;
                }
            }
            "margin" => {
                // shorthand: 1-4 values. Treat as uniform when one value.
                let parts: Vec<&str> = v.split_whitespace().collect();
                if let [single] = parts.as_slice() {
                    if let Some(px) = parse_css_length(single, 14.0) {
                        style.margin_top_px = px;
                        style.margin_bottom_px = px;
                        style.margin_left_px = px;
                    }
                } else if parts.len() >= 2 {
                    if let Some(px) = parse_css_length(parts[0], 14.0) {
                        style.margin_top_px = px;
                    }
                    if let Some(px) = parse_css_length(parts.last().unwrap(), 14.0) {
                        style.margin_bottom_px = px;
                    }
                    if parts.len() == 2 {
                        if let Some(px) = parse_css_length(parts[1], 14.0) {
                            style.margin_right_px = px;
                            style.margin_left_px = px;
                        }
                    } else if parts.len() >= 4 {
                        if let Some(px) = parse_css_length(parts[1], 14.0) {
                            style.margin_right_px = px;
                        }
                        if let Some(px) = parse_css_length(parts[3], 14.0) {
                            style.margin_left_px = px;
                        }
                    }
                }
            }
            "padding-top" => {
                if let Some(px) = parse_css_length(v, 14.0) {
                    style.padding_top_px = px;
                }
            }
            "padding-right" => {
                if let Some(px) = parse_css_length(v, 14.0) {
                    style.padding_right_px = px;
                }
            }
            "padding-bottom" => {
                if let Some(px) = parse_css_length(v, 14.0) {
                    style.padding_bottom_px = px;
                }
            }
            "padding-left" => {
                if let Some(px) = parse_css_length(v, 14.0) {
                    style.padding_left_px = px;
                }
            }
            "padding" => {
                let parts: Vec<&str> = v.split_whitespace().collect();
                match parts.as_slice() {
                    [single] => {
                        if let Some(px) = parse_css_length(single, 14.0) {
                            style.padding_top_px = px;
                            style.padding_right_px = px;
                            style.padding_bottom_px = px;
                            style.padding_left_px = px;
                        }
                    }
                    [vertical, horizontal] => {
                        if let Some(px) = parse_css_length(vertical, 14.0) {
                            style.padding_top_px = px;
                            style.padding_bottom_px = px;
                        }
                        if let Some(px) = parse_css_length(horizontal, 14.0) {
                            style.padding_right_px = px;
                            style.padding_left_px = px;
                        }
                    }
                    [top, horizontal, bottom] => {
                        if let Some(px) = parse_css_length(top, 14.0) {
                            style.padding_top_px = px;
                        }
                        if let Some(px) = parse_css_length(horizontal, 14.0) {
                            style.padding_right_px = px;
                            style.padding_left_px = px;
                        }
                        if let Some(px) = parse_css_length(bottom, 14.0) {
                            style.padding_bottom_px = px;
                        }
                    }
                    [top, right, bottom, left] => {
                        if let Some(px) = parse_css_length(top, 14.0) {
                            style.padding_top_px = px;
                        }
                        if let Some(px) = parse_css_length(right, 14.0) {
                            style.padding_right_px = px;
                        }
                        if let Some(px) = parse_css_length(bottom, 14.0) {
                            style.padding_bottom_px = px;
                        }
                        if let Some(px) = parse_css_length(left, 14.0) {
                            style.padding_left_px = px;
                        }
                    }
                    _ => {}
                }
            }
            "border-radius" => {
                let val = v.split_whitespace().next().unwrap_or("");
                if let Some(px) = parse_css_length(val, 14.0) {
                    style.border_radius_px = px;
                }
            }
            _ => {}
        }
    }
}

/// Parse a CSS length to pixels. Accepts `px`, `em`, `rem`, `pt`, `%`.
fn parse_css_length(v: &str, base_px: f32) -> Option<f32> {
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    let (num_part, unit) = split_number_unit(v);
    let n: f32 = num_part.parse().ok()?;
    Some(match unit {
        "px" | "" => n,
        "em" | "rem" => n * base_px,
        "pt" => n * 1.333,
        "%" => n * 0.01 * base_px,
        _ => n,
    })
}

fn split_number_unit(s: &str) -> (&str, &str) {
    let idx = s
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+'))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    (&s[..idx], &s[idx..])
}

/// Whether `display` value (or absence) means the element should flush
/// a paragraph around itself.
fn display_is_block(v: Option<&String>, default_block: bool) -> bool {
    match v.map(|s| s.to_ascii_lowercase()) {
        Some(s) if s == "block" || s == "list-item" || s.starts_with("table") => true,
        Some(s) if s == "inline" || s == "inline-block" => false,
        Some(s) if s == "none" => false, // handled separately
        _ => default_block,
    }
}

/// Some dictionaries use block tags like `<div>` as structural wrappers
/// around inline content while relying on browser float/layout behavior
/// that we do not fully emulate. Treating these wrappers as inline keeps
/// sense-number rows together instead of breaking after every nested div.
fn is_inline_wrapper(name: &str, classes: &[String]) -> bool {
    if !name.eq_ignore_ascii_case("div") {
        return false;
    }
    classes
        .iter()
        .any(|c| matches!(c.as_str(), "scnt" | "sense" | "sblock_labels"))
}

/// Image emission with one special case: if the image is wrapped in a
/// `sound://` link (the standard MDX pronunciation-button idiom), we
/// fold it into an inline run carrying the sound link instead of
/// emitting a standalone image block. The renderer turns that run into
/// a clickable speaker pill.
fn emit_image(b: &mut Builder, attrs: &HashMap<String, String>) {
    let Some(src) = attrs.get("src") else { return };
    if matches!(b.current_link(), Some(Link::Sound(_))) {
        b.inline_buf.push(Inline {
            text: SharedString::default(),
            style: b.current_style(),
            link: b.current_link(),
            image: Some(SharedString::from(src.clone())),
        });
        return;
    }
    b.flush_paragraph();
    b.blocks.push(Block::Image(SharedString::from(src.clone())));
}

fn build_blocks(events: Vec<Event>, styles: &Stylesheet) -> Vec<Block> {
    let mut b = Builder::new();
    let mut pending_heading: Option<u8> = None;

    for ev in events {
        match ev {
            Event::Text(t) => {
                if b.skip_depth > 0 {
                    continue;
                }
                let decoded = decode_entities(&collapse_whitespace(&t));
                if !decoded.is_empty() {
                    b.push_text(decoded);
                }
            }
            Event::SelfClose(name, attrs) => {
                if b.skip_depth > 0 {
                    continue;
                }
                match name.as_str() {
                    "br" => {
                        if pending_heading.is_some() {
                            let level = pending_heading.take().unwrap();
                            b.flush_heading(level);
                        } else {
                            b.flush_paragraph();
                        }
                    }
                    "hr" => {
                        b.flush_paragraph();
                        b.blocks.push(Block::Divider);
                    }
                    "img" => emit_image(&mut b, &attrs),
                    _ => {}
                }
            }
            Event::Open(name, attrs) => {
                handle_open(&mut b, styles, &name, &attrs, &mut pending_heading);
            }
            Event::Close(name) => {
                handle_close(&mut b, &name, &mut pending_heading);
            }
        }
    }
    if let Some(level) = pending_heading.take() {
        b.flush_heading(level);
    } else {
        b.flush_paragraph();
    }
    b.blocks
}

fn handle_open(
    b: &mut Builder,
    styles: &Stylesheet,
    name: &str,
    attrs: &HashMap<String, String>,
    pending_heading: &mut Option<u8>,
) {
    let classes = extract_classes(attrs);
    let id = extract_id(attrs);

    // If we're already inside a hidden subtree, just track the depth so
    // close events stay balanced; emit nothing.
    if b.skip_depth > 0 {
        b.element_stack.push(OpenElement {
            tag: name.to_string(),
            classes,
            id,
            style_pushed: false,
            block_display: false,
            hidden: true,
        });
        b.skip_depth += 1;
        return;
    }

    // Match CSS rules against this element + ancestor chain.
    let ancestors: Vec<ElementCtx<'_>> = b
        .element_stack
        .iter()
        .map(|e| ElementCtx {
            tag: &e.tag,
            classes: &e.classes,
            id: e.id.as_deref(),
        })
        .collect();
    let ctx = ElementCtx {
        tag: name,
        classes: &classes,
        id: id.as_deref(),
    };
    let decls = styles.matching(&ctx, &ancestors);

    let hidden = decls
        .get("display")
        .map(|v| v.eq_ignore_ascii_case("none"))
        .unwrap_or(false);

    if hidden {
        b.element_stack.push(OpenElement {
            tag: name.to_string(),
            classes,
            id,
            style_pushed: false,
            block_display: false,
            hidden: true,
        });
        b.skip_depth = 1;
        return;
    }

    // Hard-coded block-level HTML tags (independent of CSS).
    let default_block = matches!(
        name,
        "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "li" | "table" | "tr"
    ) && !is_inline_wrapper(name, &classes);
    let block_display = display_is_block(decls.get("display"), default_block);

    // Pre-flush so block-level content starts on its own paragraph.
    if block_display {
        b.flush_paragraph();
    }

    // Build the merged style for this element. Inheritable text styles
    // are inherited from the parent; box-model fields (margins, padding,
    // border-radius) and element backgrounds should NOT cascade.
    let mut style = b.current_style();
    style.margin_top_px = 0.0;
    style.margin_bottom_px = 0.0;
    style.margin_left_px = 0.0;
    style.padding_top_px = 0.0;
    style.padding_right_px = 0.0;
    style.padding_bottom_px = 0.0;
    style.padding_left_px = 0.0;
    style.border_radius_px = 0.0;
    if block_display {
        style.bg_color = None;
    }
    apply_tag_style(name, attrs, &mut style);
    apply_decls(&decls, &mut style);
    b.style_stack.push(style);

    b.element_stack.push(OpenElement {
        tag: name.to_string(),
        classes,
        id,
        style_pushed: true,
        block_display,
        hidden: false,
    });

    // Tag-specific behaviors that go beyond styling.
    match name {
        "a" => {
            let href = attrs.get("href").cloned().unwrap_or_default();
            if !href.is_empty() {
                b.link_stack.push(classify_link(&href));
            } else {
                b.link_stack.push(Link::External(SharedString::default()));
            }
        }
        "ul" => b.list_stack.push(false),
        "ol" => b.list_stack.push(true),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = name.as_bytes()[1] - b'0';
            *pending_heading = Some(level);
        }
        _ => {}
    }
}

fn handle_close(b: &mut Builder, name: &str, pending_heading: &mut Option<u8>) {
    // Void elements (`<img>`, `<br>`, `<hr>`, …) are only emitted as
    // SelfClose, so they never push to element_stack. Some real-world
    // MDX HTML still writes an explicit closer like `<img ... ></img>`;
    // popping element_stack on those would unbalance the style/link
    // chain for everything that follows.
    if matches!(name, "br" | "img" | "hr" | "meta" | "link" | "input") {
        return;
    }

    // Resilience against unbalanced HTML: if the stack top has a
    // different tag, look further down. If we find a matching open,
    // discard everything above it (treating those as auto-closed).
    let pos = b
        .element_stack
        .iter()
        .rposition(|e| e.tag.eq_ignore_ascii_case(name));
    let Some(target) = pos else {
        // No matching open — drop the stray close.
        return;
    };

    // Auto-close any elements between `target` and the top.
    while b.element_stack.len() > target + 1 {
        auto_close_top(b);
    }
    let Some(open) = b.element_stack.pop() else {
        return;
    };

    if open.hidden {
        if b.skip_depth > 0 {
            b.skip_depth -= 1;
        }
        return;
    }

    if open.block_display {
        if matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
            if let Some(level) = pending_heading.take() {
                b.flush_heading(level);
            } else {
                b.flush_paragraph();
            }
        } else {
            b.flush_paragraph();
        }
    }

    if open.style_pushed {
        b.close_style();
    }

    match name {
        "a" => {
            // If this was a sound link that produced no inline content
            // (e.g. <a href="sound://..." class="fa fa-volume-up"></a>),
            // emit a sound inline now so the renderer shows a button.
            if let Some(Link::Sound(path)) = b.link_stack.last().cloned() {
                let closing_style = b.current_style();
                let link_present = b
                    .inline_buf
                    .last()
                    .is_some_and(|r| matches!(&r.link, Some(Link::Sound(p)) if p == &path));
                if !link_present {
                    b.inline_buf.push(Inline {
                        text: SharedString::default(),
                        style: closing_style,
                        link: Some(Link::Sound(path)),
                        image: None,
                    });
                }
            }
            b.link_stack.pop();
        }
        "ul" | "ol" => {
            b.list_stack.pop();
        }
        _ => {}
    }
}

/// Pop the topmost open element as if it had been explicitly closed.
/// Used to recover from out-of-order close tags.
fn auto_close_top(b: &mut Builder) {
    let Some(open) = b.element_stack.pop() else {
        return;
    };
    if open.hidden {
        if b.skip_depth > 0 {
            b.skip_depth -= 1;
        }
        return;
    }
    if open.block_display {
        b.flush_paragraph();
    }
    if open.style_pushed {
        b.close_style();
    }
    if open.tag == "a" {
        b.link_stack.pop();
    } else if open.tag == "ul" || open.tag == "ol" {
        b.list_stack.pop();
    }
}

/// Hard-coded styles for plain HTML formatting tags.
fn apply_tag_style(name: &str, attrs: &HashMap<String, String>, s: &mut Style) {
    match name {
        "b" | "strong" => s.bold = true,
        "i" | "em" => s.italic = true,
        "u" => s.underline = true,
        "sup" => s.superscript = true,
        "sub" => s.subscript = true,
        "font" => {
            if let Some(c) = attrs.get("color") {
                s.color = Some(SharedString::from(c.clone()));
            }
        }
        _ => {}
    }
}

/// Collapse runs of whitespace (including newlines) to a single space.
/// HTML treats them as equivalent inside text content.
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = out.ends_with(' ');
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(c);
            prev_ws = false;
        }
    }
    out
}
