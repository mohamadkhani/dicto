use gpui::{
    AppContext as _, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, div, px,
};
use gpui_component::{
    scroll::ScrollableElement,
    tab::{Tab, TabBar},
    v_flex,
};

use crate::{
    colors,
    html::render_blocks,
    state::{DictResult, DictState},
};

/// Renders the right panel showing the selected word's definition.
///
/// The heading and tab strip live above the scroll area so that the
/// TabBar's horizontal-overflow scroller never captures wheel events
/// meant for the body's vertical scroll.
pub fn detail_panel(state: Entity<DictState>, cx: &mut gpui::App) -> gpui::AnyElement {
    let s = state.read(cx);

    let (header, body): (Option<gpui::AnyElement>, gpui::AnyElement) = if s.is_searching {
        (None, loading_view())
    } else if let Some(word) = &s.result_word {
        if s.results.is_empty() {
            (None, not_found_view(word))
        } else {
            let header = definition_header(word, &s.results, s.active_result, state.clone());
            let blocks = s
                .results
                .get(s.active_result)
                .map(|r| r.blocks.as_slice())
                .unwrap_or(&[]);
            (Some(header), render_blocks(blocks))
        }
    } else {
        (None, placeholder_view())
    };

    // Outer column holds the (fixed-height) header and a flex_1
    // container for the scrollable body. We need the explicit
    // inner sized container because gpui-component's `overflow_y_scrollbar`
    // wraps the element in a `size_full()` div, which only fills a
    // parent that already has a bounded height.
    let mut wrap = v_flex()
        .id("detail-panel-wrap")
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .h_full()
        .bg(colors::surface());

    if let Some(h) = header {
        wrap = wrap.child(h);
    }

    wrap.child(
        div().flex_1().min_h(px(0.)).w_full().child(
            v_flex()
                .id("detail-panel-body")
                .size_full()
                .px(px(16.))
                .py(px(8.))
                .child(body)
                .overflow_y_scrollbar(),
        ),
    )
    .into_any_element()
}

fn definition_header(
    word: &str,
    results: &[DictResult],
    active: usize,
    state: Entity<DictState>,
) -> gpui::AnyElement {
    let mut col = v_flex()
        .w_full()
        .px(px(16.))
        .pt(px(12.))
        .pb(px(4.))
        .gap(px(8.))
        .child(word_heading(word));

    if results.len() > 1 {
        col = col.child(tab_strip(results, active, state));
    } else {
        col = col.child(divider());
    }

    col.into_any_element()
}

fn tab_strip(results: &[DictResult], active: usize, state: Entity<DictState>) -> gpui::AnyElement {
    let state_for_click = state.clone();
    TabBar::new("detail-dict-tabs")
        .underline()
        .selected_index(active)
        .children(
            results
                .iter()
                .map(|r| Tab::new().label(SharedString::from(r.short_name.clone()))),
        )
        .on_click(move |idx: &usize, _window, cx| {
            let i = *idx;
            cx.update_entity(&state_for_click, |s, cx| {
                s.active_result = i;
                cx.notify();
            });
        })
        .into_any_element()
}

fn loading_view() -> gpui::AnyElement {
    v_flex()
        .w_full()
        .items_center()
        .justify_center()
        .py(px(60.))
        .child(
            div()
                .text_size(px(14.))
                .text_color(colors::text_secondary())
                .child("Loading..."),
        )
        .into_any_element()
}

fn not_found_view(word: &str) -> gpui::AnyElement {
    v_flex()
        .w_full()
        .items_center()
        .justify_center()
        .py(px(60.))
        .gap(px(8.))
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::text())
                .child(SharedString::from(word.to_string())),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors::text_secondary())
                .child("No definition found"),
        )
        .into_any_element()
}

fn placeholder_view() -> gpui::AnyElement {
    v_flex()
        .w_full()
        .items_center()
        .justify_center()
        .py(px(60.))
        .gap(px(12.))
        .child(
            div()
                .text_size(px(32.))
                .font_weight(FontWeight::BOLD)
                .text_color(colors::primary())
                .child("DICTO"),
        )
        .child(
            div()
                .text_size(px(14.))
                .text_color(colors::text_secondary())
                .child("Type to search words"),
        )
        .into_any_element()
}

fn word_heading(word: &str) -> gpui::AnyElement {
    div()
        .text_size(px(20.))
        .font_weight(FontWeight::BOLD)
        .text_color(colors::primary())
        .child(SharedString::from(word.to_string()))
        .into_any_element()
}

fn divider() -> gpui::AnyElement {
    div()
        .h(px(1.))
        .w_full()
        .bg(colors::border())
        .into_any_element()
}
