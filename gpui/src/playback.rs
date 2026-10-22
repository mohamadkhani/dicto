//! Audio playback controller for TTS clips, spoken in SENTENCE CHUNKS.
//!
//! The clip text is split into sentence chunks ([`crate::karaoke`]); each chunk
//! is synthesized separately and played back-to-back. This buys two things the
//! single-clip design could not:
//!
//! 1. **Fast first word** — playback starts as soon as chunk 0 is synthesized;
//!    later chunks synthesize on the background task while earlier ones play.
//! 2. **Accurate word highlight** — chunk boundaries are real audio
//!    boundaries, so the karaoke word estimate ([`crate::karaoke`]) resets at
//!    every sentence and cannot drift past one chunk.
//!
//! The controller owns one rodio sink at a time (the CURRENT chunk's). When a
//! chunk's source drains, `poll_progress` advances to the next chunk: starting
//! its sink immediately if its bytes are already installed, or parking in
//! `Loading` until the background task delivers them. A global position
//! (Σ finished chunk durations + current sink position) drives the seek bar,
//! and `highlight_range()` maps the live position onto a word byte range for
//! the popup's editors.
//!
//! The controller is held by `DictState`; GPUI accesses it from `&self`, so
//! every field is behind a `Mutex`.

use std::io::Cursor;
use std::ops::Range;
use std::sync::Mutex;
use std::time::Duration;

use mdict_rs::settings::TtsSettings;
use rodio::Source;

use crate::karaoke::{self, ChunkTiming};

/// Observable playback state for one clip.
#[derive(Clone, Debug, Default)]
pub enum PlaybackState {
    /// Nothing loaded yet.
    #[default]
    Idle,
    /// Synthesizing / decoding audio — before the FIRST chunk, or between
    /// chunks when playback caught up with synthesis. The Speak button
    /// renders a spinner in this state.
    Loading,
    /// Audio is playing. `pos` is the global position in seconds.
    Playing { pos: f32 },
    /// Paused; `pos` is where playback will resume.
    Paused { pos: f32 },
    /// Clip finished naturally. Chunk audio is still loaded — the user can
    /// replay or seek. We keep the controls visible (seek bar full, ▶ + ↺)
    /// instead of dropping back to Idle.
    Ended { pos: f32 },
    /// Fetch or decode failed before anything played.
    #[allow(dead_code)]
    Error(String),
}

/// One frame's view of a playback slot: state + total duration + the text the
/// clip was synthesized from + the TTS settings key it was synthesized with.
/// The popup compares the clip text with the text it is showing (and the key
/// with the CURRENT settings) to detect a stale clip — new text OR changed
/// TTS options mean the loaded audio is wrong.
pub type PlaybackSnapshot = (PlaybackState, Option<Duration>, String, Option<String>);

/// The audio-determining part of a TTS config. Clips synthesized under
/// different keys are different audio.
pub fn tts_key(tts: &TtsSettings) -> String {
    format!("{}|{}|{}", tts.model, tts.api_base_url, tts.voice)
}

impl PlaybackState {
    #[allow(dead_code)]
    pub fn is_loading(&self) -> bool {
        matches!(self, PlaybackState::Loading)
    }
}

/// One sentence chunk of the current clip.
struct Chunk {
    /// Byte range of this chunk in the clip text.
    range: Range<usize>,
    /// Total timing weight ([`crate::karaoke`]) — drives duration estimates
    /// for chunks whose audio is not decoded yet.
    weight: f32,
    /// Synthesized audio; `None` until the background task delivers it.
    bytes: Option<Vec<u8>>,
    /// Decoded duration; `Some` once installed, kept for replay/seek.
    duration: Option<Duration>,
}

/// The rodio output stream + handle. Reopened for each chunk so playback
/// follows the system's current default output device (a stream stays pinned
/// to the device it was opened on).
struct Backend {
    _stream: rodio::OutputStream,
    handle: rodio::OutputStreamHandle,
}

