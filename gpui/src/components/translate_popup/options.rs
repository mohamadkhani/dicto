//! The inline Options panel for the popup: adaptive pickers for provider,
//! model, target language, TTS preset, and voice. Each picker mutates
//! `DictState`, persists, and — for translation-affecting changes — reloads
//! the translator.
//!
//! Every row is a general [`OptionPicker`]: chips while its catalog has up to
//! [`CHIPS_UP_TO`] options (provider, TTS presets, Anthropic's models), a
//! dropdown beyond it (13 target languages, OpenAI models, preset voices).
//! The panel is a stateful entity so the pickers outlive single frames;
//! confirmation callbacks write back into `DictState`, and
//! [`OptionsPanel::sync`] reconciles every picker from the live settings on
//! the view's poll tick — refreshing dependent catalogs (model list after a
//! provider switch, voice list after a TTS switch) and keeping the popup in
//! step with edits made in the main window's settings while it is open.

use std::sync::Arc;
use std::time::Duration;

use gpui::ease_in_out;
use gpui::{
    Animation, AnimationExt as _, AppContext as _, Context, Entity, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Transformation, Window, div, percentage, px, svg,
};
use gpui_component::{h_flex, v_flex};
use mdict_rs::settings::{LlmProvider, QuickTranslateSettings};

use crate::components::option_picker::{OptionPicker, PickerItem, PickerProps};
use crate::components::qt_catalog;
use crate::{colors, state::DictState};

/// Pickers render chips for lists of up to this many options; longer
/// catalogs (13 target languages, OpenAI models, preset voices) collapse
/// into dropdowns.
const CHIPS_UP_TO: usize = 4;

/// The settings fields the pickers are built from. `sync` compares this key
/// to decide whether the catalogs need rebuilding (`QuickTranslateSettings`
/// has no `PartialEq`).
type SettingsKey = (LlmProvider, String, String, String, String, String);

