//! Quick Translate popup window.
//!
//! A small floating window that appears when the quick-translate hotkey is
//! pressed. Shows the selected text and its translation.
//!
//! Structured like a React page: `mod.rs` is the page (layout + the window
//! view), `sections.rs` holds the text sections and the Translate button,
//! `options.rs` the inline Options panel, and `playback.rs` the play button
//! + seek bar.

pub(crate) mod options;
pub(crate) mod playback;
pub(crate) mod sections;

use std::cell::Cell;
use std::rc::Rc;

use crate::{
    colors,
    components::{banner, spinner, text_editor::EditorEvent, text_editor::EditorState},
    state::DictState,
};
use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, px,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};

pub(crate) use self::playback::Slot;

/// Window height bounds for the popup. The window is sized to its content
/// (see [`MeasureProbes`]) but never below/above these.
pub(crate) const MIN_POPUP_H: f32 = 240.;
pub(crate) const MAX_POPUP_H: f32 = 780.;

/// Fixed height of the window's title strip (title + close button),
/// attached to the window's top border like a normal popup window.
const TITLE_BAR_H: f32 = 32.;

/// Layout probes for dynamic window sizing. Zero-size canvases record, at
/// paint time, the window-space Y of four points: the top of the body
/// content, its natural (unscrolled) end, and the footer's top and end. The
/// view's poll tick turns those into the desired window height — so opening
/// the Options panel or the translation section GROWS the window instead of
/// compressing the body, and only content beyond `MAX_POPUP_H` scrolls.
#[derive(Clone, Default)]
pub struct MeasureProbes {
    pub body_top: Rc<Cell<Option<f32>>>,
    pub body_end: Rc<Cell<Option<f32>>>,
    pub footer_top: Rc<Cell<Option<f32>>>,
    pub footer_end: Rc<Cell<Option<f32>>>,
    /// The toggle row's own height (probes hug the toggle, excluding the
    /// panel) — constant across frames, which makes the toggle prediction
    /// stateless and safe under rapid clicking.
    pub toggle_top: Rc<Cell<Option<f32>>>,
    pub toggle_end: Rc<Cell<Option<f32>>>,
    pub panel_top: Rc<Cell<Option<f32>>>,
    pub panel_end: Rc<Cell<Option<f32>>>,
}

impl MeasureProbes {
    /// The Options panel's painted height, from its cached probes. Persists
    /// while the panel is closed (cells keep the last painted values), so a
    /// toggle can predict the height change before the next paint.
    pub fn panel_height(&self) -> Option<f32> {
        Some(self.panel_end.get()? - self.panel_top.get()?)
    }

    /// The toggle row's height (probes hug the toggle, excluding the panel).
    pub fn toggle_height(&self) -> Option<f32> {
        Some(self.toggle_end.get()? - self.toggle_top.get()?)
    }

    /// Desired window height from the last painted probe values: the body's
    /// natural height plus the footer's height, plus the card chrome (top
    /// border + top padding + bottom border; the footer's -14 bottom margin
    /// cancels the card's bottom padding). `None` until the first paint.
    /// Both probe pairs are offset/scroll-independent deltas, so this stays
    /// correct even while the body is scrolled at the height cap.
    ///
    /// Rounded UP with 1px of slack: the compositor quantizes window sizes
    /// (fractional scale factors), and a viewport even 1px short of the
    /// content makes the BODY scrollable — a spurious outer scrollbar while
    /// only the text areas should scroll.
    pub fn desired_window_height(&self) -> Option<f32> {
        let body_top = self.body_top.get()?;
        let body_end = self.body_end.get()?;
        let footer_top = self.footer_top.get()?;
        let footer_end = self.footer_end.get()?;
        let body_natural = body_end - body_top;
        let footer_height = footer_end - footer_top;
        Some(fitted_height(body_natural, footer_height))
    }
}

