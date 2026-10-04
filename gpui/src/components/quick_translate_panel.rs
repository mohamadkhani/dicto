//! Quick Translate settings tab.
//!
//! Allows the user to configure the quick-translate feature: enable/disable,
//! hotkey, LLM provider, API key, model, target language.

use crate::{
    colors,
    state::{DictState, QtKeyTest},
};
use gpui::{
    AppContext as _, AsyncApp, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    h_flex,
    input::{Input, InputState},
    v_flex,
};

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

    // API key input
    let api_key_row = input_row(
        "API Key",
        &state.read(cx).qt_api_key_input.clone().unwrap(),
        true, // masked (password-style, with show/hide toggle)
        cx,
    );

    // Base URL input
    let base_url_row = input_row(
        "API Base URL",
        &state.read(cx).qt_base_url_input.clone().unwrap(),
        false,
        cx,
    );

    // [Test] row after the translation fields — verifies key + base URL +
    // model against the real provider and shows the translated sample.
    let translation_test_state = state.clone();
    let translation_test_row = action_status_row(
        "qt-test-translation",
        "Test",
        "Testing…",
        state.read(cx).qt_translation_test.clone(),
        move |cx| run_translation_test(translation_test_state.clone(), cx),
    );

    // Model picker — same adaptive picker (chips/dropdown) as the popup's
    // Options panel, reconciled from live settings each render.
    let model_row = model_picker_row(&settings, &state, window, cx);

    // "Load models" row: fetches the live model list from
    // {base_url}/models; until then the hardcoded catalog shows.
    let models_load_state = state.clone();
    let models_load_row = action_status_row(
        "qt-models-load",
        "Load models",
        "Loading…",
        state.read(cx).qt_models_load.clone(),
        move |cx| run_models_load(models_load_state.clone(), cx),
    );

    // Target language — same adaptive picker as the popup's Options panel.
    let target_lang_row = target_lang_picker_row(&settings, &state, window, cx);

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

    // Base URL above API key, matching the translation section's order.
    let tts_base_url_row = input_row(
        "TTS Base URL",
        &state.read(cx).qt_tts_base_url_input.clone().unwrap(),
        false,
        cx,
    );

    let tts_api_key_row = input_row(
        "TTS API Key",
        &state.read(cx).qt_tts_api_key_input.clone().unwrap(),
        true,
        cx,
    );

    // TTS model picker + live load from {base_url}/models (TTS-filtered).
    let tts_model_row = tts_model_picker_row(&settings, &state, window, cx);
    let tts_models_load_state = state.clone();
    let tts_models_load_row = action_status_row(
        "qt-tts-models-load",
        "Load TTS models",
        "Loading…",
        state.read(cx).qt_tts_models_load.clone(),
        move |cx| run_tts_models_load(tts_models_load_state.clone(), cx),
    );

    // Voice picker: only shown when the selected model publishes voices
    // via the API (`supported_voices`). A model without a published list
    // uses its default voice — the row is hidden entirely.
    let tts_voice_row = (!tts_voices_for(&settings, &state, cx).is_empty())
        .then(|| tts_voice_picker_row(&settings, &state, window, cx));

    // [Test] row after the TTS fields — synthesizes one short clip (no
    // playback) to verify key + endpoint + model + voice.
    let tts_test_state = state.clone();
    let tts_test_row = action_status_row(
        "qt-test-tts",
        "Test",
        "Testing…",
        state.read(cx).qt_tts_test.clone(),
        move |cx| run_tts_test(tts_test_state.clone(), cx),
    );

    let tts_note = div()
        .text_size(px(11.))
        .text_color(colors::text_secondary())
        .child(SharedString::from(
            "When enabled, Speak uses an OpenAI-compatible /audio/speech endpoint: \
             set the base URL, API key, model, and voice of your provider. \
             Leave disabled to use system TTS (espeak-ng).",
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
    body = body.child(section_title("Translation"));
    body = body.child(base_url_row);
    body = body.child(api_key_row);
    body = body.child(model_row);
    body = body.child(models_load_row);
    body = body.child(target_lang_row);
    body = body.child(translation_test_row);
    if let Some(n) = warning {
        body = body.child(n);
    }
    // Text-to-Speech section
    body = body.child(divider());
    body = body.child(section_title("Text-to-Speech"));
    body = body.child(tts_enable_row);
    if settings.tts.enabled {
        body = body.child(tts_base_url_row);
        body = body.child(tts_api_key_row);
        body = body.child(tts_model_row);
        body = body.child(tts_models_load_row);
        if let Some(row) = tts_voice_row {
            body = body.child(row);
        }
        body = body.child(tts_test_row);
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

/// Row with an action button plus the live result of the last run —
/// shared by the key [Test] buttons and the "Load models" action.
/// Placed directly under a related field so failures are caught at
/// configuration time instead of on first real use.
fn action_status_row(
    id: &'static str,
    idle_label: &str,
    running_label: &str,
    status: QtKeyTest,
    on_click: impl Fn(&mut gpui::App) + 'static,
) -> gpui::AnyElement {
    let (label, msg, color) = match &status {
        QtKeyTest::Idle => (idle_label.to_string(), None, colors::text_secondary()),
        QtKeyTest::Running => (running_label.to_string(), None, colors::text_secondary()),
        QtKeyTest::Ok(text) => (
            idle_label.to_string(),
            Some(format!("✓ {text}")),
            colors::success(),
        ),
        QtKeyTest::Err(text) => (
            idle_label.to_string(),
            Some(format!("✗ {text}")),
            colors::error(),
        ),
    };

    h_flex()
        .w_full()
        .items_center()
        .gap(px(8.))
        .py(px(2.))
        // Align with the content column of input_row / labeled_row
        // (120px label + 12px gap).
        .pl(px(132.))
        .child(
            div()
                .id(SharedString::from(id))
                .cursor_pointer()
                .px(px(8.))
                .py(px(3.))
                .rounded(px(6.))
                .text_size(px(12.))
                .text_color(colors::text())
                .border_1()
                .border_color(colors::border())
                .hover(|s| s.bg(colors::hover()))
                .child(SharedString::from(label))
                .on_click(move |_, _, cx| on_click(cx)),
        )
        // flex_1 + min_w(0): the message wraps inside the remaining width
        // instead of being clipped at the window edge.
        .children(msg.map(|m| {
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(11.))
                .text_color(color)
                .child(SharedString::from(m))
        }))
        .into_any_element()
}

/// Truncate a message for display, cutting on a char boundary.
fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.trim().to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{}…", cut.trim_end())
    }
}

