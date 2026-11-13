//! Native code for typed equations ([ADR 0089](../../../docs/decisions/0089-native-code-for-typed-equations.md)):
//! Rust generated from a program's phase views, built by the user's own
//! toolchain into a library, kept in a cache by a hash of what it was built
//! from, loaded, and checked bit for bit before use. While it builds, and
//! whenever it cannot be built or loaded or differs, the phases run on
//! Cranelift's code.
//!
//! What it does is set by [`set_mode`], or `SIMSANDBOX_NATIVE`: `off`;
//! `cache`, use libraries already built (the default); `build`, also build
//! the missing ones in the background (the app's default).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use simsandbox_expr::{Arg, EdgeView, Func, Op, OpKind, PhaseKernel, PhaseView};

/// What native code does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Not used.
    Off,
    /// Libraries already in the cache are used; none are built.
    Cache,
    /// Missing libraries are built in the background when a toolchain is
    /// found.
    Build,
}

static MODE: AtomicU8 = AtomicU8::new(u8::MAX);

fn mode_code(m: Mode) -> u8 {
    match m {
        Mode::Off => 0,
        Mode::Cache => 1,
        Mode::Build => 2,
    }
}

/// Set what native code does, unless `SIMSANDBOX_NATIVE` says otherwise.
pub fn set_mode(m: Mode) {
    if from_env().is_none() {
        MODE.store(mode_code(m), Ordering::Relaxed);
    }
}

fn from_env() -> Option<Mode> {
    match std::env::var("SIMSANDBOX_NATIVE").ok()?.as_str() {
        "off" | "0" => Some(Mode::Off),
        "cache" => Some(Mode::Cache),
        "build" | "1" => Some(Mode::Build),
        _ => None,
    }
}

/// What native code does now.
pub fn mode() -> Mode {
    if let Some(m) = from_env() {
        return m;
    }
    match MODE.load(Ordering::Relaxed) {
        0 => Mode::Off,
        2 => Mode::Build,
        _ => Mode::Cache,
    }
}

/// Whether `cargo` runs here.
fn toolchain() -> Option<&'static str> {
    static FOUND: OnceLock<Option<String>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let out = std::process::Command::new("rustc")
                .arg("--version")
                .output()
                .ok()?;
            let cargo = std::process::Command::new("cargo")
                .arg("--version")
                .output()
                .ok()?;
            (out.status.success() && cargo.status.success())
                .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        })
        .as_deref()
}

/// Where libraries are kept: `SIMSANDBOX_CACHE`, else the platform's cache
/// folder, under `simsandbox/native`.
fn cache_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("SIMSANDBOX_CACHE") {
        Some(d) => PathBuf::from(d),
        None => {
            let home = |v: &str| std::env::var_os(v).map(PathBuf::from);
            if cfg!(windows) {
                home("LOCALAPPDATA")?
            } else if cfg!(target_os = "macos") {
                home("HOME")?.join("Library").join("Caches")
            } else {
                home("XDG_CACHE_HOME").or_else(|| home("HOME").map(|h| h.join(".cache")))?
            }
        }
    };
    Some(base.join("simsandbox").join("native"))
}

/// FNV-1a: a hash that stays the same across builds and machines.
fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

type Entry = unsafe extern "C" fn(usize, usize, *const *mut f64, *const *const f64, *const f64);

/// A loaded library: its phases' functions.
struct Loaded {
    _lib: libloading::Library,
    fns: Vec<Entry>,
}

/// Where a program's library stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Building,
    Ready,
    Failed(String),
}

/// A program's library: being built, loaded, or not to be had.
pub struct Library {
    name: String,
    loaded: OnceLock<Result<Loaded, String>>,
    /// Phases that differed from Cranelift's code, and those checked.
    differed: AtomicU8,
}

impl Library {
    fn state(&self) -> State {
        match self.loaded.get() {
            None => State::Building,
            Some(Ok(_)) if self.differed.load(Ordering::Relaxed) > 0 => {
                State::Failed("differs from the interpreter's bits".into())
            }
            Some(Ok(_)) => State::Ready,
            Some(Err(e)) => State::Failed(e.clone()),
        }
    }
}

/// Every library asked for in this process, by name.
fn libraries() -> &'static Mutex<HashMap<String, Arc<Library>>> {
    static ALL: OnceLock<Mutex<HashMap<String, Arc<Library>>>> = OnceLock::new();
    ALL.get_or_init(Default::default)
}

