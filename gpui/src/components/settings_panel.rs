use gpui::prelude::FluentBuilder;
use gpui::{
    AppContext as _, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::{Sizable, WindowExt, h_flex, input::Input, scroll::ScrollableElement, v_flex};
use mdict_rs::settings::DictEntry;
use tracing::{info, warn};

use crate::{colors, state::DictState};

/// Build the dictionary list UI using the Table component.
pub fn dictionaries_tab_content(state: Entity<DictState>, cx: &mut gpui::App) -> gpui::AnyElement {
    let snapshot = state.read(cx).dictionaries.clone();

    let header_text = div()
        .text_size(px(12.))
        .text_color(colors::text_secondary())
        .child(SharedString::from(
            "Toggle dictionaries on or off, reorder, or view details.",
        ));

    let mut rows: Vec<gpui::AnyElement> = Vec::new();

    if snapshot.is_empty() {
        rows.push(
            div()
                .py(px(20.))
                .text_size(px(13.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(
                    "No .mdx files found. Use the Import tab to add dictionaries.",
                ))
                .into_any_element(),
        );
    } else {
        let total = snapshot.len();
        for (i, entry) in snapshot.iter().enumerate() {
            rows.push(dict_row(i, entry, total, state.clone()));
        }
    }

    let save_state = state.clone();

    v_flex()
        .w_full()
        .flex_1()
        .gap(px(10.))
        .child(header_text)
        .child(
            v_flex()
                .id("settings-dict-list")
                .w_full()
                .flex_1()
                .overflow_y_scrollbar()
                .child(
                    v_flex()
                        .w_full()
                        .gap(px(1.))
                        .child(dict_header_row())
                        .children(rows),
                ),
        )
        .child(
            h_flex()
                .justify_end()
                .gap(px(8.))
                .child(
                    div()
                        .id("settings-cancel-btn")
                        .px(px(14.))
                        .py(px(7.))
                        .rounded(px(6.))
                        .text_size(px(13.))
                        .text_color(colors::text_secondary())
                        .border_1()
                        .border_color(colors::border())
                        .cursor_pointer()
                        .hover(|s| s.bg(colors::bg()))
                        .child("Cancel")
                        .on_click(|_, window, cx| {
                            window.close_dialog(cx);
                        }),
                )
                .child(
                    div()
                        .id("settings-save-btn")
                        .px(px(14.))
                        .py(px(7.))
                        .rounded(px(6.))
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors::bg())
                        .bg(colors::primary())
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.85))
                        .child("Save")
                        .on_click(move |_, window, cx| {
                            apply_save(&save_state, cx);
                            window.close_dialog(cx);
                        }),
                ),
        )
        .into_any_element()
}

fn dict_header_row() -> gpui::AnyElement {
    h_flex()
        .w_full()
        .px(px(8.))
        .py(px(6.))
        .text_size(px(11.))
        .text_color(colors::text_secondary())
        .border_b_1()
        .border_color(colors::border())
        .child(div().w(px(28.)).flex_shrink_0())
        .child(div().flex_1().child("Display Nameca"))
        .child(div().w(px(44.)).flex_shrink_0().text_center().child(""))
        .child(div().w(px(28.)).flex_shrink_0().text_center().child(""))
        .into_any_element()
}