/// Window height that fits the body's natural height plus the footer, with
/// the title strip, chrome, and rounding slack (see
/// [`MeasureProbes::desired_window_height`]). The title strip is constant,
/// so it folds in here rather than being probed. The constant covers: the
/// title strip, the body wrapper's 12px top + 12px bottom padding, the
/// footer's 1px top border, the 2px window borders, and 1px of compositor
/// rounding slack.
pub(crate) fn fitted_height(body_natural: f32, footer_height: f32) -> f32 {
    (body_natural + footer_height + TITLE_BAR_H + 27.)
        .ceil()
        .clamp(MIN_POPUP_H, MAX_POPUP_H)
}

/// A zero-height probe that records its own window-space Y during paint.
fn probe(y: Rc<Cell<Option<f32>>>) -> gpui::AnyElement {
    div()
        .w_full()
        .h(px(0.))
        .child(gpui::canvas(
            move |_bounds, _window, _cx| {},
            move |bounds, _t, _window, _cx| y.set(Some(f32::from(bounds.origin.y))),
        ))
        .into_any_element()
}

/// Estimate the popup window's natural height from the popup state, BEFORE
/// the first paint. The text sections have fixed heights, so this lands
/// within a few px of the probed value — creating the window at (roughly)
/// its natural size avoids the visible bottom-edge jump a default-size
/// window would produce when the poll tick first resizes it (window resizes
/// anchor the top edge, so a shrink visibly moves the whole popup upward).
/// The tick still corrects the last few px from the real measurements.
pub(crate) fn estimated_window_height(state: Option<&PopupState>) -> f32 {
    use PopupState as PS;
    const CHROME: f32 = 26. + TITLE_BAR_H; // 2 window borders + the title
    // strip + the body wrapper's 12px top and bottom padding (the footer is
    // full-bleed, so nothing cancels the bottom padding anymore)
    const HEADER: f32 = 26.; // section label row (label + 26px play button)
    const ORIGINAL: f32 = 120.; // fixed original text area
    const TRANSLATION: f32 = 200.; // fixed translation text area
    const DIVIDER: f32 = 1.;
    const GAP: f32 = 8.;
    const ROW: f32 = 34.; // Translate button (~30px) + 2px vertical paddings
    const SPINNER: f32 = 30.;
    const BANNER: f32 = 46.; // 2-line error/warning banner
    const FOOTER: f32 = 33.; // toggle row + its top border

    let original_section = HEADER + GAP + ORIGINAL + GAP + DIVIDER;
    let (body, row) = match state {
        // original + "Translation" header + translation text
        Some(PS::Ready { .. }) => (
            original_section + GAP + HEADER + GAP + TRANSLATION,
            Some(ROW),
        ),
        Some(PS::Loading { .. }) => (original_section + GAP + SPINNER, Some(ROW)),
        Some(PS::Error { .. }) => (original_section + GAP + BANNER, Some(ROW)),
        // Too long: warning banner, and NO Translate row (nothing to do).
        Some(PS::TooLong { .. }) => (original_section + GAP + BANNER, None),
        Some(PS::Idle { .. }) | None => (original_section, Some(ROW)),
    };
    let row = row.map(|r| GAP + r).unwrap_or(0.);
    (CHROME + body + row + FOOTER).clamp(MIN_POPUP_H, MAX_POPUP_H)
}

/// State of the translation popup.
#[derive(Debug, Clone)]
pub enum PopupState {
    /// Selection captured; waiting for the user to click Translate.
    Idle { original: String },
    /// Loading the translation.
    Loading { original: String },
    /// Translation succeeded.
    Ready {
        original: String,
        translation: String,
        provider: String,
        model: String,
    },
    /// Translation failed.
    Error { original: String, error: String },
    /// Selection exceeds the 10,000-char limit. A warning, not an error —
    /// nothing failed; the user just needs a smaller selection.
    TooLong { original: String },
}

impl PopupState {
    /// The original (source) text — present in every variant.
    pub fn original(&self) -> &str {
        match self {
            PopupState::Idle { original }
            | PopupState::Loading { original }
            | PopupState::Ready { original, .. }
            | PopupState::Error { original, .. }
            | PopupState::TooLong { original } => original,
        }
    }
}

/// Handler that toggles the Options disclosure.
pub type OnToggleOptions = Box<dyn Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static>;

