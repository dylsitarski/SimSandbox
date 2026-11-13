//! A sparse LU on the matrix's own symmetric pattern, without row
//! interchanges (ADR 0087): multifrontal and supernodal, ordered by nested
//! dissection or approximate minimum degree on the graph of A + Aᵀ,
//! whichever needs fewer operations. A pivot that is zero, not finite, or
//! smaller than [`PIVOT_THRESHOLD`] of its column refuses the
//! factorisation, and the caller falls back to an LU with partial
//! pivoting. Everything runs on one thread, in a fixed order, so the same
//! matrix gives the same bits every time.

use faer::linalg::matmul::matmul;
use faer::reborrow::{Reborrow, ReborrowMut};
use faer::{Accum, MatMut, MatRef, Par};

/// A pivot smaller than this fraction of the largest entry below it in its
/// column refuses the factorisation.
pub const PIVOT_THRESHOLD: f64 = 1e-2;

/// Updates with fewer multiplications than this are done by plain loops.
const SMALL_UPDATE: usize = 4096;

/// Nested dissection is tried when minimum degree needs more operations
/// than this per column.
const FILLS: f64 = 32.0;

/// Parts of a nested dissection at most this large are not split further.
const LEAF: usize = 64;

/// Supernodes are merged with their parent column while they have at most
/// this many columns and the explicit zeros stay below a fifth of them.
const RELAX_COLUMNS: usize = 32;

struct Snode {
    /// The first column (in the factorisation's order) and how many.
    first: usize,
    cols: usize,
    /// The front's rows, in the factorisation's order: its columns, then
    /// the rows below them.
    rows: Vec<usize>,
    /// The supernode its update goes to, and where each of its update rows
    /// lies among that supernode's rows.
    parent: Option<usize>,
    to_parent: Vec<usize>,
    /// Where its factor starts in [`Numeric::values`].
    at: usize,
}

/// The structure of an LU: the ordering, the supernodes (children before
/// parents) and where each entry of A goes.
pub struct Symbolic {
    n: usize,
    /// `perm[k]`: the original index at place `k`.
    perm: Vec<usize>,
    snodes: Vec<Snode>,
    /// Each supernode's children.
    children: Vec<Vec<usize>>,
    /// For each supernode, the entries of A it takes: (place in its front,
    /// index into A's values), in `assemble_ptr[s]..assemble_ptr[s + 1]`.
    assemble: Vec<(usize, usize)>,
    assemble_ptr: Vec<usize>,
    /// Values a factorisation holds.
    size: usize,
    /// What a solve needs of each supernode, together, and the rows below
    /// their columns, one after another.
    plan: Vec<Step>,
    below: Vec<usize>,
}

struct Step {
    first: usize,
    cols: usize,
    m: usize,
    at: usize,
    below: usize,
}

/// The factors: for each supernode its columns (the front's rows by its
/// columns, L below the diagonal with a unit diagonal, U on and above)
/// then U's rows to the right of them.
pub struct Numeric {
    values: Vec<f64>,
}

