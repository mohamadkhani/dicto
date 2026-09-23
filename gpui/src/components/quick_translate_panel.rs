//! Quick Translate settings tab.
//!
//! Allows the user to configure the quick-translate feature: enable/disable,
//! hotkey, LLM provider, API key, model, target language.

use gpui::{
    AppContext as _, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use mdict_rs::settings::LlmProvider;

use crate::{colors, state::DictState};

/// Build the Quick Translate settings tab content.
pub fn quick_translate_tab_content(
    state: Entity<DictState>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    let settings = state.read(cx).quick_translate.clone();
    let backend = state.read(cx).hotkey_backend.clone();

    // Lazily create persistent InputState entities for each editable field on
    // first render, seeded from loaded settings. They persist on DictState so
    // focus and cursor survive re-renders. We subscribe once so typing writes
    // straight back to settings.
    ensure_qt_inputs(&state, &settings, window, cx);

    let header = v_flex()
        .gap(px(4.))
        .pb(px(16.))
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::text())
                .child(SharedString::from("Quick Translate")),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(
                    "Translate selected text anywhere on your screen with a global hotkey.",
                )),
        );

    // Enable toggle
    let toggle_state = state.clone();
    let enabled = settings.enabled;
    let enable_row = h_flex()
        .justify_between()
        .items_center()
        .py(px(10.))
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors::text())
                .child(SharedString::from("Enable Quick Translate")),
        )
        .child(toggle_switch("qt-enable", enabled, move |cx| {
            toggle_state.update(cx, |s, cx| {
                s.quick_translate.enabled = !enabled;
                s.save_settings(cx);
                s.reload_hotkey(cx);
            });
        }));

    // Hotkey display
    let hotkey_value = settings.hotkey.clone();
    let hotkey_row = h_flex()
        .justify_between()
        .items_center()
        .py(px(8.))
        .gap(px(12.))
        .child(
            div()
                .w(px(120.))
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from("Global Hotkey")),
        )
        .child(
            div()
                .px(px(10.))
                .py(px(5.))
                .rounded(px(4.))
                .bg(colors::bg())
                .border_1()
                .border_color(colors::border())
                .text_size(px(12.))
                .text_color(colors::text())
                .child(SharedString::from(hotkey_value)),
        );

    // Backend note
    let backend_note = if settings.enabled {
        Some(
            div()
                .text_size(px(11.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(format!(
                    "Hotkey backend: {}",
                    match backend.as_str() {
                        "x11" => "X11 (fully supported)",
                        "windows" => "Windows (fully supported)",
                        "tray_menu" => "Tray menu only — use the tray icon to translate",
                        other => other,
                    }
                ))),
        )
    } else {
        None
    };

    // Provider selector
    let provider_row = provider_selector(&settings, state.clone());

    // API key input
    let api_key_row = input_row(
        "API Key",
        &state.read(cx).qt_api_key_input.clone().unwrap(),
        true, // masked (password-style, with show/hide toggle)
        cx,
    );

    // Base URL input (only for OpenAI-compatible)
    let base_url_row = match settings.llm_provider {
        LlmProvider::OpenAiCompatible => Some(input_row(
            "API Base URL",
            &state.read(cx).qt_base_url_input.clone().unwrap(),
            false,
            cx,
        )),
        LlmProvider::Anthropic => None,
    };

    // Model selector — segmented buttons per provider (hardcoded valid list).
    let model_row = model_selector(&settings, state.clone());

    // Target language — common-language buttons + "Custom…" fallback input.
    let target_lang_row = target_lang_selector(&settings, state.clone(), window, cx);

    // --- Text-to-Speech section ---
    let tts_toggle_state = state.clone();
    let tts_enabled = settings.tts.enabled;
    let tts_enable_row = h_flex()
        .justify_between()
        .items_center()
        .py(px(10.))
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors::text())
                .child(SharedString::from("Use AI Text-to-Speech")),
        )
        .child(toggle_switch("qt-tts-enable", tts_enabled, move |cx| {
            tts_toggle_state.update(cx, |s, cx| {
                s.quick_translate.tts.enabled = !tts_enabled;
                s.save_settings(cx);
            });
        }));

    let tts_api_key_row = input_row(
        "TTS API Key",
        &state.read(cx).qt_tts_api_key_input.clone().unwrap(),
        true,
        cx,
    );

    // TTS provider → model + voice (hardcoded presets; sets model+base_url together).
    let tts_preset_row = tts_preset_selector(&settings, state.clone());
    let tts_voice_row = tts_voice_selector(&settings, state.clone());

    let tts_note = div()
        .text_size(px(11.))
        .text_color(colors::text_secondary())
        .child(SharedString::from(
            "When enabled, Speak uses an OpenAI-compatible /audio/speech endpoint. \
             Pick a provider+model, then a voice. Leave disabled to use system TTS (espeak-ng).",
        ));

    // Warning if API key is missing
    let warning = if settings.enabled && settings.api_key.is_empty() {
        Some(
            div()
                .mt(px(8.))
                .p(px(10.))
                .rounded_md()
                .bg(colors::surface())
                .border_1()
                .border_color(colors::update())
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors::update())
                        .child(SharedString::from(
                            "⚠ API key is required for translation. Quick translate won't work without it.",
                        )),
                ),
        )
    } else {
        None
    };

    let mut body = v_flex()
        .gap(px(4.))
        .w_full()
        .h_full()
        .p(px(4.))
        .id("qt-settings-scroll")
        .overflow_y_scroll();
    body = body.child(header);
    body = body.child(divider());
    body = body.child(enable_row);
    body = body.child(hotkey_row);
    if let Some(note) = backend_note {
        body = body.child(note);
    }
    body = body.child(divider());
    body = body.child(section_title("Translation Provider"));
    body = body.child(provider_row);
    body = body.child(api_key_row);
    if let Some(row) = base_url_row {
        body = body.child(row);
    }
    body = body.child(model_row);
    body = body.child(target_lang_row);
    if let Some(n) = warning {
        body = body.child(n);
    }
    // Text-to-Speech section
    body = body.child(divider());
    body = body.child(section_title("Text-to-Speech"));
    body = body.child(tts_enable_row);
    if settings.tts.enabled {
        body = body.child(tts_api_key_row);
        body = body.child(tts_preset_row);
        body = body.child(tts_voice_row);
    }
    body = body.child(tts_note);

    // Outer div participates in the parent's flex layout (flex_1 = remaining
    // height). The overflow wrapper from overflow_y_scroll loses flex_grow, so
    // we separate the two concerns: outer = flex sizing, inner = h_full scroll.
    div()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .child(body)
        .into_any_element()
}