/// Playback controller for one popup slot (source or translation).
pub struct PlaybackController {
    backend: Mutex<Option<Backend>>,
    /// The CURRENT chunk's sink. `None` while idle or waiting for the next
    /// chunk's bytes.
    sink: Mutex<Option<rodio::Sink>>,
    state: Mutex<PlaybackState>,
    /// True between a pause and a resume; honored when a chunk sink starts
    /// (a pause landing exactly on a chunk boundary must not auto-play).
    paused: Mutex<bool>,
    /// Bumped on every new load/stop; the background synthesis task tags its
    /// `install_chunk` calls with the generation it started under, and stale
    /// installs are dropped.
    generation: Mutex<u64>,
    /// False once synthesis failed mid-clip: reaching an unloaded chunk then
    /// ends playback gracefully instead of waiting forever.
    pipeline_alive: Mutex<bool>,
    /// The full text the current clip was synthesized from (stale detection).
    current_text: Mutex<String>,
    /// The TTS key the current clip was synthesized with (stale detection).
    current_tts: Mutex<Option<String>>,
    chunks: Mutex<Vec<Chunk>>,
    timings: Mutex<Vec<ChunkTiming>>,
    /// Index of the chunk assigned to `sink` (or the next one to start).
    current: Mutex<usize>,
    /// Global position where `current` begins: Σ durations of earlier chunks.
    finished: Mutex<Duration>,
}

impl Default for PlaybackController {
    fn default() -> Self {
        Self {
            backend: Mutex::new(None),
            sink: Mutex::new(None),
            state: Mutex::new(PlaybackState::Idle),
            paused: Mutex::new(false),
            generation: Mutex::new(0),
            pipeline_alive: Mutex::new(false),
            current_text: Mutex::new(String::new()),
            current_tts: Mutex::new(None),
            chunks: Mutex::new(Vec::new()),
            timings: Mutex::new(Vec::new()),
            current: Mutex::new(0),
            finished: Mutex::new(Duration::ZERO),
        }
    }
}

/// What `PlaybackController::start_load` decided to do — the caller spawns
/// the synthesis task on the background executor.
pub enum LoadAction {
    /// Same clip already loaded; we replayed synchronously. Nothing to spawn.
    Replay,
    /// New clip. Synthesize `chunks` IN ORDER on a background task; after each
    /// chunk's bytes arrive, call `install_chunk(generation, i, bytes)`. Stop
    /// the loop (and call `fail`) when `install_chunk` returns false or the
    /// synthesis errors.
    Synthesize {
        generation: u64,
        chunks: Vec<String>,
        lang: Option<String>,
        tts: Option<TtsSettings>,
    },
}

impl PlaybackController {
    /// Begin loading `text`. Sets Loading state (or replays if the same text
    /// is already loaded under the same TTS settings) and returns the action
    /// the caller should perform.
    pub fn start_load(
        &self,
        text: String,
        lang: Option<String>,
        tts: Option<TtsSettings>,
    ) -> LoadAction {
        // Fast path: same text AND same TTS settings already loaded → replay.
        let same_tts = {
            let current = self.current_tts.lock().unwrap();
            match (&*current, tts.as_ref()) {
                (Some(current), Some(requested)) => current == &tts_key(requested),
                (None, None) => true,
                _ => false,
            }
        };
        if *self.current_text.lock().unwrap() == text && same_tts {
            let loaded = {
                let chunks = self.chunks.lock().unwrap();
                chunks.first().is_some_and(|c| c.bytes.is_some())
            };
            if loaded {
                self.replay();
                return LoadAction::Replay;
            }
        }

        // Fresh load: chunk the text, reset everything, hand synthesis to the
        // caller. Chunk texts come from the timing ranges so the synthesized
        // strings and the highlight timeline can never diverge.
        let timings = karaoke::timings_for(&text);
        let generation = {
            let mut generation_guard = self.generation.lock().unwrap();
            *generation_guard += 1;
            *generation_guard
        };
        self.stop_sink();
        *self.state.lock().unwrap() = PlaybackState::Loading;
        *self.paused.lock().unwrap() = false;
        *self.pipeline_alive.lock().unwrap() = true;
        *self.chunks.lock().unwrap() = timings
            .iter()
            .map(|t| Chunk {
                range: t.range.clone(),
                weight: t.total_weight,
                bytes: None,
                duration: None,
            })
            .collect();
        *self.timings.lock().unwrap() = timings;
        *self.current.lock().unwrap() = 0;
        *self.finished.lock().unwrap() = Duration::ZERO;
        *self.current_text.lock().unwrap() = text.clone();
        *self.current_tts.lock().unwrap() = tts.as_ref().map(tts_key);
        LoadAction::Synthesize {
            generation,
            chunks: {
                let ranges = self.chunks.lock().unwrap();
                ranges
                    .iter()
                    .map(|c| text[c.range.clone()].to_string())
                    .collect()
            },
            lang,
            tts,
        }
    }