impl Symbolic {
    /// From A's pattern by columns (`col_ptr`, `row_idx`, each column's rows
    /// ascending, the diagonal included or not).
    pub fn new(n: usize, col_ptr: &[usize], row_idx: &[usize]) -> Self {
        let adj = symmetric_graph(n, col_ptr, row_idx);
        // Minimum degree, or nested dissection when that fills in (as on
        // 2D and 3D grids and meshes) and it needs a tenth fewer
        // operations. Minimum degree keeps a chain in order, so solves walk
        // memory forwards.
        let md = Ordered::new(&adj, minimum_degree(n, &adj));
        let o = if md.flops > FILLS * n as f64 {
            let nd = Ordered::new(&adj, dissect(&adj));
            if nd.flops < 0.9 * md.flops {
                nd
            } else {
                md
            }
        } else {
            md
        };
        let snodes = supernodes(&o);
        let mut inv = vec![0; n];
        for (k, &i) in o.perm.iter().enumerate() {
            inv[i] = k;
        }
        let mut of_col = vec![0; n];
        for (s, sn) in snodes.iter().enumerate() {
            of_col[sn.first..sn.first + sn.cols].fill(s);
        }
        let mut lists: Vec<Vec<(usize, usize)>> = vec![Vec::new(); snodes.len()];
        for j in 0..n {
            for (e, &i) in row_idx[col_ptr[j]..col_ptr[j + 1]].iter().enumerate() {
                let (r, c) = (inv[i], inv[j]);
                let s = of_col[r.min(c)];
                let sn = &snodes[s];
                let m = sn.rows.len();
                let ri = sn.rows.binary_search(&r).expect("in the front");
                let ci = sn.rows.binary_search(&c).expect("in the front");
                lists[s].push((ri + ci * m, col_ptr[j] + e));
            }
        }
        let mut assemble_ptr = vec![0];
        let mut assemble = Vec::new();
        for l in lists {
            assemble.extend(l);
            assemble_ptr.push(assemble.len());
        }
        let size = snodes.last().map_or(0, |s| {
            s.at + s.rows.len() * s.cols + s.cols * (s.rows.len() - s.cols)
        });
        let mut children = vec![Vec::new(); snodes.len()];
        for (c, sn) in snodes.iter().enumerate() {
            if let Some(p) = sn.parent {
                children[p].push(c);
            }
        }
        let mut plan = Vec::with_capacity(snodes.len());
        let mut below = Vec::new();
        for sn in &snodes {
            plan.push(Step {
                first: sn.first,
                cols: sn.cols,
                m: sn.rows.len(),
                at: sn.at,
                below: below.len(),
            });
            below.extend(&sn.rows[sn.cols..]);
        }
        Self {
            n,
            perm: o.perm,
            snodes,
            children,
            plan,
            below,
            assemble,
            assemble_ptr,
            size,
        }
    }

    /// Factorise A (its values in the pattern's order), or `None` when a
    /// pivot is too small.
    pub fn factorise(&self, values: &[f64]) -> Option<Numeric> {
        let mut out = vec![0.0; self.size];
        let mut updates: Vec<Option<Vec<f64>>> = (0..self.snodes.len()).map(|_| None).collect();
        let mut front = Vec::new();
        // Update buffers given back by parents, used again.
        let mut spare: Vec<Vec<f64>> = Vec::new();
        for (s, sn) in self.snodes.iter().enumerate() {
            let m = sn.rows.len();
            let k = sn.cols;
            front.clear();
            front.resize(m * m, 0.0);
            for &(at, e) in &self.assemble[self.assemble_ptr[s]..self.assemble_ptr[s + 1]] {
                front[at] += values[e];
            }
            // The children's updates (children come before their parent).
            for &c in &self.children[s] {
                let child = &self.snodes[c];
                let u = updates[c].take().expect("each child's update once");
                let mc = child.to_parent.len();
                for (j, &pj) in child.to_parent.iter().enumerate() {
                    let col = &mut front[pj * m..(pj + 1) * m];
                    for (i, &pi) in child.to_parent.iter().enumerate() {
                        col[pi] += u[i + j * mc];
                    }
                }
                spare.push(u);
            }
            if !factor_panel(&mut front, m, k) {
                return None;
            }
            // Keep the factor: the panel (m × k), then U's rows to its right.
            let panel = &mut out[sn.at..sn.at + m * k];
            panel.copy_from_slice(&front[..m * k]);
            let r = m - k;
            let upper_at = sn.at + m * k;
            for c in 0..r {
                let src = &front[(k + c) * m..(k + c) * m + k];
                out[upper_at + c * k..upper_at + (c + 1) * k].copy_from_slice(src);
            }
            if r == 0 {
                continue;
            }
            // The update to the parent: F22 − L21·U12.
            let mut u = spare.pop().unwrap_or_default();
            u.clear();
            u.reserve(r * r);
            for c in 0..r {
                u.extend_from_slice(&front[k + (k + c) * m..(k + c + 1) * m]);
            }
            if k * r * r < SMALL_UPDATE {
                // Too small for a product's set-up to pay.
                let l21 = &out[sn.at + k..];
                let u12 = &out[upper_at..upper_at + k * r];
                for c in 0..r {
                    let col = &mut u[c * r..(c + 1) * r];
                    for j in 0..k {
                        let f = u12[j + c * k];
                        if f == 0.0 {
                            continue;
                        }
                        let l = &l21[j * m..j * m + r];
                        for i in 0..r {
                            col[i] -= l[i] * f;
                        }
                    }
                }
            } else {
                let l21 = MatRef::from_column_major_slice_with_stride(&out[sn.at + k..], r, k, m);
                let u12 = MatRef::from_column_major_slice(&out[upper_at..upper_at + k * r], k, r);
                matmul(
                    MatMut::from_column_major_slice_mut(&mut u, r, r),
                    Accum::Add,
                    l21,
                    u12,
                    -1.0,
                    Par::Seq,
                );
            }
            updates[s] = Some(u);
        }
        Some(Numeric { values: out })
    }

