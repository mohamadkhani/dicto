# Quick Translate popup

The Quick Translate feature turns any selected text into a translation without
leaving Dicto: press a global hotkey (or click the tray menu), Dicto reads the
selection, and a small floating popup shows the original text plus Translate /
Speak buttons. This document is the spec for that popup — the trigger flow, the
popup states, the independent playback slots, the settings, and the
platform-specific backends.

> **TL;DR for the bug-prone parts:**
> - The popup only reads the selection on trigger; translation runs **on explicit
>   Translate click**, not on hotkey.
> - The **Original** and **Translation** rows each own an **independent**
>   `PlaybackController` (`playback_source` / `playback_translation` on
>   `DictState`). Their play/pause, replay, and seek never share state.
> - Every interactive popup element carries a **slot-suffixed DOM id**
>   (`qt-play-pause-src` / `-tr`, etc.) so GPUI click dispatch routes to the
>   intended button.
> - The seek bar maps window-relative click coords to element-relative fraction
>   via a bounds-recording `canvas` overlay — `ClickEvent::position()` is
>   window-relative, **not** element-relative.

---

## Trigger flow

```
global hotkey  ─┐
                ├─▶ poll() ─▶ trigger_translate() ─▶ read_selected_text()
tray menu      ─┘                                       │
                                                       ▼
                                          PopupStatus::Visible(Idle { original })
                                                       │
                                          (user clicks Translate)
                                                       │
                                                       ▼
                                    start_translation() ─▶ Loading
                                                       │ (background executor)
                                                       ▼
                                          apply_translation_result()
                                                       │
                                          ┌────────────┴────────────┐
                                          ▼                         ▼
                                       Ready                     Error
```

