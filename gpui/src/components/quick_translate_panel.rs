//! Quick Translate settings tab.
//!
//! Allows the user to configure the quick-translate feature: enable/disable,
//! hotkey, LLM provider, API key, model, target language.

use crate::{
    colors,
    components::qt_catalog,
    state::{DictState, QtKeyTest},
};
use gpui::{
    AppContext as _, AsyncApp, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement,
    v_flex,
};

/// Build the Quick Translate settings tab content.
pub fn quick_translate_tab_content(
    state: Entity<DictState>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    let settings = state.read(cx).quick_translate.clone();

    // Lazily create persistent InputState entities for each editable field on
    // first render, seeded from loaded settings. They persist on DictState so
    // focus and cursor survive re-renders. We subscribe once so typing writes
    // straight back to settings.
    ensure_qt_inputs(&state, &settings, window, cx);

    // Fill the model pickers in the background: on first open and again
    // whenever the endpoint or key changes. The buttons below stay as an
    // on-demand refresh.
    schedule_models_autoload(&state, cx);

    let header = v_flex()
        .gap(px(4.))
        .pb(px(16.))
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::text())
                .child(SharedString::from("Quick Translate")),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(
                    "Translate selected text anywhere on your screen with a global hotkey.",
                )),
        );

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
        "Test translation",
        "Testing…",
        state.read(cx).qt_translation_test.clone(),
        true,
        move |cx| run_translation_test(translation_test_state.clone(), cx),
    );

    // Model picker — same adaptive picker (chips/dropdown) as the popup's
    // Options panel, reconciled from live settings each render.
    let model_row = model_picker_row(&settings, &state, window, cx);

    // Free-tier usage from the last key probe. Providers without a /key
    // endpoint leave this empty.
    let quota_note = state
        .read(cx)
        .qt_key_info
        .as_ref()
        .and_then(|info| info.free_model_daily_requests)
        .map(|daily| {
            div()
                .pb(px(6.))
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(format!(
                    "Free models today: {} of {} requests left, shared by translation and voice.",
                    daily.remaining, daily.limit
                )))
        });

    // "Refresh models" row: re-fetches the live model list from
    // {base_url}/models. The automatic load usually beats the user to it;
    // the button retries a failed attempt on demand.
    let models_load_state = state.clone();
    let models_load_row = action_status_row(
        "qt-models-load",
        "Refresh models",
        "Loading…",
        state.read(cx).qt_models_load.clone(),
        false,
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
                .text_size(px(14.))
                .text_color(colors::text())
                .child(SharedString::from("Use AI Text-to-Speech")),
        )
        .child(toggle_switch("qt-tts-enable", tts_enabled, move |cx| {
            tts_toggle_state.update(cx, |s, cx| {
                s.quick_translate.tts.enabled = !tts_enabled;
                s.save_settings(cx);
            });
            // Turning AI voices on brings the TTS model list into play.
            schedule_models_autoload(&tts_toggle_state, cx);
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
        "Refresh TTS models",
        "Loading…",
        state.read(cx).qt_tts_models_load.clone(),
        false,
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
        "Test voice",
        "Testing…",
        state.read(cx).qt_tts_test.clone(),
        true,
        move |cx| run_tts_test(tts_test_state.clone(), cx),
    );

    let tts_note = div()
        .text_size(px(12.))
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
                        .text_size(px(13.))
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
        // Same right padding as the General tab: the scrollbar thumb lives in
        // the wrapper's right 16px and would sit on the toggles.
        .p(px(4.))
        .pr(px(16.))
        .pb(px(16.))
        .id("qt-settings-scroll")
        .overflow_y_scrollbar();
    body = body.child(header);
    body = body.child(divider());

    // Nothing is configured yet: the guided card is the whole tab. It owns
    // the key, the endpoint, the model, and the voice until it is done, so
    // the raw form cannot be the first thing a new user meets.
    if !state.read(cx).qt_setup_done {
        body = body.child(crate::components::qt_setup::setup_card(
            state.clone(),
            window,
            cx,
        ));
        return div()
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .child(body)
            .into_any_element();
    }

    body = body.child(section_title("Translation"));
    body = body.child(base_url_row);
    body = body.child(api_key_row);
    body = body.child(model_row);
    if let Some(note) = quota_note {
        body = body.child(note);
    }
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
    body = body.child(crate::components::qt_setup::run_setup_again(state.clone()));

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
        .mt(px(14.))
        .mb(px(4.))
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::text_secondary())
        .child(SharedString::from(text.to_uppercase()))
        .into_any_element()
}