    /// Solve A x = b in place.
    pub fn solve(&self, f: &Numeric, b: &mut [f64]) {
        let mut y: Vec<f64> = self.perm.iter().map(|&i| b[i]).collect();
        let v = &f.values[..];
        // L y = b, supernode by supernode.
        for p in &self.plan {
            let (m, k, f0) = (p.m, p.cols, p.first);
            let below = &self.below[p.below..p.below + m - k];
            if k == 1 {
                // One column (most supernodes of a chain or a thin mesh).
                let yj = y[f0];
                for (&r, l) in below.iter().zip(&v[p.at + 1..p.at + m]) {
                    y[r] -= l * yj;
                }
                continue;
            }
            for j in 0..k {
                let yj = y[f0 + j];
                if yj == 0.0 {
                    continue;
                }
                let col = &v[p.at + j * m..p.at + (j + 1) * m];
                for (yi, l) in y[f0 + j + 1..f0 + k].iter_mut().zip(&col[j + 1..k]) {
                    *yi -= l * yj;
                }
                for (&r, l) in below.iter().zip(&col[k..]) {
                    y[r] -= l * yj;
                }
            }
        }
        // U x = y, last supernode first.
        for p in self.plan.iter().rev() {
            let (m, k, f0) = (p.m, p.cols, p.first);
            let below = &self.below[p.below..p.below + m - k];
            let upper = &v[p.at + m * k..p.at + m * k + k * (m - k)];
            if k == 1 {
                let mut yj = y[f0];
                for (&r, u) in below.iter().zip(upper) {
                    yj -= u * y[r];
                }
                y[f0] = yj / v[p.at];
                continue;
            }
            for (&r, u) in below.iter().zip(upper.chunks_exact(k)) {
                let yc = y[r];
                if yc == 0.0 {
                    continue;
                }
                for (yj, uj) in y[f0..f0 + k].iter_mut().zip(u) {
                    *yj -= uj * yc;
                }
            }
            for j in (0..k).rev() {
                let col = &v[p.at + j * m..p.at + j * m + j + 1];
                y[f0 + j] /= col[j];
                let yj = y[f0 + j];
                for (yi, u) in y[f0..f0 + j].iter_mut().zip(&col[..j]) {
                    *yi -= u * yj;
                }
            }
        }
        for (k, &i) in self.perm.iter().enumerate() {
            b[i] = y[k];
        }
    }

    pub fn n(&self) -> usize {
        self.n
    }

    /// Values the factors hold (fill included).
    pub fn factor_size(&self) -> usize {
        self.size
    }
}

/// Columns factorised one by one before a block's products.
const BLOCK: usize = 32;

