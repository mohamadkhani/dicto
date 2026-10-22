//! OS-level shortcut auto-definition.
//!
//! Dicto's quick actions (Quick Translate, Word Lookup) are triggered from
//! anywhere in the desktop. Depending on the environment, different layers
//! own that trigger:
//!
//! - **XDG GlobalShortcuts portal** (`xdg-portal` backend): the desktop owns
//!   the binding — nothing to write, and any keybinding we previously created
//!   must be REMOVED or every press would fire twice (once via the portal,
//!   once via the IPC command).
//! - **Native backends** (`x11`, `windows`): the in-app registration owns
//!   the combo — same removal rule, same double-fire reasoning.
//! - **Tray-menu fallback** (portal unavailable): the app itself must define
//!   the OS shortcut. On **GNOME Wayland** that is a custom keybinding in
//!   `org.gnome.settings-daemon.plugins.media-keys`, written idempotently via
//!   the `gsettings` CLI: it runs `dicto --translate` / `dicto --lookup`,
//!   whose IPC clients wake the running instance (see `main.rs`). On other
//!   Wayland compositors there is no safe API to write config — we never
//!   touch user files and instead expose a copyable snippet.
//!
//! ## Safety rules
//!
//! - **Idempotent upsert**, keyed on ownership: a custom keybinding is ours
//!   iff its command's basename is `dicto` AND it ends with our flag
//!   (`--translate` / `--lookup`). Re-runs update in place; nothing else is
//!   ever touched.
//! - The command is the **absolute path of the running binary**, refreshed
//!   every sync — a moved/updated Dicto fixes its own stale bindings.
//! - Bindings exist only while the feature is enabled (or the toggle is on);
//!   disabling a feature removes its binding.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use mdict_rs::settings::{QuickTranslateSettings, WordLookupSettings};
use tracing::{info, warn};

use crate::hotkey::{QUICK_LOOKUP_ID, QUICK_TRANSLATE_ID, keys};

/// `org.gnome.settings-daemon.plugins.media-keys` — holds the list of
/// custom keybinding paths.
const MEDIA_KEYS_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
/// Relocatable schema for one custom keybinding entry; used with a
/// `schema:path` pair when reading/writing per-entry keys.
const CUSTOM_KEYBINDING_SCHEMA: &str =
    "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
/// Path prefix every custom keybinding entry lives under.
const CUSTOM_KEYBINDING_DIR: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";

/// Which layer currently owns the OS shortcut for our actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OsBindingMode {
    /// Auto setup is off — bindings we own have been removed.
    Off,
    /// The XDG portal owns the bindings; nothing for us to write.
    PortalManaged,
    /// The platform backend registered the combos natively (X11/Windows).
    NativeRegistration,
    /// GNOME custom keybindings were created/updated via gsettings.
    GnomeKeybindings,
    /// No automatic path exists on this desktop — the settings UI shows a
    /// copyable snippet instead.
    ManualRequired,
}

/// Latest sync result, for display in the settings UI.
#[derive(Debug, Clone)]
pub struct OsBindingStatus {
    pub mode: OsBindingMode,
    /// One line per action, e.g. `"Quick Translate: Ctrl+Alt+D (set)"`.
    pub actions: Vec<String>,
    /// Extra human-facing context (errors, environment notes).
    pub detail: String,
    /// Ready-to-paste compositor config for [`OsBindingMode::ManualRequired`].
    pub snippet: Option<String>,
}

/// One quick action's shortcut specification.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ActionSpec {
    /// Portal/logical id (`quick_translate` / `quick_lookup`).
    id: &'static str,
    /// Name written into the GNOME keybinding entry.
    label: &'static str,
    /// CLI flag the keybinding command runs (`--translate` / `--lookup`).
    flag: &'static str,
    hotkey: String,
    enabled: bool,
}

impl ActionSpec {
    fn key(&self) -> &'static str {
        match self.flag {
            "--translate" => "translate",
            "--lookup" => "lookup",
            _ => self.id,
        }
    }
}

