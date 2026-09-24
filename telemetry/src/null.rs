//! No-op telemetry. Installed by [`crate::init`] when the user hasn't opted
//! in or when the Aptabase app key is still a placeholder. [`track`] is a
//! zero-cost no-op, so opted-out call sites pay nothing.

use crate::Telemetry;
use crate::event::Event;

pub struct NullTelemetry;

impl Telemetry for NullTelemetry {
    #[inline]
    fn track(&self, _event: Event) {}
}
