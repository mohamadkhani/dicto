//! The real Aptabase telemetry client.
//!
//! Owns a private [`tokio::runtime::Runtime`] so call sites stay synchronous.
//! Events are pushed onto an unbounded mpsc channel; a single background task
//! batches them (≤ [`MAX_BATCH`] events ≤ [`FLUSH_INTERVAL`]) and POSTs to the
//! Aptabase `/api/v0/events` endpoint.
//!
//! Wire format follows Aptabase's documented API: an array of event bodies,
//! each with `timestamp`, `sessionId`, `eventName`, `systemProps`, `props`.

use std::sync::Arc;
use std::time::Duration;

use std::collections::BTreeMap;

use serde::{Serialize, Serializer};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::event::{sanitize_message, ErrorKind, Event};
use crate::sysprops::SystemProps;
use crate::Telemetry;

/// Flush when this many events accumulate.
const MAX_BATCH: usize = 20;
/// Flush at most this often (beats the case of a session that never hits the
/// count threshold).
const FLUSH_INTERVAL: Duration = Duration::from_secs(30);
/// Aptabase caps each batch at 25 events.
const APTABASE_BATCH_LIMIT: usize = 25;

pub struct AptabaseClient {
    sender: mpsc::UnboundedSender<OutgoingEvent>,
    _runtime: Runtime,
}

/// Serialize a list of (key, value) pairs as a JSON **object**, not an array.
///
/// `Vec<(String, String)>` serializes by default as `[["k","v"], ...]`, but the
/// Aptabase API expects `props` to be an object: `{"k": "v", ...}`. A
/// wrong-shaped `props` makes the whole event invalid and Aptabase silently
/// drops it server-side (HTTP 200, no dashboard entry).
fn serialize_props<S: Serializer>(
    props: &Option<Vec<(String, String)>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match props {
        None => serializer.serialize_none(),
        Some(pairs) => {
            // BTreeMap gives stable key ordering and serializes as { ... }.
            let map: BTreeMap<&str, &str> =
                pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            map.serialize(serializer)
        }
    }
}

/// A single enqueued event in its near-wire form (pre-batch).
#[derive(Debug, Serialize)]
struct OutgoingEvent {
    timestamp: String,
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "eventName")]
    event_name: String,
    #[serde(rename = "systemProps")]
    system_props: SystemProps,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "serialize_props")]
    props: Option<Vec<(String, String)>>,
}

impl AptabaseClient {
    /// Construct a client. Spawns the internal runtime + background flusher.
    ///
    /// Errors only if the runtime can't be created (rare; e.g. out of fds).
    pub fn new(
        app_key: String,
        installation_id: String,
        app_version: String,
    ) -> Result<Self, std::io::Error> {
        let runtime = Runtime::new()?;
        let (tx, rx) = mpsc::unbounded_channel::<OutgoingEvent>();

        let system_props = Arc::new(crate::sysprops::collect(app_version));
        let session_id = Arc::new(new_session_id());
        let installation_id = Arc::new(installation_id);
        let app_key = Arc::new(app_key);

        runtime.spawn(flusher(
            rx,
            system_props,
            session_id,
            installation_id,
            app_key,
        ));

        Ok(Self {
            sender: tx,
            _runtime: runtime,
        })
    }
}

impl Telemetry for AptabaseClient {
    fn track(&self, event: Event) {
        let (name, props) = into_wire(event);
        let wire = OutgoingEvent {
            timestamp: now_iso8601(),
            session_id: String::new(), // filled by flusher with the shared id
            event_name: name,
            system_props: SystemProps {
                os_name: String::new(),
                os_version: String::new(),
                app_version: String::new(),
                locale: String::new(),
                sdk_version: String::new(),
            }, // filled by flusher with shared system_props
            props,
        };
        // A full send buffer only happens if the runtime is gone; drop silently.
        let _ = self.sender.send(wire);
    }
}