fn specs_from_settings(qt: &QuickTranslateSettings, wl: &WordLookupSettings) -> Vec<ActionSpec> {
    vec![
        ActionSpec {
            id: QUICK_TRANSLATE_ID,
            label: "Dicto Quick Translate",
            flag: "--translate",
            hotkey: qt.hotkey.clone(),
            enabled: qt.enabled,
        },
        ActionSpec {
            id: QUICK_LOOKUP_ID,
            label: "Dicto Word Lookup",
            flag: "--lookup",
            hotkey: wl.hotkey.clone(),
            enabled: wl.enabled,
        },
    ]
}

static STATUS: OnceLock<Mutex<Option<OsBindingStatus>>> = OnceLock::new();

/// Latest known status, for the settings UI. `None` before the first sync.
pub fn status() -> Option<OsBindingStatus> {
    STATUS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .clone()
}

/// Reconcile the OS-level shortcut state with the settings. Fire-and-forget:
/// runs on a background thread (gsettings calls take tens of ms each and
/// this fires from the UI thread).
pub fn sync(qt: &QuickTranslateSettings, wl: &WordLookupSettings, auto: bool, backend: &str) {
    let specs = specs_from_settings(qt, wl);
    let backend = backend.to_string();
    let _ = std::thread::Builder::new()
        .name("os-binding".into())
        .spawn(move || {
            let status = sync_blocking(&specs, auto, &backend);
            info!(mode = ?status.mode, "os-binding: synced");
            *STATUS.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(status);
        });
}

/// Serialize concurrent syncs (settings edits can fire several threads at
/// once; interleaved read-plan-write cycles would corrupt the keybinding
/// list).
static SYNC_LOCK: Mutex<()> = Mutex::new(());

fn sync_blocking(specs: &[ActionSpec], auto: bool, backend: &str) -> OsBindingStatus {
    let _guard = SYNC_LOCK.lock().unwrap();

    let actions = |mode: &str| {
        specs
            .iter()
            .map(|s| format!("{}: {} ({})", label_for(s.id), s.hotkey, mode))
            .collect()
    };

    if !auto {
        remove_all_ours(specs);
        return OsBindingStatus {
            mode: OsBindingMode::Off,
            actions: Vec::new(),
            detail: "Automatic system shortcuts are off.".into(),
            snippet: None,
        };
    }

    match backend {
        // Portal / native backends own the combo — our gsettings entries
        // would double-fire. Remove them for a clean handover.
        "xdg-portal" => {
            remove_all_ours(specs);
            OsBindingStatus {
                mode: OsBindingMode::PortalManaged,
                actions: actions("via desktop portal"),
                detail: "Shortcuts are managed through the desktop's global \
                         shortcuts portal."
                    .into(),
                snippet: None,
            }
        }
        "x11" | "windows" => {
            remove_all_ours(specs);
            OsBindingStatus {
                mode: OsBindingMode::NativeRegistration,
                actions: actions("registered"),
                detail: "Shortcuts are registered directly with the window \
                         system."
                    .into(),
                snippet: None,
            }
        }
        // Tray-menu fallback: no in-app global hotkey — we must define the
        // OS shortcut ourselves where a supported API exists.
        _ => {
            if is_gnome_wayland() {
                match gnome_sync(specs) {
                    Ok(failed) => {
                        let mut detail = String::from(
                            "Shortcuts were created in GNOME Settings → \
                             Keyboard → Custom Shortcuts.",
                        );
                        if !failed.is_empty() {
                            detail.push_str(&format!(" Failed: {}.", failed.join("; ")));
                        }
                        OsBindingStatus {
                            mode: OsBindingMode::GnomeKeybindings,
                            actions: actions("set in GNOME"),
                            detail,
                            snippet: None,
                        }
                    }
                    Err(e) => OsBindingStatus {
                        mode: OsBindingMode::ManualRequired,
                        actions: actions("not set"),
                        detail: format!(
                            "Could not create the GNOME shortcuts ({e}). \
                             Set them up manually:"
                        ),
                        snippet: Some(gnome_manual_instructions(specs)),
                    },
                }
            } else {
                remove_all_ours(specs);
                OsBindingStatus {
                    mode: OsBindingMode::ManualRequired,
                    actions: actions("not set"),
                    detail: format!(
                        "This desktop ({}) exposes no shortcut API Dicto can \
                         use. Add the bindings below to your compositor \
                         config:",
                        desktop_name(),
                    ),
                    snippet: Some(compositor_snippet(specs)),
                }
            }
        }
    }
}