// --- Helper widgets ---

fn section_title(text: &str) -> gpui::AnyElement {
    div()
        .mt(px(12.))
        .mb(px(4.))
        .text_size(px(11.))
        .text_color(colors::text_secondary())
        .child(SharedString::from(text.to_uppercase()))
        .into_any_element()
}

fn divider() -> gpui::AnyElement {
    div().h(px(1.)).bg(colors::border()).into_any_element()
}

fn toggle_switch(
    id: &'static str,
    on: bool,
    on_click: impl Fn(&mut gpui::App) + 'static,
) -> gpui::AnyElement {
    div()
        .id(SharedString::from(id))
        .w(px(36.))
        .h(px(20.))
        .rounded(px(10.))
        .flex()
        .items_center()
        .px(px(2.))
        .cursor_pointer()
        .bg(if on {
            colors::primary()
        } else {
            colors::border()
        })
        .child(
            div()
                .w(px(16.))
                .h(px(16.))
                .rounded(px(8.))
                .bg(colors::bg())
                .ml(if on { px(16.) } else { px(0.) }),
        )
        .on_click(move |_, _, cx| on_click(cx))
        .into_any_element()
}

/// Lazily create and seed the persistent `InputState` entities for the four
/// editable Quick Translate fields, then subscribe to each so edits flow back
/// into `DictState.quick_translate` and are persisted.
///
/// On subsequent renders we only re-seed a field's value when it has drifted
/// from settings due to an *external* change (e.g. provider switch, settings
/// reload) AND the field isn't focused — so we never clobber in-progress typing.
fn ensure_qt_inputs(
    state: &Entity<DictState>,
    settings: &mdict_rs::settings::QuickTranslateSettings,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let needs_init = state.read(cx).qt_api_key_input.is_none();

    // Field descriptors: (slot getter, value, placeholder, on_change).
    // We build closures that update settings + persist + (optionally) reload.
    let api_key_cb = {
        let s = state.clone();
        move |v: String, cx: &mut gpui::App| {
            s.update(cx, |st, cx| {
                st.quick_translate.api_key = v;
                st.save_settings(cx);
                st.reload_translator(cx);
            });
        }
    };
    let base_url_cb = {
        let s = state.clone();
        move |v: String, cx: &mut gpui::App| {
            s.update(cx, |st, cx| {
                st.quick_translate.api_base_url = v;
                st.save_settings(cx);
                st.reload_translator(cx);
            });
        }
    };
    let model_cb = {
        let s = state.clone();
        move |v: String, cx: &mut gpui::App| {
            s.update(cx, |st, cx| {
                st.quick_translate.model = v;
                st.save_settings(cx);
                st.reload_translator(cx);
            });
        }
    };
    let target_lang_cb = {
        let s = state.clone();
        move |v: String, cx: &mut gpui::App| {
            s.update(cx, |st, cx| {
                st.quick_translate.target_lang = v;
                st.save_settings(cx);
            });
        }
    };

    // TTS field callback (API key is the only free-text TTS input; model/voice/
    // base_url are set via the catalog selectors).
    let tts_api_key_cb = {
        let s = state.clone();
        move |v: String, cx: &mut gpui::App| {
            s.update(cx, |st, cx| {
                st.quick_translate.tts.api_key = v;
                st.save_settings(cx);
            });
        }
    };

    if needs_init {
        // First render: create entities, seed values, set placeholders, observe.
        let api_key = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder("sk-...", window, cx);
            s.set_value(settings.api_key.clone(), window, cx);
            s
        });
        let base_url = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder("http://localhost:11434/v1", window, cx);
            s.set_value(settings.api_base_url.clone(), window, cx);
            s
        });
        let model = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder("claude-sonnet-4-6", window, cx);
            s.set_value(settings.model.clone(), window, cx);
            s
        });
        let target_lang = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder("English", window, cx);
            s.set_value(settings.target_lang.clone(), window, cx);
            s
        });
        // TTS fields (only API key remains a free-text input; model/voice/base_url
        // are now set via the catalog selectors).
        let tts_api_key = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder("sk-... (separate key allowed)", window, cx);
            s.set_value(settings.tts.api_key.clone(), window, cx);
            s
        });

        observe_input(cx, &api_key, api_key_cb);
        observe_input(cx, &base_url, base_url_cb);
        observe_input(cx, &model, model_cb);
        observe_input(cx, &target_lang, target_lang_cb);
        observe_input(cx, &tts_api_key, tts_api_key_cb);

        state.update(cx, |st, _cx| {
            st.qt_api_key_input = Some(api_key);
            st.qt_base_url_input = Some(base_url);
            st.qt_model_input = Some(model);
            st.qt_target_lang_input = Some(target_lang);
            st.qt_tts_api_key_input = Some(tts_api_key);
            st.qt_inputs_seeded = true;
        });
        return;
    }

    // Already initialized: reconcile values for external changes only.
    // `focused()` guards against clobbering the field the user is editing.
    reconcile(
        &state,
        &settings.api_key,
        |st| &st.qt_api_key_input,
        |st| &mut st.quick_translate.api_key,
        window,
        cx,
        false,
    );
    reconcile(
        &state,
        &settings.api_base_url,
        |st| &st.qt_base_url_input,
        |st| &mut st.quick_translate.api_base_url,
        window,
        cx,
        false,
    );
    reconcile(
        &state,
        &settings.model,
        |st| &st.qt_model_input,
        |st| &mut st.quick_translate.model,
        window,
        cx,
        false,
    );
    reconcile(
        &state,
        &settings.target_lang,
        |st| &st.qt_target_lang_input,
        |st| &mut st.quick_translate.target_lang,
        window,
        cx,
        true,
    );
    // TTS reconciliation (API key only — model/voice/base_url are set via selectors).
    reconcile(
        &state,
        &settings.tts.api_key,
        |st| &st.qt_tts_api_key_input,
        |st| &mut st.quick_translate.tts.api_key,
        window,
        cx,
        false,
    );
}

