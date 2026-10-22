//! An adaptive option picker: chips while the catalog is small, a dropdown
//! once it is not.
//!
//! Given `items`, `selected`, and `chips_up_to`, the picker renders itself:
//! a wrapped chip row for lists of up to that many options, a dropdown
//! beyond. The dropdown is our own popover — search box, wheel scroll with
//! a visible scrollbar, and keyboard navigation through the search input:
//! single-line inputs let `MoveUp`/`MoveDown`/`Enter`/`Escape` actions
//! propagate when they don't apply, so the popover handles them (move
//! cursor, confirm, close). It replaced `gpui_component::select::Select`
//! because that component's dropdown misbehaves with catalogs of hundreds
//! of models (no working keyboard path, janky scroll).
//!
//! [`OptionPicker::set_selection`] reconciles items + selection when the
//! backing data changes.

use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, Focusable as _, InteractiveElement, IntoElement,
    ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, anchored, deferred, div, point, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Sizable as _, h_flex,
    input::{Enter as InputEnter, Escape as InputEscape, Input, InputState, MoveDown, MoveUp},
    scroll::Scrollbar,
    v_flex,
};

use crate::colors;

/// One option row: `label` is displayed, `id` is the value handed to
/// `on_change` — they differ when an option's machine value is not its human
/// label ("gpt-4o-mini" vs "GPT-4o mini").
#[derive(Clone, PartialEq)]
pub struct PickerItem {
    pub id: SharedString,
    pub label: SharedString,
}

impl PickerItem {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// Callback invoked with the picked item's id — from a chip click, a row
/// click, or the keyboard cursor alike.
pub type OnPick = Arc<dyn Fn(&str, &mut Window, &mut gpui::App) + 'static>;

/// Props for [`OptionPicker::new`] — bundled like the popup's `PopupProps`.
pub struct PickerProps {
    /// Stable element-id prefix; keeps chip and row element ids unique
    /// across multiple pickers rendered in the same window.
    pub id: SharedString,
    pub items: Vec<PickerItem>,
    /// The initially selected item id, if any.
    pub selected: Option<SharedString>,
    /// Chips for lists of up to this many options; a dropdown beyond it
    /// (`items.len() > chips_up_to`).
    pub chips_up_to: usize,
    /// Dropdown hint shown while nothing is selected.
    pub placeholder: Option<SharedString>,
    /// Invoked with the picked item's `id`.
    pub on_change: OnPick,
}

/// Fixed geometry of the dropdown's rows and viewport — the scroll
/// follow-the-cursor math depends on both.
const ROW_H: f32 = 26.0;
const MENU_H: f32 = 312.0;

/// The adaptive picker: chips for lists of up to `chips_up_to` options, a
/// searchable dropdown beyond.
pub struct OptionPicker {
    id: SharedString,
    items: Vec<PickerItem>,
    selected: Option<SharedString>,
    chips_up_to: usize,
    placeholder: Option<SharedString>,
    on_change: OnPick,

    // Dropdown-mode state.
    open: bool,
    /// Set when the popover was dismissed by a click outside; the next
    /// trigger click consumes it instead of toggling (otherwise the
    /// outside-dismiss on mousedown plus the trigger's click would close
    /// and instantly reopen the menu).
    out_dismiss: bool,
    query: Entity<InputState>,
    /// Items matching the current query — the dropdown renders these.
    filtered: Vec<PickerItem>,
    /// Keyboard cursor index into `filtered`.
    cursor: Option<usize>,
    scroll: ScrollHandle,
}

impl OptionPicker {
    pub fn new(props: PickerProps, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let PickerProps {
            id,
            items,
            selected,
            chips_up_to,
            placeholder,
            on_change,
        } = props;

        let query = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder("Search…", window, cx);
            s
        });
        // Re-filter whenever the search box edits (observe fires on every
        // InputState notify).
        cx.observe(&query, |this, input, cx| {
            let q = input.read(cx).value().to_string();
            this.refilter(&q);
            cx.notify();
        })
        .detach();