/// Handler that closes the popup (title-strip close button).
pub type OnClose = Box<dyn Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static>;

/// Props for [`translate_popup`] — everything the popup content needs to
/// render one frame, bundled like a React component's props object.
///
/// `settings` carries the quick-translate config for the footer summary and
/// TTS buttons. `options_panel` is the stateful Options panel entity rendered
/// inside the drawer. `on_toggle_options` toggles the drawer.
pub struct PopupProps {
    pub state: PopupState,
    pub settings: mdict_rs::settings::QuickTranslateSettings,
    /// Inline Options panel expanded?
    pub options_open: bool,
    /// The stateful Options panel (adaptive pickers) expanded below the toggle.
    pub options_panel: Entity<options::OptionsPanel>,
    /// The Original text editor (our RTL-aware [`crate::components::text_editor`]),
    /// always shown and always editable.
    pub original_editor: Entity<EditorState>,
    /// The Translation text editor, read-only: selectable and copyable, but
    /// the text cannot change.
    pub translation_editor: Entity<EditorState>,
    pub playback_source: sections::PlaybackSnapshot,
    pub playback_translation: sections::PlaybackSnapshot,
    /// Records the layout's natural heights so the view can resize the
    /// window to its content (see [`MeasureProbes`]).
    pub measure: MeasureProbes,
    /// False while a requested window resize is still in flight (the
    /// compositor hasn't applied it yet) — the Options panel is held back
    /// until then so it never paints at the old size.
    pub options_settled: bool,
    pub on_toggle_options: OnToggleOptions,
    /// Fires on the title-strip close button — same path as Escape.
    pub on_close: OnClose,
}

/// The title-strip close button: square ghost with the Lucide X (close.svg),
/// styled after the section-header icon buttons so the window reads as one
/// control family. Fires `on_close` — the same dismissal path as Escape.
fn close_button(on_close: OnClose) -> gpui::Stateful<gpui::Div> {
    v_flex()
        .id("qt-close")
        .size(px(24.))
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .hover(|s| s.bg(colors::hover()))
        .cursor_pointer()
        .child(
            gpui::svg()
                .path("icons/close.svg")
                .self_center()
                .size(px(14.))
                .text_color(colors::text_secondary()),
        )
        .on_click(move |ev, window, cx| on_close(ev, window, cx))
}