/// Each library's state, for status lines: (in use, building, failed).
pub fn summary() -> (usize, usize, usize) {
    let all = libraries()
        .lock()
        .map(|m| m.values().map(|l| l.state()).collect::<Vec<_>>());
    let all = all.unwrap_or_default();
    let count = |f: &dyn Fn(&State) -> bool| all.iter().filter(|s| f(s)).count();
    (
        count(&|s| *s == State::Ready),
        count(&|s| *s == State::Building),
        count(&|s| matches!(s, State::Failed(_))),
    )
}

/// Wait until no library is being built, at most `timeout`.
pub fn wait(timeout: std::time::Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        if summary().1 == 0 {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// The library for a program's phases, if native code is on and it can be
/// had: loaded from the cache, or (in [`Mode::Build`]) being built.
pub fn library(views: &[PhaseView]) -> Option<Arc<Library>> {
    let mode = mode();
    if mode == Mode::Off || views.is_empty() {
        return None;
    }
    let rustc = toolchain().unwrap_or("none");
    let source = source(views);
    let hash = fnv(format!("{source}\n{rustc}\n{}", env!("CARGO_PKG_VERSION")).as_bytes());
    let name = format!("ssn_{hash:016x}");
    if let Some(l) = libraries().lock().ok()?.get(&name) {
        return Some(Arc::clone(l));
    }
    let dir = cache_dir()?;
    let file = dir.join("target").join("release").join(lib_file(&name));
    let lib = Arc::new(Library {
        name: name.clone(),
        loaded: OnceLock::new(),
        differed: AtomicU8::new(0),
    });
    if file.exists() {
        let _ = lib.loaded.set(load(&file, views.len()));
    } else if mode == Mode::Build && toolchain().is_some() {
        builder()
            .lock()
            .ok()?
            .send(Job {
                dir,
                name: name.clone(),
                source,
                phases: views.len(),
                lib: Arc::clone(&lib),
            })
            .ok()?;
    } else {
        return None;
    }
    libraries().lock().ok()?.insert(name, Arc::clone(&lib));
    Some(lib)
}

fn lib_file(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{name}.dylib")
    } else {
        format!("lib{name}.so")
    }
}

fn load(file: &std::path::Path, phases: usize) -> Result<Loaded, String> {
    // SAFETY: the library was built from Rust SimSandbox generated
    // (`source`), its functions with `Entry`'s signature; nothing runs
    // when it loads.
    #[allow(unsafe_code)]
    unsafe {
        let lib = libloading::Library::new(file).map_err(|e| e.to_string())?;
        let fns = (0..phases)
            .map(|k| {
                lib.get::<Entry>(format!("phase_{k}").as_bytes())
                    .map(|f| *f)
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Loaded { _lib: lib, fns })
    }
}

struct Job {
    dir: PathBuf,
    name: String,
    source: String,
    phases: usize,
    lib: Arc<Library>,
}

/// The one thread that builds, a library at a time.
fn builder() -> &'static Mutex<Sender<Job>> {
    static TX: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::spawn(move || {
            for job in rx {
                let result = build(&job).and_then(|file| load(&file, job.phases));
                let _ = job.lib.loaded.set(result);
            }
        });
        Mutex::new(tx)
    })
}

/// Build a library's crate in the cache; its file.
fn build(job: &Job) -> Result<PathBuf, String> {
    let krate = job.dir.join("crates").join(&job.name);
    std::fs::create_dir_all(krate.join("src")).map_err(|e| e.to_string())?;
    let manifest = format!(
        "[package]\nname = \"{}\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n\
         [lib]\ncrate-type = [\"cdylib\"]\npath = \"src/lib.rs\"\n\n[workspace]\n\n\
         [profile.release]\nopt-level = 3\ncodegen-units = 1\npanic = \"abort\"\ndebug = false\n",
        job.name
    );
    std::fs::write(krate.join("Cargo.toml"), manifest).map_err(|e| e.to_string())?;
    std::fs::write(krate.join("src").join("lib.rs"), &job.source).map_err(|e| e.to_string())?;
    let target = job.dir.join("target");
    let out = std::process::Command::new("cargo")
        .args(["build", "--release", "--quiet", "--offline"])
        .current_dir(&krate)
        .env("CARGO_TARGET_DIR", &target)
        // This machine's vector instructions; Rust never fuses `a*b + c`,
        // so the bits are the same.
        .env("RUSTFLAGS", "-C target-cpu=native")
        // Run from within cargo (tests, CI), these would win over ours.
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "cargo failed: {}",
            err.lines().take(5).collect::<Vec<_>>().join(" ")
        ));
    }
    Ok(target.join("release").join(lib_file(&job.name)))
}

