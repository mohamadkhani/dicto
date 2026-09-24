//! Background audio playback for MDD pronunciation clips.
//!
//! First we try to feed the raw bytes straight into rodio (works for
//! mp3/wav/ogg-vorbis/flac). For codecs rodio can't actually decode —
//! notably Speex (`.spx`) — we transcode via `ffmpeg` to a cached WAV
//! on disk and play that instead. The on-disk cache means a second
//! click on the same word replays instantly.

use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use tracing::{info, warn};

/// Look up a resource by path and play it.
pub fn play_resource(path: &str) {
    let path = path.to_string();
    thread::spawn(move || {
        let bytes = match mdict_rs::query::lookup_resource(&path) {
            Some(b) => b,
            None => {
                warn!("audio: resource not found: {path}");
                dicto_telemetry::get().track(dicto_telemetry::Event::PronunciationPlaybackFailed {
                    reason: dicto_telemetry::PlaybackFailureReason::ResourceNotFound,
                });
                return;
            }
        };
        play_or_transcode(&path, bytes);
    });
}

fn play_or_transcode(path: &str, bytes: Vec<u8>) {
    let cached = cache_wav_path(path);

    if cached.exists() {
        play_file(&cached, path);
        return;
    }

    // rodio can play these directly; skip ffmpeg.
    if !needs_transcode(path, &bytes) && try_play_buffer(&bytes) {
        return;
    }
    // fall through and let ffmpeg have a go

    if !decode_via_ffmpeg(&bytes, &cached) {
        return; // decode_via_ffmpeg already logged the reason
    }
    info!("audio: cached transcoded clip at {}", cached.display());
    play_file(&cached, path);
}

fn play_file(cached: &Path, label: &str) {
    let bytes = match fs::read(cached) {
        Ok(b) => b,
        Err(e) => {
            warn!("audio: reading cached wav failed: {e}");
            return;
        }
    };
    if !try_play_buffer(&bytes) {
        warn!(
            "audio: rodio refused cached wav at {} (clip: {})",
            cached.display(),
            label
        );
        dicto_telemetry::get().track(dicto_telemetry::Event::PronunciationPlaybackFailed {
            reason: dicto_telemetry::PlaybackFailureReason::DecoderRejected,
        });
    }
}

/// Try to play a buffer through rodio. Returns false if any step
/// before playback queued — caller can fall back to ffmpeg.
fn try_play_buffer(bytes: &[u8]) -> bool {
    let (_stream, handle) = match rodio::OutputStream::try_default() {
        Ok(pair) => pair,
        Err(e) => {
            warn!("audio: no default output: {e}");
            dicto_telemetry::get().track(dicto_telemetry::Event::PronunciationPlaybackFailed {
                reason: dicto_telemetry::PlaybackFailureReason::NoDevice,
            });
            return false;
        }
    };
    let sink = match rodio::Sink::try_new(&handle) {
        Ok(s) => s,
        Err(e) => {
            warn!("audio: sink failed: {e}");
            dicto_telemetry::get().track(dicto_telemetry::Event::PronunciationPlaybackFailed {
                reason: dicto_telemetry::PlaybackFailureReason::SinkFailed,
            });
            return false;
        }
    };
    let decoder = match rodio::Decoder::new(Cursor::new(bytes.to_vec())) {
        Ok(d) => d,
        Err(_) => {
            dicto_telemetry::get().track(dicto_telemetry::Event::PronunciationPlaybackFailed {
                reason: dicto_telemetry::PlaybackFailureReason::DecoderRejected,
            });
            return false;
        }
    };
    sink.append(decoder);
    sink.sleep_until_end();
    true
}

/// Heuristic: codecs rodio's symphonia stack can't decode.
/// Currently catches Speex (`.spx`, OGG-Speex container, raw Speex).
fn needs_transcode(path: &str, bytes: &[u8]) -> bool {
    if path.to_lowercase().ends_with(".spx") {
        return true;
    }
    if bytes.starts_with(b"Speex   ") {
        return true;
    }
    if bytes.len() >= 64 && &bytes[..4] == b"OggS" {
        return bytes[..64.min(bytes.len())]
            .windows(5)
            .any(|w| w == b"Speex");
    }
    false
}

/// Transcode the in-memory buffer via ffmpeg into `out_path`. We use
/// real files for both ends (no pipes) so ffmpeg can write a proper
/// WAV header with the correct chunk size — pipe-mode emits an
/// `0xFFFFFFFF` size sentinel that some decoders refuse.
fn decode_via_ffmpeg(bytes: &[u8], out_path: &Path) -> bool {
    let in_path = out_path.with_extension("in");
    if let Err(e) = fs::write(&in_path, bytes) {
        warn!("audio: writing ffmpeg input failed: {e}");
        return false;
    }

    let result = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-y", "-i"])
        .arg(&in_path)
        .args(["-f", "wav", "-acodec", "pcm_s16le"])
        .arg(out_path)
        .output();

    let _ = fs::remove_file(&in_path);

    match result {
        Err(e) => {
            warn!("audio: ffmpeg not available ({e}); install ffmpeg to enable .spx playback");
            dicto_telemetry::get().track(dicto_telemetry::Event::PronunciationPlaybackFailed {
                reason: dicto_telemetry::PlaybackFailureReason::FfmpegMissing,
            });
            false
        }
        Ok(output) if !output.status.success() => {
            warn!(
                "audio: ffmpeg failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
            dicto_telemetry::get().track(dicto_telemetry::Event::PronunciationPlaybackFailed {
                reason: dicto_telemetry::PlaybackFailureReason::FfmpegFailed,
            });
            false
        }
        Ok(_) => true,
    }
}

fn cache_wav_path(src: &str) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut hasher);
    let hash = hasher.finish();

    let mut dir = std::env::temp_dir();
    dir.push("mdict-rs-cache");
    let _ = fs::create_dir_all(&dir);
    dir.push(format!("{hash:016x}.wav"));
    dir
}
