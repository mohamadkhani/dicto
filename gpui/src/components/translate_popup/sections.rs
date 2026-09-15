//! The text sections of the popup: the "Original" block (shared by every
//! popup state), the "Translation" block, and the pinned Translate button.
//!
//! Both blocks are rendered by our own RTL-aware editor element
//! ([`crate::components::text_editor`]): the Original is editable, the
//! Translation is read-only (selectable + copyable, no caret). GPUI's own
//! wrapping mis-fragments RTL text, so neither block can use its text
//! elements; the editor wraps at spaces only with real shaped widths and
//! right-aligns RTL paragraphs via paint offsets. Copying additionally has a
//! dedicated icon button in each section header.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::{h_flex, v_flex};

use crate::{colors, components::text_editor, state::DictState};

use super::playback::{Slot, playback_controls};

pub(crate) type PlaybackSnapshot = crate::playback::PlaybackSnapshot;

/// Build a section header row: small uppercase label + status indicator +
/// COPY and play buttons. Mirrors the design's `dicto-section` component.
pub(crate) fn section_header(
    label: &str,
    slot: Slot,
    text: &str,
    lang: &str,
    tts: &mdict_rs::settings::TtsSettings,
    state: Entity<DictState>,
    pb: PlaybackSnapshot,
) -> gpui::AnyElement {
    h_flex()
        .id("qt-section-header")
        .items_center()
        .gap(px(8.))
        .w_full()
        // The label group GROWS to absorb all free space. This pins the
        // buttons to the row's right edge with no justify-between measurement:
        // a plain `justify_between` here measured the row's content as wider
        // than the 430px card and pushed the button OUTSIDE the popup window,
        // where it rendered fine but was clipped — "invisible".
        .child(
            h_flex()
                .flex_1()
                .items_center()
                .gap(px(6.))
                .min_w_0()
                .child(section_label(label))
                .when(
                    super::playback::section_status(&pb, text, tts).is_some(),
                    |this| this.child(super::playback::section_status(&pb, text, tts).unwrap()),
                ),
        )
        // The buttons must not drag the window: the card starts a window
        // move on any mouse-down that reaches it, so presses on interactive
        // children are stopped here.
        .child(
            h_flex()
                .gap(px(4.))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(copy_button(slot, text))
                .child(playback_controls(
                    slot,
                    text.to_string(),
                    lang.to_string(),
                    tts.clone(),
                    state,
                    pb,
                )),
        )
        .into_any_element()
}

/// Small square icon button that copies the section's text to the clipboard.
/// Styled after the TTS play button so the header reads as one control group.
fn copy_button(slot: Slot, text: &str) -> gpui::Stateful<gpui::Div> {
    let id = match slot {
        Slot::Source => "qt-copy-src",
        Slot::Translation => "qt-copy-tr",
    };
    let text = text.to_string();
    v_flex()
        .id(id)
        // Explicit square: matches the play button and keeps both axes pinned
        // inside the header row.
        .size(px(26.))
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .bg(colors::hover())
        .border_1()
        .border_color(colors::border())
        .hover(|s| s.bg(colors::border()))
        .cursor_pointer()
        .child(
            gpui::svg()
                .path("icons/copy.svg")
                .self_center()
                .size(px(14.))
                .text_color(colors::text()),
        )
        .on_click(move |_ev, _window, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
        })
}

/// The original-text section shared by every popup state: header + the
/// always-editable text block.
pub(crate) fn original_section(
    original: &str,
    block: gpui::AnyElement,
    tts: &mdict_rs::settings::TtsSettings,
    state: Entity<DictState>,
    pb: PlaybackSnapshot,
) -> gpui::AnyElement {
    v_flex()
        .gap(px(8.))
        .child(section_header(
            "Original",
            Slot::Source,
            original,
            &source_voice_lang(),
            tts,
            state,
            pb,
        ))
        .child(block)
        .child(divider())
        .into_any_element()
}

