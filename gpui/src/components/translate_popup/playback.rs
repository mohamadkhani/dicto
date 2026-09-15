//! Playback UI for the popup: the per-section status chip, the contextual
//! play button, and the seek bar. The source text and the translation each
//! get an independent `PlaybackController` on `DictState` so their states
//! never mix.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::{h_flex, v_flex};

use gpui::AppContext as _;

use crate::playback::PlaybackSnapshot;
use crate::{colors, state::DictState};

/// Which playback slot a play button drives. The source text and the
/// translation each get an independent `PlaybackController` on `DictState`
/// (`playback_source` / `playback_translation`) so their states never mix.
#[derive(Clone, Copy)]
pub(crate) enum Slot {
    Source,
    Translation,
}

impl Slot {
    pub(crate) fn controller(self, s: &DictState) -> &crate::playback::PlaybackController {
        match self {
            Slot::Source => &s.playback_source,
            Slot::Translation => &s.playback_translation,
        }
    }

    pub(crate) fn controller_mut(
        self,
        s: &mut DictState,
    ) -> &mut crate::playback::PlaybackController {
        match self {
            Slot::Source => &mut s.playback_source,
            Slot::Translation => &mut s.playback_translation,
        }
    }
}

/// The status chip next to a section label. Tells the user what the play
/// button will do before a clip exists: "AI TTS" (pressing play synthesizes
/// via the API), "synthesizing…" (in flight), "cached · replay" (clip loaded
/// for this exact text AND current TTS settings), "new text · play" (a clip
/// exists but for DIFFERENT text), "new voice · play" (clip exists for this
/// text but the TTS settings changed). `None` while playing/paused — the
/// seek bar shows live state then.
pub(crate) fn section_status(
    pb: &PlaybackSnapshot,
    shown_text: &str,
    tts: &mdict_rs::settings::TtsSettings,
) -> Option<gpui::AnyElement> {
    use crate::playback::PlaybackState;
    let (state, _total, clip_text, clip_tts) = pb;
    let stale = clip_stale(pb, shown_text, tts);
    let label = match state {
        PlaybackState::Idle => ("AI TTS", true),
        PlaybackState::Loading => ("synthesizing…", false),
        PlaybackState::Ended { .. } if stale => (
            if clip_text != shown_text {
                "new text · play"
            } else {
                "new voice · play"
            },
            true,
        ),
        PlaybackState::Ended { .. } => ("cached · replay", false),
        _ => return None,
    };
    let (label, accent) = label;
    let _ = clip_tts;
    let mut chip = div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(3.))
        .text_size(px(10.))
        .whitespace_nowrap();
    if accent {
        chip = chip
            .text_color(colors::primary())
            .border_1()
            .border_color(colors::border());
    } else {
        chip = chip
            .text_color(colors::text_secondary())
            .bg(colors::surface_alt());
    }
    Some(chip.child(SharedString::from(label)).into_any_element())
}

/// A loaded clip is STALE when it no longer matches what the play button
/// would synthesize now: different text (new selection / new translation) OR
/// different TTS settings (model / base URL / voice changed) — replaying it
/// would play the wrong audio.
pub(crate) fn clip_stale(
    pb: &PlaybackSnapshot,
    shown_text: &str,
    tts: &mdict_rs::settings::TtsSettings,
) -> bool {
    let (_state, _total, clip_text, clip_tts) = pb;
    if clip_text.is_empty() {
        return false;
    }
    clip_text != shown_text || clip_tts.as_deref() != Some(crate::playback::tts_key(tts).as_str())
}

