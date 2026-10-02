//! RTL-aware multi-line text editor (the popup's "Original" edit block).
//!
//! gpui's own wrapping (`compute_wrap_boundaries`) assumes byte order ==
//! visual order, which scrambles wrapped RTL (Persian/Arabic) lines, and its
//! `LineWrapper` does not know the Arabic script, so it breaks words mid-glyph
//! run. gpui-component's `Input` builds on that pipeline, so it cannot be
//! fixed by configuration. cosmic-text itself implements the Unicode bidi
//! algorithm CORRECTLY for a single line: shaped glyphs come out in visual
//! order with correct joined forms and ligatures, each with an absolute x
//! position and the byte index it came from.
//!
//! This editor therefore NEVER reorders characters (double-applying bidi
//! corrupts letter forms): it wraps LOGICALLY at SPACES ONLY, with real shaped
//! widths (see [`crate::bidi::wrap_line_segments`]), shapes each wrapped
//! segment itself via `layout_line`, and paints the resulting glyphs directly.
//!
//! IME composition is out of scope for v1: Persian typing uses a keyboard
//! layout (direct key events), not a composition pipeline.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, ElementId, Entity, EventEmitter, FocusHandle,
    Font, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, InteractiveElement,
    IntoElement, KeyDownEvent, LayoutId, LineLayout, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, Refineable, RenderOnce, ScrollDelta,
    ScrollWheelEvent, Style, StyleRefinement, Styled as _, Window, div, fill, point, px, size,
};

use crate::{bidi, colors};

/// Horizontal text inset inside the element bounds (2px each side → the wrap
/// width is the bounds width minus 4px).
const TEXT_INSET: Pixels = px(2.);

/// Caret quad width.
const CARET_WIDTH: Pixels = px(1.5);

/// Overlay scrollbar: thumb width, inset from the element's right edge, and
/// the smallest thumb height (for very long content).
const SCROLLBAR_WIDTH: Pixels = px(4.);
const SCROLLBAR_RIGHT_INSET: Pixels = px(3.);
const SCROLLBAR_MIN_THUMB: Pixels = px(24.);

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Emitted by [`EditorState`] on every text mutation.
pub(crate) enum EditorEvent {
    Change(String),
}

// ---------------------------------------------------------------------------
// Layout info (written by the element each prepaint, read everywhere else)
// ---------------------------------------------------------------------------

/// One wrapped line: its byte range in the edited text plus its shaped
/// layout. Glyphs inside `layout` are in VISUAL order with absolute x
/// positions and the byte index each came from.
#[derive(Clone)]
pub(crate) struct LineInfo {
    /// Byte offset of the line's first byte in the edited text.
    pub(crate) byte_start: usize,
    /// Line length in bytes (a segment from [`bidi::wrap_line_segments`]).
    pub(crate) len: usize,
    /// Shaped layout of `&text[byte_start..byte_start + len]`.
    pub(crate) layout: Arc<LineLayout>,
    /// Horizontal paint offset from the text area's left edge. 0 for LTR;
    /// for RTL paragraphs each line hugs the RIGHT edge, so this pushes it
    /// right by the unused width (no padding characters).
    pub(crate) offset: Pixels,
}

/// Everything the editor needs to paint and to map pixels ↔ byte indices,
/// refreshed by the element every prepaint.
#[derive(Clone)]
pub(crate) struct EditorLayoutInfo {
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) lines: Vec<LineInfo>,
    pub(crate) line_height: Pixels,
    /// Full (unscrolled) content height: `lines.len() * line_height`.
    pub(crate) height: Pixels,
    /// Cascaded text color captured at prepaint time.
    pub(crate) text_color: Hsla,
}

// ---------------------------------------------------------------------------
// Pure editing model (no gpui App — this crate's tests cannot construct one)
// ---------------------------------------------------------------------------

/// Pure text-editing model: the logical text plus cursor/anchor BYTE offsets.
/// Every mutation keeps `cursor`/`anchor` on UTF-8 char boundaries.
#[derive(Clone, Debug, Default)]
pub(crate) struct EditorCore {
    pub(crate) text: String,
    /// Caret position; always on a char boundary.
    pub(crate) cursor: usize,
    /// Selection origin; the selection is `min(cursor, anchor)..max(..)`.
    pub(crate) anchor: Option<usize>,
}

impl EditorCore {
    pub(crate) fn selection_range(&self) -> Option<Range<usize>> {
        self.anchor
            .map(|a| a.min(self.cursor)..a.max(self.cursor))
            .filter(|range| !range.is_empty())
    }

    pub(crate) fn selected_text(&self) -> String {
        self.selection_range()
            .map(|range| self.text[range].to_string())
            .unwrap_or_default()
    }

    /// Deletes the selection, if any. Returns true when text changed.
    fn delete_selection(&mut self) -> bool {
        if let Some(range) = self.selection_range() {
            self.text.replace_range(range.clone(), "");
            self.cursor = range.start;
            self.anchor = None;
            true
        } else {
            self.anchor = None;
            false
        }
    }

    /// Inserts `s` at the caret (replacing the selection). Returns true.
    fn insert(&mut self, s: &str) -> bool {
        self.delete_selection();
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
        true
    }

    fn backspace(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor > 0 {
            let start = prev_char(&self.text, self.cursor);
            self.text.replace_range(start..self.cursor, "");
            self.cursor = start;
            return true;
        }
        false
    }

    fn delete_forward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor < self.text.len() {
            let end = next_char(&self.text, self.cursor);
            self.text.replace_range(self.cursor..end, "");
            return true;
        }
        false
    }

    fn move_left(&mut self) {
        self.cursor = prev_char(&self.text, self.cursor);
    }

    fn move_right(&mut self) {
        self.cursor = next_char(&self.text, self.cursor);
    }

    fn move_word_left(&mut self) {
        self.cursor = prev_word_boundary(&self.text, self.cursor);
    }

    fn move_word_right(&mut self) {
        self.cursor = next_word_boundary(&self.text, self.cursor);
    }
}

