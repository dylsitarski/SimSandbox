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
pub mod native;
mod routes;
pub mod vector_state;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use simsandbox_expr::{Arg, OpKind, PhaseKernel, PhaseView, Program};

/// What a phase's code was compiled for: the sizes it takes for granted.
/// Compiled code reads and writes through raw pointers without looking at
/// lengths, so every call is checked against these first
/// ([`Shape::check`]): a caller's mistake is a panic naming it, never a
/// read or write outside an array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shape {
    cells: usize,
    written: usize,
    read: usize,
    scalars: usize,
}

impl Shape {
    /// The shape of a view, or `None` for a view code must not be made
    /// from: an operation naming an array or scalar the view does not
    /// have, or walking lines that do not fill its cells (a neighbour or
    /// a far end would lie outside the array).
    pub(crate) fn of(view: &PhaseView) -> Option<Self> {
        let arg = |a: Arg| match a {
            Arg::Written(k) => k < view.written,
            Arg::Read(k) => k < view.read,
            Arg::Scalar(k) => k < view.scalars,
        };
        let line = |stride: usize, count: usize| {
            stride > 0
                && count > 0
                && stride
                    .checked_mul(count)
                    .is_some_and(|span| view.cells.is_multiple_of(span))
        };
        let edge = |e: simsandbox_expr::EdgeView| match e {
            simsandbox_expr::EdgeView::Value(k) | simsandbox_expr::EdgeView::Gradient(Some(k)) => {
                k < view.scalars
            }
            _ => true,
        };
        let sound = view.kept.len() == view.written
            && view.ops.iter().all(|op| {
                op.d < view.written
                    && arg(op.a)
                    && arg(op.b)
                    && match op.kind {
                        OpKind::Axis { stride, count } => line(stride, count),
                        OpKind::Stencil { stride, count, .. }
                        | OpKind::Shift { stride, count, .. } => {
                            line(stride, count)
                                && matches!(op.a, Arg::Read(_))
                                && op.edges.into_iter().all(edge)
                        }
                        _ => true,
                    }
            });
        sound.then_some(Self {
            cells: view.cells,
            written: view.written,
            read: view.read,
            scalars: view.scalars,
        })
    }

