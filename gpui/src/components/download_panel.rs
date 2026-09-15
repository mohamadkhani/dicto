use gpui::{
    AppContext as _, AsyncApp, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{h_flex, progress::Progress, scroll::ScrollableElement, v_flex};

use crate::{
    catalog::{self, DictCatalogEntry, InstallStatus, format_bytes},
    colors, download,
    state::{CatalogState, DictDownloadStatus, DictState},
};

pub fn download_tab_content(
    state: Entity<DictState>,
    _window: &mut Window,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    let catalog_state = state.read(cx).catalog.clone();

    match catalog_state {
        CatalogState::Idle => {
            trigger_catalog_load(state.clone(), cx);
            loading_view("Loading catalog...")
        }
        CatalogState::Loading => loading_view("Loading catalog..."),
        CatalogState::Error(msg) => error_view(&msg, state.clone(), cx),
        CatalogState::Loaded { entries, .. } => catalog_list(state, &entries, cx),
    }
}

fn loading_view(msg: &str) -> gpui::AnyElement {
    let msg_text = msg.to_string();
    v_flex()
        .w_full()
        .h(px(380.))
        .items_center()
        .justify_center()
        .gap(px(12.))
        .child(
            div()
                .text_size(px(14.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(msg_text)),
        )
        .into_any_element()
}

fn error_view(msg: &str, state: Entity<DictState>, _cx: &mut gpui::App) -> gpui::AnyElement {
    let retry_state = state.clone();
    v_flex()
        .w_full()
        .h(px(380.))
        .items_center()
        .justify_center()
        .gap(px(12.))
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors::error())
                .child(SharedString::from(format!("Error: {msg}"))),
        )
        .child(
            div()
                .id("catalog-retry-btn")
                .px(px(14.))
                .py(px(6.))
                .rounded(px(6.))
                .text_size(px(12.))
                .text_color(colors::text())
                .border_1()
                .border_color(colors::border())
                .cursor_pointer()
                .hover(|s| s.bg(colors::surface()))
                .child("Retry")
                .on_click(move |_, _, cx| {
                    cx.update_entity(&retry_state, |s, cx| {
                        s.catalog = CatalogState::Idle;
                        cx.notify();
                    });
                }),
        )
        .into_any_element()
}

fn catalog_list(
    state: Entity<DictState>,
    entries: &[DictCatalogEntry],
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    let download_status = state.read(cx).download_status.clone();
    let active_download = state.read(cx).download_active_id.clone();
    let status_child: Option<gpui::AnyElement> = download_status_bar(&download_status);

    v_flex()
        .w_full()
        .gap(px(10.))
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child("Download dictionaries and import them automatically."),
        )
        .children(status_child)
        .child(
            v_flex()
                .id("download-catalog-list")
                .w_full()
                .h(px(380.))
                .gap(px(6.))
                .overflow_y_scrollbar()
                .children(entries.iter().map(|entry| {
                    let is_active = active_download.as_deref() == Some(&entry.id);
                    catalog_row(state.clone(), entry, is_active, &download_status)
                })),
        )
        .into_any_element()
}

fn download_status_bar(status: &DictDownloadStatus) -> Option<gpui::AnyElement> {
    match status {
        DictDownloadStatus::Idle => None,
        DictDownloadStatus::Downloading {
            progress,
            speed,
            current_file,
        } => Some(
            v_flex()
                .w_full()
                .gap(px(4.))
                .px(px(2.))
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors::text_secondary())
                                .child(SharedString::from(format!(
                                    "Downloading {current_file}..."
                                ))),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors::text_secondary())
                                .child(SharedString::from(format!(
                                    "{:.0}% \u{2014} {speed}",
                                    progress * 100.0
                                ))),
                        ),
                )
                .child(Progress::new("download-progress").value(progress * 100.0))
                .into_any_element(),
        ),
        DictDownloadStatus::Done => Some(
            div()
                .text_size(px(11.))
                .text_color(colors::success())
                .child("Download complete!")
                .into_any_element(),
        ),
        DictDownloadStatus::Error(msg) => Some(
            div()
                .text_size(px(11.))
                .text_color(colors::error())
                .child(SharedString::from(format!("Error: {msg}")))
                .into_any_element(),
        ),
    }
}

