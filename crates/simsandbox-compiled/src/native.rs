//! Native code for typed equations ([ADR 0089](../../../docs/decisions/0089-native-code-for-typed-equations.md)):
//! Rust generated from a program's phase views, built by the user's own
//! toolchain into a library, kept in a cache by a hash of what it was built
//! from and the processor it was built for, loaded, and checked bit for
//! bit before use. While it builds, and
//! whenever it cannot be built or loaded or differs, the phases run on
//! Cranelift's code.
//!
//! What it does is set by [`set_mode`], or `SIMSANDBOX_NATIVE`: `off`;
//! `cache`, use libraries already built (the default); `build`, also build
//! the missing ones in the background (the app's default).
//!
//! Loading a library runs its code, so the cache is trusted like any
//! folder of programs ([ADR 0096](../../../docs/decisions/0096-the-native-cache.md)):
//! SimSandbox makes it for its user alone, and on Unix does not load from
//! a cache that other users can write to. The cache keeps the libraries
//! used most recently, [`KEPT_LIBRARIES`] at most.

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

/// Libraries kept in the cache; the least recently used go when a build
/// adds one beyond this.
pub const KEPT_LIBRARIES: usize = 200;

/// The processor native code is built for (`target-cpu=native`): its
/// architecture and the instruction sets it has, part of a library's
/// name, so a cache carried to another machine (a shared home folder, a
/// container image) is not run where an instruction is missing.
fn processor() -> String {
    let mut s = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);
    #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
    {
        macro_rules! features {
            ($($f:tt),*) => {
                $(if std::arch::is_x86_feature_detected!($f) {
                    s.push(' ');
                    s.push_str($f);
                })*
            };
        }
        features!(
            "sse3",
            "ssse3",
            "sse4.1",
            "sse4.2",
            "popcnt",
            "avx",
            "avx2",
            "fma",
            "f16c",
            "bmi1",
            "bmi2",
            "lzcnt",
            "movbe",
            "adx",
            "aes",
            "pclmulqdq",
            "sha",
            "avx512f",
            "avx512dq",
            "avx512cd",
            "avx512bw",
            "avx512vl",
            "avx512ifma",
            "avx512vbmi",
            "avx512vbmi2",
            "avx512vnni",
            "avx512bitalg",
            "avx512vpopcntdq",
            "gfni",
            "vaes",
            "vpclmulqdq"
        );
    }
    #[cfg(target_arch = "aarch64")]
    {
        macro_rules! features {
            ($($f:tt),*) => {
                $(if std::arch::is_aarch64_feature_detected!($f) {
                    s.push(' ');
                    s.push_str($f);
                })*
            };
        }
        features!(
            "neon", "fp16", "sve", "sve2", "dotprod", "rdm", "lse", "aes", "sha2", "sha3", "crc"
        );
    }
    s
}

/// Why libraries in `dir` (a cache's `native` folder) must not be loaded:
/// on Unix, when the folder, the folder libraries are built into or the
/// library itself can be written by other users, who could then have put
/// their own code there. `None` when it may be loaded. Elsewhere nothing
/// is checked: the cache is in the user's own folders.
fn untrusted(dir: &std::path::Path, file: &std::path::Path) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let release = dir.join("target").join("release");
        for path in [dir, &dir.join("target"), &release, file] {
            match std::fs::metadata(path) {
                Ok(m) if m.mode() & 0o022 != 0 => {
                    return Some(format!(
                        "{} can be written by other users, so nothing is loaded from this \
                         cache: delete the cache folder, or set SIMSANDBOX_CACHE to a folder \
                         of your own",
                        path.display()
                    ));
                }
                Ok(_) => {}
                Err(e) => return Some(format!("{}: {e}", path.display())),
            }
        }
    }
    let _ = (dir, file);
    None
}

/// Make a folder of the cache and those above it, for the user alone.
fn private_dir(path: &std::path::Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(path)
}

/// The libraries in a cache, by name, least recently used first.
fn cached(dir: &std::path::Path) -> Vec<(String, std::time::SystemTime)> {
    let mut all: Vec<(String, std::time::SystemTime)> =
        std::fs::read_dir(dir.join("target").join("release"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let file = e.file_name().to_string_lossy().into_owned();
                let stem = file.split('.').next()?;
                let name = stem.strip_prefix("lib").unwrap_or(stem);
                (name.starts_with("ssn_") && file == lib_file(name))
                    .then(|| Some((name.to_string(), e.metadata().ok()?.modified().ok()?)))?
            })
            .collect();
    all.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    all
}

