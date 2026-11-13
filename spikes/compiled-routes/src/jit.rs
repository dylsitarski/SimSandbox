//! Route 2: a phase compiled by Cranelift into this process.
//!
//! One function per phase, `fn(lo, len, written, read, scalars)`. Cells
//! inside every line the phase's stencils and shifts walk come in runs:
//! a tight loop over each run does every operation of a cell, its
//! neighbours a stride away, the values it writes kept in registers and
//! stored once. A cell at an end of any line takes the edges into account
//! and is done alone. Lines are followed with counters, as the
//! interpreter does, updated once per run.

use std::sync::Arc;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    types, AbiParam, FuncRef, InstBuilder, MemFlagsData, UserFuncName, Value,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, Linkage, Module};
use simsandbox_expr::{Arg, EdgeView, Op, OpKind, PhaseKernel, PhaseView};

use crate::routes::{func_code, host_call, lines, POW};

type Entry = unsafe extern "C" fn(i64, i64, *const *mut f64, *const *const f64, *const f64);

/// A compiled phase.
pub struct Kernel {
    entry: Entry,
    /// The code's memory, kept while the kernel lives.
    _module: Module_,
}

/// The module, never touched again after compiling: only its memory is
/// kept.
struct Module_(#[allow(dead_code)] JITModule);

// The module is not used after compiling; the code it holds only reads
// what each call is given.
unsafe impl Send for Module_ {}
unsafe impl Sync for Module_ {}

impl PhaseKernel for Kernel {
    fn run(&self, lo: usize, written: &mut [&mut [f64]], read: &[&[f64]], scalars: &[f64]) {
        let len = written.first().map_or(0, |w| w.len());
        let w: Vec<*mut f64> = written.iter_mut().map(|w| w.as_mut_ptr()).collect();
        let r: Vec<*const f64> = read.iter().map(|r| r.as_ptr()).collect();
        // SAFETY: the code reads `len` cells of each written array from
        // its start, read arrays within their length (the cells of lines
        // the view describes) and the scalars it was compiled for.
        unsafe {
            (self.entry)(
                lo as i64,
                len as i64,
                w.as_ptr(),
                r.as_ptr(),
                scalars.as_ptr(),
            )
        }
    }
}

/// What the code of one cell is made from.
struct Cell<'a> {
    view: &'a PhaseView,
    lines: &'a [(usize, usize)],
    written: &'a [Value],
    read: &'a [Value],
    scalars: &'a [Value],
    inverse: &'a [Option<Value>],
    call: FuncRef,
    /// 0, 0.5, 1, 2.
    consts: [Value; 4],
    /// The scalars, reciprocals and 2, splat over two lanes.
    vscalars: &'a [Value],
    vinverse: &'a [Option<Value>],
    vtwo: Value,
}

/// Two cells at once.
const V: types::Type = types::F64X2;

/// Vector accesses: cells need not lie on 16 bytes.
fn vflags() -> MemFlagsData {
    MemFlagsData::new().with_notrap()
}

fn vload(b: &mut FunctionBuilder, base: Value, index: Value, offset: i64) -> Value {
    let addr = at(b, base, index);
    b.ins().load(V, vflags(), addr, (8 * offset) as i32)
}

const FLAGS: fn() -> MemFlagsData = MemFlagsData::trusted;

fn at(b: &mut FunctionBuilder, base: Value, index: Value) -> Value {
    let off = b.ins().ishl_imm_u(index, 3);
    b.ins().iadd(base, off)
}

fn load(b: &mut FunctionBuilder, base: Value, index: Value, offset: i64) -> Value {
    let addr = at(b, base, index);
    b.ins().load(types::F64, FLAGS(), addr, (8 * offset) as i32)
}