/// Factorise the first `k` columns of the `m × m` front (column-major) in
/// place: L below the diagonal of those columns (unit diagonal), U on and
/// above it and in their rows to the right. The rest of the front is left
/// for the update. Blocks of [`BLOCK`] columns: each factorised column by
/// column, then the panel to its right updated by products. False when a
/// pivot is too small.
fn factor_panel(front: &mut [f64], m: usize, k: usize) -> bool {
    let mut jb = 0;
    while jb < k {
        let je = (jb + BLOCK).min(k);
        for j in jb..je {
            let p = front[j + j * m];
            let below = front[j + 1 + j * m..(j + 1) * m]
                .iter()
                .fold(0.0f64, |a, v| a.max(v.abs()));
            if !p.is_finite() || p == 0.0 || p.abs() < PIVOT_THRESHOLD * below {
                return false;
            }
            for i in j + 1..m {
                front[i + j * m] /= p;
            }
            // The block's other columns, and its rows to the right.
            for c in j + 1..m {
                let end = if c < je { m } else { je };
                let f = front[j + c * m];
                if f == 0.0 || end <= j + 1 {
                    continue;
                }
                let (left, right) = front.split_at_mut(c * m);
                let l = &left[j * m..j * m + end];
                let col = &mut right[..end];
                for i in j + 1..end {
                    col[i] -= l[i] * f;
                }
            }
        }
        if je < m {
            // Below the block and to its right: rows je.. of the panel's
            // columns je..k, and rows je..k of the columns beyond the panel.
            let all = MatMut::from_column_major_slice_mut(front, m, m);
            let (left, right) = all.split_at_col_mut(je);
            let (top, mut bottom) = right.split_at_row_mut(je);
            let l = left.rb().submatrix(je, jb, m - je, je - jb);
            let u = top.rb().submatrix(jb, 0, je - jb, m - je);
            if je < k {
                matmul(
                    bottom.rb_mut().submatrix_mut(0, 0, m - je, k - je),
                    Accum::Add,
                    l,
                    u.submatrix(0, 0, je - jb, k - je),
                    -1.0,
                    Par::Seq,
                );
            }
            if k < m && je < k {
                matmul(
                    bottom.rb_mut().submatrix_mut(0, k - je, k - je, m - k),
                    Accum::Add,
                    l.submatrix(0, 0, k - je, je - jb),
                    u.submatrix(0, k - je, je - jb, m - k),
                    -1.0,
                    Par::Seq,
                );
            }
        }
        jb = je;
    }
    true
}

/// The graph of A + Aᵀ without its diagonal: each vertex's neighbours,
/// ascending.
fn symmetric_graph(n: usize, col_ptr: &[usize], row_idx: &[usize]) -> Vec<Vec<usize>> {
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for j in 0..n {
        for &i in &row_idx[col_ptr[j]..col_ptr[j + 1]] {
            if i != j {
                adj[i].push(j);
                adj[j].push(i);
            }
        }
    }
    for a in &mut adj {
        a.sort_unstable();
        a.dedup();
    }
    adj
}

/// An ordering with its elimination tree, postordered, and the structure
/// of each column of L below the diagonal.
struct Ordered {
    perm: Vec<usize>,
    parent: Vec<Option<usize>>,
    structure: Vec<Vec<usize>>,
    flops: f64,
}

impl Ordered {
    fn new(adj: &[Vec<usize>], perm: Vec<usize>) -> Self {
        let n = adj.len();
        let parent = etree(adj, &perm);
        // Postorder, so each subtree's columns are consecutive.
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut roots = Vec::new();
        for (k, p) in parent.iter().enumerate() {
            match *p {
                Some(p) => children[p].push(k),
                None => roots.push(k),
            }
        }
        let mut post = Vec::with_capacity(n);
        let mut stack: Vec<(usize, usize)> = roots.iter().rev().map(|&r| (r, 0)).collect();
        while let Some((v, i)) = stack.pop() {
            if i < children[v].len() {
                stack.push((v, i + 1));
                stack.push((children[v][i], 0));
            } else {
                post.push(v);
            }
        }
        let perm: Vec<usize> = post.iter().map(|&k| perm[k]).collect();
        let parent = etree(adj, &perm);
        let mut inv = vec![0; n];
        for (k, &i) in perm.iter().enumerate() {
            inv[i] = k;
        }
        let mut structure: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (k, p) in parent.iter().enumerate() {
            if let Some(p) = *p {
                kids[p].push(k);
            }
        }
        let mut mark = vec![usize::MAX; n];
        let mut flops = 0.0;
        for k in 0..n {
            let mut s: Vec<usize> = Vec::new();
            mark[k] = k;
            for &j in &adj[perm[k]] {
                let r = inv[j];
                if r > k && mark[r] != k {
                    mark[r] = k;
                    s.push(r);
                }
            }
            for &c in &kids[k] {
                for &r in &structure[c] {
                    if r > k && mark[r] != k {
                        mark[r] = k;
                        s.push(r);
                    }
                }
            }
            s.sort_unstable();
            let len = s.len() as f64;
            flops += len * len;
            structure[k] = s;
        }
        Self {
            perm,
            parent,
            structure,
            flops,
        }
    }
}