fn settings_key(s: &QuickTranslateSettings) -> SettingsKey {
    (
        s.llm_provider,
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

fn provider_id(provider: LlmProvider) -> &'static str {
    match provider {
        LlmProvider::Anthropic => "anthropic",
        LlmProvider::OpenAiCompatible => "openai",
    }
}

fn provider_items() -> Vec<PickerItem> {
    vec![
        PickerItem::new("anthropic", "Anthropic"),
        PickerItem::new("openai", "OpenAI-compatible"),
    ]
}

fn model_items(settings: &QuickTranslateSettings) -> Vec<PickerItem> {
    let mut items = qt_catalog::models_for(settings.llm_provider)
        .iter()
        .map(|&(id, label)| PickerItem::new(id, label))
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

/// The TTS picker's value: the active preset's label, or the raw model for a
/// hand-edited (model, base_url) pair.
fn tts_id(settings: &QuickTranslateSettings) -> String {
    match qt_catalog::find_tts_preset(&settings.tts.model, &settings.tts.api_base_url) {
        Some(ix) => qt_catalog::TTS_PRESETS[ix].label.to_string(),
        None => settings.tts.model.clone(),
    }
}

fn tts_items(settings: &QuickTranslateSettings) -> Vec<PickerItem> {
    let mut items = qt_catalog::TTS_PRESETS
        .iter()
        .map(|preset| PickerItem::new(preset.label, preset.label))
        .collect();
    if let Some(id) = opt(&tts_id(settings)) {
        push_custom(&mut items, &id);
    }
    items
}

fn voice_items(settings: &QuickTranslateSettings) -> Vec<PickerItem> {
    let mut items =
        match qt_catalog::find_tts_preset(&settings.tts.model, &settings.tts.api_base_url) {
            Some(ix) => qt_catalog::TTS_PRESETS[ix]
                .voices
                .iter()
                .map(|&voice| PickerItem::new(voice, voice))
                .collect(),
            // Hand-edited TTS config: no preset voices to offer.
            None => Vec::new(),
        };
    if let Some(voice) = opt(&settings.tts.voice) {
        push_custom(&mut items, &voice);
    }
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

// ---------------------------------------------------------------------------
// The panel entity
// ---------------------------------------------------------------------------

/// The inline Options panel: adaptive pickers for provider, model, target
/// language, TTS preset, and voice.
pub(crate) struct OptionsPanel {
    state: Entity<DictState>,
    provider: Entity<OptionPicker>,
    model: Entity<OptionPicker>,
    target: Entity<OptionPicker>,
    tts: Entity<OptionPicker>,
    voice: Entity<OptionPicker>,
    /// Settings snapshot the pickers were last built from; [`Self::sync`]
    /// rebuilds when the live settings diverge.
    synced: SettingsKey,
}

impl OptionsPanel {
    pub(crate) fn new(
        state: Entity<DictState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = state.read(cx).quick_translate.clone();

        let provider = Self::picker(
            "qt-provider",
            provider_items(),
            Some(provider_id(settings.llm_provider).into()),
            None,
            {
                let state = state.clone();
                move |id, _window, cx| {
                    let provider = match id {
                        "anthropic" => LlmProvider::Anthropic,
                        _ => LlmProvider::OpenAiCompatible,
                    };
                    // Switching provider swaps the model catalog: commit the
                    // first model of the new list so the model picker never
                    // shows a stale choice.
                    let first_model = qt_catalog::models_for(provider)
                        .first()
                        .map(|&(id, _)| id.to_string())
                        .unwrap_or_default();
                    commit(&state, cx, |st| {
                        st.quick_translate.llm_provider = provider;
                        st.quick_translate.model = first_model;
                    });
                }
            },
            window,
            cx,
        );

        let model = Self::picker(
            "qt-model",
            model_items(&settings),
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

        let tts = Self::picker(
            "qt-tts",
            tts_items(&settings),
            opt(&tts_id(&settings)),
            None,
            {
                let state = state.clone();
                move |id, _window, cx| {
                    // The picker's value is the preset label; a "Custom: …"
                    // item (a raw model id) matches no preset and is
                    // display-only.
                    let Some(preset) = qt_catalog::TTS_PRESETS
                        .iter()
                        .find(|preset| preset.label == id)
                    else {
                        return;
                    };
                    let model = preset.model.to_string();
                    let base_url = preset.base_url.to_string();
                    let first_voice = preset
                        .voices
                        .first()
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    commit(&state, cx, |st| {
                        st.quick_translate.tts.model = model;
                        st.quick_translate.tts.api_base_url = base_url;
                        st.quick_translate.tts.voice = first_voice;
                    });
                    // A clip loaded with the old voice/model is no longer
                    // correct — stop it (if playing/paused) so the new
                    // settings take effect on the next press.
                    state.update(cx, |st, _cx| st.invalidate_tts_clips_for_settings(None));
                }
            },
            window,
            cx,
        );

        let voice = Self::picker(
            "qt-voice",
            voice_items(&settings),
            opt(&settings.tts.voice),
            Some("Select voice…"),
            {
                let state = state.clone();
                move |id, _window, cx| {
                    let voice = id.to_string();
                    commit(&state, cx, |st| st.quick_translate.tts.voice = voice);
                    state.update(cx, |st, _cx| st.invalidate_tts_clips_for_settings(None));
                }
            },
            window,
            cx,
        );

        Self {
            state,
            provider,
            model,
            target,
            tts,
            voice,
            synced: settings_key(&settings),
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
        let settings = self.state.read(cx).quick_translate.clone();
        let key = settings_key(&settings);
        if key == self.synced {
            return;
        }
        self.synced = key;

        for (picker, items, selected) in [
            (
                &self.provider,
                provider_items(),
                Some(SharedString::from(provider_id(settings.llm_provider))),
            ),
            (&self.model, model_items(&settings), opt(&settings.model)),
            (
                &self.target,
                target_items(&settings),
                opt(&settings.target_lang),
            ),
            (&self.tts, tts_items(&settings), opt(&tts_id(&settings))),
            (
                &self.voice,
                voice_items(&settings),
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
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
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
                    .child(field("Provider", self.provider.clone().into_any_element()))
                    .child(field("Model", self.model.clone().into_any_element()))
                    .child(field("Target", self.target.clone().into_any_element()))
                    .child(divider())
                    .child(field("TTS", self.tts.clone().into_any_element()))
                    .child(field("Voice", self.voice.clone().into_any_element())),
            )
    }
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