impl Cell<'_> {
    /// The operations of cell `i` (`k = lo + i`), `j` its place along each
    /// line; `fast`: inside every line. Returns each written array's
    /// value, for those it writes.
    fn emit(
        &self,
        b: &mut FunctionBuilder,
        i: Value,
        k: Value,
        j: &[Value],
        fast: bool,
    ) -> Vec<Option<Value>> {
        let [zero, half, one, two] = self.consts;
        let mut current: Vec<Option<Value>> = vec![None; self.view.written];
        for (n, op) in self.view.ops.iter().enumerate() {
            let get = |b: &mut FunctionBuilder, a: Arg, current: &mut Vec<Option<Value>>| match a {
                Arg::Written(w) => {
                    *current[w].get_or_insert_with(|| load(b, self.written[w], i, 0))
                }
                Arg::Read(r) => load(b, self.read[r], k, 0),
                Arg::Scalar(s) => self.scalars[s],
            };
            let line = |stride: usize, count: usize| {
                self.lines
                    .iter()
                    .position(|&l| l == (stride, count))
                    .expect("a line")
            };
            let value = match op.kind {
                OpKind::Copy => get(b, op.a, &mut current),
                OpKind::Neg => {
                    let x = get(b, op.a, &mut current);
                    b.ins().fneg(x)
                }
                OpKind::Bin(o) => {
                    let x = get(b, op.a, &mut current);
                    let y = get(b, op.b, &mut current);
                    match o {
                        Op::Add => b.ins().fadd(x, y),
                        Op::Sub => b.ins().fsub(x, y),
                        Op::Mul => b.ins().fmul(x, y),
                        Op::Div => b.ins().fdiv(x, y),
                        Op::Pow => {
                            let code = b.ins().iconst(types::I64, POW);
                            let c = b.ins().call(self.call, &[code, x, y]);
                            b.inst_results(c)[0]
                        }
                        Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                            let cc = match o {
                                Op::Lt => FloatCC::LessThan,
                                Op::Le => FloatCC::LessThanOrEqual,
                                Op::Gt => FloatCC::GreaterThan,
                                _ => FloatCC::GreaterThanOrEqual,
                            };
                            let holds = b.ins().fcmp(cc, x, y);
                            b.ins().select(holds, one, zero)
                        }
                    }
                }
                OpKind::Call1(f) | OpKind::Call2(f) => {
                    let x = get(b, op.a, &mut current);
                    let y = match op.kind {
                        OpKind::Call2(_) => get(b, op.b, &mut current),
                        _ => zero,
                    };
                    let code = b.ins().iconst(types::I64, func_code(f));
                    let c = b.ins().call(self.call, &[code, x, y]);
                    b.inst_results(c)[0]
                }
                OpKind::Axis { stride, count } => {
                    let jf = b.ins().fcvt_from_uint(types::F64, j[line(stride, count)]);
                    let x = get(b, op.a, &mut current);
                    let y = get(b, op.b, &mut current);
                    let jh = b.ins().fadd(jf, half);
                    let off = b.ins().fmul(jh, y);
                    b.ins().fadd(x, off)
                }
                OpKind::Stencil { stride, count, .. } | OpKind::Shift { stride, count, .. } => {
                    let Arg::Read(r) = op.a else {
                        unreachable!("a stencil reads a whole array")
                    };
                    let h = get(b, op.b, &mut current);
                    let jl = j[line(stride, count)];
                    let src = self.read[r];
                    let (s, span) = (stride as i64, ((count - 1) * stride) as i64);
                    let stencil = matches!(op.kind, OpKind::Stencil { .. });
                    // The neighbour on one side: inside the line a stride
                    // away; at the end, by the edge, from the end cell
                    // (`k`) and the far end.
                    let side = |b: &mut FunctionBuilder, low: bool| -> Value {
                        if fast {
                            return load(b, src, k, if low { -s } else { s });
                        }
                        let edge = op.edges[usize::from(!low)];
                        let end = if low { 0 } else { count as i64 - 1 };
                        let at_end = b.ins().icmp_imm_s(IntCC::Equal, jl, end);
                        let (inside, far) = if low {
                            (b.ins().iadd_imm_s(k, -s), b.ins().iadd_imm_s(k, span))
                        } else {
                            (b.ins().iadd_imm_s(k, s), b.ins().iadd_imm_s(k, -span))
                        };
                        if edge == EdgeView::Periodic {
                            let index = b.ins().select(at_end, far, inside);
                            return load(b, src, index, 0);
                        }
                        let index = b.ins().select(at_end, k, inside);
                        // The end cell's value at the end.
                        let near = load(b, src, index, 0);
                        let beyond = match edge {
                            EdgeView::Value(v) if stencil => {
                                let twice = b.ins().fmul(two, self.scalars[v]);
                                b.ins().fsub(twice, near)
                            }
                            EdgeView::Value(v) => self.scalars[v],
                            EdgeView::Gradient(g) => {
                                let g = g.map_or(zero, |g| self.scalars[g]);
                                let gh = b.ins().fmul(g, h);
                                b.ins().fadd(near, gh)
                            }
                            EdgeView::Periodic => unreachable!(),
                        };
                        b.ins().select(at_end, beyond, near)
                    };
                    match op.kind {
                        OpKind::Stencil { second, .. } => {
                            let below = side(b, true);
                            let above = side(b, false);
                            let inv = self.inverse[n].expect("a stencil's reciprocal");
                            if second {
                                let here = load(b, src, k, 0);
                                let twice = b.ins().fmul(two, here);
                                let d = b.ins().fsub(below, twice);
                                let d = b.ins().fadd(d, above);
                                b.ins().fmul(d, inv)
                            } else {
                                let d = b.ins().fsub(above, below);
                                b.ins().fmul(d, inv)
                            }
                        }
                        OpKind::Shift { up, .. } => side(b, !up),
                        _ => unreachable!(),
                    }
                }
            };
            current[op.d] = Some(value);
        }
        current
    }

    /// The operations of cells `i` and `i + 1` (`k = lo + i`), both inside
    /// every line, two lanes at once: each lane computes as the scalar
    /// code does, so the bits are the same. Powers, comparisons, functions
    /// and axes go lane by lane.
    fn emit_vec(
        &self,
        b: &mut FunctionBuilder,
        i: Value,
        k: Value,
        j: &[Value],
    ) -> Vec<Option<Value>> {
        let [zero, half, one, _] = self.consts;
        let mut current: Vec<Option<Value>> = vec![None; self.view.written];
        for (n, op) in self.view.ops.iter().enumerate() {
            let get = |b: &mut FunctionBuilder, a: Arg, current: &mut Vec<Option<Value>>| match a {
                Arg::Written(w) => {
                    *current[w].get_or_insert_with(|| vload(b, self.written[w], i, 0))
                }
                Arg::Read(r) => vload(b, self.read[r], k, 0),
                Arg::Scalar(s) => self.vscalars[s],
            };
            // Lane by lane: `f(b, x, y, lane)` on each lane's values.
            let lanes =
                |b: &mut FunctionBuilder,
                 x: Value,
                 y: Value,
                 f: &dyn Fn(&mut FunctionBuilder, Value, Value, u8) -> Value| {
                    let mut out = None;
                    for lane in 0..2u8 {
                        let xl = b.ins().extractlane(x, lane);
                        let yl = b.ins().extractlane(y, lane);
                        let v = f(b, xl, yl, lane);
                        out = Some(match out {
                            None => b.ins().splat(V, v),
                            Some(o) => b.ins().insertlane(o, v, lane),
                        });
                    }
                    out.expect("two lanes")
                };
            let value = match op.kind {
                OpKind::Copy => get(b, op.a, &mut current),
                OpKind::Neg => {
                    let x = get(b, op.a, &mut current);
                    b.ins().fneg(x)
                }
                OpKind::Bin(o) => {
                    let x = get(b, op.a, &mut current);
                    let y = get(b, op.b, &mut current);
                    match o {
                        Op::Add => b.ins().fadd(x, y),
                        Op::Sub => b.ins().fsub(x, y),
                        Op::Mul => b.ins().fmul(x, y),
                        Op::Div => b.ins().fdiv(x, y),
                        Op::Pow => lanes(b, x, y, &|b, x, y, _| {
                            let code = b.ins().iconst(types::I64, POW);
                            let c = b.ins().call(self.call, &[code, x, y]);
                            b.inst_results(c)[0]
                        }),
                        Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                            let cc = match o {
                                Op::Lt => FloatCC::LessThan,
                                Op::Le => FloatCC::LessThanOrEqual,
                                Op::Gt => FloatCC::GreaterThan,
                                _ => FloatCC::GreaterThanOrEqual,
                            };
                            lanes(b, x, y, &|b, x, y, _| {
                                let holds = b.ins().fcmp(cc, x, y);
                                b.ins().select(holds, one, zero)
                            })
                        }
                    }
                }
                OpKind::Call1(f) | OpKind::Call2(f) => {
                    let x = get(b, op.a, &mut current);
                    let y = match op.kind {
                        OpKind::Call2(_) => get(b, op.b, &mut current),
                        _ => b.ins().splat(V, zero),
                    };
                    lanes(b, x, y, &|b, x, y, _| {
                        let code = b.ins().iconst(types::I64, func_code(f));
                        let c = b.ins().call(self.call, &[code, x, y]);
                        b.inst_results(c)[0]
                    })
                }
                OpKind::Axis { stride, count } => {
                    let l = self
                        .lines
                        .iter()
                        .position(|&l| l == (stride, count))
                        .expect("a line");
                    let x = get(b, op.a, &mut current);
                    let y = get(b, op.b, &mut current);
                    let jl = j[l];
                    lanes(b, x, y, &|b, x, y, lane| {
                        // Along a line of stride 1 the second cell is one on.
                        let jj = if stride == 1 && lane == 1 {
                            b.ins().iadd_imm_s(jl, 1)
                        } else {
                            jl
                        };
                        let jf = b.ins().fcvt_from_uint(types::F64, jj);
                        let jh = b.ins().fadd(jf, half);
                        let off = b.ins().fmul(jh, y);
                        b.ins().fadd(x, off)
                    })
                }
                OpKind::Stencil { stride, second, .. } => {
                    let Arg::Read(r) = op.a else {
                        unreachable!("a stencil reads a whole array")
                    };
                    let (src, s) = (self.read[r], stride as i64);
                    let below = vload(b, src, k, -s);
                    let above = vload(b, src, k, s);
                    let inv = self.vinverse[n].expect("a stencil's reciprocal");
                    if second {
                        let here = vload(b, src, k, 0);
                        let twice = b.ins().fmul(self.vtwo, here);
                        let d = b.ins().fsub(below, twice);
                        let d = b.ins().fadd(d, above);
                        b.ins().fmul(d, inv)
                    } else {
                        let d = b.ins().fsub(above, below);
                        b.ins().fmul(d, inv)
                    }
                }
                OpKind::Shift { stride, up, .. } => {
                    let Arg::Read(r) = op.a else {
                        unreachable!("a shift reads a whole array")
                    };
                    let s = stride as i64;
                    vload(b, self.read[r], k, if up { s } else { -s })
                }
            };
            current[op.d] = Some(value);
        }
        current
    }

    /// Store two cells' values.
    fn store_vec(&self, b: &mut FunctionBuilder, i: Value, values: &[Option<Value>]) {
        for (w, v) in values.iter().enumerate() {
            if let (Some(v), true) = (v, self.view.kept[w]) {
                let addr = at(b, self.written[w], i);
                b.ins().store(vflags(), *v, addr, 0);
            }
        }
    }

    /// Store a cell's values.
    fn store(&self, b: &mut FunctionBuilder, i: Value, values: &[Option<Value>]) {
        for (w, v) in values.iter().enumerate() {
            // Temporaries nothing reads after the phase stay in registers.
            if let (Some(v), true) = (v, self.view.kept[w]) {
                let addr = at(b, self.written[w], i);
                b.ins().store(FLAGS(), *v, addr, 0);
            }
        }
    }
}