fn dict_row(
    idx: usize,
    entry: &DictEntry,
    total: usize,
    state: Entity<DictState>,
) -> gpui::AnyElement {
    let full_title = mdict_rs::formats::mdict::mdx_header_title(&entry.path);
    let auto_short = if full_title.len() <= 25 {
        full_title.clone()
    } else {
        abbreviate_display(&full_title)
    };
    let display_name = if entry.short_name.is_empty() {
        auto_short
    } else {
        entry.short_name.clone()
    };
    let enabled = entry.enabled;

    let toggle_state = state.clone();
    let detail_state = state.clone();
    let up_state = state.clone();
    let down_state = state.clone();

    h_flex()
        .w_full()
        .px(px(8.))
        .py(px(5.))
        .items_center()
        .hover(|s| s.bg(colors::surface()))
        // Checkbox
        .child(
            div()
                .w(px(28.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .child(toggle_checkbox(idx, enabled, toggle_state)),
        )
        // Dictionary name
        .child(
            div()
                .flex_1()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::text())
                .child(SharedString::from(display_name)),
        )
        // Arrows
        .child(
            div()
                .w(px(44.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    h_flex()
                        .gap(px(2.))
                        .child(arrow_button("up", idx, idx == 0, move |cx| {
                            cx.update_entity(&up_state, |s, cx| {
                                if idx > 0 && idx < s.dictionaries.len() {
                                    s.dictionaries.swap(idx, idx - 1);
                                    cx.notify();
                                }
                            });
                        }))
                        .child(arrow_button("down", idx, idx + 1 >= total, move |cx| {
                            cx.update_entity(&down_state, |s, cx| {
                                if idx + 1 < s.dictionaries.len() {
                                    s.dictionaries.swap(idx, idx + 1);
                                    cx.notify();
                                }
                            });
                        })),
                ),
        )
        // Detail button
        .child(
            div()
                .w(px(28.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .child(detail_button(idx, detail_state)),
        )
        .into_any_element()
}

fn toggle_checkbox(idx: usize, enabled: bool, state: Entity<DictState>) -> gpui::AnyElement {
    div()
        .id(SharedString::from(format!("dict-toggle-{idx}")))
        .flex()
        .items_center()
        .justify_center()
        .w(px(18.))
        .h(px(18.))
        .rounded(px(4.))
        .border_1()
        .border_color(if enabled {
            colors::primary()
        } else {
            colors::border()
        })
        .bg(if enabled {
            colors::primary()
        } else {
            colors::bg()
        })
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(colors::bg())
        .child(if enabled { "\u{2713}" } else { "" })
        .on_click(move |_, _, cx| {
            cx.update_entity(&state, |s, cx| {
                if let Some(d) = s.dictionaries.get_mut(idx) {
                    d.enabled = !d.enabled;
                }
                cx.notify();
            });
        })
        .into_any_element()
}

fn detail_button(idx: usize, state: Entity<DictState>) -> gpui::AnyElement {
    let click_state = state.clone();
    div()
        .id(SharedString::from(format!("dict-detail-{idx}")))
        .flex()
        .items_center()
        .justify_center()
        .w(px(24.))
        .h(px(24.))
        .rounded(px(4.))
        .border_1()
        .border_color(colors::border())
        .bg(colors::bg())
        .cursor_pointer()
        .text_size(px(13.))
        .text_color(colors::text_secondary())
        .child("\u{2630}")
        .hover(|s| s.bg(colors::surface()))
        .on_click(move |_, window, cx| {
            let state = click_state.clone();
            let entry = state.read(cx).dictionaries.get(idx).cloned();
            let Some(entry) = entry else { return };

            let title = mdict_rs::formats::mdict::mdx_header_title(&entry.path);
            let (encoding, version, description) = read_header_meta(&entry.path);
            let auto_short = if title.len() <= 25 {
                title.clone()
            } else {
                abbreviate_display(&title)
            };
            let current = if entry.short_name.is_empty() {
                auto_short
            } else {
                entry.short_name.clone()
            };

            let input_entity = cx.new(|cx| gpui_component::input::InputState::new(window, cx));
            input_entity.update(cx, |is, cx| {
                is.set_value(current, window, cx);
            });

            let save_state = state.clone();
            let input_for_dialog = input_entity.clone();
            let input_for_ok = input_entity.clone();
            let idx = idx;

            window.open_dialog(cx, move |dialog, _window, _cx| {
                let input_entity = input_for_dialog.clone();
                let input_for_ok = input_for_ok.clone();
                let save_state = save_state.clone();
                let title = title.clone();
                let encoding = encoding.clone();
                let path = entry.path.clone();
                let description = description.clone();

                dialog
                    .title(div().pr(px(24.)).child(title.clone()))
                    .w(px(440.))
                    .close_button(true)
                    .overlay_closable(false)
                    .content(move |content, _window, _cx| {
                        content.child(
                            v_flex()
                                .gap(px(10.))
                                // Editable display name — first, with inline Apply
                                .child(
                                    v_flex().gap(px(4.)).child(label("Display name")).child(
                                        h_flex()
                                            .gap(px(6.))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .child(Input::new(&input_entity).small()),
                                            )
                                            .child(
                                                div()
                                                    .id("detail-apply-btn")
                                                    .h_full()
                                                    .px(px(12.))
                                                    .rounded(px(5.))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .text_size(px(12.))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(colors::bg())
                                                    .bg(colors::primary())
                                                    .cursor_pointer()
                                                    .hover(|s| s.opacity(0.85))
                                                    .child("Apply")
                                                    .on_click({
                                                        let input_for_ok = input_for_ok.clone();
                                                        let save_state = save_state.clone();
                                                        move |_, window, cx| {
                                                            let new_name = input_for_ok
                                                                .read(cx)
                                                                .value()
                                                                .to_string();
                                                            cx.update_entity(
                                                                &save_state,
                                                                |s, cx| {
                                                                    if let Some(d) =
                                                                        s.dictionaries.get_mut(idx)
                                                                    {
                                                                        d.short_name = new_name;
                                                                    }
                                                                    cx.notify();
                                                                },
                                                            );
                                                            window.close_dialog(cx);
                                                        }
                                                    }),
                                            ),
                                    ),
                                )
                                // Description (only if meaningful)
                                .when(
                                    !description.is_empty()
                                        && !is_redundant_desc(&title, &description),
                                    |el| {
                                        let desc = strip_html_tags(&description);
                                        el.child(
                                            v_flex()
                                                .gap(px(2.))
                                                .child(label("Description"))
                                                .child(value_text(&desc)),
                                        )
                                    },
                                )
                                // Metadata
                                .child(
                                    v_flex()
                                        .gap(px(6.))
                                        .child(labeled_row("Path", &path))
                                        .child(labeled_row("Encoding", &encoding))
                                        .child(labeled_row("Version", &format!("v{}", version))),
                                ),
                        )
                    })
            });
        })
        .into_any_element()
}

fn label(text: &str) -> gpui::AnyElement {
    div()
        .text_size(px(11.))
        .text_color(colors::text_secondary())
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

fn value_text(text: &str) -> gpui::AnyElement {
    div()
        .text_size(px(12.))
        .text_color(colors::text())
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

fn labeled_row(label_text: &str, value: &str) -> gpui::AnyElement {
    h_flex()
        .gap(px(8.))
        .child(div().w(px(80.)).child(label(label_text)))
        .child(div().flex_1().min_w(px(0.)).child(value_text(value)))
        .into_any_element()
}

fn is_redundant_desc(title: &str, desc: &str) -> bool {
    let clean = strip_html_tags(desc);
    let clean_lower = clean.to_lowercase();
    let title_lower = title.to_lowercase();
    clean_lower.starts_with(&title_lower.chars().take(30).collect::<String>())
        || title_lower.starts_with(&clean_lower.chars().take(30).collect::<String>())
}

/// Read encoding, version, description from MDX header without creating a full dictionary.
fn read_header_meta(path: &str) -> (String, u8, String) {
    let info = mdict_rs::formats::mdict::mdx_header_info(path);
    (info.encoding, info.version, info.description)
}

fn abbreviate_display(title: &str) -> String {
    let skip = ["the", "of", "a", "an", "and", "or", "for", "in", "on", "to"];
    let mut abbr = String::new();
    for word in title.split_whitespace() {
        let clean: String = word
            .chars()
            .filter(|c| c.is_alphabetic() || *c == '-' || *c == '\'')
            .collect();
        if clean.is_empty() || skip.contains(&clean.to_lowercase().as_str()) {
            continue;
        }
        if let Some(c) = clean.chars().next() {
            abbr.push(c.to_ascii_uppercase());
        }
    }
    abbr.truncate(10);
    abbr
}

fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn arrow_button(
    kind: &'static str,
    idx: usize,
    disabled: bool,
    on_click: impl Fn(&mut gpui::App) + 'static,
) -> gpui::AnyElement {
    let glyph = if kind == "up" { "\u{25B2}" } else { "\u{25BC}" };
    let id = SharedString::from(format!("arrow-{kind}-{idx}"));

    let mut el = div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(20.))
        .h(px(20.))
        .rounded(px(3.))
        .text_size(px(9.))
        .text_color(if disabled {
            colors::text_secondary()
        } else {
            colors::text()
        })
        .child(glyph);

    if !disabled {
        el = el
            .cursor_pointer()
            .hover(|s| s.bg(colors::surface()))
            .on_click(move |_, _, cx| on_click(cx));
    }
    el.into_any_element()
}

/// Persist the edited dictionary list, drop stale pools, and re-index
/// any newly enabled dictionary in the background.
pub fn apply_save(state: &Entity<DictState>, cx: &mut gpui::App) {
    let edited = state.read(cx).dictionaries.clone();
    let executor = cx.background_executor().clone();

    // Preserve telemetry consent + installation_id: start from the current
    // on-disk settings and overwrite only the dictionary list. (A bare
    // `Settings { dictionaries }` would wipe consent back to Undecided and
    // null the install id on every Save.)
    let mut new_settings = mdict_rs::settings::current();
    new_settings.dictionaries = edited;
    if let Err(e) = mdict_rs::settings::update(new_settings) {
        warn!("settings: save failed: {e}");
        return;
    }
    dicto_telemetry::get().track(dicto_telemetry::Event::SettingsChanged {
        change: dicto_telemetry::SettingsChange::DictionaryList,
    });
    mdict_rs::config::reset_pools();
    info!("settings: pools reset, kicking off background reindex");

    let mdx = mdict_rs::settings::enabled_mdx();
    executor
        .spawn(async move {
            for path in mdx {
                if let Some(dict) = mdict_rs::formats::detect(&path) {
                    if let Err(e) = dict.build_index(false) {
                        warn!("reindex failed for {path}: {e}");
                    }
                }
            }
            mdict_rs::registry::reload();
            info!("settings: background reindex complete");
        })
        .detach();

    cx.update_entity(state, |s, cx| {
        s.dictionaries = mdict_rs::settings::current().dictionaries;
        cx.notify();
    });
}

/// Telemetry / Privacy tab. Shows what is and isn't collected, a toggle for
/// anonymous usage data, the read-only anonymous installation id, and (in debug
/// builds only) a button to send a test event.
pub fn telemetry_tab_content(state: Entity<DictState>, _cx: &mut gpui::App) -> gpui::AnyElement {
    let settings = mdict_rs::settings::current();
    let opted_in = matches!(
        settings.telemetry_consent,
        mdict_rs::settings::TelemetryConsent::OptedIn
    );
    let install_id = settings
        .installation_id
        .clone()
        .unwrap_or_else(|| "(not set)".to_string());

    let toggle_state = state.clone();
    let id_for_test = install_id.clone();

    v_flex()
        .w_full()
        .flex_1()
        .gap(px(12.))
        .p(px(4.))
        // Header + explanation
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(
                    "Anonymous usage data helps improve Dicto. No personal information, \
                     dictionary contents, or looked-up words are ever collected.",
                )),
        )
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
        // Toggle row
        .child(
            h_flex()
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
                .child(toggle_switch("telemetry-toggle", opted_in, move |cx| {
                    let next = toggle_consent(opted_in);
                    if let Err(e) = mdict_rs::settings::update_consent(next) {
                        warn!("telemetry: failed to persist consent: {e}");
                    }
                    // Re-init immediately so opt-in/out takes effect now.
                    dicto_telemetry::init(
                        matches!(next, mdict_rs::settings::TelemetryConsent::OptedIn),
                        mdict_rs::settings::current()
                            .installation_id
                            .unwrap_or_default(),
                        env!("APP_VERSION").to_string(),
                    );
                    cx.update_entity(&toggle_state, |_s, cx| cx.notify());
                })),
        )
        // Anonymous ID (read-only, for transparency)
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .px(px(10.))
                .py(px(6.))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors::text_secondary())
                        .child(SharedString::from("Anonymous installation ID")),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors::text_secondary())
                        .child(SharedString::from(install_id)),
                ),
        )
        // Dev-only: send a test event to verify dashboard ingestion.
        .when(cfg!(debug_assertions), |el| {
            el.child(
                div()
                    .id("telemetry-test-btn")
                    .px(px(12.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .text_size(px(12.))
                    .text_color(colors::bg())
                    .bg(colors::primary())
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.85))
                    .child(SharedString::from("Send test event (dev)"))
                    .on_click(move |_, _, _cx| {
                        let _ = id_for_test.clone();
                        dicto_telemetry::get().track(dicto_telemetry::Event::AppStarted);
                        info!("telemetry: sent test event");
                    }),
            )
        })
        .into_any_element()
}

fn privacy_list(title: &str, items: &[&str]) -> gpui::AnyElement {
    let mut list = v_flex().gap(px(3.));
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
        .gap(px(4.))
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(title.to_string())),
        )
        .child(list)
        .into_any_element()
}

/// Flip consent to the opposite state. Undecided counts as off here — the
/// first toggle in the UI opts the user in.
fn toggle_consent(currently_on: bool) -> mdict_rs::settings::TelemetryConsent {
    use mdict_rs::settings::TelemetryConsent;
    if currently_on {
        TelemetryConsent::OptedOut
    } else {
        TelemetryConsent::OptedIn
    }
}

/// A small on/off toggle switch.
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