/// The single contextual play button + (when a clip is loaded) a seek bar for
/// one section header. The button icon follows the playback state:
///
///   idle    → play    (will synthesize from the AI TTS API)
///   loading → loader  (synthesizing; disabled)
///   playing → pause
///   paused  → play    (resume)
///   ended   → undo-2  (replay from start — no re-synthesis)
///
/// Icons are Lucide SVGs from gpui-component's bundled assets (resolved via
/// the app's `AssetSource`), NOT text glyphs — the runtime UI font lacks the
/// geometric shapes (U+23F8 pause etc.), which rendered as invisible tofu.
///
/// Uses the shared `PlaybackController` on `DictState` so clips persist across
/// re-renders and can be paused/seeked/replayed without re-synthesizing.
/// `playback` is the controller's current snapshot (state + total duration),
/// polled by the view on a timer and passed in so this pure view-builder needs
/// no GPUI context to read live state.
pub(crate) fn playback_controls(
    slot: Slot,
    text: String,
    lang: String,
    tts: mdict_rs::settings::TtsSettings,
    state: Entity<DictState>,
    playback: PlaybackSnapshot,
) -> gpui::AnyElement {
    use crate::playback::PlaybackState;

    // Disambiguate the source vs. translation buttons (DOM id only; routing
    // is driven by the explicit `slot` param, NOT by lang emptiness — a
    // translation can legitimately have an empty target lang).
    let btn_id = match slot {
        Slot::Source => "qt-speak-src",
        Slot::Translation => "qt-speak-tr",
    };

    let pb_state = playback.0.clone();
    let total = playback.1;
    let lang_opt = if lang.is_empty() { None } else { Some(lang) };

    // STALE clip: a clip is loaded, but for different text or under different
    // TTS settings than what the popup shows now (new selection / new
    // translation / TTS options changed). The old clip's pause/replay
    // semantics would play the wrong audio, so the button must present — and
    // act — as "synthesize new".
    let stale = clip_stale(&playback, &text, &tts);

    let (icon, disabled) = if stale {
        // Fresh synthesis for the new text, even though a clip is loaded.
        ("icons/play.svg", false)
    } else {
        match pb_state {
            PlaybackState::Idle | PlaybackState::Error(_) => ("icons/play.svg", false),
            PlaybackState::Loading => ("icons/loader.svg", true),
            PlaybackState::Playing { .. } => ("icons/pause.svg", false),
            PlaybackState::Paused { .. } => ("icons/play.svg", false),
            PlaybackState::Ended { .. } => ("icons/undo-2.svg", false),
        }
    };

    let mut btn = v_flex()
        .id(btn_id)
        // Explicit square: the button sits in a `justify_between` header row
        // where auto-sizing measured the icon's width but collapsed the
        // height, so keep both axes pinned.
        .size(px(26.))
        .flex_1()
        .items_center()
        .rounded(px(6.))
        .bg(colors::hover())
        .border_1()
        .border_color(colors::border())
        .hover(|s| s.bg(colors::border()))
        .cursor_pointer()
        .child(
            div()
                .flex()
                .h_full()
                .flex_row()
                .items_center()
                .content_center()
                .justify_center()
                .child(
                    gpui::svg()
                        .path(icon)
                        .self_center()
                        .size(px(16.))
                        .text_color(colors::text()),
                ),
        );
    if disabled {
        btn = btn.opacity(0.45);
    }
    let btn = btn.on_click({
        let state = state.clone();
        let text = text.clone();
        let lang = lang_opt.clone();
        let tts = tts.clone();
        move |_ev, _window, cx| {
            // One button, contextual action: the controller's current state
            // decides whether this synthesizes, pauses, resumes, or replays.
            // A STALE clip (loaded for different text or different TTS
            // settings) always synthesizes — pausing/replaying it would play
            // the wrong audio.
            let snapshot = slot.controller(&state.read(cx)).snapshot();
            let stale = clip_stale(&snapshot, &text, &tts);
            match snapshot.0 {
                PlaybackState::Loading => {}
                _ if stale => spawn_speak(
                    slot,
                    state.clone(),
                    text.clone(),
                    lang.clone(),
                    tts.clone(),
                    cx,
                ),
                PlaybackState::Idle | PlaybackState::Error(_) => spawn_speak(
                    slot,
                    state.clone(),
                    text.clone(),
                    lang.clone(),
                    tts.clone(),
                    cx,
                ),
                PlaybackState::Playing { .. }
                | PlaybackState::Paused { .. }
                | PlaybackState::Ended { .. } => {
                    let _ = cx.update_entity(&state, |s, _cx| {
                        slot.controller_mut(s).toggle_pause();
                    });
                }
            }
        }
    });

    // When a clip is loaded (Playing/Paused/Ended) and it belongs to THIS
    // text, append the seek bar. A stale clip keeps its bar hidden — the
    // button now means "synthesize new", and the old bar would mislead.
    // Ended keeps the seek bar full; the button replays from start.
    if stale {
        return h_flex().ml_auto().child(btn).into_any_element();
    }
    match pb_state {
        PlaybackState::Playing { pos }
        | PlaybackState::Paused { pos }
        | PlaybackState::Ended { pos } => h_flex()
            .gap(px(6.))
            .items_center()
            .ml_auto()
            .child(btn)
            .child(seek_bar(slot, state, pos, total))
            .into_any_element(),
        _ => h_flex().ml_auto().child(btn).into_any_element(),
    }
}

