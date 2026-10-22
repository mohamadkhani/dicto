//! Popup placement through the `window-calls` GNOME Shell extension.
//!
//! Wayland clients can neither set nor read a toplevel's position, so the
//! quick-translate popup cannot restore its dragged position on its own.
//! The `window-calls@domandoman.xyz` extension (compositor-side code)
//! exposes the needed D-Bus methods on GNOME Shell's bus; we call them via
//! the `gdbus` CLI to avoid a D-Bus dependency:
//!
//! - [`find_popup_winid`] locates the popup by its generation-tagged title
//!   (unique per open) + pid among `List`'s JSON windows.
//! - [`save_popup_rect`] reads the popup's frame rect right before it
//!   closes and stores it in this process. Bounded: the window dies right
//!   after, so the query cannot be fully async.
//! - [`move_popup_async`] moves a freshly-opened popup to the stored rect,
//!   retrying while the window is still mapping.
//!
//! Everything no-ops when the extension is disabled or absent (non-GNOME
//! compositors, X11 — where the popup view's bounds observer restores the
//! position natively). Enable the extension with
//! `gnome-extensions enable window-calls@domandoman.xyz`.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

const DBUS_DEST: &str = "org.gnome.Shell";
const DBUS_PATH: &str = "/org/gnome/Shell/Extensions/Windows";
const DBUS_IFACE: &str = "org.gnome.Shell.Extensions.Windows";

/// Title prefix for popup windows (see `open_translate_popup`);
/// [`next_popup_title`] appends a unique generation number.
const POPUP_TITLE_PREFIX: &str = "Dicto Translate #";

/// Generation counter for popup window titles. Bumped by
/// [`next_popup_title`] on each open; [`find_popup_winid`] matches the
/// CURRENT generation's full title. During the replace flow (old popup
/// dying while the new one maps, both alive in the extension's `List`
/// with the same pid) a plain-title match can resolve to the STALE
/// window, sending `Move`/`Activate` to the dead one while the fresh
/// popup waits unmoved and unraised. The generation tag makes that
/// impossible: a previous window can never match the current needle.
static POPUP_GEN: AtomicU64 = AtomicU64::new(0);

/// Mint the title for the popup window being opened — "Dicto Translate #N"
/// with N unique per open. `open_translate_popup` puts this into
/// `WindowOptions`; the extension-side finders match it exactly.
pub fn next_popup_title() -> String {
    // Post-increment: the counter always holds the number in the LIVE
    // window's title — `find_popup_winid` matches exactly that value.
    let n = POPUP_GEN.fetch_add(1, Ordering::AcqRel) + 1;
    format!("{POPUP_TITLE_PREFIX}{n}")
}

/// Stored frame-rect origin from the last close; `SAVED` gates validity.
static SAVED_X: AtomicU64 = AtomicU64::new(0);
static SAVED_Y: AtomicU64 = AtomicU64::new(0);
static SAVED: AtomicBool = AtomicBool::new(false);

/// False while a freshly-opened popup is still being moved to its saved
/// position. The popup view renders nothing until this flips, so the user
/// never sees the window flash at the compositor's default spot first.
static PLACED: AtomicBool = AtomicBool::new(true);

/// Deadline (ms since the Unix epoch) after which [`is_placed`] turns
/// true even if the mover thread never confirmed placement. A stuck
/// `PLACED=false` would otherwise leave the popup invisible forever —
/// present in the GNOME dock as an "opening" window, but with nothing
/// (or a zero-dimension surface) on screen.
static PLACE_DEADLINE_MS: AtomicU64 = AtomicU64::new(0);

/// Call when a popup window is about to open. `restore` tells whether a
/// move to a saved position is pending — with nothing to restore the popup
/// is renderable immediately.
pub fn begin_popup_placement(restore: bool) {
    PLACED.store(!restore, Ordering::Release);
    if restore {
        // Covers the mover's full budget (find ≤1s + 250ms + hold ≤450ms)
        // plus slack; only a dead/misbehaved mover ever hits it.
        let deadline = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64 + 2000)
            .unwrap_or(0);
        PLACE_DEADLINE_MS.store(deadline, Ordering::Release);
    }
}

/// Whether the popup may render (its position is final, was never
/// deferred, or the placement deadline expired — the popup must never
/// stay invisible because a mover thread died or mismatched).
pub fn is_placed() -> bool {
    if PLACED.load(Ordering::Acquire) {
        return true;
    }
    let deadline = PLACE_DEADLINE_MS.load(Ordering::Acquire);
    deadline == 0
        || std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64 > deadline)
            .unwrap_or(true)
}