        let mut this = Self {
            id,
            items,
            selected,
            chips_up_to,
            placeholder,
            on_change,
            open: false,
            out_dismiss: false,
            query,
            filtered: Vec::new(),
            cursor: None,
            scroll: ScrollHandle::new(),
        };
        this.refilter("");
        this
    }

    /// Reconcile items + selection with new backing data (catalog swap,
    /// external edit, cascade). Safe to call repeatedly.
    ///
    /// The rows call this on every render — when nothing changed we must
    /// return before touching the query input, or the in-progress search
    /// text would be wiped on each keystroke.
    pub fn set_selection(
        &mut self,
        items: Vec<PickerItem>,
        selected: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.items == items && self.selected == selected {
            return;
        }
        self.items = items;
        self.selected = selected;
        // A swapped catalog invalidates the query — start clean.
        self.query
            .update(cx, |s, cx| s.set_value(String::new(), window, cx));
        self.refilter("");
        cx.notify();
    }

    /// The currently selected item id, if any.
    #[allow(dead_code)]
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    fn uses_dropdown(&self) -> bool {
        self.items.len() > self.chips_up_to
    }

    fn refilter(&mut self, query: &str) {
        let q = query.to_lowercase();
        self.filtered = if q.is_empty() {
            self.items.clone()
        } else {
            self.items
                .iter()
                .filter(|item| {
                    item.label.to_lowercase().contains(&q) || item.id.to_lowercase().contains(&q)
                })
                .cloned()
                .collect()
        };
        // Park the cursor on the selected item when it survives the filter,
        // otherwise on the first row.
        self.cursor = match &self.selected {
            Some(sel) => self.filtered.iter().position(|item| item.id == *sel).or(
                if self.filtered.is_empty() {
                    None
                } else {
                    Some(0)
                },
            ),
            None if self.filtered.is_empty() => None,
            None => Some(0),
        };
    }

    fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        if open {
            // Hand focus straight to the search box: typing filters
            // immediately, and its arrow/enter actions bubble here.
            self.query
                .update(cx, |s, cx| s.focus_handle(cx).focus(window, cx));
        } else {
            self.query
                .update(cx, |s, cx| s.set_value(String::new(), window, cx));
            self.refilter("");
        }
        cx.notify();
    }

    fn move_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.filtered.len();
        if len == 0 {
            return;
        }
        let current = self.cursor.unwrap_or(0) as isize;
        // Wrap around the edges — small filtered lists make wrapping the
        // fastest way to reach the other end.
        let next = (current + delta).rem_euclid(len as isize) as usize;
        self.cursor = Some(next);
        self.scroll_cursor_into_view();
        cx.notify();
    }

    /// Keep the cursor row inside the viewport. Rows are fixed height
    /// (`ROW_H`), so this is plain offset math.
    fn scroll_cursor_into_view(&mut self) {
        let Some(ix) = self.cursor else {
            return;
        };
        let top = self.scroll.offset().y;
        let y = px(ix as f32 * ROW_H);
        let mut new_top = top;
        if y < top {
            new_top = y;
        } else if y + px(ROW_H) > top + px(MENU_H) {
            new_top = y + px(ROW_H) - px(MENU_H);
        }
        if new_top != top {
            self.scroll.set_offset(point(px(0.), new_top));
        }
    }

    fn confirm_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            self.set_open(true, window, cx);
            return;
        }
        let Some(ix) = self.cursor else {
            return;
        };
        let Some(item) = self.filtered.get(ix) else {
            return;
        };
        let id = item.id.clone();
        self.selected = Some(id.clone());
        self.set_open(false, window, cx);
        (self.on_change)(id.as_ref(), window, cx);
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.set_open(false, window, cx);
        }
    }

    // --- Keyboard handlers. The search input's own actions bubble here:
    // single-line inputs no-op `MoveUp`/`MoveDown`/`Enter` and propagate
    // `Escape`, so arrow keys, enter, and escape all work while typing.

    fn on_input_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(-1, cx);
    }
    fn on_input_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(1, cx);
    }
    fn on_input_enter(&mut self, _: &InputEnter, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_cursor(window, cx);
    }
    fn on_input_escape(&mut self, _: &InputEscape, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }
}

