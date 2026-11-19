//! The guided "turn on Quick Translate" card.
//!
//! This is the in-app version of `design/ai-setup.html`: three steps —
//! choose a service, get a key, connect — with the provider list ordered by
//! whether each service actually answers from this network. It exists so a
//! new user never has to know what a base URL is, and so a blocked provider
//! is a labelled row instead of an unexplainable failure later.
//!
//! The card is shown in place of the raw provider form while nothing is
//! configured ([`crate::state::DictState::qt_setup_done`] is false). The raw
//! form stays reachable as "Advanced" inside the card, and again as the whole
//! tab once setup is done — nothing is hidden from an experienced user.
//!
//! Everything the card needs from the world is in three pure helpers —
//! [`order`], [`classify`], [`failure_copy`] — plus the network probe in
//! `dicto_translate::reach`. The rest is layout.

use gpui::{
    AppContext as _, AsyncApp, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::{h_flex, v_flex};
use mdict_rs::settings::providers::{self, PRESETS, ProviderPreset};

use crate::colors;
use crate::components::quick_translate_panel::{
    divider, free_filter_segments, hint_line, input_row, model_picker_row, tts_model_picker_row,
    tts_voice_picker_row,
};
use crate::state::{DictState, QtKeyTest};

/// Which network the provider list should assume.
///
/// `Auto` is the default and the only mode that reorders: the probe in
/// `dicto_translate::reach` asks every service whether it answers from here,
/// and the ones that do move to the top. `Global` and `Iran` are manual
/// overrides for when the probe is wrong (a VPN, a proxy, a corporate
/// network) — they set a fixed order and leave the probe badges visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetMode {
    Auto,
    Global,
    Iran,
}

impl NetMode {
    fn label(self) -> &'static str {
        match self {
            NetMode::Auto => "Auto",
            NetMode::Global => "Global",
            NetMode::Iran => "Iran",
        }
    }
}

/// One step of the guided card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupStep {
    Choose,
    Key,
    Connect,
}

/// Everything the guided card remembers between renders.
#[derive(Debug, Clone)]
pub struct SetupState {
    pub step: SetupStep,
    /// Preset id (`openrouter`, `avalai`, …), not a display name.
    pub provider: Option<&'static str>,
    pub net: NetMode,
    /// Probe result per preset id. `None` means "still checking".
    pub reach: Vec<(&'static str, Option<dicto_translate::reach::Reach>)>,
    pub probing: bool,
    /// The full form is open under the card (base URL and friends).
    pub advanced: bool,
    /// The last Test on the Connect step passed. Sticky: changing a model
    /// afterwards does not send the user back to the key step.
    pub connected: bool,
}

impl Default for SetupState {
    fn default() -> Self {
        Self {
            step: SetupStep::Choose,
            provider: None,
            net: NetMode::Auto,
            reach: Vec::new(),
            probing: false,
            advanced: false,
            connected: false,
        }
    }
}

impl SetupState {
    /// Continue an interrupted setup instead of starting over.
    ///
    /// A base URL that matches a built-in preset means the user already
    /// chose one; with no key yet they belong on the key step, which is
    /// exactly where they left off.
    pub fn detect(base_url: &str, api_key: &str) -> Self {
        let mut setup = Self::default();
        if let Some(preset) = providers::detect(base_url) {
            setup.provider = Some(preset.id);
            setup.step = if api_key.trim().is_empty() {
                SetupStep::Key
            } else {
                SetupStep::Connect
            };
        }
        setup
    }
}

/// The preset with this id, if it is a built-in.
pub fn preset(id: &str) -> Option<&'static ProviderPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// The provider list in the order this mode wants it.
///
/// `Auto` sorts by probe result, best first, and keeps the built-in order
/// for ties — a stable sort, so the list never shuffles between renders
/// while the probe is still running. `Global` and `Iran` are fixed orders:
/// a manual choice should be predictable, not second-guessed.
pub fn order(
    net: NetMode,
    reach: &[(&'static str, Option<dicto_translate::reach::Reach>)],
) -> Vec<&'static ProviderPreset> {
    use dicto_translate::reach::Reach;

    let base: Vec<&'static ProviderPreset> = match net {
        NetMode::Iran => {
            let mut v: Vec<_> = PRESETS.iter().filter(|p| p.id == "avalai").collect();
            v.extend(PRESETS.iter().filter(|p| p.id != "avalai"));
            v
        }
        NetMode::Auto | NetMode::Global => PRESETS.iter().collect(),
    };
    if net != NetMode::Auto {
        return base;
    }

    let rank = |p: &ProviderPreset| match reach.iter().find(|(id, _)| *id == p.id) {
        Some((_, Some(Reach::Reachable))) => 0,
        // Still checking: keep the built-in order rather than guessing.
        Some((_, None)) | None => 1,
        Some((_, Some(_))) => 2,
    };
    let mut sorted = base;
    sorted.sort_by_key(|p| rank(p));
    sorted
}

/// What went wrong with a Test, in the terms the card can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// 401, or "invalid api key" — the key is wrong.
    KeyRejected,
    /// 403 / 451 — the network or the origin is refused. No key fixes it.
    Blocked,
    /// 429, or "rate limit" — wait.
    RateLimited,
    /// 402, or "insufficient credit" — top up.
    NoCredit,
    /// Something else: show the provider's own words.
    Other,
}