    /// Called by the background synthesis task after each chunk's bytes
    /// arrive. Returns false when the generation is stale (a newer load or a
    /// stop happened) — the task must stop synthesizing further chunks.
    /// Duplicate installs for an already-loaded chunk are ignored (true).
    pub fn install_chunk(&self, generation: u64, index: usize, bytes: Vec<u8>) -> bool {
        if *self.generation.lock().unwrap() != generation {
            return false;
        }
        {
            let mut chunks = self.chunks.lock().unwrap();
            let Some(chunk) = chunks.get_mut(index) else {
                return false;
            };
            if chunk.bytes.is_some() {
                return true;
            }
            // Decode duration up-front: the seek bar's timeline and the word
            // highlight both key off exact chunk durations.
            chunk.duration = match compute_total_duration(&bytes) {
                Ok(d) => Some(d),
                Err(e) => {
                    tracing::warn!(chunk = index, error = %e, "tts: chunk duration unknown");
                    None
                }
            };
            chunk.bytes = Some(bytes);
        }
        // If playback is parked waiting for THIS chunk, start it.
        self.maybe_start_current();
        true
    }

    /// Synthesis failed before producing bytes. If nothing has played yet the
    /// error is surfaced (Error state); otherwise the loaded chunks keep
    /// playing and playback ends gracefully when they run out.
    pub fn fail(&self, error: String) {
        *self.pipeline_alive.lock().unwrap() = false;
        let nothing_yet = {
            let state = self.state.lock().unwrap();
            matches!(&*state, PlaybackState::Loading)
                && *self.current.lock().unwrap() == 0
                && self.sink.lock().unwrap().is_none()
        };
        if nothing_yet {
            *self.state.lock().unwrap() = PlaybackState::Error(error);
        }
    }

    /// Start the current chunk's sink if its bytes are ready, no sink is
    /// running, and playback is parked in Loading (initial load or a chunk
    /// boundary wait).
    fn maybe_start_current(&self) {
        let parked = matches!(&*self.state.lock().unwrap(), PlaybackState::Loading);
        if !parked || self.sink.lock().unwrap().is_some() {
            return;
        }
        let current = *self.current.lock().unwrap();
        let ready = self
            .chunks
            .lock()
            .unwrap()
            .get(current)
            .is_some_and(|c| c.bytes.is_some());
        if ready {
            self.start_chunk(current);
        }
    }

    /// Open a fresh backend, decode chunk `index`'s bytes into a new sink and
    /// play it (or hold it paused). Sets Playing/Paused state. No-op when the
    /// chunk has no bytes yet.
    fn start_chunk(&self, index: usize) {
        let bytes = {
            let chunks = self.chunks.lock().unwrap();
            chunks.get(index).and_then(|c| c.bytes.clone())
        };
        let Some(bytes) = bytes else {
            return;
        };
        let result = self.open_and_play(bytes);
        match result {
            Ok(()) => {
                let paused = *self.paused.lock().unwrap();
                let pos = self.finished.lock().unwrap().as_secs_f32();
                *self.state.lock().unwrap() = if paused {
                    PlaybackState::Paused { pos }
                } else {
                    PlaybackState::Playing { pos }
                };
            }
            Err(e) => {
                tracing::warn!(error = %e, "tts: chunk playback failed");
                self.fail(e.to_string());
            }
        }
    }

