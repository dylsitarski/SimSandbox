//! Running independent units of work on several cores (ADR 0066 step 6).
//!
//! A unit is work that touches nothing another unit touches, such as the
//! blocks of one clock between instants: run apart or together, in any
//! order, it gives the same bits. Threads cost time to start, so they are
//! used only when the units are worth it, as their past steps measured.

use std::time::Duration;

/// Whether units run on several threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Parallel {
    /// When the units are worth it, by their past cost.
    #[default]
    Auto,
    /// Always one after another.
    Never,
    /// On several threads whenever there are two units or more (tests).
    Always,
}

/// Work besides the largest unit below this runs on one thread: starting
/// threads costs some tens of microseconds.
const WORTH: f64 = 2e-4;

/// The time each unit's work took, smoothed over its recent steps.
#[derive(Debug, Clone, Default)]
pub struct Costs {
    seconds: Vec<f64>,
}

impl Costs {
    pub fn new(units: usize) -> Self {
        Self {
            seconds: vec![0.0; units],
        }
    }

    /// Record that unit `u` took `took`.
    pub fn record(&mut self, u: usize, took: Duration) {
        if u >= self.seconds.len() {
            self.seconds.resize(u + 1, 0.0);
        }
        let s = took.as_secs_f64();
        let c = &mut self.seconds[u];
        *c = if *c == 0.0 { s } else { 0.7 * *c + 0.3 * s };
    }

    /// Whether running `units` on several threads is worth it under
    /// `mode`: what the others cost besides the largest exceeds starting
    /// threads.
    pub fn worth(&self, mode: Parallel, units: &[usize]) -> bool {
        if units.len() < 2 || threads() < 2 {
            return false;
        }
        match mode {
            Parallel::Never => false,
            Parallel::Always => true,
            Parallel::Auto => {
                let mut c: Vec<f64> = units
                    .iter()
                    .map(|&u| self.seconds.get(u).copied().unwrap_or(0.0))
                    .collect();
                c.sort_by(|a, b| b.total_cmp(a));
                c[1..].iter().sum::<f64>() >= WORTH
            }
        }
    }
}

/// The cores available, asked once: asking reads the system's settings
/// (on Linux, control-group files), too slow for every instant.
pub fn threads() -> usize {
    static THREADS: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *THREADS.get_or_init(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
}

/// Run `f` on each item, on several threads when `parallel`, one after
/// another otherwise; results in the items' order. Items are dealt to the
/// threads in turn; the calling thread takes a share.
pub fn each<T: Send, R: Send>(items: Vec<T>, parallel: bool, f: impl Fn(T) -> R + Sync) -> Vec<R> {
    let n = items.len();
    let k = if parallel { threads().min(n) } else { 1 };
    if k <= 1 {
        return items.into_iter().map(f).collect();
    }
    let mut shares: Vec<Vec<(usize, T)>> = (0..k).map(|_| Vec::new()).collect();
    for (i, item) in items.into_iter().enumerate() {
        shares[i % k].push((i, item));
    }
    let f = &f;
    let mut out: Vec<Option<R>> = (0..n).map(|_| None).collect();
    let mine = shares.pop().unwrap_or_default();
    let done: Vec<Vec<(usize, R)>> = std::thread::scope(|s| {
        let handles: Vec<_> = shares
            .into_iter()
            .map(|share| {
                s.spawn(move || {
                    share
                        .into_iter()
                        .map(|(i, t)| (i, f(t)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let here: Vec<(usize, R)> = mine.into_iter().map(|(i, t)| (i, f(t))).collect();
        let mut all: Vec<Vec<(usize, R)>> = handles
            .into_iter()
            .map(|h| h.join().expect("a unit of work panicked"))
            .collect();
        all.push(here);
        all
    });
    for (i, r) in done.into_iter().flatten() {
        out[i] = Some(r);
    }
    out.into_iter()
        .map(|r| r.expect("every item ran"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_come_back_in_order_either_way() {
        let items: Vec<u64> = (0..37).collect();
        let square = |x: u64| x * x;
        assert_eq!(
            each(items.clone(), true, square),
            each(items, false, square)
        );
    }

    #[test]
    fn threads_are_worth_it_only_for_enough_work() {
        let mut c = Costs::new(3);
        c.record(0, Duration::from_millis(5));
        c.record(1, Duration::from_micros(10));
        c.record(2, Duration::from_micros(10));
        // One heavy unit and two light ones: not worth it.
        assert!(!c.worth(Parallel::Auto, &[0, 1, 2]));
        c.record(1, Duration::from_millis(5));
        c.record(1, Duration::from_millis(5));
        assert_eq!(c.worth(Parallel::Auto, &[0, 1]), threads() > 1);
        assert!(!c.worth(Parallel::Never, &[0, 1]));
        assert!(!c.worth(Parallel::Always, &[0]));
    }
}
