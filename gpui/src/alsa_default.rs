//! Per-process repair of the ALSA `default` PCM on Linux.
//!
//! rodio/cpal open the literal ALSA PCM name `"default"`. On a healthy setup
//! that name resolves to the PulseAudio/PipeWire server (via the `alsa.conf.d`
//! plugin configs) and therefore follows the desktop's chosen output device.
//! But when those configs are missing from alsa-lib's search path — e.g.
//! alsa-lib ≥ 1.2.15 only loads `/etc/alsa/conf.d` while `pipewire-alsa` /
//! `pulseaudio-alsa` are not installed — `"default"` silently falls back to
//! raw card hardware. Audio then always plays from the physical speaker and
//! never follows the user's default device, and the sound server never sees
//! the stream at all.
//!
//! Fix: before the first audio open, set `ALSA_CONFIG_PATH` to a copy of the
//! system `alsa.conf` with a `pcm.!default { type pulse fallback
//! "sysdefault" }` override appended. Systems whose plugin configs already
//! define `default` re-override us afterwards (alsa-lib loads `conf.d` and
//! `asoundrc` files via hooks that run after our body), so nothing changes
//! there. Broken systems get server-routed audio, with raw-hw fallback if no
//! sound server is running. The override is per-process — no system files are
//! touched — and is skipped entirely if the user already set
//! `ALSA_CONFIG_PATH` themselves.

use std::path::PathBuf;

/// The appended override: route `default` through the sound server via the
/// `pulse` plugin (which both PulseAudio and PipeWire's pulse replacement
/// speak), falling back to plain hardware when no server answers.
const OVERRIDE: &str = "\n# dicto: keep the default PCM on the sound server even when the\n\
                        # system ALSA config does not define it (alsa-lib >= 1.2.15\n\
                        # loads only /etc/alsa/conf.d). Safe to ignore: any later\n\
                        # conf.d / asoundrc definition overrides this one.\n\
                        pcm.!default {\n\ttype pulse\n\tfallback \"sysdefault\"\n\thint {\n\t\tshow on\n\t\tdescription \"Default ALSA Output (via sound server)\"\n\t}\n}\n\
                        ctl.!default { type pulse }\n";

/// Install the per-process ALSA override. Call once, early in `main`, before
/// any audio device is opened. Best-effort: every failure path leaves the
/// environment untouched, so behavior degrades to the status quo.
pub fn ensure_server_routed_default() {
    if std::env::var_os("ALSA_CONFIG_PATH").is_some() {
        tracing::debug!("alsa: ALSA_CONFIG_PATH already set, leaving it alone");
        return;
    }

    let system_conf = PathBuf::from("/usr/share/alsa/alsa.conf");
    let Ok(base) = std::fs::read_to_string(&system_conf) else {
        tracing::debug!(
            "alsa: {} not readable, skipping default-PCM override",
            system_conf.display()
        );
        return;
    };

    let Some(stub) = stub_path() else {
        tracing::debug!("alsa: no cache directory available, skipping default-PCM override");
        return;
    };
    if let Err(e) = std::fs::write(&stub, format!("{base}{OVERRIDE}")) {
        tracing::warn!("alsa: writing {} failed: {e}", stub.display());
        return;
    }

    // SAFETY: single-threaded at this point (called from `main` before any
    // audio/GUI machinery starts), and libasound reads the variable only
    // when the first PCM is opened, which happens strictly later.
    unsafe { std::env::set_var("ALSA_CONFIG_PATH", &stub) };
    tracing::info!(
        "alsa: default PCM routed via sound server ({})",
        stub.display()
    );
}

/// Where the generated override config lives: the app's cache dir, so it
/// survives across runs but is disposable. `None` if no home/cache can be
/// determined.
fn stub_path() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|h| h.join(".cache"))
        })?;
    let dir = cache.join("dicto");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("alsa-server-default.conf"))
}
