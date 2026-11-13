# Vendored crates

## `proc-macro-error2`

- Version: 2.0.1 (latest published release at the time of vendoring)
- Source: <https://crates.io/crates/proc-macro-error2/2.0.1/download>
- License: MIT OR Apache-2.0 (see `proc-macro-error2/LICENSE-*`)

### Why it is vendored

`proc-macro-error2` reaches the build through `gpui → stacksafe → stacksafe-macro`.
Its `__export` module re-exports `proc_macro` from a private `extern crate`, which
rustc 1.99 reports as a future incompatibility (E0365,
<https://github.com/rust-lang/rust/issues/127909>). Upstream master still contains
the offending line and no fixed release exists, so the workspace overrides the
crate through `[patch.crates-io]` in the root `Cargo.toml`.

### Local change

Only one line differs from the published crate. In `src/lib.rs`:

```diff
-extern crate proc_macro;
+pub extern crate proc_macro;
```

This is the change rustc itself suggests for E0365. It has no effect on the
crate's API: `__export` still re-exports `proc_macro` under the same path.

### Removing it

Delete this directory and the `[patch.crates-io]` section plus the `exclude`
entry in the root `Cargo.toml` once a `proc-macro-error2` release that re-exports
`proc_macro` publicly is published. The upstream repository is
<https://github.com/GnomedDev/proc-macro-error-2>.
