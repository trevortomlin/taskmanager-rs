//! Small presentational building blocks shared across views.

use gpui::{
    Div, IntoElement, PathBuilder, Rgba, SharedString, canvas, div, point, prelude::*, px, relative,
};

use crate::theme::theme;

/// Centered, muted message filling the available space (loading / no results).
pub fn empty_state(message: impl Into<SharedString>) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme().text_faint)
        .child(message.into())
}

/// A small filled circle used as a color legend.
pub fn color_dot(color: Rgba) -> Div {
    div().flex_shrink_0().size_2().rounded_full().bg(color)
}

pub fn progress_bar(fraction: f32, color: Rgba) -> impl IntoElement {
    div()
        .h(px(4.))
        .w_full()
        .rounded_full()
        .bg(theme().border)
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(color)
                .w(relative(fraction.clamp(0.0, 1.0))),
        )
}

pub fn section_heading(label: &'static str) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme().text_muted)
        .child(label)
}

/// A label over a large value, used for key figures.
pub fn stat_tile(label: &'static str, value: String) -> impl IntoElement {
    let t = theme();
    div()
        .flex()
        .flex_col()
        .gap_0p5()
        .min_w(px(120.))
        .child(div().text_xs().text_color(t.text_faint).child(label))
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(t.text)
                .child(value),
        )
}

/// A magnifying glass drawn with paths, so no icon font or asset is needed.
pub fn search_icon(color: Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let r = px(4.5);
            let c = point(bounds.left() + px(6.), bounds.top() + px(6.));
            let mut icon = PathBuilder::stroke(px(1.5));
            icon.move_to(point(c.x + r, c.y));
            icon.arc_to(point(r, r), px(0.), false, true, point(c.x - r, c.y));
            icon.arc_to(point(r, r), px(0.), false, true, point(c.x + r, c.y));
            let handle = point(c.x + r * 0.72, c.y + r * 0.72);
            icon.move_to(handle);
            icon.line_to(point(handle.x + px(3.5), handle.y + px(3.5)));
            if let Ok(path) = icon.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(15.))
    .flex_shrink_0()
}