/// Build the popup view content.
///
/// `state_entity` is the shared `DictState` so selectors/Translate can persist
/// and kick off translations.
pub fn translate_popup(state_entity: &Entity<DictState>, props: PopupProps) -> gpui::AnyElement {
    let PopupProps {
        state,
        settings,
        options_open,
        options_settled,
        options_panel,
        original_editor,
        translation_editor,
        playback_source,
        playback_translation,
        measure,
        on_toggle_options,
        on_close,
    } = props;
    let state = &state;
    let settings = &settings;
    let target_lang = settings.target_lang.clone();
    let tts = settings.tts.clone();
    let card = v_flex()
        .id("qt-popup-card")
        .w(px(460.))
        // The window frame MUST have a definite height: it is the ancestor
        // that makes every descendant percentage/flex height resolvable. With
        // an auto-height frame the body's `h_full` scroll chain resolves
        // against an indefinite parent and `Scrollable` forces flex-basis 0
        // on its inner element — the content then reserves no height and the
        // frame collapses to its padding. Definiteness comes from the window,
        // and the WINDOW is resized to the content by the view's poll tick
        // using `MeasureProbes` — so the frame is content-sized in effect.
        .h_full()
        .bg(colors::surface())
        .rounded(px(10.))
        .border_1()
        .border_color(colors::border())
        // FREE-AREA DRAG: a mouse-down that reaches the frame (title strip,
        // padding, gaps, section labels — every interactive subtree stops
        // propagation first) moves the window, and also keeps the backdrop
        // from dismissing.
        .on_mouse_down(gpui::MouseButton::Left, |_, window, cx| {
            window.start_window_move();
            cx.stop_propagation();
        })
        // Title strip attached to the window's top border, like a normal
        // popup window: title left, close right, divided from the body by
        // a border. Fixed while the body scrolls. Constant height, so it is
        // folded into the chrome constant (TITLE_BAR_H), not a probe.
        .child(
            h_flex()
                .id("qt-popup-titlebar")
                .h(px(32.))
                .items_center()
                .justify_between()
                .px(px(8.))
                .border_b_1()
                .border_color(colors::border())
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors::text_secondary())
                        .child(SharedString::from("Quick Translate")),
                )
                // No-drag wrapper: the close button is interactive, so a
                // press on it must complete as a click, not a window move.
                .child(
                    div()
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation()
                        })
                        .child(close_button(on_close)),
                ),
        );

    // The Translate button is always present (disabled while loading) so the
    // layout never jumps between states and a retry is always one click away.
    // It sits at the end of the scrollable body, like the design. Hidden in
    // the TooLong state — there's nothing to translate.
    let busy = matches!(state, PopupState::Loading { .. });
    let original_for_btn = match state {
        PopupState::TooLong { .. } => None,
        PopupState::Idle { original }
        | PopupState::Loading { original }
        | PopupState::Ready { original, .. }
        | PopupState::Error { original, .. } => Some(original.clone()),
    };
    let translate_row_fixed = original_for_btn.map(|original| {
        // No-drag wrapper: the Translate button is interactive, so presses on
        // it must complete as clicks, not start a window move. 2px vertical
        // padding mirrors the design's my-0.5 on the row.
        h_flex()
            .justify_center()
            .pt(px(2.))
            .pb(px(2.))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(sections::translate_button(
                original,
                state_entity.clone(),
                busy,
            ))
    });

    // The Original block is ALWAYS the RTL-aware editor — no edit toggle,
    // no separate display mode. The view keeps the editor's text in sync
    // with the engine (see TranslatePopupView::render).
    let original_block = sections::original_text_editor(&original_editor);
    let content = match state {
        PopupState::Idle { original } => v_flex().gap(px(8.)).child(sections::original_section(
            original,
            original_block,
            &tts,
            state_entity.clone(),
            playback_source.clone(),
        )),

        PopupState::Loading { original } => v_flex()
            .gap(px(8.))
            .child(sections::original_section(
                original,
                original_block,
                &tts,
                state_entity.clone(),
                playback_source.clone(),
            ))
            .child(spinner::spinner_row("Translating…")),

        PopupState::Ready {
            original,
            translation,
            provider: _,
            model: _,
        } => {
            // No "via provider · model" meta line — the Options footer summary
            // already shows the active provider.
            v_flex()
                .gap(px(8.))
                .child(sections::original_section(
                    original,
                    original_block,
                    &tts,
                    state_entity.clone(),
                    playback_source.clone(),
                ))
                .child(sections::section_header(
                    "Translation",
                    Slot::Translation,
                    translation,
                    &target_lang,
                    &tts,
                    state_entity.clone(),
                    playback_translation.clone(),
                ))
                .child(sections::translation_text_select(&translation_editor))
        }

        PopupState::Error { original, error } => v_flex()
            .gap(px(8.))
            .child(sections::original_section(
                original,
                original_block,
                &tts,
                state_entity.clone(),
                playback_source.clone(),
            ))
            .child(banner::error_text("Translation failed", error)),

        PopupState::TooLong { original } => {
            // Selection exceeds the 10,000-char limit. A warning, not an error:
            // nothing failed — the user just needs a smaller selection.
            let len = original.chars().count();
            v_flex()
                .gap(px(8.))
                .child(sections::original_section(
                    original,
                    original_block,
                    &tts,
                    state_entity.clone(),
                    playback_source.clone(),
                ))
                .child(banner::warning_banner(
                    "Selection is too long",
                    &format!(
                        "{len} characters — the limit is 10,000. \
                         Select a shorter passage and press the hotkey again."
                    ),
                ))
        }
    };

    // The footer region owns the window frame's bottom edge: full-bleed top
    // border, bottom rounding, toggle above the panel it reveals. The frame
    // has no padding of its own (the body carries it), so no negative
    // margins are needed.
    let provider_summary = format!(
        "{} · {}",
        crate::quick_translate::provider_display_name(settings.llm_provider),
        settings.hotkey
    );
    let mut footer = v_flex()
        .flex_shrink_0()
        .border_t_1()
        .border_color(colors::border())
        .overflow_hidden()
        .rounded_b(px(9.))
        .child(probe(measure.footer_top.clone()))
        .child(probe(measure.toggle_top.clone()))
        // No-drag wrapper: the toggle row is clickable, not a drag handle.
        .child(
            div()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(options::options_toggle(
                    options_open,
                    &provider_summary,
                    on_toggle_options,
                )),
        )
        .child(probe(measure.toggle_end.clone()));
    // Disclosure expands BELOW the toggle, like the design: the panel stacks
    // in flow inside the footer, clipped by its overflow-hidden and the
    // card's rounded bottom corners. While a window resize is still in
    // flight (options_settled false), the panel is held back: painting it at
    // the old window size would flash it over the compressed translation
    // text before the resize lands.
    if options_open && options_settled {
        footer = footer.child(
            div()
                .mx(px(14.))
                .mb(px(12.))
                // No-drag wrapper: the panel's pickers are interactive.
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(probe(measure.panel_top.clone()))
                .child(options_panel)
                // The panel probes cache the panel's painted height, so the
                // toggle can predict the window-height change before paint.
                .child(probe(measure.panel_end.clone())),
        );
    }
    // Records the footer's end Y: footer height feeds the window-height
    // computation (see MeasureProbes).
    footer = footer.child(probe(measure.footer_end.clone()));

    // Layout mirrors the design: a flexing, scrollable body (text sections +
    // Translate row) above the footer. The view resizes the WINDOW to the
    // natural content height (MeasureProbes), so opening Options or showing
    // the translation GROWS the window rather than compressing the body;
    // only content taller than MAX_POPUP_H makes the body scroll.
    //
    // Two-layer pattern, both layers required: the outer div does the FLEX
    // SIZING (flex_1 + min_h 0 — takes the space the footer leaves inside
    // the definite-height card), the inner does the SCROLLING (h_full +
    // overflow_y_scrollbar — `overflow_y_scrollbar` re-wraps its element in
    // a `size_full` div, so it needs a definite height from the outer layer;
    // sizing and scrolling on one div overflows instead of scrolling).
    // The two probes inside the scroll content record its NATURAL height:
    // both move with the scroll offset, so their delta is offset-independent.
    // The probes are gap-free siblings of the content wrapper — the 8px gap
    // lives INSIDE the wrapper, between content and the Translate row — so
    // the probe delta is exactly the content height, with no phantom gaps.
    let mut inner = v_flex().gap(px(8.)).child(content);
    if let Some(row) = translate_row_fixed {
        inner = inner.child(row);
    }
    let body_scroll = v_flex()
        .id("qt-popup-body")
        .h_full()
        .overflow_y_scrollbar()
        .child(probe(measure.body_top.clone()))
        .child(inner)
        .child(probe(measure.body_end.clone()));
    // The body carries the frame's horizontal padding now that the title
    // strip (full-bleed) and the footer (full-bleed) own the top and bottom
    // edges. 12px bottom padding mirrors the design's py-3.
    let body = v_flex()
        .flex_1()
        .min_h(px(0.))
        .px(px(14.))
        .pt(px(12.))
        .pb(px(12.))
        .child(body_scroll);

    card.child(body).child(footer).into_any_element()
}

