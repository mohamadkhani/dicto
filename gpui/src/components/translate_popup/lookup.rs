//! The Word Lookup popup content.
//!
//! Sibling of `sections.rs` (translate states): builds the word header
//! (word + copy + Speak), the dictionary tab strip, the definition body,
//! and the footer action row ("Open in Dicto" / "Translate"). Everything
//! visual reuses existing pieces: the playback controls + copy button from
//! `sections.rs`/`playback.rs`, the `TabBar` from gpui_component (same as
//! the main window's detail panel), and the MDX block renderer from
//! `crate::html`.

use gpui::{
    AppContext as _, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::{
    h_flex,
    tab::{Tab, TabBar},
    v_flex,
};

use super::playback::{Slot, playback_controls};
use super::sections::{copy_button, spawn_translation};
use crate::{
    colors,
    components::{banner, spinner},
    html::render_blocks,
    state::{DictResult, DictState},
};

use crate::word_lookup::LookupState;

/// Body content for a lookup state — rendered inside the popup's shared
/// scrollable body, exactly like the translate sections. Returns the same
/// `Div` type as the translate content arms so the popup's content match
/// stays homogeneous.
pub(crate) fn lookup_body(
    state_entity: &Entity<DictState>,
    ps: &LookupState,
    tts: &mdict_rs::settings::TtsSettings,
    pb: super::sections::PlaybackSnapshot,
) -> gpui::Div {
    match ps {
        LookupState::Loading { word } => v_flex()
            .gap(px(8.))
            .child(word_header(word, None, tts, state_entity.clone(), pb))
            .child(spinner::spinner_row("Looking up…")),

        LookupState::Ready {
            word,
            results,
            active,
        } => {
            let audio = results
                .get(*active)
                .and_then(|r| r.audio.as_deref())
                .map(str::to_string);
            let mut col = v_flex().gap(px(8.)).child(word_header(
                word,
                audio.as_deref(),
                tts,
                state_entity.clone(),
                pb,
            ));
            // Tab strip only when more than one dictionary had a hit — a
            // single hit gets the whole card (same rule as the main window).
            if results.len() > 1 {
                col = col.child(dict_tabs(results, *active, state_entity.clone()));
            }
            if let Some(result) = results.get(*active) {
                col = col.child(definition_body(&result.blocks));
            }
            col
        }

        LookupState::NotFound { word } => v_flex()
            .gap(px(8.))
            .child(word_header(word, None, tts, state_entity.clone(), pb))
            .child(not_found_note()),

        // Hint states — no word header; their action lives in the body.
        LookupState::Disabled => v_flex()
            .gap(px(8.))
            .child(banner::warning_banner(
                "Word Lookup is off",
                "Enable it to look selected words up in your local \
                 dictionaries — no internet needed.",
            ))
            .child(enable_row(state_entity.clone())),

        LookupState::Error { message } => v_flex()
            .gap(px(8.))
            .child(banner::error_text("Word Lookup failed", message)),
    }
}

/// The "Enable Word Lookup" action for the Disabled hint state: flips the
/// setting, persists, re-registers the hotkey, and immediately runs the
/// lookup again so the word the user had selected just… looks up.
fn enable_row(state: Entity<DictState>) -> gpui::AnyElement {
    div()
        .id("wl-enable-btn")
        .px(px(14.))
        .py(px(7.))
        .rounded(px(6.))
        .bg(colors::primary())
        .text_size(px(12.))
        .text_color(gpui::rgb(0x0f1117))
        .font_weight(FontWeight::SEMIBOLD)
        .cursor_pointer()
        .hover(|s| s.opacity(0.9))
        .child(SharedString::from("Enable Word Lookup"))
        .on_click(move |_ev, _window, cx| {
            let state = state.clone();
            cx.update_entity(&state, |s, cx| {
                s.word_lookup.enabled = true;
                s.save_settings(cx);
                s.reload_hotkey(cx);
                if let Some(wl) = s.word_lookup_engine.as_mut() {
                    wl.hide_popup();
                }
                cx.notify();
            });
            // Re-run the lookup now that the feature is on. The engine
            // exists unconditionally, so this can't fail to find it.
            let job = state.update(cx, |s, _cx| {
                // Only re-runs when a selection is actually readable;
                // otherwise the engine shows its read-failure hint.
                s.word_lookup_engine
                    .as_mut()
                    .and_then(|wl| wl.trigger_lookup())
            });
            if let Some(job) = job {
                spawn_lookup(job, state, cx);
            }
        })
        .into_any_element()
}

/// Run a `LookupJob` on the background executor and feed the outcome back
/// into the engine. Lookup-side twin of `sections::spawn_translation`.
fn spawn_lookup(job: crate::word_lookup::LookupJob, entity: Entity<DictState>, cx: &mut gpui::App) {
    cx.spawn(async move |cx| {
        let outcome = cx
            .background_executor()
            .spawn(async move { job.run() })
            .await;
        cx.update_entity(&entity, |s, cx| {
            if let Some(engine) = s.word_lookup_engine.as_mut() {
                engine.apply_result(outcome);
            }
            cx.notify();
        });
    })
    .detach();
}

/// Footer action row for a lookup state: "Open in Dicto" (hand the parsed
/// results to the main window's detail panel) and "Translate" (AI fallback,
/// reuses the quick-translate engine). Hidden when no translator is
/// configured — the feature is disabled in settings.
pub(crate) fn lookup_footer(
    state_entity: &Entity<DictState>,
    has_translator: bool,
    measure: &super::MeasureProbes,
) -> gpui::AnyElement {
    let state = state_entity.clone();
    let open_row = div()
        .id("wl-open-dicto")
        .px(px(12.))
        .py(px(6.))
        .rounded(px(6.))
        .bg(colors::hover())
        .border_1()
        .border_color(colors::border())
        .text_size(px(12.))
        .text_color(colors::text())
        .cursor_pointer()
        .hover(|s| s.bg(colors::border()))
        .child(SharedString::from("Open in Dicto"))
        .on_click(move |_ev, window, cx| {
            // Move the parsed results into the main window's state, then
            // tear the popup down through the shared close path. The main
            // window shows the definition as soon as the popup uncovers it.
            let mut results_ready = false;
            state.update(cx, |s, _| {
                if let Some(wl) = s.word_lookup_engine.as_mut()
                    && let Some((word, results)) = wl.take_results()
                {
                    s.result_word = Some(word);
                    s.results = results;
                    s.active_result = 0;
                    results_ready = true;
                }
            });
            if results_ready {
                super::close_popup(&state, window, cx);
            }
        });

    let mut row = h_flex().justify_end().items_center().gap(px(8.));
    if has_translator {
        let state = state_entity.clone();
        row = row.child(
            div()
                .id("wl-translate-btn")
                .px(px(12.))
                .py(px(6.))
                .rounded(px(6.))
                .bg(colors::primary())
                .text_size(px(12.))
                .text_color(gpui::rgb(0x0f1117))
                .font_weight(FontWeight::SEMIBOLD)
                .cursor_pointer()
                .hover(|s| s.opacity(0.9))
                .child(SharedString::from("Translate"))
                .on_click(move |_ev, _window, cx| {
                    translate_word_from_lookup(&state, cx);
                }),
        );
    }
    let row = row.child(open_row);

    // Full-bleed footer with the same probe wiring as the Options footer so
    // the window-height machinery keeps working unchanged.
    v_flex()
        .flex_shrink_0()
        .border_t_1()
        .border_color(colors::border())
        .rounded_b(px(9.))
        .child(super::probe(measure.footer_top.clone()))
        .child(
            div()
                .flex()
                .justify_end()
                .px(px(14.))
                .py(px(8.))
                // No-drag wrapper: the buttons are interactive.
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(row),
        )
        .child(super::probe(measure.footer_end.clone()))
        .into_any_element()
}

/// The word header: big primary-colored word (like the main window's word
/// heading) + copy + Speak. Speak prefers the dictionary's bundled
/// pronunciation clip (MDD resource, played by `crate::audio`) and falls
/// back to TTS — the SOURCE playback slot, which the lookup popup never
/// uses for anything else.
fn word_header(
    word: &str,
    audio: Option<&str>,
    tts: &mdict_rs::settings::TtsSettings,
    state: Entity<DictState>,
    pb: super::sections::PlaybackSnapshot,
) -> gpui::AnyElement {
    h_flex()
        .items_center()
        .gap(px(8.))
        .w_full()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(20.))
                .font_weight(FontWeight::BOLD)
                .text_color(colors::primary())
                .child(SharedString::from(word.to_string())),
        )
        .child(
            h_flex()
                .gap(px(4.))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(copy_button("wl-copy", word))
                // Both controls when the dictionary bundles a clip: ▶ plays
                // the real human recording, the TTS controls stay available
                // for the synthesized voice.
                .children(audio.map(dict_audio_button))
                .child(playback_controls(
                    Slot::Source,
                    word.to_string(),
                    // No language hint — let the platform TTS pick a voice.
                    String::new(),
                    tts.clone(),
                    state,
                    pb,
                )),
        )
        .into_any_element()
}