/// Read the provider's error text as one of the four situations the card
/// knows how to answer.
///
/// The status code is the reliable signal, and the API errors carry it in
/// their message (`"HTTP 403: …"`, `"API error 401 Unauthorized: …"`). The
/// word checks are a fallback for providers that report the reason without
/// a code.
pub fn classify(message: &str) -> FailureKind {
    for token in message.split(|c: char| !c.is_ascii_digit()) {
        let Ok(code) = token.parse::<u16>() else {
            continue;
        };
        if token.len() != 3 || !(100..600).contains(&code) {
            continue;
        }
        return match code {
            401 => FailureKind::KeyRejected,
            402 => FailureKind::NoCredit,
            403 | 451 => FailureKind::Blocked,
            429 => FailureKind::RateLimited,
            // Any other status is an ordinary failure; the message below
            // says what the provider said.
            _ => FailureKind::Other,
        };
    }

    let lower = message.to_ascii_lowercase();
    if lower.contains("insufficient") || lower.contains("no credit") || lower.contains("quota") {
        FailureKind::NoCredit
    } else if lower.contains("rate limit") || lower.contains("too many") {
        FailureKind::RateLimited
    } else if lower.contains("forbidden") || lower.contains("blocked") {
        FailureKind::Blocked
    } else if lower.contains("unauthorized")
        || lower.contains("invalid api key")
        || lower.contains("incorrect api key")
        || lower.contains("key is empty")
    {
        FailureKind::KeyRejected
    } else {
        FailureKind::Other
    }
}

/// Title and next action for each failure. The point of the four cases is
/// that they do not lead to the same place: a rejected key is fixed on the
/// provider page, a block is fixed by switching provider or VPN, a rate
/// limit by waiting, and missing credit at the provider's billing page.
pub fn failure_copy(kind: FailureKind) -> (&'static str, &'static str) {
    match kind {
        FailureKind::KeyRejected => (
            "Key rejected",
            "Copy the key again from the provider page. Extra spaces count.",
        ),
        FailureKind::Blocked => (
            "Service blocked on your network",
            "Pick a provider marked reachable, or turn on your VPN.",
        ),
        FailureKind::RateLimited => (
            "Too many requests",
            "The free tier allows a few requests a minute. Wait a moment, then test again.",
        ),
        FailureKind::NoCredit => (
            "No credit left",
            "Top up at the provider. Your settings and key stay as they are.",
        ),
        FailureKind::Other => (
            "Could not connect",
            "Check the details below, then test again.",
        ),
    }
}

/// Probe every built-in provider once, off the UI thread.
///
/// The function is idempotent: it returns immediately while a probe is in
/// flight or once results exist. "Check again" calls [`reprobe`] instead.
pub fn start_probe(state: &Entity<DictState>, cx: &mut gpui::App) {
    let (probing, has_results) = {
        let s = state.read(cx);
        (s.qt_setup.probing, !s.qt_setup.reach.is_empty())
    };
    if probing || has_results {
        return;
    }
    probe(state, cx);
}

/// Throw away the previous results and probe again.
pub fn reprobe(state: &Entity<DictState>, cx: &mut gpui::App) {
    state.update(cx, |s, _| {
        s.qt_setup.reach.clear();
    });
    probe(state, cx);
}