/// Whether the placement hint is worth showing at all: only GNOME Wayland
/// sessions can use (and lack) the helper extension.
pub fn placement_hint_relevant() -> bool {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    let gnome = std::env::var("XDG_CURRENT_DESKTOP")
        .map(|d| d.to_uppercase().contains("GNOME"))
        .unwrap_or(false);
    wayland && gnome
}

/// Whether the window-calls extension answers — i.e. installed AND
/// enabled. One ~10ms gdbus round trip; run off the UI thread.
pub fn extension_available() -> bool {
    gdbus("List", &[]).is_some()
}

/// The extension's D-Bus methods, if reachable. `gdbus` spawns are cheap
/// (~10ms) and rare (a few per popup open/close).
fn gdbus(method: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("gdbus")
        .stdin(Stdio::null())
        .arg("call")
        .arg("--session")
        .arg("--dest")
        .arg(DBUS_DEST)
        .arg("--object-path")
        .arg(DBUS_PATH)
        .arg("--method")
        .arg(format!("{DBUS_IFACE}.{method}"))
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        // gdbus prints the variant text either plainly or with escaped
        // quotes depending on context; dropping backslashes normalizes both
        // (our needles contain no literal backslashes).
        .then(|| String::from_utf8_lossy(&output.stdout).replace('\\', ""))
}

/// The popup's window id from the extension's `List` JSON: the one entry
/// whose title is the CURRENT generation's (see [`POPUP_GEN`]) and whose
/// pid matches this process.
fn find_popup_winid() -> Option<u32> {
    let list = gdbus("List", &[])?;
    let pid = std::process::id().to_string();
    let pid_field = format!("\"pid\":{pid}");
    let title_field = format!(
        "\"title\":\"{POPUP_TITLE_PREFIX}{}\"",
        POPUP_GEN.load(Ordering::Acquire)
    );
    for entry in list.split("},{") {
        if entry.contains(&title_field) && entry.contains(&pid_field) {
            let id = entry
                .split("\"id\":")
                .nth(1)?
                .split([',', '}'])
                .next()?
                .trim()
                .parse::<u32>()
                .ok();
            if id.is_some() {
                return id;
            }
        }
    }
    None
}

/// Frame rect origin + size of the live popup, from `GetFrameRect`'s JSON:
/// `{"x": 412, "y": 250, "width": 460, "height": 300}`.
fn popup_rect_sync() -> Option<(i32, i32)> {
    let winid = find_popup_winid()?.to_string();
    let out = gdbus("GetFrameRect", &[&winid])?;
    let x = out
        .split("\"x\":")
        .nth(1)?
        .split([',', '}'])
        .next()?
        .trim()
        .parse()
        .ok()?;
    let y = out
        .split("\"y\":")
        .nth(1)?
        .split([',', '}'])
        .next()?
        .trim()
        .parse()
        .ok()?;
    Some((x, y))
}

/// Remember where the user left the popup. Called right before the popup
/// window is destroyed. Synchronous ON PURPOSE: the window disappears
/// moments later, and an async query would race its teardown. Two gdbus
/// spawns cost ~20ms — imperceptible on dismissal.
pub fn save_popup_rect() {
    let Some((x, y)) = popup_rect_sync() else {
        tracing::debug!("window_move: popup rect unavailable at close");
        return;
    };
    SAVED_X.store(x.max(0) as u64, Ordering::Release);
    SAVED_Y.store(y.max(0) as u64, Ordering::Release);
    SAVED.store(true, Ordering::Release);
    tracing::debug!(x, y, "window_move: saved popup position");
}

/// The rect origin saved by the last [`save_popup_rect`].
pub fn saved_pos() -> Option<(i32, i32)> {
    if !SAVED.load(Ordering::Acquire) {
        return None;
    }
    Some((
        SAVED_X.load(Ordering::Acquire) as i32,
        SAVED_Y.load(Ordering::Acquire) as i32,
    ))
}

