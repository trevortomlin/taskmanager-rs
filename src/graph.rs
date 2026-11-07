//! A lightweight time-series graph drawn with gpui paths: gradient area fill,
//! a crisp line on top, and optional horizontal grid lines.
//!
//! Graphs are redrawn every frame (hover alone triggers a full redraw), but their
//! data only changes once per sample. Tessellated paths are therefore cached per
//! graph and rebuilt only when the data, scale or on-screen bounds change.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use gpui::{
    Background, Bounds, IntoElement, Path, PathBuilder, Pixels, Point, Rgba, Styled, canvas, fill,
    linear_color_stop, linear_gradient, point, px,
};

use crate::theme::theme;

/// Number of samples kept per series (one per refresh, so ~60 seconds).
pub const HISTORY_LEN: usize = 60;
/// Axis label for the time span covered by [`HISTORY_LEN`] samples.
pub const HISTORY_LABEL: &str = "60 seconds";

/// A fixed-capacity rolling series of samples, oldest first.
#[derive(Clone, Debug)]
pub struct Series {
    samples: VecDeque<f32>,
    /// Contiguous copy shared with graphs, rebuilt once per push instead of per frame.
    shared: Arc<[f32]>,
    /// Bumped on every push so caches can tell when the data changed.
    generation: u64,
}

impl Default for Series {
    fn default() -> Self {
        Self {
            samples: VecDeque::with_capacity(HISTORY_LEN),
            shared: Arc::new([]),
            generation: 0,
        }
    }
}

impl Series {
    pub fn push(&mut self, value: f32) {
        if self.samples.len() == HISTORY_LEN {
            self.samples.pop_front();
        }
        self.samples.push_back(value);
        self.shared = self.samples.iter().copied().collect();
        self.generation += 1;
    }

    pub fn max(&self) -> f32 {
        self.shared.iter().copied().fold(0.0, f32::max)
    }
}

pub struct Line {
    values: Arc<[f32]>,
    generation: u64,
    color: Rgba,
    /// Draw a gradient area under the line.
    area: bool,
}

impl Line {
    pub fn new(series: &Series, color: Rgba) -> Self {
        Self {
            values: series.shared.clone(),
            generation: series.generation,
            color,
            area: true,
        }
    }

    pub fn without_area(mut self) -> Self {
        self.area = false;
        self
    }
}

/// Identifies a graph across frames so its paths can be cached.
pub type GraphId = (&'static str, usize);

/// Renders `lines` scaled so `max` touches the top edge. The newest sample sits at
/// the right edge, so a partially filled history grows in from the right.
pub fn graph(
    id: GraphId,
    lines: Vec<Line>,
    max: f32,
    grid_rows: usize,
) -> impl IntoElement + Styled {
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
            let key = cache_key(&lines, max, bounds);
            PATH_CACHE.with_borrow_mut(|cache| {
                let entry = cache.entry(id).or_default();
                if entry.key != key {
                    entry.key = key;
                    entry.paths = build_paths(&lines, max, bounds);
                }
                for (path, background) in &entry.paths {
                    window.paint_path(path.clone(), *background);
                }
            });
        },
    )
}

#[derive(Default)]
struct CachedGraph {
    key: u64,
    paths: Vec<(Path<Pixels>, Background)>,
}

thread_local! {
    /// gpui renders on a single thread, so a thread-local cache needs no locking.
    static PATH_CACHE: RefCell<HashMap<GraphId, CachedGraph>> = RefCell::default();
}

fn cache_key(lines: &[Line], max: f32, bounds: Bounds<Pixels>) -> u64 {
    let mut hasher = DefaultHasher::new();
    max.to_bits().hash(&mut hasher);
    for v in [
        bounds.origin.x,
        bounds.origin.y,
        bounds.size.width,
        bounds.size.height,
    ] {
        f32::from(v).to_bits().hash(&mut hasher);
    }
    for line in lines {
        line.generation.hash(&mut hasher);
        line.values.len().hash(&mut hasher);
        line.area.hash(&mut hasher);
        [line.color.r, line.color.g, line.color.b, line.color.a]
            .map(f32::to_bits)
            .hash(&mut hasher);
    }
    hasher.finish()
}

fn build_paths(
    lines: &[Line],
    max: f32,
    bounds: Bounds<Pixels>,
) -> Vec<(Path<Pixels>, Background)> {
    let mut paths = Vec::new();
    for line in lines {
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
                paths.push((
                    path,
                    linear_gradient(
                        180.,
                        linear_color_stop(line.color.opacity(0.38), 0.),
                        linear_color_stop(line.color.opacity(0.02), 1.),
                    ),
                ));
            }
        }
        let mut stroke = PathBuilder::stroke(px(1.5));
        stroke.move_to(points[0]);
        for p in &points[1..] {
            stroke.line_to(*p);
        }
        if let Ok(path) = stroke.build() {
            paths.push((path, line.color.into()));
        }
    }
    paths
}

fn plot(values: &[f32], max: f32, bounds: Bounds<Pixels>) -> Vec<Point<Pixels>> {
    let max = if max > 0.0 { max } else { 1.0 };
    let step = bounds.size.width / (HISTORY_LEN - 1) as f32;
    let values = &values[values.len().saturating_sub(HISTORY_LEN)..];
    let first_slot = HISTORY_LEN - values.len();
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let level = (v / max).clamp(0.0, 1.0);
            point(
                bounds.left() + step * (first_slot + i) as f32,
                // Inset by one pixel so flat-out values aren't clipped.
                bounds.bottom() - (bounds.size.height - px(1.)) * level,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_label_matches_length() {
        assert_eq!(HISTORY_LABEL, format!("{HISTORY_LEN} seconds"));
    }

    #[test]
    fn series_keeps_only_recent_samples() {
        let mut s = Series::default();
        for i in 0..(HISTORY_LEN + 5) {
            s.push(i as f32);
        }
        assert_eq!(s.shared.len(), HISTORY_LEN);
        assert_eq!(s.shared[0], 5.0);
        assert_eq!(s.max(), (HISTORY_LEN + 4) as f32);
        assert_eq!(s.generation, (HISTORY_LEN + 5) as u64);
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

    #[test]
    fn cache_key_changes_with_data_and_bounds() {
        let mut s = Series::default();
        s.push(1.0);
        let bounds = Bounds::new(point(px(0.), px(0.)), gpui::size(px(100.), px(50.)));
        let color = gpui::rgb(0x123456);
        let key = |s: &Series, b| cache_key(&[Line::new(s, color)], 100.0, b);

        let before = key(&s, bounds);
        assert_eq!(before, key(&s, bounds), "stable for identical input");
        let moved = Bounds::new(point(px(1.), px(0.)), bounds.size);
        assert_ne!(before, key(&s, moved));
        s.push(2.0);
        assert_ne!(before, key(&s, bounds));
    }
}
