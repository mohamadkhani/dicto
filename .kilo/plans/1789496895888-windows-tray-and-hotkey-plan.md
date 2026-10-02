# Windows Tray Icon + Global Hotkey

Add Windows tray icon and Windows global hotkey (Ctrl+Alt+D quick translate) with modular, readable structure mirroring the existing `hotkey/` backend pattern.

## Context

- GUI crate: `gpui` workspace member (package `dicto`); CLI/lib crate: root `mdict-rs`.
- Today: `gpui/src/tray.rs` is a Linux-only monolith (ksni/SNI). `gpui/src/hotkey/` has `mod.rs` (trait + `create_hotkey_manager()`), `fallback.rs`, `portal.rs` (Linux Wayland), `x11.rs` (Linux X11, wraps `global-hotkey` crate).
- Commit `8dfd049` already cfg-gated Linux-only code in `main.rs` so Windows compiles; Windows currently gets `FallbackHotkeyManager` (no hotkey) and no tray.
- `QuickTranslateEngine` (quick_translate.rs) owns the manager and **re-creates it at runtime** on settings changes (`reconfigure_hotkey`) — the Windows backend must survive repeated register/unregister cycles.
- Windows verification: `cargo xwin check --target x86_64-pc-windows-msvc -p dicto` (patch script already fixed). CI: `.github/workflows/pr.yml` builds both OSes on PRs.

## Decisions (agreed)

| # | Decision |
|---|---|
| D1 | Unify X11 + Windows hotkey backends into one `hotkey/global_hotkey.rs` (same `global-hotkey` crate); `parse_hotkey` moves to shared `hotkey/keys.rs` |
| D2 | Tray: plain facade `tray/mod.rs`, **no** trait (OS known at compile time; only hotkey keeps runtime detection) |
| D3 | `SharedToken` stays cross-platform; always `None` on Windows (Wayland-only concept) |
| D4 | Windows hotkey manager lives on a dedicated Win32 pump thread; create/register/unregister routed via command channel — engine thread-agnostic |
| D5 | One shared `win32.rs` helper (`run_message_loop` + thread spawn) used by both tray and hotkey on Windows |
| D6 | Tray menu identical on both OSes (Show Dictionary / Quick Translate / Quit); reuse `assets/tray-icon-128.raw` (RGBA) — no new assets or decode deps |
| D7 | Hotkey-taken errors surface as `HotkeyError` (logged); tray menu remains the trigger fallback — same as Linux portal failure path |

## Tasks (in order)

1. **`hotkey/keys.rs`** (new): move `ParsedHotkey`, `parse_hotkey`, `key_name_to_code` + their tests out of `x11.rs`. Export from `hotkey/mod.rs`.

