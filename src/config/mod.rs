use std::path::PathBuf;

use tracing::{info, warn};

pub const APP_NAME: &str = "dicto";

// ── directory helpers ─────────────────────────────────────────────────────────

pub fn discover_mdx_files() -> Vec<String> {
    let config_dir = dirs_config_path();
    let mut files = Vec::new();

    if config_dir.exists() {
        info!("scanning for MDX files in {}", config_dir.display());
        collect_files_with_ext(&config_dir, "mdx", &mut files);
    }

    let fallback = PathBuf::from("./resources/mdx");
    if fallback.exists() && !config_dir.exists() {
        info!("scanning for MDX files in {}", fallback.display());
        collect_files_with_ext(&fallback, "mdx", &mut files);
    }

    if files.is_empty() {
        warn!("no MDX files found in {}", config_dir.display());
    } else {
        info!("found {} MDX file(s)", files.len());
        for f in &files {
            info!("  - {f}");
        }
    }
    files
}

pub fn discover_mdd_files() -> Vec<String> {
    let config_dir = dirs_config_path();
    let mut files = Vec::new();
    if config_dir.exists() {
        collect_files_with_ext(&config_dir, "mdd", &mut files);
    }
    let fallback = PathBuf::from("./resources/mdx");
    if fallback.exists() && !config_dir.exists() {
        collect_files_with_ext(&fallback, "mdd", &mut files);
    }
    if !files.is_empty() {
        info!("found {} MDD file(s)", files.len());
    }
    files
}

fn collect_files_with_ext(dir: &PathBuf, ext: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_with_ext(&path, ext, out);
        } else if path
            .extension()
            .map(|e| e.eq_ignore_ascii_case(ext))
            .unwrap_or(false)
            && let Some(s) = path.to_str()
        {
            out.push(s.to_string());
        }
    }
}

pub fn dirs_config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config"));
    base.join(APP_NAME).join("dicts")
}

pub fn static_path() -> anyhow::Result<PathBuf> {
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/static"))
}

/// No-op after FST migration; cache invalidation happens via registry::reload().
pub fn reset_pools() {}