/// Remove the least recently used libraries of a cache beyond `keep`,
/// with what they were built from, sparing those named in `in_use`.
/// Returns the names removed.
fn evict(dir: &std::path::Path, keep: usize, in_use: &[String]) -> Vec<String> {
    let all = cached(dir);
    let excess = all.len().saturating_sub(keep);
    let gone: Vec<String> = all
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| !in_use.contains(name))
        .take(excess)
        .collect();
    let release = dir.join("target").join("release");
    for name in &gone {
        let _ = std::fs::remove_dir_all(dir.join("crates").join(name));
        // The library, and cargo's files for it (dependency lists, the
        // copy it links from, its fingerprint).
        for folder in [
            release.clone(),
            release.join("deps"),
            release.join(".fingerprint"),
        ] {
            for e in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
                if e.file_name().to_string_lossy().contains(name.as_str()) {
                    let path = e.path();
                    let _ = if path.is_dir() {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
                }
            }
        }
    }
    gone
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
    let hash = fnv(format!(
        "{source}\n{rustc}\n{}\n{}",
        env!("CARGO_PKG_VERSION"),
        processor()
    )
    .as_bytes());
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
        let _ = lib.loaded.set(load(&dir, &file, views.len()));
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

fn load(dir: &std::path::Path, file: &std::path::Path, phases: usize) -> Result<Loaded, String> {
    if let Some(why) = untrusted(dir, file) {
        return Err(why);
    }
    // Used now: the cache keeps the libraries used most recently.
    if let Ok(f) = std::fs::File::options().append(true).open(file) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
    // SAFETY: loading runs whatever the file holds, so this is sound only
    // for a file SimSandbox built: one in the user's own cache, which
    // `untrusted` has refused if anyone else could have written it, under
    // a name that is the hash of its source, compiler and processor. Such
    // a library is the Rust `source` generates, with no code that runs on
    // loading and its functions of `Entry`'s signature.
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
                let result = build(&job).and_then(|file| load(&job.dir, &file, job.phases));
                let _ = job.lib.loaded.set(result);
                // Room for it: the least recently used go, never one
                // this process holds.
                let in_use: Vec<String> = libraries()
                    .lock()
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default();
                evict(&job.dir, KEPT_LIBRARIES, &in_use);
            }
        });
        Mutex::new(tx)
    })
}

/// Build a library's crate in the cache; its file.
fn build(job: &Job) -> Result<PathBuf, String> {
    let krate = job.dir.join("crates").join(&job.name);
    let target = job.dir.join("target");
    // The cache's folders are the user's alone, whatever the umask;
    // cargo makes the rest inside them.
    for folder in [krate.join("src"), target.join("release")] {
        private_dir(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    let manifest = format!(
        "[package]\nname = \"{}\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n\
         [lib]\ncrate-type = [\"cdylib\"]\npath = \"src/lib.rs\"\n\n[workspace]\n\n\
         [profile.release]\nopt-level = 3\ncodegen-units = 1\npanic = \"abort\"\ndebug = false\n",
        job.name
    );
    std::fs::write(krate.join("Cargo.toml"), manifest).map_err(|e| e.to_string())?;
    std::fs::write(krate.join("src").join("lib.rs"), &job.source).map_err(|e| e.to_string())?;
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
    let file = target.join("release").join(lib_file(&job.name));
    // Written by the user alone, whatever the umask it was linked under.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(&file) {
            let mode = m.permissions().mode() & !0o022;
            let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode));
        }
    }
    Ok(file)
}

/// A phase that runs Cranelift's code until its library is loaded, then,
/// once a call has given the same bits both ways, the library's.
pub struct Switching {
    jit: Arc<dyn PhaseKernel>,
    lib: Arc<Library>,
    phase: usize,
    kept: Vec<bool>,
    /// What the phase was generated for; every call of the library's
    /// code is checked against it.
    shape: crate::Shape,
    /// 0 not checked, 1 the same, 2 different.
    checked: AtomicU8,
}

impl Switching {
    pub(crate) fn new(
        jit: Arc<dyn PhaseKernel>,
        lib: Arc<Library>,
        phase: usize,
        (view, shape): (&PhaseView, crate::Shape),
    ) -> Self {
        Self {
            jit,
            lib,
            phase,
            kept: view.kept.clone(),
            shape,
            checked: AtomicU8::new(0),
        }
    }

    fn call(
        &self,
        f: Entry,
        lo: usize,
        written: &mut [&mut [f64]],
        read: &[&[f64]],
        scalars: &[f64],
    ) {
        let len = self.shape.check(lo, written, read, scalars);
        let w: Vec<*mut f64> = written.iter_mut().map(|w| w.as_mut_ptr()).collect();
        let r: Vec<*const f64> = read.iter().map(|r| r.as_ptr()).collect();
        // SAFETY: as for Cranelift's code: the generated function takes
        // `len` cells of each of the view's written arrays, the view's
        // read arrays over all its cells and its scalars, and
        // `Shape::check` has just made sure these are that many and that
        // long.
        #[allow(unsafe_code)]
        unsafe {
            f(lo, len, w.as_ptr(), r.as_ptr(), scalars.as_ptr())
        }
    }
}

impl PhaseKernel for Switching {
    /// `native` once its library is loaded (checked against Cranelift's
    /// code on its first call); `compiled` and why not, otherwise.
    fn describe(&self) -> String {
        if self.checked.load(Ordering::Relaxed) == 2 {
            return "compiled (native code differed)".into();
        }
        match self.lib.loaded.get() {
            None => "compiled (native code building)".into(),
            Some(Ok(_)) => "native".into(),
            Some(Err(e)) => format!("compiled (native code not built: {e})"),
        }
    }