pub(crate) fn divider() -> gpui::AnyElement {
    div().h(px(1.)).bg(colors::border()).into_any_element()
}

/// Row with an action button plus the live result of the last run —
/// shared by the key [Test] buttons and the "Refresh models" actions.
/// Placed directly under a related field so failures are caught at
/// configuration time instead of on first real use.
fn action_status_row(
    id: &'static str,
    idle_label: &str,
    running_label: &str,
    status: QtKeyTest,
    ok_is_success: bool,
    on_click: impl Fn(&mut gpui::App) + 'static,
) -> gpui::AnyElement {
    // `ok_is_success` separates "what you entered works" (green check, the
    // Test buttons) from "a list arrived in the background" (neutral
    // text), so green always means the user's own action succeeded.
    let (label, msg, color) = match &status {
        QtKeyTest::Idle => (idle_label.to_string(), None, colors::text_secondary()),
        QtKeyTest::Running => (running_label.to_string(), None, colors::text_secondary()),
        QtKeyTest::Ok(text) if ok_is_success => (
            idle_label.to_string(),
            Some(format!("✓ {text}")),
            colors::success(),
        ),
        QtKeyTest::Ok(text) => (
            idle_label.to_string(),
            Some(text.clone()),
            colors::text_secondary(),
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
                .flex()
                .items_center()
                .h(px(crate::components::option_picker::CONTROL_H))
                .cursor_pointer()
                .px(px(10.))
                .rounded(px(6.))
                .line_height(px(16.))
                .text_size(px(13.))
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
                .text_size(px(12.))
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

/// How long the automatic load waits for the endpoint fields to settle,
/// so typing a URL or pasting a key costs one request instead of one per
/// keystroke.
const MODELS_AUTOLOAD_DELAY: std::time::Duration = std::time::Duration::from_millis(1000);

/// Fill the model pickers without a button press.
///
/// Runs on every render of the tab but does work only when the endpoint
/// and key a list was last loaded for no longer match the settings: the
/// first open, switching providers, or pasting the key after a keyless
/// request was refused. The wait above lets a burst of typing settle, and
/// each scheduled run re-checks the live settings before it fires, so only
/// the last edit of a burst reaches the network.
fn schedule_models_autoload(state: &Entity<DictState>, cx: &mut gpui::App) {
    let settings = state.read(cx).quick_translate.clone();
    let text_key = qt_catalog::load_key(&settings.api_base_url, &settings.api_key);
    let voice_key = qt_catalog::load_key(&settings.tts.api_base_url, &settings.tts.api_key);

    let text_wanted = !settings.api_base_url.trim().is_empty()
        && state.read(cx).qt_models_loaded_for.as_ref() != Some(&text_key);
    // The TTS section is only on screen while AI TTS is on — loading its
    // catalog when the user has not asked for AI voices would just add a
    // failing request against an unconfigured endpoint.
    let voice_wanted = settings.tts.enabled
        && !settings.tts.api_base_url.trim().is_empty()
        && state.read(cx).qt_tts_models_loaded_for.as_ref() != Some(&voice_key);
    if !text_wanted && !voice_wanted {
        return;
    }

    // Reserve the targets before the wait. Every render in that second
    // would otherwise schedule another timer, and each of them would find
    // the settings unchanged and fire its own request.
    state.update(cx, |s, _| {
        if text_wanted {
            s.qt_models_loaded_for = Some(text_key.clone());
        }
        if voice_wanted {
            s.qt_tts_models_loaded_for = Some(voice_key.clone());
        }
    });

    let text_target = text_wanted.then_some(text_key);
    let voice_target = voice_wanted.then_some(voice_key);
    let state = state.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor().timer(MODELS_AUTOLOAD_DELAY).await;
        cx.update(|cx| {
            let (text_now, voice_now) = {
                let s = state.read(cx);
                (
                    qt_catalog::load_key(
                        &s.quick_translate.api_base_url,
                        &s.quick_translate.api_key,
                    ),
                    qt_catalog::load_key(
                        &s.quick_translate.tts.api_base_url,
                        &s.quick_translate.tts.api_key,
                    ),
                )
            };
            // A later keystroke scheduled its own run; this one is stale.
            if text_target.as_ref() == Some(&text_now) {
                run_models_load(state.clone(), cx);
            }
            if voice_target.as_ref() == Some(&voice_now) {
                run_tts_models_load(state.clone(), cx);
            }
        });
    })
    .detach();
}

/// Fetch the model list from the configured OpenAI-compatible endpoint
/// (`GET {base_url}/models`) and cache it in `qt_openai_models`; the
/// model picker rebuilds from it on the next render. The hardcoded
/// catalog stays the fallback until a load succeeds.
///
/// Called by "Refresh models" and by [`schedule_models_autoload`]. The
/// endpoint + key are recorded when the request starts, so a failed
/// attempt is not retried in a loop and the auto loader knows the list is
/// current.
fn run_models_load(state: Entity<DictState>, cx: &mut gpui::App) {
    let settings = state.read(cx).quick_translate.clone();
    let base = settings.api_base_url.trim().to_string();
    if base.is_empty() {
        state.update(cx, |s, cx| {
            s.qt_models_load = QtKeyTest::Err("API base URL is empty".into());
            cx.notify();
        });
        return;
    }
    // /models is public on several endpoints (e.g. OpenRouter) — the key
    // is sent only when present.
    let key = settings.api_key;
    let loaded_for = qt_catalog::load_key(&base, &key);

    state.update(cx, |s, cx| {
        s.qt_models_loaded_for = Some(loaded_for);
        s.qt_models_load = QtKeyTest::Running;
        cx.notify();
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = cx
            .background_executor()
            .spawn({
                let (key, base) = (key.clone(), base.clone());
                async move { dicto_translate::openai::list_models(&key, &base) }
            })
            .await;

        cx.update(|cx| {
            cx.update_entity(&state, |s, cx| {
                // Another load may have started for a different endpoint or
                // key while this one was in flight; its result is the one
                // that matches the settings.
                if s.quick_translate.api_base_url.trim() != base || s.quick_translate.api_key != key
                {
                    return;
                }
                s.qt_models_load = match result {
                    Ok(models) if models.is_empty() => {
                        QtKeyTest::Err("Endpoint's model list is empty".into())
                    }
                    Ok(models) => {
                        s.qt_openai_models = models;
                        QtKeyTest::Ok("Model list updated".into())
                    }
                    Err(e) => QtKeyTest::Err(shorten(&e.to_string(), 96)),
                };
                cx.notify();
            });
        });
    })
    .detach();
}

/// Fetch the TTS-capable model list from the configured OpenAI-compatible
/// endpoint and cache it in `qt_tts_models`; the TTS model picker rebuilds
/// from it on the next render. Same automatic/manual split as
/// [`run_models_load`].
fn run_tts_models_load(state: Entity<DictState>, cx: &mut gpui::App) {
    let tts = state.read(cx).quick_translate.tts.clone();
    let base = tts.api_base_url.trim().to_string();
    if base.is_empty() {
        state.update(cx, |s, cx| {
            s.qt_tts_models_load = QtKeyTest::Err("TTS base URL is empty".into());
            cx.notify();
        });
        return;
    }
    // /models is public on several endpoints (e.g. OpenRouter) — the key
    // is sent only when present.
    let key = tts.api_key;
    let loaded_for = qt_catalog::load_key(&base, &key);

    state.update(cx, |s, cx| {
        s.qt_tts_models_loaded_for = Some(loaded_for);
        s.qt_tts_models_load = QtKeyTest::Running;
        cx.notify();
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = cx
            .background_executor()
            .spawn({
                let (key, base) = (key.clone(), base.clone());
                async move { dicto_translate::openai::list_tts_models(&key, &base) }
            })
            .await;

        cx.update(|cx| {
            cx.update_entity(&state, |s, cx| {
                if s.quick_translate.tts.api_base_url.trim() != base
                    || s.quick_translate.tts.api_key != key
                {
                    return;
                }
                s.qt_tts_models_load = match result {
                    Ok(models) if models.is_empty() => {
                        // Some endpoints (OpenRouter) serve TTS models
                        // without listing them in /models — keep the
                        // fallback list and say so, instead of a bare
                        // "updated" that would imply the list is real.
                        QtKeyTest::Err(
                            "Endpoint's model list has no TTS models — use the fallback list"
                                .into(),
                        )
                    }
                    Ok(models) => {
                        s.qt_tts_models = models;
                        QtKeyTest::Ok("TTS model list updated".into())
                    }
                    Err(e) => QtKeyTest::Err(shorten(&e.to_string(), 96)),
                };
                cx.notify();
            });
        });
    })
    .detach();
}

/// Model to save after a key probe, when the key turns out to have no credit
/// and the stored model is paid.
///
/// `None` means "keep the stored model": either the key can pay, or the model
/// is already free. A paid model without credit answers 402, so the free
/// fallback is chosen before the user hits that error.
fn free_model_after_probe(
    base_url: &str,
    current: &str,
    loaded: &[String],
    has_credit: bool,
) -> Option<String> {
    use mdict_rs::settings::free;
    if has_credit || free::is_free(current) {
        return None;
    }
    free::pick_text_model(true, false, loaded).or_else(|| {
        free::is_openrouter(base_url).then(|| free::default_text_model_for(base_url).to_string())
    })
}

/// Same rule for the voice model: keep a free model, otherwise take the
/// best free one the endpoint offers.
fn free_tts_model_after_probe(
    current: &str,
    loaded: &[String],
    has_credit: bool,
) -> Option<String> {
    use mdict_rs::settings::free;
    if has_credit || free::is_free(current) {
        return None;
    }
    free::pick_tts_model(true, false, loaded)
}

/// Send one short sample through the configured translation provider and
/// record the outcome in `qt_translation_test`. Credentials are tested
/// regardless of the feature's enable toggle.
pub(crate) fn run_translation_test(state: Entity<DictState>, cx: &mut gpui::App) {
    use dicto_translate::TranslationRequest;

    let settings = state.read(cx).quick_translate.clone();
    // A local server needs no key; every hosted provider does.
    if settings.api_key.is_empty()
        && !mdict_rs::settings::providers::is_local_url(&settings.api_base_url)
    {
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
        let (probe_key, probe_base) = (settings.api_key.clone(), settings.api_base_url.clone());
        // Credit and free-tier usage, when the provider exposes them. A
        // missing /key endpoint is not an error — the test still counts.
        let key_info =
            cx.background_executor()
                .spawn(async move {
                    dicto_translate::openai::fetch_key_info(&probe_key, &probe_base).ok()
                })
                .await;

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

        cx.update(|cx| {
            cx.update_entity(&state, |s, cx| {
                // A key without credit cannot use a paid model, so switch to
                // the best free one before the user sees a 402.
                let switched = key_info.as_ref().and_then(|info| {
                    free_model_after_probe(
                        &s.quick_translate.api_base_url,
                        &s.quick_translate.model,
                        &s.qt_openai_models,
                        info.has_credit(),
                    )
                });
                if let Some(model) = &switched {
                    s.quick_translate.model = model.clone();
                    s.save_settings(cx);
                    s.reload_translator(cx);
                }

                let status = match (&result, &switched) {
                    (Ok(r), Some(model)) => QtKeyTest::Ok(format!(
                        "Works — {} · no credit on this key, using free model {model}",
                        shorten(&r.translated_text, 40)
                    )),
                    (Ok(r), None) => {
                        QtKeyTest::Ok(format!("Works — {}", shorten(&r.translated_text, 48)))
                    }
                    (Err(e), Some(model)) => QtKeyTest::Err(format!(
                        "{} · no credit on this key, switched to free model {model}; test again",
                        shorten(&e.to_string(), 64)
                    )),
                    (Err(e), None) => QtKeyTest::Err(shorten(&e.to_string(), 96)),
                };
                s.qt_translation_test = status;
                // The guided card steps forward on the first passing test
                // and stays there; later model changes must not push the
                // user back to the key field.
                if matches!(s.qt_translation_test, QtKeyTest::Ok(_)) {
                    s.qt_setup.connected = true;
                }
                if key_info.is_some() {
                    s.qt_key_info = key_info;
                }
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
        // Same credit probe as the translation test. The TTS key may be a
        // different one, so it is probed separately; an empty key skips it.
        let key_info = if tts.api_key.is_empty() {
            None
        } else {
            let (key, base) = (tts.api_key.clone(), tts.api_base_url.clone());
            cx.background_executor()
                .spawn(async move { dicto_translate::openai::fetch_key_info(&key, &base).ok() })
                .await
        };

        let result = cx
            .background_executor()
            .spawn(async move { crate::tts::test_synthesis(&tts) })
            .await;

        cx.update(|cx| {
            cx.update_entity(&state, |s, cx| {
                let switched = key_info.as_ref().and_then(|info| {
                    let loaded: Vec<String> =
                        s.qt_tts_models.iter().map(|m| m.id.clone()).collect();
                    free_tts_model_after_probe(
                        &s.quick_translate.tts.model,
                        &loaded,
                        info.has_credit(),
                    )
                });
                if let Some(model) = &switched {
                    let voiceless = s
                        .qt_tts_models
                        .iter()
                        .find(|m| &m.id == model)
                        .map(|m| m.voices.is_empty())
                        .unwrap_or(false);
                    s.quick_translate.tts.model = model.clone();
                    if voiceless {
                        s.quick_translate.tts.voice = String::new();
                    }
                    s.save_settings(cx);
                    s.invalidate_tts_clips_for_settings(None);
                }

                let status = match (&result, &switched) {
                    (Ok(len), Some(model)) => QtKeyTest::Ok(format!(
                        "Works — {len} bytes · no credit on this key, using free model {model}"
                    )),
                    (Ok(len), None) => QtKeyTest::Ok(format!("Works — {len} bytes of audio")),
                    (Err(e), Some(model)) => QtKeyTest::Err(format!(
                        "{} · no credit on this key, switched to free model {model}; test again",
                        shorten(&e.to_string(), 64)
                    )),
                    (Err(e), None) => QtKeyTest::Err(shorten(&e.to_string(), 96)),
                };
                s.qt_tts_test = status;
                if key_info.is_some() && s.quick_translate.tts.api_key == s.quick_translate.api_key
                {
                    s.qt_key_info = key_info;
                }
                cx.notify();
            });
        });
    })
    .detach();
}

pub(crate) fn toggle_switch(
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
                let old_key = st.quick_translate.api_key.clone();
                st.quick_translate.api_key = v;
                // One key covers translation and voice by default: while the
                // voice still follows the shared key, it follows this edit
                // too. A key typed into the TTS section stops that.
                mdict_rs::settings::providers::sync_shared_key(&mut st.quick_translate, &old_key);
                // Credit and quota belonged to the previous key, and so
                // did any note about a model it switched.
                st.qt_key_info = None;
                st.qt_free_note = None;
                st.qt_translation_test = QtKeyTest::Idle;
                st.save_settings(cx);
                st.reload_translator(cx);
            });
            // A keyed endpoint lists models only once a key is present.
            schedule_models_autoload(&s, cx);
        }
    };
    let base_url_cb = {
        let s = state.clone();
        move |v: String, cx: &mut gpui::App| {
            s.update(cx, |st, cx| {
                let old_base = st.quick_translate.api_base_url.clone();
                st.quick_translate.api_base_url = v;
                // Same shared-by-default rule for the endpoint: a voice
                // endpoint of its own is left alone.
                mdict_rs::settings::providers::sync_shared_base_url(
                    &mut st.quick_translate,
                    &old_base,
                );
                // A different endpoint reports its own credit and quota,
                // and lists its own models.
                st.qt_key_info = None;
                st.qt_free_note = None;
                st.qt_translation_test = QtKeyTest::Idle;
                st.save_settings(cx);
                st.reload_translator(cx);
            });
            // A new endpoint publishes a new model list.
            schedule_models_autoload(&s, cx);
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
                st.qt_free_note_tts = None;
                st.qt_tts_test = QtKeyTest::Idle;
                st.save_settings(cx);
            });
            schedule_models_autoload(&s, cx);
        }
    };
    let tts_base_url_cb = {
        let s = state.clone();
        move |v: String, cx: &mut gpui::App| {
            s.update(cx, |st, cx| {
                st.quick_translate.tts.api_base_url = v;
                st.qt_free_note_tts = None;
                st.qt_tts_test = QtKeyTest::Idle;
                st.save_settings(cx);
            });
            schedule_models_autoload(&s, cx);
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
pub(crate) fn input_row(
    label: &str,
    input: &Entity<InputState>,
    masked: bool,
    _cx: &mut gpui::App,
) -> gpui::AnyElement {
    use gpui_component::Sizable as _;

    // 13px text in a control-height box: the inner editor keeps the
    // library's small box and is centered inside the wrapper, which is what
    // actually sets the field's height (`.h()` on `Input` itself is ignored
    // — its root is `size_full`).
    let mut el = Input::new(input)
        .appearance(true)
        .small()
        .w_full()
        .text_size(px(14.))
        .bg(colors::surface())
        .min_h(px(crate::components::option_picker::CONTROL_H))
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
                .text_size(px(13.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(label)),
        )
        // Flexible: the field grows with the panel instead of leaving the
        // right half of the row empty.
        .child(div().flex_1().min_w(px(0.)).child(el))
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
                .text_size(px(13.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(label)),
        )
        .child(div().flex_1().min_w(px(0.)).child(content))
        .into_any_element()
}

/// Two-state filter for one model picker: "Free only" | "All".
///
/// It sits in the same row as the picker it narrows, so there is no doubt
/// which list it applies to: the translation picker and the voice picker
/// each get their own, and turning one on never hides the other's models.
///
/// The bordered group is exactly as tall as the picker beside it
/// ([`CONTROL_H`]); its segments fill the inside instead of sizing
/// themselves, so the group can never end up taller or shorter than the
/// control it belongs to.
pub(crate) fn free_filter_segments(
    id: &'static str,
    free_only: bool,
    on_click: impl Fn(bool, &mut gpui::App) + 'static,
) -> gpui::AnyElement {
    use crate::components::option_picker::CONTROL_H;

    let on_click = std::rc::Rc::new(on_click);
    let segment = |value: bool, label: &'static str| {
        let active = free_only == value;
        let on_click = on_click.clone();
        let segment = h_flex()
            .id(SharedString::from(format!(
                "{id}-{}",
                if value { "free" } else { "all" }
            )))
            .h_full()
            .px(px(8.))
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .line_height(px(16.))
            .text_size(px(12.))
            .child(SharedString::from(label));
        // The active half is filled, the other only reacts to hover.
        let segment = if active {
            segment.bg(colors::primary()).text_color(colors::bg())
        } else {
            segment
                .text_color(colors::text_secondary())
                .hover(|s| s.bg(colors::hover()))
        };
        segment
            .on_click(move |_, _, cx| on_click(value, cx))
            .into_any_element()
    };

    h_flex()
        .items_center()
        .h(px(CONTROL_H))
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(6.))
        .border_1()
        .border_color(colors::border())
        .bg(colors::surface())
        .child(segment(true, "Free only"))
        .child(segment(false, "All"))
        .into_any_element()
}

