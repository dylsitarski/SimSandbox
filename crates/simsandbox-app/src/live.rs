//! The open model on its own thread (ADR 0035). A worker thread owns the
//! [`Session`]: it steps the model, paced by wall-clock time (ADR 0010),
//! evaluates sweeps a slice at a time and runs the window's commands
//! between steps. The window never steps a Block: it sends commands, reads
//! the shared history to draw, and reads what the worker publishes.

use crate::pacing::Pacer;
use eframe::egui;
use simsandbox_core::block::MethodLabels;
use simsandbox_core::SimError;
use simsandbox_file::{FilterEntry, FusedGroup, ModelFile};
use simsandbox_session::session::{ParamRow, Session, SharedHistory, Sweep};
use simsandbox_views::{Available, History};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError, RwLockReadGuard};
use std::time::{Duration, Instant};

/// The worker steps in ticks of this length while running.
const TICK: Duration = Duration::from_micros(8_333);
/// Share of a tick spent stepping or sweeping (ADR 0010's frame budget).
const TICK_BUDGET: Duration = Duration::from_millis(7);

/// The model as the worker last published it: everything the window shows
/// besides the history and the run status. Republished after every batch of
/// commands.
pub struct Model {
    pub title: String,
    pub description: Option<String>,
    /// The file as loaded, with the controls edited since.
    pub file: ModelFile,
    pub groups: Vec<FusedGroup>,
    pub params: Vec<ParamRow>,
    pub sweep: Option<Arc<Sweep>>,
    pub filters: Vec<FilterEntry>,
    pub dt: f64,
    pub labels: Option<MethodLabels>,
    /// Sliders apply while dragged (ADR 0037).
    pub real_time: bool,
    grids: Vec<String>,
    points: Vec<String>,
    lines: Vec<String>,
    block_types: Vec<String>,
}

impl Model {
    fn of(s: &Session) -> Self {
        let [grids, points, lines, block_types] = s.bindable().map(<[String]>::to_vec);
        Self {
            title: s.title.clone(),
            description: s.description.clone(),
            file: s.file.clone(),
            groups: s.groups.clone(),
            params: s.params(),
            sweep: s.sweep_shared(),
            filters: s.filters().to_vec(),
            dt: s.dt,
            labels: s.labels(),
            real_time: s.real_time(),
            grids,
            points,
            lines,
            block_types,
        }
    }

    pub fn sweep(&self) -> Option<&Sweep> {
        self.sweep.as_deref()
    }

    /// What views can bind, with `h` the model's history.
    pub fn available<'a>(&'a self, h: &'a History) -> Available<'a> {
        Available {
            scalars: h.names(),
            grids: &self.grids,
            points: &self.points,
            block_types: &self.block_types,
            lines: &self.lines,
            coordinates: h.coordinates(),
            swept: self.sweep.as_ref().map_or(&[], |sw| sw.keys()),
        }
    }
}

/// How the run is going, updated every tick.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub time: f64,
    pub history_range: (f64, f64),
    pub running: bool,
    /// Simulated seconds per wall-clock second actually reached.
    pub achieved: f64,
    /// The tick budget cut stepping short: the model runs slower than asked.
    pub limited: bool,
    /// A sweep being evaluated: samples done and in all.
    pub sweep: Option<(usize, usize)>,
    /// Recorded input lies ahead: running on replays it (ADR 0058).
    pub replaying: bool,
}

/// Something for the log.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Info(String),
    Error(String),
}

/// The run settings a command may change, and the messages it leaves.
pub struct Control {
    pub running: bool,
    pub speed: f64,
    reset: bool,
    events: Vec<Event>,
}

impl Control {
    /// Forget accumulated time (after pausing, restarting or rewinding).
    pub fn reset_pacer(&mut self) {
        self.reset = true;
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.events.push(Event::Info(text.into()));
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.events.push(Event::Error(text.into()));
    }
}

/// Run on the worker; may return a reply, sent after the model is
/// republished.
type Job = Box<dyn FnOnce(&mut Session, &mut Control) -> Option<Reply> + Send>;
type Reply = Box<dyn FnOnce() + Send>;

