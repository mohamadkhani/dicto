//! User-editable settings persisted to `~/.config/mdict-dict/settings.toml`.
//!
//! Tracks the list of MDX dictionaries (path + enabled flag) in the order
//! the user wants them queried. New `.mdx` files dropped into the mdict
//! directory get appended (enabled by default); removed files are dropped
//! on next save.

use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config::{APP_NAME, discover_mdx_files};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub dictionaries: Vec<DictEntry>,
    /// Telemetry opt-in state. Stored here so it persists in settings.toml
    /// without coupling this crate to any analytics dependency.
    #[serde(default)]
    pub telemetry_consent: TelemetryConsent,
    /// Anonymous installation id (UUID v4), generated once on first launch.
    /// Local-only; identifies an installation, never a person.
    #[serde(default)]
    pub installation_id: Option<String>,
    /// Quick Translate feature: global hotkey + LLM translation settings.
    /// Stored in the core crate as plain data so the GPUI app and
    /// `dicto-translate` crate both have access without circular deps.
    #[serde(default)]
    pub quick_translate: QuickTranslateSettings,
}

/// Three-state telemetry consent, persisted in settings.toml.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryConsent {
    /// Never shown the consent prompt yet → app shows it on launch.
    #[default]
    Undecided,
    /// User opted in → anonymous usage events are sent.
    OptedIn,
    /// User declined → nothing is sent, ever, until they re-enable.
    OptedOut,
}

/// Quick Translate feature settings.
///
/// Controls the global hotkey, LLM provider configuration, and target
/// language for the selection-translation popup. Stored here in the core
/// crate as plain data so both the GPUI UI and the `dicto-translate`
/// crate can use them without circular dependencies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickTranslateSettings {
    /// Whether the quick-translate feature is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Hotkey string in "Mod+Mod+Key" format, e.g. "Ctrl+Alt+D".
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// Which LLM provider to use for translation of longer text.
    #[serde(default)]
    pub llm_provider: LlmProvider,
    /// API key for the configured LLM provider.
    /// Stored in plaintext; future work may move to OS keyring.
    #[serde(default)]
    pub api_key: String,
    /// Base URL for the API. Required for OpenAI-compatible providers,
    /// optional for Anthropic (defaults to api.anthropic.com).
    #[serde(default)]
    pub api_base_url: String,
    /// Model name, e.g. "claude-sonnet-4-6" or "gpt-4o-mini".
    #[serde(default = "default_model")]
    pub model: String,
    /// Target language for translation, e.g. "English", "Persian".
    #[serde(default = "default_target_lang")]
    pub target_lang: String,
    /// Text-to-speech settings for the popup's Speak button.
    #[serde(default)]
    pub tts: TtsSettings,
}

impl Default for QuickTranslateSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            hotkey: default_hotkey(),
            llm_provider: LlmProvider::default(),
            api_key: String::new(),
            api_base_url: String::new(),
            model: default_model(),
            target_lang: default_target_lang(),
            tts: TtsSettings::default(),
        }
    }
}

/// Text-to-speech settings.
///
/// When `enabled`, the popup's Speak button synthesizes speech via an
/// OpenAI-compatible `/audio/speech` endpoint (OpenAI, Groq, OpenRouter,
/// local servers). When disabled, it falls back to the platform TTS
/// (espeak-ng on Linux).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsSettings {
    /// Use the AI TTS API instead of the platform TTS.
    #[serde(default)]
    pub enabled: bool,
    /// API key for the TTS provider.
    #[serde(default)]
    pub api_key: String,
    /// Base URL, e.g. "https://api.openai.com/v1".
    #[serde(default = "default_tts_base_url")]
    pub api_base_url: String,
    /// TTS model, e.g. "gpt-4o-mini-tts" or "tts-1".
    #[serde(default = "default_tts_model")]
    pub model: String,
    /// Voice name, e.g. "alloy", "nova", "shimmer".
    #[serde(default = "default_tts_voice")]
    pub voice: String,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: String::new(),
            api_base_url: default_tts_base_url(),
            model: default_tts_model(),
            voice: default_tts_voice(),
        }
    }
}

fn default_tts_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_tts_model() -> String {
    "gpt-4o-mini-tts".to_string()
}

fn default_tts_voice() -> String {
    "alloy".to_string()
}

fn default_hotkey() -> String {
    "Ctrl+Alt+D".to_string()
}

fn default_model() -> String {
    "claude-sonnet-4-6".to_string()
}

fn default_target_lang() -> String {
    "English".to_string()
}

/// LLM provider for AI-powered translation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LlmProvider {
    /// Anthropic Claude API.
    #[default]
    Anthropic,
    /// Any OpenAI-compatible endpoint (Ollama, Groq, OpenRouter, etc.).
    OpenAiCompatible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictEntry {
    pub path: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// User-chosen short name for tab titles. Empty string means "auto-generate".
    #[serde(default)]
    pub short_name: String,
}