fn catalog_row(
    state: Entity<DictState>,
    entry: &DictCatalogEntry,
    is_active: bool,
    download_status: &DictDownloadStatus,
) -> gpui::AnyElement {
    let lang_pair = if entry.lang_from == entry.lang_to {
        entry.lang_from.to_uppercase()
    } else {
        format!(
            "{} \u{2194} {}",
            entry.lang_from.to_uppercase(),
            entry.lang_to.to_uppercase()
        )
    };
    let size = entry.display_size();
    let entry_id = entry.id.clone();
    let entry_name = entry.name.clone();

    let status = entry.install_status();

    let is_downloading =
        matches!(download_status, DictDownloadStatus::Downloading { .. }) && is_active;

    let button = match status {
        _ if is_downloading => downloading_badge(),
        InstallStatus::UpToDate => installed_badge(),
        InstallStatus::UpdateAvailable => {
            let click_state = state.clone();
            let click_id = entry_id.clone();
            update_button(move |_, _, cx| {
                start_download(&click_id, &click_state, cx);
            })
        }
        InstallStatus::NotInstalled => {
            let click_state = state.clone();
            let click_id = entry_id.clone();
            download_button(move |_, _, cx| {
                start_download(&click_id, &click_state, cx);
            })
        }
    };

    h_flex()
        .id(SharedString::from(format!("catalog-row-{entry_id}")))
        .w_full()
        .px(px(12.))
        .py(px(10.))
        .rounded(px(8.))
        .bg(colors::bg())
        .border_1()
        .border_color(if is_active {
            colors::primary()
        } else {
            colors::border()
        })
        .gap(px(12.))
        .items_start()
        .child(
            v_flex()
                .flex_1()
                .min_w(px(0.))
                .gap(px(3.))
                .child(
                    h_flex()
                        .gap(px(8.))
                        .items_center()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors::text())
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .child(SharedString::from(entry_name)),
                        )
                        .child(
                            div()
                                .px(px(6.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(colors::surface())
                                .text_size(px(10.))
                                .text_color(colors::text_secondary())
                                .child(SharedString::from(lang_pair)),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors::text_secondary())
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(SharedString::from(entry.description.clone())),
                )
                .child(
                    h_flex()
                        .gap(px(10.))
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(colors::text_secondary())
                                .child(SharedString::from(size)),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(colors::text_secondary())
                                .child(SharedString::from(format!("v{}", entry.version))),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(colors::text_secondary())
                                .child(SharedString::from(format!("\u{1F4DC} {}", entry.license))),
                        ),
                ),
        )
        .child(div().flex_shrink_0().mt(px(2.)).child(button))
        .into_any_element()
}

