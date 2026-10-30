//! The inline Options panel for the popup: adaptive pickers for model,
//! target language, TTS preset, and voice. Each picker mutates
//! `DictState`, persists, and — for translation-affecting changes — reloads
//! the translator.
//!
//! Every row is a general [`OptionPicker`]: chips while its catalog has up to
//! [`CHIPS_UP_TO`] options (TTS presets), a dropdown beyond it (13 target
//! languages, models, preset voices). The panel is a stateful entity so the
//! pickers outlive single frames; confirmation callbacks write back into
//! `DictState`, and [`OptionsPanel::sync`] reconciles every picker from the
//! live settings on the view's poll tick — refreshing dependent catalogs
//! (voice list after a TTS switch) and keeping the popup in step with edits
//! made in the main window's settings while it is open.

use std::sync::Arc;
use std::time::Duration;

use gpui::ease_in_out;
use gpui::{
    Animation, AnimationExt as _, App, AppContext as _, AsyncApp, Context, Entity,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Transformation, Window, div, percentage,
    prelude::FluentBuilder as _, px, svg,
};
use gpui_component::{h_flex, v_flex};
use mdict_rs::settings::QuickTranslateSettings;

use crate::components::option_picker::{OptionPicker, PickerItem, PickerProps};
use crate::components::qt_catalog;
use crate::{colors, state::DictState};

/// Pickers render chips for lists of up to this many options; longer
/// catalogs (13 target languages, models, preset voices) collapse
/// into dropdowns.
const CHIPS_UP_TO: usize = 4;

/// The settings fields the pickers are built from. `sync` compares this key
/// to decide whether the catalogs need rebuilding (`QuickTranslateSettings`
/// has no `PartialEq`).
type SettingsKey = (String, String, String, String, String);

fn settings_key(s: &QuickTranslateSettings) -> SettingsKey {
    (
        s.model.clone(),
        s.target_lang.clone(),
        s.tts.model.clone(),
        s.tts.api_base_url.clone(),
        s.tts.voice.clone(),
    )
}

// ---------------------------------------------------------------------------
// Per-picker catalogs, built from the current settings
// ---------------------------------------------------------------------------

fn model_items(
    state: &Entity<DictState>,
    settings: &QuickTranslateSettings,
    cx: &App,
) -> Vec<PickerItem> {
    // Catalog comes from the /models load only — no hardcoded list.
    let mut items: Vec<PickerItem> = state
        .read(cx)
        .qt_openai_models
        .iter()
        .map(|m| PickerItem::new(m.clone(), m.clone()))
        .collect();
    push_custom(&mut items, &settings.model);
    items
}

fn target_items(settings: &QuickTranslateSettings) -> Vec<PickerItem> {
    let mut items = qt_catalog::TARGET_LANGS
        .iter()
        .map(|&lang| PickerItem::new(lang, lang))
        .collect();
    push_custom(&mut items, &settings.target_lang);
    items
}

/// Keep hand-edited settings values visible: when the current value is not
/// in the catalog, append a "Custom: …" item so nothing is silently lost —
/// the value stays committed, just labeled as off-catalog.
fn push_custom(items: &mut Vec<PickerItem>, current: &str) {
    if !current.is_empty() && !items.iter().any(|i| i.id.as_ref() == current) {
        items.push(PickerItem::new(
            current.to_string(),
            format!("Custom: {current}"),
        ));
    }
}

/// `None` for empty strings — "nothing selected".
fn opt(s: &str) -> Option<SharedString> {
    (!s.is_empty()).then(|| s.to_string().into())
}

/// Commit a settings mutation: persist it and reload the translator so the
/// engine's settings copy (which the popup reads) stays in sync. The pickers
/// reconcile on the next poll tick ([`OptionsPanel::sync`]).
fn commit(state: &Entity<DictState>, cx: &mut gpui::App, mutate: impl FnOnce(&mut DictState)) {
    state.update(cx, |st, cx| {
        mutate(st);
        st.save_settings(cx);
        st.reload_translator(cx);
    });
}

