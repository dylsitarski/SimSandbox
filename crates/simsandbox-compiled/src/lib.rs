//! Typed equations' phases of cells compiled by Cranelift into the process
//! ([ADR 0084](../../../docs/decisions/0084-where-typed-equations-stop.md)
//! point 6). A phase's view ([`PhaseView`]) becomes machine code doing the
//! same operations in the same order, two cells at a time inside lines, so
//! the results are the interpreter's bits; the interpreter stays the
//! reference and the fallback.
//!
//! Compiled code is kept by what it compiles: a model built again (an
//! edit undone, a study's runs, a restart) reuses it. Parameters and inputs
//! are read at every call, so moving a slider compiles nothing.
//! `SIMSANDBOX_INTERPRET=1` turns compiling off, for comparing.

// Calling compiled code and keeping its memory: here only.
#[allow(unsafe_code)]
mod jit;
mod routes;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use simsandbox_expr::{PhaseKernel, PhaseView, Program};

/// Compiled phases kept, at most.
const KEPT: usize = 512;

/// Compiled code by the phase it was compiled from.
fn kept() -> &'static Mutex<HashMap<String, Arc<dyn PhaseKernel>>> {
    static KEPT_CODE: OnceLock<Mutex<HashMap<String, Arc<dyn PhaseKernel>>>> = OnceLock::new();
    KEPT_CODE.get_or_init(Default::default)
}

/// Whether phases are compiled (not when `SIMSANDBOX_INTERPRET` is set).
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SIMSANDBOX_INTERPRET").is_none())
}

/// The code for a phase: kept, or compiled now; `None` if it cannot be.
pub fn kernel(view: &PhaseView) -> Option<Arc<dyn PhaseKernel>> {
    let key = format!("{view:?}");
    if let Some(k) = kept().lock().ok()?.get(&key) {
        return Some(Arc::clone(k));
    }
    let k = jit::compile(view)?;
    let mut map = kept().lock().ok()?;
    if map.len() >= KEPT {
        map.clear();
    }
    map.insert(key, Arc::clone(&k));
    Some(k)
}

/// Compile `program`'s phases of cells, if compiling is on. Returns how
/// many run compiled.
pub fn compile(program: &mut Program) -> usize {
    if !enabled() {
        return 0;
    }
    program.compile_phases(&mut |view| kernel(view))
}
