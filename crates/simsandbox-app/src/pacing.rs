//! Wall-clock pacing of the live loop (ADR 0010).
//!
//! Each frame adds `elapsed wall time × speed` to an accumulator and steps
//! while it holds at least one `dt`. Stepping stops when the frame's work
//! budget is used; the leftover backlog is dropped, so a slow model runs in
//! slow motion instead of falling ever further behind and freezing the UI.

use std::time::Duration;

/// Share of each frame the simulation may use (ADR 0010: start with 8 ms).
pub const DEFAULT_BUDGET: Duration = Duration::from_millis(8);

/// Wall time over which the achieved speed is measured.
const MEASURE_WINDOW: f64 = 0.5;

pub struct Pacer {
    /// Requested simulated seconds per wall-clock second; 0 is paused.
    pub speed: f64,
    pub budget: Duration,
    accumulator: f64,
    achieved: f64,
    window_sim: f64,
    window_wall: f64,
    limited: bool,
}

/// What one frame did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameReport {
    pub steps: usize,
    /// The work budget cut stepping short and the backlog was dropped.
    pub limited: bool,
}

impl Pacer {
    pub fn new(speed: f64) -> Self {
        Self {
            speed,
            budget: DEFAULT_BUDGET,
            accumulator: 0.0,
            achieved: speed,
            window_sim: 0.0,
            window_wall: 0.0,
            limited: false,
        }
    }

    /// Run one frame. `wall_elapsed` is the wall time since the previous
    /// frame in seconds; `work_elapsed` reports the time spent stepping so far
    /// this frame; `step` advances the simulation by `dt`.
    pub fn frame<E>(
        &mut self,
        wall_elapsed: f64,
        dt: f64,
        mut work_elapsed: impl FnMut() -> Duration,
        mut step: impl FnMut() -> Result<(), E>,
    ) -> Result<FrameReport, E> {
        let mut report = FrameReport {
            steps: 0,
            limited: false,
        };
        if self.speed > 0.0 && dt > 0.0 && wall_elapsed > 0.0 {
            self.accumulator += wall_elapsed * self.speed;
            // A tiny tolerance so accumulated rounding never skips a step.
            while self.accumulator >= dt * (1.0 - 1e-9) {
                if work_elapsed() >= self.budget {
                    report.limited = true;
                    self.accumulator = 0.0;
                    break;
                }
                step()?;
                self.accumulator -= dt;
                report.steps += 1;
            }
            self.window_sim += report.steps as f64 * dt;
            self.window_wall += wall_elapsed;
            if self.window_wall >= MEASURE_WINDOW {
                self.achieved = self.window_sim / self.window_wall;
                self.window_sim = 0.0;
                self.window_wall = 0.0;
            }
        }
        self.limited = report.limited;
        Ok(report)
    }

    /// Simulated seconds per wall-clock second actually reached, measured
    /// over the last half second.
    pub fn achieved_speed(&self) -> f64 {
        self.achieved
    }

    /// Whether the last frame was cut short by the work budget.
    pub fn is_limited(&self) -> bool {
        self.limited
    }

    /// Forget accumulated time, e.g. after pausing, rewinding or loading.
    pub fn reset(&mut self) {
        self.accumulator = 0.0;
        self.achieved = self.speed;
        self.window_sim = 0.0;
        self.window_wall = 0.0;
        self.limited = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::convert::Infallible;

    /// Run `seconds` of wall time at `fps`; each step costs `cost`.
    fn run(pacer: &mut Pacer, fps: f64, seconds: f64, dt: f64, cost: Duration) -> (usize, usize) {
        let mut steps = 0;
        let mut limited_frames = 0;
        for _ in 0..(fps * seconds).round() as usize {
            let work = Cell::new(Duration::ZERO);
            let r = pacer
                .frame(
                    1.0 / fps,
                    dt,
                    || work.get(),
                    || {
                        work.set(work.get() + cost);
                        Ok::<_, Infallible>(())
                    },
                )
                .unwrap();
            steps += r.steps;
            limited_frames += r.limited as usize;
        }
        (steps, limited_frames)
    }

    #[test]
    fn playback_speed_does_not_depend_on_the_refresh_rate() {
        for fps in [30.0, 60.0, 144.0] {
            let mut p = Pacer::new(1.0);
            let (steps, limited) = run(&mut p, fps, 10.0, 0.01, Duration::ZERO);
            assert!((999..=1000).contains(&steps), "{fps} fps: {steps} steps");
            assert_eq!(limited, 0);
            assert!((p.achieved_speed() - 1.0).abs() < 0.05);
        }
    }

    #[test]
    fn speed_scales_simulated_time_and_zero_pauses() {
        let mut p = Pacer::new(10.0);
        let (steps, _) = run(&mut p, 60.0, 1.0, 0.01, Duration::ZERO);
        assert!((999..=1000).contains(&steps), "{steps}");
        p.speed = 0.0;
        assert_eq!(run(&mut p, 60.0, 1.0, 0.01, Duration::ZERO).0, 0);
    }

    #[test]
    fn slow_models_run_in_slow_motion_without_a_backlog() {
        // 5 ms per step and an 8 ms budget allow 2 steps per frame, so 60 fps
        // reaches 120 steps/s = 1.2 simulated s/s of the 10 requested.
        let mut p = Pacer::new(10.0);
        let (steps, limited) = run(&mut p, 60.0, 2.0, 0.01, Duration::from_millis(5));
        assert_eq!(steps, 240);
        assert_eq!(limited, 120);
        assert!(p.is_limited());
        assert!(
            (p.achieved_speed() - 1.2).abs() < 0.05,
            "{}",
            p.achieved_speed()
        );

        // Back to a cheap model: no burst from dropped backlog.
        let (steps, _) = run(&mut p, 60.0, 1.0 / 60.0, 0.01, Duration::ZERO);
        assert!(steps <= 17, "{steps}");
    }

    #[test]
    fn step_errors_stop_the_frame() {
        let mut p = Pacer::new(1.0);
        let r = p.frame(0.1, 0.01, || Duration::ZERO, || Err("boom"));
        assert_eq!(r, Err("boom"));
    }
}
