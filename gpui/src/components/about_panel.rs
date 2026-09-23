use gpui::prelude::FluentBuilder;
use gpui::{
    FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, img, px,
};
use gpui_component::{h_flex, v_flex};

use crate::colors;

const VERSION: &str = env!("APP_VERSION");
const LICENSE: &str = env!("CARGO_PKG_LICENSE");
const AUTHORS: &str = env!("CARGO_PKG_AUTHORS");
const REPO_URL: &str = "https://github.com/logi-camp/dicto";

pub fn panel_content() -> gpui::AnyElement {
    v_flex()
        .w_full()
        .h(px(420.))
        .items_center()
        .justify_center()
        .gap(px(0.))
        .child(hero_section())
        .child(div().py(px(8.)))
        .child(description_card())
        .child(div().py(px(6.)))
        .child(info_card())
        .into_any_element()
}

fn hero_section() -> gpui::AnyElement {
    h_flex()
        .items_center()
        .gap(px(14.))
        .child(app_icon())
        .child(
            v_flex()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(20.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(colors::text())
                        .child("Dicto"),
                )
                .child(
                    div()
                        .px(px(8.))
                        .py(px(2.))
                        .rounded(px(10.))
                        .bg(colors::surface())
                        .border_1()
                        .border_color(colors::border())
                        .text_size(px(11.))
                        .text_color(colors::text_secondary())
                        .child(SharedString::from(VERSION.to_string())),
                ),
        )
        .into_any_element()
}

fn app_icon() -> gpui::AnyElement {
    img("icons/app-icon.svg")
        .w(px(52.))
        .h(px(52.))
        .into_any_element()
}

fn description_card() -> gpui::AnyElement {
    div()
        .w(px(340.))
        .px(px(16.))
        .py(px(12.))
        .rounded(px(8.))
        .bg(colors::surface())
        .border_1()
        .border_color(colors::border())
        .child(
            div()
                .text_center()
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(
                    "A fast, offline desktop dictionary for Linux. \
                     Reads MDX/MDD dictionary files \u{2014} the same format \
                     used by GoldenDict, MDict, and most popular \
                     dictionary packs.",
                )),
        )
        .into_any_element()
}

fn info_card() -> gpui::AnyElement {
    v_flex()
        .w(px(340.))
        .rounded(px(8.))
        .bg(colors::surface())
        .border_1()
        .border_color(colors::border())
        .overflow_hidden()
        .child(info_row("License", LICENSE, true))
        .child(info_row("Author", AUTHORS, true))
        .child(info_row("Engine", "GPUI (Zed)", true))
        .child(source_row())
        .into_any_element()
}

fn info_row(label: &str, value: &str, show_divider: bool) -> gpui::AnyElement {
    let row = h_flex()
        .w_full()
        .px(px(14.))
        .py(px(10.))
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors::text())
                .child(SharedString::from(value.to_string())),
        );

    v_flex()
        .w_full()
        .child(row)
        .when(show_divider, |el| {
            el.child(div().mx(px(14.)).h(px(1.)).bg(colors::border()))
        })
        .into_any_element()
}

fn source_row() -> gpui::AnyElement {
    h_flex()
        .w_full()
        .px(px(14.))
        .py(px(10.))
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors::text_secondary())
                .child("Source"),
        )
        .child(
            div()
                .id("about-source-link")
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors::primary())
                .cursor_pointer()
                .hover(|s| s.opacity(0.8))
                .child("github.com/logi-camp/dicto")
                .on_click(move |_, _, cx| {
                    cx.open_url(REPO_URL);
                }),
        )
        .into_any_element()
}
