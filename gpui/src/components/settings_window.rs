use gpui::{AppContext as _, Entity, IntoElement, ParentElement, Styled};
use gpui_component::{
    h_flex,
    tab::{Tab, TabBar},
};

use crate::state::DictState;

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
                .child(Tab::new().label("Dictionaries"))
                .child(Tab::new().label("Import"))
                .child(Tab::new().label("Download"))
                .child(Tab::new().label("Quick Translate"))
                .child(Tab::new().label("Telemetry"))
                .child(Tab::new().label("About")),
        )
        .into_any_element()
}

/// Map a settings tab index (0-5) to its [`SettingsTab`] variant for
/// telemetry. Returns `None` if the index doesn't match a known tab, so
/// a future re-order never fires a bogus event — it goes silent instead.
fn settings_tab_from_index(ix: usize) -> Option<dicto_telemetry::SettingsTab> {
    match ix {
        0 => Some(dicto_telemetry::SettingsTab::Dictionaries),
        1 => Some(dicto_telemetry::SettingsTab::Import),
        2 => Some(dicto_telemetry::SettingsTab::Download),
        3 => Some(dicto_telemetry::SettingsTab::QuickTranslate),
        4 => Some(dicto_telemetry::SettingsTab::Telemetry),
        5 => Some(dicto_telemetry::SettingsTab::About),
        _ => None,
    }
}
