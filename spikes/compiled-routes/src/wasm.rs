//! Route 3: a phase compiled to WebAssembly and run by Wasmtime (which
//! compiles it with Cranelift).
//!
//! The generated function does what route 2's does, in the same shape:
//! runs of cells inside every line, cells at an end one at a time,
//! temporaries nothing reads after the phase kept in locals. Its arrays
//! live in the module's memory, so each call copies the arrays read in
//! and those kept out: the price of running in a sandbox whose memory is
//! its own.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use simsandbox_expr::{Arg, EdgeView, Op, OpKind, PhaseKernel, PhaseView};
use wasm_encoder::{
    BlockType, CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection,
    ImportSection, Instruction as I, MemArg, MemoryType, Module, TypeSection, ValType,
};

use crate::routes::{func_code, host_call, lines, POW};

/// Nanoseconds spent inside WebAssembly calls, for the time without
/// copying.
pub static INSIDE: AtomicU64 = AtomicU64::new(0);

/// A compiled phase, its instance and memory.
pub struct Kernel {
    view: PhaseView,
    run: Mutex<Instance>,
}

struct Instance {
    store: wasmtime::Store<()>,
    memory: wasmtime::Memory,
    func: wasmtime::Func,
}

/// Where each array lies in memory for `cells` cells, `len` written.
fn layout(view: &PhaseView, len: usize) -> (Vec<usize>, Vec<usize>, usize) {
    let mut at = 0;
    let read: Vec<usize> = (0..view.read)
        .map(|_| {
            let a = at;
            at += view.cells * 8;
            a
        })
        .collect();
    let written: Vec<usize> = (0..view.written)
        .map(|_| {
            let a = at;
            at += len * 8;
            a
        })
        .collect();
    (read, written, at)
}

impl PhaseKernel for Kernel {
    fn run(&self, lo: usize, written: &mut [&mut [f64]], read: &[&[f64]], scalars: &[f64]) {
        let len = written.first().map_or(0, |w| w.len());
        let (r_at, w_at, _) = layout(&self.view, len);
        let mut inst = self.run.lock().unwrap();
        let Instance {
            store,
            memory,
            func,
        } = &mut *inst;
        {
            let mem = memory.data_mut(&mut *store);
            let put = |mem: &mut [u8], at: usize, x: &[f64]| {
                for (c, v) in mem[at..at + 8 * x.len()]
                    .as_chunks_mut::<8>()
                    .0
                    .iter_mut()
                    .zip(x)
                {
                    *c = v.to_le_bytes();
                }
            };
            for (k, r) in read.iter().enumerate() {
                put(mem, r_at[k], r);
            }
            for (k, w) in written.iter().enumerate() {
                put(mem, w_at[k], w);
            }
        }
        let mut args: Vec<wasmtime::Val> = vec![
            wasmtime::Val::I32(lo as i32),
            wasmtime::Val::I32(len as i32),
        ];
        args.extend(
            r_at.iter()
                .chain(&w_at)
                .map(|&a| wasmtime::Val::I32(a as i32)),
        );
        args.extend(scalars.iter().map(|&s| wasmtime::Val::F64(s.to_bits())));
        let start = std::time::Instant::now();
        func.call(&mut *store, &args, &mut []).unwrap();
        INSIDE.fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        let mem = memory.data(&*store);
        for (k, w) in written.iter_mut().enumerate() {
            if self.view.kept[k] {
                for (v, c) in w.iter_mut().zip(mem[w_at[k]..].as_chunks::<8>().0.iter()) {
                    *v = f64::from_le_bytes(*c);
                }
            }
        }
    }
}

