//! System Shortcuts settings tab.
//!
//! Dicto can register the global shortcuts for *both* quick features
//! (Word Lookup and Quick Translate) with the desktop itself, so they work
//! from anywhere. This tab owns that shared auto-registration toggle and
//! shows the live OS-level status.

use gpui::{
    Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
    px,
};
use gpui_component::{h_flex, scroll::ScrollableElement, v_flex};

use crate::{colors, state::DictState};

use super::quick_translate_panel::divider;
#[cfg(target_os = "linux")]
use super::quick_translate_panel::toggle_switch;

/// Build the System Shortcuts settings tab content.
pub fn shortcuts_tab_content(state: Entity<DictState>, cx: &mut gpui::App) -> gpui::AnyElement {
    let header = v_flex()
        .gap(px(4.))
        .pb(px(16.))
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::text())
                .child(SharedString::from("System Shortcuts")),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(
                    "Dicto can register its global shortcuts with the desktop itself, \
                     so they work from anywhere — no manual setup.",
                )),
        );

    let mut body = v_flex()
        .gap(px(4.))
        .w_full()
        .h_full()
        .p(px(4.))
        .id("shortcuts-settings-scroll")
        .overflow_y_scrollbar()
        .child(header)
        .child(divider());

    #[cfg(target_os = "linux")]
    {
        body = body.child(system_shortcuts_section(&state, cx));
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (&state, cx);
        body = body.child(
            div()
                .pt(px(8.))
                .text_size(px(11.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(
                    "No extra setup is needed on this platform — shortcuts are \
                     registered directly with the window system.",
                )),
        );
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .child(body)
        .into_any_element()
}

/// Auto-definition toggle plus the live status of the OS-level shortcut
/// layer (portal / GNOME keybindings / manual snippet). Backed by
/// `hotkey::os_binding`.
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