fn prev_char_boundary(text: &str, index: usize) -> usize {
    let mut i = index.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn next_char_boundary(text: &str, index: usize) -> usize {
    let mut i = index.min(text.len());
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// One character backward. Unlike [`prev_char_boundary`] (which only snaps),
/// this always moves a full char when not already at 0.
fn prev_char(text: &str, index: usize) -> usize {
    let i = prev_char_boundary(text, index);
    if i == 0 {
        return 0;
    }
    prev_char_boundary(text, i - 1)
}

/// One character forward. Unlike [`next_char_boundary`] (which only snaps),
/// this always moves a full char when not already at the end.
fn next_char(text: &str, index: usize) -> usize {
    let i = next_char_boundary(text, index);
    if i >= text.len() {
        return text.len();
    }
    next_char_boundary(text, i + 1)
}

/// Previous whitespace-delimited word boundary (byte-safe, logical order).
fn prev_word_boundary(text: &str, index: usize) -> usize {
    let mut i = prev_char(text, index);
    //.skip trailing whitespace before the word
    while i > 0 && char_before(text, i).is_whitespace() {
        i = prev_char(text, i);
    }
    //.then skip the word itself
    while i > 0 && !char_before(text, i).is_whitespace() {
        i = prev_char(text, i);
    }
    i
}

/// Next whitespace-delimited word boundary (byte-safe, logical order).
fn next_word_boundary(text: &str, index: usize) -> usize {
    let mut i = next_char(text, index);
    //.skip trailing whitespace after the cursor
    while i < text.len() && char_at(text, i).is_whitespace() {
        i = next_char(text, i);
    }
    //.then skip ahead to the end of the word
    while i < text.len() && !char_at(text, i).is_whitespace() {
        i = next_char(text, i);
    }
    //.and consume the separator so the caret lands on the next word start
    while i < text.len() && char_at(text, i).is_whitespace() {
        i = next_char(text, i);
    }
    i
}

fn char_at(text: &str, index: usize) -> char {
    text[index..].chars().next().unwrap_or('\0')
}

fn char_before(text: &str, index: usize) -> char {
    text[..index].chars().next_back().unwrap_or('\0')
}

// ---------------------------------------------------------------------------
// Pure pixel ↔ byte-index mapping (works on shaped LineLayouts)
// ---------------------------------------------------------------------------

/// Index of the wrapped line the cursor sits on. Bytes that belong to no
/// segment (break spaces, `\n`) attach to the line before them.
fn line_index_for_cursor(lines: &[LineInfo], cursor: usize) -> usize {
    let mut best = 0;
    for (ix, line) in lines.iter().enumerate() {
        if cursor >= line.byte_start && cursor <= line.byte_start + line.len {
            return ix;
        }
        if cursor >= line.byte_start {
            best = ix;
        }
    }
    best
}

/// Whether the caret at `index` follows RTL reading order: decided by the
/// script of the char starting at `index`, falling back to the previous char
/// for neutral chars (spaces) and end-of-line.
fn caret_is_rtl(line_text: &str, index: usize) -> bool {
    if index < line_text.len() {
        let c = char_at(line_text, index);
        if bidi::is_rtl_script_char(c) {
            return true;
        }
        if !c.is_whitespace() {
            return false;
        }
    }
    if index > 0 {
        let prev = char_before(line_text, index);
        return bidi::is_rtl_script_char(prev);
    }
    false
}

/// The caret's x position (relative to the line's left edge) for the byte
/// boundary `index`.
///
/// Glyphs are in VISUAL order with absolute x. For LTR text the caret sits at
/// the left edge of the next glyph (`LineLayout::x_for_index` semantics).
/// For RTL text the boundary between logical chars k and k+1 is the LEFT edge
/// of char k's glyph (logical reading proceeds right→left), so we take the
/// glyph with the largest byte index below `index`. Never reorder characters.
fn caret_x(layout: &LineLayout, line_text: &str, index: usize) -> Pixels {
    if caret_is_rtl(line_text, index) {
        let mut best_x = layout.width;
        let mut best_index = usize::MIN;
        for run in &layout.runs {
            for glyph in &run.glyphs {
                if glyph.index < index && glyph.index >= best_index {
                    best_index = glyph.index;
                    best_x = glyph.position.x;
                }
            }
        }
        best_x
    } else {
        for run in &layout.runs {
            for glyph in &run.glyphs {
                if glyph.index >= index {
                    return glyph.position.x;
                }
            }
        }
        layout.width
    }
}

/// The visual x span covering every glyph whose byte index lies in
/// `[start, end)` — a union, so it is correct for both LTR and RTL segments
/// (for RTL, `x_for_index(start)..x_for_index(end)` collapses to nothing).
/// A glyph's right edge is the next glyph's x (or the line width for the
/// last glyph), matching how cosmic-text lays out consecutive glyphs.
fn glyph_x_range(layout: &LineLayout, start: usize, end: usize) -> Option<(Pixels, Pixels)> {
    let mut min_x: Option<Pixels> = None;
    let mut max_x = px(0.);
    let glyphs: Vec<&gpui::ShapedGlyph> = layout
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter())
        .collect();
    for (i, glyph) in glyphs.iter().enumerate() {
        if glyph.index >= start && glyph.index < end {
            let right = glyphs
                .get(i + 1)
                .map_or(layout.width, |next| next.position.x.max(glyph.position.x));
            min_x = Some(min_x.map_or(glyph.position.x, |min| min.min(glyph.position.x)));
            max_x = max_x.max(right);
        }
    }
    min_x.map(|min| (min, max_x.max(min)))
}

/// Map a window-space point to the closest byte index. `scroll_y` is the
/// editor's current vertical scroll offset.
fn index_for_position(
    info: &EditorLayoutInfo,
    position: Point<Pixels>,
    scroll_y: Pixels,
    text_len: usize,
) -> usize {
    let y = position.y - info.bounds.origin.y + scroll_y;
    let line_ix = ((f32::from(y) / f32::from(info.line_height)).floor().max(0.) as usize)
        .min(info.lines.len().saturating_sub(1));
    let line = &info.lines[line_ix];
    // Undo the line's right-alignment offset before asking the layout.
    let x = position.x - info.bounds.origin.x - TEXT_INSET - line.offset;
    let local = line.layout.closest_index_for_x(x.max(px(0.)));
    (line.byte_start + local).min(text_len)
}

/// Move the cursor one wrapped line up/down, keeping the VISUAL x position
/// (line offsets may differ between right-aligned RTL lines).
/// Returns true when the cursor moved.
fn move_cursor_vertically(core: &mut EditorCore, info: &EditorLayoutInfo, down: bool) -> bool {
    let line_ix = line_index_for_cursor(&info.lines, core.cursor);
    let Some(current) = info.lines.get(line_ix) else {
        return false;
    };
    let line_text = core
        .text
        .get(current.byte_start..current.byte_start + current.len)
        .unwrap_or("");
    let visual_x =
        current.offset + caret_x(&current.layout, line_text, core.cursor - current.byte_start);

    let target_line = if down {
        info.lines.get(line_ix + 1)
    } else {
        line_ix.checked_sub(1).and_then(|ix| info.lines.get(ix))
    };
    let target_cursor = match target_line {
        Some(target) => {
            let local = target
                .layout
                .closest_index_for_x((visual_x - target.offset).max(px(0.)));
            (target.byte_start + local).min(core.text.len())
        }
        // Past the last/first line: clamp to the text end/start.
        None if down => core.text.len(),
        None => 0,
    };
    let target_cursor = prev_char_boundary(&core.text, target_cursor);
    if target_cursor == core.cursor {
        return false;
    }
    core.cursor = target_cursor;
    true
}

/// The scroll offset that brings the cursor line into view, if the current
/// `scroll_y` hides it.
fn desired_scroll_y(
    info: &EditorLayoutInfo,
    cursor: usize,
    scroll_y: Pixels,
    max_scroll: Pixels,
) -> Option<Pixels> {
    let ix = line_index_for_cursor(&info.lines, cursor);
    let top = info.line_height * (ix as f32);
    let bottom = top + info.line_height;
    let view = info.bounds.size.height;
    let target = if top < scroll_y {
        top
    } else if bottom > scroll_y + view {
        bottom - view
    } else {
        return None;
    };
    Some(target.clamp(px(0.), max_scroll))
}

/// Overlay-scrollbar thumb height for a viewport of `viewport` px and a
/// content height of `content` px: proportional to the visible fraction,
/// clamped to a usable minimum. Both values must be positive.
fn scrollbar_thumb_height(viewport: Pixels, content: Pixels) -> Pixels {
    let vh = f32::from(viewport);
    let ch = f32::from(content).max(1.);
    px((vh * vh / ch).clamp(f32::from(SCROLLBAR_MIN_THUMB), vh))
}

/// The overlay scrollbar's thumb rect inside `bounds`, or `None` when the
/// content does not overflow (`max_scroll == 0`). The thumb's top follows
/// `scroll_y` proportionally along the scroll range.
fn scrollbar_thumb(
    bounds: &Bounds<Pixels>,
    content_height: Pixels,
    scroll_y: Pixels,
    max_scroll: Pixels,
) -> Option<Bounds<Pixels>> {
    if max_scroll <= px(0.) {
        return None;
    }
    let viewport = bounds.size.height;
    let thumb_height = scrollbar_thumb_height(viewport, content_height);
    let usable = (viewport - thumb_height).max(px(0.));
    let progress = (f32::from(scroll_y) / f32::from(max_scroll)).clamp(0., 1.);
    let top = bounds.origin.y + usable * progress;
    let right = bounds.right() - SCROLLBAR_RIGHT_INSET;
    Some(Bounds::new(
        point(right - SCROLLBAR_WIDTH, top),
        size(SCROLLBAR_WIDTH, thumb_height),
    ))
}

// ---------------------------------------------------------------------------
// EditorState — the gpui entity
// ---------------------------------------------------------------------------

/// Stateful editor: the pure [`EditorCore`] model plus the gpui-bound state
/// (focus, scroll, last layout, drag). Text mutations emit
/// [`EditorEvent::Change`]; every visible change calls `cx.notify()`.
pub(crate) struct EditorState {
    pub(crate) core: EditorCore,
    /// Vertical scroll offset (clamped to the content height).
    pub(crate) scroll_y: Pixels,
    focus: FocusHandle,
    /// Layout captured by the element's last prepaint.
    pub(crate) last_layout: Option<EditorLayoutInfo>,
    /// A left-drag is extending the selection.
    pub(crate) dragging: bool,
    /// Selectable but NOT editable (the Translation block): mouse selection,
    /// arrows, ctrl+a/ctrl+c work; every text mutation is ignored and no
    /// caret is painted.
    pub(crate) read_only: bool,
    /// Whether the next paint should scroll the caret's line into view.
    /// Cursor moves and edits set it, paint consumes it. Wheel scrolling
    /// never sets it — otherwise paint's reveal would snap the view back to
    /// the caret every frame and the wheel could never scroll away.
    pub(crate) reveal_cursor: bool,
    /// While dragging the overlay scrollbar: the pointer's y offset inside
    /// the thumb at grab time. `None` when not dragging.
    pub(crate) scrollbar_grab: Option<Pixels>,
}

impl EventEmitter<EditorEvent> for EditorState {}

impl EditorState {
    pub(crate) fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            core: EditorCore::default(),
            scroll_y: px(0.),
            focus: cx.focus_handle(),
            last_layout: None,
            dragging: false,
            read_only: false,
            reveal_cursor: false,
            scrollbar_grab: None,
        }
    }

    /// A selectable-but-not-editable editor (the Translation block).
    pub(crate) fn new_read_only(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            read_only: true,
            ..Self::new(_window, cx)
        }
    }

    pub(crate) fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// Programmatic text reset (not an edit: no Change event). Resets the
    /// cursor/anchor and scroll.
    pub(crate) fn set_text(&mut self, text: &str, _window: &mut Window, cx: &mut Context<Self>) {
        self.core = EditorCore {
            text: text.to_string(),
            cursor: text.len(),
            anchor: None,
        };
        self.scroll_y = px(0.);
        self.reveal_cursor = false;
        cx.notify();
    }

    pub(crate) fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.reveal_cursor = true;
        cx.emit(EditorEvent::Change(self.core.text.clone()));
        cx.notify();
    }

    fn move_cursor(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut EditorCore)) {
        let before = (self.core.cursor, self.core.anchor);
        f(&mut self.core);
        if (self.core.cursor, self.core.anchor) != before {
            self.reveal_cursor = true;
            cx.notify();
        }
    }

    pub(crate) fn insert_text(&mut self, s: &str, cx: &mut Context<Self>) {
        self.core.insert(s);
        self.changed(cx);
    }

    pub(crate) fn backspace(&mut self, cx: &mut Context<Self>) {
        if self.core.backspace() {
            self.changed(cx);
        }
    }

    pub(crate) fn delete_forward(&mut self, cx: &mut Context<Self>) {
        if self.core.delete_forward() {
            self.changed(cx);
        }
    }

    pub(crate) fn insert_newline(&mut self, cx: &mut Context<Self>) {
        self.insert_text("\n", cx);
    }

    pub(crate) fn move_left(&mut self, cx: &mut Context<Self>) {
        self.move_cursor(cx, EditorCore::move_left);
    }

    pub(crate) fn move_right(&mut self, cx: &mut Context<Self>) {
        self.move_cursor(cx, EditorCore::move_right);
    }

    pub(crate) fn move_word_left(&mut self, cx: &mut Context<Self>) {
        self.move_cursor(cx, EditorCore::move_word_left);
    }

    pub(crate) fn move_word_right(&mut self, cx: &mut Context<Self>) {
        self.move_cursor(cx, EditorCore::move_word_right);
    }

    pub(crate) fn move_line_start(&mut self, cx: &mut Context<Self>) {
        let target = self
            .last_layout
            .as_ref()
            .map(|info| info.lines[line_index_for_cursor(&info.lines, self.core.cursor)].byte_start)
            .unwrap_or(0);
        self.move_cursor(cx, |core| core.cursor = target);
    }

    pub(crate) fn move_line_end(&mut self, cx: &mut Context<Self>) {
        let target = self
            .last_layout
            .as_ref()
            .map(|info| {
                let line = &info.lines[line_index_for_cursor(&info.lines, self.core.cursor)];
                (line.byte_start + line.len).min(self.core.text.len())
            })
            .unwrap_or(self.core.text.len());
        self.move_cursor(cx, |core| core.cursor = target);
    }

    pub(crate) fn move_up(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.last_layout.clone() else {
            return;
        };
        if move_cursor_vertically(&mut self.core, &info, false) {
            cx.notify();
        }
    }

    pub(crate) fn move_down(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.last_layout.clone() else {
            return;
        };
        if move_cursor_vertically(&mut self.core, &info, true) {
            cx.notify();
        }
    }

    pub(crate) fn select_all(&mut self, cx: &mut Context<Self>) {
        self.move_cursor(cx, |core| {
            core.anchor = Some(0);
            core.cursor = core.text.len();
        });
    }

    pub(crate) fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let selected = self.core.selected_text();
        if !selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(selected));
        }
    }

    pub(crate) fn cut_selection(&mut self, cx: &mut Context<Self>) {
        if self.core.selection_range().is_some() {
            self.copy_selection(cx);
            self.core.delete_selection();
            self.changed(cx);
        }
    }

    pub(crate) fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            // Normalize line endings; the editor's hard breaks are '\n'.
            let text = text.replace("\r\n", "\n").replace('\r', "\n");
            if !text.is_empty() {
                self.insert_text(&text, cx);
            }
        }
    }

    /// Map a window-space click/drag position to a byte index using the last
    /// layout (clamped to the text and its char boundaries).
    fn index_for_click(&self, position: Point<Pixels>) -> usize {
        match &self.last_layout {
            Some(info) => {
                let index = index_for_position(info, position, self.scroll_y, self.core.text.len());
                prev_char_boundary(&self.core.text, index)
            }
            None => self.core.cursor,
        }
    }

    /// Key dispatch: arrows (+shift extends the selection), home/end,
    /// backspace/delete, enter, ctrl+a/c/x/v, and printable input. Escape is
    /// left unhandled so it bubbles to the popup root.
    pub(crate) fn handle_key(
        &mut self,
        ev: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &ev.keystroke;
        let modifiers = &keystroke.modifiers;

        // Shift+move extends: set the anchor (once) before moving; a plain
        // move drops the selection.
        let extend = modifiers.shift;
        let prepare = |this: &mut Self, cx: &mut Context<Self>| {
            this.move_cursor(cx, |core| {
                if extend {
                    core.anchor = core.anchor.or(Some(core.cursor));
                } else {
                    core.anchor = None;
                }
            });
        };

        if modifiers.control {
            match keystroke.key.as_str() {
                "a" => self.select_all(cx),
                "c" => self.copy_selection(cx),
                "x" if !self.read_only => self.cut_selection(cx),
                "v" if !self.read_only => self.paste_clipboard(cx),
                // Ctrl+arrows jump by words.
                "left" => {
                    prepare(self, cx);
                    self.move_word_left(cx);
                }
                "right" => {
                    prepare(self, cx);
                    self.move_word_right(cx);
                }
                _ => {}
            }
            return;
        }
        if modifiers.alt || modifiers.platform {
            return;
        }

        match keystroke.key.as_str() {
            "left" => {
                prepare(self, cx);
                self.move_left(cx);
            }
            "right" => {
                prepare(self, cx);
                self.move_right(cx);
            }
            "up" => {
                prepare(self, cx);
                self.move_up(cx);
            }
            "down" => {
                prepare(self, cx);
                self.move_down(cx);
            }
            "home" => {
                prepare(self, cx);
                self.move_line_start(cx);
            }
            "end" => {
                prepare(self, cx);
                self.move_line_end(cx);
            }
            "backspace" if !self.read_only => self.backspace(cx),
            "delete" if !self.read_only => self.delete_forward(cx),
            "enter" if !self.read_only => self.insert_newline(cx),
            "space" if !self.read_only => self.insert_text(" ", cx),
            _ => {
                if self.read_only {
                    return;
                }
                // Printable input: prefer the platform-typed char; fall back
                // to the key glyph (uppercasing ascii for shift ourselves —
                // tolerate gpui already reporting the shifted case).
                let ch = match keystroke.key_char.as_ref() {
                    Some(c) => c.clone(),
                    None if keystroke.key.chars().count() == 1 => {
                        let mut c = keystroke.key.clone();
                        if modifiers.shift {
                            let lower = c.chars().next().unwrap();
                            if lower.is_ascii_lowercase() {
                                c = lower.to_ascii_uppercase().to_string();
                            }
                        }
                        c
                    }
                    None => String::new(),
                };
                if !ch.is_empty() {
                    self.insert_text(&ch, cx);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Element
// ---------------------------------------------------------------------------

/// RenderOnce wrapper: the focusable div (focus + key dispatch + layout box)
/// around the custom [`TextEditorElement`] that lays out and paints the text.
#[derive(IntoElement)]
pub(crate) struct TextEditor {
    state: Entity<EditorState>,
    height: Pixels,
    id: &'static str,
}

/// Build the editor element tree for `state`. The `height` is the definite
/// layout height; the actual box comes from the parent (`.size_full()` here).
/// `id` must be unique per editor instance in the popup (stateful div id).
pub(crate) fn text_editor(
    state: &Entity<EditorState>,
    height: Pixels,
    id: &'static str,
) -> AnyElement {
    TextEditor {
        state: state.clone(),
        height,
        id,
    }
    .into_any_element()
}

impl RenderOnce for TextEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = self.state.read(cx).focus_handle().clone();
        div()
            .id(self.id)
            .track_focus(&focus)
            .on_key_down(window.listener_for(&self.state, EditorState::handle_key))
            .size_full()
            // Clip painted lines to this box: the element paints with its own
            // fixed height regardless of the layout bounds, and its line
            // culling only knows the (much larger) window content mask —
            // without this mask, content past the box paints over the
            // sections below.
            .overflow_hidden()
            .child(TextEditorElement {
                state: self.state,
                height: self.height,
            })
    }
}

/// The custom element: space-only logical wrapping (via [`bidi`]), per-line
/// shaping, glyph painting, and mouse position↔index mapping. Mirrors gpui's
/// `TextElement` mechanics.
struct TextEditorElement {
    state: Entity<EditorState>,
    height: Pixels,
}

impl IntoElement for TextEditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for TextEditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // Full width, definite height.
        let mut style = Style::default();
        style.refine(&StyleRefinement::default().w_full().h(self.height));
        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        // Derive font/size/color from the cascaded text style so the popup's
        // .text_size()/.text_color() apply.
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let font = Font {
            family: text_style.font_family.clone(),
            features: text_style.font_features.clone(),
            fallbacks: text_style.font_fallbacks.clone(),
            weight: text_style.font_weight,
            style: text_style.font_style,
        };
        let wrap_width = (bounds.size.width - TEXT_INSET * 2.).max(px(0.));
        let line_height = window.line_height();

        let info = {
            let state = self.state.read(cx);
            let text = state.core.text.clone();
            // Auto RTL: an RTL paragraph hugs the right edge (UAX #9 P2/P3
            // direction from the first strong character). Each line is
            // painted with an x offset — no padding characters, the edited
            // text stays pristine.
            let rtl = bidi::first_paragraph_is_rtl(&text);
            let segments =
                bidi::wrap_line_segments(window.text_system(), &text, font, font_size, wrap_width);
            let lines = segments
                .into_iter()
                .map(|range| {
                    let line_text = &text[range.clone()];
                    let run = text_style.to_run(line_text.len());
                    let layout =
                        window
                            .text_system()
                            .layout_line(line_text, font_size, &[run], None);
                    let offset = if rtl {
                        (wrap_width - layout.width).max(px(0.))
                    } else {
                        px(0.)
                    };
                    LineInfo {
                        byte_start: range.start,
                        len: range.len(),
                        layout,
                        offset,
                    }
                })
                .collect::<Vec<_>>();
            let height = line_height * (lines.len() as f32);
            EditorLayoutInfo {
                bounds,
                lines,
                line_height,
                height,
                text_color: text_style.color,
            }
        };
        // Invisible state: no notify.
        self.state
            .update(cx, |state, _| state.last_layout = Some(info));
        Some(window.insert_hitbox(bounds, HitboxBehavior::Normal))
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(hitbox) = prepaint.take() else {
            return;
        };
        let (info, cursor, selection, focused, read_only, mut scroll_y, max_scroll, reveal_cursor) = {
            let state = self.state.read(cx);
            let Some(info) = state.last_layout.clone() else {
                return;
            };
            let max_scroll = (info.height - info.bounds.size.height).max(px(0.));
            let scroll_y = state.scroll_y.clamp(px(0.), max_scroll);
            (
                info,
                state.core.cursor.min(state.core.text.len()),
                state.core.selection_range(),
                state.focus.is_focused(window),
                state.read_only,
                scroll_y,
                max_scroll,
                state.reveal_cursor,
            )
        };

        // Scroll the caret's line into view, but ONLY when a cursor move or
        // an edit asked for it (reveal_cursor). Revealing unconditionally
        // here would snap the view back to the caret on every frame and make
        // mouse-wheel scrolling impossible: the wheel moves scroll_y, the
        // notify re-renders, and the reveal immediately undoes it. The flag
        // is consumed either way, so a reveal fires at most once per move.
        if reveal_cursor {
            if let Some(target) = desired_scroll_y(&info, cursor, scroll_y, max_scroll) {
                scroll_y = target;
                let state = self.state.clone();
                state.update(cx, |state, cx| {
                    state.scroll_y = target;
                    state.reveal_cursor = false;
                    cx.notify();
                });
            } else {
                self.state
                    .update(cx, |state, _cx| state.reveal_cursor = false);
            }
        }

        let cursor_line = line_index_for_cursor(&info.lines, cursor);
        let content_mask = window.content_mask().bounds;

        for (i, line) in info.lines.iter().enumerate() {
            let line_top = info.bounds.origin.y + info.line_height * (i as f32) - scroll_y;
            let line_bottom = line_top + info.line_height;
            if line_bottom < content_mask.top() || line_top > content_mask.bottom() {
                continue;
            }
            let text_x = info.bounds.origin.x + TEXT_INSET + line.offset;

            // Selection quad: the union of the glyphs whose byte indices fall
            // inside the selection ∩ this line.
            if let Some(sel) = &selection {
                let line_end = line.byte_start + line.len;
                let start = sel.start.max(line.byte_start);
                let end = sel.end.min(line_end);
                if start < end {
                    let local_start = start - line.byte_start;
                    let local_end = end - line.byte_start;
                    if let Some((x1, x2)) = glyph_x_range(&line.layout, local_start, local_end) {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(text_x + x1, line_top),
                                size((x2 - x1).max(px(1.)), info.line_height),
                            ),
                            colors::primary().alpha(0.25),
                        ));
                    }
                }
            }

            // Glyphs — cosmic-text already produced them in visual order with
            // absolute x positions and correct joined forms; paint as-is.
            // Baseline math copied from gpui's line painting.
            let padding_top = (info.line_height - line.layout.ascent - line.layout.descent) / 2.;
            let baseline_y = line_top + padding_top + line.layout.ascent;
            for run in &line.layout.runs {
                for glyph in &run.glyphs {
                    let origin = point(text_x + glyph.position.x, baseline_y + glyph.position.y);
                    if glyph.is_emoji {
                        window
                            .paint_emoji(origin, run.font_id, glyph.id, line.layout.font_size)
                            .ok();
                    } else {
                        window
                            .paint_glyph(
                                origin,
                                run.font_id,
                                glyph.id,
                                line.layout.font_size,
                                info.text_color,
                            )
                            .ok();
                    }
                }
            }

            // Caret (only on its line, only while focused; a read-only
            // editor shows selections but never a caret).
            if focused && !read_only && i == cursor_line {
                let state = self.state.read(cx);
                let line_text = &state.core.text[line.byte_start..line.byte_start + line.len];
                let local = cursor.saturating_sub(line.byte_start).min(line.len);
                let x = caret_x(&line.layout, line_text, local);
                window.paint_quad(fill(
                    Bounds::new(
                        point(text_x + x, line_top),
                        size(CARET_WIDTH, info.line_height),
                    ),
                    colors::text(),
                ));
            }
        }

        // Overlay scrollbar thumb: only when the content overflows, painted
        // last so it sits on top of the text.
        if let Some(thumb) = scrollbar_thumb(&info.bounds, info.height, scroll_y, max_scroll) {
            window.paint_quad(gpui::quad(
                thumb,
                px(2.),
                colors::text_secondary().alpha(0.55),
                gpui::Edges::default(),
                gpui::transparent_black(),
                gpui::BorderStyle::default(),
            ));
        }

        // --- Mouse handling (paint-phase listeners, hitbox-guarded) ---
        // Left-down: focus, place the caret (shift+click extends), start a drag.
        {
            let state = self.state.clone();
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
                if !phase.bubble() || ev.button != MouseButton::Left || !hitbox.is_hovered(window) {
                    return;
                }
                let index = state.read(cx).index_for_click(ev.position);
                state.update(cx, |state, cx| {
                    state.core.cursor = index;
                    state.core.anchor = if ev.modifiers.shift {
                        state.core.anchor.or(Some(index))
                    } else {
                        Some(index)
                    };
                    state.dragging = true;
                    state.reveal_cursor = true;
                    state.focus.focus(window, cx);
                    cx.notify();
                });
                cx.stop_propagation();
            });
        }

        // Left-drag: extend the selection to the mouse.
        let state = self.state.clone();
        window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _window, cx| {
            if !phase.bubble() || ev.pressed_button != Some(MouseButton::Left) {
                return;
            }
            let index = {
                let state = state.read(cx);
                if !state.dragging {
                    return;
                }
                state.index_for_click(ev.position)
            };
            state.update(cx, |state, cx| {
                if state.core.cursor != index {
                    state.core.cursor = index;
                    state.reveal_cursor = true;
                    cx.notify();
                }
            });
            cx.stop_propagation();
        });

        // Left-up: end the drag.
        let state = self.state.clone();
        window.on_mouse_event(move |ev: &MouseUpEvent, phase, _window, cx| {
            if !phase.bubble() || ev.button != MouseButton::Left {
                return;
            }
            if state.read(cx).dragging {
                state.update(cx, |state, _| state.dragging = false);
                cx.stop_propagation();
            }
        });

        // Scroll wheel: scroll the content, clamped.
        {
            let state = self.state.clone();
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |ev: &ScrollWheelEvent, phase, window, cx| {
                if !phase.bubble() || !hitbox.should_handle_scroll(window) {
                    return;
                }
                let Some(info) = state.read(cx).last_layout.clone() else {
                    return;
                };
                let max_scroll = (info.height - info.bounds.size.height).max(px(0.));
                // gpui's wheel deltas are INVERTED relative to a top-down
                // offset: wheel-down arrives as a NEGATIVE delta.y (X11 maps
                // Down to -SCROLL_LINES; Wayland multiplies the positive axis
                // value by a -1.0 modifier), and gpui's own scroller absorbs
                // that by keeping its offset negative (clamped to
                // [-scroll_max, 0]). Our scroll_y is positive-downward, so
                // SUBTRACT the delta: wheel-down scrolls toward the text end.
                let dy = match ev.delta {
                    ScrollDelta::Pixels(delta) => delta.y,
                    ScrollDelta::Lines(delta) => info.line_height * delta.y,
                };
                let mut scrolled = false;
                state.update(cx, |state, cx| {
                    let target = (state.scroll_y - dy).clamp(px(0.), max_scroll);
                    if target != state.scroll_y {
                        state.scroll_y = target;
                        scrolled = true;
                        cx.notify();
                    }
                });
                // Consume the wheel only when this editor actually scrolled:
                // once it is at either end (or has no overflow at all) the
                // event must bubble so ancestors — e.g. the popup body —
                // scroll instead.
                if scrolled {
                    cx.stop_propagation();
                }
            });
        }

        // --- Scrollbar drag (registered after the text handlers: mouse
        // events dispatch in REVERSE registration order, so these run first
        // and their stop_propagation keeps a thumb press from also placing
        // the caret or ending the caret drag). ---
        // Left-down on the thumb: start the drag, remembering where inside
        // the thumb the pointer grabbed.
        {
            let state = self.state.clone();
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
                if !phase.bubble() || ev.button != MouseButton::Left || !hitbox.is_hovered(window) {
                    return;
                }
                let (info, scroll_y, max_scroll) = {
                    let state = state.read(cx);
                    let Some(info) = state.last_layout.clone() else {
                        return;
                    };
                    let max_scroll = (info.height - info.bounds.size.height).max(px(0.));
                    let scroll_y = state.scroll_y.clamp(px(0.), max_scroll);
                    (info, scroll_y, max_scroll)
                };
                let Some(thumb) = scrollbar_thumb(&info.bounds, info.height, scroll_y, max_scroll)
                else {
                    return;
                };
                // A little slack around the 4px thumb makes it grabbable.
                if !thumb.dilate(px(2.)).contains(&ev.position) {
                    return;
                }
                let grab = ev.position.y - thumb.origin.y;
                state.update(cx, |state, cx| {
                    state.scrollbar_grab = Some(grab);
                    state.focus.focus(window, cx);
                    cx.notify();
                });
                cx.stop_propagation();
            });
        }

        // Drag: the thumb's top follows the pointer (minus the grab offset),
        // and scroll_y follows the thumb proportionally along the track.
        {
            let state = self.state.clone();
            window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _window, cx| {
                if !phase.bubble() || ev.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let grab = match state.read(cx).scrollbar_grab {
                    Some(grab) => grab,
                    None => return,
                };
                let Some(info) = state.read(cx).last_layout.clone() else {
                    return;
                };
                let max_scroll = (info.height - info.bounds.size.height).max(px(0.));
                if max_scroll <= px(0.) {
                    return;
                }
                let thumb_height = scrollbar_thumb_height(info.bounds.size.height, info.height);
                let usable = (info.bounds.size.height - thumb_height).max(px(0.));
                let desired_top = ev.position.y - info.bounds.origin.y - grab;
                let progress = if usable > px(0.) {
                    (f32::from(desired_top) / f32::from(usable)).clamp(0., 1.)
                } else {
                    0.
                };
                let target = px(progress * f32::from(max_scroll));
                state.update(cx, |state, cx| {
                    if state.scroll_y != target {
                        state.scroll_y = target;
                        cx.notify();
                    }
                });
                cx.stop_propagation();
            });
        }

        // Left-up: end the scrollbar drag.
        {
            let state = self.state.clone();
            window.on_mouse_event(move |ev: &MouseUpEvent, phase, _window, cx| {
                if !phase.bubble() || ev.button != MouseButton::Left {
                    return;
                }
                if state.read(cx).scrollbar_grab.is_some() {
                    state.update(cx, |state, _cx| state.scrollbar_grab = None);
                    cx.stop_propagation();
                }
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (pure logic — this crate's tests cannot construct a gpui App)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{FontId, GlyphId, ShapedGlyph, ShapedRun};

    fn make_layout(glyphs: &[(f32, usize)], width: f32, len: usize) -> Arc<LineLayout> {
        Arc::new(LineLayout {
            font_size: px(13.),
            width: px(width),
            ascent: px(10.),
            descent: px(3.),
            len,
            runs: vec![ShapedRun {
                font_id: FontId(0),
                glyphs: glyphs
                    .iter()
                    .map(|&(x, index)| ShapedGlyph {
                        id: GlyphId(0),
                        position: point(px(x), px(0.)),
                        index,
                        is_emoji: false,
                    })
                    .collect(),
            }],
        })
    }

    fn core(text: &str, cursor: usize, anchor: Option<usize>) -> EditorCore {
        EditorCore {
            text: text.to_string(),
            cursor,
            anchor,
        }
    }

    // --- EditorCore: editing ---

    #[test]
    fn insert_appends_at_cursor() {
        // Cursor between ا and م (byte 6): the typed char lands between them.
        let mut c = core("سلام", 6, None);
        c.insert("د");
        assert_eq!(c.text, "سلادم");
        assert_eq!(c.cursor, 8); // past the inserted char
        assert!(c.text.is_char_boundary(c.cursor));
    }

    #[test]
    fn insert_replaces_selection() {
        let mut c = core("hello", 1, Some(4)); // selection 1..4 = "ell"
        c.insert("E");
        assert_eq!(c.text, "hEo");
        assert_eq!(c.cursor, 2);
        assert!(c.anchor.is_none());
    }

    #[test]
    fn backspace_removes_one_multibyte_char() {
        // Cursor between ل and ا (byte 4) — backspace removes ل (bytes 2..4).
        let mut c = core("سلام", 4, None);
        c.backspace();
        assert_eq!(c.text, "سام");
        assert_eq!(c.cursor, 2);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut c = core("سلام", 0, None);
        assert!(!c.backspace());
        assert_eq!(c.text, "سلام");
    }

    #[test]
    fn backspace_deletes_selection() {
        let mut c = core("سلام دنیا", "سلام".len() + 1, Some(0));
        c.backspace();
        assert_eq!(c.text, "دنیا");
        assert_eq!(c.cursor, 0);
    }

    #[test]
    fn delete_forward_removes_one_multibyte_char() {
        let mut c = core("سلام", 4, None);
        c.delete_forward();
        assert_eq!(c.text, "سلم"); // ا removed
        assert_eq!(c.cursor, 4);
    }

    #[test]
    fn move_left_right_walk_char_boundaries() {
        let text = "aسلامb";
        let mut c = core(text, text.len(), None);
        let mut seen = vec![c.cursor];
        while c.cursor > 0 {
            c.move_left();
            seen.push(c.cursor);
        }
        seen.reverse();
        let expected: Vec<usize> = std::iter::once(0)
            .chain(text.char_indices().map(|(i, ch)| i + ch.len_utf8()))
            .collect();
        assert_eq!(seen, expected);
        for _ in 0..text.len() {
            c.move_right();
        }
        assert_eq!(c.cursor, text.len());
    }

    #[test]
    fn move_word_is_whitespace_delimited_and_byte_safe() {
        let text = "سلام دنیا خد";
        let mut c = core(text, text.len(), None);
        c.move_word_left();
        assert_eq!(&text[..c.cursor], "سلام دنیا ");
        c.move_word_left();
        assert_eq!(&text[..c.cursor], "سلام ");
        c.move_word_left();
        assert_eq!(c.cursor, 0);
        c.move_word_right();
        assert_eq!(&text[..c.cursor], "سلام ");
        c.move_word_right();
        assert_eq!(&text[..c.cursor], "سلام دنیا ");
        c.move_word_right();
        assert_eq!(c.cursor, text.len());
    }

    #[test]
    fn insert_at_every_boundary_keeps_boundaries() {
        let text = "می‌روم و abc"; // Persian incl. ZWNJ + Latin
        for i in 0..=text.len() {
            if !text.is_char_boundary(i) {
                continue;
            }
            let mut c = core(text, i, None);
            c.insert("X");
            assert!(c.text.is_char_boundary(c.cursor), "cursor off boundary");
            assert_eq!(c.text.len(), text.len() + 1);
            // The parts around the insertion are untouched.
            assert!(c.text.starts_with(&text[..i]));
            assert!(c.text.ends_with(&text[i..]));
        }
    }

    #[test]
    fn selection_range_is_min_max_and_direction_free() {
        let mut c = core("abcdefgh", 2, Some(6));
        assert_eq!(c.selection_range(), Some(2..6));
        c.cursor = 6;
        c.anchor = Some(2);
        assert_eq!(c.selection_range(), Some(2..6));
        c.anchor = Some(c.cursor);
        assert_eq!(c.selection_range(), None); // empty = no selection
    }

    #[test]
    fn selected_and_cut_text_match_selection() {
        let text = "سلام دنیا";
        let mut c = core(text, 0, Some("سلام".len() + 1)); // "سلام "
        assert_eq!(c.selected_text(), "سلام ");
        // Cut = copy (asserted above) + delete the selection.
        c.delete_selection();
        assert_eq!(c.text, "دنیا");
        assert_eq!(c.cursor, 0);
    }

    // --- Pure mapping: caret_x / glyph_x_range (RTL + LTR) ---

    #[test]
    fn caret_x_ltr_uses_next_glyph_left_edge() {
        // "ab": glyphs a@0, b@10, width 20.
        let layout = make_layout(&[(0., 0), (10., 1)], 20., 2);
        assert_eq!(caret_x(&layout, "ab", 0), px(0.));
        assert_eq!(caret_x(&layout, "ab", 1), px(10.));
        assert_eq!(caret_x(&layout, "ab", 2), px(20.));
    }

    #[test]
    fn caret_x_rtl_uses_left_edge_of_preceding_char_glyph() {
        // "سلام" shaped: visual order م(6)@0, ا(4)@10, ل(2)@20, س(0)@30.
        let layout = make_layout(&[(0., 6), (10., 4), (20., 2), (30., 0)], 40., 8);
        assert_eq!(caret_x(&layout, "سلام", 0), px(40.)); // before س = right edge
        assert_eq!(caret_x(&layout, "سلام", 2), px(30.)); // between س and ل
        assert_eq!(caret_x(&layout, "سلام", 4), px(20.));
        assert_eq!(caret_x(&layout, "سلام", 6), px(10.));
        assert_eq!(caret_x(&layout, "سلام", 8), px(0.)); // after م = left edge
    }

    #[test]
    fn glyph_x_range_covers_rtl_selection_union() {
        // Selecting logical bytes [0, 4) (س and ل) covers visual [20, 40).
        let layout = make_layout(&[(0., 6), (10., 4), (20., 2), (30., 0)], 40., 8);
        assert_eq!(glyph_x_range(&layout, 0, 4), Some((px(20.), px(40.))));
        assert_eq!(glyph_x_range(&layout, 4, 8), Some((px(0.), px(20.))));
        assert_eq!(glyph_x_range(&layout, 0, 0), None);
    }

    #[test]
    fn glyph_x_range_ltr() {
        let layout = make_layout(&[(0., 0), (10., 1)], 20., 2);
        assert_eq!(glyph_x_range(&layout, 0, 1), Some((px(0.), px(10.))));
        assert_eq!(glyph_x_range(&layout, 0, 2), Some((px(0.), px(20.))));
    }

    // --- Pure mapping: lines ---

    fn two_line_info() -> (String, EditorLayoutInfo) {
        // "ab\ncd": line 0 = 0..2 ("ab"), line 1 = 3..5 ("cd"). Line height 20.
        let text = "ab\ncd".to_string();
        let info = EditorLayoutInfo {
            bounds: Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(100.), px(40.)),
            },
            lines: vec![
                LineInfo {
                    byte_start: 0,
                    len: 2,
                    layout: make_layout(&[(0., 0), (10., 1)], 20., 2),
                    offset: px(0.),
                },
                LineInfo {
                    byte_start: 3,
                    len: 2,
                    layout: make_layout(&[(0., 0), (10., 1)], 20., 2),
                    offset: px(0.),
                },
            ],
            line_height: px(20.),
            height: px(40.),
            text_color: colors::text(),
        };
        (text, info)
    }

    #[test]
    fn line_index_for_cursor_handles_gaps() {
        let (_, info) = two_line_info();
        assert_eq!(line_index_for_cursor(&info.lines, 0), 0);
        assert_eq!(line_index_for_cursor(&info.lines, 2), 0); // the '\n' byte
        assert_eq!(line_index_for_cursor(&info.lines, 3), 1);
        assert_eq!(line_index_for_cursor(&info.lines, 5), 1);
    }

    #[test]
    fn index_for_position_maps_lines_and_clamps() {
        let (text, info) = two_line_info();
        assert_eq!(
            index_for_position(&info, point(px(5.), px(5.)), px(0.), text.len()),
            0
        );
        assert_eq!(
            index_for_position(&info, point(px(5.), px(25.)), px(0.), text.len()),
            3
        );
        // Far right on a line → its end.
        assert_eq!(
            index_for_position(&info, point(px(500.), px(5.)), px(0.), text.len()),
            2
        );
        // Above/below the content clamps to the first/last line.
        assert_eq!(
            index_for_position(&info, point(px(5.), px(-50.)), px(0.), text.len()),
            0
        );
        assert_eq!(
            index_for_position(&info, point(px(5.), px(500.)), px(0.), text.len()),
            3
        );
        // Scroll offset shifts the line mapping.
        assert_eq!(
            index_for_position(&info, point(px(5.), px(5.)), px(20.), text.len()),
            3
        );
    }

    #[test]
    fn move_cursor_vertically_keeps_x_and_clamps() {
        let (text, info) = two_line_info();
        let mut c = core(&text, 4, None); // after 'c' on line 1
        assert!(move_cursor_vertically(&mut c, &info, true));
        assert_eq!(c.cursor, text.len()); // past the last line → text end
        let mut c = core(&text, 1, None); // between a and b on line 0 (x=10)
        assert!(move_cursor_vertically(&mut c, &info, true));
        assert_eq!(c.cursor, 4); // same x on line 1
        assert!(move_cursor_vertically(&mut c, &info, false));
        assert_eq!(c.cursor, 1);
        assert!(move_cursor_vertically(&mut c, &info, false));
        assert_eq!(c.cursor, 0); // above the first line → text start
    }

    #[test]
    fn desired_scroll_keeps_cursor_line_visible() {
        let (_, info) = two_line_info();
        // Bounds height 40 ≥ content 40 → nothing to do.
        assert_eq!(desired_scroll_y(&info, 3, px(0.), px(0.)), None);
        // Shrink the viewport: only line 0 visible, cursor on line 1.
        let mut scrolled = info.clone();
        scrolled.bounds.size.height = px(20.);
        assert_eq!(
            desired_scroll_y(&scrolled, 3, px(0.), px(20.)),
            Some(px(20.))
        );
        assert_eq!(desired_scroll_y(&scrolled, 3, px(20.), px(20.)), None);
        assert_eq!(
            desired_scroll_y(&scrolled, 0, px(20.), px(20.)),
            Some(px(0.))
        );
    }

    // --- Overlay scrollbar geometry ---

    #[test]
    fn scrollbar_thumb_none_without_overflow() {
        let (_, info) = two_line_info();
        // Bounds 100x40, content 40 → no overflow, no thumb.
        assert_eq!(
            scrollbar_thumb(&info.bounds, info.height, px(0.), px(0.)),
            None
        );
    }

    #[test]
    fn scrollbar_thumb_tracks_progress_proportionally() {
        let bounds = Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(100.), px(200.)),
        };
        let content = px(400.);
        let max = content - bounds.size.height; // 200
        let thumb = scrollbar_thumb(&bounds, content, px(0.), max).unwrap();
        assert_eq!(thumb.size.width, SCROLLBAR_WIDTH);
        assert_eq!(thumb.origin.y, px(0.)); // thumb height 200·200/400 = 100
        let mid = scrollbar_thumb(&bounds, content, px(100.), max).unwrap();
        assert_eq!(mid.origin.y, px(50.));
        let end = scrollbar_thumb(&bounds, content, px(300.), max).unwrap();
        assert_eq!(end.origin.y, px(100.)); // clamped to the track end
    }

    #[test]
    fn scrollbar_thumb_height_has_min_clamp() {
        // Very long content: thumb clamps to the minimum.
        assert_eq!(
            scrollbar_thumb_height(px(200.), px(100_000.)),
            SCROLLBAR_MIN_THUMB
        );
        // Content only slightly taller than the viewport: near-full thumb.
        assert!(scrollbar_thumb_height(px(200.), px(210.)) > px(190.));
    }

    /// Same two lines, but right-aligned (RTL): each line's glyphs are
    /// pushed right by its offset inside the 100px-wide text area.
    fn two_line_info_rtl() -> (String, EditorLayoutInfo) {
        let (text, mut info) = two_line_info();
        for line in &mut info.lines {
            line.offset = px(80.); // line width 20 → right edge at 100
        }
        (text, info)
    }

    #[test]
    fn index_for_position_accounts_for_rtl_right_alignment() {
        let (text, info) = two_line_info_rtl();
        // Clicking at the line's visual start (x=80) hits index 0…
        assert_eq!(
            index_for_position(&info, point(px(80.), px(5.)), px(0.), text.len()),
            0
        );
        // …and the far-left click (x=0) clamps to the line's first byte.
        assert_eq!(
            index_for_position(&info, point(px(0.), px(5.)), px(0.), text.len()),
            0
        );
        // x=90 is 10px into the line → between the two glyphs (index 1).
        assert_eq!(
            index_for_position(&info, point(px(90.), px(5.)), px(0.), text.len()),
            1
        );
    }

    #[test]
    fn move_cursor_vertically_keeps_visual_x_across_different_offsets() {
        // Line 0 offset 80 (as if width 20 in a 100px area), line 1 offset 60
        // (as if width 40). Caret at visual x=90 on line 0 (local 10) must
        // land at local 30 on line 1 — same VISUAL column, not same local x.
        let (text, mut info) = two_line_info();
        info.lines[0].offset = px(80.);
        info.lines[1].offset = px(60.);
        info.lines[1].layout = make_layout(&[(0., 0), (10., 1), (20., 1), (30., 1)], 40., 2);
        let mut c = core(&text, 1, None); // local x=10 on line 0 → visual 90
        assert!(move_cursor_vertically(&mut c, &info, true));
        assert_eq!(c.cursor, 3 + 1); // local 30 on line 1 = byte 4
    }
}
