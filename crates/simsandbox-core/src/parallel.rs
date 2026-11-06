//! Running independent units of work on several cores (ADR 0066 step 6).
//!
//! A unit is work that touches nothing another unit touches, such as the
//! blocks of one clock between instants: run apart or together, in any
//! order, it gives the same bits. Threads cost time to start, so they are
//! used only when the units are worth it, as their past steps measured.
//! A [`Pool`] keeps threads for a whole run, so that work which moves
//! from instant to instant need not start them each time.

use std::sync::mpsc;
use std::thread::JoinHandle;
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

/// The same for a [`Pool`]'s threads, already started: waking one and
/// moving work to it costs some microseconds.
const WORTH_POOLED: f64 = 2e-5;

/// The time each unit's work took, smoothed over its recent steps.
#[derive(Debug, Clone, Default)]
pub struct Costs {
    seconds: Vec<f64>,
    worth: f64,
}

impl Costs {
    /// For units run on threads started for them ([`each`]).
    pub fn new(units: usize) -> Self {
        Self {
            seconds: vec![0.0; units],
            worth: WORTH,
        }
    }

    /// For units run on a [`Pool`]'s threads.
    pub fn pooled(units: usize) -> Self {
        Self {
            worth: WORTH_POOLED,
            ..Self::new(units)
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
                c[1..].iter().sum::<f64>() >= self.worth
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

/// Work for a pool's thread.
type Job = Box<dyn FnOnce() + Send>;

/// Threads kept for a whole run (ADR 0066 step 6), each waiting for work:
/// waking one costs microseconds where starting one costs tens. Work goes
/// to them owned, not borrowed, and comes back when done.
pub struct Pool {
    senders: Vec<mpsc::Sender<Job>>,
    handles: Vec<JoinHandle<()>>,
}

impl Pool {
    /// A pool of `workers` threads besides the caller's.
    pub fn new(workers: usize) -> Self {
        let (senders, handles) = (0..workers)
            .map(|_| {
                let (tx, rx) = mpsc::channel::<Job>();
                let handle = std::thread::spawn(move || {
                    while let Some(job) = wait(&rx) {
                        job();
                    }
                });
                (tx, handle)
            })
            .unzip();
        Self { senders, handles }
    }

    /// The threads work runs on, the caller's included.
    pub fn size(&self) -> usize {
        self.senders.len() + 1
    }

    /// Run `f` on each item, dealt in turn to the pool's threads and the
    /// calling thread; results in the items' order.
    pub fn each<T: Send + 'static, R: Send + 'static>(
        &self,
        items: Vec<T>,
        f: fn(T) -> R,
    ) -> Vec<R> {
        let n = items.len();
        let k = self.size().min(n).max(1);
        let mut shares: Vec<Vec<(usize, T)>> = (0..k).map(|_| Vec::new()).collect();
        for (i, item) in items.into_iter().enumerate() {
            shares[i % k].push((i, item));
        }
        let mine = shares.remove(0);
        let (tx, rx) = mpsc::channel::<Vec<(usize, R)>>();
        let mut here: Vec<(usize, R)> = Vec::new();
        for (share, worker) in shares.into_iter().zip(&self.senders) {
            let back = tx.clone();
            let job: Job = Box::new(move || {
                let done: Vec<(usize, R)> = share.into_iter().map(|(i, t)| (i, f(t))).collect();
                // The caller waits for it; if it has gone, so has the use.
                let _ = back.send(done);
            });
            match worker.send(job) {
                Ok(()) => {}
                // A thread that panicked earlier is gone: its share runs here.
                Err(mpsc::SendError(job)) => job(),
            }
        }
        drop(tx);
        here.extend(mine.into_iter().map(|(i, t)| (i, f(t))));
        let mut out: Vec<Option<R>> = (0..n).map(|_| None).collect();
        for (i, r) in here {
            out[i] = Some(r);
        }
        while let Some(done) = wait(&rx) {
            for (i, r) in done {
                out[i] = Some(r);
            }
        }
        out.into_iter()
            .map(|r| r.expect("a unit of work panicked"))
            .collect()
    }
}

/// How long a thread waiting for work or results keeps checking before it
/// sleeps: work that comes again within it starts without the cost of
/// waking a sleeping thread, and a run between instants keeps the cores
/// busy only this long.
const SPIN: Duration = Duration::from_micros(50);

/// The next message on `rx`, checked for [`SPIN`] then waited for; `None`
/// once every sender is gone.
fn wait<T>(rx: &mpsc::Receiver<T>) -> Option<T> {
    let start = std::time::Instant::now();
    loop {
        match rx.try_recv() {
            Ok(v) => return Some(v),
            Err(mpsc::TryRecvError::Disconnected) => return None,
            Err(mpsc::TryRecvError::Empty) if start.elapsed() < SPIN => std::hint::spin_loop(),
            Err(mpsc::TryRecvError::Empty) => return rx.recv().ok(),
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        // Closing the channels ends the threads' loops.
        self.senders.clear();
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
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
    fn a_pool_gives_results_in_order_and_survives_many_uses() {
        let pool = Pool::new(3);
        for n in [0, 1, 2, 5, 37] {
            let items: Vec<u64> = (0..n).collect();
            let want: Vec<u64> = items.iter().map(|x| x * x).collect();
            assert_eq!(pool.each(items, |x| x * x), want);
        }
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