fn label_for(id: &str) -> &'static str {
    match id {
        QUICK_TRANSLATE_ID => "Quick Translate",
        QUICK_LOOKUP_ID => "Word Lookup",
        _ => "Action",
    }
}

/// True on GNOME running a Wayland session — the environment where the
/// portal is typically unavailable and custom keybindings are the fix.
fn is_gnome_wayland() -> bool {
    let wayland = std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|s| s == "wayland")
            .unwrap_or(false);
    let gnome = std::env::var("XDG_CURRENT_DESKTOP")
        .map(|d| d.to_ascii_lowercase().contains("gnome"))
        .unwrap_or(false);
    wayland && gnome
}

fn desktop_name() -> String {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into())
}

/// Remove every keybinding we own (all actions), best effort.
fn remove_all_ours(specs: &[ActionSpec]) {
    let entries = match read_gnome_entries() {
        Some(e) => e,
        None => return, // no gsettings / no GNOME — nothing to clean
    };
    let removed: Vec<String> = entries
        .iter()
        .filter(|e| entry_owner(e, specs).is_some())
        .map(|e| e.path.clone())
        .collect();
    if removed.is_empty() {
        return;
    }
    if write_list(&entries, &removed, &[]).is_ok() {
        info!(count = removed.len(), "os-binding: removed our keybindings");
    }
}

// --- GNOME custom keybinding machinery ---

/// One entry from the custom-keybindings list.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GnomeEntry {
    path: String,
    name: String,
    command: String,
    binding: String,
}

/// Run `gsettings …` and return trimmed stdout, or `None` on any failure.
fn gsettings(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("gsettings")
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        warn!(
            args = args.join(" "),
            stderr = %String::from_utf8_lossy(&output.stderr).trim(),
            "os-binding: gsettings failed"
        );
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Read every custom keybinding entry currently configured.
fn read_gnome_entries() -> Option<Vec<GnomeEntry>> {
    let raw = gsettings(&["get", MEDIA_KEYS_SCHEMA, "custom-keybindings"])?;
    let paths = parse_string_list(&raw);
    let entries = paths
        .iter()
        .filter_map(|path| {
            let schema_path = format!("{CUSTOM_KEYBINDING_SCHEMA}:{path}");
            let name = gsettings(&["get", &schema_path, "name"])?;
            let command = gsettings(&["get", &schema_path, "command"])?;
            let binding = gsettings(&["get", &schema_path, "binding"])?;
            Some(GnomeEntry {
                path: path.clone(),
                name: parse_string(&name),
                command: parse_string(&command),
                binding: parse_string(&binding),
            })
        })
        .collect();
    Some(entries)
}

/// Which action an entry belongs to — `Some(spec)` iff this is OUR
/// keybinding.
///
/// Primary rule ([`owned_action`]): a dicto command with one of our flags.
/// Fallback: our exact label on an entry with an empty or dicto-ish command —
/// that pattern only arises from partially-written entries of our own (an
/// older build failing mid-write) and must be repaired, not left behind.
/// Anything else is never touched.
fn entry_owner<'s>(entry: &GnomeEntry, specs: &'s [ActionSpec]) -> Option<&'s ActionSpec> {
    if let Some(spec) = owned_action(&entry.command, specs) {
        return Some(spec);
    }
    let exe = entry.command.split_whitespace().next();
    let dicto_ish = entry.command.is_empty()
        || exe.is_some_and(|exe| {
            std::path::Path::new(&unquote_token(exe))
                .file_name()
                .is_some_and(|f| f.to_string_lossy().starts_with("dicto"))
        });
    if dicto_ish {
        specs.iter().find(|s| s.label == entry.name)
    } else {
        None
    }
}