/// Subscribe to an InputState, firing `on_change(value)` whenever its text
/// changes. The write-back updates `DictState` (and persists), which is the
/// source of truth — the InputState is just the editable view of it.
fn observe_input(
    cx: &mut gpui::App,
    input: &Entity<InputState>,
    on_change: impl Fn(String, &mut gpui::App) + 'static,
) {
    cx.observe(input, move |input, cx| {
        let value = input.read(cx).value().to_string();
        on_change(value, cx);
    })
    .detach();
}

/// If the InputState's text has drifted from the settings value and the field
/// is not focused, push the settings value back into the InputState. This
/// handles external mutations (settings reload, provider preset) without
/// disrupting active typing.
#[allow(clippy::type_complexity)]
fn reconcile(
    state: &Entity<DictState>,
    settings_value: &str,
    slot: impl Fn(&DictState) -> &Option<Entity<InputState>>,
    _settings_field: impl Fn(&mut DictState) -> &mut String,
    window: &mut Window,
    cx: &mut gpui::App,
    _is_target_lang: bool,
) {
    let Some(input) = slot(&state.read(cx)).clone() else {
        return;
    };
    let current = input.read(cx).value().to_string();
    if current.as_str() == settings_value {
        return;
    }
    input.update(cx, |s, cx| {
        s.set_value(settings_value.to_string(), window, cx);
    });
}