/// Fetch the model list from the configured OpenAI-compatible endpoint
/// (`GET {base_url}/models`) and cache it in `qt_openai_models`; the
/// model picker rebuilds from it on the next render. The hardcoded
/// catalog stays the fallback until a load succeeds.
fn run_models_load(state: Entity<DictState>, cx: &mut gpui::App) {
    let settings = state.read(cx).quick_translate.clone();
    // /models is public on several endpoints (e.g. OpenRouter) — the key
    // is sent only when present.
    if settings.api_base_url.is_empty() {
        state.update(cx, |s, cx| {
            s.qt_models_load = QtKeyTest::Err("API base URL is empty".into());
            cx.notify();
        });
        return;
    }

    state.update(cx, |s, cx| {
        s.qt_models_load = QtKeyTest::Running;
        cx.notify();
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let (key, base) = (settings.api_key.clone(), settings.api_base_url.clone());
        let result = cx
            .background_executor()
            .spawn(async move { dicto_translate::openai::list_models(&key, &base) })
            .await;

        let status = match result {
            Ok(models) if models.is_empty() => {
                QtKeyTest::Err("Endpoint's model list is empty".into())
            }
            Ok(models) => {
                let n = models.len();
                let status = QtKeyTest::Ok(format!("{n} models loaded"));
                cx.update(|cx| {
                    cx.update_entity(&state, |s, cx| {
                        s.qt_openai_models = models;
                        cx.notify();
                    });
                });
                status
            }
            Err(e) => QtKeyTest::Err(shorten(&e.to_string(), 96)),
        };
        cx.update(|cx| {
            cx.update_entity(&state, |s, cx| {
                s.qt_models_load = status;
                cx.notify();
            });
        });
    })
    .detach();
}