    /// Check a call's arguments against what the code was compiled for;
    /// the number of cells to compute. Panics on a mismatch: the code
    /// would read or write outside the arrays.
    pub(crate) fn check(
        &self,
        lo: usize,
        written: &[&mut [f64]],
        read: &[&[f64]],
        scalars: &[f64],
    ) -> usize {
        let len = written.first().map_or(0, |w| w.len());
        assert!(
            written.len() == self.written && written.iter().all(|w| w.len() == len),
            "compiled phase: {} written arrays of {len} cells each expected",
            self.written
        );
        assert!(
            read.len() == self.read && read.iter().all(|r| r.len() == self.cells),
            "compiled phase: {} read arrays of {} cells each expected",
            self.read,
            self.cells
        );
        assert!(
            scalars.len() == self.scalars,
            "compiled phase: {} scalars expected, {} given",
            self.scalars,
            scalars.len()
        );
        assert!(
            lo.checked_add(len).is_some_and(|end| end <= self.cells),
            "compiled phase: cells {lo}..{lo}+{len} of {}",
            self.cells
        );
        len
    }
}

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
/// many run compiled. With native code on ([`native`]), each runs the
/// program's library once it is there and gives the same bits.
pub fn compile(program: &mut Program) -> usize {
    if !enabled() {
        return 0;
    }
    let mut views: Vec<PhaseView> = Vec::new();
    program.compile_phases(&mut |view| {
        views.push(view.clone());
        None
    });
    let library = native::library(&views);
    let mut phase = 0;
    program.compile_phases(&mut |view| {
        // The phase's place in the library, whether or not it compiles.
        let k = phase;
        phase += 1;
        let jit = kernel(view)?;
        // Compiled, so its shape is sound.
        let shape = Shape::of(view)?;
        Some(match &library {
            Some(lib) => Arc::new(native::Switching::new(
                jit,
                Arc::clone(lib),
                k,
                (view, shape),
            )),
            None => jit,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use simsandbox_expr::{EdgeView, OpView};

    /// Written array 0 = read array 0 + scalar 0, over `cells` cells.
    fn adding(cells: usize) -> PhaseView {
        PhaseView {
            cells,
            written: 1,
            read: 1,
            scalars: 1,
            ops: vec![OpView {
                kind: OpKind::Bin(simsandbox_expr::Op::Add),
                d: 0,
                a: Arg::Read(0),
                b: Arg::Scalar(0),
                edges: [EdgeView::Periodic; 2],
            }],
            kept: vec![true],
        }
    }

    #[test]
    fn compiled_code_computes_the_cells_it_is_given() {
        let k = kernel(&adding(8)).expect("compiles");
        let read: Vec<f64> = (0..8).map(f64::from).collect();
        let mut out = vec![0.0; 3];
        k.run(4, &mut [&mut out[..]], &[&read[..]], &[10.0]);
        assert_eq!(out, [14.0, 15.0, 16.0]);
    }

    /// Each mistake a caller could make is a panic before the code runs,
    /// where without the check it would read or write outside an array.
    #[test]
    fn calls_that_do_not_fit_the_compiled_shape_panic() {
        let k = kernel(&adding(8)).expect("compiles");
        let read = vec![0.0; 8];
        let call = |lo: usize, cells: usize, read: &[f64], scalars: &[f64], arrays: usize| {
            let k = Arc::clone(&k);
            let (read, scalars) = (read.to_vec(), scalars.to_vec());
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let mut outs = vec![vec![0.0; cells]; arrays];
                let mut written: Vec<&mut [f64]> = outs.iter_mut().map(Vec::as_mut_slice).collect();
                k.run(lo, &mut written, &[&read[..]], &scalars);
            }))
            .is_err()
        };
        assert!(!call(0, 8, &read, &[1.0], 1));
        assert!(call(6, 3, &read, &[1.0], 1), "cells past the end");
        assert!(call(0, 8, &read[..7], &[1.0], 1), "a short read array");
        assert!(call(0, 8, &read, &[], 1), "a scalar missing");
        assert!(call(0, 8, &read, &[1.0], 0), "a written array missing");
        assert!(
            call(usize::MAX, 2, &read, &[1.0], 1),
            "an overflowing start"
        );
    }

    #[test]
    fn views_that_do_not_hold_together_are_not_compiled() {
        assert!(Shape::of(&adding(8)).is_some());
        let broken = |change: &dyn Fn(&mut PhaseView)| {
            let mut v = adding(8);
            change(&mut v);
            Shape::of(&v).is_none() && kernel(&v).is_none()
        };
        assert!(broken(&|v| v.ops[0].d = 1), "a written array it lacks");
        assert!(
            broken(&|v| v.ops[0].a = Arg::Read(1)),
            "a read array it lacks"
        );
        assert!(
            broken(&|v| v.ops[0].b = Arg::Scalar(1)),
            "a scalar it lacks"
        );
        assert!(broken(&|v| v.kept.clear()), "kept of another length");
        let stencil = |stride, count| OpKind::Stencil {
            second: true,
            stride,
            count,
        };
        // Lines of 3 cells do not fill 8 cells: a neighbour would lie
        // outside the array.
        assert!(broken(&|v| v.ops[0].kind = stencil(1, 3)));
        assert!(broken(&|v| v.ops[0].kind = stencil(0, 8)));
        assert!(broken(&|v| {
            v.ops[0].kind = stencil(1, 8);
            v.ops[0].edges = [EdgeView::Value(5), EdgeView::Periodic];
        }));
        assert!(!broken(&|v| v.ops[0].kind = stencil(2, 4)));
    }
}
