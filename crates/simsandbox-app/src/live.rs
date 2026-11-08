//! The open model on its own thread (ADR 0035). A worker thread owns the
//! [`Session`]: it steps the model, paced by wall-clock time (ADR 0010),
//! and runs the window's commands between steps. The window never steps a Block: it sends commands, reads
//! the shared history to draw, and reads what the worker publishes.

use crate::pacing::Pacer;
use eframe::egui;
use simsandbox_core::block::MethodLabels;
use simsandbox_core::SimError;
use simsandbox_file::{FilterEntry, FusedGroup, ModelFile};
use simsandbox_session::run::{given_up, watch_studies, Progress};
use simsandbox_session::session::{ParamRow, Session, SharedHistory};
use simsandbox_views::{Available, History};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError, RwLockReadGuard};
use std::time::{Duration, Instant};

/// The worker steps in ticks of this length while running.
const TICK: Duration = Duration::from_micros(8_333);
/// Share of a tick spent stepping (ADR 0010's frame budget).
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
    pub filters: Vec<FilterEntry>,
    /// Probes besides the implicit one, as the file writes them (ADR 0062).
    pub probes: Vec<simsandbox_file::ProbeEntry>,
    /// The recorder: with it the model can be rewound (ADR 0064).
    pub recorder: Option<simsandbox_file::RecorderEntry>,
    /// What each probe has recorded, shared for drawing, by name.
    pub probe_histories: Vec<(String, simsandbox_session::session::SharedHistory)>,
    pub dt: f64,
    pub labels: Option<MethodLabels>,
    /// Sliders apply while dragged (ADR 0037).
    pub real_time: bool,
    /// Its studies have not run yet (ADR 0071): the worker runs them first.
    pub unevaluated: bool,
    /// Nothing in it runs in time: evaluated, not run (ADR 0072).
    pub timeless: bool,
    /// Only maps that never read time: it iterates, unpaced (ADR 0083).
    pub iterates: bool,
    /// Everything views can bind, recorded or not (ADR 0063).
    pub scalars: Vec<String>,
    /// The 2D grids cut filters can take.
    pub cut_inputs: Vec<String>,
    /// The outputs of filters with clocks of their own (ADR 0063).
    derived: Vec<String>,
    /// What those filters recorded, shared for drawing.
    pub derived_histories: Vec<(String, simsandbox_session::session::SharedHistory)>,
    coordinates: Vec<simsandbox_views::Coordinate>,
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
            filters: s.filters().to_vec(),
            probes: s.probe_entries(),
            recorder: s.recorder().cloned(),
            probe_histories: s
                .probe_names()
                .into_iter()
                .filter_map(|n| s.probe(n).map(|h| (n.to_string(), h)))
                .collect(),
            dt: s.dt,
            labels: s.labels(),
            real_time: s.real_time(),
            unevaluated: s.unevaluated(),
            timeless: s.timeless(),
            iterates: s.iterates(),
            scalars: s.bindable_scalars().to_vec(),
            cut_inputs: s.cut_inputs(),
            derived: s.derived_names().to_vec(),
            derived_histories: s.derived_histories(),
            coordinates: s.all_coordinates().to_vec(),
            grids,
            points,
            lines,
            block_types,
        }
    }

    /// What views can bind: everything a step yields, whether or not the
    /// history `h` records it now (ADR 0063).
    /// The grids views can bind, recorded or not.
    pub fn grids(&self) -> &[String] {
        &self.grids
    }

    pub fn available<'a>(&'a self, h: &'a History) -> Available<'a> {
        let _ = h;
        Available {
            scalars: &self.scalars,
            grids: &self.grids,
            points: &self.points,
            block_types: &self.block_types,
            lines: &self.lines,
            coordinates: &self.coordinates,
            derived: &self.derived,
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
    /// Recorded input lies ahead: running on replays it (ADR 0058).
    pub replaying: bool,
    /// An iterating model (ADR 0083): rounds run, whether rounds remain,
    /// and whether the last changed nothing.
    pub round: u64,
    pub iterating: bool,
    pub held_still: bool,
    /// What the recordings hold against the memory budget (ADR 0062
    /// step 5).
    pub memory: Option<simsandbox_session::session::MemoryUse>,
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
    /// The runs of the studies the worker is running (ADR 0071 step 2).
    progress: Arc<Progress>,
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
    pub fn start(mut session: Session, speed: f64, running: bool) -> Self {
        // A model with nothing in time is evaluated here after a change,
        // where a newer change can give a study up (ADR 0072).
        session.evaluate_later();
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

    /// The runs done and in all of the studies being run, while some are
    /// left (ADR 0071 step 2).
    pub fn study_runs(&self) -> Option<(usize, usize)> {
        self.shared
            .progress
            .runs()
            .filter(|(done, total)| done < total)
    }

    /// Give up the studies of the step in progress: the step is undone and
    /// runs again once the commands sent have run (ADR 0071 step 2).
    pub fn give_up_studies(&self) {
        if self.stepping_for().is_some() {
            self.shared.progress.give_up();
        }
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
        // A change to what a study reads, or Pause or Run, gives up the
        // study running: its results would be for older values.
        if key.is_some_and(|k| k.starts_with("param:") || k.starts_with("input:") || k == "running")
        {
            self.give_up_studies();
        }
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
    /// progress, and after the model is republished). The window waits, so
    /// a study running is given up first and runs again after `f`
    /// (ADR 0071).
    pub fn call<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Session, &mut Control) -> R + Send + 'static,
    ) -> Result<R, String> {
        self.give_up_studies();
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

impl Drop for Live {
    /// The worker stops after the step in progress, its studies given up.
    fn drop(&mut self) {
        self.give_up_studies();
    }
}

const STOPPED: &str = "the simulation thread stopped";

/// Why stepping stopped within a tick.
enum Stop {
    /// The file's `run.t_end` was reached (ADR 0013).
    EndReached,
    Failed(SimError),
    /// A study was given up for a newer change (ADR 0071 step 2): the step
    /// is undone, and runs again after the commands.
    GivenUp,
}

fn status(s: &Session, running: bool, achieved: f64, limited: bool) -> Status {
    Status {
        time: s.time(),
        history_range: s.history_range(),
        running,
        achieved,
        limited,
        replaying: s.replaying(),
        round: s.round(),
        iterating: s.rounds_pending(),
        held_still: s.held_still(),
        memory: Some(s.memory_use()),
    }
}

/// The worker's loop: commands, then a tick of stepping.
fn work(mut s: Session, shared: Arc<Shared>, rx: Receiver<Command>, mut c: Control) {
    watch_studies(Some(Arc::clone(&shared.progress)));
    let mut pacer = Pacer::new(c.speed);
    pacer.budget = TICK_BUDGET;
    let mut last = Instant::now();
    loop {
        let mut commands = Vec::new();
        // A model with nothing in time is never stepped (ADR 0072); one
        // that iterates steps while it has rounds to run (ADR 0083).
        let stepping = stepping_now(&s, &c);
        if !stepping && !s.unevaluated() {
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
            // A study given up in the last step is not given up here.
            shared.progress.reset();
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

        // Built without running its studies (the window opened it), or a
        // model with nothing in time changed: evaluate it first, as a step is run, with progress, given up for newer
        // commands and then run again (ADR 0071).
        if s.unevaluated() {
            shared.progress.reset();
            *lock(&shared.stepping) = Some(Instant::now());
            let r = s.evaluate();
            *lock(&shared.stepping) = None;
            match r {
                Err(e) if given_up(&e) => {}
                Err(e) => {
                    c.running = false;
                    c.error(format!("its studies did not run: {e}"));
                }
                Ok(()) => *lock(&shared.model) = Some(Arc::new(Model::of(&s))),
            }
            s.flush(false);
            *lock(&shared.status) = status(&s, c.running, pacer.achieved_speed(), false);
            lock(&shared.events).extend(std::mem::take(&mut c.events));
            shared.repaint();
            last = Instant::now();
            continue;
        }

        // The commands may have paused or run it.
        let stepping = stepping_now(&s, &c);
        let tick = Instant::now();
        let elapsed = tick.duration_since(last).as_secs_f64();
        last = tick;
        let mut recorded = false;
        if stepping && s.iterates() {
            // Rounds, unpaced, as many as the tick allows (ADR 0083).
            while s.rounds_pending() && tick.elapsed() < TICK_BUDGET {
                shared.progress.reset();
                *lock(&shared.stepping) = Some(Instant::now());
                let r = s.step();
                *lock(&shared.stepping) = None;
                match r {
                    Err(e) if given_up(&e) => break,
                    Err(e) => {
                        s.stop_rounds();
                        c.error(format!("stopped at round {}: {e}", s.round()));
                        break;
                    }
                    Ok(()) => recorded = true,
                }
                if s.take_end_reached() {
                    s.stop_rounds();
                }
            }
        } else if stepping {
            pacer.speed = c.speed;
            let dt = s.step_dt();
            let result = pacer.frame(
                elapsed,
                dt,
                || tick.elapsed(),
                || {
                    shared.progress.reset();
                    *lock(&shared.stepping) = Some(Instant::now());
                    let r = s.step();
                    *lock(&shared.stepping) = None;
                    match r {
                        Err(e) if given_up(&e) => return Err(Stop::GivenUp),
                        r => r.map_err(Stop::Failed)?,
                    }
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
                        "Paused at the end time t = {:.3} s (the file's, or its data's last sample); Run continues",
                        s.time()
                    ));
                }
                Err(Stop::GivenUp) => {}
                Err(Stop::Failed(e)) => {
                    c.running = false;
                    c.error(format!("stopped at t = {:.4}: {e}", s.time()));
                }
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
        if stepping && !s.iterates() {
            if let Some(rest) = TICK.checked_sub(tick.elapsed()) {
                std::thread::sleep(rest);
            }
        }
    }
}

/// Whether the worker steps now: a model in time while running, an
/// iterating one while it has rounds to run (ADR 0083), a model with
/// nothing in time never (ADR 0072).
fn stepping_now(s: &Session, c: &Control) -> bool {
    if s.iterates() {
        s.rounds_pending()
    } else {
        c.running && !s.timeless()
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
        let mut s = Session::load("l", text, &registry()).unwrap();
        // Rewinding needs a recorder (ADR 0064).
        s.set_recorder(Some(Default::default()));
        let live = Live::start(s, 200.0, true);
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
    fn a_change_while_a_study_runs_gives_it_up_and_runs_it_again() {
        // ADR 0071 step 2: the study's step is undone and run again with
        // the newer value; nothing older is recorded.
        let file = serde_json::json!({"format": 1,
            "models": {"decay": {"blocks": [
                {"name": "k", "type": "input.value", "params": {"y": 1.0}},
                {"name": "x", "type": "eq.equations", "method": {"name": "rk4"},
                 "equations": ["input k", "state x = 1", "dx/dt = -k*x"]}],
              "connections": [{"from": "k.y", "to": "x.k"}],
              "run": {"dt": 0.001, "t_end": 3.0}}},
            // A state of its own, so it runs in time (ADR 0072) and the
            // study runs within a step.
            "blocks": [{"name": "clock", "type": "eq.equations", "method": {"name": "euler"},
                 "equations": ["state s [s] = 0", "ds/dt = 1"]},
                {"name": "rate", "type": "input.value", "params": {"y": 1.0}},
                {"name": "s", "type": "study", "model": "decay",
                 "sample": {"design": "grid", "params": [{"param": "x.x0", "range": [1.0, 2.0], "points": 48}],
                            "reduce": {"half": "first_time(x.x < 0.5)"}}}],
            "connections": [{"from": "rate.y", "to": "s.k"}],
            "run": {"dt": 1.0}})
        .to_string();
        let half = |s: &Session| {
            let h = s.signals();
            let f = h.fields().iter().find(|f| f.name == "s.half").unwrap();
            f.frame(h.len() - 1).to_vec()
        };
        let live = Live::start(
            Session::load("decay", &file, &registry()).unwrap(),
            1.0,
            true,
        );
        wait_for(&live, 2);
        // A new rate: the next step runs the study again.
        live.send(Some("param:rate.y"), |s, _| {
            s.set_param_key("rate.y", 2.0).unwrap();
        });
        let start = Instant::now();
        while live.study_runs().is_none() {
            assert!(start.elapsed() < Duration::from_secs(20), "timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        // Another while it runs: given up, run again from the newer value.
        live.send(Some("param:rate.y"), |s, _| {
            s.set_param_key("rate.y", 4.0).unwrap();
        });
        let n = live.history().len();
        wait_for(&live, n + 1);
        live.set_running(false);
        live.sync();
        let mut direct = Session::load("decay", &file, &registry()).unwrap();
        direct.set_param_key("rate.y", 4.0).unwrap();
        let expected = half(&direct);
        let h = live.history();
        let f = h.fields().iter().find(|f| f.name == "s.half").unwrap();
        assert_eq!(f.frame(h.len() - 1), &expected[..]);
        // Every frame is the first rate's or the last's, none the second's.
        let first = (0.5f64.ln() / -1.0, 0.5f64.ln() / -4.0);
        for k in 0..h.len() {
            let x = f.frame(k)[0];
            assert!(
                (x - first.0).abs() < 1e-2 || (x - first.1).abs() < 1e-2,
                "frame {k}: {x}"
            );
        }
    }

    #[test]
    fn a_model_opened_without_its_studies_has_them_run_by_the_worker() {
        // ADR 0071: the window builds the model; the worker runs its
        // studies first, a change made meanwhile giving them up.
        let text = example("Resonance map");
        let s = simsandbox_session::study::without_running_studies(|| {
            Session::load("m", text, &registry()).unwrap()
        });
        assert!(s.unevaluated());
        let live = Live::start(s, 1.0, false);
        live.send(Some("param:m.y"), |s, _| {
            s.set_param_key("map.m", 2.0).unwrap();
        });
        live.sync();
        let start = Instant::now();
        // Polled from what the worker publishes: a call would give the
        // studies up.
        while live.model().unevaluated {
            assert!(start.elapsed() < Duration::from_secs(60), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        live.sync();
        let mut direct = Session::load("m", text, &registry()).unwrap();
        direct.set_param_key("map.m", 2.0).unwrap();
        let newest = |h: &History| {
            let f = h
                .fields()
                .iter()
                .find(|f| f.name == "map.spring_amplitude")
                .unwrap();
            f.frame(h.len() - 1).to_vec()
        };
        assert_eq!(newest(&live.history()), newest(&direct.signals()));
    }

    #[test]
    fn a_model_with_nothing_in_time_is_evaluated_not_run() {
        // ADR 0072: running, it stays at t = 0 with one sample; a change
        // evaluates it again.
        let text = example("Resonance curve");
        let live = Live::start(Session::load("c", text, &registry()).unwrap(), 1.0, true);
        std::thread::sleep(Duration::from_millis(200));
        live.send(Some("param:c.y"), |s, _| {
            s.set_param_key("curve.c", 0.05).unwrap();
        });
        let start = Instant::now();
        loop {
            let m = live.model();
            if !m.unevaluated
                && m.params
                    .iter()
                    .any(|r| r.key == "curve.c" && r.value == 0.05)
            {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        live.sync();
        assert!(live.model().timeless);
        assert_eq!(live.status().time, 0.0);
        assert_eq!(live.history().len(), 1);
        let mut direct = Session::load("c", text, &registry()).unwrap();
        direct.set_param_key("curve.c", 0.05).unwrap();
        let newest = |h: &History| {
            let f = h
                .fields()
                .iter()
                .find(|f| f.name == "curve.spring_amplitude")
                .unwrap();
            f.frame(h.len() - 1).to_vec()
        };
        assert_eq!(newest(&live.history()), newest(&direct.signals()));
    }

    #[test]
    fn a_model_that_iterates_runs_its_rounds_unpaced() {
        // ADR 0083: at 1× the search would take a second a round; it holds
        // still within its 40 rounds at once, and a change runs it again
        // from round 0 to the same values a direct run gives.
        let text = example("Searching");
        let live = Live::start(Session::load("s", text, &registry()).unwrap(), 1.0, false);
        let settled = |live: &Live| {
            let start = Instant::now();
            loop {
                let st = live.status();
                if !live.model().unevaluated && !st.iterating && st.round > 0 {
                    return st;
                }
                assert!(start.elapsed() < Duration::from_secs(20), "timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let start = Instant::now();
        let st = settled(&live);
        assert!(live.model().iterates);
        assert!(st.held_still && st.round < 40, "round {}", st.round);
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            start.elapsed()
        );
        live.send(Some("param:search.limit"), |s, _| {
            s.set_param_key("search.limit", 30.0).unwrap();
        });
        // The change restarts it; wait for it to hold still again.
        let begin = Instant::now();
        while live.status().round != 0 && !live.status().iterating {
            assert!(begin.elapsed() < Duration::from_secs(20), "timed out");
            std::thread::sleep(Duration::from_millis(1));
            if live
                .model()
                .params
                .iter()
                .any(|r| r.key == "search.limit" && r.value == 30.0)
            {
                break;
            }
        }
        live.sync();
        let st = settled(&live);
        let mut direct = Session::load("s", text, &registry()).unwrap();
        direct.set_param_key("search.limit", 30.0).unwrap();
        for _ in 0..st.round {
            direct.step().unwrap();
        }
        let last = |h: &History, n: &str| *values(h, n).last().unwrap();
        for n in ["search.kp", "search.ki", "search.best_score"] {
            assert_eq!(
                last(&live.history(), n).to_bits(),
                last(&direct.signals(), n).to_bits(),
                "{n}"
            );
        }
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