    /// Open a fresh output stream + sink and play (or pause) `bytes`.
    fn open_and_play(&self, bytes: Vec<u8>) -> anyhow::Result<()> {
        let mut backend_guard = self.backend.lock().unwrap();
        let (stream, handle) = rodio::OutputStream::try_default()
            .map_err(|e| anyhow::anyhow!("no audio device: {e}"))?;
        *backend_guard = Some(Backend {
            _stream: stream,
            handle,
        });
        let handle = backend_guard
            .as_ref()
            .expect("backend just set")
            .handle
            .clone();
        drop(backend_guard);

        let sink = rodio::Sink::try_new(&handle).map_err(|e| anyhow::anyhow!("sink: {e}"))?;
        let decoder =
            rodio::Decoder::new(Cursor::new(bytes)).map_err(|e| anyhow::anyhow!("decode: {e}"))?;
        sink.append(decoder);
        if *self.paused.lock().unwrap() {
            sink.pause();
        }
        *self.sink.lock().unwrap() = Some(sink);
        Ok(())
    }

    /// Stop + drop the current sink and backend.
    fn stop_sink(&self) {
        if let Some(sink) = self.sink.lock().unwrap().take() {
            sink.stop();
        }
        *self.backend.lock().unwrap() = None;
    }

    /// Global timeline position: Σ finished chunk durations + current sink
    /// position. `None` when no sink is live (the parked position is already
    /// observable via the state).
    fn global_pos(&self) -> Option<Duration> {
        let sink = self.sink.lock().unwrap();
        sink.as_ref()
            .map(|s| *self.finished.lock().unwrap() + s.get_pos())
    }

    /// (durations, weights) parallel vectors for the timeline math.
    fn timeline_parts(&self) -> (Vec<Option<Duration>>, Vec<f32>) {
        let chunks = self.chunks.lock().unwrap();
        (
            chunks.iter().map(|c| c.duration).collect(),
            chunks.iter().map(|c| c.weight).collect(),
        )
    }

    /// Estimated total clip duration (exact once every chunk is decoded).
    fn estimate_total(&self) -> Duration {
        let (durations, weights) = self.timeline_parts();
        karaoke::total_duration(&durations, &weights)
    }

    /// Re-decode the current chunk's bytes and append to `sink`, then play.
    /// A rodio Sink consumes + drops its source once the queue drains, so a
    /// finished/paused-at-end sink can only be restarted with fresh bytes.
    fn restart_current_sink(&self, sink: &rodio::Sink) -> anyhow::Result<()> {
        let bytes = {
            let (chunks, current) = (self.chunks.lock().unwrap(), self.current.lock().unwrap());
            chunks
                .get(*current)
                .and_then(|c| c.bytes.clone())
                .ok_or_else(|| anyhow::anyhow!("no bytes for current chunk"))?
        };
        sink.stop();
        let decoder =
            rodio::Decoder::new(Cursor::new(bytes)).map_err(|e| anyhow::anyhow!("decode: {e}"))?;
        sink.append(decoder);
        sink.play();
        Ok(())
    }