fn probe(state: &Entity<DictState>, cx: &mut gpui::App) {
    state.update(cx, |s, cx| {
        s.qt_setup.probing = true;
        s.qt_setup.reach = PRESETS.iter().map(|p| (p.id, None)).collect();
        cx.notify();
    });

    let state = state.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let base_urls: Vec<&'static str> = PRESETS.iter().map(|p| p.base_url).collect();
        let results = cx
            .background_executor()
            .spawn(async move { dicto_translate::reach::probe_all(&base_urls) })
            .await;

        cx.update(|cx| {
            cx.update_entity(&state, |s, cx| {
                s.qt_setup.reach = PRESETS
                    .iter()
                    .map(|p| p.id)
                    .zip(results.into_iter().map(Some))
                    .collect();
                s.qt_setup.probing = false;
                cx.notify();
            });
        });
    })
    .detach();
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// The whole guided card.
pub fn setup_card(
    state: Entity<DictState>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    start_probe(&state, cx);

    let setup = state.read(cx).qt_setup.clone();
    let provider = setup.provider.and_then(preset);

    let mut card = v_flex()
        .gap(px(10.))
        .p(px(14.))
        .rounded(px(10.))
        .border_1()
        .border_color(colors::border())
        .bg(colors::surface());

    // Header: the title names the chosen service, so the card stops being a
    // generic wizard as soon as one is picked.
    let mut header = h_flex().items_start().justify_between().gap(px(8.));
    header = header.child(
        v_flex()
            .gap(px(3.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(colors::text())
                    .child(SharedString::from(match provider {
                        Some(p) => format!("Set up {}", p.name),
                        None => "Turn on Quick Translate".to_string(),
                    })),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(colors::text_secondary())
                    .child(SharedString::from(match provider {
                        Some(p) => p.tagline.to_string(),
                        None => "Translate anything you select and hear it read aloud. \
                                 Two minutes, one key."
                            .to_string(),
                    })),
            ),
    );
    if provider.is_some() {
        header = header.child(ghost_button(
            "qt-setup-change",
            "Change",
            setup_again(state.clone()),
        ));
    }
    card = card.child(header);

    card = card.child(net_row(&state, &setup));
    card = card.child(stepper(setup.step, setup.connected));

    card = card.child(match setup.step {
        SetupStep::Choose => choose_body(&state, &setup),
        SetupStep::Key => key_body(&state, provider, cx),
        SetupStep::Connect => connect_body(&state, &setup, provider, window, cx),
    });

    card.child(
        div()
            .text_size(px(11.))
            .text_color(colors::text_secondary())
            .child(SharedString::from(
                "The offline dictionary keeps working without any of this.",
            )),
    )
    .into_any_element()
}

/// "Auto | Global | Iran" plus a re-check link.
///
/// Auto is not a guess about the user's country: it is the probe result.
/// The two manual modes exist because a probe can be wrong behind a VPN or a
/// proxy, and the user should not have to argue with the card.
fn net_row(state: &Entity<DictState>, setup: &SetupState) -> gpui::AnyElement {
    let mut group = h_flex()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(6.))
        .border_1()
        .border_color(colors::border())
        .bg(colors::bg());

    for mode in [NetMode::Auto, NetMode::Global, NetMode::Iran] {
        let active = setup.net == mode;
        let state = state.clone();
        let mut segment = div()
            .id(SharedString::from(format!("qt-net-{}", mode.label())))
            .px(px(8.))
            .py(px(2.))
            .rounded(px(4.))
            .cursor_pointer()
            .text_size(px(11.))
            .child(SharedString::from(mode.label()));
        segment = if active {
            segment.bg(colors::primary()).text_color(colors::bg())
        } else {
            segment
                .text_color(colors::text_secondary())
                .hover(|s| s.text_color(colors::text()))
        };
        group = group.child(segment.on_click(move |_, _, cx| {
            state.update(cx, |s, cx| {
                if s.qt_setup.net == mode {
                    return;
                }
                s.qt_setup.net = mode;
                cx.notify();
            });
        }));
    }

    let check_state = state.clone();
    let probing = setup.probing;
    h_flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .child(
            h_flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors::text_secondary())
                        .child(SharedString::from("Network")),
                )
                .child(group),
        )
        .child(
            div()
                .id("qt-net-recheck")
                .cursor_pointer()
                .text_size(px(11.))
                .text_color(colors::primary())
                .hover(|s| s.opacity(0.8))
                .child(SharedString::from(if setup.probing {
                    "Checking…"
                } else {
                    "Check again"
                }))
                .on_click(move |_, _, cx| {
                    // A second click while the probe is in flight would start
                    // a racing probe for the same list.
                    if !probing {
                        reprobe(&check_state, cx);
                    }
                }),
        )
        .into_any_element()
}

/// "1 Choose a service — 2 Get a key — 3 Connect".
fn stepper(current: SetupStep, connected: bool) -> gpui::AnyElement {
    let items = [
        ("Choose a service", SetupStep::Choose),
        ("Get a key", SetupStep::Key),
        ("Connect", SetupStep::Connect),
    ];
    let mut row = h_flex().items_center().gap(px(4.)).flex_wrap();
    for (ix, (label, step)) in items.iter().enumerate() {
        let done = current as u8 > *step as u8 || (*step == SetupStep::Connect && connected);
        let active = current == *step;
        let color = if done {
            colors::success()
        } else if active {
            colors::text()
        } else {
            colors::text_secondary()
        };
        let mut dot = div()
            .size(px(18.))
            .items_center()
            .justify_center()
            .rounded(px(9.))
            .border_1()
            .border_color(color)
            .text_size(px(10.))
            .text_color(color);
        if done {
            dot = dot.child(SharedString::from("✓"));
        } else {
            dot = dot.child(SharedString::from((ix + 1).to_string()));
        }
        row = row.child(
            h_flex().items_center().gap(px(5.)).child(dot).child(
                div()
                    .text_size(px(11.))
                    .text_color(color)
                    .child(SharedString::from(*label)),
            ),
        );
        if ix < items.len() - 1 {
            row = row.child(div().w(px(10.)).h(px(1.)).bg(colors::border()));
        }
    }
    row.into_any_element()
}

