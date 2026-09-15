//! An adaptive option picker: chips while the catalog is small, a dropdown
//! once it is not.
//!
//! Given `items`, `selected`, and `chips_up_to`, the picker renders itself:
//! a wrapped chip row for lists of up to that many options, a compact
//! [`Select`] dropdown beyond it. Both modes commit through one
//! `on_change(value)` callback, so owners never care which mode is active —
//! a catalog that grows or shrinks past `chips_up_to` (e.g. the model list
//! when the provider switches) simply re-renders in the other mode.
//! [`OptionPicker::set_selection`] reconciles items + selection when the
//! backing data changes.
//!
//! General by design: any small-vs-large choice list (languages, models,
//! voices, dictionaries, themes…) can use it as-is.

use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_component::{IndexPath, Sizable, h_flex};

use crate::colors;

/// One option row: `label` is displayed, `id` is the value handed to
/// `on_change` — they differ when an option's machine value is not its human
/// label ("gpt-4o-mini" vs "GPT-4o mini").
#[derive(Clone)]
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

impl SelectItem for PickerItem {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

type PickerCatalog = Vec<PickerItem>;
type PickerSelect = Entity<SelectState<PickerCatalog>>;

/// Callback invoked with the picked item's id — from a chip click or a
/// dropdown confirmation alike.
pub type OnPick = Arc<dyn Fn(&str, &mut Window, &mut gpui::App) + 'static>;

/// Props for [`OptionPicker::new`] — bundled like the popup's `PopupProps`.
pub struct PickerProps {
    /// Stable element-id prefix; keeps chip element ids unique across
    /// multiple pickers rendered in the same window.
    pub id: SharedString,
    pub items: PickerCatalog,
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

/// The adaptive picker: chips for lists of up to `chips_up_to` options, a
/// dropdown beyond.
pub struct OptionPicker {
    select: PickerSelect,
    id: SharedString,
    items: PickerCatalog,
    selected: Option<SharedString>,
    chips_up_to: usize,
    placeholder: Option<SharedString>,
    on_change: OnPick,
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
        let ix = position_of(&items, selected.as_deref());
        let select = cx.new(|cx| SelectState::new(items.clone(), ix, window, cx));
        cx.subscribe_in(&select, window, Self::on_confirm).detach();
        Self {
            select,
            id,
            items,
            selected,
            chips_up_to,
            placeholder,
            on_change,
        }
    }

    /// Dropdown confirmations: remember the choice and hand it to the shared
    /// `on_change` (the [`SelectState`] has already moved its own cursor).
    fn on_confirm(
        &mut self,
        _: &PickerSelect,
        event: &SelectEvent<PickerCatalog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(id)) = event else {
            return;
        };
        self.selected = Some(id.clone());
        (self.on_change)(id.as_ref(), window, cx);
        cx.notify();
    }

    /// Reconcile items + selection with new backing data (catalog swap,
    /// external edit, cascade). Safe to call repeatedly.
    pub fn set_selection(
        &mut self,
        items: PickerCatalog,
        selected: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ix = position_of(&items, selected.as_deref());
        let select = self.select.clone();
        select.update(cx, |state, cx| {
            state.set_items(items.clone(), window, cx);
            state.set_selected_index(ix, window, cx);
        });
        self.items = items;
        self.selected = selected;
        cx.notify();
    }

    /// The currently selected item id, if any. Part of the component's
    /// general API; the translate popup reads settings from `DictState`
    /// instead, hence the allow.
    #[allow(dead_code)]
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    fn uses_dropdown(&self) -> bool {
        self.items.len() > self.chips_up_to
    }
}

impl Render for OptionPicker {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.uses_dropdown() {
            // The trigger is restyled to the chip-mode tokens (app design
            // system in `colors`): same text size, surface, border, and
            // radius as a chip, so both picker modes read as one control.
            // `Styled` refinements are applied after the theme's
            // `input_size`/`input_text_size`, so they win.
            let mut el = Select::new(&self.select)
                .small()
                .w_full()
                .text_size(px(11.))
                .py(px(3.))
                .bg(colors::surface())
                .text_color(colors::text())
                .border_color(colors::border())
                .rounded(px(4.));
            if let Some(placeholder) = &self.placeholder {
                el = el.placeholder(placeholder.clone());
            }
            el.into_any_element()
        } else {
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
            row.into_any_element()
        }
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

fn position_of(items: &PickerCatalog, id: Option<&str>) -> Option<IndexPath> {
    let id = id?;
    items
        .iter()
        .position(|item| item.id.as_ref() == id)
        .map(|row| IndexPath::default().row(row))
}