/// A phase that runs Cranelift's code until its library is loaded, then,
/// once a call has given the same bits both ways, the library's.
pub struct Switching {
    jit: Arc<dyn PhaseKernel>,
    lib: Arc<Library>,
    phase: usize,
    kept: Vec<bool>,
    /// 0 not checked, 1 the same, 2 different.
    checked: AtomicU8,
}

impl Switching {
    pub fn new(
        jit: Arc<dyn PhaseKernel>,
        lib: Arc<Library>,
        phase: usize,
        view: &PhaseView,
    ) -> Self {
        Self {
            jit,
            lib,
            phase,
            kept: view.kept.clone(),
            checked: AtomicU8::new(0),
        }
    }

    fn call(f: Entry, lo: usize, written: &mut [&mut [f64]], read: &[&[f64]], scalars: &[f64]) {
        let len = written.first().map_or(0, |w| w.len());
        let w: Vec<*mut f64> = written.iter_mut().map(|w| w.as_mut_ptr()).collect();
        let r: Vec<*const f64> = read.iter().map(|r| r.as_ptr()).collect();
        // SAFETY: as for Cranelift's code: `len` cells of each written
        // array, read arrays within their length, the scalars it was
        // generated for.
        #[allow(unsafe_code)]
        unsafe {
            f(lo, len, w.as_ptr(), r.as_ptr(), scalars.as_ptr())
        }
    }
}

impl PhaseKernel for Switching {
    fn run(&self, lo: usize, written: &mut [&mut [f64]], read: &[&[f64]], scalars: &[f64]) {
        let f = match self.lib.loaded.get() {
            Some(Ok(l)) => l.fns[self.phase],
            _ => return self.jit.run(lo, written, read, scalars),
        };
        match self.checked.load(Ordering::Relaxed) {
            1 => Self::call(f, lo, written, read, scalars),
            2 => self.jit.run(lo, written, read, scalars),
            _ => {
                // Both ways, the library's on copies: the same bits or it
                // is not used.
                let mut copies: Vec<Vec<f64>> = written.iter().map(|w| w.to_vec()).collect();
                self.jit.run(lo, written, read, scalars);
                {
                    let mut views: Vec<&mut [f64]> =
                        copies.iter_mut().map(|c| c.as_mut_slice()).collect();
                    Self::call(f, lo, &mut views, read, scalars);
                }
                let same = written
                    .iter()
                    .zip(&copies)
                    .zip(&self.kept)
                    .all(|((w, c), &kept)| {
                        !kept || w.iter().zip(c).all(|(a, b)| a.to_bits() == b.to_bits())
                    });
                self.checked
                    .store(if same { 1 } else { 2 }, Ordering::Relaxed);
                if !same {
                    self.lib.differed.fetch_add(1, Ordering::Relaxed);
                    eprintln!(
                        "native code {}: phase {} differs from the interpreter's bits; not used",
                        self.lib.name, self.phase
                    );
                }
            }
        }
    }
}

/// The library's source: one `extern "C"` function per phase, doing what
/// its view says in the same order (the rules of [`PhaseView`]), cell by
/// cell, runs of cells inside lines in tight loops LLVM vectorises.
pub fn source(views: &[PhaseView]) -> String {
    let mut s = String::from(
        "// Generated by SimSandbox from typed equations (ADR 0089). Do not edit.\n\
         #![allow(clippy::all, unused, non_snake_case, unused_unsafe)]\n\n",
    );
    for (n, v) in views.iter().enumerate() {
        phase(&mut s, n, v);
    }
    s
}

fn lines_of(view: &PhaseView) -> Vec<(usize, usize, bool)> {
    let mut lines: Vec<(usize, usize, bool)> = Vec::new();
    for op in &view.ops {
        let (l, edged) = match op.kind {
            OpKind::Axis { stride, count } => ((stride, count), false),
            OpKind::Stencil { stride, count, .. } | OpKind::Shift { stride, count, .. } => {
                ((stride, count), true)
            }
            _ => continue,
        };
        match lines.iter_mut().find(|x| (x.0, x.1) == l) {
            Some(x) => x.2 |= edged,
            None => lines.push((l.0, l.1, edged)),
        }
    }
    lines
}