struct Command {
    /// Of several pending commands with the same key, only the newest runs
    /// (a dragged slider sends many).
    key: Option<String>,
    job: Job,
}

#[derive(Default)]
struct Shared {
    model: Mutex<Option<Arc<Model>>>,
    status: Mutex<Status>,
    events: Mutex<Vec<Event>>,
    /// When the step in progress started.
    stepping: Mutex<Option<Instant>>,
    repaint: Mutex<Option<egui::Context>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// Ask the main window for a frame (it asks popped-out windows in
    /// turn). Named, because from this thread the current viewport could
    /// be any window's.
    fn repaint(&self) {
        if let Some(ctx) = lock(&self.repaint).as_ref() {
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }
}

/// The window's handle on a model running on its worker thread. Dropping it
/// stops the worker after the step in progress.
pub struct Live {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    history: SharedHistory,
}

impl Live {
    /// Start a worker for `session`, running at `speed` if `running`.
    pub fn start(session: Session, speed: f64, running: bool) -> Self {
        let shared = Arc::new(Shared::default());
        let history = session.shared_history();
        *lock(&shared.model) = Some(Arc::new(Model::of(&session)));
        *lock(&shared.status) = status(&session, running, speed, false);
        let (commands, rx) = mpsc::channel();
        let worker_shared = Arc::clone(&shared);
        let control = Control {
            running,
            speed,
            reset: false,
            events: Vec::new(),
        };
        std::thread::Builder::new()
            .name("simulation".into())
            .spawn(move || work(session, worker_shared, rx, control))
            .expect("the simulation thread starts");
        Self {
            shared,
            commands,
            history,
        }
    }

    /// The model as last published.
    pub fn model(&self) -> Arc<Model> {
        lock(&self.shared.model)
            .clone()
            .expect("published at start")
    }

    /// The recorded signals, for drawing; the worker waits only to append.
    pub fn history(&self) -> RwLockReadGuard<'_, History> {
        self.history.read()
    }

    pub fn status(&self) -> Status {
        lock(&self.shared.status).clone()
    }

    /// How long the step in progress has been running, if one is.
    pub fn stepping_for(&self) -> Option<Duration> {
        lock(&self.shared.stepping).map(|t| t.elapsed())
    }

    /// Messages for the log since the last call.
    pub fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut *lock(&self.shared.events))
    }

    /// Ask `ctx` to repaint whenever the worker records something.
    pub fn set_repaint(&self, ctx: &egui::Context) {
        let mut r = lock(&self.shared.repaint);
        if r.is_none() {
            *r = Some(ctx.clone());
        }
    }

    /// Run `f` on the worker between steps. A newer command with the same
    /// `key` replaces this one if it has not run yet.
    pub fn send(
        &self,
        key: Option<&str>,
        f: impl FnOnce(&mut Session, &mut Control) + Send + 'static,
    ) {
        let job: Job = Box::new(move |s, c| {
            f(s, c);
            None
        });
        let _ = self.commands.send(Command {
            key: key.map(str::to_string),
            job,
        });
    }

    /// Run `f` on the worker and wait for its result (after the step in
    /// progress, and after the model is republished).
    pub fn call<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Session, &mut Control) -> R + Send + 'static,
    ) -> Result<R, String> {
        let (tx, rx) = mpsc::channel();
        let job: Job = Box::new(move |s, c| {
            let r = f(s, c);
            Some(Box::new(move || {
                let _ = tx.send(r);
            }))
        });
        self.commands
            .send(Command { key: None, job })
            .map_err(|_| STOPPED.to_string())?;
        rx.recv().map_err(|_| STOPPED.to_string())
    }

    /// Wait until every command sent so far has run.
    #[cfg(test)]
    pub fn sync(&self) {
        let _ = self.call(|_, _| ());
    }

    pub fn set_running(&self, on: bool) {
        self.send(Some("running"), move |_, c| {
            c.running = on;
            c.reset_pacer();
        });
    }

    pub fn set_speed(&self, speed: f64) {
        self.send(Some("speed"), move |_, c| c.speed = speed);
    }
}

const STOPPED: &str = "the simulation thread stopped";

