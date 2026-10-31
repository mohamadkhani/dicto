//! System tray support.
//!
//! Platform backends behind a plain facade (the OS is known at compile time;
//! only the hotkey layer needs runtime detection):
//! - Linux: `ksni` speaking SNI over D-Bus ([`ksni`])
//! - Windows: the `tray-icon` crate ([`win`])
//!
//! Both backends expose the same contract: they spawn the tray and return
//! the action channel the GPUI main loop polls plus the shared
//! activation-token slot. Actions are handled in `main.rs`; the "Quick
//! Translate" item shares the trigger path used by `dicto --translate` IPC
//! (see `app.rs`). The quick-translate / lookup items appear only while
//! their feature is enabled — [`sync_menu_visibility`] keeps the menu in
//! step with the settings.

pub mod icon;

#[cfg(target_os = "linux")]
pub mod ksni;
#[cfg(target_os = "windows")]
pub mod win;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};

/// The last compositor-provided activation token, shared between the SNI
/// service thread (which receives it) and the menu-item click handlers.
///
/// Forwarded to the popup window's `Window::activate_with_token` (provided by
/// the mohamadkhani/zed gpui fork) for authoritative focus on GNOME/Wayland.
/// Always `None` on Windows (a Wayland-only concept).
pub type SharedToken = Arc<Mutex<Option<String>>>;

/// Which optional items the tray menu shows and the shortcut labels they
/// display, mirroring the features' `enabled` / `hotkey` settings. Backends
/// read it when building their menu and rebuild when any value flips (both
/// cache their menu layout).
#[derive(Default)]
pub struct MenuVisibility {
    quick_translate: AtomicBool,
    quick_lookup: AtomicBool,
    translate_hotkey: Mutex<String>,
    lookup_hotkey: Mutex<String>,
}

impl MenuVisibility {
    /// Store the current flags and hotkeys; returns `true` when any value
    /// changed (i.e. the menu needs a rebuild).
    fn sync(
        &self,
        quick_translate: bool,
        quick_lookup: bool,
        translate_hotkey: &str,
        lookup_hotkey: &str,
    ) -> bool {
        let qt_changed = self
            .quick_translate
            .swap(quick_translate, Ordering::Relaxed)
            != quick_translate;
        let wl_changed = self.quick_lookup.swap(quick_lookup, Ordering::Relaxed) != quick_lookup;
        let th_changed = self.swap_hotkey(&self.translate_hotkey, translate_hotkey);
        let lh_changed = self.swap_hotkey(&self.lookup_hotkey, lookup_hotkey);
        qt_changed || wl_changed || th_changed || lh_changed
    }

    fn swap_hotkey(&self, slot: &Mutex<String>, value: &str) -> bool {
        let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::replace(&mut *guard, value.to_owned()) != value
    }

    pub fn quick_translate(&self) -> bool {
        self.quick_translate.load(Ordering::Relaxed)
    }

    pub fn quick_lookup(&self) -> bool {
        self.quick_lookup.load(Ordering::Relaxed)
    }

    pub fn translate_hotkey(&self) -> String {
        self.translate_hotkey
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn lookup_hotkey(&self) -> String {
        self.lookup_hotkey
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

pub type SharedVisibility = Arc<MenuVisibility>;

/// Asks the backend to rebuild its cached menu; fired by
/// [`sync_menu_visibility`] when a visibility flag flips.
pub type MenuRefresh = Arc<dyn Fn() + Send + Sync>;

/// The menu-sync state installed by [`spawn_tray`]. `OnceLock` because there
/// is exactly one tray per process.
static MENU_SYNC: OnceLock<(SharedVisibility, MenuRefresh)> = OnceLock::new();

/// Keep the tray menu in step with the features' `enabled` settings. Called
/// from `DictState::save_settings`, so every settings save re-syncs the
/// menu; the rebuild only fires when a flag actually flipped.
pub fn sync_menu_visibility(
    quick_translate: bool,
    quick_lookup: bool,
    translate_hotkey: &str,
    lookup_hotkey: &str,
) {
    if let Some((visibility, refresh)) = MENU_SYNC.get()
        && visibility.sync(
            quick_translate,
            quick_lookup,
            translate_hotkey,
            lookup_hotkey,
        )
    {
        refresh();
    }
}

/// Actions the tray requests the GPUI main loop to perform.
#[derive(Debug)]
pub enum TrayAction {
    /// Open or raise the dictionary window. The optional token is the
    /// compositor-minted xdg-activation token to feed `activate_with_token`
    /// (captured at click time — same pattern as netkeep's "Settings…"
    /// item).
    Show {
        token: Option<String>,
    },
    QuickTranslate,
    QuickLookup,
    Quit,
}

/// Spawn the platform tray. Returns the channel the GPUI main loop polls for
/// actions and the shared activation-token slot.
pub fn spawn_tray() -> (mpsc::Receiver<TrayAction>, SharedToken) {
    // Seed the menu with the enabled features' items only; the app side
    // keeps it in step afterwards via `sync_menu_visibility`.
    let settings = mdict_rs::settings::current();
    let visibility = Arc::new(MenuVisibility {
        quick_translate: AtomicBool::new(settings.quick_translate.enabled),
        quick_lookup: AtomicBool::new(settings.word_lookup.enabled),
        translate_hotkey: Mutex::new(settings.quick_translate.hotkey.clone()),
        lookup_hotkey: Mutex::new(settings.word_lookup.hotkey.clone()),
    });

    #[cfg(target_os = "linux")]
    let (actions, token, refresh) = ksni::spawn(visibility.clone());
    #[cfg(target_os = "windows")]
    let (actions, token, refresh) = win::spawn(visibility.clone());

    let _ = MENU_SYNC.set((visibility, refresh));

    (actions, token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_sync_reports_changes_only() {
        let vis = MenuVisibility::default();
        assert!(!vis.quick_translate());
        assert!(!vis.quick_lookup());

        // First sync always counts as a change (the menu was seeded from
        // the settings, but the caller's values are authoritative).
        assert!(vis.sync(true, true, "Ctrl+Alt+D", "Ctrl+Alt+L"));
        assert!(vis.quick_translate());
        assert!(vis.quick_lookup());
        assert_eq!(vis.translate_hotkey(), "Ctrl+Alt+D");
        assert_eq!(vis.lookup_hotkey(), "Ctrl+Alt+L");

        // Re-syncing identical values must not trigger a rebuild.
        assert!(!vis.sync(true, true, "Ctrl+Alt+D", "Ctrl+Alt+L"));

        assert!(vis.sync(true, false, "Ctrl+Alt+D", "Ctrl+Alt+L"));
        assert!(vis.quick_translate());
        assert!(!vis.quick_lookup());

        // A hotkey change alone must trigger a rebuild.
        assert!(vis.sync(true, false, "Ctrl+Alt+T", "Ctrl+Alt+L"));
        assert_eq!(vis.translate_hotkey(), "Ctrl+Alt+T");
    }
}
