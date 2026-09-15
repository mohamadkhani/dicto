use std::time::Duration;

use gpui::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    Root, TitleBar, WindowExt, h_flex,
    input::InputState,
    tab::{Tab, TabBar},
    v_flex,
};

use crate::colors;
use crate::components::{
    detail_panel,
    search_bar::{self, SearchBarProps},
    word_list::{self, WordListProps},
};
use crate::state::{DictResult, DictState};

pub struct DictApp {
    pub state: Entity<DictState>,
    input: Entity<InputState>,
}

impl DictApp {
    pub fn new(state: Entity<DictState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder("Search words...", window, cx);
            s
        });

        let dict_state = state.clone();
        cx.observe(&input, move |this: &mut DictApp, input, cx| {
            let text = input.read(cx).value().to_string();
            if text.is_empty() {
                cx.update_entity(&this.state, |s, cx| {
                    s.suggestions.clear();
                    s.selected_suggestion = None;
                    cx.notify();
                });
                return;
            }

            let dict_state = dict_state.clone();
            cx.spawn(async move |_this, cx| {
                // Debounce: wait for the user to stop typing before fetching
                // suggestions. 150ms is short enough to feel responsive but
                // long enough to avoid hammering the FST on every keystroke.
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;

                let q = text.clone();
                let query_len = q.len();
                let suggestions = cx
                    .background_executor()
                    .spawn(async move { mdict_rs::query::search_suggestions(&q, 50) })
                    .await;

                // Auto-select the first suggestion for queries of 3+ chars.
                // For shorter queries the list is shown but nothing is
                // selected, avoiding jarring previews on single-letter input.
                let auto_select = if query_len >= 3 && !suggestions.is_empty() {
                    Some(0)
                } else {
                    None
                };

                cx.update_entity(&dict_state, |s, cx| {
                    let changed = s.suggestions != suggestions;
                    s.suggestions = suggestions;
                    if changed {
                        s.selected_suggestion = auto_select;
                    }
                    cx.notify();
                });

                // Debounced definition preview: load the definition only
                // after the user pauses typing for 200ms. This prevents
                // rapid-fire definition parsing (which involves HTML
                // parsing, CSS matching, and MDD resource lookups) on
                // every intermediate keystroke.
                if auto_select.is_some() {
                    // Read the first suggestion to preview.
                    let Some(word) =
                        cx.update_entity(&dict_state, |s, _cx| s.suggestions.first().cloned())
                    else {
                        return;
                    };

                    cx.background_executor()
                        .timer(Duration::from_millis(200))
                        .await;

                    // Re-read state after the quiet period — if the
                    // user typed more, the suggestions will have
                    // changed and we should skip this stale preview.
                    let should_preview = cx.update_entity(&dict_state, |s, _cx| {
                        s.selected_suggestion == Some(0)
                            && s.result_word.as_deref() != Some(word.as_str())
                    });

                    if should_preview {
                        let word_for_result = word.clone();
                        dicto_telemetry::get().track(dicto_telemetry::Event::LookupPerformed {
                            source: dicto_telemetry::LookupSource::AutoPreview,
                        });
                        let results = cx
                            .background_executor()
                            .spawn(async move {
                                mdict_rs::query::query_all(&word)
                                    .into_iter()
                                    .map(|hit| {
                                        let blocks =
                                            crate::html::parse_styled(&hit.definition, &hit.stem);
                                        DictResult {
                                            short_name: hit.short_name,
                                            blocks,
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .await;

                        cx.update_entity(&dict_state, |s, cx| {
                            if s.selected_suggestion == Some(0) {
                                s.result_word = Some(word_for_result);
                                s.is_searching = false;
                                s.results = results;
                                s.active_result = 0;
                            }
                            cx.notify();
                        });
                    }
                }
            })
            .detach();
        })
        .detach();

        // Focus the search input on startup
        cx.update_entity(&input, |input_state, cx| {
            input_state.focus(window, cx);
        });

        // First-run telemetry consent: if the user hasn't decided yet (fresh
        // install, or an upgrader whose pre-telemetry settings.toml defaulted
        // the field to Undecided), surface a one-time consent dialog. The
        // dialog records OptedIn / OptedOut; closing it (without choosing) is
        // treated as OptedOut so it never nags.
        //
        // Deferred because `open_dialog` ultimately calls
        // `gpui_component::Root::update`, which requires the window's root
        // view to already be a `Root`. At construction time the root is the
        // `DictApp` being built here — `Root::new(view, ...)` only becomes the
        // window root *after* this constructor returns. `window.defer` runs
        // the open at the end of the current effect cycle, by which point the
        // root is committed.
        if matches!(
            mdict_rs::settings::current().telemetry_consent,
            mdict_rs::settings::TelemetryConsent::Undecided
        ) {
            window.defer(cx, |window, cx| {
                crate::components::consent_dialog::open_consent_dialog(window, cx);
            });
        }

        // Start periodic polling for quick-translate hotkey events and
        // tray-menu triggers. Runs every 100ms while the app is alive.
        let poll_state = state.clone();
        cx.spawn(async move |_this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;

                // Check the tray menu trigger flag
                let tray_triggered = crate::TRAY_TRANSLATE_TRIGGERED
                    .swap(false, std::sync::atomic::Ordering::Acquire);

                // Check hotkey events and tray flag. `poll()` opens the popup
                // for hotkey activations; the tray/IPC flag is a separate manual
                // trigger. Both end up in an `Idle` state showing the selection
                // + a Translate button — the translation only runs on click.
                let triggered = cx.update_entity(&poll_state, |s, _cx| {
                    let tts_key = crate::playback::tts_key(&s.quick_translate.tts);
                    let (triggered, new_text) =
                        if let Some(engine) = s.quick_translate_engine.as_mut() {
                            let hotkey_fired = engine.poll();
                            if hotkey_fired || tray_triggered {
                                // `poll()` already opened the popup for hotkey
                                // events; for the tray flag we trigger manually.
                                if !hotkey_fired {
                                    engine.trigger_translate();
                                }
                                // Read the new text from popup_status (now set
                                // to the new selection's Idle state).
                                let new_text: String = match engine.popup_status() {
                                    crate::quick_translate::PopupStatus::Visible(
                                        crate::components::translate_popup::PopupState::Idle {
                                            original,
                                        },
                                    ) => original.clone(),
                                    _ => String::new(),
                                };
                                (true, new_text)
                            } else {
                                (false, String::new())
                            }
                        } else {
                            (false, String::new())
                        };
                    // A new selection (or a re-trigger) makes any TTS clip
                    // speaking the previous selection stale — stop it. The
                    // translation slot is also keyed to the popup's
                    // original, so it's stale too.
                    if triggered {
                        if new_text.is_empty() {
                            s.invalidate_tts_clips(None, Some(&tts_key));
                        } else {
                            s.invalidate_tts_clips(Some(&new_text), Some(&tts_key));
                        }
                    }
                    triggered
                });

                // The popup is open whenever the engine status is Visible.
                // Reopen it whenever it is visible but has no window — this
                // both opens the first time AND replaces the window after a
                // tokenless re-trigger closed it on the previous tick (the
                // tick gap lets GPUI finish tearing the old surface down;
                // removing + opening in one tick leaves the new window
                // unpainted/black).
                let popup_visible = cx.read_entity(&poll_state, |s, _cx| {
                    matches!(
                        s.quick_translate_engine.as_ref().map(|e| e.popup_status()),
                        Some(crate::quick_translate::PopupStatus::Visible(_))
                    )
                });
                let has_window =
                    cx.read_entity(&poll_state, |s, _cx| s.qt_popup_window.is_some());
                // True while a tokenless re-trigger has closed the window and
                // is waiting for its reopen tick (the window-closed observer
                // hides the engine otherwise).
                let replace_pending = cx
                    .read_entity(&poll_state, |s, _cx| s.qt_replace_pending);
                let wants_popup = popup_visible || replace_pending;

                let mut replaced_this_tick = false;
                // Drain the tray token EVERY tick: it is only meaningful in
                // the same tick as its trigger (the tray click sets both).
                // A token left over from an earlier click is stale — Mutter
                // silently ignores activations with it, so letting it take
                // the activate branch would swallow the whole trigger.
                let tray_token = crate::take_tray_translate_token();
                tracing::debug!(
                    triggered, popup_visible, has_window,
                    tray_token = tray_token.is_some(),
                    "qt poll tick"
                );
                if triggered && popup_visible && has_window {
                    if let Some(token) = tray_token {
                        cx.update(|cx: &mut gpui::App| {
                            activate_popup_window(&poll_state, &token, cx)
                        });
                    } else {
                        // Keyboard-shortcut / IPC re-trigger: close the old
                        // popup now; the next poll tick opens a fresh one
                        // (freshly mapped windows get keyboard focus on
                        // GNOME/Mutter, activated ones don't).
                        cx.update(|cx: &mut gpui::App| close_popup_window(&poll_state, cx));
                        replaced_this_tick = true;
                    }
                }

                if wants_popup && !replaced_this_tick && !has_window {
                    let res = cx.update(|cx: &mut gpui::App| {
                        open_translate_popup(&poll_state, cx)
                    });
                    if let Err(e) = res {
                        tracing::error!(error = %e, "failed to open translate popup");
                    }
                }
            }
        })
        .detach();

        Self { state, input }
    }

    pub fn lookup_word(&mut self, word: String, cx: &mut Context<Self>) {
        if word.is_empty() {
            return;
        }
        dicto_telemetry::get().track(dicto_telemetry::Event::LookupPerformed {
            source: dicto_telemetry::LookupSource::Keyboard,
        });

        cx.update_entity(&self.state, |s, cx| {
            s.result_word = Some(word.clone());
            s.results.clear();
            s.active_result = 0;
            s.is_searching = true;
            cx.notify();
        });

        let dict_state = self.state.clone();
        cx.spawn(async move |_this, cx| {
            let q = word.clone();
            let results = cx
                .background_executor()
                .spawn(async move {
                    mdict_rs::query::query_all(&q)
                        .into_iter()
                        .map(|hit| {
                            let blocks = crate::html::parse_styled(&hit.definition, &hit.stem);
                            DictResult {
                                short_name: hit.short_name,
                                blocks,
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .await;

            cx.update_entity(&dict_state, |s, cx| {
                s.is_searching = false;
                s.results = results;
                s.active_result = 0;
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for DictApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Open the init dialog on first render (after Root is initialized)
        if self.state.read(cx).show_import_modal {
            cx.update_entity(&self.state, |s, _| {
                s.show_import_modal = false;
            });
            open_get_dictionaries_dialog(self.state.clone(), window, cx);
        }

        // Root keeps active dialogs in a list, but doesn't render them
        // automatically — we have to append the dialog layer as a
        // sibling of the main view ourselves.
        let dialog_layer = Root::render_dialog_layer(window, cx);

        let input_handle = self.input.clone();
        let main = v_flex()
            .size_full()
            .bg(colors::bg())
            .child(
                TitleBar::new()
                    .on_close_window(|_, window, _| {
                        dicto_telemetry::get().track(dicto_telemetry::Event::WindowClosed);
                        window.remove_window();
                    })
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(colors::primary())
                            .child("Dicto"),
                    ),
            )
            // Search row hosts the cog on its right edge so the button
            // sits outside the title bar's OS-claimed drag region.
            .child(search_bar::search_bar(
                SearchBarProps {
                    input: self.input.clone(),
                    state: self.state.clone(),
                    right_slot: Some(cog_button(self.state.clone())),
                },
                cx,
            ))
            .child(indexing_bar(&self.state, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .w_full()
                    .child(word_list::word_list(
                        WordListProps {
                            state: self.state.clone(),
                        },
                        cx,
                    ))
                    .child(detail_panel::detail_panel(self.state.clone(), cx)),
            );

        div()
            .size_full()
            .on_key_down(cx.listener(move |_this, event: &KeyDownEvent, window, cx| {
                let m = &event.keystroke.modifiers;
                let key = event.keystroke.key.as_str();

                if m.control && (key == "l" || key == "f") {
                    // Ctrl+L / Ctrl+F: focus search input
                    cx.update_entity(&input_handle, |input, cx| {
                        input.focus(window, cx);
                    });
                } else if key == "escape" {
                    // Escape: clear the search field
                    cx.update_entity(&input_handle, |input, cx| {
                        input.set_value("", window, cx);
                    });
                } else if key == "f1" {
                    open_about_dialog(window, cx);
                }
            }))
            .child(main)
            .children(dialog_layer)
            .into_any_element()
    }
}

/// Open (or refresh) the Quick Translate popup window.
///
/// The popup is a borderless `WindowKind::PopUp` that reads its content from
/// the shared `DictState`'s `popup_status`, so once it exists we only need to
/// notify it to re-render with the latest translation result.
fn open_translate_popup(state: &Entity<DictState>, cx: &mut gpui::App) -> anyhow::Result<()> {
    use gpui::{
        Bounds, WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowKind,
        WindowOptions, size,
    };

    // Open the popup at (roughly) its natural height. Creating it at a
    // default 560px and shrinking after the first paint visibly moved the
    // popup upward (window resizes anchor the top edge). Text sections have
    // fixed heights, so the estimate lands within a few px; the poll tick
    // corrects the remainder from the real painted measurements. Resizable
    // as an escape hatch.
    let status = state
        .read(cx)
        .quick_translate_engine
        .as_ref()
        .map(|e| e.popup_status().clone());
    let initial_height =
        crate::components::translate_popup::estimated_window_height(match &status {
            Some(crate::quick_translate::PopupStatus::Visible(popup_state)) => Some(popup_state),
            _ => None,
        });
    let bounds = Bounds::centered(None, size(px(460.), px(initial_height)), cx);
    let state_for_window = state.clone();

    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_decorations: Some(WindowDecorations::Client),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Dicto Translate".into()),
                ..Default::default()
            }),
            kind: WindowKind::PopUp,
            is_resizable: true,
            is_minimizable: false,
            focus: true,
            show: true,
            app_id: Some("dicto".into()),
            // The popup paints its own rounded card; everything outside the
            // card must be real transparency. Since the gpui fork bump the
            // popup is a real xdg_popup on Wayland, and Mutter no longer
            // rounds its corners for us — an opaque window would show a
            // black square behind the card's rounded corners.
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        },
        |window, cx| {
            cx.new(|cx| {
                let view = cx.new(|cx| {
                    crate::components::translate_popup::TranslatePopupView::new(
                        state_for_window.clone(),
                        window,
                        cx,
                    )
                });
                // `Root` wrapper: the Original text editor (gpui-component
                // `Input`) requires the window's root view to be a `Root`.
                // `bordered(false)`: the popup card paints its own border and
                // rounding; Root's client-decoration border would draw a
                // second frame around it. `bg(transparent)`: Root's default
                // theme background (near-black in dark mode) would otherwise
                // fill the corners outside the rounded card.
                gpui_component::Root::new(view, window, cx)
                    .bordered(false)
                    .bg(gpui::transparent_black())
            })
        },
    )?;

    state.update(cx, |s, _cx| {
        s.qt_popup_window = Some(handle);
        s.qt_replace_pending = false;
    });
    tracing::info!("translate popup: created new window");

    Ok(())
}

/// Close the popup window only — the engine keeps its (new) popup state and
/// the poll loop reopens a fresh window on the next tick. The replace-pending
/// flag makes the window-closed observer keep the popup state alive.
fn close_popup_window(state: &Entity<DictState>, cx: &mut gpui::App) {
    // Flag + detach the handle BEFORE removing: the window-closed observer
    // fires inside `remove_window`'s update and must see replace_pending
    // (and no stored handle) to leave the popup state untouched.
    let handle = state.read(cx).qt_popup_window;
    state.update(cx, |s, _cx| {
        s.qt_replace_pending = true;
        s.qt_popup_window = None;
    });
    if let Some(handle) = handle {
        let _ = handle.update(cx, |_, window, _cx| {
            window.remove_window();
        });
    }
}

/// Raise + focus the existing popup with a compositor-minted token (tray
/// click). Clears the stored handle if the window is already gone.
fn activate_popup_window(state: &Entity<DictState>, token: &str, cx: &mut gpui::App) {
    let Some(handle) = state.read(cx).qt_popup_window else {
        return;
    };
    let token = token.to_string();
    let alive = handle
        .update(cx, move |_view, window, cx| {
            window.activate_with_token(&token);
            cx.notify();
        })
        .is_ok();
    if alive {
        tracing::info!("translate popup: activating existing window (tray token)");
    } else {
        tracing::warn!("translate popup: stale window handle on activation");
        state.update(cx, |s, _cx| s.qt_popup_window = None);
    }
}

/// Slim progress bar shown while background indexing is running.
/// Returns an empty fragment when `indexing_total == 0` so we don't
/// reserve vertical space in the idle state.
fn indexing_bar(state: &Entity<DictState>, cx: &Context<DictApp>) -> gpui::AnyElement {
    let s = state.read(cx);
    if s.indexing_total == 0 {
        return div().into_any_element();
    }

    let done = s.indexing_done;
    let total = s.indexing_total;
    let pct = if total == 0 {
        0.0
    } else {
        (done as f32 / total as f32).clamp(0.0, 1.0)
    };
    let label = match &s.indexing_current {
        Some(name) => format!("Indexing {done}/{total} — {name}"),
        None => format!("Indexing {done}/{total}"),
    };

    v_flex()
        .w_full()
        .px(px(12.))
        .py(px(6.))
        .gap(px(4.))
        .bg(colors::surface())
        .border_b_1()
        .border_color(colors::border())
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(label)),
        )
        .child(
            div()
                .w_full()
                .h(px(4.))
                .rounded(px(2.))
                .bg(colors::border())
                .child(
                    div()
                        .h(px(4.))
                        .rounded(px(2.))
                        .bg(colors::primary())
                        .w(gpui::relative(pct)),
                ),
        )
        .into_any_element()
}

fn cog_button(state: Entity<DictState>) -> gpui::AnyElement {
    div()
        .id("cog-settings-btn")
        .px(px(10.))
        .py(px(4.))
        .mr(px(8.))
        .rounded(px(6.))
        .text_size(px(12.))
        .text_color(colors::text())
        .bg(colors::bg())
        .border_1()
        .border_color(colors::border())
        .cursor_pointer()
        .hover(|s| s.bg(colors::surface()))
        .child(SharedString::from("\u{2699} Settings"))
        .on_click(move |_, window, cx| {
            let state = state.clone();
            dicto_telemetry::get().track(dicto_telemetry::Event::SettingsOpened {
                source: dicto_telemetry::SettingsSource::GearButton,
            });

            window.open_dialog(cx, move |dialog, _window, _cx| {
                let state = state.clone();

                dialog
                    .title(div().child("Settings"))
                    .w_full()
                    .h(px(560.))
                    .close_button(true)
                    .overlay_closable(true)
                    .content(move |content, window, cx| {
                        let active_tab = state.read(cx).settings_active_tab;

                        content.child(
                            v_flex()
                                .w_full()
                                .h_full()
                                .gap(px(12.))
                                .child(crate::components::settings_window::header_tabs_for_dialog(
                                    state.clone(),
                                    active_tab,
                                    cx,
                                ))
                                .child(if active_tab == 0 {
                                    crate::components::settings_panel::dictionaries_tab_content(
                                        state.clone(),
                                        cx,
                                    )
                                } else if active_tab == 1 {
                                    let is_importing =
                                        state.read(cx).import_files.iter().any(|f| {
                                            matches!(
                                                f.status,
                                                crate::state::ImportStatus::Copying
                                                    | crate::state::ImportStatus::Indexing
                                            )
                                        });
                                    crate::components::import_panel::import_panel_content(
                                        state.clone(),
                                        is_importing,
                                        cx,
                                    )
                                } else if active_tab == 2 {
                                    crate::components::download_panel::download_tab_content(
                                        state.clone(),
                                        window,
                                        cx,
                                    )
                                } else if active_tab == 3 {
                                    crate::components::quick_translate_panel::quick_translate_tab_content(
                                        state.clone(),
                                        window,
                                        cx,
                                    )
                                } else if active_tab == 4 {
                                    crate::components::settings_panel::telemetry_tab_content(
                                        state.clone(),
                                        cx,
                                    )
                                } else if active_tab == 5 {
                                    crate::components::about_panel::panel_content()
                                } else {
                                    let is_importing =
                                        state.read(cx).import_files.iter().any(|f| {
                                            matches!(
                                                f.status,
                                                crate::state::ImportStatus::Copying
                                                    | crate::state::ImportStatus::Indexing
                                            )
                                        });
                                    crate::components::import_panel::import_panel_content(
                                        state.clone(),
                                        is_importing,
                                        cx,
                                    )
                                }),
                        )
                    })
            });
        })
        .into_any_element()
}

fn open_get_dictionaries_dialog(
    state: Entity<DictState>,
    window: &mut Window,
    cx: &mut Context<DictApp>,
) {
    let s = state;
    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .title(div().child("Get Dictionaries"))
            .w_full()
            .h(px(560.))
            .close_button(true)
            .overlay_closable(false)
            .content({
                let s = s.clone();
                move |content, window, cx| {
                    let active_tab = s.read(cx).import_modal_tab;
                    let is_importing = s.read(cx).import_files.iter().any(|f| {
                        matches!(
                            f.status,
                            crate::state::ImportStatus::Copying
                                | crate::state::ImportStatus::Indexing
                        )
                    });

                    content.child(
                        v_flex()
                            .w_full()
                            .gap(px(12.))
                            .child(
                                h_flex().w_full().child(
                                    TabBar::new("import-modal-tabs")
                                        .underline()
                                        .selected_index(active_tab)
                                        .cursor_pointer()
                                        .on_click({
                                            let ts = s.clone();
                                            move |&ix, _window, cx| {
                                                cx.update_entity(&ts, |s, cx| {
                                                    s.import_modal_tab = ix;
                                                    cx.notify();
                                                });
                                            }
                                        })
                                        .child(Tab::new().label("Download"))
                                        .child(Tab::new().label("Import")),
                                ),
                            )
                            .child(if active_tab == 0 {
                                crate::components::download_panel::download_tab_content(
                                    s.clone(),
                                    window,
                                    cx,
                                )
                            } else {
                                crate::components::import_panel::import_panel_content(
                                    s.clone(),
                                    is_importing,
                                    cx,
                                )
                            }),
                    )
                }
            })
            .footer(
                h_flex().justify_end().child(
                    div()
                        .id("import-modal-done-btn")
                        .px(px(14.))
                        .py(px(7.))
                        .rounded(px(6.))
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors::bg())
                        .bg(colors::primary())
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.85))
                        .child("Done")
                        .on_click(|_, window, cx| {
                            window.close_dialog(cx);
                        }),
                ),
            )
    });
}

fn open_about_dialog(window: &mut Window, cx: &mut Context<DictApp>) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .title(div().child("About"))
            .w_full()
            .h(px(560.))
            .close_button(true)
            .overlay_closable(true)
            .content(move |content, _window, _cx| {
                content.child(crate::components::about_panel::panel_content())
            })
            .footer(
                h_flex().justify_end().child(
                    div()
                        .id("about-close-btn")
                        .px(px(14.))
                        .py(px(7.))
                        .rounded(px(6.))
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors::bg())
                        .bg(colors::primary())
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.85))
                        .child("Close")
                        .on_click(|_, window, cx| {
                            window.close_dialog(cx);
                        }),
                ),
            )
    });
}