/// Why stepping stopped within a tick.
enum Stop {
    /// The file's `run.t_end` was reached (ADR 0013).
    EndReached,
    Failed(SimError),
}

fn status(s: &Session, running: bool, achieved: f64, limited: bool) -> Status {
    Status {
        time: s.time(),
        history_range: s.history_range(),
        running,
        achieved,
        limited,
        sweep: s.resampling(),
        replaying: s.replaying(),
    }
}

/// The worker's loop: commands, then a tick of stepping or sweeping.
fn work(mut s: Session, shared: Arc<Shared>, rx: Receiver<Command>, mut c: Control) {
    let mut pacer = Pacer::new(c.speed);
    pacer.budget = TICK_BUDGET;
    let mut last = Instant::now();
    loop {
        let stepping = c.running && s.sweep().is_none();
        let busy = stepping || s.resampling().is_some();
        let mut commands = Vec::new();
        if !busy {
            // Idle: everything recorded is visible, then wait for a command.
            s.flush(true);
            *lock(&shared.status) = status(&s, false, pacer.achieved_speed(), false);
            shared.repaint();
            match rx.recv() {
                Ok(cmd) => commands.push(cmd),
                Err(_) => return,
            }
            last = Instant::now();
        }
        loop {
            match rx.try_recv() {
                Ok(cmd) => commands.push(cmd),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if !commands.is_empty() {
            let mut replies = Vec::new();
            for cmd in newest_of_each_key(commands) {
                replies.extend((cmd.job)(&mut s, &mut c));
            }
            if std::mem::take(&mut c.reset) {
                pacer.reset();
            }
            *lock(&shared.model) = Some(Arc::new(Model::of(&s)));
            *lock(&shared.status) = status(
                &s,
                c.running,
                pacer.achieved_speed(),
                c.running && pacer.is_limited(),
            );
            for r in replies {
                r();
            }
        }

        let tick = Instant::now();
        let elapsed = tick.duration_since(last).as_secs_f64();
        last = tick;
        let mut recorded = false;
        if c.running && s.sweep().is_none() {
            pacer.speed = c.speed;
            let dt = s.dt;
            let result = pacer.frame(
                elapsed,
                dt,
                || tick.elapsed(),
                || {
                    *lock(&shared.stepping) = Some(Instant::now());
                    let r = s.step();
                    *lock(&shared.stepping) = None;
                    r.map_err(Stop::Failed)?;
                    if s.take_end_reached() {
                        return Err(Stop::EndReached);
                    }
                    Ok(())
                },
            );
            match result {
                Ok(report) => recorded = report.steps > 0,
                Err(Stop::EndReached) => {
                    c.running = false;
                    recorded = true;
                    c.info(format!(
                        "Paused at the file's end time t = {:.3} s; Run continues",
                        s.time()
                    ));
                }
                Err(Stop::Failed(e)) => {
                    c.running = false;
                    c.error(format!("stopped at t = {:.4}: {e}", s.time()));
                }
            }
        }
        if s.resampling().is_some() {
            recorded = true;
            if s.resume_resample(Some(TICK_BUDGET)) {
                // Finished: whether it is real-time is now known.
                *lock(&shared.model) = Some(Arc::new(Model::of(&s)));
            }
        }
        s.flush(false);
        *lock(&shared.status) = status(
            &s,
            c.running,
            pacer.achieved_speed(),
            c.running && pacer.is_limited(),
        );
        let events = std::mem::take(&mut c.events);
        let news = !events.is_empty();
        lock(&shared.events).extend(events);
        if recorded || news {
            shared.repaint();
        }
        if c.running && s.sweep().is_none() {
            if let Some(rest) = TICK.checked_sub(tick.elapsed()) {
                std::thread::sleep(rest);
            }
        }
    }
}

/// Commands in order, without those a newer command with the same key
/// replaces.
fn newest_of_each_key(commands: Vec<Command>) -> Vec<Command> {
    let mut kept: Vec<Command> = Vec::with_capacity(commands.len());
    for cmd in commands {
        if let Some(k) = &cmd.key {
            kept.retain(|c| c.key.as_ref() != Some(k));
        }
        kept.push(cmd);
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use simsandbox_session::examples::{registry, EXAMPLES};

    fn example(part: &str) -> &'static str {
        EXAMPLES
            .iter()
            .find(|e| e.name.contains(part))
            .unwrap()
            .text
    }

    fn values(h: &History, name: &str) -> Vec<f64> {
        h.values(h.resolve(name).unwrap())
    }

    /// Wait until the worker has recorded at least `n` samples.
    fn wait_for(live: &Live, n: usize) {
        let start = Instant::now();
        while live.history().len() < n {
            assert!(start.elapsed() < Duration::from_secs(20), "timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn the_worker_records_exactly_what_direct_stepping_does() {
        let text = example("Lorenz");
        let mut direct = Session::load("l", text, &registry()).unwrap();
        for _ in 0..400 {
            direct.step().unwrap();
        }
        let reference = values(&direct.signals(), "lorenz.xyz[0]");

        // Fast, so the worker gets there quickly; it records whole steps.
        let live = Live::start(
            Session::load_deferred("l", text, &registry()).unwrap(),
            200.0,
            true,
        );
        wait_for(&live, 401);
        live.set_running(false);
        live.sync();
        let recorded = values(&live.history(), "lorenz.xyz[0]");
        assert_eq!(&recorded[..401], &reference[..]);

        // Rewind and run on: the same numbers again (ADR 0035).
        let t = live
            .call(|s, _| {
                s.rewind_to(1.05);
                s.time()
            })
            .unwrap();
        assert!((t - 1.0).abs() < 1e-9, "{t}");
        assert_eq!(live.history().len(), 101);
        live.set_running(true);
        wait_for(&live, 401);
        live.set_running(false);
        live.sync();
        assert_eq!(
            &values(&live.history(), "lorenz.xyz[0]")[..401],
            &reference[..]
        );
        assert!(!live.status().running);
    }

    #[test]
    fn sweeps_are_evaluated_in_slices_and_restarted_by_changes() {
        let text = example("Resonance map");
        let full = Session::load("m", text, &registry()).unwrap();
        let reference = values(&full.signals(), "spring.amplitude");

        let live = Live::start(
            Session::load_deferred("m", text, &registry()).unwrap(),
            1.0,
            false,
        );
        // A change while it runs starts it again with the new value.
        let id = live.call(|s, _| s.ids["spring"]).unwrap();
        live.send(Some("param:spring.m"), move |s, _| {
            s.set_param(id, "m", 2.0).unwrap();
        });
        live.send(Some("param:spring.m"), move |s, _| {
            s.set_param(id, "m", 1.0).unwrap();
        });
        let start = Instant::now();
        while live.status().sweep.is_some() || live.history().len() < reference.len() {
            assert!(start.elapsed() < Duration::from_secs(60), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        live.sync();
        assert_eq!(values(&live.history(), "spring.amplitude"), reference);
        let m = live.model();
        assert_eq!(
            m.params.iter().find(|r| r.key == "spring.m").unwrap().value,
            1.0
        );
        // Its grids are complete too.
        let h = live.history();
        let grid = h
            .fields()
            .iter()
            .find(|f| f.name == "grid.spring.amplitude")
            .unwrap();
        assert_eq!(grid.frame(0), &reference[..]);
    }

    #[test]
    fn of_several_pending_commands_with_one_key_only_the_newest_runs() {
        let run = std::sync::Arc::new(Mutex::new(Vec::new()));
        let job = |k: Option<&str>, n: u32| {
            let run = run.clone();
            Command {
                key: k.map(str::to_string),
                job: Box::new(move |_, _| {
                    lock(&run).push(n);
                    None
                }),
            }
        };
        let kept = newest_of_each_key(vec![
            job(Some("a"), 1),
            job(None, 2),
            job(Some("a"), 3),
            job(Some("b"), 4),
            job(None, 5),
        ]);
        let keys: Vec<Option<String>> = kept.iter().map(|c| c.key.clone()).collect();
        assert_eq!(keys.len(), 4);
        assert_eq!(keys[0], None);
        assert_eq!(keys.iter().filter(|k| k.as_deref() == Some("a")).count(), 1);
    }
}