/// Spawn the synthesis + install pipeline on the background executor.
fn spawn_speak(
    slot: Slot,
    state: Entity<DictState>,
    text: String,
    lang: Option<String>,
    tts: mdict_rs::settings::TtsSettings,
    cx: &mut gpui::App,
) {
    // Read the controller's decision via the entity (slot picks source/translation).
    let action =
        slot.controller(&state.read(cx))
            .start_load(text.clone(), lang.clone(), Some(tts.clone()));
    if let crate::playback::LoadAction::Synthesize { text, lang, tts } = action {
        // `text` and `tts` are each needed twice: once for synthesis (moved
        // into the bg task) and once to tag the installed clip (so replays
        // detect text/settings changes). Clone the latter before the move.
        let text_for_install = text.clone();
        let tts_for_install = tts.clone();
        cx.spawn(async move |cx| {
            let result =
                cx.background_executor()
                    .spawn(async move {
                        crate::tts::synthesize_bytes(&text, lang.as_deref(), tts.as_ref())
                    })
                    .await;
            match result {
                Ok(bytes) => {
                    let _ = cx.update_entity(&state, |s, _cx| {
                        slot.controller_mut(s).install_from_bytes(
                            text_for_install,
                            tts_for_install,
                            bytes,
                        );
                    });
                }
                Err(e) => {
                    let _ = cx.update_entity(&state, |s, _cx| {
                        slot.controller_mut(s).fail(e.to_string());
                    });
                }
            }
        })
        .detach();
    }
}

/// Clickable seek bar showing playback progress. Click position sets the seek
/// fraction. Rendered with a fixed track width + a filled portion.
///
/// `ClickEvent::position()` is **window-relative**, not element-relative, so we
/// capture the seek bar's own bounds (via a zero-size `canvas` that records them
/// during paint) and subtract its left edge in the click handler.
fn seek_bar(
    slot: Slot,
    state: Entity<DictState>,
    pos: f32,
    total: Option<std::time::Duration>,
) -> gpui::AnyElement {
    // Fraction of the clip played. If we don't know total duration, show an
    // indeterminate-ish 0% track that's still clickable (seek is a no-op then).
    let frac = total
        .filter(|t| t.as_secs_f32() > 0.0)
        .map(|t| (pos / t.as_secs_f32()).clamp(0.0, 1.0))
        .unwrap_or(0.0);

    let track_w = px(120.);
    let fill_w = track_w * frac;

    // Shared cell holding the seek bar's last-painted window-relative bounds.
    // Updated every paint; read in the click handler to localize the click.
    let bounds_cell: Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>> = Rc::new(Cell::new(None));
    let bounds_for_canvas = bounds_cell.clone();
    let bounds_for_click = bounds_cell.clone();

    let id = match slot {
        Slot::Source => "qt-seek-bar-src",
        Slot::Translation => "qt-seek-bar-tr",
    };
    div()
        .id(id)
        .w(track_w)
        .h(px(6.))
        .rounded(px(3.))
        .bg(colors::surface_alt())
        .border_1()
        .border_color(colors::border())
        .relative()
        .child(
            div()
                .w(fill_w)
                .h_full()
                .rounded(px(3.))
                .bg(colors::primary())
                .absolute()
                .left_0()
                .top_0(),
        )
        // An invisible full-size canvas overlay whose only job is to record
        // this element's bounds during paint, so the click handler can map
        // window-relative coords → a fraction along the track.
        .child(div().absolute().size_full().child(gpui::canvas(
            move |_bounds, _window, _cx| {},
            move |bounds, _t, _window, _cx| {
                bounds_for_canvas.set(Some(bounds));
            },
        )))
        .cursor_pointer()
        .on_click(move |ev: &gpui::ClickEvent, _window, cx| {
            let Some(total) = total else { return };
            if total.as_secs_f32() <= 0.0 {
                return;
            }
            // Localize the window-relative click to the seek bar's own bounds.
            let Some(b) = bounds_for_click.get() else {
                return;
            };
            let rel = (ev.position().x - b.left()).max(px(0.));
            let fraction = (rel / b.size.width).clamp(0.0, 1.0);
            let _ = cx.update_entity(&state, |s, _cx| {
                slot.controller_mut(s).seek(fraction);
            });
        })
        .into_any_element()
}