/// Which action a *command* belongs to — `Some(spec)` iff the command is
/// exactly two tokens whose exe's basename is `dicto*` (matching dev/test
/// binaries like `dicto-ab12cd`) and whose flag is one of ours. The exe
/// token may carry shell quotes — the stored command quotes paths with
/// spaces.
fn owned_action<'s>(command: &str, specs: &'s [ActionSpec]) -> Option<&'s ActionSpec> {
    let mut parts = command.split_whitespace();
    let exe = unquote_token(parts.next()?);
    let flag = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let exe_basename = std::path::Path::new(&exe)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| exe.clone());
    if !exe_basename.starts_with("dicto") {
        return None;
    }
    specs.iter().find(|s| s.flag == flag)
}

/// Strip surrounding single quotes and unescape `\'` from one shell token.
fn unquote_token(token: &str) -> String {
    let inner = token
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .unwrap_or(token);
    inner.replace(r"\'", "'")
}

/// Absolute path of the running binary + our flag — what the keybinding runs.
fn dicto_command(flag: &str) -> Option<String> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    Some(format!("{} {}", shell_quote(&exe.to_string_lossy()), flag))
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// One planned mutation of the custom-keybindings list.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GnomeOp {
    /// Create a new entry at `path` with these fields.
    Add {
        path: String,
        name: String,
        command: String,
        binding: String,
    },
    /// Update an existing (ours) entry in place.
    Update {
        path: String,
        name: String,
        command: String,
        binding: String,
    },
    /// Drop an entry from the list.
    Remove { path: String },
}

/// Pure reconciliation: diff the desired specs against the current entries.
/// Unit-tested without touching gsettings.
fn plan_gnome(
    specs: &[ActionSpec],
    entries: &[GnomeEntry],
    commands: &HashMap<&str, String>,
) -> Vec<GnomeOp> {
    let mut ops = Vec::new();
    let mut used_paths: Vec<String> = Vec::new();

    for spec in specs {
        let desired_command = match commands.get(spec.key()) {
            Some(c) => c.clone(),
            None => continue, // couldn't resolve our own binary — skip
        };
        let desired_binding = match keys::to_gtk_accel(&spec.hotkey) {
            Ok(b) => b,
            Err(e) => {
                warn!(id = spec.id, error = %e, "os-binding: bad hotkey, skipping");
                continue;
            }
        };

        // Every entry owned by this action — the first is canonical, the
        // rest are duplicates from earlier runs and get removed.
        let owned: Vec<&GnomeEntry> = entries
            .iter()
            .filter(|e| entry_owner(e, specs) == Some(spec))
            .collect();

        match owned.first() {
            Some(entry) => {
                used_paths.push(entry.path.clone());
                if spec.enabled {
                    if entry.command != desired_command || entry.binding != desired_binding {
                        ops.push(GnomeOp::Update {
                            path: entry.path.clone(),
                            name: spec.label.to_string(),
                            command: desired_command,
                            binding: desired_binding,
                        });
                    }
                } else {
                    ops.push(GnomeOp::Remove {
                        path: entry.path.clone(),
                    });
                }
                for dup in owned.iter().skip(1) {
                    ops.push(GnomeOp::Remove {
                        path: dup.path.clone(),
                    });
                }
            }
            None => {
                if spec.enabled {
                    let path = free_path(entries, &used_paths);
                    used_paths.push(path.clone());
                    ops.push(GnomeOp::Add {
                        path,
                        name: spec.label.to_string(),
                        command: desired_command,
                        binding: desired_binding,
                    });
                }
            }
        }
    }

    ops
}

/// Next free `customN` path, avoiding both current and to-be-added paths.
fn free_path(entries: &[GnomeEntry], extra_used: &[String]) -> String {
    let taken: Vec<&str> = entries
        .iter()
        .map(|e| e.path.as_str())
        .chain(extra_used.iter().map(|s| s.as_str()))
        .collect();
    let mut n = 0;
    loop {
        let candidate = format!("{CUSTOM_KEYBINDING_DIR}custom{n}/");
        if !taken.contains(&candidate.as_str()) {
            return candidate;
        }
        n += 1;
    }
}

