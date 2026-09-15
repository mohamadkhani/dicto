//! Telemetry for the Dicto desktop app.
//!
//! Privacy-first, opt-in analytics. The app calls [`init`] once at startup
//! with the user's consent state and an anonymous installation id. From then
//! on, any call site — UI handler or background thread — can call
//! [`get`]().track`(event)` to record an event.
//!
//! ## Design
//!
//! - Events are an [`Event`] enum (compile-time-safe event names — no string
//!   typos that silently miss the dashboard).
//! - The public [`Telemetry`] trait has two implementations:
//!   - [`AptabaseClient`] — the real client. Owns its own
//!     [`tokio::runtime::Runtime`], batches events, flushes to
//!     `https://(us|eu).aptabase.com/api/v0/events`.
//!   - [`NullTelemetry`] — a no-op. Used before [`init`] and whenever the user
//!     has not opted in. Tracking is then a zero-cost no-op with no HTTP.
//! - The [`Telemetry`] API is **synchronous** and infallible. Calling sites
//!   never `.await` and never handle errors: analytics is best-effort.
//! - Failed sends are dropped (warn-logged). The crate is a fire-and-forget
//!   sink, much like `tracing` itself.
//!
//! ## Privacy
//!
//! - No persistent user identifier. The supplied `installation_id` is a
//!   local UUID v4, attached to events as a custom property. It identifies an
//!   installation, not a person.
//! - Session ids are generated at [`init`] (one per process launch) and are
//!   NOT persisted across launches.
//! - [`Event::ErrorOccurred`] messages are truncated to 180 chars and any
//!   absolute path prefixes (`/home/...`, `C:\\...`) are stripped before
//!   sending, so usernames and filesystem layout never leak.

mod aptabase;
mod event;
mod null;
mod sysprops;

use std::sync::Arc;

pub use aptabase::AptabaseClient;
pub use event::{
    ErrorKind, Event, LookupSource, PlaybackFailureReason, QuickTranslateSource, SettingsChange,
    SettingsSource, SettingsTab,
};
pub use null::NullTelemetry;

/// Aptabase App Key. This value is intentionally public (Aptabase ships the
/// key in client apps by design — it identifies the app, it is NOT used for
/// authentication). Replace the placeholder with a real key from
/// https://aptabase.com once an app is created there.
///
/// While this stays a non-key placeholder, [`init`] selects [`NullTelemetry`]
/// so the app compiles and runs without sending anything.
const APTABASE_APP_KEY: &str = "A-EU-4565835089";

/// The public analytics interface. Implementations are cheap to clone
/// (`Arc`-backed internally) and safe to call from any thread.
pub trait Telemetry: Send + Sync {
    /// Record an event. Always safe to call; a no-op when the user has not
    /// opted in or the client isn't initialized.
    fn track(&self, event: Event);
}

use std::sync::RwLock;

static TELEMETRY: RwLock<Option<Arc<dyn Telemetry>>> = RwLock::new(None);

/// Initialize (or replace) the global telemetry client.
///
/// Safe to call at any time — including from the consent dialog after the user
/// opts in mid-session. The first startup call installs a client (real or
/// `NullTelemetry`); a later call (e.g. the user clicking "Allow") swaps it,
/// so the new choice takes effect in the **current** process without a restart.
///
/// - `enabled` — whether the user has opted in (`consent == OptedIn`).
///   When false, or when [`APTABASE_APP_KEY`] is still a placeholder,
///   [`NullTelemetry`] is installed and tracking becomes a no-op.
/// - `installation_id` — the local anonymous installation id (a UUID v4).
///   Attached to every event as a custom property.
/// - `app_version` — the Dicto version, sent as a system property.
pub fn init(enabled: bool, installation_id: String, app_version: String) {
    let client: Arc<dyn Telemetry> = if enabled && !is_placeholder_key() {
        match AptabaseClient::new(
            APTABASE_APP_KEY.to_string(),
            installation_id,
            app_version,
        ) {
            Ok(client) => Arc::new(client),
            // Constructing the runtime failed — degrade to no-op rather than
            // crash the app over analytics.
            Err(e) => {
                tracing::warn!("telemetry: failed to init client, falling back to no-op: {e}");
                Arc::new(NullTelemetry)
            }
        }
    } else {
        Arc::new(NullTelemetry)
    };

    match TELEMETRY.write() {
        Ok(mut slot) => *slot = Some(client),
        Err(poisoned) => *poisoned.into_inner() = Some(client),
    }
}

/// Always-safe accessor. Returns a [`NullTelemetry`] fallback if [`init`] has
/// not been called yet, so any call site can use `telemetry::get().track(...)`
/// without guarding for availability or consent.
pub fn get() -> Arc<dyn Telemetry> {
    TELEMETRY
        .read()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_else(|| Arc::new(NullTelemetry))
}

fn is_placeholder_key() -> bool {
    APTABASE_APP_KEY.ends_with("0000000000") || APTABASE_APP_KEY.is_empty()
}