    /// Toggle between Playing and Paused. From Ended → replay from the start.
    /// No-op if no clip is loaded (Idle/Loading/Error).
    pub fn toggle_pause(&self) {
        let sink_alive = self.sink.lock().unwrap().is_some();
        let state = self.state.lock().unwrap().clone();
        if !sink_alive {
            // No live sink: only an Ended clip has anything to do (play it
            // again from the start).
            if matches!(state, PlaybackState::Ended { .. }) {
                self.replay();
            }
            return;
        }
        match state {
            PlaybackState::Playing { .. } => {
                if let Some(sink) = self.sink.lock().unwrap().as_ref() {
                    sink.pause();
                }
                *self.paused.lock().unwrap() = true;
                let pos = self.global_pos().unwrap_or(Duration::ZERO).as_secs_f32();
                *self.state.lock().unwrap() = PlaybackState::Paused { pos };
            }
            PlaybackState::Paused { .. } => {
                if let Some(sink) = self.sink.lock().unwrap().as_ref() {
                    if sink.empty() {
                        let _ = self.restart_current_sink(sink);
                    } else {
                        sink.play();
                    }
                }
                *self.paused.lock().unwrap() = false;
                let pos = self.global_pos().unwrap_or(Duration::ZERO).as_secs_f32();
                *self.state.lock().unwrap() = PlaybackState::Playing { pos };
            }
            PlaybackState::Ended { .. } => {
                self.replay();
            }
            _ => {}
        }
    }

    /// Seek back to the start and play. Replays the loaded clip without
    /// re-synthesizing.
    pub fn replay(&self) {
        let ready = {
            let chunks = self.chunks.lock().unwrap();
            chunks.first().is_some_and(|c| c.bytes.is_some())
        };
        if !ready {
            return;
        }
        self.stop_sink();
        *self.current.lock().unwrap() = 0;
        *self.finished.lock().unwrap() = Duration::ZERO;
        *self.paused.lock().unwrap() = false;
        self.start_chunk(0);
    }

    /// Seek to a fraction (0.0..=1.0) of the clip's (estimated) total
    /// duration. Seeks into chunks whose audio is not installed yet are
    /// ignored — synthesis usually catches up long before a user reaches for
    /// the bar.
    pub fn seek(&self, fraction: f32) {
        let (durations, weights) = self.timeline_parts();
        if durations.is_empty() {
            return;
        }
        let total = karaoke::total_duration(&durations, &weights);
        let target = total.mul_f32(fraction.clamp(0.0, 1.0));
        let Some((index, elapsed)) = karaoke::locate(target, &durations, &weights) else {
            return;
        };
        let ready = {
            let chunks = self.chunks.lock().unwrap();
            chunks.get(index).is_some_and(|c| c.bytes.is_some())
        };
        if !ready {
            return;
        }
        let starts = karaoke::chunk_starts(&durations, &weights);
        let sink_alive = self.sink.lock().unwrap().is_some();
        let same_chunk = *self.current.lock().unwrap() == index && sink_alive;
        if same_chunk {
            let sink_guard = self.sink.lock().unwrap();
            let Some(sink) = sink_guard.as_ref() else {
                return;
            };
            let seek_result = if sink.empty() {
                self.restart_current_sink(sink).and_then(|_| {
                    sink.try_seek(elapsed)
                        .map_err(|e| anyhow::anyhow!("seek: {e}"))
                })
            } else {
                sink.try_seek(elapsed)
                    .map_err(|e| anyhow::anyhow!("seek: {e}"))
            };
            if let Err(e) = &seek_result {
                tracing::warn!(error = ?e, fraction, "tts: try_seek failed");
            }
            let was_playing = matches!(&*self.state.lock().unwrap(), PlaybackState::Playing { .. });
            if was_playing {
                sink.play();
            } else {
                sink.pause();
            }
        } else {
            // Jump to a different chunk: tear the current sink down and start
            // the target chunk fresh (paused if we were paused).
            self.stop_sink();
            *self.current.lock().unwrap() = index;
            *self.finished.lock().unwrap() = starts[index];
            self.start_chunk(index);
            return;
        }
        *self.finished.lock().unwrap() = starts[index];
        let pos = self.global_pos().unwrap_or(starts[index]).as_secs_f32();
        let playing = matches!(&*self.state.lock().unwrap(), PlaybackState::Playing { .. });
        *self.state.lock().unwrap() = if playing {
            PlaybackState::Playing { pos }
        } else {
            PlaybackState::Paused { pos }
        };
    }

    /// Stop and clear the current clip (e.g. when the popup closes).
    pub fn stop(&self) {
        *self.generation.lock().unwrap() += 1;
        self.stop_sink();
        self.clear_clip();
        *self.state.lock().unwrap() = PlaybackState::Idle;
    }