/// View backing the Quick Translate popup window.
///
/// Holds the shared `DictState` (so it sees the engine's current `PopupStatus`),
/// focuses itself on mount (so it receives key events), and dismisses the popup
/// on Escape or a click outside the popup card.
pub struct TranslatePopupView {
    state: Entity<DictState>,
    focus: FocusHandle,
    /// Inline Options panel expanded?
    options_open: bool,
    /// The stateful Options panel (adaptive pickers), expanded below the
    /// footer toggle. Lives as long as the window so pickers keep their state.
    options_panel: Entity<options::OptionsPanel>,
    /// Painted-layout probes the card fills in; the poll tick reads them to
    /// resize the window to the content's natural height.
    measure: MeasureProbes,
    /// Window height the tick last applied, so we resize only on change.
    applied_height: Option<f32>,
    /// Ticks elapsed since the last (unconfirmed) resize request.
    unsettled_ticks: u8,

    /// EDITABLE ORIGINAL editor — always rendered in the Original section.
    /// Kept in sync with the engine's original (see `render`).
    original_editor: Entity<EditorState>,
    /// READ-ONLY translation editor — selectable/copyable, never editable.
    /// Re-seeded whenever the translation text changes.
    translation_editor: Entity<EditorState>,
    /// The original text the editor last pushed into the engine (or was
    /// seeded from). If the engine's original diverges from this, it changed
    /// externally (a new selection) and the editor is re-seeded.
    last_pushed_original: Option<String>,
    /// The translation text last shown in the read-only editor, so a new
    /// translation re-seeds it exactly once.
    last_shown_translation: Option<String>,
}