fn phase(s: &mut String, n: usize, view: &PhaseView) {
    let lines = lines_of(view);
    let _ = writeln!(
        s,
        "#[no_mangle]\npub unsafe extern \"C\" fn phase_{n}(lo: usize, len: usize, w: *const *mut f64, r: *const *const f64, s: *const f64) {{"
    );
    for k in 0..view.written {
        let _ = writeln!(
            s,
            "    let w{k}: &mut [f64] = std::slice::from_raw_parts_mut(*w.add({k}), len);"
        );
    }
    for k in 0..view.read {
        let _ = writeln!(
            s,
            "    let r{k}: &[f64] = std::slice::from_raw_parts(*r.add({k}), {});",
            view.cells
        );
    }
    for k in 0..view.scalars {
        let _ = writeln!(s, "    let s{k}: f64 = *s.add({k});");
    }
    for (o, op) in view.ops.iter().enumerate() {
        if let (OpKind::Stencil { second, .. }, Arg::Scalar(h)) = (op.kind, op.b) {
            let d = if second {
                format!("s{h} * s{h}")
            } else {
                format!("2.0 * s{h}")
            };
            let _ = writeln!(s, "    let inv{o}: f64 = 1.0 / ({d});");
        }
    }
    for (l, &(st, c, _)) in lines.iter().enumerate() {
        let _ = writeln!(s, "    let mut within{l}: usize = lo % {st};\n    let mut j{l}: usize = (lo / {st}) % {c};");
    }
    s.push_str("    let mut i: usize = 0;\n    while i < len {\n");
    // At an end of a line a stencil or shift walks?
    let ends: Vec<String> = lines
        .iter()
        .enumerate()
        .filter(|(_, x)| x.2)
        .map(|(l, &(_, c, _))| format!("j{l} == 0 || j{l} == {}", c - 1))
        .collect();
    if !ends.is_empty() {
        let _ = writeln!(
            s,
            "        if {} {{\n            let k = lo + i;",
            ends.join(" || ")
        );
        let js: Vec<String> = (0..lines.len()).map(|l| format!("j{l}")).collect();
        cell(s, view, &lines, &js, false, "            ");
        advance(s, &lines, "1", "            ");
        s.push_str("            i += 1;\n            continue;\n        }\n");
    }
    // A run: as many cells as are left before any line's place changes.
    s.push_str("        let mut count = len - i;\n");
    for (l, &(st, c, edged)) in lines.iter().enumerate() {
        let left = if st == 1 {
            format!("{} - j{l}", c - usize::from(edged))
        } else {
            format!("{st} - within{l}")
        };
        let _ = writeln!(s, "        count = count.min({left});");
    }
    s.push_str(
        "        for q in 0..count {\n            let i = i + q;\n            let k = lo + i;\n",
    );
    let js: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(l, &(st, _, _))| {
            if st == 1 {
                format!("(j{l} + q)")
            } else {
                format!("j{l}")
            }
        })
        .collect();
    cell(s, view, &lines, &js, true, "            ");
    s.push_str("        }\n");
    advance(s, &lines, "count", "        ");
    s.push_str("        i += count;\n    }\n}\n\n");
}

/// Move the lines' counters on by `m` cells (no further than the next
/// change of place along a line of stride above 1).
fn advance(s: &mut String, lines: &[(usize, usize, bool)], m: &str, pad: &str) {
    for (l, &(st, c, _)) in lines.iter().enumerate() {
        if st == 1 {
            let _ = writeln!(s, "{pad}j{l} += {m};\n{pad}if j{l} == {c} {{ j{l} = 0; }}");
        } else {
            let _ = writeln!(
                s,
                "{pad}within{l} += {m};\n{pad}if within{l} == {st} {{ within{l} = 0; j{l} += 1; if j{l} == {c} {{ j{l} = 0; }} }}"
            );
        }
    }
}