/// Read current state, plan, and apply. Returns `Err(message)` when nothing
/// could be done (gsettings missing/failing); `Ok(failed_ops)` when the sync
/// went through with some per-op failures.
fn gnome_sync(specs: &[ActionSpec]) -> Result<Vec<String>, String> {
    let entries = read_gnome_entries().ok_or_else(|| {
        "gsettings is unavailable — is this a GNOME session with \
         gsettings installed?"
            .to_string()
    })?;

    let mut commands: HashMap<&str, String> = HashMap::new();
    for spec in specs {
        if let Some(cmd) = dicto_command(spec.flag) {
            commands.insert(spec.key(), cmd);
        }
    }

    let ops = plan_gnome(specs, &entries, &commands);
    if ops.is_empty() {
        return Ok(Vec::new());
    }

    let mut failed: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    let mut added: Vec<String> = Vec::new();

    for op in ops {
        match op {
            GnomeOp::Remove { path } => removed.push(path),
            GnomeOp::Add {
                path,
                name,
                command,
                binding,
            } => match apply_entry(&path, &name, &command, &binding) {
                // Only list entries whose fields were fully written — a
                // half-written entry would show up with a name and nothing
                // else.
                Ok(()) => added.push(path),
                Err(e) => failed.push(format!("{path}: {e}")),
            },
            GnomeOp::Update {
                path,
                name,
                command,
                binding,
            } => {
                if let Err(e) = apply_entry(&path, &name, &command, &binding) {
                    failed.push(format!("{path}: {e}"));
                }
            }
        }
    }

    // Rewrite the list ONCE: current minus removed plus added.
    if !(removed.is_empty() && added.is_empty()) && write_list(&entries, &removed, &added).is_err()
    {
        return Err("could not update the custom keybindings list".into());
    }

    // Best-effort cleanup of removed entries' orphaned dconf subpaths so
    // they don't linger in dconf-editor forever.
    for path in &removed {
        let _ = std::process::Command::new("dconf")
            .args(["reset", "-f", path])
            .output();
    }

    Ok(failed)
}

/// Write the three fields of one custom-keybinding entry.
fn apply_entry(path: &str, name: &str, command: &str, binding: &str) -> Result<(), String> {
    let schema_path = format!("{CUSTOM_KEYBINDING_SCHEMA}:{path}");
    for (key, value) in [("name", name), ("command", command), ("binding", binding)] {
        gsettings(&["set", &schema_path, key, &gvariant_string(value)])
            .ok_or_else(|| format!("could not set {key} for {path}"))?;
    }
    Ok(())
}

/// Encode a Rust string as a GVariant text-format string literal.
///
/// NOTE: this is NOT shell quoting — `gsettings` parses the value with the
/// GVariant grammar, where the only escape inside a quoted string is a
/// backslash (`\'`, `\\"`, `\\`). The shell-style `'\''` trick produces a
/// parse error and silently breaks the write.
fn gvariant_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Write the custom-keybindings list: current entries minus `removed` plus
/// `added`, preserving order.
fn write_list(current: &[GnomeEntry], removed: &[String], added: &[String]) -> Result<(), String> {
    let mut paths: Vec<String> = current
        .iter()
        .map(|e| e.path.clone())
        .filter(|p| !removed.contains(p))
        .collect();
    for path in added {
        if !paths.contains(path) {
            paths.push(path.clone());
        }
    }
    let value = if paths.is_empty() {
        "@as []".to_string()
    } else {
        let items: Vec<String> = paths.iter().map(|p| format!("'{p}'")).collect();
        format!("[{}]", items.join(", "))
    };
    gsettings(&["set", MEDIA_KEYS_SCHEMA, "custom-keybindings", &value])
        .ok_or_else(|| "gsettings set custom-keybindings failed".to_string())?;
    Ok(())
}

// --- Value parsing (gsettings GVariant text format) ---

