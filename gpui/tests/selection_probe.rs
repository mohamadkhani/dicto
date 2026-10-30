//! Probe: does the arboard clipboard read (used by quick-translate's
//! selection reader) block on this desktop? Run:
//! `cargo test -p dicto --test selection_probe -- --ignored --nocapture`

use std::time::Instant;

use arboard::{Clipboard, GetExtLinux as _, LinuxClipboardKind};

#[test]
#[ignore]
fn probe_clipboard_reads() {
    println!("-- Clipboard::new --");
    let t = Instant::now();
    let mut clipboard = Clipboard::new().expect("clipboard new failed");
    println!("new ok in {:?}", t.elapsed());

    println!("-- get primary --");
    let t = Instant::now();
    let primary = clipboard
        .get()
        .clipboard(LinuxClipboardKind::Primary)
        .text();
    println!(
        "primary done in {:?} -> {:?}",
        t.elapsed(),
        primary.as_ref().map(|s| s.len())
    );

    println!("-- get clipboard text --");
    let t = Instant::now();
    let text = clipboard.get_text();
    println!(
        "clipboard done in {:?} -> {:?}",
        t.elapsed(),
        text.as_ref().map(|s| s.len())
    );
}