/// Step 1: the provider rows, in probe order.
fn choose_body(state: &Entity<DictState>, setup: &SetupState) -> gpui::AnyElement {
    let mut list = v_flex().gap(px(6.));
    for preset in order(setup.net, &setup.reach) {
        let reach = setup
            .reach
            .iter()
            .find(|(id, _)| *id == preset.id)
            .and_then(|(_, r)| *r);
        list = list.child(provider_row(state, preset, reach, setup.probing));
    }

    let manual_state = state.clone();
    list.child(
        div()
            .id("qt-setup-manual")
            .cursor_pointer()
            .pt(px(2.))
            .text_size(px(11.))
            .text_color(colors::text_secondary())
            .hover(|s| s.text_color(colors::text()))
            .child(SharedString::from(
                "Another OpenAI-compatible service… set it up by hand",
            ))
            .on_click(move |_, _, cx| {
                manual_state.update(cx, |s, cx| {
                    s.qt_setup_done = true;
                    cx.notify();
                });
            }),
    )
    .into_any_element()
}

fn provider_row(
    state: &Entity<DictState>,
    preset: &'static ProviderPreset,
    reach: Option<dicto_translate::reach::Reach>,
    probing: bool,
) -> gpui::AnyElement {
    let state = state.clone();
    let id = preset.id;

    let mut badges = h_flex().items_center().gap(px(4.)).flex_wrap().child(
        div()
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(colors::text())
            .child(SharedString::from(preset.name)),
    );
    if reach == Some(dicto_translate::reach::Reach::Reachable) {
        badges = badges.child(badge("✓ reachable", colors::success()));
    } else if reach == Some(dicto_translate::reach::Reach::Blocked) {
        badges = badges.child(badge("✕ blocked on your network", colors::error()));
    } else if reach == Some(dicto_translate::reach::Reach::Unreachable) {
        badges = badges.child(badge("no answer", colors::update()));
    } else if probing {
        badges = badges.child(badge("checking your network…", colors::text_secondary()));
    }
    if preset.speaks() {
        badges = badges.child(badge("speaks", colors::success()));
    } else {
        badges = badges.child(badge("text only", colors::text_secondary()));
    }

    h_flex()
        .id(SharedString::from(format!("qt-setup-pick-{id}")))
        .w_full()
        .items_center()
        .justify_between()
        .gap(px(10.))
        .px(px(10.))
        .py(px(8.))
        .rounded(px(8.))
        .border_1()
        .border_color(colors::border())
        .cursor_pointer()
        .hover(|s| s.bg(colors::hover()))
        .child(
            v_flex().gap(px(2.)).min_w(px(0.)).child(badges).child(
                div()
                    .text_size(px(11.))
                    .text_color(colors::text_secondary())
                    .child(SharedString::from(preset.tagline)),
            ),
        )
        .child(
            div()
                .text_size(px(14.))
                .text_color(colors::text_secondary())
                .child(SharedString::from("›")),
        )
        .on_click(move |_, window, cx| {
            let (key, voice_key) = state.update(cx, |s, cx| {
                providers::apply(preset, &mut s.quick_translate);
                s.qt_setup.provider = Some(id);
                s.qt_setup.step = SetupStep::Key;
                s.qt_setup.connected = false;
                s.qt_setup.advanced = false;
                s.qt_translation_test = QtKeyTest::Idle;
                s.save_settings(cx);
                s.reload_translator(cx);
                cx.notify();
                (
                    s.quick_translate.api_key.clone(),
                    s.quick_translate.tts.api_key.clone(),
                )
            });
            // A key belongs to the service that issued it, so switching
            // providers clears both fields. Push that into the visible inputs
            // too: the next render's reconcile skips a focused field, and the
            // old key would otherwise be typed back into settings.
            sync_key_inputs(&state, key, voice_key, window, cx);
        })
        .into_any_element()
}

