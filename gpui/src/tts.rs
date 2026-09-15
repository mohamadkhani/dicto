//! Text-to-speech for arbitrary text.
//!
//! Two backends:
//! 1. **AI TTS** — an OpenAI-compatible `/audio/speech` endpoint (OpenAI,
//!    Groq, OpenRouter, local servers). Used when [`TtsSettings::enabled`]
//!    is set and fully configured. Higher quality, natural voices.
//! 2. **Platform TTS** — shells out to the OS TTS (`espeak-ng` on Linux,
//!    `say` on macOS, PowerShell `System.Speech` on Windows). Always
//!    available as a fallback; needs no configuration or network.
//!
//! Both backends play through `rodio` so we share the audio backend with the
//! rest of the app (`audio.rs`).

use std::io::Cursor;
use std::process::Command;
use std::thread;

use mdict_rs::settings::TtsSettings;
use tracing::{info, warn};

/// Speak `text` using the AI TTS client if configured, else the platform TTS.///
/// `lang_hint` is an optional BCP-47 tag (e.g. `"fa"`, `"en"`) used to pick a
/// voice when the provider/platform supports it. It is best-effort.
///
/// Runs on a background thread so the UI never blocks on the HTTP request or
/// audio decoding.
pub fn speak(text: &str, lang_hint: Option<&str>, tts: Option<&TtsSettings>) {
    if text.trim().is_empty() {
        return;
    }
    let text = text.to_string();
    let lang = lang_hint.map(str::to_string);
    let tts = tts.cloned();
    thread::spawn(move || {
        if let Err(e) = speak_blocking(&text, lang.as_deref(), tts.as_ref()) {
            warn!("tts: playback failed: {e}");
        }
    });
}

/// Synthesize `text` to an in-memory audio buffer (MP3/WAV bytes) without
/// playing. Tries the AI TTS endpoint when configured, else platform TTS.
/// Exposed so the playback controller can fetch bytes on a background task and
/// own the decode + play lifecycle (pause/seek/replay).
pub(crate) fn synthesize_bytes(
    text: &str,
    lang_hint: Option<&str>,
    tts: Option<&TtsSettings>,
) -> anyhow::Result<Vec<u8>> {
    if let Some(tts) = tts.filter(|t| t.enabled && !t.api_key.is_empty()) {
        match AiTtsClient::new(tts).synthesize(text) {
            Ok(bytes) => {
                info!(model = %tts.model, voice = %tts.voice, "tts: synthesized via AI");
                return Ok(bytes);
            }
            Err(e) => {
                // Fall through to platform TTS — don't leave the user silent.
                warn!(error = %e, "tts: AI TTS failed, falling back to platform TTS");
            }
        }
    }
    synthesize_platform(text, lang_hint)
}

fn speak_blocking(
    text: &str,
    lang_hint: Option<&str>,
    tts: Option<&TtsSettings>,
) -> anyhow::Result<()> {
    let bytes = synthesize_bytes(text, lang_hint, tts)?;
    play_bytes(&bytes)
}

// ---------------------------------------------------------------------------
// AI TTS (OpenAI-compatible /audio/speech)
// ---------------------------------------------------------------------------

/// Client for an OpenAI-compatible text-to-speech endpoint.
///
/// Mirrors the blocking-`reqwest` pattern used by the translation clients in
/// the `dicto-translate` crate.
struct AiTtsClient {
    client: reqwest::blocking::Client,
    api_key: String,
    endpoint: String,
    model: String,
    voice: String,
}

