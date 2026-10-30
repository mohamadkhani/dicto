//! General settings tab — the everyday toggles: start in tray at login,
//! system shortcut registration, the two quick features' enable + hotkey,
//! and telemetry consent (Allow / Don't Allow + a click-activated Details
//! panel). Advanced per-feature fields stay in their own tabs.

use gpui::{
    Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::{h_flex, scroll::ScrollableElement, v_flex};

use crate::{colors, state::DictState};

use super::quick_translate_panel::{divider, toggle_switch};

/// Build the General settings tab content.
pub fn general_tab_content(state: Entity<DictState>, cx: &mut gpui::App) -> gpui::AnyElement {
    let header = v_flex()
        .gap(px(4.))
        .pb(px(16.))
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::text())
                .child(SharedString::from("General")),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(
                    "Everyday settings: startup behavior, system shortcuts, \
                     quick features, and telemetry.",
                )),
        );

    let mut body = v_flex()
        .gap(px(4.))
        .w_full()
        .h_full()
        .p(px(4.))
        .id("general-settings-scroll")
        .overflow_y_scrollbar()
        .child(header)
        .child(divider());

    body = body
        .child(section_title("Startup"))
        .child(start_in_tray_section(&state, cx))
        .child(divider())
        .child(section_title("System Shortcuts"))
        .child(system_shortcuts_section(&state, cx))
        .child(divider())
        .child(section_title("Word Lookup"))
        .child(word_lookup_section(&state, cx))
        .child(divider())
        .child(section_title("Quick Translate"))
        .child(quick_translate_section(&state, cx))
        .child(divider())
        .child(section_title("Telemetry"))
        .child(telemetry_section(&state, cx));

    div()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .child(body)
        .into_any_element()
}

fn section_title(text: &str) -> gpui::AnyElement {
    div()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::text_secondary())
        .pt(px(8.))
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

/// A label + value chip for the (read-only) hotkey displays.
fn hotkey_row(label: &str, hotkey: &str) -> gpui::AnyElement {
    h_flex()
        .items_center()
        .py(px(8.))
        .gap(px(12.))
        .child(
            div()
                .w(px(120.))
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(label.to_string())),
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
                .child(SharedString::from(hotkey.to_string())),
        )
        .into_any_element()
}

/// Start in tray at login — the toggle is the autostart entry itself (no
/// settings.toml state; `is_enabled()` reads the entry's existence).
fn start_in_tray_section(state: &Entity<DictState>, _cx: &gpui::App) -> gpui::AnyElement {
    let enabled = crate::autostart::is_enabled();
    let toggle_state = state.clone();
    h_flex()
        .justify_between()
        .items_center()
        .py(px(10.))
        .child(
            v_flex()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(colors::text())
                        .child(SharedString::from("Start Dicto in tray at login")),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors::text_secondary())
                        .child(SharedString::from(
                            "Dicto starts in the background at login with no window — \
                         use the tray icon or the shortcuts.",
                        )),
                ),
        )
        .child(toggle_switch("autostart", enabled, move |cx| {
            let result = if enabled {
                crate::autostart::disable()
            } else {
                crate::autostart::enable()
            };
            if let Err(e) = result {
                tracing::warn!("autostart: failed to toggle: {e}");
            }
            toggle_state.update(cx, |_s, cx| cx.notify());
        }))
        .into_any_element()
}