impl TranslatePopupView {
    /// Whether the last requested window resize has been applied by the
    /// platform, checked against the LIVE window size (`viewport_size`) —
    /// not the painted probes, which lag one frame behind and would keep the
    /// panel hidden an extra frame after the resize lands. While unset, the
    /// Options panel is held back — painting it at the old window size would
    /// flash it over the translation text before the resize lands (resizes
    /// are async on Wayland/X11).
    fn options_settled(&self, window: &Window) -> bool {
        let Some(applied) = self.applied_height else {
            return true;
        };
        (f32::from(window.viewport_size().height) - applied).abs() <= 1.5
    }

    pub fn new(state: Entity<DictState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Poll the playback controller ~10 Hz while the popup is open so the
        // seek bar tracks live position and Playing→Idle (clip finished) is
        // observed. Each tick updates the controller's state, reconciles the
        // Options dropdowns with the live settings (see OptionsPanel::sync),
        // resizes the window to the measured content height (MeasureProbes),
        // and notifies the view so render() re-reads the snapshots. The loop
        // stops once the window is gone (update returns Err).
        let poll_state = state.clone();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                let Ok(()) = cx.update(|window, cx| {
                    // Update each controller's live position…
                    cx.update_entity(&poll_state, |s, _cx| {
                        s.playback_source.poll_progress();
                        s.playback_translation.poll_progress();
                    });
                    // …then sync the Options dropdowns, size the window to
                    // the content, and re-render.
                    let _ = this.update(cx, |view, cx| {
                        view.options_panel
                            .update(cx, |panel, cx| panel.sync(window, cx));
                        // Resize only once the platform has applied the last
                        // requested size — resizes are async, and stacking a
                        // new one mid-flight fights the compositor. Live
                        // viewport size, not the (one-frame-stale) probes.
                        let painted = f32::from(window.viewport_size().height);
                        let settled = match view.applied_height {
                            Some(applied) => (applied - painted).abs() <= 1.5,
                            None => true,
                        };
                        if settled {
                            view.unsettled_ticks = 0;
                            if let Some(desired) = view.measure.desired_window_height() {
                                if view
                                    .applied_height
                                    .is_none_or(|applied| (applied - desired).abs() > 0.5)
                                {
                                    window.resize(gpui::size(px(460.), px(desired)));
                                    view.applied_height = Some(desired);
                                    view.unsettled_ticks = 1;
                                }
                            }
                        } else {
                            // Give the compositor a few ticks to apply the
                            // resize; if it never lands (or the user resized
                            // the window manually), accept the painted size
                            // as the new baseline so the panel un-gates.
                            view.unsettled_ticks += 1;
                            if view.unsettled_ticks > 4 {
                                view.applied_height = Some(painted);
                                view.unsettled_ticks = 0;
                            }
                        }
                        cx.notify();
                    });
                }) else {
                    break;
                };
            }
        })
        .detach();

        let options_panel = cx.new(|cx| options::OptionsPanel::new(state.clone(), window, cx));
        let original_editor = cx.new(|cx| EditorState::new(window, cx));
        let translation_editor = cx.new(|cx| EditorState::new_read_only(window, cx));

        // Push edits from the Original editor into the engine as they happen,
        // so the Translate button / TTS / the header always see the current
        // text. set_original hides a stale translation (popup → Idle).
        let state_for_editor = state.clone();
        cx.subscribe(&original_editor, move |view, _, event: &EditorEvent, cx| {
            let EditorEvent::Change(value) = event;
            if view.last_pushed_original.as_deref() == Some(value.as_str()) {
                return;
            }
            view.last_pushed_original = Some(value.clone());
            state_for_editor.update(cx, |s, cx| {
                if let Some(engine) = s.quick_translate_engine.as_mut() {
                    engine.set_original(value.clone());
                }
                cx.notify();
            });
        })
        .detach();

        Self {
            state,
            focus: cx.focus_handle(),
            options_open: false,
            options_panel,
            measure: MeasureProbes::default(),
            applied_height: None,
            unsettled_ticks: 0,
            original_editor,
            translation_editor,
            last_pushed_original: None,
            last_shown_translation: None,
        }
    }

    fn toggle_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.options_open = !self.options_open;
        // A toggle (re)starts a resize; the panel stays hidden until the
        // painted size matches (options_settled).
        self.unsettled_ticks = 0;

        // Resize BEFORE the re-render so the panel's first frame already
        // paints at the right height — otherwise that frame renders at the
        // old height and the window "flashes" until the poll tick corrects
        // it.
        //
        // The prediction is STATELESS, built from cached constants — the
        // body's natural height, the toggle row's height, and the panel's
        // cached height. It deliberately does NOT read the footer's current
        // measured height: that reflects whatever the last frame rendered
        // (the panel is gated out while a resize is in flight), so deriving
        // from it made rapid toggling compound stale measurements and resize
        // to nonsense (e.g. clamping to the minimum). First-ever open has no
        // cached panel height yet — the tick handles that one.
        let body_natural = self
            .measure
            .body_end
            .get()
            .zip(self.measure.body_top.get())
            .map(|(end, top)| end - top);
        let toggle_height = self.measure.toggle_height();
        let panel_height = self.measure.panel_height();
        if let (Some(body_natural), Some(toggle_height), Some(panel)) =
            (body_natural, toggle_height, panel_height)
        {
            let footer = toggle_height + if self.options_open { panel + 12. } else { 0. };
            let desired = fitted_height(body_natural, footer);
            window.resize(gpui::size(px(460.), px(desired)));
            self.applied_height = Some(desired);
        }

        cx.notify();
    }
}