/// A labeled, focusable text-input row backed by a real `gpui-component` Input.
fn input_row(
    label: &str,
    input: &Entity<InputState>,
    masked: bool,
    _cx: &mut gpui::App,
) -> gpui::AnyElement {
    let mut el = Input::new(input).appearance(true);
    if masked {
        el = el.mask_toggle();
    }

    h_flex()
        .justify_between()
        .items_center()
        .py(px(8.))
        .gap(px(12.))
        .child(
            div()
                .w(px(120.))
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(label)),
        )
        .child(div().w(px(260.)).child(el))
        .into_any_element()
}

fn provider_selector(
    settings: &mdict_rs::settings::QuickTranslateSettings,
    state: Entity<DictState>,
) -> gpui::AnyElement {
    let anthropic_selected = matches!(settings.llm_provider, LlmProvider::Anthropic);
    let openai_selected = matches!(settings.llm_provider, LlmProvider::OpenAiCompatible);

    h_flex()
        .gap(px(8.))
        .py(px(8.))
        .child(provider_button("Anthropic", anthropic_selected, {
            let s = state.clone();
            move |cx| {
                s.update(cx, |st, cx| {
                    st.quick_translate.llm_provider = LlmProvider::Anthropic;
                    st.save_settings(cx);
                    st.reload_translator(cx);
                });
            }
        }))
        .child(provider_button("OpenAI-compatible", openai_selected, {
            let s = state;
            move |cx| {
                s.update(cx, |st, cx| {
                    st.quick_translate.llm_provider = LlmProvider::OpenAiCompatible;
                    st.save_settings(cx);
                    st.reload_translator(cx);
                });
            }
        }))
        .into_any_element()
}

fn provider_button<F>(label: &str, selected: bool, on_click: F) -> gpui::AnyElement
where
    F: Fn(&mut gpui::App) + 'static,
{
    let base = div()
        .id(SharedString::from(label))
        .px(px(12.))
        .py(px(6.))
        .rounded(px(6.))
        .cursor_pointer()
        .text_size(px(12.))
        .on_click(move |_, _, cx| on_click(cx));

    if selected {
        base.bg(colors::primary())
            .text_color(colors::bg())
            .child(SharedString::from(label))
            .into_any_element()
    } else {
        base.bg(colors::surface())
            .text_color(colors::text_secondary())
            .border_1()
            .border_color(colors::border())
            .child(SharedString::from(label))
            .into_any_element()
    }
}

/// Wrap a control in the standard label (w=120) + content row, matching
/// `input_row`'s geometry so selectors line up with text fields.
fn labeled_row(label: &str, content: gpui::AnyElement) -> gpui::AnyElement {
    h_flex()
        .items_start()
        .py(px(8.))
        .gap(px(12.))
        .child(
            div()
                .w(px(120.))
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(label)),
        )
        .child(div().w(px(260.)).child(content))
        .into_any_element()
}

/// Translation model selector: segmented buttons built from the catalog for
/// the active provider. If the current model isn't in the list, render an
/// extra "Custom: <model>" button so a hand-edited value stays visible.
fn model_selector(
    settings: &mdict_rs::settings::QuickTranslateSettings,
    state: Entity<DictState>,
) -> gpui::AnyElement {
    use crate::components::qt_catalog;

    let models = qt_catalog::models_for(settings.llm_provider);
    let current = settings.model.as_str();
    let in_list = models.iter().any(|(id, _)| *id == current);

    let mut row = h_flex().gap(px(6.)).flex_wrap();

    for (id, label) in models {
        let selected = *id == current;
        let id_owned = id.to_string();
        row = row.child(provider_button(label, selected, {
            let s = state.clone();
            move |cx| {
                s.update(cx, |st, cx| {
                    st.quick_translate.model = id_owned.clone();
                    st.save_settings(cx);
                    st.reload_translator(cx);
                });
            }
        }));
    }

    // Custom fallback: keep the current value visible if it's outside the catalog.
    if !in_list && !current.is_empty() {
        let label = format!("Custom: {current}");
        // Already selected by definition (it's the active value); clicking is a no-op.
        row = row.child(provider_button(&label, true, |_| {}));
    }

    labeled_row("Model", row.into_any_element())
}