/// Step 2: how to get a key, with a real link to the provider page.
fn key_body(
    state: &Entity<DictState>,
    provider: Option<&'static ProviderPreset>,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    let Some(provider) = provider else {
        return v_flex().into_any_element();
    };

    let mut body = v_flex().gap(px(10.));

    // Numbered walkthrough. A checklist is easier to follow than a paragraph
    // of prose, and each line is one action on the provider's dashboard.
    let mut steps = v_flex().gap(px(6.));
    for (ix, step) in provider.steps.iter().enumerate() {
        steps = steps.child(
            h_flex()
                .items_start()
                .gap(px(8.))
                .child(
                    div()
                        .size(px(16.))
                        .items_center()
                        .justify_center()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(colors::border())
                        .text_size(px(10.))
                        .text_color(colors::text_secondary())
                        .child(SharedString::from((ix + 1).to_string())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(12.))
                        .text_color(colors::text())
                        .child(SharedString::from(*step)),
                ),
        );
    }
    body = body.child(steps);

    if let Some(url) = provider.key_url {
        let label = if provider.key_label.is_empty() {
            "Open the provider page"
        } else {
            provider.key_label
        };
        body = body.child(
            h_flex()
                .id(SharedString::from(format!(
                    "qt-setup-key-link-{}",
                    provider.id
                )))
                .w_full()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(6.))
                .border_1()
                .border_color(colors::border())
                .bg(colors::bg())
                .cursor_pointer()
                .hover(|s| s.border_color(colors::primary()))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors::primary())
                        .child(SharedString::from(format!("{label} ↗"))),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors::text_secondary())
                        .child(SharedString::from("opens in your browser")),
                )
                .on_click(move |_, _, cx| cx.open_url(url)),
        );
    }

    body = body.child(
        div()
            .text_size(px(11.))
            .text_color(colors::text_secondary())
            .child(SharedString::from(provider.signup_note)),
    );

    // A blocked provider is worth saying out loud here, before the user
    // spends time on a key that will never work from this network.
    if blocked_from_here(state.read(cx), provider) {
        body = body.child(
            div()
                .p(px(8.))
                .rounded(px(6.))
                .border_1()
                .border_color(colors::update())
                .text_size(px(11.))
                .text_color(colors::update())
                .child(SharedString::from(
                    "This service does not answer from your network. A VPN is required. \
                     If you do not have one, go back and pick a provider marked reachable.",
                )),
        );
    }

    let back_state = state.clone();
    let next_state = state.clone();
    body.child(
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(ghost_button("qt-setup-back", "Back", move |cx| {
                back_state.update(cx, |s, cx| {
                    s.qt_setup.step = SetupStep::Choose;
                    s.qt_setup.provider = None;
                    cx.notify();
                });
            }))
            .child(primary_button(
                "qt-setup-have-key",
                if provider.keyed() {
                    "I have a key"
                } else {
                    "It's running — test it"
                },
                move |cx| {
                    next_state.update(cx, |s, cx| {
                        s.qt_setup.step = SetupStep::Connect;
                        cx.notify();
                    });
                },
            )),
    )
    .into_any_element()
}

/// Push key values into the visible `InputState`s.
///
/// Used after a provider switch, whose whole point is that the previous
/// service's key is gone. `reconcile` in the panel would do this on the next
/// render unless the field has focus — and clicking a provider row does not
/// necessarily take focus away from the key field.
fn sync_key_inputs(
    state: &Entity<DictState>,
    key: String,
    voice_key: String,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    if let Some(input) = state.read(cx).qt_api_key_input.clone() {
        input.update(cx, |input, cx| input.set_value(key, window, cx));
    }
    if let Some(input) = state.read(cx).qt_tts_api_key_input.clone() {
        input.update(cx, |input, cx| input.set_value(voice_key, window, cx));
    }
}

fn blocked_from_here(state: &DictState, provider: &ProviderPreset) -> bool {
    state
        .qt_setup
        .reach
        .iter()
        .find(|(id, _)| *id == provider.id)
        .and_then(|(_, r)| *r)
        == Some(dicto_translate::reach::Reach::Blocked)
}

/// Step 3: the key, the test, and — once connected — the model and voice.
fn connect_body(
    state: &Entity<DictState>,
    setup: &SetupState,
    provider: Option<&'static ProviderPreset>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    let Some(provider) = provider else {
        return v_flex().into_any_element();
    };

    if setup.connected {
        return connected_body(state, setup, provider, window, cx);
    }

    let mut body = v_flex().gap(px(10.));

    if provider.keyed() {
        body = body.child(input_row(
            "API key",
            &state.read(cx).qt_api_key_input.clone().unwrap(),
            true,
            cx,
        ));
        body = body.child(
            div()
                .text_size(px(11.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(format!(
                    "Stored only on this computer. It is never sent anywhere except {}.",
                    provider.name
                ))),
        );
    } else {
        body = body.child(
            div()
                .px(px(10.))
                .py(px(7.))
                .rounded(px(6.))
                .border_1()
                .border_color(colors::border())
                .bg(colors::bg())
                .text_size(px(12.))
                .text_color(colors::text())
                .child(SharedString::from(format!(
                    "No key needed for {}.",
                    provider.name
                ))),
        );
    }

    let test_state = state.clone();
    let back_state = state.clone();
    body = body.child(
        h_flex()
            .items_center()
            .gap(px(8.))
            .child(primary_button("qt-setup-test", "Test & save", move |cx| {
                crate::components::quick_translate_panel::run_translation_test(
                    test_state.clone(),
                    cx,
                );
            }))
            .child(ghost_button("qt-setup-connect-back", "Back", move |cx| {
                back_state.update(cx, |s, cx| {
                    s.qt_setup.step = SetupStep::Key;
                    cx.notify();
                });
            })),
    );

    body = body.child(test_status(state.read(cx)));
    body.child(
        div()
            .text_size(px(11.))
            .text_color(colors::text_secondary())
            .child(SharedString::from(
                "Test sends one short sentence, checks the key, and saves the \
                 settings only when it works.",
            )),
    )
    .into_any_element()
}

