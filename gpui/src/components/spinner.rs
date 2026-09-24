//! Small shared loading widgets: an inline three-dot spinner row with a
//! status label. Reusable by any page while an async operation runs.

use gpui::{IntoElement, ParentElement, SharedString, Styled as _, div, px};
use gpui_component::h_flex;

use crate::colors;

/// A row of three pulsing dots + a status label (e.g. "Translating…").
/// The label is optional — pass an empty string for dots only.
pub fn spinner_row(label: &str) -> gpui::AnyElement {
    let mut row = h_flex().gap(px(6.)).items_center().pb(px(4.));
    row = row.child(dots());
    if !label.is_empty() {
        row = row.child(
            div()
                .text_size(px(13.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(label.to_string())),
        );
    }
    row.into_any_element()
}

/// Just the three dots, no label.
pub fn dots() -> gpui::AnyElement {
    h_flex()
        .gap(px(2.))
        .child(dot(colors::text_secondary()))
        .child(dot(colors::text_secondary()))
        .child(dot(colors::text_secondary()))
        .into_any_element()
}

fn dot(color: gpui::Hsla) -> gpui::AnyElement {
    div()
        .w(px(4.))
        .h(px(4.))
        .rounded(px(2.))
        .bg(color)
        .into_any_element()
}