1. **Trigger.** The global hotkey manager (or the tray menu flag) surfaces an
   event. `DictApp`'s 100 ms poll loop ([gpui/src/app.rs](../gpui/src/app.rs))
   calls `QuickTranslateEngine::poll()`, which on a `QUICK_TRANSLATE_ID` event
   calls [`trigger_translate`](#trigger_translate).
2. **Read selection.** [`read_selected_text`](#selection-reading) reads the
   primary selection, falling back to the clipboard.
3. **Show Idle popup.** The popup opens in `PopupState::Idle` — the original
   text is shown with **Translate** and **Speak (Original)** buttons. **No
   translation runs yet.**
4. **Translate.** Clicking Translate calls `start_translation()`, which moves to
   `Loading` and returns a `TranslationJob`. The job's blocking HTTP request
   runs on GPUI's background executor; the result feeds back through
   `apply_translation_result()` → `Ready` (or `Error`).
5. **Dismiss.** Escape or a click outside the card closes the popup
   ([`close_popup`](#window-lifecycle)).

### `trigger_translate`

`gpui/src/quick_translate.rs:148`. Reads the selection and shows it as `Idle`.
Emits a `QuickTranslateTriggered { source: Hotkey }` telemetry event. On
selection-read failure it shows an `Error` popup with a copy-it-first hint.
Returns `true` if a popup is now visible.

### `start_translation`

`gpui/src/quick_translate.rs:194`. Only valid from `Idle`. Moves to `Loading`
and returns a `TranslationJob` (provider display name, target lang, request).
The caller spawns `job.run()` on the background executor — translation never
blocks the UI thread.

---

## Popup states

The engine wraps the popup state in `PopupStatus`
([gpui/src/quick_translate.rs:23](../gpui/src/quick_translate.rs#L23)): `Hidden`
or `Visible(PopupState)`. The window is open while the status is `Visible`.

`PopupState` ([gpui/src/components/translate_popup.rs:17](../gpui/src/components/translate_popup.rs#L17)):

| Variant | Meaning |
|---|---|
| `Idle { original }` | Selection captured; waiting for Translate click. Shows Original + Speak + Translate + Options toggle. |
| `Loading { original }` | Translation in flight. Shows Original + Speak + a "Translating…" spinner. |
| `Ready { original, translation, provider, model }` | Success. Shows Original (Speak) + Translation (Speak) + `via {provider} · {model}` + Translate (re-run) + Options. |
| `Error { original, error }` | Translation or selection-read failed. Shows the error + Translate (retry). |

The popup is wrapped in a bounded scroll container (`max_h 560px`) so long text
or an expanded Options panel scrolls instead of overflowing.

### Inline Options panel

A collapsible "▸ Options" toggle at the bottom of every state expands an inline
panel with compact selectors for **Provider**, **Model**, **Target language**,
and **TTS**. Changing a selector persists immediately (`DictState::save_settings`)
and, where relevant, rebuilds the translator via `reload_translator`.

---

## Two independent playback slots

The Original and the Translation each have their own playback state. They never
mix. `DictState` holds two controllers:

```rust
// gpui/src/state.rs:102
pub playback_source: PlaybackController,       // the Original row
pub playback_translation: PlaybackController,  // the Translation row
```

A small enum routes every action to the right controller:

```rust
// gpui/src/components/translate_popup.rs
#[derive(Clone, Copy)]
enum Slot { Source, Translation }

impl Slot {
    fn controller<'a>(self, s: &'a DictState) -> &'a PlaybackController { ... }
    fn controller_mut<'a>(self, s: &'a mut DictState) -> &'a mut PlaybackController { ... }
}
```

`Slot` is passed **explicitly** into `speak_button(...)` at each call site
(`Slot::Source` for the Original header, `Slot::Translation` for the Translation
row). It is **not** inferred from the button's `lang` string — a translation can
legitimately have an empty target lang, so inferring would misroute the
translation's controls to the source controller.

### Why two controllers

A single shared controller meant speaking the original then the translation
overwrote the shared sink/state: pausing one affected the other, and seek bars
bled across rows. Two controllers + explicit slot routing fixed it.

### Why slot-suffixed element IDs

GPUI interactive elements dispatch clicks keyed by their `.id()`. When both
slots' play/pause buttons shared `"qt-play-pause"`, the translation's click was
claimed by the source's hitbox and the button appeared dead. Every interactive
popup element now carries a slot suffix:

| Element | IDs |
|---|---|
| Speak button | `qt-speak-src` / `qt-speak-tr` |
| Play/Pause | `qt-play-pause-src` / `qt-play-pause-tr` |
| Replay | `qt-replay-src` / `qt-replay-tr` |
| Seek bar | `qt-seek-bar-src` / `qt-seek-bar-tr` |

### Seek bar: window-relative → element-relative

`ClickEvent::position()` returns the **window-relative** mouse position, not the
position within the clicked element. Clicking the seek bar therefore gave an X
of e.g. ~600px (from the window's left edge); `600 / 120` clamped to `1.0` and
seeked to the clip's end — the bar "filled" and playback stopped.

Fix: the seek bar overlays an invisible full-size `canvas` that records its own
painted bounds every frame into a shared `Rc<Cell<Option<Bounds<Pixels>>>>`. The
click handler subtracts the bar's left edge and divides by its real width:

```rust
let rel = (ev.position().x - b.left()).max(px(0.));
let fraction = (rel / b.size.width).clamp(0.0, 1.0);
slot.controller_mut(s).seek(fraction);
```

---

## Playback controller

`gpui/src/playback.rs`. Owns a persistent rodio stream + the current clip's
`Sink` so clips can be paused, seeked, and replayed without re-synthesizing.

### `PlaybackState`

```rust
pub enum PlaybackState {
    Idle,                 // nothing loaded
    Loading,              // synthesizing / decoding (Speak button shows spinner)
    Playing { pos: f32 }, // pos = seconds
    Paused { pos: f32 },
    Ended { pos: f32 },   // finished; clip still loaded → replay/seek available
    Error(String),
}
```

### Public API

| Method | Purpose |
|---|---|
| `start_load(text, lang, tts) -> LoadAction` | Begin loading. Returns `Replay` (same clip already loaded — replayed synchronously) or `Synthesize { text, lang, tts }` (caller spawns synthesis, then calls `install_from_bytes`). |
| `install_from_bytes(text, bytes)` | Decode + install the synthesized audio; transitions `Loading → Playing`. |
| `fail(error)` | Transition to `Error` from `Loading`. |
| `toggle_pause()` | Play ⇄ Pause. |
| `replay()` | Seek to 0 + play (re-decodes from kept `bytes` if the source was consumed). |
| `seek(fraction)` | Seek to `fraction` (0.0..=1.0) of total duration; restart-from-bytes if exhausted. |
| `poll_progress()` | Refresh live position; transitions `Playing → Ended` when the clip finishes. Called ~10 Hz by the popup. |
| `snapshot() -> (PlaybackState, Option<Duration>)` | Read the current state + total duration for rendering. |
| `stop()` | Stop and clear the sink (used on popup close). |

`LoadAction` exists so the controller stays GPUI-context-free: it decides
*whether* to synthesize, and the view (which has the executor) spawns the actual
task.

### Live-position polling

`TranslatePopupView::new` spawns a 100 ms loop that calls `poll_progress()` on
**both** controllers and notifies the view, so both seek bars track live and
`Playing → Ended` is observed without the user interacting.

---

## Window lifecycle

The popup is its own GPUI window holding a `TranslatePopupView`
([gpui/src/components/translate_popup.rs:874](../gpui/src/components/translate_popup.rs#L874)).

- **Open:** `open_translate_popup`
  ([gpui/src/app.rs:362](../gpui/src/app.rs#L362)) — if a window already exists,
  it activates it; otherwise it opens a centered 460×560 window
  (`WindowKind::PopUp`, `app_id: "dicto"`, `is_resizable`, client decorations)
  and stores the `WindowHandle` in `DictState::qt_popup_window`. Called when
  `trigger_translate` returns `true`.
- **Focus:** the view focuses itself on mount (`FocusHandle`), so it receives
  key events.
- **Dismiss:**
  - **Escape** — `on_key_down` checks `ev.keystroke.key == "escape"`.
  - **Click-away** — `on_mouse_down(Left)` on the root closes; the card itself
    calls `cx.stop_propagation()` so clicks on the card don't dismiss.
- **`close_popup`** ([line 977](../gpui/src/components/translate_popup.rs#L977)):
  calls `engine.hide_popup()` (→ `PopupStatus::Hidden`), clears
  `qt_popup_window`, and `window.remove_window()`.
- When `PopupStatus` is `Hidden`, render yields an empty root and the window is
  removed by the trigger logic.

---

## Trigger backends

### Global hotkey

`gpui/src/hotkey/`. `QUICK_TRANSLATE_ID = "quick_translate"`. The
`HotkeyManager` trait (`try_recv() -> Option<&str>`) is implemented by:

| Backend | File | Notes |
|---|---|---|
| **X11** | `x11.rs` | Direct global key grab. |
| **Wayland portal** | `portal.rs` | XDG Desktop `GlobalShortcuts` portal via `ashpd`/`zbus`; needs a reverse-DNS app id and a tokio runtime on the portal thread. |
| **Tray fallback** | `fallback.rs` | No global grab; the tray menu's "Quick Translate" item sets a flag polled by `DictApp`. |

`create_hotkey_manager()` picks the right backend at runtime. `poll()` drains
pending events via `try_recv()`. `update_settings` / `reconfigure_hotkey`
re-register when the hotkey string or `enabled` flag changes.

### Tray menu

The tray menu has a "Quick Translate" item. Selecting it sets a trigger flag;
`DictApp`'s 100 ms poll loop sees it and calls `trigger_translate()` — the same
path as the hotkey. This is the reliable fallback on Wayland compositors that
don't support the GlobalShortcuts portal.

---

## Selection reading

`gpui/src/selection.rs`. `read_selected_text() -> Result<(String,
SelectionSource), SelectionError>`.

- Tries the **primary selection** first (X11 PRIMARY / Wayland primary), falling
  back to the **clipboard** (Ctrl+C buffer).
- `SelectionSource` is `Primary` or `Clipboard` (logged/telemetered).
- Text is trimmed; `Empty` is returned if blank, `TooLong(len)` if over
  `MAX_SELECTION_LENGTH = 10_000` chars.
- On failure, `trigger_translate` shows an `Error` popup with a hint to copy the
  text first.

`arboard` handles clipboard/primary on most platforms; `ashpd` is used where a
portal is required.

---

## Text-to-speech

`gpui/src/tts.rs`. `synthesize_bytes(text, lang, tts) -> Result<Vec<u8>>`. Two
backends, chosen by `TtsSettings.enabled`:

1. **AI TTS** — an OpenAI-compatible `POST {base}/audio/speech` endpoint
   (OpenAI, Groq, OpenRouter, local servers). Returns raw audio bytes. Used when
   `TtsSettings.enabled` is `true` and an API key is set. **On AI failure, it
   logs and falls through to platform TTS** rather than erroring.
2. **Platform TTS** — cross-platform: `espeak-ng`/`espeak` (Linux), `say` (macOS),
   PowerShell `System.Speech` (Windows).

Synthesis always runs on the background executor; `install_from_bytes` decodes
the result into the slot's `PlaybackController`. The Original row uses the
source-language voice hint; the Translation row uses the target language.

---

## Settings

`src/settings/mod.rs`. Persisted in `settings.toml` under
`quick_translate` (and `tts`).

### `QuickTranslateSettings`

| Field | Type | Default | Purpose |
|---|---|---|---|
| `enabled` | `bool` | `false` | Master switch for the feature + hotkey registration. |
| `hotkey` | `String` | `"Ctrl+Alt+D"` | Hotkey in `Mod+Mod+Key` form. |
| `llm_provider` | `LlmProvider` | `Anthropic` | `Anthropic` or `OpenAiCompatible`. |
| `api_key` | `String` | `""` | API key (plaintext; future: OS keyring). |
| `api_base_url` | `String` | `""` | Required for OpenAI-compatible; optional for Anthropic. |
| `model` | `String` | `"claude-sonnet-4-6"` | Model name. |
| `target_lang` | `String` | `"English"` | Target language for translation. |
| `tts` | `TtsSettings` | (see below) | TTS configuration. |

### `TtsSettings`

| Field | Type | Default | Purpose |
|---|---|---|---|
| `enabled` | `bool` | `false` | Use the AI TTS API instead of platform TTS. |
| `api_key` | `String` | `""` | TTS provider API key. |
| `api_base_url` | `String` | `"https://api.openai.com/v1"` | TTS base URL. |
| `model` | `String` | `"gpt-4o-mini-tts"` | TTS model. |
| `voice` | `String` | `"alloy"` | Voice name. |

The Quick Translate settings tab (`gpui/src/components/quick_translate_panel.rs`)
exposes all of the above: enable toggle + hotkey (+ a note showing the detected
hotkey backend), provider/model/key rows, a base-url row that appears only for
the OpenAI-compatible provider, a target-language selector (preset list plus
free-text), a "⚠ API key is required" warning when enabled without a key, and a
TTS section (toggle + api-key + provider/model/base-url preset + voice).

---

## Telemetry

Two events fire from this feature (`telemetry/`):

- `QuickTranslateTriggered { source }` — on hotkey/tray trigger. `source` is
  `Hotkey` (the tray path also reports `Hotkey` today).
- `QuickTranslateLookupPerformed { source }` — when a translation completes.
  `source` distinguishes `AutoPreview` from explicit triggers.

Both are no-ops under `NullTelemetry` when the user has not opted in. See
[telemetry.md](telemetry.md).

---

## Key files

| File | Role |
|---|---|
| [gpui/src/quick_translate.rs](../gpui/src/quick_translate.rs) | Orchestrator: hotkey poll, trigger, translate, popup status. |
| [gpui/src/components/translate_popup.rs](../gpui/src/components/translate_popup.rs) | Popup view, states, two playback slots, seek bar, Options panel. |
| [gpui/src/playback.rs](../gpui/src/playback.rs) | `PlaybackController` + `PlaybackState`. |
| [gpui/src/tts.rs](../gpui/src/tts.rs) | AI TTS + platform espeak synthesis. |
| [gpui/src/selection.rs](../gpui/src/selection.rs) | Primary/clipboard selection reading. |
| [gpui/src/hotkey/](../gpui/src/hotkey/) | X11, Wayland-portal, and tray-fallback hotkey backends. |
| [gpui/src/components/quick_translate_panel.rs](../gpui/src/components/quick_translate_panel.rs) | Settings tab UI. |
| [gpui/src/state.rs](../gpui/src/state.rs) | `DictState`: holds the engine + the two playback controllers + `qt_popup_window`. |
| [src/settings/mod.rs](../src/settings/mod.rs) | `QuickTranslateSettings` + `TtsSettings`. |