/// The module for a phase: its bytes.
pub fn module(view: &PhaseView) -> Vec<u8> {
    let lines = lines(view);
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
    let mut types = TypeSection::new();
    // 0: the host's functions; 1: the phase.
    types
        .ty()
        .function([ValType::I32, ValType::F64, ValType::F64], [ValType::F64]);
    let mut params = vec![ValType::I32; 2 + view.read + view.written];
    params.extend(vec![ValType::F64; view.scalars]);
    types.ty().function(params.clone(), []);
    let mut imports = ImportSection::new();
    imports.import("env", "call", EntityType::Function(0));
    let mut functions = FunctionSection::new();
    functions.function(1);
    let (_, _, bytes) = layout(view, view.cells);
    let mut memories = wasm_encoder::MemorySection::new();
    memories.memory(MemoryType {
        minimum: (bytes as u64).div_ceil(65536) + 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    let mut exports = ExportSection::new();
    exports.export("phase", ExportKind::Func, 1);
    exports.export("memory", ExportKind::Memory, 0);

    // Locals after the parameters.
    let np = params.len() as u32;
    let (p_lo, p_len) = (0u32, 1u32);
    let p_read = |r: usize| 2 + r as u32;
    let p_written = |w: usize| 2 + (view.read + w) as u32;
    let p_scalar = |s: usize| 2 + (view.read + view.written + s) as u32;
    let mut i32s = 0u32;
    let mut f64s = 0u32;
    let mut new_i = || {
        i32s += 1;
        i32s - 1
    };
    let li = new_i();
    let lk = new_i();
    let lend = new_i();
    let lcount = new_i();
    let within: Vec<u32> = lines.iter().map(|_| new_i()).collect();
    let jl: Vec<u32> = lines.iter().map(|_| new_i()).collect();
    let run_j: Vec<u32> = lines.iter().map(|_| new_i()).collect();
    // f64 locals: per op its value, per written array its current value,
    // per stencil its reciprocal.
    let mut new_f = || {
        f64s += 1;
        f64s - 1
    };
    let op_val: Vec<u32> = view.ops.iter().map(|_| new_f()).collect();
    let inv: Vec<u32> = view.ops.iter().map(|_| new_f()).collect();
    let tmp: Vec<u32> = (0..4).map(|_| new_f()).collect();
    let i32_local = |l: u32| np + l;
    let f64_local = |l: u32| np + i32s + l;
    let mut f = Function::new([(i32s, ValType::I32), (f64s, ValType::F64)]);
    let mem = |offset: u64| MemArg {
        offset,
        align: 3,
        memory_index: 0,
    };
    let (li, lk, lend, lcount) = (
        i32_local(li),
        i32_local(lk),
        i32_local(lend),
        i32_local(lcount),
    );
    let within: Vec<u32> = within.into_iter().map(i32_local).collect();
    let jl: Vec<u32> = jl.into_iter().map(i32_local).collect();
    let run_j: Vec<u32> = run_j.into_iter().map(i32_local).collect();
    let op_val: Vec<u32> = op_val.into_iter().map(f64_local).collect();
    let inv: Vec<u32> = inv.into_iter().map(f64_local).collect();
    let tmp: Vec<u32> = tmp.into_iter().map(f64_local).collect();

    // Reciprocals, once per call.
    for (n, op) in view.ops.iter().enumerate() {
        if let (OpKind::Stencil { second, .. }, Arg::Scalar(h)) = (op.kind, op.b) {
            f.instruction(&I::F64Const(1.0.into()));
            if second {
                f.instruction(&I::LocalGet(p_scalar(h)));
                f.instruction(&I::LocalGet(p_scalar(h)));
            } else {
                f.instruction(&I::F64Const(2.0.into()));
                f.instruction(&I::LocalGet(p_scalar(h)));
            }
            f.instruction(&I::F64Mul);
            f.instruction(&I::F64Div);
            f.instruction(&I::LocalSet(inv[n]));
        }
    }
    // Counters from `lo`.
    for (l, &(s, c)) in lines.iter().enumerate() {
        f.instruction(&I::LocalGet(p_lo));
        f.instruction(&I::I32Const(s as i32));
        f.instruction(&I::I32RemU);
        f.instruction(&I::LocalSet(within[l]));
        f.instruction(&I::LocalGet(p_lo));
        f.instruction(&I::I32Const(s as i32));
        f.instruction(&I::I32DivU);
        f.instruction(&I::I32Const(c as i32));
        f.instruction(&I::I32RemU);
        f.instruction(&I::LocalSet(jl[l]));
    }

    // One cell's operations into `op_val`, its values stored where kept.
    let emit = |f: &mut Function, fast: bool, j: &[u32]| {
        // The address of cell `index` (on the stack, i32) of an array whose
        // base is in local `base`.
        let addr = |f: &mut Function, base: u32| {
            f.instruction(&I::I32Const(3));
            f.instruction(&I::I32Shl);
            f.instruction(&I::LocalGet(base));
            f.instruction(&I::I32Add);
        };
        // Which op last wrote each written array, as the cell goes.
        let mut last: Vec<Option<usize>> = vec![None; view.written];
        for (n, op) in view.ops.iter().enumerate() {
            let push = |f: &mut Function, a: Arg, last: &[Option<usize>]| match a {
                Arg::Written(w) => match last[w] {
                    Some(o) => {
                        f.instruction(&I::LocalGet(op_val[o]));
                    }
                    None => {
                        f.instruction(&I::LocalGet(li));
                        addr(f, p_written(w));
                        f.instruction(&I::F64Load(mem(0)));
                    }
                },
                Arg::Read(r) => {
                    f.instruction(&I::LocalGet(lk));
                    addr(f, p_read(r));
                    f.instruction(&I::F64Load(mem(0)));
                }
                Arg::Scalar(s) => {
                    f.instruction(&I::LocalGet(p_scalar(s)));
                }
            };
            let line = |stride: usize, count: usize| {
                lines
                    .iter()
                    .position(|&l| l == (stride, count))
                    .expect("a line")
            };
            match op.kind {
                OpKind::Copy => push(f, op.a, &last),
                OpKind::Neg => {
                    push(f, op.a, &last);
                    f.instruction(&I::F64Neg);
                }
                OpKind::Bin(o) => {
                    if o == Op::Pow {
                        f.instruction(&I::I32Const(POW as i32));
                    }
                    push(f, op.a, &last);
                    push(f, op.b, &last);
                    match o {
                        Op::Add => f.instruction(&I::F64Add),
                        Op::Sub => f.instruction(&I::F64Sub),
                        Op::Mul => f.instruction(&I::F64Mul),
                        Op::Div => f.instruction(&I::F64Div),
                        Op::Pow => f.instruction(&I::Call(0)),
                        Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                            f.instruction(&match o {
                                Op::Lt => I::F64Lt,
                                Op::Le => I::F64Le,
                                Op::Gt => I::F64Gt,
                                _ => I::F64Ge,
                            });
                            f.instruction(&I::F64ConvertI32U)
                        }
                    };
                }
                OpKind::Call1(g) | OpKind::Call2(g) => {
                    f.instruction(&I::I32Const(func_code(g) as i32));
                    push(f, op.a, &last);
                    match op.kind {
                        OpKind::Call2(_) => push(f, op.b, &last),
                        _ => {
                            f.instruction(&I::F64Const(0.0.into()));
                        }
                    }
                    f.instruction(&I::Call(0));
                }
                OpKind::Axis { stride, count } => {
                    push(f, op.a, &last);
                    f.instruction(&I::LocalGet(j[line(stride, count)]));
                    f.instruction(&I::F64ConvertI32U);
                    f.instruction(&I::F64Const(0.5.into()));
                    f.instruction(&I::F64Add);
                    push(f, op.b, &last);
                    f.instruction(&I::F64Mul);
                    f.instruction(&I::F64Add);
                }
                OpKind::Stencil { stride, count, .. } | OpKind::Shift { stride, count, .. } => {
                    let Arg::Read(r) = op.a else {
                        unreachable!("a stencil reads a whole array")
                    };
                    let Arg::Scalar(h) = op.b else {
                        unreachable!("a spacing")
                    };
                    let jv = j[line(stride, count)];
                    let (s, span) = (stride as i32, ((count - 1) * stride) as i32);
                    let stencil = matches!(op.kind, OpKind::Stencil { .. });
                    let load_at = |f: &mut Function, delta: i32| {
                        f.instruction(&I::LocalGet(lk));
                        f.instruction(&I::I32Const(delta));
                        f.instruction(&I::I32Add);
                        addr(f, p_read(r));
                        f.instruction(&I::F64Load(mem(0)));
                    };
                    // The neighbour on one side into local `into`.
                    let side = |f: &mut Function, low: bool, into: u32| {
                        if fast {
                            load_at(f, if low { -s } else { s });
                            f.instruction(&I::LocalSet(into));
                            return;
                        }
                        let edge = op.edges[usize::from(!low)];
                        let at_end = |f: &mut Function| {
                            f.instruction(&I::LocalGet(jv));
                            f.instruction(&I::I32Const(if low { 0 } else { count as i32 - 1 }));
                            f.instruction(&I::I32Eq);
                        };
                        let (inside, far) = if low { (-s, span) } else { (s, -span) };
                        if edge == EdgeView::Periodic {
                            f.instruction(&I::LocalGet(lk));
                            f.instruction(&I::I32Const(far));
                            f.instruction(&I::I32Add);
                            f.instruction(&I::LocalGet(lk));
                            f.instruction(&I::I32Const(inside));
                            f.instruction(&I::I32Add);
                            at_end(f);
                            f.instruction(&I::Select);
                            addr(f, p_read(r));
                            f.instruction(&I::F64Load(mem(0)));
                            f.instruction(&I::LocalSet(into));
                            return;
                        }
                        // The end cell's value at the end, else inside.
                        f.instruction(&I::LocalGet(lk));
                        f.instruction(&I::LocalGet(lk));
                        f.instruction(&I::I32Const(inside));
                        f.instruction(&I::I32Add);
                        at_end(f);
                        f.instruction(&I::Select);
                        addr(f, p_read(r));
                        f.instruction(&I::F64Load(mem(0)));
                        f.instruction(&I::LocalSet(tmp[3]));
                        match edge {
                            EdgeView::Value(v) if stencil => {
                                f.instruction(&I::F64Const(2.0.into()));
                                f.instruction(&I::LocalGet(p_scalar(v)));
                                f.instruction(&I::F64Mul);
                                f.instruction(&I::LocalGet(tmp[3]));
                                f.instruction(&I::F64Sub);
                            }
                            EdgeView::Value(v) => {
                                f.instruction(&I::LocalGet(p_scalar(v)));
                            }
                            EdgeView::Gradient(g) => {
                                f.instruction(&I::LocalGet(tmp[3]));
                                match g {
                                    Some(g) => f.instruction(&I::LocalGet(p_scalar(g))),
                                    None => f.instruction(&I::F64Const(0.0.into())),
                                };
                                f.instruction(&I::LocalGet(p_scalar(h)));
                                f.instruction(&I::F64Mul);
                                f.instruction(&I::F64Add);
                            }
                            EdgeView::Periodic => unreachable!(),
                        }
                        f.instruction(&I::LocalGet(tmp[3]));
                        at_end(f);
                        f.instruction(&I::Select);
                        f.instruction(&I::LocalSet(into));
                    };
                    match op.kind {
                        OpKind::Stencil { second, .. } => {
                            side(f, true, tmp[0]);
                            side(f, false, tmp[1]);
                            if second {
                                f.instruction(&I::LocalGet(tmp[0]));
                                f.instruction(&I::F64Const(2.0.into()));
                                load_at(f, 0);
                                f.instruction(&I::F64Mul);
                                f.instruction(&I::F64Sub);
                                f.instruction(&I::LocalGet(tmp[1]));
                                f.instruction(&I::F64Add);
                            } else {
                                f.instruction(&I::LocalGet(tmp[1]));
                                f.instruction(&I::LocalGet(tmp[0]));
                                f.instruction(&I::F64Sub);
                            }
                            f.instruction(&I::LocalGet(inv[n]));
                            f.instruction(&I::F64Mul);
                        }
                        OpKind::Shift { up, .. } => {
                            side(f, !up, tmp[0]);
                            f.instruction(&I::LocalGet(tmp[0]));
                        }
                        _ => unreachable!(),
                    }
                }
            }
            f.instruction(&I::LocalSet(op_val[n]));
            last[op.d] = Some(n);
        }
        for (w, o) in last.iter().enumerate() {
            if let (Some(o), true) = (o, view.kept[w]) {
                f.instruction(&I::LocalGet(li));
                addr(f, p_written(w));
                f.instruction(&I::LocalGet(op_val[*o]));
                f.instruction(&I::F64Store(mem(0)));
            }
        }
    };
    // Move the counters on by the i32 in local `by`.
    let advance = |f: &mut Function, by: u32| {
        for (l, &(s, c)) in lines.iter().enumerate() {
            if s == 1 {
                f.instruction(&I::LocalGet(jl[l]));
                f.instruction(&I::LocalGet(by));
                f.instruction(&I::I32Add);
                f.instruction(&I::LocalTee(jl[l]));
                f.instruction(&I::I32Const(c as i32));
                f.instruction(&I::I32Eq);
                f.instruction(&I::If(BlockType::Empty));
                f.instruction(&I::I32Const(0));
                f.instruction(&I::LocalSet(jl[l]));
                f.instruction(&I::End);
            } else {
                f.instruction(&I::LocalGet(within[l]));
                f.instruction(&I::LocalGet(by));
                f.instruction(&I::I32Add);
                f.instruction(&I::LocalTee(within[l]));
                f.instruction(&I::I32Const(s as i32));
                f.instruction(&I::I32Eq);
                f.instruction(&I::If(BlockType::Empty));
                f.instruction(&I::I32Const(0));
                f.instruction(&I::LocalSet(within[l]));
                f.instruction(&I::LocalGet(jl[l]));
                f.instruction(&I::I32Const(1));
                f.instruction(&I::I32Add);
                f.instruction(&I::LocalTee(jl[l]));
                f.instruction(&I::I32Const(c as i32));
                f.instruction(&I::I32Eq);
                f.instruction(&I::If(BlockType::Empty));
                f.instruction(&I::I32Const(0));
                f.instruction(&I::LocalSet(jl[l]));
                f.instruction(&I::End);
                f.instruction(&I::End);
            }
        }
    };

    f.instruction(&I::Block(BlockType::Empty)); // exit
    f.instruction(&I::Loop(BlockType::Empty)); // cells
    f.instruction(&I::LocalGet(li));
    f.instruction(&I::LocalGet(p_len));
    f.instruction(&I::I32GeS);
    f.instruction(&I::BrIf(1));
    f.instruction(&I::LocalGet(p_lo));
    f.instruction(&I::LocalGet(li));
    f.instruction(&I::I32Add);
    f.instruction(&I::LocalSet(lk));
    // At an end of any line?
    f.instruction(&I::I32Const(0));
    for (l, &(_, c)) in lines.iter().enumerate() {
        if !edged[l] {
            continue;
        }
        f.instruction(&I::LocalGet(jl[l]));
        f.instruction(&I::I32Eqz);
        f.instruction(&I::I32Or);
        f.instruction(&I::LocalGet(jl[l]));
        f.instruction(&I::I32Const(c as i32 - 1));
        f.instruction(&I::I32Eq);
        f.instruction(&I::I32Or);
    }
    f.instruction(&I::If(BlockType::Empty));
    emit(&mut f, false, &jl);
    f.instruction(&I::I32Const(1));
    f.instruction(&I::LocalSet(lcount));
    advance(&mut f, lcount);
    f.instruction(&I::LocalGet(li));
    f.instruction(&I::I32Const(1));
    f.instruction(&I::I32Add);
    f.instruction(&I::LocalSet(li));
    f.instruction(&I::Else);
    // A run: as many cells as are left before any line's place changes.
    f.instruction(&I::LocalGet(p_len));
    f.instruction(&I::LocalGet(li));
    f.instruction(&I::I32Sub);
    f.instruction(&I::LocalSet(lcount));
    for (l, &(s, c)) in lines.iter().enumerate() {
        if s == 1 {
            f.instruction(&I::I32Const(c as i32 - i32::from(edged[l])));
            f.instruction(&I::LocalGet(jl[l]));
        } else {
            f.instruction(&I::I32Const(s as i32));
            f.instruction(&I::LocalGet(within[l]));
        }
        f.instruction(&I::I32Sub);
        // Cells left along this line, in its run place's local (set below).
        f.instruction(&I::LocalTee(run_j[l]));
        f.instruction(&I::LocalGet(lcount));
        f.instruction(&I::LocalGet(run_j[l]));
        f.instruction(&I::LocalGet(lcount));
        f.instruction(&I::I32LtU);
        f.instruction(&I::Select);
        f.instruction(&I::LocalSet(lcount));
        f.instruction(&I::LocalGet(jl[l]));
        f.instruction(&I::LocalSet(run_j[l]));
    }
    f.instruction(&I::LocalGet(li));
    f.instruction(&I::LocalGet(lcount));
    f.instruction(&I::I32Add);
    f.instruction(&I::LocalSet(lend));
    f.instruction(&I::Block(BlockType::Empty));
    f.instruction(&I::Loop(BlockType::Empty));
    f.instruction(&I::LocalGet(li));
    f.instruction(&I::LocalGet(lend));
    f.instruction(&I::I32GeS);
    f.instruction(&I::BrIf(1));
    f.instruction(&I::LocalGet(p_lo));
    f.instruction(&I::LocalGet(li));
    f.instruction(&I::I32Add);
    f.instruction(&I::LocalSet(lk));
    emit(&mut f, true, &run_j);
    for (l, &(s, _)) in lines.iter().enumerate() {
        if s == 1 {
            f.instruction(&I::LocalGet(run_j[l]));
            f.instruction(&I::I32Const(1));
            f.instruction(&I::I32Add);
            f.instruction(&I::LocalSet(run_j[l]));
        }
    }
    f.instruction(&I::LocalGet(li));
    f.instruction(&I::I32Const(1));
    f.instruction(&I::I32Add);
    f.instruction(&I::LocalSet(li));
    f.instruction(&I::Br(0));
    f.instruction(&I::End);
    f.instruction(&I::End);
    advance(&mut f, lcount);
    f.instruction(&I::End); // if
    f.instruction(&I::Br(0));
    f.instruction(&I::End); // loop
    f.instruction(&I::End); // block
    f.instruction(&I::End); // function
    let mut code = CodeSection::new();
    code.function(&f);
    let mut module = Module::new();
    module
        .section(&types)
        .section(&imports)
        .section(&functions)
        .section(&memories)
        .section(&exports)
        .section(&code);
    module.finish()
}

/// Compile a phase: generate its module, compile and instantiate it.
pub fn compile(engine: &wasmtime::Engine, view: &PhaseView) -> Arc<dyn PhaseKernel> {
    let bytes = module(view);
    let module = wasmtime::Module::new(engine, &bytes).unwrap();
    let mut store = wasmtime::Store::new(engine, ());
    let call = wasmtime::Func::wrap(&mut store, |code: i32, a: f64, b: f64| {
        host_call(i64::from(code), a, b)
    });
    let instance = wasmtime::Instance::new(&mut store, &module, &[call.into()]).unwrap();
    let memory = instance.get_memory(&mut store, "memory").unwrap();
    let func = instance.get_func(&mut store, "phase").unwrap();
    Arc::new(Kernel {
        view: view.clone(),
        run: Mutex::new(Instance {
            store,
            memory,
            func,
        }),
    })
}
