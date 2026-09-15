//! First-run telemetry consent dialog.
//!
//! Shown exactly once, on launch, when [`TelemetryConsent`] is `Undecided`
//! (true for both fresh installs and upgraders whose pre-telemetry
//! `settings.toml` lacks the field). The user makes an explicit choice:
//!
//! - **Allow** → persists `OptedIn` and re-inits telemetry so events start
//!   flowing immediately (no restart).
//! - **Don't Allow** → persists `OptedOut`.
//! - **close/X** → treated as `OptedOut` (implicit, respectful "not now, and
//!   don't ask again"). The user can still flip it on later via the
//!   Settings → Telemetry tab.
//!
//! Closing → `OptedOut` means the dialog never nags: it appears at most once
//! per installation, on the first launch where consent is undecided.

use gpui::{
    FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{WindowExt, h_flex, v_flex};

use crate::colors;

/// Open the consent dialog on `window`. No-op if the user already decided.
pub fn open_consent_dialog(window: &mut Window, cx: &mut gpui::App) {
    // Guard: only show when genuinely undecided.
    let consent = mdict_rs::settings::current().telemetry_consent;
    if !matches!(consent, mdict_rs::settings::TelemetryConsent::Undecided) {
        return;
    }

    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .title(div().child("Telemetry Consent"))
            .w(px(440.))
            .close_button(true)
            .overlay_closable(true)
            .on_close(|_event, _window, cx| {
                // Closing the dialog (X button, overlay click, Escape) is an
                // implicit "not now" — record OptedOut so we never re-prompt.
                set_consent(mdict_rs::settings::TelemetryConsent::OptedOut, cx);
            })
            .content(move |content, _window, _cx| {
                content.child(
                    v_flex()
                        .w_full()
                        .gap(px(14.))
                        // Heading
                        .child(
                            div()
                                .text_size(px(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors::text())
                                .child(SharedString::from("Help improve Dicto")),
                        )
                        // Body
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(colors::text_secondary())
                                .child(SharedString::from(
                                    "Allow Dicto to collect anonymous usage statistics? \
                                         This helps fix bugs and prioritize features. You can \
                                         change this anytime in Settings \u{2192} Telemetry.",
                                )),
                        )
                        // Collected / not collected
                        .child(summary_list(
                            "What we collect",
                            &[
                                "Lookups performed (count only)",
                                "Pronunciation plays and playback failures",
                                "Dictionaries imported (count, never names)",
                                "Operating system, app version, system locale",
                                "Indexing/import errors (paths stripped)",
                            ],
                        ))
                        .child(summary_list(
                            "What we never collect",
                            &[
                                "Which words you look up",
                                "Dictionary contents or names",
                                "Your name, username, or personal data",
                            ],
                        )),
                )
            })
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("consent-deny-btn")
                            .px(px(14.))
                            .py(px(7.))
                            .rounded(px(6.))
                            .text_size(px(13.))
                            .text_color(colors::text_secondary())
                            .border_1()
                            .border_color(colors::border())
                            .cursor_pointer()
                            .hover(|s| s.bg(colors::bg()))
                            .child("Don't Allow")
                            .on_click(|_, window, cx| {
                                set_consent(mdict_rs::settings::TelemetryConsent::OptedOut, cx);
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        div()
                            .id("consent-allow-btn")
                            .px(px(14.))
                            .py(px(7.))
                            .rounded(px(6.))
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colors::bg())
                            .bg(colors::primary())
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.85))
                            .child("Allow")
                            .on_click(|_, window, cx| {
                                set_consent(mdict_rs::settings::TelemetryConsent::OptedIn, cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}

/// Persist a consent decision and re-initialize the global telemetry client
/// so the choice takes effect immediately in the current process — opt-in
/// starts an `AptabaseClient` and events begin flowing without a restart;
/// opt-out swaps in `NullTelemetry` and pending events stop. Either way the
/// persisted decision is also picked up on next launch.
fn set_consent(consent: mdict_rs::settings::TelemetryConsent, _cx: &mut gpui::App) {
    if let Err(e) = mdict_rs::settings::update_consent(consent) {
        tracing::warn!("telemetry: failed to persist consent: {e}");
        return;
    }
    let settings = mdict_rs::settings::current();
    let opted_in = matches!(
        settings.telemetry_consent,
        mdict_rs::settings::TelemetryConsent::OptedIn
    );
    dicto_telemetry::init(
        opted_in,
        settings.installation_id.unwrap_or_default(),
        env!("APP_VERSION").to_string(),
    );
}

fn summary_list(title: &str, items: &[&str]) -> gpui::AnyElement {
    let mut list = v_flex().gap(px(2.));
    for item in items {
        list = list.child(
            h_flex()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(colors::text())
                .child(
                    div()
                        .text_color(colors::text_secondary())
                        .child(SharedString::from("\u{2022}")),
                )
                .child(SharedString::from((*item).to_string())),
        );
    }
    v_flex()
        .w_full()
        .gap(px(3.))
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(title.to_string())),
        )
        .child(list)
        .into_any_element()
}