/// The TTS model catalog: a TTS-filtered /models load when available, else
/// the hardcoded OpenAI TTS fallback — plus a "Custom: …" item for
/// hand-edited values.
fn tts_model_items(
    settings: &QuickTranslateSettings,
    loaded: Vec<dicto_translate::openai::TtsModel>,
) -> Vec<PickerItem> {
    // Catalog: the TTS-filtered /models load when it found models, else
    // the curated fallback.
    let mut items: Vec<PickerItem> = if !loaded.is_empty() {
        loaded
            .iter()
            .map(|m| PickerItem::new(m.id.clone(), m.id.clone()))
            .collect()
    } else {
        qt_catalog::OPENAI_TTS_MODELS
            .iter()
            .map(|&id| PickerItem::new(id, id))
            .collect()
    };
    let current = settings.tts.model.as_str();
    if !current.is_empty() && !items.iter().any(|i| i.id.as_ref() == current) {
        items.push(PickerItem::new(
            current.to_string(),
            format!("Custom: {current}"),
        ));
    }
    items
}

/// The voices of the selected TTS model, from the /models load
/// (`supported_voices`) — nothing hardcoded. The current value stays
/// reachable as "Custom: …".
fn voice_items(
    state: &Entity<DictState>,
    settings: &QuickTranslateSettings,
    cx: &App,
) -> Vec<PickerItem> {
    let current_model = settings.tts.model.as_str();
    let mut items: Vec<PickerItem> = state
        .read(cx)
        .qt_tts_models
        .iter()
        .find(|m| m.id == current_model)
        .map(|m| {
            m.voices
                .iter()
                .map(|v| PickerItem::new(v.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();
    let current = settings.tts.voice.as_str();
    if !current.is_empty() && !items.iter().any(|i| i.id.as_ref() == current) {
        items.push(PickerItem::new(
            current.to_string(),
            format!("Custom: {current}"),
        ));
    }
    items
}

/// One-shot per app run: fetch chat models (for the Model picker) and
/// TTS models + their published voices (for the TTS Model/Voice pickers)
/// from the configured endpoints, in the background. Silent on failure —
/// the pickers keep their fallback lists / current values.
fn autoload_models(state: &Entity<DictState>, cx: &mut Context<OptionsPanel>) {
    // Mark immediately so concurrent sync ticks don't double-spawn.
    state.update(cx, |st, _| st.qt_models_autoloaded = true);

    let quick = state.read(cx).quick_translate.clone();
    let (t_key, t_base) = (quick.api_key.clone(), quick.api_base_url.clone());
    let (v_key, v_base) = (quick.tts.api_key.clone(), quick.tts.api_base_url.clone());
    let state = state.clone();

    cx.spawn(async move |this, cx: &mut AsyncApp| {
        let bg = cx.background_executor();
        let chat =
            bg.spawn(async move { dicto_translate::openai::list_models(&t_key, &t_base).ok() });
        let tts =
            bg.spawn(async move { dicto_translate::openai::list_tts_models(&v_key, &v_base).ok() });
        let (chat, tts) = (chat.await, tts.await);

        cx.update_entity(&state, |st, cx| {
            if let Some(models) = chat
                && !models.is_empty()
            {
                st.qt_openai_models = models;
            }
            if let Some(models) = tts
                && !models.is_empty()
            {
                st.qt_tts_models = models;
            }
            cx.notify();
        });
        let _ = this;
    })
    .detach();
}

// ---------------------------------------------------------------------------
// The panel entity
// ---------------------------------------------------------------------------

/// The inline Options panel: adaptive pickers for model and target
/// language, plus free-text TTS model/voice inputs.
pub(crate) struct OptionsPanel {
    state: Entity<DictState>,
    model: Entity<OptionPicker>,
    target: Entity<OptionPicker>,
    tts_model: Entity<OptionPicker>,
    tts_voice: Entity<OptionPicker>,
    /// Settings snapshot the pickers were last built from; [`Self::sync`]
    /// rebuilds when the live settings diverge.
    synced: SettingsKey,
    /// Length of the shared TTS /models load at last sync — its change
    /// alone must also trigger a picker rebuild.
    synced_loaded: usize,
}

impl OptionsPanel {
    pub(crate) fn new(
        state: Entity<DictState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = state.read(cx).quick_translate.clone();

        let model = Self::picker(
            "qt-model",
            model_items(&state, &settings, cx),
            opt(&settings.model),
            None,
            {
                let state = state.clone();
                move |id, _window, cx| {
                    let model = id.to_string();
                    commit(&state, cx, |st| st.quick_translate.model = model);
                }
            },
            window,
            cx,
        );

        let target = Self::picker(
            "qt-target",
            target_items(&settings),
            opt(&settings.target_lang),
            Some("Select language…"),
            {
                let state = state.clone();
                move |id, _window, cx| {
                    let target = id.to_string();
                    commit(&state, cx, |st| st.quick_translate.target_lang = target);
                }
            },
            window,
            cx,
        );

        // TTS model: adaptive picker over the TTS-filtered /models load
        // (fallback: hardcoded OpenAI TTS list). Voice: free text — no API
        // lists voices.
        let tts_model = Self::picker(
            "qt-tts-model",
            tts_model_items(&settings, state.read(cx).qt_tts_models.clone()),
            opt(&settings.tts.model),
            Some("Select TTS model…"),
            {
                let state = state.clone();
                move |id, _window, cx| {
                    commit(&state, cx, |st| {
                        st.quick_translate.tts.model = id.to_string();
                        // A model without a published voice list takes its
                        // default voice — drop the stale one.
                        let voiceless = st
                            .qt_tts_models
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| m.voices.is_empty())
                            .unwrap_or(false);
                        if voiceless {
                            st.quick_translate.tts.voice = String::new();
                        }
                    });
                    // A clip loaded with the old model is no longer correct.
                    state.update(cx, |st, _cx| st.invalidate_tts_clips_for_settings(None));
                }
            },
            window,
            cx,
        );
        let tts_voice = Self::picker(
            "qt-tts-voice",
            voice_items(&state, &settings, cx),
            opt(&settings.tts.voice),
            Some("Select voice…"),
            {
                let state = state.clone();
                move |id, _window, cx| {
                    commit(&state, cx, |st| {
                        st.quick_translate.tts.voice = id.to_string();
                    });
                    // A clip loaded with the old voice is no longer correct.
                    state.update(cx, |st, _cx| st.invalidate_tts_clips_for_settings(None));
                }
            },
            window,
            cx,
        );

        let synced_loaded = state.read(cx).qt_tts_models.len();
        Self {
            state,
            model,
            target,
            tts_model,
            tts_voice,
            synced: settings_key(&settings),
            synced_loaded,
        }
    }

    fn picker(
        id: &str,
        items: Vec<PickerItem>,
        selected: Option<SharedString>,
        placeholder: Option<&str>,
        on_change: impl Fn(&str, &mut Window, &mut gpui::App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<OptionPicker> {
        cx.new(|cx| {
            OptionPicker::new(
                PickerProps {
                    id: id.into(),
                    items,
                    selected,
                    chips_up_to: CHIPS_UP_TO,
                    placeholder: placeholder.map(Into::into),
                    on_change: Arc::new(on_change),
                },
                window,
                cx,
            )
        })
    }

    /// Reconcile every picker with the live settings. Called on the view's
    /// poll tick; skips work while the settings are unchanged.
    pub(crate) fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Kick the once-per-session models+voices autoload before the
        // unchanged-check: the popup has no manual load button, and without
        // this the model/voice pickers would stay on their fallback lists
        // forever.
        if !self.state.read(cx).qt_models_autoloaded {
            autoload_models(&self.state, cx);
        }

        let settings = self.state.read(cx).quick_translate.clone();
        let key = settings_key(&settings);
        let loaded_key = self.state.read(cx).qt_tts_models.len();
        // The TTS voice input is its own source of truth (its edits commit
        // straight into settings) — only the pickers need reconciliation.
        // The TTS model catalog also changes when a /models load lands in
        // the shared DictState, so key on its length too.
        if key == self.synced && loaded_key == self.synced_loaded {
            return;
        }
        self.synced = key;
        self.synced_loaded = loaded_key;

        for (picker, items, selected) in [
            (
                &self.model,
                model_items(&self.state, &settings, cx),
                opt(&settings.model),
            ),
            (
                &self.target,
                target_items(&settings),
                opt(&settings.target_lang),
            ),
            (
                &self.tts_model,
                tts_model_items(&settings, self.state.read(cx).qt_tts_models.clone()),
                opt(&settings.tts.model),
            ),
            (
                &self.tts_voice,
                voice_items(&self.state, &settings, cx),
                opt(&settings.tts.voice),
            ),
        ] {
            picker.update(cx, |picker, cx| {
                picker.set_selection(items, selected, window, cx)
            });
        }
    }
}

impl Render for OptionsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Natural height, like the design: the panel stacks in flow below the
        // footer toggle; the card's flexing body absorbs the growth.
        v_flex()
            .bg(colors::bg())
            .rounded(px(6.))
            .border_1()
            .border_color(colors::border())
            .child(
                v_flex()
                    .gap(px(8.))
                    .p(px(10.))
                    .child(field("Model", self.model.clone().into_any_element()))
                    .child(field("Target", self.target.clone().into_any_element()))
                    .child(divider())
                    .child(field(
                        "TTS Model",
                        self.tts_model.clone().into_any_element(),
                    ))
                    // Hide Voice entirely for models that publish no voice
                    // list — they use their default voice.
                    .when(tts_model_has_voices_impl(self.state.read(cx)), |row| {
                        row.child(field("Voice", self.tts_voice.clone().into_any_element()))
                    }),
            )
    }
}

/// Whether the currently selected TTS model publishes a `supported_voices`
/// list. Voiceless models use their default voice, so the Voice row is
/// hidden for them.
fn tts_model_has_voices_impl(state: &DictState) -> bool {
    state
        .qt_tts_models
        .iter()
        .find(|m| m.id == state.quick_translate.tts.model)
        .map(|m| !m.voices.is_empty())
        .unwrap_or(false)
}

/// Label (small) + control, side by side like the design's
/// `grid-cols-[64px_1fr]`: fixed 64px label column, 12px gap, both
/// top-aligned; compact for the 460px popup.
fn field(label: &str, content: gpui::AnyElement) -> gpui::AnyElement {
    h_flex()
        .items_start()
        .gap(px(12.))
        .child(
            div()
                .w(px(64.))
                .pt(px(6.))
                .text_size(px(10.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(label.to_uppercase())),
        )
        .child(div().flex_1().child(content))
        .into_any_element()
}

fn divider() -> gpui::AnyElement {
    div().h(px(1.)).bg(colors::border()).into_any_element()
}

// ---------------------------------------------------------------------------
// The footer toggle
// ---------------------------------------------------------------------------

/// The Options footer toggle: chevron + label + right-aligned provider ·
/// hotkey summary. Disclosure order: the toggle sits above the panel it
/// reveals, both clipped by the card's rounded bottom corners.
pub(crate) fn options_toggle(
    open: bool,
    summary: &str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::AnyElement {
    h_flex()
        .id("qt-popup-options-toggle")
        .items_center()
        .gap(px(6.))
        .px(px(14.))
        .py(px(8.))
        .text_size(px(12.))
        .text_color(colors::text_secondary())
        .cursor_pointer()
        .hover(|s| s.text_color(colors::text()))
        .child(chevron(open))
        .child(SharedString::from("Options"))
        .child(div().flex_1())
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(summary.to_string())),
        )
        .on_click(on_click)
        .into_any_element()
}

/// The disclosure chevron next to the "Options" label. A single
/// chevron-right SVG that rotates 90° to point down when the panel is open —
/// animated, not a character swap.
///
/// The animation id embeds the state (`chevron-open` / `chevron-closed`), so
/// each toggle restarts the rotation from 0 rather than continuing mid-flight.
/// The *target* angle also flips with the state: closed → 0°, open → 90°.
/// Both matter: a shared id would make the second click snap (animation
/// already finished), and a fixed 90° target would spin 90° backward instead
/// of 90° forward when closing.
fn chevron(open: bool) -> gpui::AnyElement {
    // `Transformation::rotate` takes a fraction of a FULL turn: percentage(1.0)
    // is 360°. A quarter turn (90°: right → down) is 0.25. The animation
    // always replays from delta 0, so the animator interpolates between the
    // START and END angles of THIS transition: open = 0 → 0.25, close = 0.25 → 0.
    let (id, start, end) = if open {
        ("qt-chevron-open", 0.0, 0.25)
    } else {
        ("qt-chevron-closed", 0.25, 0.0)
    };
    svg()
        .path("icons/chevron-right.svg")
        .size(px(12.))
        .text_color(if open {
            colors::primary()
        } else {
            colors::text_secondary()
        })
        .with_animation(
            id,
            Animation::new(Duration::from_millis(180)).with_easing(ease_in_out),
            move |svg, delta| {
                let angle = start + (end - start) * delta;
                svg.with_transformation(Transformation::rotate(percentage(angle)))
            },
        )
        .into_any_element()
}
