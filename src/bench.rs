//! Opt-in frame benchmark, used to check the UI keeps up with high-refresh displays.
//!
//! Run with `TIDEWATCH_BENCH=1 cargo run --release`. The app redraws every frame,
//! cycles through each view (including continuous scrolling), prints frame
//! statistics per view to stderr, then exits. Keep the window visible while it
//! runs: compositors stop delivering frames to hidden windows.

use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use crate::performance::Resource;

/// Shared slot a paint callback fills with the time the frame finished painting.
pub type PaintMarker = Rc<Cell<Option<Instant>>>;

const WARMUP: Duration = Duration::from_secs(3);
const PHASE_LEN: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    ProcessesIdle,
    ProcessesScrolling,
    Performance(Resource),
}

impl Phase {
    const ALL: [Phase; 6] = [
        Phase::ProcessesIdle,
        Phase::ProcessesScrolling,
        Phase::Performance(Resource::Cpu),
        Phase::Performance(Resource::Memory),
        Phase::Performance(Resource::Network),
        Phase::Performance(Resource::Disk),
    ];

    fn label(self) -> String {
        match self {
            Phase::ProcessesIdle => "processes".into(),
            Phase::ProcessesScrolling => "processes (scrolling)".into(),
            Phase::Performance(r) => format!("performance/{}", r.label().to_lowercase()),
        }
    }
}

pub struct Bench {
    started: Instant,
    phase_ix: usize,
    phase_started: Option<Instant>,
    last_frame: Option<Instant>,
    frame_start: Instant,
    /// Set by a paint callback at the end of the frame; shared so the callback
    /// doesn't need to borrow the view while it is being drawn.
    painted_at: PaintMarker,
    intervals_ms: Vec<f32>,
    work_ms: Vec<f32>,
    /// When each frame of the current phase started, to correlate drops with samples.
    frame_times: Vec<Instant>,
    sample_times: Vec<Instant>,
}

/// What the view should show this frame.
pub struct BenchFrame {
    pub phase: Phase,
    /// Seconds since the phase began, for driving animation such as scrolling.
    pub elapsed: f32,
    pub painted_at: PaintMarker,
}

impl Bench {
    pub fn from_env() -> Option<Self> {
        std::env::var_os("TIDEWATCH_BENCH")?;
        Some(Self {
            started: Instant::now(),
            phase_ix: 0,
            phase_started: None,
            last_frame: None,
            frame_start: Instant::now(),
            painted_at: Rc::new(Cell::new(None)),
            intervals_ms: Vec::new(),
            work_ms: Vec::new(),
            frame_times: Vec::new(),
            sample_times: Vec::new(),
        })
    }

    /// Call whenever a new snapshot is applied.
    pub fn note_sample(&mut self) {
        self.sample_times.push(Instant::now());
    }

    /// Call at the start of each render. Returns `None` once every phase is done.
    pub fn begin_frame(&mut self) -> Option<BenchFrame> {
        let now = Instant::now();
        self.record_previous_frame(now);
        self.frame_start = now;

        if now.duration_since(self.started) < WARMUP {
            return Some(self.frame(Phase::ALL[0], 0.0));
        }
        let phase_started = *self.phase_started.get_or_insert(now);
        if now.duration_since(phase_started) >= PHASE_LEN {
            self.report(Phase::ALL[self.phase_ix]);
            self.phase_ix += 1;
            self.phase_started = Some(now);
            self.last_frame = None;
        }
        let phase = *Phase::ALL.get(self.phase_ix)?;
        let elapsed = now
            .duration_since(self.phase_started.unwrap_or(now))
            .as_secs_f32();
        Some(self.frame(phase, elapsed))
    }

    fn frame(&self, phase: Phase, elapsed: f32) -> BenchFrame {
        BenchFrame {
            phase,
            elapsed,
            painted_at: self.painted_at.clone(),
        }
    }

    fn record_previous_frame(&mut self, now: Instant) {
        let measuring = self.phase_started.is_some();
        if let (true, Some(last)) = (measuring, self.last_frame) {
            self.frame_times.push(now);
            self.intervals_ms
                .push(now.duration_since(last).as_secs_f32() * 1000.0);
            if let Some(painted) = self.painted_at.take() {
                self.work_ms
                    .push(painted.duration_since(self.frame_start).as_secs_f32() * 1000.0);
            }
        }
        self.last_frame = Some(now);
    }

    fn report(&mut self, phase: Phase) {
        let intervals = std::mem::take(&mut self.intervals_ms);
        let work = std::mem::take(&mut self.work_ms);
        let frame_times = std::mem::take(&mut self.frame_times);
        let samples = std::mem::take(&mut self.sample_times);
        let budget = percentile(&intervals, 0.5) * 1.5;
        let drops: Vec<Instant> = intervals
            .iter()
            .zip(&frame_times)
            .filter(|(ms, _)| **ms > budget)
            .map(|(_, at)| *at)
            .collect();
        let near_sample = drops
            .iter()
            .filter(|drop| {
                samples
                    .iter()
                    .any(|s| drop.saturating_duration_since(*s) < Duration::from_millis(50))
            })
            .count();
        let total_secs: f32 = intervals.iter().sum::<f32>() / 1000.0;
        let fps = if total_secs > 0.0 {
            intervals.len() as f32 / total_secs
        } else {
            0.0
        };
        eprintln!(
            "{:<24} {:>6.1} fps | frame interval p50 {:>5.2} p99 {:>5.2} max {:>6.2} ms | \
             frame CPU p50 {:>5.2} p99 {:>5.2} max {:>6.2} ms | dropped {} ({} near a sample)",
            phase.label(),
            fps,
            percentile(&intervals, 0.50),
            percentile(&intervals, 0.99),
            percentile(&intervals, 1.0),
            percentile(&work, 0.50),
            percentile(&work, 0.99),
            percentile(&work, 1.0),
            drops.len(),
            near_sample,
        );
    }
}

fn percentile(values: &[f32], p: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let ix = ((sorted.len() - 1) as f32 * p).round() as usize;
    sorted[ix]
}

#[cfg(test)]
mod tests {
    use super::percentile;

    #[test]
    fn percentile_picks_nearest_rank() {
        let values = [5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(percentile(&values, 0.0), 1.0);
        assert_eq!(percentile(&values, 0.5), 3.0);
        assert_eq!(percentile(&values, 1.0), 5.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }
}