/// Auto-registration toggle plus the live status of the OS-level shortcut
/// layer (portal / GNOME keybindings / manual snippet). Backed by
/// `hotkey::os_binding`. Moved here from the retired System Shortcuts tab.
#[cfg(target_os = "linux")]
fn system_shortcuts_section(state: &Entity<DictState>, cx: &gpui::App) -> gpui::AnyElement {
    let auto_shortcuts = state.read(cx).auto_shortcuts;
    let toggle_state = state.clone();
    let auto_row = h_flex()
        .justify_between()
        .items_center()
        .py(px(10.))
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors::text())
                .child(SharedString::from("Set up system shortcuts automatically")),
        )
        .child(toggle_switch("auto-shortcuts", auto_shortcuts, move |cx| {
            toggle_state.update(cx, |s, cx| {
                s.auto_shortcuts = !auto_shortcuts;
                s.save_settings(cx);
                s.sync_os_bindings();
            });
        }));

    let mut section = v_flex().gap(px(4.)).child(auto_row);

    match crate::hotkey::os_binding::status() {
        None => {
            section = section.child(
                div()
                    .text_size(px(11.))
                    .text_color(colors::text_secondary())
                    .child(SharedString::from("Setting up shortcuts…")),
            );
        }
        Some(status) => {
            let mode_line = match status.mode {
                crate::hotkey::os_binding::OsBindingMode::Off => "Off".to_string(),
                crate::hotkey::os_binding::OsBindingMode::PortalManaged => {
                    "Managed by the desktop portal".to_string()
                }
                crate::hotkey::os_binding::OsBindingMode::NativeRegistration => {
                    "Registered with the window system".to_string()
                }
                crate::hotkey::os_binding::OsBindingMode::GnomeKeybindings => {
                    "Set in GNOME custom shortcuts".to_string()
                }
                crate::hotkey::os_binding::OsBindingMode::ManualRequired => {
                    "Manual setup required".to_string()
                }
            };
            let mut text = format!("System shortcuts: {mode_line}");
            for action in &status.actions {
                text.push('\n');
                text.push_str(action);
            }
            if !status.detail.is_empty() {
                text.push('\n');
                text.push_str(&status.detail);
            }
            section = section.child(
                div()
                    .text_size(px(11.))
                    .text_color(colors::text_secondary())
                    .child(SharedString::from(text)),
            );

            if let Some(snippet) = status.snippet {
                section = section.child(
                    div()
                        .mt(px(6.))
                        .p(px(8.))
                        .rounded(px(4.))
                        .bg(colors::bg())
                        .border_1()
                        .border_color(colors::border())
                        .text_size(px(10.))
                        .text_color(colors::text())
                        .child(SharedString::from(snippet)),
                );
            }
        }
    }

    section.into_any_element()
}

#[cfg(not(target_os = "linux"))]
fn system_shortcuts_section(state: &Entity<DictState>, cx: &gpui::App) -> gpui::AnyElement {
    let _ = (state, cx);
    div()
        .pt(px(8.))
        .text_size(px(11.))
        .text_color(colors::text_secondary())
        .child(SharedString::from(
            "No extra setup is needed on this platform — shortcuts are \
             registered directly with the window system.",
        ))
        .into_any_element()
}

/// Word Lookup enable + hotkey (the popup's own settings live here too).
fn word_lookup_section(state: &Entity<DictState>, cx: &gpui::App) -> gpui::AnyElement {
    let settings = state.read(cx).word_lookup.clone();
    let enabled = settings.enabled;
    let registering = state.read(cx).word_lookup_registering;

    let toggle_state = state.clone();
    let enable_row = h_flex()
        .justify_between()
        .items_center()
        .py(px(10.))
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors::text())
                .child(SharedString::from("Enable Word Lookup")),
        )
        .child(toggle_switch("wl-enable", enabled, move |cx| {
            super::word_lookup_panel::set_word_lookup_enabled(toggle_state.clone(), !enabled, cx);
        }));

    let mut section = v_flex().gap(px(4.)).child(enable_row);
    if registering {
        section = section.child(
            div()
                .text_size(px(11.))
                .text_color(colors::text_secondary())
                .child(SharedString::from("Registering hotkey…")),
        );
    }
    if enabled {
        section = section.child(hotkey_row("Global Hotkey", &settings.hotkey));
    }
    section.into_any_element()
}

/// Quick Translate enable + hotkey (advanced fields live in its own tab).
fn quick_translate_section(state: &Entity<DictState>, cx: &gpui::App) -> gpui::AnyElement {
    let settings = state.read(cx).quick_translate.clone();
    let enabled = settings.enabled;

    let toggle_state = state.clone();
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

    let mut section = v_flex().gap(px(4.)).child(enable_row);
    if enabled {
        section = section.child(hotkey_row("Global Hotkey", &settings.hotkey));
    }
    section.into_any_element()
}