impl AiTtsClient {
    fn new(tts: &TtsSettings) -> Self {
        // Trim a trailing slash so `{base}/audio/speech` always joins cleanly.
        let base = tts.api_base_url.trim_end_matches('/').to_string();
        let endpoint = format!("{base}/audio/speech");
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build tts reqwest client"),
            api_key: tts.api_key.clone(),
            endpoint,
            model: if tts.model.is_empty() {
                "gpt-4o-mini-tts".to_string()
            } else {
                tts.model.clone()
            },
            voice: if tts.voice.is_empty() {
                "alloy".to_string()
            } else {
                tts.voice.clone()
            },
        }
    }

    /// POST the text to `/audio/speech` and return the raw audio bytes.
    /// We request MP3 (universally decodable by rodio) at a comfortable rate.
    fn synthesize(&self, text: &str) -> anyhow::Result<Vec<u8>> {
        let body = serde_json::json!({
            "model": self.model,
            "voice": self.voice,
            "input": text,
            "response_format": "mp3",
        });
        let resp = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("tts: API error {status}: {body}");
        }
        let bytes = resp.bytes()?.to_vec();
        if bytes.is_empty() {
            anyhow::bail!("tts: API returned empty audio");
        }
        Ok(bytes)
    }
}

// ---------------------------------------------------------------------------
// Platform TTS fallback
// ---------------------------------------------------------------------------

fn synthesize_platform(text: &str, lang_hint: Option<&str>) -> anyhow::Result<Vec<u8>> {
    #[cfg(target_os = "linux")]
    {
        return synthesize_linux(text, lang_hint);
    }
    #[cfg(target_os = "macos")]
    {
        return synthesize_macos(text, lang_hint);
    }
    #[cfg(target_os = "windows")]
    {
        return synthesize_windows(text, lang_hint);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        anyhow::bail!("tts: unsupported platform");
    }
}

#[cfg(target_os = "linux")]
fn synthesize_linux(text: &str, lang_hint: Option<&str>) -> anyhow::Result<Vec<u8>> {
    for cmd in ["espeak-ng", "espeak"] {
        if Command::new(cmd).arg("--version").output().is_err() {
            continue;
        }
        let mut command = Command::new(cmd);
        command.arg("--stdout");
        if let Some(lang) = lang_hint {
            let primary = lang.split('-').next().unwrap_or(lang);
            command.args(["-v", primary]);
        }
        command.arg(text);
        let output = command.output()?;
        if !output.status.success() || output.stdout.is_empty() {
            anyhow::bail!(
                "tts: {cmd} produced no audio: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return Ok(output.stdout);
    }
    anyhow::bail!("tts: neither espeak-ng nor espeak is installed")
}

#[cfg(target_os = "macos")]
fn synthesize_macos(text: &str, lang_hint: Option<&str>) -> anyhow::Result<Vec<u8>> {
    let mut command = Command::new("say");
    command.args(["--data-format=LEF32@22050", "--output=-"]);
    if let Some(lang) = lang_hint {
        let primary = lang.split('-').next().unwrap_or(lang);
        command.args(["-v", primary]);
    }
    command.arg(text);
    let output = command.output()?;
    if output.stdout.is_empty() {
        anyhow::bail!("tts: say produced no audio");
    }
    Ok(output.stdout)
}

#[cfg(target_os = "windows")]
fn synthesize_windows(text: &str, _lang_hint: Option<&str>) -> anyhow::Result<Vec<u8>> {
    let tmp = std::env::temp_dir().join(format!("dicto-tts-{}.wav", std::process::id()));
    let script = format!(
        r#"Add-Type -AssemblyName System.Speech; \
         $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         $s.SetOutputToWaveFile('{}'); $s.Speak(''' + {} + '''); $s.Dispose()"#,
        tmp.display(),
        text.replace('\'', "''")
    );
    let status = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status()?;
    if !status.success() {
        anyhow::bail!("tts: powershell speech failed");
    }
    let bytes = std::fs::read(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    Ok(bytes)
}

/// Decode and play an in-memory audio buffer through rodio (mirrors
/// `audio::try_play_buffer` but without the telemetry / fallback paths).
fn play_bytes(bytes: &[u8]) -> anyhow::Result<()> {
    let (_stream, handle) = rodio::OutputStream::try_default()
        .map_err(|e| anyhow::anyhow!("no default audio output: {e}"))?;
    let sink = rodio::Sink::try_new(&handle).map_err(|e| anyhow::anyhow!("sink: {e}"))?;
    let decoder = rodio::Decoder::new(Cursor::new(bytes.to_vec()))
        .map_err(|e| anyhow::anyhow!("decode: {e}"))?;
    sink.append(decoder);
    sink.sleep_until_end();
    Ok(())
}
