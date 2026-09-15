use std::path::PathBuf;

use gpui::{
    AppContext as _, AsyncApp, Entity, ExternalPaths, FontWeight, InteractiveElement, IntoElement,
    ParentElement, PathPromptOptions, SharedString, StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::{h_flex, progress::Progress, scroll::ScrollableElement, v_flex};
use mdict_rs::settings::DictEntry;
use tracing::warn;

use crate::{
    colors, indexing,
    state::{DictState, ImportFile, ImportStatus},
};

/// The full import UI body (instructions + drop zone + divider + open-file button + file list).
/// Shared between the first-run init modal and the Settings → Import tab.
pub fn import_panel_content(
    state: Entity<DictState>,
    is_importing: bool,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    let files = &state.read(cx).import_files;
    let has_files = !files.is_empty();

    let progress_pct = if has_files {
        let per_file: f32 = files
            .iter()
            .map(|f| match &f.status {
                ImportStatus::Pending => 0.0,
                ImportStatus::Copying => 33.0,
                ImportStatus::Indexing => 66.0,
                ImportStatus::Done | ImportStatus::Error(_) => 100.0,
            })
            .sum::<f32>();
        per_file / files.len() as f32
    } else {
        0.0
    };

    v_flex()
        .w_full()
        .h_full()
        .gap(px(16.))
        .child(instructions())
        .child(drop_zone(state.clone(), is_importing))
        .children(if has_files {
            Some(
                v_flex()
                    .w_full()
                    .gap(px(6.))
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
                                        "{} / {} files",
                                        files
                                            .iter()
                                            .filter(|f| matches!(
                                                f.status,
                                                ImportStatus::Done | ImportStatus::Error(_)
                                            ))
                                            .count(),
                                        files.len()
                                    ))),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(colors::text_secondary())
                                    .child(SharedString::from(format!("{:.0}%", progress_pct))),
                            ),
                    )
                    .child(Progress::new("import-overall-progress").value(progress_pct)),
            )
        } else {
            None
        })
        .children(if has_files {
            Some(file_list(state, cx))
        } else {
            None
        })
        .into_any_element()
}

fn instructions() -> gpui::AnyElement {
    v_flex()
        .w_full()
        .gap(px(4.))
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors::text())
                .child("To get started, add your MDict dictionary files."),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(
                    "Supported: .mdx, .mdd, .css, .js, .png  ·  All must share the same name as a dictionary. Files are copied to ~/.config/dicto/dicts/",
                ),
        )
        .into_any_element()
}

fn drop_zone(state: Entity<DictState>, is_importing: bool) -> gpui::AnyElement {
    let drop_state = state.clone();

    let mut zone = div()
        .id("init-drop-zone")
        .w_full()
        .h(px(150.))
        .rounded(px(10.))
        .border_2()
        .border_color(colors::border())
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(6.));

    if !is_importing {
        let pick_state = state.clone();
        zone = zone
            .cursor_pointer()
            .hover(|s| s.border_color(colors::primary()).bg(colors::surface()))
            .on_drop::<ExternalPaths>(move |paths, _window, cx| {
                start_import(paths.paths().to_vec(), drop_state.clone(), cx);
            })
            .on_click(move |_, _window, cx| {
                let state_inner = pick_state.clone();
                cx.spawn(async move |cx: &mut AsyncApp| {
                    let rx = cx.update(|cx| {
                        cx.prompt_for_paths(PathPromptOptions {
                            files: true,
                            directories: false,
                            multiple: true,
                            prompt: Some(SharedString::from(
                                "Select Dictionary Files (MDX · MDD · CSS · JS · PNG)",
                            )),
                        })
                    });
                    if let Ok(Ok(Some(paths))) = rx.await {
                        cx.update(|cx| start_import(paths, state_inner, cx));
                    }
                })
                .detach();
            });
    }

    zone.child(
        div()
            .text_size(px(24.))
            .text_color(colors::text_secondary())
            .child("⊕"),
    )
    .child(
        div()
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(colors::text())
            .child("Drop files or click to browse"),
    )
    .child(
        div()
            .text_size(px(11.))
            .text_color(colors::text_secondary())
            .child("MDX · MDD · CSS · JS · PNG  (all need a matching dictionary)"),
    )
    .into_any_element()
}