/// Focus (raise + give keyboard focus to) the popup after it maps.
///
/// This is the reliable focus path when Dicto is a BACKGROUND app: Mutter
/// refuses keyboard focus for freshly mapped windows of background apps
/// (focus-stealing prevention), and activation tokens only exist when a
/// focused app mints them (tray clicks). The window-calls extension runs
/// INSIDE the compositor, so its Activate is not subject to that policy.
/// Requires a version of the extension with the `Activate` method; older
/// versions no-op (gdbus returns an error, we give up quietly).
/// Fire-and-forget. Retries while the window is still mapping, and until
/// the deadline passes.
pub fn focus_popup_async() {
    std::thread::spawn(move || {
        let winid = wait_popup_winid();
        let Some(winid) = winid else {
            tracing::debug!("window_move: popup never appeared to focus");
            return;
        };
        activate_until_deadline(winid, Duration::from_millis(1000));
    });
}

/// Wait (bounded) for the extension to see the popup window; returns its id.
fn wait_popup_winid() -> Option<u32> {
    for _ in 0..50 {
        if let Some(id) = find_popup_winid() {
            return Some(id);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

/// Call `Activate` on the window until it succeeds or `deadline` passes.
fn activate_until_deadline(winid: u32, deadline_from_now: Duration) {
    let id = winid.to_string();
    let deadline = std::time::Instant::now() + deadline_from_now;
    while std::time::Instant::now() < deadline {
        if gdbus("Activate", &[&id]).is_some() {
            tracing::debug!(winid, "window_move: popup focused");
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    tracing::debug!(
        winid,
        "window_move: Activate unsupported by installed window-calls version"
    );
}

/// Move the popup window to (x, y), and — once the position has survived
/// Mutter's late placement override — focus it.
///
/// The move waits for the window to appear in the extension's `List` — the
/// extension only sees windows Mutter already knows about — and then
/// re-applies the position for a short hold, because the compositor's own
/// initial placement can land *after* our move (late first-configure) and
/// would otherwise win. Focusing FIRST would race that placement fight:
/// Activate fires while the window is still configuring (ignored) or
/// expires before the move settles. Fire-and-forget.
pub fn move_popup_async(x: i32, y: i32) {
    std::thread::spawn(move || {
        let mut winid = None;
        for _ in 0..50 {
            if let Some(id) = find_popup_winid()
                && gdbus("Move", &[&id.to_string(), &x.to_string(), &y.to_string()]).is_some()
            {
                winid = Some(id);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let Some(winid) = winid else {
            tracing::debug!(x, y, "window_move: popup never appeared to move");
            PLACED.store(true, Ordering::Release);
            // The move failed (extension slow/absent, window slow to map)
            // but the fresh popup still MUST be raised: as a background
            // app, Mutter denies its freshly-mapped window keyboard focus,
            // so without an Activate it opens BELOW other windows. Bounded
            // retry; no-ops without the extension.
            if let Some(id) = wait_popup_winid() {
                activate_until_deadline(id, Duration::from_millis(1000));
            }
            return;
        };
        tracing::debug!(winid, x, y, "window_move: popup moved");
        // Mutter applies its own initial placement ~100ms after our move
        // (late first-configure). Reveal the popup only once the position
        // has survived that override, so it never flashes at the default
        // spot; the hold keeps correcting until the deadline regardless.
        std::thread::sleep(Duration::from_millis(250));
        let deadline = std::time::Instant::now() + Duration::from_millis(450);
        while std::time::Instant::now() < deadline {
            let Some((cx, cy)) = popup_rect_at(winid) else {
                std::thread::sleep(Duration::from_millis(40));
                continue;
            };
            if (cx - x).abs() > 1 || (cy - y).abs() > 1 {
                if gdbus(
                    "Move",
                    &[&winid.to_string(), &x.to_string(), &y.to_string()],
                )
                .is_some()
                {
                    tracing::debug!(winid, x, y, "window_move: re-applied after override");
                }
            } else {
                break;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        // Position applied (or hold expired) — the popup may render. Focus
        // LAST, strictly after placement settled: see the doc above.
        PLACED.store(true, Ordering::Release);
        activate_until_deadline(winid, Duration::from_millis(1500));
    });
}

/// Frame-rect origin of a specific window id, via the extension.
fn popup_rect_at(winid: u32) -> Option<(i32, i32)> {
    let out = gdbus("GetFrameRect", &[&winid.to_string()])?;
    let x = out
        .split("\"x\":")
        .nth(1)?
        .split([',', '}'])
        .next()?
        .trim()
        .parse()
        .ok()?;
    let y = out
        .split("\"y\":")
        .nth(1)?
        .split([',', '}'])
        .next()?
        .trim()
        .parse()
        .ok()?;
    Some((x, y))
}
