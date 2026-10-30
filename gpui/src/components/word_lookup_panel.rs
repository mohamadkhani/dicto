//! Word Lookup settings tab.
//!
//! Word Lookup is its own feature, separate from Quick Translate: select a
//! word anywhere, press the global hotkey, and a small popup shows the
//! definition from the local MDict dictionaries — no network, no API key.

use gpui::{
    AppContext as _, AsyncApp, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, div, px,
};
use gpui_component::{h_flex, scroll::ScrollableElement, v_flex};

use crate::{colors, state::DictState};

use super::quick_translate_panel::{divider, toggle_switch};

/// Build the Word Lookup settings tab content.
pub fn word_lookup_tab_content(state: Entity<DictState>, cx: &mut gpui::App) -> gpui::AnyElement {
    let settings = state.read(cx).word_lookup.clone();
    let enabled = settings.enabled;

    let header = v_flex()
        .gap(px(4.))
        .pb(px(16.))
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::text())
                .child(SharedString::from("Word Lookup")),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(
                    "Select a word anywhere and press a global hotkey to see its \
                     definition from your local dictionaries — works offline.",
                )),
        );

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
            set_word_lookup_enabled(toggle_state.clone(), !enabled, cx);
        }));

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
                .child(SharedString::from(settings.hotkey.clone())),
        );

    // Backend note (same treatment as the translate hotkey's note).
    let backend = state
        .read(cx)
        .word_lookup_engine
        .as_ref()
        .map(|e| e.backend_name().to_string())
        .unwrap_or_else(|| "none".to_string());
    let backend_note = div()
        .text_size(px(11.))
        .text_color(colors::text_secondary())
        .child(SharedString::from(format!(
            "Hotkey backend: {}",
            match backend.as_str() {
                "x11" => "X11 (fully supported)",
                "windows" => "Windows (fully supported)",
                "tray_menu" => "Tray menu only — use the tray icon to look up",
                other => other,
            }
        )));

    // Smart-dispatch note: the translate hotkey already routes single words
    // here when the feature is enabled.
    let note = div()
        .text_size(px(11.))
        .text_color(colors::text_secondary())
        .child(SharedString::from(
            "With this enabled, the Quick Translate hotkey also opens the lookup \
             when you select a single word; phrases still translate.",
        ));

    let registering = state.read(cx).word_lookup_registering;

    let mut body = v_flex()
        .gap(px(4.))
        .w_full()
        .h_full()
        .p(px(4.))
        .id("wl-settings-scroll")
        .overflow_y_scrollbar()
        .child(header)
        .child(divider())
        .child(enable_row);

    if registering {
        body = body.child(
            div()
                .text_size(px(11.))
                .text_color(colors::text_secondary())
                .child(SharedString::from("Registering hotkey…")),
        );
    }
    if enabled {
        body = body.child(hotkey_row).child(backend_note);
    }
    body = body.child(note);

    // Outer div participates in the parent's flex layout (flex_1 = remaining
    // height); the scroll wrapper is the inner body.
    div()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .child(body)
        .into_any_element()
}

/// Enable or disable Word Lookup.
///
/// Enabling registers the hotkey off the UI thread: the XDG portal backend
/// blocks for up to a few seconds on its first bind, which would otherwise
/// freeze the window. `word_lookup_registering` drives a "Registering…"
/// hint until the background task finishes.
pub fn set_word_lookup_enabled(state: Entity<DictState>, enabled: bool, cx: &mut gpui::App) {
    let attempt = state.update(cx, |s, cx| {
        s.word_lookup.enabled = enabled;
        s.word_lookup_registering = enabled;
        s.word_lookup_toggle_gen = s.word_lookup_toggle_gen.wrapping_add(1);
        s.save_settings(cx);

        // Drop the current manager up front (unregistering is cheap); the
        // enabled case re-attaches a fresh one once the background task
        // completes.
        if let Some(engine) = s.word_lookup_engine.as_mut() {
            engine.set_hotkey_manager(None);
            engine.set_settings(s.word_lookup.clone());
        }
        cx.notify();
        s.word_lookup_toggle_gen
    });

    if !enabled {
        return;
    }

    let settings = state.read(cx).word_lookup.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let manager = cx
            .background_executor()
            .spawn(async move { crate::word_lookup::make_hotkey_manager(settings) })
            .await;

        cx.update(|cx| {
            cx.update_entity(&state, |s, cx| {
                // A newer toggle (or a disable) superseded this attempt:
                // drop the manager instead of attaching a stale one.
                if s.word_lookup_toggle_gen != attempt || !s.word_lookup.enabled {
                    return;
                }
                if let Some(engine) = s.word_lookup_engine.as_mut() {
                    engine.set_hotkey_manager(manager);
                    engine.set_settings(s.word_lookup.clone());
                }
                s.word_lookup_registering = false;
                cx.notify();
            });
        });
    })
    .detach();
}