    fn run(&self, lo: usize, written: &mut [&mut [f64]], read: &[&[f64]], scalars: &[f64]) {
        let f = match self.lib.loaded.get() {
            Some(Ok(l)) => l.fns[self.phase],
            _ => return self.jit.run(lo, written, read, scalars),
        };
        match self.checked.load(Ordering::Relaxed) {
            1 => self.call(f, lo, written, read, scalars),
            2 => self.jit.run(lo, written, read, scalars),
            _ => {
                // Both ways, the library's on copies: the same bits or it
                // is not used.
                let mut copies: Vec<Vec<f64>> = written.iter().map(|w| w.to_vec()).collect();
                self.jit.run(lo, written, read, scalars);
                {
                    let mut views: Vec<&mut [f64]> =
                        copies.iter_mut().map(|c| c.as_mut_slice()).collect();
                    self.call(f, lo, &mut views, read, scalars);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for a test, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("simsandbox-native-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            private_dir(&dir.join("target").join("release")).unwrap();
            Self(dir)
        }

        /// A file standing for library `name`, last used `age` seconds
        /// ago, with the files a build leaves around it.
        fn library(&self, name: &str, age: u64) -> PathBuf {
            let release = self.0.join("target").join("release");
            let file = release.join(lib_file(name));
            std::fs::write(&file, b"not a library").unwrap();
            let then = std::time::SystemTime::now() - std::time::Duration::from_secs(age);
            std::fs::File::options()
                .append(true)
                .open(&file)
                .unwrap()
                .set_modified(then)
                .unwrap();
            private_dir(&self.0.join("crates").join(name).join("src")).unwrap();
            private_dir(&release.join("deps")).unwrap();
            std::fs::write(release.join("deps").join(format!("{name}-0123.d")), b"").unwrap();
            std::fs::write(release.join(format!("lib{name}.d")), b"").unwrap();
            file
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_cache_keeps_the_libraries_used_most_recently() {
        let cache = Scratch::new("evict");
        for (name, age) in [
            ("ssn_a", 400),
            ("ssn_b", 300),
            ("ssn_c", 200),
            ("ssn_d", 100),
        ] {
            cache.library(name, age);
        }
        // Something else in the folder is left alone.
        let other = cache.0.join("target").join("release").join("other.txt");
        std::fs::write(&other, b"").unwrap();
        let names = |dir: &std::path::Path| -> Vec<String> {
            cached(dir).into_iter().map(|(n, _)| n).collect()
        };
        assert_eq!(names(&cache.0), ["ssn_a", "ssn_b", "ssn_c", "ssn_d"]);
        assert!(evict(&cache.0, 4, &[]).is_empty());
        // Two too many: the oldest go, but not one in use.
        let gone = evict(&cache.0, 2, &["ssn_a".to_string()]);
        assert_eq!(gone, ["ssn_b", "ssn_c"]);
        assert_eq!(names(&cache.0), ["ssn_a", "ssn_d"]);
        let release = cache.0.join("target").join("release");
        assert!(!cache.0.join("crates").join("ssn_b").exists());
        assert!(!release.join("deps").join("ssn_b-0123.d").exists());
        assert!(!release.join("libssn_b.d").exists());
        assert!(cache.0.join("crates").join("ssn_a").exists());
        assert!(release.join("deps").join("ssn_d-0123.d").exists());
        assert!(other.exists());
    }

    /// A cache other users can write to is not loaded from: the refusal
    /// comes before the file is opened as a library (this one is not a
    /// library at all).
    #[cfg(unix)]
    #[test]
    fn a_cache_others_can_write_to_is_not_loaded() {
        use std::os::unix::fs::PermissionsExt;
        let cache = Scratch::new("trust");
        let file = cache.library("ssn_x", 0);
        let set = |path: &std::path::Path, mode: u32| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        set(&file, 0o644);
        assert_eq!(untrusted(&cache.0, &file), None);
        let release = cache.0.join("target").join("release");
        for (path, open, shut) in [
            (file.as_path(), 0o666, 0o644),
            (release.as_path(), 0o777, 0o700),
            (cache.0.as_path(), 0o775, 0o700),
        ] {
            set(path, open);
            let why = untrusted(&cache.0, &file).expect("refused");
            assert!(why.contains("written by other users"), "{why}");
            let e = load(&cache.0, &file, 1).err().expect("not loaded");
            assert_eq!(e, why);
            set(path, shut);
        }
        assert_eq!(untrusted(&cache.0, &file), None);
        // A library that is not there is not trusted either.
        assert!(untrusted(&cache.0, &release.join(lib_file("ssn_none"))).is_some());
    }

    #[test]
    fn a_librarys_name_holds_the_processor() {
        let p = processor();
        assert!(p.starts_with(std::env::consts::ARCH), "{p}");
        // Built for other instructions, another name.
        let name = |cpu: &str| fnv(format!("source\nrustc\n0.0.1\n{cpu}").as_bytes());
        assert_ne!(name(&p), name(&format!("{p} avx512f")));
    }
}
