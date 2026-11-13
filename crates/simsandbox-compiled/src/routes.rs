//! What the compiled routes share: how operations outside plain
//! arithmetic are computed (by the interpreter's own functions, so the
//! bits are the same), and the lines a phase walks.

use simsandbox_expr::{Func, OpKind, PhaseView};

/// Every function, by its code for [`host_call`].
pub const FUNCS: [Func; 21] = [
    Func::Sin,
    Func::Cos,
    Func::Tan,
    Func::Asin,
    Func::Acos,
    Func::Atan,
    Func::Atan2,
    Func::Sinh,
    Func::Cosh,
    Func::Tanh,
    Func::Exp,
    Func::Ln,
    Func::Log10,
    Func::Sqrt,
    Func::Abs,
    Func::Sign,
    Func::Floor,
    Func::Ceil,
    Func::Min,
    Func::Max,
    Func::Pow,
];

/// The code [`host_call`] takes for `x^y`.
pub const POW: i64 = -1;

/// The code of a function.
pub fn func_code(f: Func) -> i64 {
    FUNCS.iter().position(|&g| g == f).expect("every function") as i64
}

/// Functions and powers, called from compiled code: the interpreter's.
pub extern "C" fn host_call(code: i64, a: f64, b: f64) -> f64 {
    if code == POW {
        a.powf(b)
    } else {
        FUNCS[code as usize].apply(a, b)
    }
}

/// The lines (stride, count) the phase's axes, stencils and shifts walk,
/// each once.
pub fn lines(view: &PhaseView) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    for op in &view.ops {
        if let OpKind::Axis { stride, count }
        | OpKind::Stencil { stride, count, .. }
        | OpKind::Shift { stride, count, .. } = op.kind
        {
            if !lines.contains(&(stride, count)) {
                lines.push((stride, count));
            }
        }
    }
    lines
}