fn file_list(state: Entity<DictState>, cx: &gpui::App) -> gpui::AnyElement {
    let files = &state.read(cx).import_files;

    let mut list = v_flex()
        .id("import-file-list")
        .w_full()
        .h_full()
        .gap(px(4.))
        .overflow_y_scrollbar();

    for (i, file) in files.iter().enumerate() {
        list = list.child(file_row(i, file));
    }

    // Outer div participates in flex layout (flex_1 = remaining space).
    // The Scrollable wrapper from overflow_y_scrollbar() inherits only `size`
    // from the inner element, losing flex_grow. Giving the inner element h_full()
    // and wrapping it in a flex_1 div fixes the scroll height resolution.
    div().flex_1().min_h(px(0.)).child(list).into_any_element()
}

fn file_row(idx: usize, file: &ImportFile) -> gpui::AnyElement {
    let (status_text, color) = match &file.status {
        ImportStatus::Pending => ("Waiting…".to_string(), colors::text_secondary()),
        ImportStatus::Copying => ("Copying…".to_string(), colors::primary()),
        ImportStatus::Indexing => ("Building index…".to_string(), colors::primary()),
        ImportStatus::Done => (
            "✓ Imported".to_string(),
            gpui::Hsla {
                h: 0.33,
                s: 0.6,
                l: 0.5,
                a: 1.0,
            },
        ),
        ImportStatus::Error(msg) => (
            format!("✗ {msg}"),
            gpui::Hsla {
                h: 0.0,
                s: 0.7,
                l: 0.5,
                a: 1.0,
            },
        ),
    };

    h_flex()
        .id(SharedString::from(format!("import-file-row-{idx}")))
        .w_full()
        .gap(px(8.))
        .items_center()
        .px(px(10.))
        .py(px(6.))
        .rounded(px(6.))
        .bg(colors::bg())
        .border_1()
        .border_color(colors::border())
        .child(
            v_flex()
                .flex_1()
                .min_w(px(0.))
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors::text())
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(SharedString::from(file.name.clone())),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(color)
                        .child(SharedString::from(status_text)),
                ),
        )
        .into_any_element()
}