/// The line under Test: idle, running, success, or the classified failure.
fn test_status(state: &DictState) -> gpui::AnyElement {
    match &state.qt_translation_test {
        QtKeyTest::Idle => hint_line(
            "Nothing is sent until you press Test.".to_string(),
            colors::text_secondary(),
        ),
        QtKeyTest::Running => hint_line("Testing…".to_string(), colors::text_secondary()),
        QtKeyTest::Ok(msg) => hint_line(format!("✓ Connected. {msg}"), colors::success()),
        QtKeyTest::Err(msg) => {
            let kind = classify(msg);
            let (title, next) = failure_copy(kind);
            v_flex()
                .gap(px(2.))
                .child(hint_line(format!("✗ {title}"), colors::error()))
                .child(hint_line(next.to_string(), colors::text_secondary()))
                // The provider's own words stay visible: the four cases are
                // a summary, not a replacement for the evidence.
                .child(hint_line(msg.clone(), colors::text_secondary()))
                .into_any_element()
        }
    }
}

fn connected_body(
    state: &Entity<DictState>,
    setup: &SetupState,
    provider: &'static ProviderPreset,
    window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    let settings = state.read(cx).quick_translate.clone();
    let mut body = v_flex().gap(px(10.));

    body = body.child(
        v_flex()
            .gap(px(3.))
            .p(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(colors::success())
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors::success())
                            .child(SharedString::from(format!("✓ {} connected", provider.name))),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors::text_secondary())
                            .child(SharedString::from(settings.model.clone())),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(colors::text_secondary())
                    .child(SharedString::from(format!(
                        "Translate into {}. Change the target language in the \
                         popup's Options panel.",
                        settings.target_lang
                    ))),
            ),
    );

    body = body.child(model_picker_row(&settings, state, window, cx));
    body = body.child(voice_section(state, &settings, provider, window, cx));

    // Advanced: the endpoint the preset filled in. The model and voice are
    // above already, so this is only the one field a preset can get wrong.
    let adv_state = state.clone();
    let advanced_button = ghost_button(
        "qt-setup-advanced",
        if setup.advanced {
            "Hide advanced"
        } else {
            "Advanced"
        },
        move |cx| {
            adv_state.update(cx, |s, cx| {
                s.qt_setup.advanced = !s.qt_setup.advanced;
                cx.notify();
            });
        },
    );
    if setup.advanced {
        body = body.child(divider());
        body = body.child(
            div()
                .text_size(px(11.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(
                    "These already work. Change them only if you have a reason.",
                )),
        );
        body = body.child(input_row(
            "Base URL",
            &state.read(cx).qt_base_url_input.clone().unwrap(),
            false,
            cx,
        ));
    }

    let done_state = state.clone();
    let try_state = state.clone();
    let hotkey = settings.hotkey.clone();
    body.child(
        v_flex()
            .gap(px(6.))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(advanced_button)
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .child(ghost_button("qt-setup-done", "Done", move |cx| {
                                finish_setup(done_state.clone(), cx);
                            }))
                            .child(primary_button("qt-setup-try", "Try it now", move |cx| {
                                finish_setup(try_state.clone(), cx)
                            })),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(colors::text_secondary())
                    .child(SharedString::from(format!(
                        "Select text anywhere, then press {hotkey}."
                    ))),
            ),
    )
    .into_any_element()
}