    /// Drop the clip entirely: chunks, timings, identity.
    fn clear_clip(&self) {
        self.chunks.lock().unwrap().clear();
        self.timings.lock().unwrap().clear();
        *self.current.lock().unwrap() = 0;
        *self.finished.lock().unwrap() = Duration::ZERO;
        *self.current_text.lock().unwrap() = String::new();
        *self.current_tts.lock().unwrap() = None;
        *self.pipeline_alive.lock().unwrap() = false;
        *self.paused.lock().unwrap() = false;
    }

    /// Stop playback if the loaded clip no longer matches what would be
    /// synthesized now: a different text (new selection / new translation)
    /// OR a different TTS key (options changed). Playing/Paused stale clips
    /// are stopped and the controller returns to Idle — continuing stale
    /// audio would contradict the UI ("new text · play"). Ended/Loading are
    /// untouched (the UI already exposes their stale affordance).
    ///
    /// `text`/`tts_key` are compared independently — pass `None` to skip a
    /// comparison (e.g. for a settings-only change).
    pub fn invalidate_on_change(&self, text: Option<&str>, tts_key: Option<&str>) {
        let stale = {
            let current_text = self.current_text.lock().unwrap();
            let current_tts = self.current_tts.lock().unwrap();
            !current_text.is_empty()
                && (text.is_some_and(|t| *current_text != t)
                    || tts_key.is_some_and(|k| current_tts.as_deref() != Some(k)))
        };
        if !stale {
            return;
        }
        let active = {
            let state = self.state.lock().unwrap();
            matches!(
                &*state,
                PlaybackState::Playing { .. } | PlaybackState::Paused { .. }
            )
        };
        if active {
            *self.generation.lock().unwrap() += 1;
            self.stop_sink();
            self.clear_clip();
            *self.state.lock().unwrap() = PlaybackState::Idle;
        }
    }

    /// Poll the live playback position. Call this on a timer (~10 Hz) while
    /// the popup is open to drive the seek bar, the word highlight, and the
    /// chunk-advance state machine.
    ///
    /// Note: `Sink::empty()` returns `true` for a brief moment right after
    /// `append()` and after `try_seek(0)` while the source buffer refills —
    /// so we only treat `empty` as "chunk finished" once playback has
    /// advanced past ~150 ms within the chunk.
    pub fn poll_progress(&self) {
        if self.sink.lock().unwrap().is_none() {
            return; // Idle, or parked between chunks waiting for bytes.
        }
        let (pos_in_chunk, empty) = {
            let sink = self.sink.lock().unwrap();
            let sink = sink.as_ref().expect("checked above");
            (sink.get_pos(), sink.empty())
        };
        if empty && pos_in_chunk > Duration::from_millis(150) {
            self.advance_chunk();
            return;
        }
        let finished = *self.finished.lock().unwrap();
        let pos = (finished + pos_in_chunk).as_secs_f32();
        let mut state = self.state.lock().unwrap();
        match &*state {
            PlaybackState::Playing { .. } => {
                *state = PlaybackState::Playing { pos };
            }
            PlaybackState::Paused { .. } => {
                *state = PlaybackState::Paused { pos };
            }
            _ => {}
        }
    }

