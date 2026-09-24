use std::path::PathBuf;

use serde::Deserialize;

pub const CATALOG_URL: &str = "https://dicto-files.logicamp.dev/catalog.json";

#[derive(Debug, Clone, Deserialize)]
pub struct DictCatalog {
    #[allow(dead_code)]
    pub version: u32,
    pub base_url: String,
    pub dictionaries: Vec<DictCatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DictCatalogEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default = "default_version")]
    pub version: String,
    pub lang_from: String,
    pub lang_to: String,
    pub license: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub license_url: String,
    pub files: Vec<CatalogFile>,
    #[serde(default)]
    #[allow(dead_code)]
    pub tags: Vec<String>,
}

fn default_version() -> String {
    "1.0".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogFile {
    pub filename: String,
    pub size_bytes: u64,
    #[serde(default)]
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallStatus {
    NotInstalled,
    UpToDate,
    UpdateAvailable,
}

impl DictCatalogEntry {
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size_bytes).sum()
    }

    pub fn install_dir(&self) -> PathBuf {
        mdict_rs::config::dirs_config_path().join(&self.id)
    }

    pub fn install_status(&self) -> InstallStatus {
        let dir = self.install_dir();
        let version_file = dir.join(".version");

        if !version_file.exists() {
            // Check legacy install (files present but no version file)
            if self.files.iter().all(|f| dir.join(&f.filename).exists()) {
                return InstallStatus::UpdateAvailable;
            }
            return InstallStatus::NotInstalled;
        }

        let installed = std::fs::read_to_string(&version_file)
            .unwrap_or_default()
            .trim()
            .to_string();

        if installed == self.version {
            InstallStatus::UpToDate
        } else {
            InstallStatus::UpdateAvailable
        }
    }

    pub fn write_version(&self) -> anyhow::Result<()> {
        let dir = self.install_dir();
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(".version"), &self.version)?;
        Ok(())
    }

    pub fn display_size(&self) -> String {
        format_bytes(self.total_size())
    }

    pub fn download_urls(&self, base_url: &str) -> Vec<(String, PathBuf)> {
        let dir = self.install_dir();
        self.files
            .iter()
            .map(|f| {
                let url = format!(
                    "{}/{}/{}",
                    base_url.trim_end_matches('/'),
                    self.id,
                    f.filename
                );
                let dest = dir.join(&f.filename);
                (url, dest)
            })
            .collect()
    }
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

pub fn cache_path() -> PathBuf {
    mdict_rs::config::dirs_config_path()
        .parent()
        .unwrap_or(&mdict_rs::config::dirs_config_path())
        .join("catalog-cache.json")
}

pub fn fetch_catalog() -> anyhow::Result<DictCatalog> {
    let cached = cache_path();
    if cached.exists()
        && let Ok(meta) = std::fs::metadata(&cached)
        && let Ok(modified) = meta.modified()
        && modified.elapsed().unwrap_or_default().as_secs() < 24 * 3600
        && let Ok(text) = std::fs::read_to_string(&cached)
        && let Ok(catalog) = serde_json::from_str::<DictCatalog>(&text)
    {
        return Ok(catalog);
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let resp = client.get(CATALOG_URL).send()?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to fetch catalog: HTTP {}", resp.status());
    }

    let text = resp.text()?;
    let catalog: DictCatalog = serde_json::from_str(&text)?;

    if let Some(parent) = cached.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&cached, &text);

    Ok(catalog)
}