/// The voice half of the card: one AI voice, two languages.
///
/// Providers reachable today publish one multilingual voice rather than a
/// separate English and Persian voice, so both rows show the same choice and
/// each plays its own sample. A provider without any speech model says so
/// per language, because that is where the system voice takes over.
fn voice_section(
    state: &Entity<DictState>,
    settings: &mdict_rs::settings::QuickTranslateSettings,
    provider: &'static ProviderPreset,
    window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    let mut section = v_flex()
        .gap(px(8.))
        .p(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(colors::border());

    let tts_on = settings.tts.enabled && provider.speaks();
    let toggle_state = state.clone();
    let mut head = h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .child(
            v_flex()
                .gap(px(1.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors::text())
                        .child(SharedString::from("AI voice")),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors::text_secondary())
                        .child(SharedString::from(if provider.speaks() {
                            "Uses the same key."
                        } else {
                            "None on this provider. Your computer speaks instead."
                        })),
                ),
        );
    if provider.speaks() {
        head = head.child(crate::components::quick_translate_panel::toggle_switch(
            "qt-setup-voice-toggle",
            tts_on,
            move |cx| {
                toggle_state.update(cx, |s, cx| {
                    s.quick_translate.tts.enabled = !tts_on;
                    s.save_settings(cx);
                    s.invalidate_tts_clips_for_settings(None);
                    cx.notify();
                });
            },
        ));
    }
    section = section.child(head);

    if tts_on {
        let free_state = state.clone();
        let free_on = settings.tts.free_only;
        section = section.child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors::text_secondary())
                        .child(SharedString::from("Voice models")),
                )
                .child(free_filter_segments(
                    "qt-setup-free-voice",
                    free_on,
                    move |want, cx| {
                        free_state.update(cx, |s, cx| {
                            s.quick_translate.tts.free_only = want;
                            s.save_settings(cx);
                            cx.notify();
                        });
                    },
                )),
        );
        section = section.child(tts_model_picker_row(settings, state, window, cx));
        if !crate::components::quick_translate_panel::tts_voices_for(settings, state, cx).is_empty()
        {
            section = section.child(tts_voice_picker_row(settings, state, window, cx));
        }
    }

    for (label, lang, sample) in [
        (
            "English",
            "en",
            "The quick brown fox jumps over the lazy dog.",
        ),
        ("Persian", "fa", "سلام! این یک تست صداست."),
    ] {
        let tts = settings.tts.clone();
        // A voice that cannot speak this language is not a voice for it:
        // the row says "system" so the promise in the design holds.
        let speaks = provider.speaks() && tts_on && crate::tts::ai_voice_speaks(Some(lang), &tts);
        let mut row = h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .pt(px(4.))
            .child(
                v_flex()
                    .gap(px(1.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(if speaks {
                                colors::success()
                            } else {
                                colors::text_secondary()
                            })
                            .child(SharedString::from(format!(
                                "{label} {}",
                                if speaks { "✓" } else { "→ system" }
                            ))),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(colors::text_secondary())
                            .child(SharedString::from(if speaks {
                                format!("{} · {}", tts.model, tts.voice)
                            } else {
                                String::from("No AI voice for this language.")
                            })),
                    ),
            );
        if speaks {
            let sample = sample.to_string();
            row = row.child(ghost_button(
                match lang {
                    "en" => "qt-voice-sample-en",
                    _ => "qt-voice-sample-fa",
                },
                "Play sample",
                move |_cx| {
                    crate::tts::speak(&sample, Some(lang), Some(&tts));
                },
            ));
        }
        section = section.child(row);
    }

    section.into_any_element()
}

/// Turn the card off and leave a working configuration behind.
fn finish_setup(state: Entity<DictState>, cx: &mut gpui::App) {
    state.update(cx, |s, cx| {
        s.qt_setup_done = true;
        s.qt_setup.connected = false;
        s.qt_setup.advanced = false;
        // The card exists to turn the feature on: leaving it off after a
        // successful setup would make the hotkey do nothing.
        s.quick_translate.enabled = true;
        s.save_settings(cx);
        s.reload_hotkey(cx);
        s.reload_translator(cx);
        cx.notify();
    });
}

/// "Change" in the header, and "Run setup again" in the full form.
fn setup_again(state: Entity<DictState>) -> impl Fn(&mut gpui::App) + 'static {
    move |cx| {
        state.update(cx, |s, cx| {
            s.qt_setup_done = false;
            // Explicitly starting over: always the provider list, never a
            // half-filled step from the previous configuration.
            s.qt_setup = SetupState::default();
            s.qt_translation_test = QtKeyTest::Idle;
            cx.notify();
        });
    }
}

/// Link at the bottom of the full form: return to the guided card.
pub fn run_setup_again(state: Entity<DictState>) -> gpui::AnyElement {
    let start = setup_again(state);
    div()
        .id("qt-setup-again")
        .cursor_pointer()
        .mt(px(10.))
        .text_size(px(11.))
        .text_color(colors::primary())
        .hover(|s| s.opacity(0.8))
        .child(SharedString::from("Run setup again…"))
        .on_click(move |_, _, cx| start(cx))
        .into_any_element()
}

// ---------------------------------------------------------------------------
// Small widgets
// ---------------------------------------------------------------------------