2. **`hotkey/global_hotkey.rs`** (new, evolves `x11.rs`): rename struct to `GlobalHotkeyBackend`, constructor `new(backend_name: &'static str) -> Result<Self, HotkeyError>`. Keep the existing shape (Arc'd manager, events vec, background thread polling `GlobalHotKeyEvent::receiver()`, `try_recv`, `backend_name()`).
   - Linux path: current direct behavior unchanged.
   - Windows path: on `new()`, lazily create a **singleton pump thread** (via `win32.rs`) that owns one process-lifetime `GlobalHotKeyManager`; `register`/`unregister` are command-channel messages with reply channels so errors surface synchronously as `HotkeyError` (map `AlreadyRegistered` explicitly). Engine reconfigure = Unregister + Register commands; the manager itself is never dropped/recreated.

3. **`hotkey/mod.rs`**: cfg routing —
   ```rust
   #[cfg(target_os = "linux")] mod portal;
   #[cfg(any(target_os = "linux", target_os = "windows"))] mod global_hotkey;
   ```
   `create_hotkey_manager()`: Linux logic unchanged (Wayland → portal, X11 → `GlobalHotkeyBackend::new("x11")`); Windows → `GlobalHotkeyBackend::new("windows")`, on error warn + `FallbackHotkeyManager`. Keep function infallible.

4. **`gpui/Cargo.toml`**:
   - Move `global-hotkey = "0.5"` from `[target.'cfg(target_os = "linux")'.dependencies]` to common `[dependencies]`.
   - Add `[target.'cfg(target_os = "windows")'.dependencies]`: `tray-icon = "0.19"` (verify latest) and `windows` crate with minimal features (`Win32_Foundation`, `Win32_UI_WindowsAndMessaging`) — pin a version that unifies with the workspace's transitive `windows 0.58` (check with `cargo tree`).

5. **`win32.rs`** (new, `#[cfg(target_os = "windows")]` at module level): `run_message_loop()` (GetMessageW/TranslateMessage/DispatchMessageW) and `spawn_message_thread(name, setup)` running setup then the loop. Used by both Windows backends. Hidden windows belong to the crate internals; we only pump.

6. **`tray/` module** (replaces `tray.rs`; delete `tray.rs`):
   - `tray/mod.rs`: `TrayAction` enum, `SharedToken` type, doc comment; `spawn_tray() -> (mpsc::Receiver<TrayAction>, SharedToken)` routing by cfg to `ksni::spawn()` / `win::spawn()`.
   - `tray/icon.rs`: load `assets/tray-icon-128.raw`; `rgba()` (for tray-icon) and `#[cfg(target_os = "linux")] argb()` (rotate_right(1) for SNI); move the existing icon test here.
   - `tray/ksni.rs`: current `DictoTray` impl + spawn logic, unchanged except icon source.
   - `tray/win.rs`: dedicated thread creates `TrayIconBuilder` with icon + menu (identical three items) → actions over mpsc; then `win32` message loop. A second small poller thread forwards `tray_icon::menu::MenuEvent::receiver()` events to the action channel. SharedToken created, stays `None`.

7. **`main.rs`**: un-gate `mod tray;`, the `use crate::tray::...` import, the `spawn_tray()` block, `poll_tray_actions`, and `set_tray_translate_token` (keep `spawn_ipc_server` + `--translate` + `gtk::init` Linux-gated; unix-socket IPC stays Linux-only).

## Validation

1. `cargo check -p dicto` — Linux compiles, warnings not worse than the current 16.
2. `cargo xwin check --target x86_64-pc-windows-msvc -p dicto` — Windows compiles.
3. `cargo test -p dicto` — icon tests + `parse_hotkey` tests pass from new locations.
4. Commit + push → PR CI (`pr.yml`) green: `package-check`, `build-linux`, `build-windows`.
5. Manual smoke (Windows machine or user): tray icon visible, all three menu items work, Ctrl+Alt+D triggers the popup, settings hotkey change re-registers.

## Risks

- `windows` crate version must unify with gpui's transitive `0.58` or Cargo builds two copies (build-time cost only).
- Long-lived managers on the pump thread: verify unregister-then-register on reconfigure clears old bindings (test by changing the hotkey string in settings).
- tray-icon at 128×128: Windows scales down; if blurry in taskbar, add a 32×32 raw later (non-blocking).
- The gpui fork pin (`patch` section) is untouched; no cross-compile patch changes needed for this work.

## Out of scope

- macOS backends; Windows IPC (`--translate` via named pipes); hotkey rebinding UI; tray tooltip config.

## Docs (Gitea wiki, implementation phase — repo rules forbid local CONTEXT.md/ADRs)

- ADR: "Per-platform tray/hotkey backends behind compile-time facades (ksni+portal on Linux, tray-icon+global-hotkey on Windows)" — passes the ADR bar: real trade-off (tray-icon-everywhere rejected for the documented GNOME/Wayland appindicator failure), surprising to future readers, costly to reverse.
- Glossary terms: Backend, Facade, Activation Token, Trigger Path.