/// Fetch the TTS-capable model list from the configured OpenAI-compatible
/// endpoint and cache it in `qt_tts_models`; the TTS model picker rebuilds
/// from it on the next render.
fn run_tts_models_load(state: Entity<DictState>, cx: &mut gpui::App) {
    let tts = state.read(cx).quick_translate.tts.clone();
    // /models is public on several endpoints (e.g. OpenRouter) — the key
    // is sent only when present.
    if tts.api_base_url.is_empty() {
        state.update(cx, |s, cx| {
            s.qt_tts_models_load = QtKeyTest::Err("TTS base URL is empty".into());
            cx.notify();
        });
        return;
    }

    state.update(cx, |s, cx| {
        s.qt_tts_models_load = QtKeyTest::Running;
        cx.notify();
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let (key, base) = (tts.api_key.clone(), tts.api_base_url.clone());
        let result = cx
            .background_executor()
            .spawn(async move { dicto_translate::openai::list_tts_models(&key, &base) })
            .await;

        let status = match result {
            Ok(models) if models.is_empty() => {
                // Some endpoints (OpenRouter) serve TTS models without
                // listing them in /models — keep the fallback list and say
                // so instead of a misleading green "0 loaded".
                QtKeyTest::Err(
                    "Endpoint's model list has no TTS models — use the fallback list".into(),
                )
            }
            Ok(models) => {
                let voices_n = models.iter().filter(|m| !m.voices.is_empty()).count();
                let status = if voices_n > 0 {
                    QtKeyTest::Ok(format!("{voices_n} TTS models loaded (with voice lists)"))
                } else {
                    QtKeyTest::Ok(format!("{} TTS models loaded", models.len()))
                };
                cx.update(|cx| {
                    cx.update_entity(&state, |s, cx| {
                        s.qt_tts_models = models;
                        cx.notify();
                    });
                });
                status
            }
            Err(e) => QtKeyTest::Err(shorten(&e.to_string(), 96)),
        };
        cx.update(|cx| {
            cx.update_entity(&state, |s, cx| {
                s.qt_tts_models_load = status;
                cx.notify();
            });
        });
    })
    .detach();
}

/// Send one short sample through the configured translation provider and
/// record the outcome in `qt_translation_test`. Credentials are tested
/// regardless of the feature's enable toggle.
fn run_translation_test(state: Entity<DictState>, cx: &mut gpui::App) {
    use dicto_translate::TranslationRequest;

    let settings = state.read(cx).quick_translate.clone();
    if settings.api_key.is_empty() {
        state.update(cx, |s, cx| {
            s.qt_translation_test = QtKeyTest::Err("API key is empty".into());
            cx.notify();
        });
        return;
    }

    state.update(cx, |s, cx| {
        s.qt_translation_test = QtKeyTest::Running;
        cx.notify();
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = cx
            .background_executor()
            .spawn(async move {
                // `enabled` is forced true: the test is about the
                // credentials, not the feature toggle.
                let translator = dicto_translate::translator_from_settings(
                    true,
                    &settings.api_key,
                    &settings.api_base_url,
                    &settings.model,
                );
                translator.translate(TranslationRequest {
                    text: "Hello! How are you?".to_string(),
                    source_lang: None,
                    target_lang: settings.target_lang.clone(),
                })
            })
            .await;

        let status = match result {
            Ok(r) => QtKeyTest::Ok(format!("Works — {}", shorten(&r.translated_text, 48))),
            Err(e) => QtKeyTest::Err(shorten(&e.to_string(), 96)),
        };
        cx.update(|cx| {
            cx.update_entity(&state, |s, cx| {
                s.qt_translation_test = status;
                cx.notify();
            });
        });
    })
    .detach();
}

