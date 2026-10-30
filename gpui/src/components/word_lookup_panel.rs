//! Word Lookup enable flow.
//!
//! Word Lookup is its own feature, separate from Quick Translate: select a
//! word anywhere, press the global hotkey, and a small popup shows the
//! definition from the local MDict dictionaries — no network, no API key.
//! The enable toggle + hotkey display live in the General settings tab
//! (see [`super::general_panel`]); this module owns the async enable flow.

use gpui::{AppContext as _, AsyncApp, Entity};

use crate::state::DictState;

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