fn download_button(
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::AnyElement {
    div()
        .id("catalog-download-btn")
        .px(px(12.))
        .py(px(5.))
        .rounded(px(6.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::bg())
        .bg(colors::primary())
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .child("Download")
        .on_click(on_click)
        .into_any_element()
}

fn downloading_badge() -> gpui::AnyElement {
    div()
        .px(px(12.))
        .py(px(5.))
        .rounded(px(6.))
        .text_size(px(11.))
        .text_color(colors::text_secondary())
        .bg(colors::surface())
        .child("Downloading...")
        .into_any_element()
}

fn installed_badge() -> gpui::AnyElement {
    div()
        .px(px(12.))
        .py(px(5.))
        .rounded(px(6.))
        .text_size(px(11.))
        .text_color(colors::success())
        .bg(colors::surface())
        .child("\u{2713} Installed")
        .into_any_element()
}

fn update_button(
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::AnyElement {
    div()
        .id("catalog-update-btn")
        .px(px(12.))
        .py(px(5.))
        .rounded(px(6.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::bg())
        .bg(colors::update())
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .child("Update")
        .on_click(on_click)
        .into_any_element()
}

fn trigger_catalog_load(state: Entity<DictState>, cx: &mut gpui::App) {
    cx.update_entity(&state, |s, cx| {
        s.catalog = CatalogState::Loading;
        cx.notify();
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = catalog::fetch_catalog();

        cx.update(|cx| {
            cx.update_entity(&state, |s, cx| match result {
                Ok(cat) => {
                    s.catalog = CatalogState::Loaded {
                        base_url: cat.base_url,
                        entries: cat.dictionaries,
                    };
                    cx.notify();
                }
                Err(e) => {
                    s.catalog = CatalogState::Error(e.to_string());
                    cx.notify();
                }
            });
        });
    })
    .detach();
}

fn start_download(entry_id: &str, state: &Entity<DictState>, cx: &mut gpui::App) {
    let (base_url, entries) = match &state.read(cx).catalog {
        CatalogState::Loaded { base_url, entries } => (base_url.clone(), entries.clone()),
        _ => return,
    };

    let entry = match entries.iter().find(|e| e.id == entry_id) {
        Some(e) => e.clone(),
        None => return,
    };

    let urls = entry.download_urls(&base_url);
    let sha256s: Vec<String> = entry.files.iter().map(|f| f.sha256.clone()).collect();
    let first_file = entry
        .files
        .first()
        .map(|f| f.filename.clone())
        .unwrap_or_default();

    cx.update_entity(state, |s, cx| {
        s.download_status = DictDownloadStatus::Downloading {
            progress: 0.0,
            speed: String::new(),
            current_file: first_file,
        };
        s.download_active_id = Some(entry_id.to_string());
        cx.notify();
    });

    let state_clone = state.clone();
    let progress = download::new_shared_progress();
    let poll_progress = progress.clone();
    let poll_state = state.clone();
    let dict_entry = entry.clone();

    // Spawn a timer that polls shared progress and updates the UI
    let timer_handle = cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            if let Ok(p) = poll_progress.lock() {
                let pct = if p.total_bytes > 0 {
                    p.bytes_downloaded as f32 / p.total_bytes as f32
                } else {
                    0.0
                };
                let speed = format_bytes(p.speed_bytes_per_sec) + "/s";
                let file = p.current_file.clone();
                cx.update(|cx| {
                    cx.update_entity(&poll_state, |s, cx| {
                        s.download_status = DictDownloadStatus::Downloading {
                            progress: pct,
                            speed,
                            current_file: file,
                        };
                        cx.notify();
                    });
                });
            }
            cx.background_executor()
                .timer(std::time::Duration::from_millis(200))
                .await;
        }
    });

    // Spawn the actual download on background executor
    cx.spawn(async move |cx: &mut AsyncApp| {
        let bg_urls = urls.clone();
        let bg_sha = sha256s.clone();
        let bg_progress = progress.clone();
        let result = cx
            .background_executor()
            .spawn(async move { download::download_entry(&bg_urls, &bg_sha, &bg_progress) })
            .await;

        // Stop the progress timer by dropping its handle
        drop(timer_handle);

        match result {
            Ok(()) => {
                let mdx_paths: Vec<std::path::PathBuf> = urls
                    .iter()
                    .map(|(_, p)| p.clone())
                    .filter(|p| {
                        p.extension()
                            .map(|e| e.eq_ignore_ascii_case("mdx"))
                            .unwrap_or(false)
                    })
                    .collect();

                for path in &mdx_paths {
                    let dest_str = path.to_string_lossy().to_string();
                    let mut settings = mdict_rs::settings::current();
                    if !settings.dictionaries.iter().any(|d| d.path == dest_str) {
                        settings.dictionaries.push(mdict_rs::settings::DictEntry {
                            path: dest_str.clone(),
                            enabled: true,
                            short_name: String::new(),
                        });
                        let _ = mdict_rs::settings::update(settings);
                    }
                    mdict_rs::config::reset_pools();

                    let idx_path = dest_str.clone();
                    cx.background_executor()
                        .spawn(async move {
                            if let Some(dict) = mdict_rs::formats::detect(&idx_path) {
                                let _ = dict.build_index(false);
                            }
                        })
                        .await;
                }

                mdict_rs::registry::reload();
                crate::indexing::load_stylesheets();
                let _ = dict_entry.write_version();

                cx.update(|cx| {
                    cx.update_entity(&state_clone, |s, cx| {
                        s.download_status = DictDownloadStatus::Done;
                        s.download_active_id = None;
                        s.dictionaries = mdict_rs::settings::current().dictionaries;
                        cx.notify();
                    });
                });
            }
            Err(e) => {
                cx.update(|cx| {
                    cx.update_entity(&state_clone, |s, cx| {
                        s.download_status = DictDownloadStatus::Error(e.to_string());
                        s.download_active_id = None;
                        cx.notify();
                    });
                });
            }
        }
    })
    .detach();
}