fn default_enabled() -> bool {
    true
}

pub fn settings_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config"));
    base.join(APP_NAME).join("settings.toml")
}

fn load_from_disk() -> Settings {
    let path = settings_path();
    if !path.exists() {
        return Settings::default();
    }
    match fs::read_to_string(&path) {
        Ok(s) => match toml::from_str(&s) {
            Ok(parsed) => parsed,
            Err(e) => {
                warn!("settings: parse failed ({e}); using defaults");
                Settings::default()
            }
        },
        Err(e) => {
            warn!("settings: read failed ({e}); using defaults");
            Settings::default()
        }
    }
}

/// Reconcile a stored settings list with what's currently on disk:
/// new files get appended (enabled by default), missing files dropped.
fn merge_with_disk(mut s: Settings) -> Settings {
    let on_disk = discover_mdx_files();
    let known: std::collections::HashSet<String> = on_disk.iter().cloned().collect();

    s.dictionaries.retain(|d| known.contains(&d.path));

    let mut existing: std::collections::HashSet<String> =
        s.dictionaries.iter().map(|d| d.path.clone()).collect();
    for path in on_disk {
        if !existing.contains(&path) {
            s.dictionaries.push(DictEntry {
                path: path.clone(),
                enabled: true,
                short_name: String::new(),
            });
            existing.insert(path);
        }
    }
    s
}

static SETTINGS: LazyLock<RwLock<Settings>> = LazyLock::new(|| {
    let merged = merge_with_disk(load_from_disk());
    if let Err(e) = save(&merged) {
        warn!("settings: initial save failed: {e}");
    }
    RwLock::new(merged)
});

pub fn current() -> Settings {
    SETTINGS.read().unwrap().clone()
}

/// Replace the settings on disk and in memory. Caller is responsible for
/// telling the rest of the system to react (re-index, rebuild pools).
pub fn update(new: Settings) -> anyhow::Result<()> {
    let cleaned = merge_with_disk(new);
    save(&cleaned)?;
    *SETTINGS.write().unwrap() = cleaned;
    Ok(())
}

/// Persist only the telemetry consent, preserving every other field.
/// Returns the newly saved settings so the caller can re-init telemetry
/// without a separate read.
pub fn update_consent(consent: TelemetryConsent) -> anyhow::Result<Settings> {
    let mut current = current();
    current.telemetry_consent = consent;
    // Bypass merge_with_disk — consent has nothing to do with the dict list,
    // and merging could reorder/drop entries based on disk state.
    save(&current)?;
    *SETTINGS.write().unwrap() = current.clone();
    Ok(current)
}

/// Generate + persist the installation id if none exists yet. Returns the id
/// (newly generated or existing). Cheap to call every launch — it's a no-op
/// once an id is present.
pub fn ensure_installation_id() -> anyhow::Result<String> {
    let mut current = current();
    if let Some(id) = current.installation_id.clone() {
        return Ok(id);
    }
    let id = format_uuid_v4();
    current.installation_id = Some(id.clone());
    save(&current)?;
    *SETTINGS.write().unwrap() = current;
    Ok(id)
}

/// Minimal RFC 4122 v4 UUID generator without an extra dependency: 16 random
/// bytes with the v4 and variant bits set, formatted as 8-4-4-4-12.
fn format_uuid_v4() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let mut bytes = [0u8; 16];
    rng.fill(&mut bytes);
    // Version 4
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    // Variant (RFC 4122)
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let b = bytes;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

/// Persist settings to disk.
pub fn save(s: &Settings) -> anyhow::Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = toml::to_string_pretty(s)?;
    fs::write(&path, body)?;
    info!(
        "settings: saved {} entries to {}",
        s.dictionaries.len(),
        path.display()
    );
    Ok(())
}

/// MDX paths the user wants queried, in display order.
pub fn enabled_mdx() -> Vec<String> {
    current()
        .dictionaries
        .into_iter()
        .filter(|d| d.enabled)
        .map(|d| d.path)
        .collect()
}

/// Look up a user-specified short_name override for the given MDX path.
/// Returns `None` if no override is set (meaning auto-generate).
pub fn short_name_override(mdx_path: &str) -> Option<String> {
    let s = current();
    s.dictionaries
        .iter()
        .find(|d| d.path == mdx_path)
        .and_then(|d| {
            if d.short_name.is_empty() {
                None
            } else {
                Some(d.short_name.clone())
            }
        })
}

/// MDD paths corresponding to enabled MDX entries (matched by stem).
pub fn enabled_mdd() -> Vec<String> {
    enabled_mdx()
        .into_iter()
        .map(|mdx| {
            let p = PathBuf::from(&mdx);
            p.with_extension("mdd").to_string_lossy().to_string()
        })
        .filter(|p| PathBuf::from(p).exists())
        .collect()
}