pub fn start_import(paths: Vec<PathBuf>, state: Entity<DictState>, cx: &mut gpui::App) {
    let already_running = state
        .read(cx)
        .import_files
        .iter()
        .any(|f| matches!(f.status, ImportStatus::Copying | ImportStatus::Indexing));
    if already_running {
        return;
    }

    let paths: Vec<PathBuf> = paths;
    let (mdx_mdd, non_mdx_mdd): (Vec<PathBuf>, Vec<PathBuf>) = paths.into_iter().partition(|p| {
        p.extension()
            .map(|e| e.eq_ignore_ascii_case("mdx") || e.eq_ignore_ascii_case("mdd"))
            .unwrap_or(false)
    });

    let (css_js_png, others): (Vec<PathBuf>, Vec<PathBuf>) =
        non_mdx_mdd.into_iter().partition(|p| {
            p.extension()
                .map(|e| {
                    e.eq_ignore_ascii_case("css")
                        || e.eq_ignore_ascii_case("js")
                        || e.eq_ignore_ascii_case("png")
                })
                .unwrap_or(false)
        });

    let (css_files, js_png): (Vec<PathBuf>, Vec<PathBuf>) = css_js_png.into_iter().partition(|p| {
        p.extension()
            .map(|e| e.eq_ignore_ascii_case("css"))
            .unwrap_or(false)
    });

    let (js_files, png_files): (Vec<PathBuf>, Vec<PathBuf>) = js_png.into_iter().partition(|p| {
        p.extension()
            .map(|e| e.eq_ignore_ascii_case("js"))
            .unwrap_or(false)
    });

    let all_valid_stems: std::collections::HashSet<String> = mdx_mdd
        .iter()
        .filter_map(|p| {
            p.file_stem()
                .and_then(|s| s.to_str().map(|s| s.to_lowercase()))
        })
        .collect();

    let dicts_dir = mdict_rs::config::dirs_config_path();
    let existing_stems: std::collections::HashSet<String> = std::fs::read_dir(&dicts_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension()?.eq_ignore_ascii_case("mdx")
                || path.extension()?.eq_ignore_ascii_case("mdd")
            {
                path.file_stem()
                    .and_then(|s| s.to_str().map(|s| s.to_lowercase()))
            } else {
                None
            }
        })
        .collect();

    let all_stems: std::collections::HashSet<String> =
        all_valid_stems.union(&existing_stems).cloned().collect();

    let mut valid: Vec<PathBuf> = mdx_mdd;
    let mut invalid: Vec<(PathBuf, String)> = Vec::new();

    for path in others {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown")
            .to_string();
        invalid.push((path, format!(".{ext} files are not supported")));
    }

    for css in css_files {
        let css_stem = css
            .file_stem()
            .and_then(|s| s.to_str().map(|s| s.to_lowercase()));
        if css_stem.map_or(true, |stem| !all_stems.contains(&stem)) {
            invalid.push((
                css,
                "CSS file must have a matching .mdx or .mdd file".into(),
            ));
        } else {
            valid.push(css);
        }
    }

    for js in js_files {
        let js_stem = js
            .file_stem()
            .and_then(|s| s.to_str().map(|s| s.to_lowercase()));
        if js_stem.map_or(true, |stem| !all_stems.contains(&stem)) {
            invalid.push((js, "JS file must have a matching .mdx or .mdd file".into()));
        } else {
            valid.push(js);
        }
    }

    for png in png_files {
        let png_stem = png
            .file_stem()
            .and_then(|s| s.to_str().map(|s| s.to_lowercase()));
        if png_stem.map_or(true, |stem| !all_stems.contains(&stem)) {
            invalid.push((
                png,
                "PNG file must have a matching .mdx or .mdd file".into(),
            ));
        } else {
            valid.push(png);
        }
    }

    // Record the starting index so we know which entries belong to this batch.
    let base_idx = state.read(cx).import_files.len();

    cx.update_entity(&state, |s, cx| {
        for (path, error_msg) in &invalid {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            s.import_files.push(ImportFile {
                path: path.clone(),
                name,
                status: ImportStatus::Error(error_msg.clone()),
            });
        }
        for path in &valid {
            let name = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            s.import_files.push(ImportFile {
                path: path.clone(),
                name,
                status: ImportStatus::Pending,
            });
        }
        cx.notify();
    });

    if valid.is_empty() {
        return;
    }

    // Indices of the valid entries we just pushed (invalid ones come first in the batch).
    let valid_start = base_idx + invalid.len();
    let valid_end = valid_start + valid.len();

    #[derive(Default)]
    struct StemGroup {
        mdx_idx: Option<usize>,
        files: Vec<(usize, PathBuf)>,
    }

    cx.spawn(async move |cx: &mut AsyncApp| {
        let import_started = std::time::Instant::now();
        let mut total_bytes: u64 = 0;
        let mut stems: std::collections::HashMap<String, StemGroup> =
            std::collections::HashMap::new();

        for idx in valid_start..valid_end {
            let (path, stem_lower) = cx.update(|cx| {
                let f = &state.read(cx).import_files[idx];
                let stem = f
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
                let stem_lower = stem.to_lowercase();
                (f.path.clone(), stem_lower)
            });

            let group = stems.entry(stem_lower).or_default();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase());
            if ext.as_deref() == Some("mdx") {
                group.mdx_idx = Some(idx);
            }
            group.files.push((idx, path));
        }

        for (stem, group) in stems {
            let stem_path = dicts_dir.join(&stem);

            if let Err(e) = std::fs::create_dir_all(&stem_path) {
                dicto_telemetry::get().track(dicto_telemetry::Event::ErrorOccurred {
                    kind: dicto_telemetry::ErrorKind::Import,
                    message: format!("cannot create dict folder: {e}"),
                });
                for (idx, _) in &group.files {
                    cx.update(|cx| {
                        cx.update_entity(&state, |s, cx| {
                            if let Some(f) = s.import_files.get_mut(*idx) {
                                f.status =
                                    ImportStatus::Error(format!("Cannot create dict folder: {e}"));
                            }
                            cx.notify();
                        });
                    });
                }
                continue;
            }

            for (idx, src_path) in group.files {
                let filename = src_path.file_name().unwrap_or_default();
                let dest_path = stem_path.join(&filename);

                cx.update(|cx| {
                    cx.update_entity(&state, |s, cx| {
                        if let Some(f) = s.import_files.get_mut(idx) {
                            f.status = ImportStatus::Copying;
                        }
                        cx.notify();
                    });
                });

                match std::fs::copy(&src_path, &dest_path) {
                    Ok(bytes) => total_bytes = total_bytes.saturating_add(bytes),
                    Err(e) => {
                        dicto_telemetry::get().track(dicto_telemetry::Event::ErrorOccurred {
                            kind: dicto_telemetry::ErrorKind::Import,
                            message: format!("copy failed: {e}"),
                        });
                        cx.update(|cx| {
                            cx.update_entity(&state, |s, cx| {
                                if let Some(f) = s.import_files.get_mut(idx) {
                                    f.status = ImportStatus::Error(format!("Copy failed: {e}"));
                                }
                                cx.notify();
                            });
                        });
                        continue;
                    }
                }

                let dest_str = dest_path.to_string_lossy().to_string();

                let is_mdd = dest_path
                    .extension()
                    .map(|e| e.eq_ignore_ascii_case("mdd"))
                    .unwrap_or(false);
                let is_css_js_png = dest_path
                    .extension()
                    .map(|e| {
                        e.eq_ignore_ascii_case("css")
                            || e.eq_ignore_ascii_case("js")
                            || e.eq_ignore_ascii_case("png")
                    })
                    .unwrap_or(false);

                if is_mdd {
                    cx.update(|cx| {
                        cx.update_entity(&state, |s, cx| {
                            if let Some(f) = s.import_files.get_mut(idx) {
                                f.status = ImportStatus::Done;
                            }
                            cx.notify();
                        });
                    });
                    continue;
                }

                if is_css_js_png {
                    cx.update(|cx| {
                        cx.update_entity(&state, |s, cx| {
                            if let Some(f) = s.import_files.get_mut(idx) {
                                f.status = ImportStatus::Done;
                            }
                            cx.notify();
                        });
                    });
                    continue;
                }

                cx.update(|cx| {
                    cx.update_entity(&state, |s, cx| {
                        if let Some(f) = s.import_files.get_mut(idx) {
                            f.status = ImportStatus::Indexing;
                        }
                        cx.notify();
                    });
                });

                cx.update(|_cx| {
                    let mut settings = mdict_rs::settings::current();
                    if !settings.dictionaries.iter().any(|d| d.path == dest_str) {
                        settings.dictionaries.push(DictEntry {
                            path: dest_str.clone(),
                            enabled: true,
                            short_name: String::new(),
                        });
                        if let Err(e) = mdict_rs::settings::update(settings) {
                            warn!("import_panel: failed to update settings for {dest_str}: {e}");
                        }
                    }
                    mdict_rs::config::reset_pools();
                });

                let index_result = cx
                    .background_executor()
                    .spawn(async move {
                        if let Some(dict) = mdict_rs::formats::detect(&dest_str) {
                            dict.build_index(false)
                        } else {
                            Err(anyhow::anyhow!("unrecognized dictionary format"))
                        }
                    })
                    .await;

                if let Err(e) = index_result {
                    warn!("import_panel: index failed: {e}");
                    dicto_telemetry::get().track(dicto_telemetry::Event::ErrorOccurred {
                        kind: dicto_telemetry::ErrorKind::Import,
                        message: format!("index failed: {e}"),
                    });
                    cx.update(|cx| {
                        cx.update_entity(&state, |s, cx| {
                            if let Some(f) = s.import_files.get_mut(idx) {
                                f.status = ImportStatus::Error(format!("Index failed: {e}"));
                            }
                            cx.notify();
                        });
                    });
                    continue;
                }

                mdict_rs::registry::reload();
                indexing::load_stylesheets();

                cx.update(|cx| {
                    cx.update_entity(&state, |s, cx| {
                        if let Some(f) = s.import_files.get_mut(idx) {
                            f.status = ImportStatus::Done;
                        }
                        cx.notify();
                    });
                });
            }
        }

        cx.update(|cx| {
            cx.update_entity(&state, |s, cx| {
                s.dictionaries = mdict_rs::settings::current().dictionaries;
                cx.notify();
            });
        });

        // Fire one aggregate event for the whole batch: the count of enabled
        // dictionaries after the import, never the names/paths. Duration
        // covers the whole batch (copy + index) from task start to here.
        // Size is total bytes copied, rounded to the nearest MB — rounding
        // breaks the precise size fingerprint while preserving perf signal.
        let count = mdict_rs::settings::enabled_mdx().len();
        let size_mb = (total_bytes + 500_000) / 1_000_000;
        dicto_telemetry::get().track(dicto_telemetry::Event::DictionaryImported {
            count,
            duration_ms: import_started.elapsed().as_millis() as u64,
            size_mb,
        });
    })
    .detach();
}