/// Map an [`Event`] to its Aptabase event name + custom properties.
/// `installation_id` is NOT added here — the flusher adds it to every event's
/// `props` before sending, so it can never be forgotten at a call site.
fn into_wire(event: Event) -> (String, Option<Vec<(String, String)>>) {
    match event {
        Event::AppStarted => ("app_started".to_string(), None),
        Event::WindowOpened => ("window_opened".to_string(), None),
        Event::WindowClosed => ("window_closed".to_string(), None),
        Event::LookupPerformed { source } => (
            "lookup_performed".to_string(),
            Some(vec![("source".to_string(), source.as_str().to_string())]),
        ),
        Event::PronunciationPlayed => ("pronunciation_played".to_string(), None),
        Event::PronunciationPlaybackFailed { reason } => (
            "pronunciation_playback_failed".to_string(),
            Some(vec![("reason".to_string(), reason.as_str().to_string())]),
        ),
        Event::DictionaryImported {
            count,
            duration_ms,
            size_mb,
        } => (
            "dictionary_imported".to_string(),
            Some(vec![
                ("count".to_string(), count.to_string()),
                ("duration_ms".to_string(), duration_ms.to_string()),
                ("size_mb".to_string(), size_mb.to_string()),
            ]),
        ),
        Event::SettingsChanged { change } => (
            "settings_changed".to_string(),
            Some(vec![("change".to_string(), change.as_str().to_string())]),
        ),
        Event::SettingsOpened { source } => (
            "settings_opened".to_string(),
            Some(vec![("source".to_string(), source.as_str().to_string())]),
        ),
        Event::SettingsTabSelected { tab } => (
            "settings_tab_selected".to_string(),
            Some(vec![("tab".to_string(), tab.as_str().to_string())]),
        ),
        Event::ErrorOccurred { kind, message } => {
            let cleaned = sanitize_message(&message);
            let kind = match kind {
                ErrorKind::Indexing => "indexing",
                ErrorKind::Import => "import",
            };
            (
                "error_occurred".to_string(),
                Some(vec![
                    ("kind".to_string(), kind.to_string()),
                    ("message".to_string(), cleaned),
                ]),
            )
        }
        Event::QuickTranslateTriggered { source } => (
            "quick_translate_triggered".to_string(),
            Some(vec![("source".to_string(), source.as_str().to_string())]),
        ),
        Event::QuickTranslateCompleted {
            provider,
            success,
            duration_ms,
        } => (
            "quick_translate_completed".to_string(),
            Some(vec![
                ("provider".to_string(), provider.to_string()),
                ("success".to_string(), success.to_string()),
                ("duration_ms".to_string(), duration_ms.to_string()),
            ]),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
async fn flusher(
    mut rx: mpsc::UnboundedReceiver<OutgoingEvent>,
    system_props: Arc<SystemProps>,
    session_id: Arc<String>,
    installation_id: Arc<String>,
    app_key: Arc<String>,
) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("reqwest client build");

    let host = host_for_key(&app_key);
    let url = format!("{host}/api/v0/events");

    let mut buffer: Vec<OutgoingEvent> = Vec::with_capacity(MAX_BATCH);

    loop {
        let tick = tokio::time::sleep(FLUSH_INTERVAL);

        tokio::pin!(tick);

        // Either we accumulate MAX_BATCH events, or the timer fires — whichever
        // first, we flush.
        tokio::select! {
            biased;

            _ = &mut tick => {
                if buffer.is_empty() {
                    continue;
                }
                // timer flush
            }

            Some(ev) = rx.recv() => {
                buffer.push(ev);
                if buffer.len() < MAX_BATCH {
                    continue;
                }
                // count flush
            }

            else => {
                // sender dropped (runtime shutting down) — final flush attempt.
                if !buffer.is_empty() {
                    finalize_and_send(&client, &url, app_key.as_str(), &mut buffer, &system_props, &session_id, &installation_id).await;
                }
                return;
            }
        }

        // Drain anything else that's already queued before sending, up to the
        // Aptabase per-request limit. Surplus stays for the next flush.
        while buffer.len() < APTABASE_BATCH_LIMIT {
            match rx.try_recv() {
                Ok(ev) => buffer.push(ev),
                Err(_) => break,
            }
        }
        finalize_and_send(&client, &url, app_key.as_str(), &mut buffer, &system_props, &session_id, &installation_id).await;
    }
}

/// Fill shared fields (session id, system props, install id) on each buffered
/// event, split into Aptabase-sized chunks, and POST. Failed sends are
/// dropped (warn-logged) — telemetry is best-effort.
async fn finalize_and_send(
    client: &reqwest::Client,
    url: &str,
    app_key: &str,
    buffer: &mut Vec<OutgoingEvent>,
    system_props: &Arc<SystemProps>,
    session_id: &Arc<String>,
    installation_id: &Arc<String>,
) {
    if buffer.is_empty() {
        return;
    }

    let total = buffer.len();
    let mut sent = 0;
    while sent < total {
        let chunk_end = (sent + APTABASE_BATCH_LIMIT).min(total);

        // Finalize each event in the chunk with the shared session/system
        // props and the installation id, then collect owned clones for
        // serialization (avoids holding &mut while we serialize &).
        let mut batch: Vec<OutgoingEvent> = Vec::with_capacity(chunk_end - sent);
        for ev in &buffer[sent..chunk_end] {
            let mut ev = OutgoingEvent {
                timestamp: ev.timestamp.clone(),
                session_id: session_id.as_str().to_string(),
                event_name: ev.event_name.clone(),
                system_props: SystemProps {
                    os_name: system_props.os_name.clone(),
                    os_version: system_props.os_version.clone(),
                    app_version: system_props.app_version.clone(),
                    locale: system_props.locale.clone(),
                    sdk_version: system_props.sdk_version.clone(),
                },
                props: ev.props.clone(),
            };
            // Attach installation_id as a custom property on every event.
            let props = ev.props.get_or_insert_with(Vec::new);
            props.push((
                "installation_id".to_string(),
                installation_id.as_str().to_string(),
            ));
            batch.push(ev);
        }

        match serde_json::to_value(&batch) {
            Ok(json) => {
                let resp = client
                    .post(url)
                    .header("App-Key", app_key)
                    .header("Content-Type", "application/json")
                    .json(&json)
                    .send()
                    .await;
                let count = chunk_end - sent;
                match resp {
                    Ok(r) if r.status().is_success() => {
                        tracing::debug!("telemetry: flushed {count} events");
                    }
                    Ok(r) => {
                        tracing::warn!(
                            "telemetry: aptabase returned status {}; dropping batch",
                            r.status()
                        );
                    }
                    Err(e) => {
                        tracing::warn!("telemetry: send failed; dropping batch: {e}");
                    }
                }
            }
            Err(e) => {
                tracing::warn!("telemetry: serialize failed; dropping batch: {e}");
            }
        }

        sent = chunk_end;
    }

    buffer.clear();
}

/// Derive the Aptabase API host from the app key's region prefix:
/// `A-EU-...` → EU, otherwise US.
fn host_for_key(app_key: &str) -> &'static str {
    if app_key.contains("EU") {
        "https://eu.aptabase.com"
    } else {
        "https://us.aptabase.com"
    }
}

/// Generate an Aptabase-style session id: `epochSeconds` (10 digits) + 8
/// random digits. The start-time encoding lets Aptabase reject stale sessions
/// server-side.
fn new_session_id() -> String {
    use rand::Rng;
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let rand_part: u32 = rand::rng().random_range(0..100_000_000);
    format!("{epoch}{rand_part:08}")
}

/// RFC 3339 / ISO 8601 UTC timestamp for the event body.
fn now_iso8601() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    // Minimal UTC formatting: YYYY-MM-DDTHH:MM:SSZ derived from epoch seconds.
    epoch_to_iso8601(secs)
}

/// Convert epoch seconds to an ISO 8601 UTC string without external crates.
fn epoch_to_iso8601(epoch: u64) -> String {
    let days = epoch / 86_400;
    let secs_of_day = (epoch % 86_400) as u32;
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    let (year, month, day) = civil_from_days(days as i64);

    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    )
}

/// Howard Hinnant's days→(year,month,day) algorithm. Proleptic Gregorian.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + if m <= 2 { 1 } else { 0 }, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_18_digits() {
        let id = new_session_id();
        assert_eq!(id.len(), 18);
        assert!(id.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn eu_key_routes_to_eu() {
        assert_eq!(host_for_key("A-EU-1234567890"), "https://eu.aptabase.com");
    }

    #[test]
    fn us_key_routes_to_us() {
        assert_eq!(host_for_key("A-US-1234567890"), "https://us.aptabase.com");
    }

    #[test]
    fn epoch_to_iso8601_known_value() {
        // 2021-01-01T00:00:00Z == 1609459200
        assert_eq!(epoch_to_iso8601(1_609_459_200), "2021-01-01T00:00:00Z");
    }
}