/// One cell's operations (`i`, `k` in scope; `js` each line's place),
/// then the stores of what is kept.
fn cell(
    s: &mut String,
    view: &PhaseView,
    lines: &[(usize, usize, bool)],
    js: &[String],
    fast: bool,
    pad: &str,
) {
    let mut set = vec![false; view.written];
    let arg = |a: Arg, set: &[bool]| match a {
        Arg::Written(w) if set[w] => format!("c{w}"),
        Arg::Written(w) => format!("(*w{w}.get_unchecked(i))"),
        Arg::Read(r) => format!("(*r{r}.get_unchecked(k))"),
        Arg::Scalar(x) => format!("s{x}"),
    };
    for (o, op) in view.ops.iter().enumerate() {
        let (x, y) = (arg(op.a, &set), arg(op.b, &set));
        let line = |stride: usize, count: usize| {
            lines
                .iter()
                .position(|l| (l.0, l.1) == (stride, count))
                .expect("a line")
        };
        let value = match op.kind {
            OpKind::Copy => x,
            OpKind::Neg => format!("-{x}"),
            OpKind::Bin(b) => match b {
                Op::Add => format!("{x} + {y}"),
                Op::Sub => format!("{x} - {y}"),
                Op::Mul => format!("{x} * {y}"),
                Op::Div => format!("{x} / {y}"),
                Op::Pow => format!("{x}.powf({y})"),
                Op::Lt => format!("if {x} < {y} {{ 1.0 }} else {{ 0.0 }}"),
                Op::Le => format!("if {x} <= {y} {{ 1.0 }} else {{ 0.0 }}"),
                Op::Gt => format!("if {x} > {y} {{ 1.0 }} else {{ 0.0 }}"),
                Op::Ge => format!("if {x} >= {y} {{ 1.0 }} else {{ 0.0 }}"),
            },
            OpKind::Call1(f) => func(f, &x, "0.0"),
            OpKind::Call2(f) => func(f, &x, &y),
            OpKind::Axis { stride, count } => {
                format!("{x} + (({} as f64) + 0.5) * {y}", js[line(stride, count)])
            }
            OpKind::Stencil {
                stride,
                count,
                second,
            }
            | OpKind::Shift {
                stride,
                count,
                up: second,
            } => {
                let Arg::Read(r) = op.a else {
                    unreachable!("a stencil reads a whole array")
                };
                let stencil = matches!(op.kind, OpKind::Stencil { .. });
                let at = |e: &str| format!("(*r{r}.get_unchecked({e}))");
                let (below, above) = if fast {
                    (at(&format!("k - {stride}")), at(&format!("k + {stride}")))
                } else {
                    let j = &js[line(stride, count)];
                    let span = (count - 1) * stride;
                    let beyond = |e: EdgeView, far: String| match e {
                        EdgeView::Value(v) if stencil => format!("2.0 * s{v} - {}", at("k")),
                        EdgeView::Value(v) => format!("s{v}"),
                        EdgeView::Gradient(g) => {
                            let g = g.map_or("0.0".to_string(), |g| format!("s{g}"));
                            format!("{} + {g} * {y}", at("k"))
                        }
                        EdgeView::Periodic => far,
                    };
                    (
                        format!(
                            "(if {j} == 0 {{ {} }} else {{ {} }})",
                            beyond(op.edges[0], at(&format!("k + {span}"))),
                            at(&format!("k - {stride}"))
                        ),
                        format!(
                            "(if {j} == {} {{ {} }} else {{ {} }})",
                            count - 1,
                            beyond(op.edges[1], at(&format!("k - {span}"))),
                            at(&format!("k + {stride}"))
                        ),
                    )
                };
                match (stencil, second) {
                    (true, true) => format!("({below} - 2.0 * {} + {above}) * inv{o}", at("k")),
                    (true, false) => format!("({above} - {below}) * inv{o}"),
                    (false, up) => (if up { above } else { below }).to_string(),
                }
            }
        };
        let _ = writeln!(s, "{pad}let c{}: f64 = {value};", op.d);
        set[op.d] = true;
    }
    for (w, &done) in set.iter().enumerate() {
        if done && view.kept[w] {
            let _ = writeln!(s, "{pad}*w{w}.get_unchecked_mut(i) = c{w};");
        }
    }
}

/// A function as [`Func::apply`] computes it.
fn func(f: Func, x: &str, y: &str) -> String {
    match f {
        Func::Sin => format!("{x}.sin()"),
        Func::Cos => format!("{x}.cos()"),
        Func::Tan => format!("{x}.tan()"),
        Func::Asin => format!("{x}.asin()"),
        Func::Acos => format!("{x}.acos()"),
        Func::Atan => format!("{x}.atan()"),
        Func::Atan2 => format!("{x}.atan2({y})"),
        Func::Sinh => format!("{x}.sinh()"),
        Func::Cosh => format!("{x}.cosh()"),
        Func::Tanh => format!("{x}.tanh()"),
        Func::Exp => format!("{x}.exp()"),
        Func::Ln => format!("{x}.ln()"),
        Func::Log10 => format!("{x}.log10()"),
        Func::Sqrt => format!("{x}.sqrt()"),
        Func::Abs => format!("{x}.abs()"),
        Func::Sign => format!("{{ let a: f64 = {x}; if a > 0.0 {{ 1.0 }} else if a < 0.0 {{ -1.0 }} else {{ 0.0 }} }}"),
        Func::Floor => format!("{x}.floor()"),
        Func::Ceil => format!("{x}.ceil()"),
        Func::Min => format!("{x}.min({y})"),
        Func::Max => format!("{x}.max({y})"),
        Func::Pow => format!("{x}.powf({y})"),
    }
}
