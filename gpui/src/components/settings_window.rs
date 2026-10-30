use gpui::{AppContext as _, Entity, IntoElement, ParentElement, Styled};
use gpui_component::{
    h_flex,
    tab::{Tab, TabBar},
};

use crate::state::DictState;

/// Settings tab indices — the single source of truth for the tab order in
/// [`header_tabs_for_dialog`] and every hardcoded jump-to-tab elsewhere.
pub const TAB_GENERAL: usize = 0;
pub const TAB_DICTIONARIES: usize = 1;
pub const TAB_IMPORT: usize = 2;
pub const TAB_DOWNLOAD: usize = 3;
pub const TAB_QUICK_TRANSLATE: usize = 4;
pub const TAB_ABOUT: usize = 5;

/// Dialog-compatible header tabs (takes &mut App).
pub fn header_tabs_for_dialog(
    state: Entity<DictState>,
    active_tab: usize,
    _cx: &mut gpui::App,
) -> gpui::AnyElement {
    let tab_state = state.clone();

    h_flex()
        .w_full()
        .child(
            TabBar::new("settings-tabs")
                .underline()
                .selected_index(active_tab)
                .cursor_pointer()
                .on_click(move |&ix, _window, cx| {
                    if let Some(tab) = settings_tab_from_index(ix) {
                        dicto_telemetry::get()
                            .track(dicto_telemetry::Event::SettingsTabSelected { tab });
                    }
                    cx.update_entity(&tab_state, |s, cx| {
                        s.settings_active_tab = ix;
                        cx.notify();
                    });
                })
                .child(Tab::new().label("General"))
                .child(Tab::new().label("Dictionaries"))
                .child(Tab::new().label("Import"))
                .child(Tab::new().label("Download"))
                .child(Tab::new().label("Quick Translate"))
                .child(Tab::new().label("About")),
        )
        .into_any_element()
}

/// Map a settings tab index to its [`SettingsTab`] variant for telemetry.
/// Returns `None` if the index doesn't match a known tab, so a future
/// re-order never fires a bogus event — it goes silent instead.
fn settings_tab_from_index(ix: usize) -> Option<dicto_telemetry::SettingsTab> {
    match ix {
        TAB_GENERAL => Some(dicto_telemetry::SettingsTab::General),
        TAB_DICTIONARIES => Some(dicto_telemetry::SettingsTab::Dictionaries),
        TAB_IMPORT => Some(dicto_telemetry::SettingsTab::Import),
        TAB_DOWNLOAD => Some(dicto_telemetry::SettingsTab::Download),
        TAB_QUICK_TRANSLATE => Some(dicto_telemetry::SettingsTab::QuickTranslate),
        TAB_ABOUT => Some(dicto_telemetry::SettingsTab::About),
        _ => None,
    }
}