/// Telemetry consent with Allow / Don't Allow buttons and a click-activated
/// Details panel (no popover component exists in gpui_component, so the
/// details expand inline).
fn telemetry_section(state: &Entity<DictState>, cx: &mut gpui::App) -> gpui::AnyElement {
    let settings = mdict_rs::settings::current();
    let opted_out = matches!(
        settings.telemetry_consent,
        mdict_rs::settings::TelemetryConsent::OptedOut
    );
    let details_open = state.read(cx).telemetry_details_open;

    let consent_note = div()
        .text_size(px(11.))
        .text_color(colors::text_secondary())
        .child(SharedString::from(
            "Anonymous usage data helps improve Dicto. No personal information, \
             dictionary contents, or looked-up words are ever collected.",
        ));

    let deny_state = state.clone();
    let allow_state = state.clone();
    let details_state = state.clone();

    let buttons = h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .px(px(10.))
        .py(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(colors::border())
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::text())
                .child(SharedString::from("Share anonymous usage data")),
        )
        .child(
            h_flex()
                .gap(px(8.))
                .child(
                    div()
                        .id("telemetry-details-btn")
                        .px(px(14.))
                        .py(px(7.))
                        .rounded(px(6.))
                        .text_size(px(13.))
                        .text_color(colors::primary())
                        .border_1()
                        .border_color(colors::border())
                        .cursor_pointer()
                        .hover(|s| s.bg(colors::hover()))
                        .child("Details")
                        .on_click(move |_ev, _window, cx| {
                            details_state.update(cx, |s, cx| {
                                s.telemetry_details_open = !s.telemetry_details_open;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    div()
                        .id("telemetry-deny-btn")
                        .px(px(14.))
                        .py(px(7.))
                        .rounded(px(6.))
                        .text_size(px(13.))
                        .font_weight(if opted_out {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if opted_out {
                            colors::text()
                        } else {
                            colors::text_secondary()
                        })
                        .border_1()
                        .border_color(colors::border())
                        .cursor_pointer()
                        .hover(|s| s.bg(colors::bg()))
                        .child("Don't Allow")
                        .on_click(move |_ev, _window, cx| {
                            super::consent_dialog::set_consent(
                                mdict_rs::settings::TelemetryConsent::OptedOut,
                                cx,
                            );
                            deny_state.update(cx, |_s, cx| cx.notify());
                        }),
                )
                .child(
                    div()
                        .id("telemetry-allow-btn")
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
                        .on_click(move |_ev, _window, cx| {
                            super::consent_dialog::set_consent(
                                mdict_rs::settings::TelemetryConsent::OptedIn,
                                cx,
                            );
                            allow_state.update(cx, |_s, cx| cx.notify());
                        }),
                ),
        );

    let mut section = v_flex().gap(px(4.)).child(consent_note).child(buttons);
    if details_open {
        section = section.child(telemetry_details_panel(&settings));
    }
    section.into_any_element()
}

/// The Details panel: installation id, app version, and what is (not)
/// collected. Shown/hidden by the Details button above.
fn telemetry_details_panel(settings: &mdict_rs::settings::Settings) -> gpui::AnyElement {
    let install_id = settings
        .installation_id
        .clone()
        .unwrap_or_else(|| "(not set)".to_string());

    v_flex()
        .gap(px(8.))
        .mt(px(6.))
        .p(px(10.))
        .rounded(px(6.))
        .bg(colors::bg())
        .border_1()
        .border_color(colors::border())
        .child(details_row("App version", env!("APP_VERSION")))
        .child(details_row("Installation ID", &install_id))
        .child(privacy_list(
            "What we collect",
            &[
                "Lookups performed (count only, never the word)",
                "Pronunciation plays (count) and playback failures (reason)",
                "Dictionaries imported (count, never the name)",
                "Operating system, app version, system locale",
                "Errors during indexing/import (message, paths stripped)",
            ],
        ))
        .child(privacy_list(
            "What we DON'T collect",
            &[
                "Which words you look up",
                "Dictionary names or contents",
                "Your name, username, or any personal data",
                "Anything at all when this is turned off",
            ],
        ))
        .into_any_element()
}

fn details_row(label: &str, value: &str) -> gpui::AnyElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors::text())
                .child(SharedString::from(value.to_string())),
        )
        .into_any_element()
}

fn privacy_list(title: &str, items: &[&str]) -> gpui::AnyElement {
    let mut list = v_flex().gap(px(3.)).child(
        div()
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(colors::text())
            .child(SharedString::from(title.to_string())),
    );
    for item in items {
        list = list.child(
            h_flex()
                .gap(px(6.))
                .text_size(px(11.))
                .text_color(colors::text_secondary())
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors::text_secondary())
                        .child("•"),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors::text_secondary())
                        .child(SharedString::from(*item)),
                ),
        );
    }
    list.into_any_element()
}
