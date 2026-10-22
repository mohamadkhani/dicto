//! Small shared feedback widgets: inline warning banners and error text.
//! Reusable by any page — the popup, settings panels, import/download flows.

use gpui::{IntoElement, ParentElement, SharedString, Styled as _, div, px};
use gpui_component::{h_flex, v_flex};

use crate::colors;

/// Inline warning banner: ⚠ icon + bold title + detail line, left accent edge
/// in the warn color. Use for "nothing failed, but pay attention" states
/// (over-limit selections, missing API keys, …).
pub fn warning_banner(title: &str, detail: &str) -> gpui::AnyElement {
    h_flex()
        .gap(px(10.))
        .items_start()
        .p(px(10.))
        .border_1()
        .border_color(colors::border())
        // Left accent edge in the warn color.
        .border_l(px(2.))
        .rounded(px(6.))
        .bg(colors::bg())
        .child(
            div()
                .text_size(px(14.))
                .text_color(colors::update())
                .child(SharedString::from("⚠")),
        )
        .child(
            v_flex()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors::text())
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(SharedString::from(title.to_string())),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors::text_secondary())
                        .line_height(gpui::relative(1.4))
                        .child(SharedString::from(detail.to_string())),
                ),
        )
        .into_any_element()
}

/// Single-line error message in the error color. Use for "an operation
/// failed" states, prefixed with `context` (e.g. "Translation failed").
pub fn error_text(context: &str, error: &str) -> gpui::AnyElement {
    div()
        .text_size(px(12.))
        .text_color(colors::error())
        .line_height(gpui::relative(1.4))
        .child(SharedString::from(format!("{context}: {error}")))
        .into_any_element()
}