/// The elimination tree of the graph `adj` in the order `perm` (Liu's
/// algorithm with path compression).
fn etree(adj: &[Vec<usize>], perm: &[usize]) -> Vec<Option<usize>> {
    let n = adj.len();
    let mut inv = vec![0; n];
    for (k, &i) in perm.iter().enumerate() {
        inv[i] = k;
    }
    let mut parent = vec![None; n];
    let mut ancestor: Vec<Option<usize>> = vec![None; n];
    for k in 0..n {
        for &j in &adj[perm[k]] {
            let mut r = inv[j];
            if r >= k {
                continue;
            }
            // Climb from r to its root, pointing everything on the way at k.
            while let Some(a) = ancestor[r] {
                if a == k {
                    break;
                }
                ancestor[r] = Some(k);
                r = a;
            }
            if ancestor[r].is_none() {
                ancestor[r] = Some(k);
                parent[r] = Some(k);
            }
        }
    }
    parent
}

/// Supernodes from consecutive columns along a chain of parents, merged
/// while they share their rows below, or while small with few explicit
/// zeros.
fn supernodes(o: &Ordered) -> Vec<Snode> {
    let n = o.perm.len();
    let count: Vec<usize> = o.structure.iter().map(Vec::len).collect();
    let mut starts = Vec::new();
    let mut k = 0;
    while k < n {
        let first = k;
        // Stored and true entries below the diagonal of the columns so far.
        let mut true_entries = count[k];
        while k + 1 < n && o.parent[k] == Some(k + 1) {
            let cols = k + 2 - first;
            let stored = (0..cols).map(|i| cols - 1 - i).sum::<usize>() + cols * count[k + 1];
            let truth = true_entries + count[k + 1];
            let exact = count[k] == count[k + 1] + 1;
            let zeros = stored - truth.min(stored);
            if !(exact && zeros == 0 || cols <= RELAX_COLUMNS && 5 * zeros <= stored) {
                break;
            }
            true_entries = truth;
            k += 1;
        }
        starts.push((first, k + 1 - first));
        k += 1;
    }
    let mut of_col = vec![0; n];
    for (s, &(f, c)) in starts.iter().enumerate() {
        of_col[f..f + c].fill(s);
    }
    let mut snodes: Vec<Snode> = Vec::with_capacity(starts.len());
    let mut at = 0;
    for &(first, cols) in &starts {
        let last = first + cols - 1;
        let mut rows: Vec<usize> = (first..=last).collect();
        rows.extend(&o.structure[last]);
        // A merged column's rows below are within its last column's.
        let m = rows.len();
        snodes.push(Snode {
            first,
            cols,
            rows,
            parent: o.parent[last].map(|p| of_col[p]),
            to_parent: Vec::new(),
            at,
        });
        at += m * cols + cols * (m - cols);
    }
    for s in 0..snodes.len() {
        let Some(p) = snodes[s].parent else { continue };
        let map: Vec<usize> = snodes[s].rows[snodes[s].cols..]
            .iter()
            .map(|r| {
                snodes[p]
                    .rows
                    .binary_search(r)
                    .expect("rows within the parent's")
            })
            .collect();
        snodes[s].to_parent = map;
    }
    snodes
}