/// Synthesize one short clip through the configured AI TTS endpoint (no
/// playback) and record the outcome in `qt_tts_test`.
fn run_tts_test(state: Entity<DictState>, cx: &mut gpui::App) {
    let tts = state.read(cx).quick_translate.tts.clone();
    if tts.api_key.is_empty() {
        state.update(cx, |s, cx| {
            s.qt_tts_test = QtKeyTest::Err("TTS API key is empty".into());
            cx.notify();
        });
        return;
    }

    state.update(cx, |s, cx| {
        s.qt_tts_test = QtKeyTest::Running;
        cx.notify();
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = cx
            .background_executor()
            .spawn(async move { crate::tts::test_synthesis(&tts) })
            .await;

        let status = match result {
            Ok(len) => QtKeyTest::Ok(format!("Works — {len} bytes of audio")),
            Err(e) => QtKeyTest::Err(shorten(&e.to_string(), 96)),
        };
        cx.update(|cx| {
            cx.update_entity(&state, |s, cx| {
                s.qt_tts_test = status;
                cx.notify();
            });
        });
    })
    .detach();
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

    // TTS field callbacks (API key and base URL are free-text inputs;
    // model/voice are set via the catalog selectors — base_url is committed
    // by presets too, and the input reconciles to match).
    let tts_api_key_cb = {
        let s = state.clone();
        move |v: String, cx: &mut gpui::App| {
            s.update(cx, |st, cx| {
                st.quick_translate.tts.api_key = v;
                st.save_settings(cx);
            });
        }
    };
    let tts_base_url_cb = {
        let s = state.clone();
        move |v: String, cx: &mut gpui::App| {
            s.update(cx, |st, cx| {
                st.quick_translate.tts.api_base_url = v;
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
        // TTS fields (all free-text: base URL, key, model, voice).
        let tts_api_key = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder("sk-... (separate key allowed)", window, cx);
            s.set_value(settings.tts.api_key.clone(), window, cx);
            s
        });
        let tts_base_url = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder("https://api.openai.com/v1", window, cx);
            s.set_value(settings.tts.api_base_url.clone(), window, cx);
            s
        });

        observe_input(cx, &api_key, api_key_cb);
        observe_input(cx, &base_url, base_url_cb);
        observe_input(cx, &model, model_cb);
        observe_input(cx, &target_lang, target_lang_cb);
        observe_input(cx, &tts_api_key, tts_api_key_cb);
        observe_input(cx, &tts_base_url, tts_base_url_cb);

        state.update(cx, |st, _cx| {
            st.qt_api_key_input = Some(api_key);
            st.qt_base_url_input = Some(base_url);
            st.qt_model_input = Some(model);
            st.qt_target_lang_input = Some(target_lang);
            st.qt_tts_api_key_input = Some(tts_api_key);
            st.qt_tts_base_url_input = Some(tts_base_url);
            st.qt_inputs_seeded = true;
        });
        return;
    }

    // Already initialized: reconcile values for external changes only.
    // `focused()` guards against clobbering the field the user is editing.
    reconcile(
        state,
        &settings.api_key,
        |st| &st.qt_api_key_input,
        |st| &mut st.quick_translate.api_key,
        window,
        cx,
        false,
    );
    reconcile(
        state,
        &settings.api_base_url,
        |st| &st.qt_base_url_input,
        |st| &mut st.quick_translate.api_base_url,
        window,
        cx,
        false,
    );
    reconcile(
        state,
        &settings.model,
        |st| &st.qt_model_input,
        |st| &mut st.quick_translate.model,
        window,
        cx,
        false,
    );
    reconcile(
        state,
        &settings.target_lang,
        |st| &st.qt_target_lang_input,
        |st| &mut st.quick_translate.target_lang,
        window,
        cx,
        true,
    );
    // TTS reconciliation (base URL, key, model, voice — all free text).
    reconcile(
        state,
        &settings.tts.api_key,
        |st| &st.qt_tts_api_key_input,
        |st| &mut st.quick_translate.tts.api_key,
        window,
        cx,
        false,
    );
    reconcile(
        state,
        &settings.tts.api_base_url,
        |st| &st.qt_tts_base_url_input,
        |st| &mut st.quick_translate.tts.api_base_url,
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
    let Some(input) = slot(state.read(cx)).clone() else {
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
/// Same geometry as [`labeled_row`] (label 120px + 12px gap + 260px content)
/// and the same size tokens as the OptionPicker's dropdown/chips (small,
/// 11px text, 3px vertical padding) so inputs and pickers read as one.
fn input_row(
    label: &str,
    input: &Entity<InputState>,
    masked: bool,
    _cx: &mut gpui::App,
) -> gpui::AnyElement {
    use gpui_component::Sizable as _;

    let mut el = Input::new(input)
        .appearance(true)
        .small()
        .w_full()
        .text_size(px(11.))
        .py(px(3.))
        .bg(colors::surface())
        .text_color(colors::text())
        .border_color(colors::border())
        .rounded(px(4.));
    if masked {
        el = el.mask_toggle();
    }

    h_flex()
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

/// Chips for model lists of up to this many options; a dropdown beyond —
/// same threshold as the popup's Options panel, so both pickers render
/// identically for the same catalog.
const CHIPS_UP_TO: usize = 4;

/// Translation model picker built on the same adaptive [`OptionPicker`] as
/// the popup's Options panel: chips for short catalogs, a
/// dropdown beyond (OpenAI-compatible), plus a "Custom: …" entry that
/// keeps hand-edited values visible. The picker entity persists on
/// `DictState` and is reconciled from the live settings on every render
/// (provider switch, popup edits, settings reload).
fn model_picker_row(
    settings: &mdict_rs::settings::QuickTranslateSettings,
    state: &Entity<DictState>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    use crate::components::option_picker::{OptionPicker, PickerProps};

    let items = model_picker_items(settings, &state.read(cx).qt_openai_models);
    let selected: Option<SharedString> =
        (!settings.model.is_empty()).then(|| SharedString::from(settings.model.clone()));

    if state.read(cx).qt_model_picker.is_none() {
        let on_change_state = state.clone();
        let picker = cx.new(|cx| {
            OptionPicker::new(
                PickerProps {
                    id: "qt-model-picker".into(),
                    items: items.clone(),
                    selected: selected.clone(),
                    chips_up_to: CHIPS_UP_TO,
                    placeholder: Some("Select model…".into()),
                    on_change: std::sync::Arc::new(move |id, _window, cx| {
                        on_change_state.update(cx, |st, cx| {
                            st.quick_translate.model = id.to_string();
                            st.save_settings(cx);
                            st.reload_translator(cx);
                        });
                    }),
                },
                window,
                cx,
            )
        });
        state.update(cx, |st, _| st.qt_model_picker = Some(picker));
    }

    let picker = state.read(cx).qt_model_picker.clone().unwrap();
    picker.update(cx, |p, cx| {
        p.set_selection(items, selected, window, cx);
    });

    labeled_row("Model", picker.into_any_element())
}

/// The model catalog for the active provider, with a "Custom: …" item
/// appended when the current model isn't in it (mirrors the popup).
/// For OpenAI-compatible, a loaded /models list replaces the hardcoded
/// catalog.
fn model_picker_items(
    settings: &mdict_rs::settings::QuickTranslateSettings,
    loaded: &[String],
) -> Vec<crate::components::option_picker::PickerItem> {
    use crate::components::option_picker::PickerItem;

    // Catalog comes from the /models load only — no hardcoded list.
    let mut items: Vec<PickerItem> = loaded
        .iter()
        .map(|m| PickerItem::new(m.clone(), m.clone()))
        .collect();
    let current = settings.model.as_str();
    if !current.is_empty() && !items.iter().any(|i| i.id.as_ref() == current) {
        items.push(PickerItem::new(
            current.to_string(),
            format!("Custom: {current}"),
        ));
    }
    items
}

/// The published voices of the current TTS model, from the /models load.
/// Empty when nothing was loaded or the model publishes no voice list.
fn tts_voices_for(
    settings: &mdict_rs::settings::QuickTranslateSettings,
    state: &Entity<DictState>,
    cx: &gpui::App,
) -> Vec<String> {
    let current = settings.tts.model.as_str();
    state
        .read(cx)
        .qt_tts_models
        .iter()
        .find(|m| m.id == current)
        .map(|m| m.voices.clone())
        .unwrap_or_default()
}

/// TTS voice picker over the current model's published voices (same
/// adaptive [`OptionPicker`] as the other rows). Commits the voice id and
/// invalidates stale playing clips.
fn tts_voice_picker_row(
    settings: &mdict_rs::settings::QuickTranslateSettings,
    state: &Entity<DictState>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    use crate::components::option_picker::{OptionPicker, PickerItem, PickerProps};

    // Voices come only from the API (`supported_voices` of the selected
    // model) — nothing hardcoded. A model that publishes none shows an
    // empty picker; the current value stays reachable as "Custom: …".
    let voices = tts_voices_for(settings, state, cx);
    let mut items: Vec<PickerItem> = voices
        .iter()
        .map(|v| PickerItem::new(v.clone(), v.clone()))
        .collect();
    let current = settings.tts.voice.as_str();
    if !current.is_empty() && !items.iter().any(|i| i.id.as_ref() == current) {
        items.push(PickerItem::new(
            current.to_string(),
            format!("Custom: {current}"),
        ));
    }
    let selected: Option<SharedString> = (!current.is_empty()).then(|| SharedString::from(current));

    if state.read(cx).qt_tts_voice_picker.is_none() {
        let on_change_state = state.clone();
        let picker = cx.new(|cx| {
            OptionPicker::new(
                PickerProps {
                    id: "qt-tts-voice-picker".into(),
                    items: items.clone(),
                    selected: selected.clone(),
                    chips_up_to: CHIPS_UP_TO,
                    placeholder: Some("Select voice…".into()),
                    on_change: std::sync::Arc::new(move |id, _window, cx| {
                        on_change_state.update(cx, |st, cx| {
                            st.quick_translate.tts.voice = id.to_string();
                            st.save_settings(cx);
                            st.invalidate_tts_clips_for_settings(None);
                        });
                    }),
                },
                window,
                cx,
            )
        });
        state.update(cx, |st, _| st.qt_tts_voice_picker = Some(picker));
    }

    let picker = state.read(cx).qt_tts_voice_picker.clone().unwrap();
    picker.update(cx, |p, cx| {
        p.set_selection(items, selected, window, cx);
    });

    labeled_row("TTS Voice", picker.into_any_element())
}

/// TTS model picker built on the same adaptive [`OptionPicker`] as the
/// popup's Options panel. Catalog: a TTS-filtered /models load when
/// available, else the hardcoded OpenAI TTS fallback list — plus a
/// "Custom: …" item so a hand-typed model stays visible. The picker entity
/// persists on `DictState` and is reconciled from the live settings.
fn tts_model_picker_row(
    settings: &mdict_rs::settings::QuickTranslateSettings,
    state: &Entity<DictState>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    use crate::components::option_picker::{OptionPicker, PickerItem, PickerProps};

    let loaded = state.read(cx).qt_tts_models.clone();
    // Catalog: the TTS-filtered /models load when it found models, else
    // the curated fallback (endpoints like OpenRouter serve TTS without
    // listing it in /models).
    let mut items: Vec<PickerItem> = if !loaded.is_empty() {
        loaded
            .iter()
            .map(|m| PickerItem::new(m.id.clone(), m.id.clone()))
            .collect()
    } else {
        crate::components::qt_catalog::OPENAI_TTS_MODELS
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
    let selected: Option<SharedString> = (!current.is_empty()).then(|| SharedString::from(current));

    if state.read(cx).qt_tts_model_picker.is_none() {
        let on_change_state = state.clone();
        let picker = cx.new(|cx| {
            OptionPicker::new(
                PickerProps {
                    id: "qt-tts-model-picker".into(),
                    items: items.clone(),
                    selected: selected.clone(),
                    chips_up_to: CHIPS_UP_TO,
                    placeholder: Some("Select TTS model…".into()),
                    on_change: std::sync::Arc::new(move |id, _window, cx| {
                        on_change_state.update(cx, |st, cx| {
                            st.quick_translate.tts.model = id.to_string();
                            // A model without a published voice list takes
                            // its default voice — drop the stale one.
                            let voiceless = st
                                .qt_tts_models
                                .iter()
                                .find(|m| m.id == id)
                                .map(|m| m.voices.is_empty())
                                .unwrap_or(false);
                            if voiceless {
                                st.quick_translate.tts.voice = String::new();
                            }
                            st.save_settings(cx);
                        });
                    }),
                },
                window,
                cx,
            )
        });
        state.update(cx, |st, _| st.qt_tts_model_picker = Some(picker));
    }

    let picker = state.read(cx).qt_tts_model_picker.clone().unwrap();
    picker.update(cx, |p, cx| {
        p.set_selection(items, selected, window, cx);
    });

    labeled_row("TTS Model", picker.into_any_element())
}

/// Target language picker built on the same adaptive [`OptionPicker`] as
/// the popup's Options panel: the 13-language catalog collapses into a
/// dropdown, and a hand-edited language stays visible as "Custom: …".
/// The picker entity persists on `DictState` and is reconciled from the
/// live settings on every render.
fn target_lang_picker_row(
    settings: &mdict_rs::settings::QuickTranslateSettings,
    state: &Entity<DictState>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    use crate::components::option_picker::{OptionPicker, PickerItem, PickerProps};
    use crate::components::qt_catalog;

    let items: Vec<PickerItem> = {
        let mut items: Vec<PickerItem> = qt_catalog::TARGET_LANGS
            .iter()
            .map(|&lang| PickerItem::new(lang, lang))
            .collect();
        let current = settings.target_lang.as_str();
        if !current.is_empty() && !items.iter().any(|i| i.id.as_ref() == current) {
            items.push(PickerItem::new(
                current.to_string(),
                format!("Custom: {current}"),
            ));
        }
        items
    };
    let selected: Option<SharedString> = (!settings.target_lang.is_empty())
        .then(|| SharedString::from(settings.target_lang.clone()));

    if state.read(cx).qt_target_lang_picker.is_none() {
        let on_change_state = state.clone();
        let picker = cx.new(|cx| {
            OptionPicker::new(
                PickerProps {
                    id: "qt-target-lang-picker".into(),
                    items: items.clone(),
                    selected: selected.clone(),
                    chips_up_to: CHIPS_UP_TO,
                    placeholder: Some("Select language…".into()),
                    on_change: std::sync::Arc::new(move |id, _window, cx| {
                        on_change_state.update(cx, |st, cx| {
                            st.quick_translate.target_lang = id.to_string();
                            st.save_settings(cx);
                        });
                    }),
                },
                window,
                cx,
            )
        });
        state.update(cx, |st, _| st.qt_target_lang_picker = Some(picker));
    }

    let picker = state.read(cx).qt_target_lang_picker.clone().unwrap();
    picker.update(cx, |p, cx| {
        p.set_selection(items, selected, window, cx);
    });

    labeled_row("Target Language", picker.into_any_element())
}