/// Compile a phase.
pub fn compile(view: &PhaseView) -> Arc<dyn PhaseKernel> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").unwrap();
    let isa = cranelift_native::builder()
        .unwrap()
        .finish(settings::Flags::new(flags))
        .unwrap();
    let mut builder = JITBuilder::with_isa(isa, default_libcall_names());
    builder.symbol("host_call", host_call as *const u8);
    let mut module = JITModule::new(builder);
    let ptr = module.target_config().pointer_type();

    let mut sig = module.make_signature();
    for _ in 0..2 {
        sig.params.push(AbiParam::new(types::I64));
    }
    for _ in 0..3 {
        sig.params.push(AbiParam::new(ptr));
    }
    let id = module
        .declare_function("phase", Linkage::Export, &sig)
        .unwrap();
    let mut call_sig = module.make_signature();
    call_sig.params.push(AbiParam::new(types::I64));
    call_sig.params.push(AbiParam::new(types::F64));
    call_sig.params.push(AbiParam::new(types::F64));
    call_sig.returns.push(AbiParam::new(types::F64));
    let call_id = module
        .declare_function("host_call", Linkage::Import, &call_sig)
        .unwrap();

    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    ctx.func.name = UserFuncName::user(0, id.as_u32());
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let call = module.declare_func_in_func(call_id, b.func);
        let entry = b.create_block();
        let header = b.create_block();
        let dispatch = b.create_block();
        let edge = b.create_block();
        let run = b.create_block();
        let inner_header = b.create_block();
        let inner = b.create_block();
        let run_done = b.create_block();
        let vec_header = b.create_block();
        let vec_body = b.create_block();
        let exit = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let p = b.block_params(entry).to_vec();
        let (lo, len, wp, rp, sp) = (p[0], p[1], p[2], p[3], p[4]);
        let written: Vec<Value> = (0..view.written)
            .map(|k| b.ins().load(ptr, FLAGS(), wp, (8 * k) as i32))
            .collect();
        let read: Vec<Value> = (0..view.read)
            .map(|k| b.ins().load(ptr, FLAGS(), rp, (8 * k) as i32))
            .collect();
        let scalars: Vec<Value> = (0..view.scalars)
            .map(|k| b.ins().load(types::F64, FLAGS(), sp, (8 * k) as i32))
            .collect();
        let consts = [0.0, 0.5, 1.0, 2.0].map(|c| b.ins().f64const(c));
        let [_, _, one, two] = consts;
        // Each stencil's reciprocal, once per call, as the interpreter.
        let inverse: Vec<Option<Value>> = view
            .ops
            .iter()
            .map(|op| match (op.kind, op.b) {
                (OpKind::Stencil { second, .. }, Arg::Scalar(h)) => {
                    let h = scalars[h];
                    let d = if second {
                        b.ins().fmul(h, h)
                    } else {
                        b.ins().fmul(two, h)
                    };
                    Some(b.ins().fdiv(one, d))
                }
                _ => None,
            })
            .collect();
        let lines = lines(view);
        // Lines whose ends a cell must look out for: those stencils and
        // shifts walk.
        let edged: Vec<bool> = lines
            .iter()
            .map(|&l| {
                view.ops.iter().any(|op| {
                    matches!(op.kind,
                        OpKind::Stencil { stride, count, .. } | OpKind::Shift { stride, count, .. }
                            if (stride, count) == l)
                })
            })
            .collect();
        // Counters along each line: the cell within a stride and the
        // place `j`, from `lo`.
        let i = b.declare_var(types::I64);
        let end = b.declare_var(types::I64);
        let zero_i = b.ins().iconst(types::I64, 0);
        b.def_var(i, zero_i);
        let counters: Vec<(Variable, Variable)> = lines
            .iter()
            .map(|&(s, c)| {
                let (within, j) = (b.declare_var(types::I64), b.declare_var(types::I64));
                let w0 = b.ins().urem_imm_u(lo, s as i64);
                let q = b.ins().udiv_imm_u(lo, s as i64);
                let j0 = b.ins().urem_imm_u(q, c as i64);
                b.def_var(within, w0);
                b.def_var(j, j0);
                (within, j)
            })
            .collect();
        // Within a run, the place along each line of stride 1 moves with
        // every cell; along the others it stays.
        let run_j: Vec<Variable> = lines.iter().map(|_| b.declare_var(types::I64)).collect();
        let vscalars: Vec<Value> = scalars.iter().map(|&s| b.ins().splat(V, s)).collect();
        let vinverse: Vec<Option<Value>> = inverse
            .iter()
            .map(|v| v.map(|v| b.ins().splat(V, v)))
            .collect();
        let vtwo = b.ins().splat(V, two);
        let simd = std::env::var("SPIKE_NO_SIMD").is_err();
        let cell = Cell {
            vscalars: &vscalars,
            vinverse: &vinverse,
            vtwo,
            view,
            lines: &lines,
            written: &written,
            read: &read,
            scalars: &scalars,
            inverse: &inverse,
            call,
            consts,
        };
        b.ins().jump(header, &[]);

        // Cells left?
        b.switch_to_block(header);
        let iv = b.use_var(i);
        let more = b.ins().icmp(IntCC::SignedLessThan, iv, len);
        b.ins().brif(more, dispatch, &[], exit, &[]);

        // At an end of a line, or inside them all?
        b.switch_to_block(dispatch);
        let mut any = None;
        for (l, &(_, c)) in lines.iter().enumerate() {
            if !edged[l] {
                continue;
            }
            let j = b.use_var(counters[l].1);
            let low = b.ins().icmp_imm_s(IntCC::Equal, j, 0);
            let high = b.ins().icmp_imm_s(IntCC::Equal, j, c as i64 - 1);
            let e = b.ins().bor(low, high);
            any = Some(match any {
                None => e,
                Some(a) => b.ins().bor(a, e),
            });
        }
        match any {
            Some(any) => b.ins().brif(any, edge, &[], run, &[]),
            None => b.ins().jump(run, &[]),
        };

        // One cell at an end: its edges, then on by one.
        b.switch_to_block(edge);
        let iv = b.use_var(i);
        let k = b.ins().iadd(lo, iv);
        let js: Vec<Value> = counters.iter().map(|&(_, j)| b.use_var(j)).collect();
        let values = cell.emit(&mut b, iv, k, &js, false);
        cell.store(&mut b, iv, &values);
        let one_i = b.ins().iconst(types::I64, 1);
        advance(&mut b, &lines, &counters, one_i);
        let next = b.ins().iadd_imm_s(iv, 1);
        b.def_var(i, next);
        b.ins().jump(header, &[]);

        // A run of cells inside every line: as many as are left before
        // any line's place changes (or, along a line of stride 1, before
        // its last cell, or its end for one no stencil walks).
        b.switch_to_block(run);
        let iv = b.use_var(i);
        let mut count = b.ins().isub(len, iv);
        for (l, &(s, c)) in lines.iter().enumerate() {
            let (within, j) = counters[l];
            let left = if s == 1 {
                let j = b.use_var(j);
                let last = c as i64 - i64::from(edged[l]);
                let last = b.ins().iconst(types::I64, last);
                b.ins().isub(last, j)
            } else {
                let w = b.use_var(within);
                let s = b.ins().iconst(types::I64, s as i64);
                b.ins().isub(s, w)
            };
            count = b.ins().umin(count, left);
            let j = b.use_var(j);
            b.def_var(run_j[l], j);
        }
        let stop = b.ins().iadd(iv, count);
        b.def_var(end, stop);
        b.ins()
            .jump(if simd { vec_header } else { inner_header }, &[]);

        // Two cells at a time while two are left in the run.
        b.switch_to_block(vec_header);
        let iv = b.use_var(i);
        let stop = b.use_var(end);
        let two_on = b.ins().iadd_imm_s(iv, 2);
        let more = b.ins().icmp(IntCC::SignedLessThanOrEqual, two_on, stop);
        b.ins().brif(more, vec_body, &[], inner_header, &[]);

        b.switch_to_block(vec_body);
        let iv = b.use_var(i);
        let k = b.ins().iadd(lo, iv);
        let js: Vec<Value> = run_j.iter().map(|&j| b.use_var(j)).collect();
        let values = cell.emit_vec(&mut b, iv, k, &js);
        cell.store_vec(&mut b, iv, &values);
        for (l, &(s, _)) in lines.iter().enumerate() {
            if s == 1 {
                let j = b.ins().iadd_imm_s(js[l], 2);
                b.def_var(run_j[l], j);
            }
        }
        let next = b.ins().iadd_imm_s(iv, 2);
        b.def_var(i, next);
        b.ins().jump(vec_header, &[]);

        b.switch_to_block(inner_header);
        let iv = b.use_var(i);
        let stop = b.use_var(end);
        let more = b.ins().icmp(IntCC::SignedLessThan, iv, stop);
        b.ins().brif(more, inner, &[], run_done, &[]);

        b.switch_to_block(inner);
        let iv = b.use_var(i);
        let k = b.ins().iadd(lo, iv);
        let js: Vec<Value> = run_j.iter().map(|&j| b.use_var(j)).collect();
        let values = cell.emit(&mut b, iv, k, &js, true);
        cell.store(&mut b, iv, &values);
        for (l, &(s, _)) in lines.iter().enumerate() {
            if s == 1 {
                let j = b.ins().iadd_imm_s(js[l], 1);
                b.def_var(run_j[l], j);
            }
        }
        let next = b.ins().iadd_imm_s(iv, 1);
        b.def_var(i, next);
        b.ins().jump(inner_header, &[]);

        b.switch_to_block(run_done);
        advance(&mut b, &lines, &counters, count);
        b.ins().jump(header, &[]);

        b.switch_to_block(exit);
        b.ins().return_(&[]);
        b.seal_all_blocks();
        b.finalize(module.target_config());
    }
    let dump = std::env::var("SPIKE_DUMP").is_ok();
    ctx.set_disasm(dump);
    module.define_function(id, &mut ctx).unwrap();
    if dump {
        let code = ctx.compiled_code().unwrap();
        eprintln!(
            "{} ops, {} bytes\n{}",
            view.ops.len(),
            code.code_info().total_size,
            code.vcode.clone().unwrap_or_default()
        );
    }
    module.clear_context(&mut ctx);
    module.finalize_definitions().unwrap();
    let code = module.get_finalized_function(id);
    // SAFETY: the function was declared with `Entry`'s signature.
    let entry: Entry = unsafe { std::mem::transmute(code) };
    Arc::new(Kernel {
        entry,
        _module: Module_(module),
    })
}