impl Focusable for TranslatePopupView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TranslatePopupView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.focus.clone();

        // Read popup state + full quick-translate settings + both playback
        // snapshots (source + translation are independent controllers) out of
        // DictState (cloned so the borrow ends before we pass cx).
        let (status, settings, options_open, pb_src, pb_tr) = {
            let st = self.state.read(cx);
            let engine = st.quick_translate_engine.as_ref();
            (
                engine.map(|e| e.popup_status().clone()),
                engine.map(|e| e.settings().clone()),
                self.options_open,
                st.playback_source.snapshot(),
                st.playback_translation.snapshot(),
            )
        };
        // Keep the always-editable Original editor in sync with the engine:
        // seed it when the popup opens and re-seed (with focus) when a new
        // hotkey selection replaces the text. Typing pushes Change →
        // engine.set_original, so the engine's original equals
        // last_pushed_original while the user edits — no re-seed loop.
        // The read-only translation editor re-seeds whenever the Ready
        // state carries a different translation.
        if let Some(crate::quick_translate::PopupStatus::Visible(ps)) = status.as_ref() {
            let original = ps.original().to_string();
            if self.last_pushed_original.as_deref() != Some(original.as_str()) {
                self.last_pushed_original = Some(original.clone());
                self.original_editor.update(cx, |s, cx| {
                    s.set_text(&original, window, cx);
                    s.focus(window, cx);
                });
            }
            if let PopupState::Ready { translation, .. } = ps
                && self.last_shown_translation.as_deref() != Some(translation.as_str())
            {
                self.last_shown_translation = Some(translation.clone());
                self.translation_editor.update(cx, |s, cx| {
                    s.set_text(translation, window, cx);
                });
            }
        }
        let card = match (status, settings) {
            (Some(crate::quick_translate::PopupStatus::Visible(ps)), Some(settings)) => {
                // Build props first: `options_settled` reborrows `window`
                // mutably, so the shared `window`/`cx` borrows passed to
                // `translate_popup` must not coexist with its evaluation.
                let props = PopupProps {
                    state: ps.clone(),
                    settings: settings.clone(),
                    options_open,
                    options_panel: self.options_panel.clone(),
                    original_editor: self.original_editor.clone(),
                    translation_editor: self.translation_editor.clone(),
                    playback_source: pb_src,
                    playback_translation: pb_tr,
                    measure: self.measure.clone(),
                    options_settled: self.options_settled(window),
                    on_toggle_options: Box::new(
                        cx.listener(|this, _ev, window, cx| this.toggle_options(window, cx)),
                    ),
                    on_close: Box::new(cx.listener(|this, _ev, window, cx| {
                        close_popup(&this.state, window, cx)
                    })),
                };
                translate_popup(&self.state, props)
            }
            // No content: render an empty (invisible) root; the window will
            // be closed by the trigger logic.
            _ => div().into_any_element(),
        };

        let mut root = div()
            .track_focus(&focus)
            .size_full()
            .on_key_down(
                cx.listener(move |this, ev: &gpui::KeyDownEvent, window, cx| {
                    if ev.keystroke.key == "escape" {
                        close_popup(&this.state, window, cx);
                    }
                }),
            )
            .on_mouse_down(gpui::MouseButton::Left, {
                let state = self.state.clone();
                move |_ev, window, cx| {
                    close_popup(&state, window, cx);
                }
            });

        // Pre-measure the Options panel: until its probes have painted once,
        // render the panel invisibly — clipped to zero height, fully
        // transparent — BEFORE the h_full card so it stays inside the
        // viewport (a sibling after the card would sit below the window and
        // never paint). `panel_height()` is cached BEFORE the first toggle,
        // so the toggle can predict the window resize and the first open
        // paints flash-free. The layer drops out by itself once the cache is
        // populated.
        //
        // The wrapper mirrors the footer's panel wrapper EXACTLY (same
        // horizontal margins) — the panel's chips wrap differently at
        // different widths, so a mismatched width would cache the wrong
        // height and the prediction would miss.
        // Pre-measure the Options panel: until its probes have painted once,
        // render the panel invisibly — fully transparent (opacity only, NOT
        // clipped: a zero-height overflow-hidden subtree can be culled from
        // painting entirely, leaving the probes forever unset). Positioned
        // absolute so it takes no space in the column; transparent so it
        // never shows — BUT it still paints, populating `panel_height()`
        // BEFORE the first toggle, so the toggle's prediction always has a
        // real panel height. The layer drops out by itself once the cache is
        // populated.
        if self.measure.panel_height().is_none() {
            root = root.child(
                div()
                    .absolute()
                    .top(px(0.))
                    .left(px(0.))
                    .opacity(0.)
                    // Mirror the footer's panel wrapper width (chips wrap
                    // differently at different widths).
                    .child(
                        div()
                            .mx(px(14.))
                            .child(probe(self.measure.panel_top.clone()))
                            .child(self.options_panel.clone())
                            .child(probe(self.measure.panel_end.clone())),
                    ),
            );
        }
        root.child(card)
    }
}

/// Hide the popup state, stop any playing TTS clips, and close the popup
/// window. Shared by the title-strip close button and Escape.
fn close_popup(state: &Entity<DictState>, window: &mut Window, cx: &mut gpui::App) {
    state.update(cx, |s, cx| {
        if let Some(engine) = s.quick_translate_engine.as_mut() {
            engine.hide_popup();
        }
        // The window is going away — stop speaking the source/translation
        // clips rather than leaving orphaned audio playing.
        s.playback_source.stop();
        s.playback_translation.stop();
        s.qt_popup_window = None;
        // A user dismissal cancels any in-flight programmatic replace.
        s.qt_replace_pending = false;
        cx.notify();
    });
    window.remove_window();
}