    /// The current chunk's source drained: record its duration, move to the
    /// next chunk (start it, park in Loading until its bytes arrive, or end
    /// the clip).
    fn advance_chunk(&self) {
        // Tear the drained sink down first: from here on the next chunk owns
        // the audio (or the clip ends).
        self.stop_sink();
        let (current, last) = {
            let current = *self.current.lock().unwrap();
            let last = self.chunks.lock().unwrap().len().saturating_sub(1);
            (current, last)
        };
        let chunk_duration = {
            let chunks = self.chunks.lock().unwrap();
            chunks.get(current).and_then(|c| c.duration)
        };
        *self.finished.lock().unwrap() += chunk_duration.unwrap_or_default();
        if current >= last {
            let total = self.estimate_total();
            *self.state.lock().unwrap() = PlaybackState::Ended {
                pos: total.as_secs_f32(),
            };
            return;
        }
        let next = current + 1;
        *self.current.lock().unwrap() = next;
        let next_ready = {
            let chunks = self.chunks.lock().unwrap();
            chunks.get(next).is_some_and(|c| c.bytes.is_some())
        };
        if next_ready {
            self.start_chunk(next);
        } else if *self.pipeline_alive.lock().unwrap() {
            // Playback caught up with synthesis; park until install_chunk
            // starts the next chunk (maybe_start_current).
            *self.state.lock().unwrap() = PlaybackState::Loading;
        } else {
            // Synthesis failed mid-clip; end gracefully.
            let total = self.estimate_total();
            *self.state.lock().unwrap() = PlaybackState::Ended {
                pos: total.as_secs_f32(),
            };
        }
    }

    /// Snapshot the current observable state + total duration + the text and
    /// TTS key the current clip was synthesized with. Read by the popup each
    /// render for the play button, the status chip, and the seek bar.
    pub fn snapshot(&self) -> PlaybackSnapshot {
        let state = self.state.lock().unwrap().clone();
        let (total, text, tts) = {
            let chunks_empty = self.chunks.lock().unwrap().is_empty();
            let total = if chunks_empty {
                None
            } else {
                Some(self.estimate_total())
            };
            (
                total,
                self.current_text.lock().unwrap().clone(),
                self.current_tts.lock().unwrap().clone(),
            )
        };
        (state, total, text, tts)
    }

    /// The byte range of the word currently being spoken, per the karaoke
    /// estimate — `None` whenever nothing is actively sounding (idle, loading,
    /// between chunks, ended). The popup paints this range inside the editor
    /// whose text matches the clip.
    pub fn highlight_range(&self) -> Option<Range<usize>> {
        match &*self.state.lock().unwrap() {
            PlaybackState::Playing { .. } | PlaybackState::Paused { .. } => {}
            _ => return None,
        }
        let current = *self.current.lock().unwrap();
        let pos_in_chunk = {
            let sink = self.sink.lock().unwrap();
            let sink = sink.as_ref()?;
            if sink.empty() {
                return None;
            }
            sink.get_pos()
        };
        let timing = self.timings.lock().unwrap().get(current)?.clone();
        let duration = {
            let chunks = self.chunks.lock().unwrap();
            chunks.get(current).and_then(|c| c.duration)?
        };
        timing.word_at(pos_in_chunk, duration)
    }
}

/// Compute a clip's total duration from its encoded bytes.
///
/// Tries the decoder's `total_duration()` first (cheap, accurate when the
/// container/embedded metadata provides it). Falls back to fully decoding the
/// clip once and dividing sample count by sample rate — reliable for MP3 where
/// `total_duration()` returns `None`. For short TTS chunks the full decode is
/// negligible.
fn compute_total_duration(bytes: &[u8]) -> anyhow::Result<std::time::Duration> {
    // Fast path: trust the decoder's duration if it has one.
    let decoder = rodio::Decoder::new(Cursor::new(bytes.to_vec()))
        .map_err(|e| anyhow::anyhow!("decode: {e}"))?;
    if let Some(d) = decoder.total_duration() {
        return Ok(d);
    }

    // Fallback: decode fully, counting samples. sample_rate + channels let us
    // convert the sample count into seconds.
    let mut decoder = rodio::Decoder::new(Cursor::new(bytes.to_vec()))
        .map_err(|e| anyhow::anyhow!("decode: {e}"))?;
    let rate = decoder.sample_rate();
    let channels = decoder.channels() as u64;
    if rate == 0 || channels == 0 {
        anyhow::bail!("decode: zero sample_rate/channels");
    }
    let mut samples: u64 = 0;
    while decoder.next().is_some() {
        samples += 1;
    }
    let frames = samples / channels;
    let secs = frames as f64 / rate as f64;
    Ok(std::time::Duration::from_secs_f64(secs))
}

