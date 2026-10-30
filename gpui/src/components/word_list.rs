use gpui::{
    AppContext as _, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::v_flex;

use crate::{colors, state::DictState};

const LIST_WIDTH: f32 = 300.;

/// Props for the word list component.
pub struct WordListProps {
    pub state: Entity<DictState>,
}

/// Renders the left panel with a scrollable list of matching words.
pub fn word_list(props: WordListProps, cx: &mut gpui::App) -> gpui::AnyElement {
    let state = props.state.read(cx);
    let suggestions: Vec<String> = state.suggestions.clone();
    let selected = state.selected_suggestion;
    let state_handle = props.state.clone();

    if suggestions.is_empty() {
        return empty_list();
    }

    let items: Vec<_> = suggestions
        .into_iter()
        .enumerate()
        .map(|(idx, word)| word_item(idx, word, selected == Some(idx), state_handle.clone()))
        .collect();

    v_flex()
        .id("word-list")
        .track_scroll(&state_handle.read(cx).word_list_scroll)
        .w(px(LIST_WIDTH))
        .min_w(px(LIST_WIDTH))
        .max_w(px(LIST_WIDTH))
        .flex_shrink_0()
        .min_h(px(0.))
        .h_full()
        .bg(colors::bg())
        .border_r_1()
        .border_color(colors::border())
        .overflow_y_scroll()
        .children(items)
        .into_any_element()
}

fn empty_list() -> gpui::AnyElement {
    v_flex()
        .id("word-list-empty")
        .w(px(LIST_WIDTH))
        .min_w(px(LIST_WIDTH))
        .max_w(px(LIST_WIDTH))
        .flex_shrink_0()
        .min_h(px(0.))
        .h_full()
        .bg(colors::bg())
        .border_r_1()
        .border_color(colors::border())
        .items_center()
        .justify_center()
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child("No matches"),
        )
        .into_any_element()
}

fn word_item(
    idx: usize,
    word: String,
    is_selected: bool,
    state: Entity<DictState>,
) -> gpui::AnyElement {
    let bg = if is_selected {
        colors::primary()
    } else {
        colors::bg()
    };
    let text_col = if is_selected {
        colors::bg()
    } else {
        colors::text()
    };
    let hover_bg = if is_selected {
        colors::primary()
    } else {
        colors::surface()
    };

    div()
        .id(SharedString::from(format!("word-{idx}")))
        .w_full()
        .px(px(12.))
        .py(px(5.))
        .cursor_pointer()
        .bg(bg)
        .hover(move |s| s.bg(hover_bg))
        .child(
            div()
                .text_size(px(13.))
                .text_color(text_col)
                .overflow_hidden()
                .whitespace_nowrap()
                .child(SharedString::from(word.clone())),
        )
        .on_click(move |_, _, cx| {
            let w = word.clone();
            dicto_telemetry::get().track(dicto_telemetry::Event::LookupPerformed {
                source: dicto_telemetry::LookupSource::Click,
            });
            let results: Vec<crate::state::DictResult> = mdict_rs::query::query_all(&w)
                .into_iter()
                .map(|hit| {
                    let blocks = crate::html::parse_styled(&hit.definition, &hit.stem);
                    let audio = crate::html::first_sound_path(&blocks);
                    crate::state::DictResult {
                        short_name: hit.short_name,
                        blocks,
                        audio,
                    }
                })
                .collect();
            cx.update_entity(&state, |s, cx| {
                s.selected_suggestion = Some(idx);
                s.result_word = Some(w);
                s.is_searching = false;
                s.results = results;
                s.active_result = 0;
                cx.notify();
            });
        })
        .into_any_element()
}