/// Parse a GVariant string-array dump: `@as []`, `[]`, or `['/a/', '/b/']`.
fn parse_string_list(raw: &str) -> Vec<String> {
    let body = raw.trim().strip_prefix("@as").unwrap_or(raw.trim()).trim();
    let inner = body
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(body);
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .map(parse_string)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse a GVariant string literal — single- or double-quoted (gsettings
/// echoes values containing quotes with double quotes) — and unescape it.
fn parse_string(raw: &str) -> String {
    let s = raw.trim();
    let inner = if s.len() >= 2
        && ((s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')))
    {
        &s[1..s.len() - 1]
    } else {
        s
    };
    unescape_gvariant(inner)
}

/// Undo GVariant string escapes (`\'`, `\"`, `\\`, `\n`, …).
fn unescape_gvariant(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\'') => out.push('\''),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

// --- Snippets for desktops without an API ---

/// Sway-style binding lines (works for sway and most wlroots compositors).
fn compositor_snippet(specs: &[ActionSpec]) -> String {
    let mut lines = Vec::new();
    for spec in specs.iter().filter(|s| s.enabled) {
        if let (Some(cmd), Ok(binding)) =
            (dicto_command(spec.flag), keys::to_gtk_accel(&spec.hotkey))
        {
            let accel = binding
                .replace("<Control>", "Ctrl+")
                .replace("<Alt>", "Alt+")
                .replace("<Shift>", "Shift+")
                .replace("<Super>", "Super+");
            if std::env::var("XDG_CURRENT_DESKTOP")
                .map(|d| d.to_ascii_lowercase().contains("hyprland"))
                .unwrap_or(false)
            {
                // Hyprland: bind = MODS, key, dispatcher, args
                let parts: Vec<&str> = accel.split('+').collect();
                let (mods, key) = parts.split_at(parts.len() - 1);
                lines.push(format!(
                    "bind = {}, {}, exec, {}",
                    mods.join(" ").to_uppercase(),
                    key[0].to_uppercase(),
                    cmd
                ));
            } else {
                lines.push(format!("bindsym {accel} exec {cmd}"));
            }
        }
    }
    if lines.is_empty() {
        lines.push("# enable Quick Translate or Word Lookup first".into());
    }
    lines.join("\n")
}

/// Manual GNOME instructions when gsettings itself is unusable.
fn gnome_manual_instructions(specs: &[ActionSpec]) -> String {
    let mut lines = Vec::new();
    for spec in specs.iter().filter(|s| s.enabled) {
        if let (Some(cmd), Ok(binding)) =
            (dicto_command(spec.flag), keys::to_gtk_accel(&spec.hotkey))
        {
            lines.push(format!("{} — {} runs {}", label_for(spec.id), binding, cmd));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs() -> Vec<ActionSpec> {
        vec![
            ActionSpec {
                id: QUICK_TRANSLATE_ID,
                label: "Dicto Quick Translate",
                flag: "--translate",
                hotkey: "Ctrl+Alt+D".into(),
                enabled: true,
            },
            ActionSpec {
                id: QUICK_LOOKUP_ID,
                label: "Dicto Word Lookup",
                flag: "--lookup",
                hotkey: "Ctrl+Alt+W".into(),
                enabled: true,
            },
        ]
    }

    fn commands() -> HashMap<&'static str, String> {
        HashMap::from([
            ("translate", "'/usr/bin/dicto' --translate".to_string()),
            ("lookup", "'/usr/bin/dicto' --lookup".to_string()),
        ])
    }

    #[test]
    fn parses_string_list_variants() {
        assert_eq!(parse_string_list("@as []"), Vec::<String>::new());
        assert_eq!(parse_string_list("[]"), Vec::<String>::new());
        assert_eq!(
            parse_string_list("['/a/custom0/', '/a/custom1/']"),
            vec!["/a/custom0/", "/a/custom1/"]
        );
    }

    #[test]
    fn parses_string_literals() {
        assert_eq!(
            parse_string("'/usr/bin/dicto --translate'"),
            "/usr/bin/dicto --translate"
        );
        assert_eq!(parse_string("'it\\'s'"), "it's");
    }

    #[test]
    fn ownership_matches_only_dicto_with_our_flags() {
        let s = specs();
        assert!(owned_action("'/usr/bin/dicto' --translate", &s).is_some());
        assert!(owned_action("/home/u/bin/dicto --lookup", &s).is_some());
        // Dev/test binaries are hash-suffixed; basename prefix still ours.
        assert!(owned_action("/t/deps/dicto-bc38db --translate", &s).is_some());
        assert!(owned_action("dicto", &s).is_none());
        assert!(owned_action("/usr/bin/dicto --translate extra", &s).is_none());
        assert!(owned_action("/usr/bin/other --translate", &s).is_none());
        assert!(owned_action("/usr/bin/dicto --unknown", &s).is_none());
        assert!(owned_action("firefox https://x", &s).is_none());
    }

    #[test]
    fn plan_adds_missing_entries() {
        let ops = plan_gnome(&specs(), &[], &commands());
        assert_eq!(ops.len(), 2);
        assert_eq!(
            ops[0],
            GnomeOp::Add {
                path: format!("{CUSTOM_KEYBINDING_DIR}custom0/"),
                name: "Dicto Quick Translate".into(),
                command: "'/usr/bin/dicto' --translate".into(),
                binding: "<Control><Alt>d".into(),
            }
        );
        assert_eq!(
            ops[1],
            GnomeOp::Add {
                path: format!("{CUSTOM_KEYBINDING_DIR}custom1/"),
                name: "Dicto Word Lookup".into(),
                command: "'/usr/bin/dicto' --lookup".into(),
                binding: "<Control><Alt>w".into(),
            }
        );
    }

    #[test]
    fn plan_is_idempotent() {
        let entries = vec![
            GnomeEntry {
                path: format!("{CUSTOM_KEYBINDING_DIR}custom0/"),
                name: "Dicto Quick Translate".into(),
                command: "'/usr/bin/dicto' --translate".into(),
                binding: "<Control><Alt>d".into(),
            },
            GnomeEntry {
                path: format!("{CUSTOM_KEYBINDING_DIR}custom1/"),
                name: "Dicto Word Lookup".into(),
                command: "'/usr/bin/dicto' --lookup".into(),
                binding: "<Control><Alt>w".into(),
            },
        ];
        assert!(plan_gnome(&specs(), &entries, &commands()).is_empty());
    }

    #[test]
    fn plan_updates_stale_binding_in_place() {
        let entries = vec![GnomeEntry {
            path: format!("{CUSTOM_KEYBINDING_DIR}custom3/"),
            name: "Dicto Quick Translate".into(),
            command: "'/old/path/dicto' --translate".into(),
            binding: "<Control><Alt>d".into(),
        }];
        let ops = plan_gnome(&specs(), &entries, &commands());
        assert_eq!(ops.len(), 2); // update translate + add lookup
        assert!(ops.contains(&GnomeOp::Update {
            path: format!("{CUSTOM_KEYBINDING_DIR}custom3/"),
            name: "Dicto Quick Translate".into(),
            command: "'/usr/bin/dicto' --translate".into(),
            binding: "<Control><Alt>d".into(),
        }));
        // The new lookup entry avoids custom3.
        assert!(matches!(&ops[1], GnomeOp::Add { path, .. } if path.ends_with("custom0/")));
    }

    #[test]
    fn plan_removes_disabled_and_foreign_owned() {
        let mut s = specs();
        s[1].enabled = false;
        let entries = vec![
            GnomeEntry {
                path: format!("{CUSTOM_KEYBINDING_DIR}custom0/"),
                name: "Dicto Quick Translate".into(),
                command: "'/usr/bin/dicto' --translate".into(),
                binding: "<Control><Alt>d".into(),
            },
            GnomeEntry {
                path: format!("{CUSTOM_KEYBINDING_DIR}custom5/"),
                name: "Dicto Word Lookup".into(),
                command: "'/usr/bin/dicto' --lookup".into(),
                binding: "<Control><Alt>w".into(),
            },
        ];
        let ops = plan_gnome(&s, &entries, &commands());
        assert_eq!(
            ops,
            vec![GnomeOp::Remove {
                path: format!("{CUSTOM_KEYBINDING_DIR}custom5/"),
            }]
        );
    }

    #[test]
    fn plan_never_touches_foreign_entries() {
        let entries = vec![GnomeEntry {
            path: format!("{CUSTOM_KEYBINDING_DIR}custom0/"),
            name: "Screenshot".into(),
            command: "'/usr/bin/screenshot-tool' --now".into(),
            binding: "<Super>p".into(),
        }];
        let ops = plan_gnome(&specs(), &entries, &commands());
        assert!(ops.iter().all(|op| match op {
            GnomeOp::Add { path, .. } => path != &entries[0].path,
            _ => true,
        }));
    }

    #[test]
    fn free_path_skips_taken() {
        let entries = vec![GnomeEntry {
            path: format!("{CUSTOM_KEYBINDING_DIR}custom0/"),
            name: "Dicto Quick Translate".into(),
            command: "'/usr/bin/dicto' --translate".into(),
            binding: "<Control><Alt>d".into(),
        }];
        assert_eq!(
            free_path(&entries, &[format!("{CUSTOM_KEYBINDING_DIR}custom1/")]),
            format!("{CUSTOM_KEYBINDING_DIR}custom2/")
        );
    }

    #[test]
    fn gvariant_escaping_round_trips() {
        // Quotes inside the command are the exact case that broke writes.
        let command = "'/usr/bin/dicto' --translate";
        let encoded = gvariant_string(command);
        assert_eq!(encoded, r"'\'/usr/bin/dicto\' --translate'");
        assert_eq!(parse_string(&encoded), command);
        assert_eq!(parse_string(&gvariant_string("it's")), "it's");
        assert_eq!(parse_string(&gvariant_string(r"a\b")), r"a\b");
        assert_eq!(parse_string(&gvariant_string("plain")), "plain");
    }

    #[test]
    fn parse_handles_double_quoted_echo() {
        // gsettings echoes values containing quotes with double quotes.
        assert_eq!(
            parse_string(r#""'/usr/bin/dicto' --translate""#),
            "'/usr/bin/dicto' --translate"
        );
    }

    #[test]
    fn plan_repairs_half_written_entries() {
        // Entry left by a failed write: our label, empty command/binding.
        // It must be recognized as ours and UPDATED in place, not duplicated.
        let entries = vec![GnomeEntry {
            path: format!("{CUSTOM_KEYBINDING_DIR}custom0/"),
            name: "Dicto Quick Translate".into(),
            command: String::new(),
            binding: String::new(),
        }];
        let ops = plan_gnome(&specs(), &entries, &commands());
        assert_eq!(ops.len(), 2);
        assert!(ops.contains(&GnomeOp::Update {
            path: format!("{CUSTOM_KEYBINDING_DIR}custom0/"),
            name: "Dicto Quick Translate".into(),
            command: "'/usr/bin/dicto' --translate".into(),
            binding: "<Control><Alt>d".into(),
        }));
        // The lookup Add must reuse custom0? No — custom0 is taken; new path.
        assert!(matches!(&ops[1], GnomeOp::Add { path, .. } if path.ends_with("custom1/")));
    }

    #[test]
    fn plan_ignores_foreign_entries_even_with_shared_label_words() {
        let entries = vec![GnomeEntry {
            path: format!("{CUSTOM_KEYBINDING_DIR}custom0/"),
            name: "Dicto Quick Translate clone".into(),
            command: "'/usr/bin/other-tool' --translate".into(),
            binding: "<Control><Alt>d".into(),
        }];
        let ops = plan_gnome(&specs(), &entries, &commands());
        // Only Adds (custom0 is foreign — a fresh entry is created elsewhere)
        // and NEVER an Update/Remove touching the foreign path.
        assert!(ops.iter().all(
            |op| !matches!(op, GnomeOp::Update { path, .. } | GnomeOp::Remove { path }
                if *path == entries[0].path)
        ));
    }

    #[test]
    fn snippet_formats_for_sway() {
        let text = compositor_snippet(&specs());
        // sway bindsym syntax: modifiers + key, then exec.
        assert!(text.contains("bindsym Ctrl+Alt+d exec"), "{text}");
        assert!(text.contains("--translate"), "{text}");
        assert!(text.contains("--lookup"), "{text}");
    }
}