/// Nested dissection by level sets of breadth-first searches: each part is
/// split at the middle level of a search from a far vertex; the parts come
/// first, the separator after them.
fn dissect(adj: &[Vec<usize>]) -> Vec<usize> {
    let n = adj.len();
    let mut order = Vec::with_capacity(n);
    let mut part = vec![usize::MAX; n];
    let mut level = vec![usize::MAX; n];
    enum Task {
        Split(Vec<usize>),
        Emit(Vec<usize>),
    }
    let mut tasks = vec![Task::Split((0..n).collect())];
    let mut id = 0;
    while let Some(t) = tasks.pop() {
        let nodes = match t {
            Task::Emit(v) => {
                order.extend(v);
                continue;
            }
            Task::Split(v) => v,
        };
        if nodes.len() <= LEAF {
            order.extend(nodes);
            continue;
        }
        id += 1;
        for &v in &nodes {
            part[v] = id;
        }
        let search = |start: usize, level: &mut [usize]| -> Vec<usize> {
            let mut q = vec![start];
            level[start] = 0;
            let mut h = 0;
            while h < q.len() {
                let v = q[h];
                h += 1;
                for &w in &adj[v] {
                    if part[w] == id && level[w] == usize::MAX {
                        level[w] = level[v] + 1;
                        q.push(w);
                    }
                }
            }
            q
        };
        let clear = |q: &[usize], level: &mut [usize]| {
            for &v in q {
                level[v] = usize::MAX;
            }
        };
        let mut q = search(nodes[0], &mut level);
        if q.len() < nodes.len() {
            // Not connected: this component, then the rest, apart.
            let rest: Vec<usize> = nodes
                .iter()
                .copied()
                .filter(|&v| level[v] == usize::MAX)
                .collect();
            clear(&q, &mut level);
            for &v in &nodes {
                part[v] = 0;
            }
            tasks.push(Task::Split(rest));
            tasks.push(Task::Split(q));
            continue;
        }
        // A far vertex: search again from the farthest while it reaches further.
        for _ in 0..4 {
            let far = *q.last().expect("not empty");
            let depth = level[far];
            clear(&q, &mut level);
            let q2 = search(far, &mut level);
            if level[*q2.last().expect("not empty")] <= depth {
                clear(&q2, &mut level);
                q = search(far, &mut level);
                break;
            }
            q = q2;
        }
        let depth = level[*q.last().expect("not empty")];
        if depth < 2 {
            clear(&q, &mut level);
            for &v in &nodes {
                part[v] = 0;
            }
            order.extend(nodes);
            continue;
        }
        let mut count = vec![0usize; depth + 1];
        for &v in &q {
            count[level[v]] += 1;
        }
        let (mut seen, mut mid) = (0, 1);
        for (l, c) in count.iter().enumerate() {
            seen += c;
            if 2 * seen >= q.len() {
                mid = l.clamp(1, depth - 1);
                break;
            }
        }
        let (mut lo, mut sep, mut hi) = (Vec::new(), Vec::new(), Vec::new());
        for &v in &q {
            match level[v].cmp(&mid) {
                std::cmp::Ordering::Less => lo.push(v),
                std::cmp::Ordering::Equal => sep.push(v),
                std::cmp::Ordering::Greater => hi.push(v),
            }
        }
        clear(&q, &mut level);
        for &v in &nodes {
            part[v] = 0;
        }
        tasks.push(Task::Emit(sep));
        tasks.push(Task::Split(hi));
        tasks.push(Task::Split(lo));
    }
    order
}