/// Move every line's counters on by `m` cells, `m` no more than to the
/// next change of place along a line of stride above 1 (one wrap at most).
fn advance(
    b: &mut FunctionBuilder,
    lines: &[(usize, usize)],
    counters: &[(Variable, Variable)],
    m: Value,
) {
    let zero = b.ins().iconst(types::I64, 0);
    for (l, &(s, c)) in lines.iter().enumerate() {
        let (within, j) = counters[l];
        let jv = b.use_var(j);
        let moved = if s == 1 {
            // Along the line: no further than its end.
            let j1 = b.ins().iadd(jv, m);
            let wrap = b.ins().icmp_imm_s(IntCC::Equal, j1, c as i64);
            b.ins().select(wrap, zero, j1)
        } else {
            let w = b.use_var(within);
            let w1 = b.ins().iadd(w, m);
            let full = b.ins().icmp_imm_s(IntCC::Equal, w1, s as i64);
            let w = b.ins().select(full, zero, w1);
            b.def_var(within, w);
            let j1 = b.ins().iadd_imm_s(jv, 1);
            let wrap = b.ins().icmp_imm_s(IntCC::Equal, j1, c as i64);
            let j1 = b.ins().select(wrap, zero, j1);
            b.ins().select(full, j1, jv)
        };
        b.def_var(j, moved);
    }
}