impl Render for OptionPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.uses_dropdown() {
            let mut row = h_flex().gap(px(4.)).flex_wrap();
            for (ix, item) in self.items.iter().enumerate() {
                let selected = Some(item.id.as_ref()) == self.selected.as_deref();
                row = row.child(chip(
                    format!("{}-chip-{ix}", self.id),
                    item.label.clone(),
                    selected,
                    item.id.clone(),
                    self.on_change.clone(),
                ));
            }
            return row.into_any_element();
        }

        // While the popover is open the search box owns the keyboard: keep
        // focus on it even after clicks elsewhere inside the popover.
        if self.open {
            let query_handle = self.query.read(cx).focus_handle(cx);
            if !query_handle.is_focused(window) {
                query_handle.focus(window, cx);
            }
        }

        // Dropdown mode: the trigger toggles the popover; the popover's
        // search input owns the keyboard (see the on_action handlers on the
        // root — they catch the input's propagated actions).
        let trigger_label = self
            .selected
            .as_ref()
            .and_then(|sel| self.items.iter().find(|i| &i.id == sel))
            .map(|item| item.label.clone())
            .or_else(|| self.placeholder.clone())
            .unwrap_or_else(|| SharedString::from("Select…"));

        let trigger = h_flex()
            .id(SharedString::from(format!("{}-trigger", self.id)))
            .items_center()
            .gap(px(6.))
            .w_full()
            .px(px(8.))
            .py(px(3.))
            .rounded(px(4.))
            .cursor_pointer()
            .text_size(px(11.))
            .bg(colors::surface())
            .text_color(colors::text())
            .border_1()
            .border_color(colors::border())
            .hover(|s| s.bg(colors::hover()))
            .child(div().flex_1().overflow_hidden().child(trigger_label))
            .child(
                div()
                    .text_color(colors::text_secondary())
                    .child(SharedString::from(if self.open { "▴" } else { "▾" })),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                // Consume the click that follows an outside-dismiss (the
                // mousedown already closed the menu — reopening it here
                // would make the menu un-closable via the trigger).
                if this.out_dismiss {
                    this.out_dismiss = false;
                    return;
                }
                this.set_open(!this.open, window, cx);
            }));

        let root = div()
            .on_action(cx.listener(Self::on_input_up))
            .on_action(cx.listener(Self::on_input_down))
            .on_action(cx.listener(Self::on_input_enter))
            .on_action(cx.listener(Self::on_input_escape))
            .w_full()
            .child(trigger);

        if !self.open {
            return root.into_any_element();
        }

        let rows: Vec<_> = self
            .filtered
            .iter()
            .enumerate()
            .map(|(ix, item)| {
                let is_cursor = self.cursor == Some(ix);
                let is_selected = Some(item.id.as_ref()) == self.selected.as_deref();
                let mut row = div()
                    .id(SharedString::from(format!("{}-row-{ix}", self.id)))
                    .flex()
                    .items_center()
                    .w_full()
                    .h(px(ROW_H))
                    .px(px(8.))
                    .cursor_pointer()
                    .text_size(px(11.))
                    .when(is_selected, |s| {
                        s.text_color(colors::primary())
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                    })
                    .when(is_cursor, |s| s.bg(colors::hover()))
                    .when(!is_cursor, |s| s.hover(|h| h.bg(colors::hover())))
                    .child(div().overflow_hidden().child(item.label.clone()));
                row = row.on_click(cx.listener(move |this, _, window, cx| {
                    this.cursor = Some(ix);
                    this.confirm_cursor(window, cx);
                }));
                row
            })
            .collect();

        // Overlay popover: floats above the content below it instead of
        // pushing it down. `occlude` + `on_mouse_down_out` close it on any
        // outside click. (The earlier typing bug was the per-render query
        // wipe, not the overlay layer.)
        let dropdown = v_flex()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                if this.open {
                    this.out_dismiss = true;
                    this.close(window, cx);
                }
            }))
            .w(px(300.))
            .mt(px(6.))
            .bg(colors::surface())
            .border_1()
            .border_color(colors::border())
            .rounded(px(6.))
            .shadow_md()
            .child(
                div().px(px(6.)).pt(px(6.)).pb(px(4.)).child(
                    Input::new(&self.query)
                        .appearance(true)
                        .small()
                        .w_full()
                        .text_size(px(11.))
                        .py(px(2.))
                        .bg(colors::bg())
                        .border_color(colors::border())
                        .rounded(px(4.)),
                ),
            )
            // Scroll area + visible scrollbar need a relative wrapper so the
            // bar can position itself along the container's edge.
            .child(
                div()
                    .relative()
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-scroll", self.id)))
                            .max_h(px(MENU_H))
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .py(px(4.))
                            .when(self.filtered.is_empty(), |s| {
                                s.child(
                                    div()
                                        .w_full()
                                        .flex()
                                        .justify_center()
                                        .py(px(12.))
                                        .text_size(px(11.))
                                        .text_color(colors::text_secondary())
                                        .child(SharedString::from("No matches")),
                                )
                            })
                            .children(rows),
                    )
                    .child(Scrollbar::vertical(&self.scroll)),
            );

        root.child(
            deferred(
                anchored()
                    .snap_to_window_with_margin(px(8.))
                    .child(dropdown),
            )
            .with_priority(1),
        )
        .into_any_element()
    }
}

/// One compact chip of the chip mode (list of up to `chips_up_to` options);
/// commits its id through the shared `on_change`.
fn chip(
    id: String,
    label: SharedString,
    selected: bool,
    value: SharedString,
    on_change: OnPick,
) -> gpui::AnyElement {
    let base = div()
        .id(id)
        .px(px(8.))
        .py(px(3.))
        .rounded(px(4.))
        .cursor_pointer()
        .text_size(px(11.))
        .on_click(move |_, window, cx| on_change(value.as_ref(), window, cx));

    if selected {
        base.bg(colors::primary())
            .text_color(colors::bg())
            .child(label)
            .into_any_element()
    } else {
        base.bg(colors::surface())
            .text_color(colors::text_secondary())
            .border_1()
            .border_color(colors::border())
            .child(label)
            .into_any_element()
    }
}