/// The always-EDITABLE Original block, built on our own RTL-aware editor
/// element ([`text_editor`]): raw text, logical space-only wrapping, auto
/// right-alignment for RTL paragraphs, shaped glyphs painted as cosmic-text
/// produced them — no scrambled lines, no mid-word breaks.
pub(crate) fn original_text_editor(state: &Entity<text_editor::EditorState>) -> gpui::AnyElement {
    h_flex()
        .id("qt-original-editor-wrap")
        .w_full()
        .h(px(120.))
        // Presses inside the editor place the cursor / select text — they
        // must not start a window drag (the card moves the window on any
        // mouse-down that reaches it).
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .text_size(px(13.))
        .text_color(colors::text())
        .child(text_editor::text_editor(
            state,
            px(120.),
            "qt-original-editor",
        ))
        .into_any_element()
}

/// The selectable-but-NOT-editable Translation block: same editor element in
/// read-only mode — mouse selection, ctrl+a/ctrl+c work, text cannot change
/// and no caret is shown.
pub(crate) fn translation_text_select(
    state: &Entity<text_editor::EditorState>,
) -> gpui::AnyElement {
    h_flex()
        .id("qt-translation-select-wrap")
        .w_full()
        .h(px(200.))
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .text_size(px(14.))
        .text_color(colors::text())
        .child(text_editor::text_editor(
            state,
            px(200.),
            "qt-translation-select",
        ))
        .into_any_element()
}

/// The primary action button: kicks off a translation via the engine. Always
/// present (disabled while loading) so the layout never jumps; label becomes
/// "Translating…" while busy.
pub(crate) fn translate_button(
    original: String,
    state: Entity<DictState>,
    busy: bool,
) -> gpui::AnyElement {
    let label = if busy { "Translating…" } else { "Translate" };
    let mut btn = div()
        .id("qt-translate-btn")
        .px(px(16.))
        .py(px(6.))
        .rounded(px(6.))
        .bg(colors::primary())
        .text_size(px(12.))
        .text_color(gpui::rgb(0x0f1117))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer();
    if busy {
        btn = btn.opacity(0.45);
    } else {
        btn = btn.hover(|s| s.opacity(0.9));
    }
    btn.child(SharedString::from(label))
        .on_click(move |_ev, _window, cx| {
            if busy {
                return;
            }
            let original = original.clone();
            let state = state.clone();
            cx.update_entity(&state, |s, cx| {
                if let Some(engine) = s.quick_translate_engine.as_mut() {
                    if let Some(job) = engine.restart_translation(original) {
                        spawn_translation(job, state.clone(), cx);
                    }
                    cx.notify();
                }
            });
        })
        .into_any_element()
}

fn divider() -> gpui::AnyElement {
    div().h(px(1.)).bg(colors::border()).into_any_element()
}

fn section_label(text: &str) -> gpui::AnyElement {
    div()
        .text_size(px(11.))
        .text_color(colors::text_secondary())
        .child(SharedString::from(text.to_uppercase()))
        .into_any_element()
}

/// Language hint for speaking the *source* text. We don't reliably know the
/// source language, so let the platform TTS pick its default voice.
fn source_voice_lang() -> String {
    String::new()
}

/// Run a `TranslationJob` on the background executor and feed its outcome back
/// into the engine, then notify the popup to re-render. Mirrors the poll loop's
/// spawn in app.rs.
fn spawn_translation(
    job: crate::quick_translate::TranslationJob,
    entity: Entity<DictState>,
    cx: &mut gpui::App,
) {
    cx.spawn(async move |cx| {
        let outcome = cx
            .background_executor()
            .spawn(async move { job.run() })
            .await;
        cx.update_entity(&entity, |s, cx| {
            if let Some(engine) = s.quick_translate_engine.as_mut() {
                engine.apply_translation_result(outcome);
            }
            // The translation slot was speaking the PREVIOUS translation
            // (if any); a fresh translation is now in `popup_status`. Stop
            // the stale playing clip — Ended/Loading remain (their stale
            // affordance is shown by the popup).
            let new_translation: String =
                s.quick_translate_engine
                    .as_ref()
                    .and_then(|e| match e.popup_status() {
                        crate::quick_translate::PopupStatus::Visible(
                            super::PopupState::Ready { translation, .. },
                        ) => Some(translation.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
            if !new_translation.is_empty() {
                s.invalidate_tts_clips(Some(&new_translation), None);
            } else {
                s.invalidate_tts_clips(None, None);
            }
            cx.notify();
        });
    })
    .detach();
}