/// Square speaker button that plays the dictionary's bundled MDD clip —
/// the same playback path (`crate::audio::play_resource`, rodio + ffmpeg
/// Speex transcode) the main window's inline sound pills use. Styled after
/// the copy button so the header reads as one control group.
fn dict_audio_button(path: &str) -> gpui::Stateful<gpui::Div> {
    let path = path.to_string();
    v_flex()
        .id("wl-dict-audio")
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
                .path("icons/play.svg")
                .self_center()
                .size(px(14.))
                .text_color(colors::text()),
        )
        .on_click(move |_ev, _window, _cx| {
            tracing::info!(path = %path, "word lookup: dict audio button clicked");
            dicto_telemetry::get().track(dicto_telemetry::Event::PronunciationPlayed);
            crate::audio::play_resource(&path);
        })
}

/// Dictionary tab strip — the same underlined `TabBar` the main window's
/// detail panel uses, with `wl-` prefixed ids.
fn dict_tabs(results: &[DictResult], active: usize, state: Entity<DictState>) -> gpui::AnyElement {
    let state_for_click = state;
    TabBar::new("wl-dict-tabs")
        .underline()
        .selected_index(active)
        .children(
            results
                .iter()
                .map(|r| Tab::new().label(SharedString::from(r.short_name.clone()))),
        )
        .on_click(move |idx: &usize, _window, cx| {
            let i = *idx;
            cx.update_entity(&state_for_click, |s, _cx| {
                if let Some(wl) = s.word_lookup_engine.as_mut() {
                    wl.set_active(i);
                }
            });
        })
        .into_any_element()
}

