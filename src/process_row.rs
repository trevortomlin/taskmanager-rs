//! A custom element that paints all cells of a process-table row directly.
//!
//! Composing a row from divs costs ~10 layout nodes per row, and gpui re-runs
//! layout for the whole window on every frame (hover included). Painting the cells
//! ourselves needs a single layout node per row, roughly halving the Processes
//! tab's frame time. Shaped text is still cached by gpui between frames.

use gpui::{
    App, Bounds, Element, ElementId, Font, FontWeight, GlobalElementId, Hsla, InspectorElementId,
    IntoElement, LayoutId, Pixels, SharedString, Style, TextAlign, TextRun, TruncateFrom, Window,
    fill, point, px, relative, size,
};

use crate::{
    processes::Column,
    sampler::ProcessRow,
    theme::{heat, theme},
};

/// Horizontal padding inside each cell, matching the header's `px_2`.
const CELL_PADDING: Pixels = px(8.);
const BADGE_SIZE: Pixels = px(18.);
const BADGE_GAP: Pixels = px(8.);
/// Disk rate at which a cell's heat tint saturates.
const DISK_HEAT_FULL: f32 = 50.0 * 1024.0 * 1024.0;

/// Left edge and width of each column for a row of the given width, in
/// [`Column::ALL`] order. Mirrors the flex layout the header uses.
pub fn column_spans(width: Pixels) -> [(Pixels, Pixels); 6] {
    let fixed: Pixels = Column::ALL.iter().filter_map(|c| c.fixed_width()).sum();
    let mut x = px(0.);
    Column::ALL.map(|column| {
        let w = column
            .fixed_width()
            .unwrap_or_else(|| (width - fixed).max(Column::NAME_MIN_WIDTH));
        let span = (x, w);
        x += w;
        span
    })
}

pub struct RowCells {
    name: SharedString,
    initial: SharedString,
    badge_hue: f32,
    pid: SharedString,
    user: SharedString,
    cpu: SharedString,
    memory: SharedString,
    disk: SharedString,
    heat: [Hsla; 3],
}

impl RowCells {
    pub fn new(row: &ProcessRow, memory_total: u64) -> Self {
        let t = theme();
        let d = &row.display;
        Self {
            name: row.name.clone(),
            initial: d.initial.clone(),
            badge_hue: d.badge_hue,
            pid: d.pid.clone(),
            user: row.user.clone(),
            cpu: d.cpu.clone(),
            memory: d.memory.clone(),
            disk: d.disk.clone(),
            heat: [
                heat(t.cpu, row.cpu / 25.0),
                heat(
                    t.memory,
                    row.memory as f32 / memory_total.max(1) as f32 * 4.0,
                ),
                heat(t.disk, row.disk as f32 / DISK_HEAT_FULL),
            ],
        }
    }
}

impl IntoElement for RowCells {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for RowCells {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let t = theme();
        let style = window.text_style();
        let painter = TextPainter {
            font: style.font(),
            font_size: style.font_size.to_pixels(window.rem_size()),
            line_height: window.line_height(),
            top: bounds.top(),
            height: bounds.size.height,
        };
        let spans = column_spans(bounds.size.width);
        let cell = |column: Column| {
            let (x, w) = spans[column as usize];
            (bounds.left() + x, w)
        };

        // Heat tints sit behind the numeric cells.
        for (column, tint) in [Column::Cpu, Column::Memory, Column::Disk]
            .into_iter()
            .zip(self.heat)
        {
            if tint.a > 0.0 {
                let (x, w) = cell(column);
                window.paint_quad(fill(
                    Bounds::new(point(x, bounds.top()), size(w, bounds.size.height)),
                    tint,
                ));
            }
        }

