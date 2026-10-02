# mdict-rs documentation

This folder collects everything you need to navigate or extend the
project. Start with [architecture.md](architecture.md) for the
30-second tour, then drop into a topic when you need depth.

## Project-specific guides (write when you change behavior)

| File | What's in it |
|------|--------------|
| [architecture.md](architecture.md) | High-level overview, data flow, crate layout, threading model. |
| [mdict-format.md](mdict-format.md) | MDX/MDD on-disk layout, the parser's interpretation, FST index build, the query pipeline (including `@@@LINK=` redirects). |
| [rendering.md](rendering.md) | HTML → GPUI element pipeline: tokenizer, CSS subset, grouped link chips, separate text/background dark-theme remap, image extraction, audio playback, and rendering gotchas. |
| [settings.md](settings.md) | Per-dictionary configuration: enable/disable, reorder, hot reload, where settings live on disk. |
| [quick-translate.md](quick-translate.md) | Selection-translation popup: trigger flow (hotkey/tray), popup states, the two independent playback slots, seek bar, TTS, and the platform backends. |
| [telemetry.md](telemetry.md) | Privacy-first opt-in analytics: consent flow, the full event schema (with fire points), the privacy model, and how to add a new event. |

## External-library references (don't edit unless GPUI/gpui-component changes)

| File | What's in it |
|------|--------------|
| [gpui-api.md](gpui-api.md) | GPUI 0.2.2 API snapshot: entities, render trait, styling, colors, window options, async/background. |
| [gpui-components.md](gpui-components.md) | gpui-component 0.5.1 API snapshot: layout helpers, Button, Checkbox, Input, Table, Dialog, TabBar, Label, key imports. |
| [gpui-async.md](gpui-async.md) | Async patterns in GPUI: spawning, observe, background executor, timers. |

> The "external-library" docs were extracted from upstream sources
> early in the project. The current build pulls gpui-component from
> **git** (not crates.io) — most APIs line up but a few names differ
> (`Tab::new()` takes no id; `Scrollbar::vertical(&handle)` instead of
> a positional id; `AlertDialog::new(cx)` etc.). Sanity-check against
> `~/.cargo/git/checkouts/gpui-component-*/` when something doesn't
> compile.

## Conventions for these docs

- Lead with the *why*. The code already says what it does; doc value
  is in motivation, constraints, and pitfalls.
- Prefer concrete file:line references over abstract descriptions.
- Anything we hit during a debugging session and don't want to
  re-discover belongs in the relevant doc's "Gotchas" section.
- Don't paste large slabs of code. Show the smallest snippet that
  makes the point, then link to the file.