/// The parsed MDX entry, rendered by the same block renderer the main
/// window's detail panel uses (dict stylesheets already registered).
fn definition_body(blocks: &[crate::html::Block]) -> gpui::AnyElement {
    div()
        .w_full()
        .text_size(px(13.))
        .text_color(colors::text())
        .child(render_blocks(blocks))
        .into_any_element()
}

/// The "no dictionary has this word" note.
fn not_found_note() -> gpui::AnyElement {
    div()
        .w_full()
        .px(px(12.))
        .py(px(16.))
        .rounded(px(8.))
        .bg(colors::bg())
        .border_1()
        .border_color(colors::border())
        .text_size(px(12.))
        .text_color(colors::text_secondary())
        .child(SharedString::from(
            "No dictionary has this word. Translate it with AI instead?",
        ))
        .into_any_element()
}

/// Hand the lookup's word to the quick-translate engine (AI fallback): the
/// lookup popup hides, the same window re-renders as the translate popup in
/// its Loading state, and the background job feeds the result back. No-op
/// when the translate feature is disabled (no engine).
fn translate_word_from_lookup(state_entity: &Entity<DictState>, cx: &mut gpui::App) {
    let word = state_entity
        .read(cx)
        .word_lookup_engine
        .as_ref()
        .and_then(|wl| match wl.status() {
            crate::word_lookup::LookupStatus::Visible(state) => state.word().map(str::to_string),
            crate::word_lookup::LookupStatus::Hidden => None,
        });
    let Some(word) = word else {
        return;
    };
    cx.update_entity(state_entity, |s, cx| {
        if let Some(wl) = s.word_lookup_engine.as_mut() {
            wl.hide_popup();
        }
        if let Some(qt) = s.quick_translate_engine.as_mut()
            && let Some(job) = qt.restart_translation(word)
        {
            spawn_translation(job, state_entity.clone(), cx);
        }
        cx.notify();
    });
}