        // Name cell: colored initial badge, then the (truncated) name.
        let (x, w) = cell(Column::Name);
        let badge_x = x + CELL_PADDING;
        let badge = Bounds::new(
            point(
                badge_x,
                bounds.top() + (bounds.size.height - BADGE_SIZE) / 2.,
            ),
            size(BADGE_SIZE, BADGE_SIZE),
        );
        window.paint_quad(
            fill(badge, gpui::hsla(self.badge_hue, 0.45, 0.35, 1.0)).corner_radii(px(5.)),
        );
        let badge_painter = TextPainter {
            font: Font {
                weight: FontWeight::BOLD,
                ..painter.font.clone()
            },
            font_size: px(12.),
            ..painter.clone()
        };
        badge_painter.paint(
            &self.initial,
            gpui::hsla(self.badge_hue, 0.6, 0.88, 1.0),
            badge.left(),
            BADGE_SIZE,
            Align::Center,
            window,
            cx,
        );
        let text_x = badge_x + BADGE_SIZE + BADGE_GAP;
        painter.paint(
            &self.name,
            t.text.into(),
            text_x,
            x + w - CELL_PADDING - text_x,
            Align::Left,
            window,
            cx,
        );

        let mut text_cell = |column: Column, text: &SharedString, color: Hsla, align: Align| {
            let (x, w) = cell(column);
            painter.paint(
                text,
                color,
                x + CELL_PADDING,
                w - CELL_PADDING * 2.,
                align,
                window,
                cx,
            );
        };
        text_cell(Column::Pid, &self.pid, t.text_faint.into(), Align::Right);
        text_cell(Column::User, &self.user, t.text_muted.into(), Align::Left);
        text_cell(Column::Cpu, &self.cpu, t.text.into(), Align::Right);
        text_cell(Column::Memory, &self.memory, t.text.into(), Align::Right);
        text_cell(Column::Disk, &self.disk, t.text.into(), Align::Right);
    }
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone)]
struct TextPainter {
    font: Font,
    font_size: Pixels,
    line_height: Pixels,
    top: Pixels,
    height: Pixels,
}

impl TextPainter {
    /// Paints one line of text inside `[x, x + width)`, vertically centered,
    /// truncating with an ellipsis when it doesn't fit.
    #[allow(clippy::too_many_arguments)]
    fn paint(
        &self,
        text: &SharedString,
        color: Hsla,
        x: Pixels,
        width: Pixels,
        align: Align,
        window: &mut Window,
        cx: &mut App,
    ) {
        if text.is_empty() || width <= px(0.) {
            return;
        }
        let run = TextRun {
            len: text.len(),
            font: self.font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = [run];
        let mut line = window
            .text_system()
            .shape_line(text.clone(), self.font_size, &runs, None);
        if line.width() > width {
            let mut wrapper = cx
                .text_system()
                .line_wrapper(self.font.clone(), self.font_size);
            let (truncated, truncated_runs) =
                wrapper.truncate_line(text.clone(), width, "…", &runs, TruncateFrom::End);
            line =
                window
                    .text_system()
                    .shape_line(truncated, self.font_size, &truncated_runs, None);
        }
        let x = match align {
            Align::Left => x,
            Align::Center => x + (width - line.width()) / 2.,
            Align::Right => x + width - line.width(),
        };
        let y = self.top + (self.height - self.line_height) / 2.;
        line.paint(
            point(x, y),
            self.line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        )
        .ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_fill_the_row_and_match_fixed_widths() {
        let spans = column_spans(px(900.));
        let fixed: Pixels = Column::ALL.iter().filter_map(|c| c.fixed_width()).sum();
        assert_eq!(spans[0], (px(0.), px(900.) - fixed));
        for pair in spans.windows(2) {
            assert_eq!(pair[0].0 + pair[0].1, pair[1].0, "columns are contiguous");
        }
        let (last_x, last_w) = spans[5];
        assert_eq!(last_x + last_w, px(900.));
    }

    #[test]
    fn name_column_keeps_its_minimum_width() {
        let spans = column_spans(px(100.));
        assert_eq!(spans[0].1, Column::NAME_MIN_WIDTH);
    }
}