fn primary_button(
    id: &'static str,
    label: impl Into<SharedString>,
    on_click: impl Fn(&mut gpui::App) + 'static,
) -> gpui::AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .px(px(12.))
        .py(px(6.))
        .rounded(px(6.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(colors::bg())
        .bg(colors::primary())
        .hover(|s| s.opacity(0.9))
        .child(label.into())
        .on_click(move |_, _, cx| on_click(cx))
        .into_any_element()
}

fn ghost_button(
    id: &'static str,
    label: impl Into<SharedString>,
    on_click: impl Fn(&mut gpui::App) + 'static,
) -> gpui::AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .px(px(12.))
        .py(px(6.))
        .rounded(px(6.))
        .text_size(px(12.))
        .text_color(colors::text())
        .border_1()
        .border_color(colors::border())
        .hover(|s| s.bg(colors::hover()))
        .child(label.into())
        .on_click(move |_, _, cx| on_click(cx))
        .into_any_element()
}

fn badge(text: &str, color: gpui::Hsla) -> gpui::AnyElement {
    div()
        .px(px(5.))
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_color(color)
        .text_size(px(10.))
        .text_color(color)
        .child(SharedString::from(text))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dicto_translate::reach::Reach;

    fn reach(pairs: &[(&'static str, Reach)]) -> Vec<(&'static str, Option<Reach>)> {
        pairs.iter().map(|(id, r)| (*id, Some(*r))).collect()
    }

    fn ids(list: &[&'static ProviderPreset]) -> Vec<&'static str> {
        list.iter().map(|p| p.id).collect()
    }

    #[test]
    fn auto_mode_puts_reachable_services_first() {
        let reach = reach(&[
            ("openrouter", Reach::Blocked),
            ("avalai", Reach::Reachable),
            ("github", Reach::Reachable),
            ("local", Reach::Unreachable),
        ]);
        assert_eq!(
            ids(&order(NetMode::Auto, &reach)),
            vec!["avalai", "github", "openrouter", "local"]
        );
    }

    #[test]
    fn auto_mode_keeps_the_built_in_order_while_checking() {
        let checking: Vec<(&'static str, Option<Reach>)> =
            PRESETS.iter().map(|p| (p.id, None)).collect();
        assert_eq!(
            ids(&order(NetMode::Auto, &checking)),
            vec!["openrouter", "avalai", "github", "local"]
        );
    }

    #[test]
    fn the_iran_override_starts_with_a_service_that_works_there() {
        assert_eq!(
            ids(&order(NetMode::Iran, &[])),
            vec!["avalai", "openrouter", "github", "local"]
        );
    }

    #[test]
    fn manual_modes_ignore_the_probe() {
        let reach = reach(&[
            ("openrouter", Reach::Blocked),
            ("avalai", Reach::Reachable),
            ("github", Reach::Reachable),
            ("local", Reach::Unreachable),
        ]);
        assert_eq!(
            ids(&order(NetMode::Global, &reach)),
            vec!["openrouter", "avalai", "github", "local"]
        );
    }

    #[test]
    fn a_rejected_key_and_a_blocked_network_are_different_problems() {
        assert_eq!(
            classify("HTTP 401 Unauthorized: bad key"),
            FailureKind::KeyRejected
        );
        assert_eq!(
            classify("API error 403 Forbidden: nope"),
            FailureKind::Blocked
        );
        assert_eq!(classify("HTTP 451: legal reasons"), FailureKind::Blocked);
    }

    #[test]
    fn rate_limits_and_credit_are_told_apart() {
        assert_eq!(
            classify("HTTP 429 Too Many Requests"),
            FailureKind::RateLimited
        );
        assert_eq!(classify("HTTP 402 Payment Required"), FailureKind::NoCredit);
        assert_eq!(
            classify("insufficient credit on this key"),
            FailureKind::NoCredit
        );
    }

    #[test]
    fn every_failure_case_gives_a_different_next_step() {
        let mut next_steps = std::collections::HashSet::new();
        for kind in [
            FailureKind::KeyRejected,
            FailureKind::Blocked,
            FailureKind::RateLimited,
            FailureKind::NoCredit,
        ] {
            let (title, next) = failure_copy(kind);
            assert!(!title.is_empty());
            assert!(next_steps.insert(next), "duplicate next step for {title}");
        }
    }

    #[test]
    fn an_unrecognised_failure_keeps_the_generic_message() {
        assert_eq!(classify("connection reset by peer"), FailureKind::Other);
        assert_eq!(failure_copy(FailureKind::Other).0, "Could not connect");
    }

    #[test]
    fn an_interrupted_setup_resumes_on_the_key_step() {
        let setup = SetupState::detect("https://api.avalai.ir/v1", "");
        assert_eq!(setup.provider, Some("avalai"));
        assert_eq!(setup.step, SetupStep::Key);

        let done = SetupState::detect("https://api.avalai.ir/v1", "sk-1");
        assert_eq!(done.step, SetupStep::Connect);

        let unknown = SetupState::detect("https://example.com/v1", "");
        assert_eq!(unknown.provider, None);
        assert_eq!(unknown.step, SetupStep::Choose);
    }
}