/// Approximate minimum degree (faer's AMD) on the graph.
fn minimum_degree(n: usize, adj: &[Vec<usize>]) -> Vec<usize> {
    use faer::dyn_stack::{MemBuffer, MemStack};
    use faer::sparse::linalg::amd;
    use faer::sparse::SymbolicSparseColMatRef;
    let mut col_ptr = vec![0];
    let mut row_idx = Vec::new();
    for (j, a) in adj.iter().enumerate() {
        row_idx.extend(a.iter().copied().filter(|&i| i < j));
        row_idx.push(j);
        col_ptr.push(row_idx.len());
    }
    let pattern = SymbolicSparseColMatRef::new_checked(n, n, &col_ptr, None, &row_idx);
    let mut fwd = vec![0usize; n];
    let mut inv = vec![0usize; n];
    let mut mem = MemBuffer::new(amd::order_scratch::<usize>(n, row_idx.len()));
    match amd::order(
        &mut fwd,
        &mut inv,
        pattern,
        Default::default(),
        MemStack::new(&mut mem),
    ) {
        Ok(_) => fwd,
        Err(_) => (0..n).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A in compressed columns from (row, col, value), rows ascending.
    fn csc(n: usize, mut t: Vec<(usize, usize, f64)>) -> (Vec<usize>, Vec<usize>, Vec<f64>) {
        t.sort_by_key(|&(i, j, _)| (j, i));
        let mut col_ptr = vec![0; n + 1];
        for &(_, j, _) in &t {
            col_ptr[j + 1] += 1;
        }
        for j in 0..n {
            col_ptr[j + 1] += col_ptr[j];
        }
        (
            col_ptr,
            t.iter().map(|e| e.0).collect(),
            t.iter().map(|e| e.2).collect(),
        )
    }

    fn residual(n: usize, cp: &[usize], ri: &[usize], v: &[f64], x: &[f64], b: &[f64]) -> f64 {
        let mut r = b.to_vec();
        for j in 0..n {
            for e in cp[j]..cp[j + 1] {
                r[ri[e]] -= v[e] * x[j];
            }
        }
        r.iter().fold(0.0f64, |a, v| a.max(v.abs()))
    }

    /// Two coupled fields on a periodic m × m grid, like a reaction–
    /// diffusion system's I − c·h·J, unsymmetric.
    fn grid(m: usize) -> (usize, Vec<(usize, usize, f64)>) {
        let cells = m * m;
        let mut t = Vec::new();
        for y in 0..m {
            for x in 0..m {
                let k = x + m * y;
                let nb = [
                    (x + 1) % m + m * y,
                    (x + m - 1) % m + m * y,
                    x + m * ((y + 1) % m),
                    x + m * ((y + m - 1) % m),
                ];
                for (f, d) in [(0, 0.3), (cells, 0.15)] {
                    t.push((f + k, f + k, 1.0 + 4.0 * d + 0.01 * (k % 7) as f64));
                    for (q, &j) in nb.iter().enumerate() {
                        t.push((f + k, f + j, -d * (1.0 + 0.1 * q as f64)));
                    }
                    t.push((
                        f + k,
                        (f + cells) % (2 * cells) + k,
                        0.05 * (1 + k % 3) as f64,
                    ));
                }
            }
        }
        (2 * cells, t)
    }

    #[test]
    fn solves_grids_and_chains_to_rounding() {
        for m in [3, 8, 31] {
            let (n, t) = grid(m);
            let (cp, ri, v) = csc(n, t);
            let s = Symbolic::new(n, &cp, &ri);
            let f = s.factorise(&v).expect("diagonally dominant");
            let b: Vec<f64> = (0..n).map(|i| ((i * 37) % 11) as f64 - 5.0).collect();
            let mut x = b.clone();
            s.solve(&f, &mut x);
            assert!(residual(n, &cp, &ri, &v, &x, &b) < 1e-12, "m = {m}");
            // The same matrix gives the same bits.
            let mut y = b.clone();
            s.solve(&s.factorise(&v).unwrap(), &mut y);
            assert_eq!(
                x.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                y.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            );
        }
        // A tridiagonal chain and two disconnected blocks.
        let n = 300;
        let mut t = Vec::new();
        for i in 0..n {
            t.push((i, i, 3.0));
            if i + 1 < n && i != 149 {
                t.push((i, i + 1, -1.0));
                t.push((i + 1, i, -0.5));
            }
        }
        let (cp, ri, v) = csc(n, t);
        let s = Symbolic::new(n, &cp, &ri);
        let b = vec![1.0; n];
        let mut x = b.clone();
        s.solve(&s.factorise(&v).unwrap(), &mut x);
        assert!(residual(n, &cp, &ri, &v, &x, &b) < 1e-13);
    }

    #[test]
    fn a_small_pivot_refuses() {
        // [[0, 1], [1, 0]] needs a row interchange.
        let (cp, ri, v) = csc(2, vec![(0, 1, 1.0), (1, 0, 1.0), (0, 0, 0.0), (1, 1, 0.0)]);
        let s = Symbolic::new(2, &cp, &ri);
        assert!(s.factorise(&v).is_none());
        let (cp, ri, v) = csc(2, vec![(0, 1, 1.0), (1, 0, 1.0), (0, 0, 1e-5), (1, 1, 1.0)]);
        assert!(Symbolic::new(2, &cp, &ri).factorise(&v).is_none());
    }

    #[test]
    fn nested_dissection_beats_minimum_degree_on_large_grids() {
        let (n, t) = grid(64);
        let (cp, ri, _) = csc(n, t);
        let adj = symmetric_graph(n, &cp, &ri);
        let nd = Ordered::new(&adj, dissect(&adj));
        let md = Ordered::new(&adj, minimum_degree(n, &adj));
        let mut seen = nd.perm.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..n).collect::<Vec<_>>());
        // On a 2D grid nested dissection needs a fifth fewer operations
        // (5.2e7 against 6.8e7 at 64 × 64), so it is the one used.
        assert!(md.flops > FILLS * n as f64);
        assert!(nd.flops < 0.8 * md.flops, "{} {}", nd.flops, md.flops);
    }
}
