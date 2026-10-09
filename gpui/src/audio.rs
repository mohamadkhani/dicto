//! Background audio playback for MDD pronunciation clips.
//!
//! All clips play on ONE dedicated thread that serializes every playback.
//! This is deliberate: opening streams concurrently from per-click threads
//! raced the TTS controllers' streams on some ALSA setups and silently
//! dropped clips — the "sometimes it plays" bug.
//!
//! Each clip opens a FRESH output stream, plays to the end, and drops it —
//! the same open/play/drop lifecycle as the TTS path. On ALSA with
//! sound-server routing a long-lived stream goes stale when the device
//! topology shifts (append succeeds but stays silent), so nothing is kept
//! open between clips.
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
use std::sync::LazyLock;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use tracing::{debug, warn};

/// Commands for the dedicated clip-audio thread.
enum ClipCmd {
    /// Fetch + play the MDD resource at this path.
    Play(String),
}

/// Handle to the dedicated clip-audio thread, created lazily on first
/// use. The thread owns the rodio stream; `rodio::OutputStream` is
/// `!Send` on some hosts, which is exactly why the stream lives on one
/// thread instead of in a static.
static CLIP_TX: LazyLock<Sender<ClipCmd>> = LazyLock::new(|| {
    let (tx, rx) = mpsc::channel::<ClipCmd>();
    thread::Builder::new()
        .name("dicto-clips".into())
        .spawn(move || clip_thread(rx))
        .expect("spawn clip audio thread");
    tx
});

/// Look up a resource by path and play it.
pub fn play_resource(path: &str) {
    if let Err(e) = CLIP_TX.send(ClipCmd::Play(path.to_string())) {
        warn!("audio: clip thread gone: {e}");
    }
}

/// The clip thread: serializes all playback. The output stream is opened
/// FRESH for every clip and held until the clip finishes
/// (`sleep_until_end`), then dropped — the same open/play/drop lifecycle
/// as the TTS path, the only rodio usage that proved reliable on
/// ALSA-with-sound-server routing: a long-lived stream goes stale there
/// when the device topology shifts, and appending to it succeeds but
/// stays silent. Blocking also serializes rapid clicks: the next queued
/// clip plays after the current one finishes.
fn clip_thread(rx: Receiver<ClipCmd>) {
    while let Ok(ClipCmd::Play(path)) = rx.recv() {
        debug!(path = %path, "audio: clip command received");
        let Some(bytes) = mdict_rs::query::lookup_resource(&path) else {
            warn!("audio: resource not found: {path}");
            dicto_telemetry::get().track(dicto_telemetry::Event::PronunciationPlaybackFailed {
                reason: dicto_telemetry::PlaybackFailureReason::ResourceNotFound,
            });
            continue;
        };
        debug!(path = %path, bytes = bytes.len(), "audio: resource fetched");
        play_bytes(&path, bytes);
    }
}

/// Play raw clip bytes: direct rodio decode when possible, otherwise
/// transcode via ffmpeg into a cached WAV and play that.
fn play_bytes(path: &str, bytes: Vec<u8>) {
    let cached = cache_wav_path(path);

    if cached.exists() {
        play_file(&cached, path);
        return;
    }

    // rodio can play these directly; skip ffmpeg.
    if !needs_transcode(path, &bytes) && play_direct(&bytes) {
        return;
    }
    // fall through and let ffmpeg have a go

    if !decode_via_ffmpeg(&bytes, &cached) {
        return; // decode_via_ffmpeg already logged the reason
    }
    debug!("audio: cached transcoded clip at {}", cached.display());
    play_file(&cached, path);
}

/// Decode `bytes` and play them on a fresh output stream, blocking until
/// the clip finishes. False = device/sink failed or the decoder rejected
/// the format — the caller falls back to ffmpeg.
fn play_direct(bytes: &[u8]) -> bool {
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
        Ok(sink) => sink,
        Err(e) => {
            warn!("audio: sink failed: {e}");
            dicto_telemetry::get().track(dicto_telemetry::Event::PronunciationPlaybackFailed {
                reason: dicto_telemetry::PlaybackFailureReason::SinkFailed,
            });
            return false;
        }
    };
    let Ok(decoder) = rodio::Decoder::new(Cursor::new(bytes.to_vec())) else {
        warn!("audio: decoder rejected clip");
        dicto_telemetry::get().track(dicto_telemetry::Event::PronunciationPlaybackFailed {
            reason: dicto_telemetry::PlaybackFailureReason::DecoderRejected,
        });
        return false;
    };
    sink.append(decoder);
    // Holding the stream until the clip ends is what keeps it audible —
    // `_stream` must outlive playback.
    sink.sleep_until_end();
    debug!("audio: clip finished");
    true
}

fn play_file(cached: &Path, label: &str) {
    let bytes = match fs::read(cached) {
        Ok(b) => b,
        Err(e) => {
            warn!("audio: reading cached wav failed: {e}");
            return;
        }
    };
    if !play_direct(&bytes) {
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