/// Small secondary line under a picker: counts, or the note left by a
/// filter that had to move the selection.
pub(crate) fn hint_line(text: String, color: gpui::Hsla) -> gpui::AnyElement {
    div()
        .text_size(px(12.))
        .text_color(color)
        .child(SharedString::from(text))
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
pub(crate) fn model_picker_row(
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
                            st.qt_free_note = None;
                            st.qt_translation_test = QtKeyTest::Idle;
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

    let free_state = state.clone();
    let free_on = settings.free_only;
    let filter = free_filter_segments("qt-free-text", free_on, move |want, cx| {
        free_state.update(cx, |s, cx| {
            // Clicking the half that is already on changes nothing.
            if s.quick_translate.free_only == want {
                return;
            }
            s.quick_translate.free_only = want;
            s.qt_free_note = None;
            if want {
                let previous = s.quick_translate.model.clone();
                if let Some(model) = free_model_after_probe(
                    &s.quick_translate.api_base_url,
                    &previous,
                    &s.qt_openai_models,
                    false,
                ) {
                    s.quick_translate.model = model.clone();
                    s.reload_translator(cx);
                    // Say so: the picker just changed the user's choice.
                    s.qt_free_note = Some(format!("Switched from {previous} to {model} (free)"));
                }
            }
            s.save_settings(cx);
        });
    });

    let loaded = state.read(cx).qt_openai_models.clone();
    let free_n = loaded
        .iter()
        .filter(|m| mdict_rs::settings::free::is_free(m))
        .count();
    let hint = qt_catalog::free_hint(free_on, loaded.len(), free_n);
    let note = state.read(cx).qt_free_note.clone();

    labeled_row(
        "Model",
        v_flex()
            .gap(px(3.))
            .child(
                h_flex()
                    .items_start()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(picker.into_any_element()),
                    )
                    .child(filter),
            )
            .children(note.map(|n| hint_line(n, colors::update())))
            .children(hint.map(|h| hint_line(h, colors::text_secondary())))
            .into_any_element(),
    )
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

    // Catalog comes from the /models load only — no hardcoded list. The
    // "Free models only" switch trims it to `:free` variants when the
    // endpoint publishes any.
    let visible = qt_catalog::filter_free(loaded, settings.free_only);
    let mut items: Vec<PickerItem> = visible
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
pub(crate) fn tts_voices_for(
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
pub(crate) fn tts_voice_picker_row(
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
                            // The checked clip used the old voice.
                            st.qt_tts_test = QtKeyTest::Idle;
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
pub(crate) fn tts_model_picker_row(
    settings: &mdict_rs::settings::QuickTranslateSettings,
    state: &Entity<DictState>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    use crate::components::option_picker::{OptionPicker, PickerItem, PickerProps};

    let loaded = state.read(cx).qt_tts_models.clone();
    // Catalog: the TTS-filtered /models load when it found models, else
    // the curated fallback (endpoints like OpenRouter serve TTS without
    // listing it in /models). Both pass through the free-only switch.
    let catalog: Vec<String> = if !loaded.is_empty() {
        loaded.iter().map(|m| m.id.clone()).collect()
    } else {
        crate::components::qt_catalog::OPENAI_TTS_MODELS
            .iter()
            .map(|&id| id.to_string())
            .collect()
    };
    let visible = qt_catalog::filter_free(&catalog, settings.tts.free_only);
    let mut items: Vec<PickerItem> = visible
        .into_iter()
        .map(|id| PickerItem::new(id.clone(), id))
        .collect();
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
                            st.qt_free_note_tts = None;
                            st.qt_tts_test = QtKeyTest::Idle;
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

    let free_state = state.clone();
    let free_on = settings.tts.free_only;
    let filter = free_filter_segments("qt-free-tts", free_on, move |want, cx| {
        free_state.update(cx, |s, cx| {
            if s.quick_translate.tts.free_only == want {
                return;
            }
            s.quick_translate.tts.free_only = want;
            s.qt_free_note_tts = None;
            let models: Vec<String> = s.qt_tts_models.iter().map(|m| m.id.clone()).collect();
            if want {
                let previous = s.quick_translate.tts.model.clone();
                if let Some(model) = free_tts_model_after_probe(&previous, &models, false) {
                    let voiceless = s
                        .qt_tts_models
                        .iter()
                        .find(|m| m.id == model)
                        .map(|m| m.voices.is_empty())
                        .unwrap_or(false);
                    s.quick_translate.tts.model = model.clone();
                    if voiceless {
                        s.quick_translate.tts.voice = String::new();
                    }
                    s.invalidate_tts_clips_for_settings(None);
                    s.qt_free_note_tts =
                        Some(format!("Switched from {previous} to {model} (free)"));
                }
            }
            s.save_settings(cx);
        });
    });

    let free_n = catalog
        .iter()
        .filter(|id| mdict_rs::settings::free::is_free(id))
        .count();
    let hint = qt_catalog::free_hint(free_on, catalog.len(), free_n);
    let note = state.read(cx).qt_free_note_tts.clone();

    labeled_row(
        "TTS Model",
        v_flex()
            .gap(px(3.))
            .child(
                h_flex()
                    .items_start()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(picker.into_any_element()),
                    )
                    .child(filter),
            )
            .children(note.map(|n| hint_line(n, colors::update())))
            .children(hint.map(|h| hint_line(h, colors::text_secondary())))
            .into_any_element(),
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn paid_model_switches_to_free_when_the_key_has_no_credit() {
        let loaded = ids(&["google/gemma-4-31b-it:free", "google/gemini-3.8-flash-lite"]);
        assert_eq!(
            free_model_after_probe(
                "https://openrouter.ai/api/v1",
                "google/gemini-3.8-flash-lite",
                &loaded,
                false
            )
            .as_deref(),
            Some("google/gemma-4-31b-it:free")
        );
    }

    #[test]
    fn a_key_with_credit_keeps_its_paid_model() {
        let loaded = ids(&["google/gemma-4-31b-it:free"]);
        assert_eq!(
            free_model_after_probe(
                "https://openrouter.ai/api/v1",
                "google/gemini-3.8-flash-lite",
                &loaded,
                true
            ),
            None
        );
    }

    #[test]
    fn a_free_model_is_left_alone() {
        let loaded = ids(&["google/gemma-4-31b-it:free"]);
        assert_eq!(
            free_model_after_probe(
                "https://openrouter.ai/api/v1",
                "google/gemma-4-26b-a4b-it:free",
                &loaded,
                false
            ),
            None
        );
    }

    #[test]
    fn openrouter_falls_back_to_the_free_default_before_any_load() {
        assert_eq!(
            free_model_after_probe("https://openrouter.ai/api/v1", "gpt-4o-mini", &[], false)
                .as_deref(),
            Some("google/gemma-4-26b-a4b-it:free")
        );
    }

    #[test]
    fn other_providers_keep_their_model_when_nothing_free_is_loaded() {
        // No /models load yet and not OpenRouter: do not invent a model.
        assert_eq!(
            free_model_after_probe("https://api.openai.com/v1", "gpt-4o-mini", &[], false),
            None
        );
    }

    #[test]
    fn voice_switches_to_the_free_model_only_when_one_is_loaded() {
        let loaded = ids(&["fish-audio/s2.1-pro:free", "fish-audio/s2.1-pro-free:free"]);
        assert_eq!(
            free_tts_model_after_probe("fish-audio/s2.1-pro", &loaded, false).as_deref(),
            Some("fish-audio/s2.1-pro-free:free")
        );
        assert_eq!(
            free_tts_model_after_probe("fish-audio/s2.1-pro-free:free", &loaded, false),
            None
        );
        assert_eq!(
            free_tts_model_after_probe("fish-audio/s2.1-pro", &[], false),
            None
        );
    }
}