/// TTS provider+model selector. Each preset commits model AND base_url
/// together, then resets voice to the preset's first voice.
fn tts_preset_selector(
    settings: &mdict_rs::settings::QuickTranslateSettings,
    state: Entity<DictState>,
) -> gpui::AnyElement {
    use crate::components::qt_catalog;

    let presets = qt_catalog::TTS_PRESETS;
    let active = qt_catalog::find_tts_preset(&settings.tts.model, &settings.tts.api_base_url);

    let mut row = h_flex().gap(px(6.)).flex_wrap();

    for (i, preset) in presets.iter().enumerate() {
        let selected = active == Some(i);
        let model = preset.model.to_string();
        let base_url = preset.base_url.to_string();
        let first_voice = preset
            .voices
            .first()
            .map(|v| v.to_string())
            .unwrap_or_default();
        row = row.child(provider_button(preset.label, selected, {
            let s = state.clone();
            move |cx| {
                s.update(cx, |st, cx| {
                    st.quick_translate.tts.model = model.clone();
                    st.quick_translate.tts.api_base_url = base_url.clone();
                    st.quick_translate.tts.voice = first_voice.clone();
                    st.save_settings(cx);
                });
            }
        }));
    }

    // Custom fallback for hand-edited model/base_url combos.
    if active.is_none() && !settings.tts.model.is_empty() {
        let label = format!("Custom: {}", settings.tts.model);
        row = row.child(provider_button(&label, true, |_| {}));
    }

    labeled_row("Provider", row.into_any_element())
}

/// TTS voice selector: segmented buttons from the active preset's voice list.
/// Only shown when a known preset is active (otherwise there's no voice list).
fn tts_voice_selector(
    settings: &mdict_rs::settings::QuickTranslateSettings,
    state: Entity<DictState>,
) -> gpui::AnyElement {
    use crate::components::qt_catalog;

    let active = qt_catalog::find_tts_preset(&settings.tts.model, &settings.tts.api_base_url);

    let mut content = h_flex().gap(px(6.)).flex_wrap();

    if let Some(idx) = active {
        let preset = &qt_catalog::TTS_PRESETS[idx];
        let current = settings.tts.voice.as_str();
        let in_list = preset.voices.iter().any(|v| *v == current);

        for voice in preset.voices {
            let selected = *voice == current;
            let v = voice.to_string();
            content = content.child(provider_button(voice, selected, {
                let s = state.clone();
                move |cx| {
                    s.update(cx, |st, cx| {
                        st.quick_translate.tts.voice = v.clone();
                        st.save_settings(cx);
                    });
                }
            }));
        }

        if !in_list && !current.is_empty() {
            let label = format!("Custom: {current}");
            content = content.child(provider_button(&label, true, |_| {}));
        }
    } else {
        // No known preset: show the current voice as read-only text.
        content = content.child(
            div()
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(settings.tts.voice.clone())),
        );
    }

    labeled_row("Voice", content.into_any_element())
}

/// Target language selector: common-language buttons + "Custom…". When
/// "Custom…" is active (or the current value isn't in the list), a free-text
/// input appears below so the user can type any language.
fn target_lang_selector(
    settings: &mdict_rs::settings::QuickTranslateSettings,
    state: Entity<DictState>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    use crate::components::qt_catalog;

    let current = settings.target_lang.as_str();
    let in_list = qt_catalog::TARGET_LANGS.iter().any(|l| *l == current);

    let mut buttons = h_flex().gap(px(6.)).flex_wrap();
    for lang in qt_catalog::TARGET_LANGS {
        let selected = *lang == current;
        let l = lang.to_string();
        buttons = buttons.child(provider_button(lang, selected, {
            let s = state.clone();
            move |cx| {
                s.update(cx, |st, cx| {
                    st.quick_translate.target_lang = l.clone();
                    st.save_settings(cx);
                });
            }
        }));
    }

    // "Custom…" button: selected whenever the current value isn't a known language.
    let custom_selected = !in_list;
    buttons = buttons.child(provider_button("Custom…", custom_selected, {
        let s = state.clone();
        move |cx| {
            // Switch to a blank custom value if currently on a known language.
            s.update(cx, |st, cx| {
                if qt_catalog::TARGET_LANGS
                    .iter()
                    .any(|l| *l == st.quick_translate.target_lang.as_str())
                    || st.quick_translate.target_lang.is_empty()
                {
                    st.quick_translate.target_lang = String::new();
                    st.save_settings(cx);
                }
            });
        }
    }));

    let mut block = v_flex().gap(px(6.)).child(buttons);

    // Reveal the free-text input only in custom mode.
    if !in_list {
        let input = input_row(
            "Custom",
            &state.read(cx).qt_target_lang_input.clone().unwrap(),
            false,
            cx,
        );
        block = block.child(input);
    }

    labeled_row("Target Language", block.into_any_element())
}
