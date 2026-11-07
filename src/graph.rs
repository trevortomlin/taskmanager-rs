//! A lightweight time-series graph drawn with gpui paths: gradient area fill,
//! a crisp line on top, and optional horizontal grid lines.

use std::collections::VecDeque;

use gpui::{
    Bounds, IntoElement, PathBuilder, Pixels, Point, Rgba, Styled, canvas, fill, linear_color_stop,
    linear_gradient, point, px,
};

use crate::theme::theme;

/// Number of samples kept per series (one per refresh, so ~60 seconds).
pub const HISTORY_LEN: usize = 60;

/// A fixed-capacity rolling series of samples, oldest first.
#[derive(Clone, Debug, Default)]
pub struct Series(VecDeque<f32>);

impl Series {
    pub fn push(&mut self, value: f32) {
        if self.0.len() == HISTORY_LEN {
            self.0.pop_front();
        }
        self.0.push_back(value);
    }

    pub fn max(&self) -> f32 {
        self.0.iter().copied().fold(0.0, f32::max)
    }

    pub fn values(&self) -> Vec<f32> {
        self.0.iter().copied().collect()
    }
}

pub struct Line {
    pub values: Vec<f32>,
    pub color: Rgba,
    /// Draw a gradient area under the line.
    pub area: bool,
}

impl Line {
    pub fn new(series: &Series, color: Rgba) -> Self {
        Self {
            values: series.values(),
            color,
            area: true,
        }
    }

    pub fn without_area(mut self) -> Self {
        self.area = false;
        self
    }
}

/// Renders `lines` scaled so `max` touches the top edge. The newest sample sits at
/// the right edge, so a partially filled history grows in from the right.
pub fn graph(lines: Vec<Line>, max: f32, grid_rows: usize) -> impl IntoElement + Styled {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let t = theme();
            for row in 1..grid_rows {
                let y = bounds.top() + bounds.size.height * (row as f32 / grid_rows as f32);
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.left(), y),
                        gpui::size(bounds.size.width, px(1.)),
                    ),
                    t.grid,
                ));
            }
            for line in &lines {
                let points = plot(&line.values, max, bounds);
                if points.len() < 2 {
                    continue;
                }
                if line.area {
                    let mut area = PathBuilder::fill();
                    area.move_to(point(points[0].x, bounds.bottom()));
                    for p in &points {
                        area.line_to(*p);
                    }
                    area.line_to(point(points[points.len() - 1].x, bounds.bottom()));
                    area.close();
                    if let Ok(path) = area.build() {
                        window.paint_path(
                            path,
                            linear_gradient(
                                180.,
                                linear_color_stop(line.color.opacity(0.38), 0.),
                                linear_color_stop(line.color.opacity(0.02), 1.),
                            ),
                        );
                    }
                }
                let mut stroke = PathBuilder::stroke(px(1.5));
                stroke.move_to(points[0]);
                for p in &points[1..] {
                    stroke.line_to(*p);
                }
                if let Ok(path) = stroke.build() {
                    window.paint_path(path, line.color);
                }
            }
        },
    )
}

fn plot(values: &[f32], max: f32, bounds: Bounds<Pixels>) -> Vec<Point<Pixels>> {
    let max = if max > 0.0 { max } else { 1.0 };
    let step = bounds.size.width / (HISTORY_LEN - 1) as f32;
    let first_slot = HISTORY_LEN.saturating_sub(values.len());
    values
        .iter()
        .rev()
        .take(HISTORY_LEN)
        .rev()
        .enumerate()
        .map(|(i, v)| {
            let level = (v / max).clamp(0.0, 1.0);
            point(
                bounds.left() + step * (first_slot + i) as f32,
                // Inset by half the stroke so flat-out values aren't clipped.
                bounds.bottom() - (bounds.size.height - px(1.)) * level,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_keeps_only_recent_samples() {
        let mut s = Series::default();
        for i in 0..(HISTORY_LEN + 5) {
            s.push(i as f32);
        }
        let values = s.values();
        assert_eq!(values.len(), HISTORY_LEN);
        assert_eq!(values[0], 5.0);
        assert_eq!(s.max(), (HISTORY_LEN + 4) as f32);
    }

    #[test]
    fn plot_right_aligns_partial_history() {
        let bounds = Bounds::new(point(px(0.), px(0.)), gpui::size(px(590.), px(100.)));
        let points = plot(&[0.0, 50.0, 100.0], 100.0, bounds);
        assert_eq!(points.len(), 3);
        assert_eq!(points[2].x, px(590.));
        assert_eq!(points[0].y, px(100.));
        assert_eq!(points[2].y, px(1.));
    }
}