// ---------------------------------------------------------------------------
// Tests (pure logic — this crate's tests cannot construct a gpui App)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// `start_load` on fresh text chunks the clip and asks for per-chunk
    /// synthesis; the chunk texts match the timing ranges exactly.
    #[test]
    fn start_load_splits_into_chunks() {
        let c = PlaybackController::default();
        let action = c.start_load("First one. Second one?".to_string(), None, None);
        let LoadAction::Synthesize {
            generation, chunks, ..
        } = action
        else {
            panic!("expected synthesize");
        };
        assert_eq!(generation, 1);
        assert_eq!(chunks, vec!["First one.", "Second one?"]);
        assert!(matches!(&*c.state.lock().unwrap(), PlaybackState::Loading));
        assert_eq!(c.chunks.lock().unwrap().len(), 2);
    }

    /// The fast path: same text + no TTS settings twice → replay, and the
    /// generation does NOT advance (the pending task stays valid).
    #[test]
    fn start_load_same_text_replays() {
        let c = PlaybackController::default();
        // Simulate an installed clip: run a load, then fake chunk 0 bytes.
        let _ = c.start_load("Hello.".to_string(), None, None);
        assert!(c.install_chunk(1, 0, vec![0u8; 16]));
        let action = c.start_load("Hello.".to_string(), None, None);
        assert!(matches!(action, LoadAction::Replay));
        // Replay resets the chunk position but keeps the clip.
        assert_eq!(*c.current.lock().unwrap(), 0);
        assert_eq!(*c.finished.lock().unwrap(), Duration::ZERO);
    }

    /// A stale install (generation bumped by a newer load) is rejected so the
    /// old task stops feeding the new clip.
    #[test]
    fn stale_generation_is_rejected() {
        let c = PlaybackController::default();
        let _ = c.start_load("A. B.".to_string(), None, None); // gen 1
        let _ = c.start_load("C. D.".to_string(), None, None); // gen 2
        assert!(!c.install_chunk(1, 0, vec![0u8; 8]));
        assert!(c.install_chunk(2, 0, vec![0u8; 8]));
    }

    /// Mid-clip synthesis failure keeps playing what's loaded; a failure
    /// before anything played surfaces the error.
    #[test]
    fn fail_graceful_mid_clip() {
        let c = PlaybackController::default();
        let _ = c.start_load("A.".to_string(), None, None);
        c.fail("boom".into());
        assert!(matches!(&*c.state.lock().unwrap(), PlaybackState::Error(e) if e == "boom"));

        let c = PlaybackController::default();
        let _ = c.start_load("One. Two.".to_string(), None, None);
        // The second chunk lands while playback sits on chunk 0…
        assert!(c.install_chunk(1, 1, vec![0u8; 8]));
        // …then playback moves past chunk 0 (no real audio in tests, so we
        // move the index by hand).
        *c.current.lock().unwrap() = 1;
        c.fail("boom".into());
        assert!(!matches!(
            &*c.state.lock().unwrap(),
            PlaybackState::Error(_)
        ));
    }

    /// stop() clears everything and bumps the generation.
    #[test]
    fn stop_clears_clip() {
        let c = PlaybackController::default();
        let _ = c.start_load("A. B.".to_string(), None, None);
        c.stop();
        assert!(matches!(&*c.state.lock().unwrap(), PlaybackState::Idle));
        assert!(c.chunks.lock().unwrap().is_empty());
        assert!(c.current_text.lock().unwrap().is_empty());
        let _ = c.start_load("X.".to_string(), None, None);
        // generation is now 3; a task from generation 1 must be rejected.
        assert!(!c.install_chunk(1, 0, vec![0u8; 8]));
    }

    /// highlight_range only speaks while a chunk sink is live.
    #[test]
    fn highlight_needs_active_playback() {
        let c = PlaybackController::default();
        assert!(c.highlight_range().is_none());
        let _ = c.start_load("Hello world.".to_string(), None, None);
        assert!(c.highlight_range().is_none()); // Loading
    }
}
