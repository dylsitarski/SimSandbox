//! Refinement of meshes (ADR 0085 step 2): simplices by newest-vertex
//! bisection (Maubach's rule, conforming, any dimension), boxes into 2^d
//! children (hanging sides, neighbours across a side at most one level
//! apart), and merging what a split made.
//!
//! A refined table of cells holds, after its corners, each cell's `level`,
//! `parent` (a row id of its parents' table, -1 for a starting cell) and
//! `child` (which child of its parent), then its states. The parents'
//! table holds each cell split and not merged back: its corners, level,
//! parent and child. The first child keeps its parent's row and id, so a
//! split then a merge gives the cell back as it was.

use crate::mesh::CellShape;
use simsandbox_core::table::Table;
use std::collections::{HashMap, HashSet};

/// How a column's value passes to the rows a split or merge makes.
#[derive(Debug, Clone, PartialEq)]
pub enum Transfer {
    /// Each child the parent's value; on merge the first child's.
    Copy,
    /// Each child its share by measure: the parent's value over the number
    /// of children (exact: a power of two), so the sum is kept.
    Share,
    /// On merge the mean of the children; a new point the mean of the two
    /// points it lies between.
    Mean,
    /// On merge the children's sum.
    Sum,
    /// A value per row as the rule was applied: a new row takes the value
    /// of the row (cell) that made it.
    Given(Vec<f64>),
}

/// A table of cells over a table of points, with its parents.
pub struct Mesh<'a> {
    pub shape: CellShape,
    pub cells: &'a mut Table,
    pub parents: &'a mut Table,
    pub points: &'a mut Table,
    /// The points' columns before their states.
    pub point_at: usize,
    pub max_cells: usize,
    pub max_points: usize,
}

/// The transfers of a rule: one per state over the cells, one per state
/// over the points, in column order.
pub struct Transfers {
    pub cells: Vec<Transfer>,
    pub points: Vec<Transfer>,
}

/// Each corner's place along the axes of a box shape, as bits: Gmsh's
/// order for quads and hexahedra, binary for `box d`.
pub fn box_codes(shape: CellShape) -> Option<Vec<usize>> {
    match shape {
        CellShape::Quad => Some(vec![0, 1, 3, 2]),
        CellShape::Hexa => Some(vec![0, 1, 3, 2, 4, 5, 7, 6]),
        CellShape::Box(d) => Some((0..1usize << d).collect()),
        _ => None,
    }
}

/// For a box shape, each side's axis and end (0 or 1), in the order of
/// [`CellShape::sides`].
pub fn box_side_axes(shape: CellShape) -> Option<Vec<(usize, usize)>> {
    let codes = box_codes(shape)?;
    let (_, faces) = shape.sides()?;
    Some(
        faces
            .iter()
            .map(|f| {
                (0..shape.dim())
                    .find_map(|i| {
                        let s = (codes[f[0]] >> i) & 1;
                        f.iter()
                            .all(|&p| (codes[p] >> i) & 1 == s)
                            .then_some((i, s))
                    })
                    .expect("a box side lies at one end of one axis")
            })
            .collect(),
    )
}

/// Maubach's tag: the refinement edge of a d-simplex at `level` joins its
/// corners 0 and `tag`; starting cells (level 0) have tag d.
fn tag(d: usize, level: usize) -> usize {
    d - level % d
}

/// The most nested refinements one split may need before it is made.
const DEPTH: usize = 4096;

fn corners_of(t: &Table, r: usize, n: usize) -> Vec<u64> {
    (0..n).map(|c| t.column(c)[r] as u64).collect()
}

fn sorted(mut v: Vec<u64>) -> Vec<u64> {
    v.sort_unstable();
    v
}

struct Work<'m, 'a> {
    m: &'m mut Mesh<'a>,
    t: &'m Transfers,
    nc: usize,
    d: usize,
    codes: Vec<usize>,
    faces: Vec<Vec<usize>>,
    axes: Vec<(usize, usize)>,
    point_row: HashMap<u64, usize>,
    parent_row: HashMap<u64, usize>,
    /// Each current row's row when the rule was applied.
    origin: Vec<usize>,
    /// Simplices: the rows with each point as a corner, in row order.
    by_point: HashMap<u64, Vec<usize>>,
    /// Boxes: the rows with each side (corners sorted).
    leaf_faces: HashMap<Vec<u64>, Vec<usize>>,
    /// Boxes: points made by splits, by the corners of the part of the
    /// parent they are the middle of (sorted).
    mid: HashMap<Vec<u64>, u64>,
}

impl<'m, 'a> Work<'m, 'a> {
    fn new(m: &'m mut Mesh<'a>, t: &'m Transfers) -> Self {
        let nc = m.shape.corners();
        let d = m.shape.dim();
        let codes = box_codes(m.shape).unwrap_or_default();
        let faces = m.shape.sides().map(|s| s.1).unwrap_or_default();
        let axes = box_side_axes(m.shape).unwrap_or_default();
        let point_row = m
            .points
            .ids()
            .iter()
            .enumerate()
            .map(|(k, &i)| (i, k))
            .collect();
        let parent_row = m
            .parents
            .ids()
            .iter()
            .enumerate()
            .map(|(k, &i)| (i, k))
            .collect();
        let origin = (0..m.cells.len()).collect();
        let mut w = Self {
            m,
            t,
            nc,
            d,
            codes,
            faces,
            axes,
            point_row,
            parent_row,
            origin,
            by_point: HashMap::new(),
            leaf_faces: HashMap::new(),
            mid: HashMap::new(),
        };
        for r in 0..w.m.cells.len() {
            w.index(r);
        }
        if !w.simplicial() {
            w.find_middles();
        }
        w
    }

    fn simplicial(&self) -> bool {
        self.m.shape.simplicial()
    }

    fn corners(&self, r: usize) -> Vec<u64> {
        corners_of(self.m.cells, r, self.nc)
    }

    fn level(&self, r: usize) -> usize {
        self.m.cells.column(self.nc)[r] as usize
    }

    fn parent(&self, r: usize) -> f64 {
        self.m.cells.column(self.nc + 1)[r]
    }

    fn child(&self, r: usize) -> usize {
        self.m.cells.column(self.nc + 2)[r] as usize
    }

    /// Row `r` in the indexes (simplices by point, boxes by side).
    fn index(&mut self, r: usize) {
        let c = self.corners(r);
        if self.simplicial() {
            for p in c {
                let rows = self.by_point.entry(p).or_default();
                if let Err(at) = rows.binary_search(&r) {
                    rows.insert(at, r);
                }
            }
        } else {
            for f in &self.faces {
                let key = sorted(f.iter().map(|&k| c[k]).collect());
                self.leaf_faces.entry(key).or_default().push(r);
            }
        }
    }

    /// Row `r` out of the indexes, before its corners change.
    fn unindex(&mut self, r: usize) {
        let c = self.corners(r);
        if self.simplicial() {
            for p in c {
                if let Some(rows) = self.by_point.get_mut(&p) {
                    rows.retain(|&q| q != r);
                }
            }
        } else {
            for f in &self.faces {
                let key = sorted(f.iter().map(|&k| c[k]).collect());
                if let Some(rows) = self.leaf_faces.get_mut(&key) {
                    rows.retain(|&q| q != r);
                    if rows.is_empty() {
                        self.leaf_faces.remove(&key);
                    }
                }
            }
        }
    }

    /// The point at `l` (a place in the parent's 3^d lattice, base 3) is
    /// the middle of the parent's corners spanning it.
    fn lattice_key(&self, pc: &[u64], l: usize) -> Vec<u64> {
        let mut parts = vec![0usize];
        let mut rest = l;
        for i in 0..self.d {
            let digit = rest % 3;
            rest /= 3;
            parts = match digit {
                0 => parts,
                2 => parts.into_iter().map(|q| q | 1 << i).collect(),
                _ => parts.iter().flat_map(|&q| [q, q | 1 << i]).collect(),
            };
        }
        sorted(parts.into_iter().map(|q| pc[q]).collect())
    }

    /// The parent's corners by their bits.
    fn by_code(&self, c: &[u64]) -> Vec<u64> {
        let mut pc = vec![0; c.len()];
        for (pos, &code) in self.codes.iter().enumerate() {
            pc[code] = c[pos];
        }
        pc
    }

    /// The lattice place of child `b`'s corner at position `pos`.
    fn place(&self, b: usize, pos: usize) -> usize {
        let code = self.codes[pos];
        (0..self.d)
            .fold((0, 1), |(l, w), i| {
                (l + w * (((b >> i) & 1) + ((code >> i) & 1)), w * 3)
            })
            .0
    }

    /// Boxes: the middles made by earlier splits, from the parents and
    /// their children (cells, or parents themselves).
    fn find_middles(&mut self) {
        let nc = self.nc;
        let mut kids: HashMap<u64, Vec<(usize, Vec<u64>)>> = HashMap::new();
        for r in 0..self.m.cells.len() {
            let p = self.parent(r);
            if p >= 0.0 {
                kids.entry(p as u64)
                    .or_default()
                    .push((self.child(r), self.corners(r)));
            }
        }
        let tree = &*self.m.parents;
        for r in 0..tree.len() {
            let p = tree.column(nc + 1)[r];
            if p >= 0.0 {
                kids.entry(p as u64)
                    .or_default()
                    .push((tree.column(nc + 2)[r] as usize, corners_of(tree, r, nc)));
            }
        }
        let ids = self.m.parents.ids().to_vec();
        for (r, &id) in ids.iter().enumerate() {
            let pc = self.by_code(&corners_of(self.m.parents, r, nc));
            for (b, c) in kids.get(&id).into_iter().flatten() {
                for (pos, &p) in c.iter().enumerate() {
                    let l = self.place(*b, pos);
                    if has_middle(l, self.d) {
                        let key = self.lattice_key(&pc, l);
                        self.mid.insert(key, p);
                    }
                }
            }
        }
    }

    /// A new point's states: the mean of points `a` and `b`, or the value
    /// given for the cell `r` making it.
    fn new_point(&mut self, a: u64, b: u64, r: usize) -> u64 {
        let at = self.m.point_at;
        let (ra, rb) = (self.point_row[&a], self.point_row[&b]);
        let mut row = vec![0.0; self.m.points.columns()];
        for (c, t) in self.t.points.iter().enumerate() {
            row[at + c] = match t {
                Transfer::Given(v) => v[self.origin[r]],
                _ => {
                    let col = self.m.points.column(at + c);
                    (col[ra] + col[rb]) / 2.0
                }
            };
        }
        let id = self.m.points.push(&row);
        self.point_row.insert(id, self.m.points.len() - 1);
        id
    }

    /// The cell states each of `n` children of row `r` take.
    fn child_values(&self, r: usize, n: usize) -> Vec<f64> {
        let at = self.nc + 3;
        self.t
            .cells
            .iter()
            .enumerate()
            .map(|(c, t)| {
                let v = self.m.cells.column(at + c)[r];
                match t {
                    Transfer::Share => v / n as f64,
                    Transfer::Given(g) => g[self.origin[r]],
                    _ => v,
                }
            })
            .collect()
    }

    /// Record row `r` (before it changes) as a parent: its parents' id.
    fn keep_parent(&mut self, r: usize) -> u64 {
        let nc = self.nc;
        let mut row = self.m.cells.row(r);
        row.truncate(nc + 3);
        let id = self.m.parents.push(&row);
        self.parent_row.insert(id, self.m.parents.len() - 1);
        id
    }

    /// Row `r` becomes the first child; the others are added after.
    fn replace(&mut self, r: usize, children: Vec<Vec<u64>>, level: usize, parent: u64) {
        let values = self.child_values(r, children.len());
        self.unindex(r);
        let nc = self.nc;
        for (b, c) in children.into_iter().enumerate() {
            let mut row: Vec<f64> = c.iter().map(|&p| p as f64).collect();
            row.extend([level as f64, parent as f64, b as f64]);
            row.extend_from_slice(&values);
            let at = if b == 0 {
                for (k, v) in row.iter().enumerate() {
                    self.m.cells.column_mut(k)[r] = *v;
                }
                r
            } else {
                self.m.cells.push(&row);
                self.origin.push(self.origin[r]);
                self.m.cells.len() - 1
            };
            debug_assert_eq!(self.m.cells.column(nc)[at] as usize, level);
            self.index(at);
        }
    }

    // ---- simplices ----

    fn refinement_edge(&self, r: usize) -> (u64, u64) {
        let c = self.corners(r);
        let k = tag(self.d, self.level(r));
        (c[0].min(c[k]), c[0].max(c[k]))
    }

    fn patch(&self, (a, b): (u64, u64)) -> Vec<usize> {
        let (Some(ra), Some(rb)) = (self.by_point.get(&a), self.by_point.get(&b)) else {
            return Vec::new();
        };
        ra.iter()
            .copied()
            .filter(|r| rb.binary_search(r).is_ok())
            .collect()
    }

    /// Refine cell `r` and whatever the closure needs first; false when a
    /// table is full.
    fn refine_simplex(&mut self, r: usize, depth: usize) -> Result<bool, String> {
        let level = self.level(r);
        loop {
            if self.level(r) != level {
                return Ok(true);
            }
            let e = self.refinement_edge(r);
            let patch = self.patch(e);
            if let Some(&q) = patch.iter().find(|&&q| self.refinement_edge(q) != e) {
                if depth > DEPTH {
                    return Err(not_labelled());
                }
                if !self.refine_simplex(q, depth + 1)? {
                    return Ok(false);
                }
                continue;
            }
            return Ok(self.bisect(e, &patch));
        }
    }

    /// Bisect every cell of `patch` at the middle of their common
    /// refinement edge `e`.
    fn bisect(&mut self, e: (u64, u64), patch: &[usize]) -> bool {
        if self.m.cells.len() + patch.len() > self.m.max_cells
            || self.m.points.len() + 1 > self.m.max_points
        {
            return false;
        }
        let z = self.new_point(e.0, e.1, patch[0]);
        for &r in patch {
            let c = self.corners(r);
            let level = self.level(r);
            let k = tag(self.d, level);
            let parent = self.keep_parent(r);
            let mut first: Vec<u64> = c[..k].to_vec();
            first.push(z);
            first.extend_from_slice(&c[k + 1..]);
            let mut second: Vec<u64> = c[1..=k].to_vec();
            second.push(z);
            second.extend_from_slice(&c[k + 1..]);
            self.replace(r, vec![first, second], level + 1, parent);
        }
        true
    }

    // ---- boxes ----

    /// A neighbour across a side one level coarser, which must be split
    /// first (2:1).
    fn coarser_neighbour(&self, r: usize) -> Option<usize> {
        let p = self.parent(r);
        if p < 0.0 {
            return None;
        }
        let pr = *self.parent_row.get(&(p as u64))?;
        let pc = corners_of(self.m.parents, pr, self.nc);
        let b = self.child(r);
        for (f, &(i, s)) in self.faces.iter().zip(&self.axes) {
            if (b >> i) & 1 != s {
                continue;
            }
            let key = sorted(f.iter().map(|&k| pc[k]).collect());
            if let Some(&n) = self.leaf_faces.get(&key).and_then(|rows| rows.first()) {
                return Some(n);
            }
        }
        None
    }

    fn refine_box(&mut self, r: usize, depth: usize) -> Result<bool, String> {
        while let Some(n) = self.coarser_neighbour(r) {
            if depth > DEPTH {
                return Err("boxes could not be balanced".into());
            }
            if !self.refine_box(n, depth + 1)? {
                return Ok(false);
            }
        }
        Ok(self.split_box(r))
    }

    fn split_box(&mut self, r: usize) -> bool {
        let d = self.d;
        let n3 = 3usize.pow(d as u32);
        let pc = self.by_code(&self.corners(r));
        let mut todo: Vec<usize> = (0..n3).filter(|&l| has_middle(l, d)).collect();
        todo.sort_by_key(|&l| middles(l, d));
        let missing = todo
            .iter()
            .filter(|&&l| !self.mid.contains_key(&self.lattice_key(&pc, l)))
            .count();
        if self.m.cells.len() + (1 << d) - 1 > self.m.max_cells
            || self.m.points.len() + missing > self.m.max_points
        {
            return false;
        }
        let mut lat = vec![0u64; n3];
        for (q, &p) in pc.iter().enumerate() {
            let l = (0..d)
                .fold((0, 1), |(l, w), i| (l + w * 2 * ((q >> i) & 1), w * 3))
                .0;
            lat[l] = p;
        }
        for l in todo {
            let key = self.lattice_key(&pc, l);
            if let Some(&id) = self.mid.get(&key) {
                lat[l] = id;
                continue;
            }
            let i = (0..d)
                .find(|&i| (l / 3usize.pow(i as u32)) % 3 == 1)
                .expect("a middle");
            let w = 3usize.pow(i as u32);
            let id = self.new_point(lat[l - w], lat[l + w], r);
            self.mid.insert(key, id);
            lat[l] = id;
        }
        let children: Vec<Vec<u64>> = (0..1usize << d)
            .map(|b| (0..self.nc).map(|pos| lat[self.place(b, pos)]).collect())
            .collect();
        let level = self.level(r);
        let parent = self.keep_parent(r);
        self.replace(r, children, level + 1, parent);
        true
    }
}

/// Whether lattice place `l` has a middle digit (is not a parent corner).
fn has_middle(l: usize, d: usize) -> bool {
    middles(l, d) > 0
}

fn middles(l: usize, d: usize) -> usize {
    (0..d)
        .filter(|&i| (l / 3usize.pow(i as u32)) % 3 == 1)
        .count()
}

fn not_labelled() -> String {
    "these simplices are not labelled for bisection: refine a mesh started from a grid".into()
}

/// Split the cells `marked` (by row) by their shape's rule (ADR 0085
/// point 5), in row order, with what the closure (simplices) or the 2:1
/// balance (boxes) needs; a cell made by this split is not split again by
/// it. Stops when a table is full.
pub fn split(m: &mut Mesh, marked: &[bool], t: &Transfers) -> Result<(), String> {
    let n0 = m.cells.len();
    let nc = m.shape.corners();
    let start: Vec<f64> = m.cells.column(nc).to_vec();
    let mut w = Work::new(m, t);
    for (r, &level) in start.iter().enumerate().take(n0) {
        if !marked.get(r).copied().unwrap_or(false) || w.m.cells.column(nc)[r] != level {
            continue;
        }
        let go = if w.simplicial() {
            w.refine_simplex(r, 0)?
        } else {
            w.refine_box(r, 0)?
        };
        if !go {
            break;
        }
    }
    Ok(())
}

/// Merge the children of a split back into their parent where all of
/// them are `marked` (by row): for simplices, where every cell around a
/// split's new point is a child of a split there (so no hanging point is
/// left), for boxes where no neighbour would be two levels finer. One
/// level per merge. Returns the points no longer corners of these cells.
pub fn merge(m: &mut Mesh, marked: &[bool], t: &Transfers) -> Vec<u64> {
    let nc = m.shape.corners();
    let d = m.shape.dim();
    let simplicial = m.shape.simplicial();
    let n_kids = if simplicial { 2 } else { 1 << d };
    let held = |r: usize| marked.get(r).copied().unwrap_or(false);
    let mut w = Work::new(m, t);
    // Leaves by parent, in child order.
    let mut kids: HashMap<u64, Vec<usize>> = HashMap::new();
    for r in 0..w.m.cells.len() {
        let p = w.parent(r);
        if p >= 0.0 {
            kids.entry(p as u64).or_default().push(r);
        }
    }
    for rows in kids.values_mut() {
        rows.sort_by_key(|&r| w.child(r));
    }
    // Boxes: sides of cells split (and not merged), counted.
    let mut split_faces: HashMap<Vec<u64>, usize> = HashMap::new();
    let faces_of = |c: &[u64], faces: &[Vec<usize>]| -> Vec<Vec<u64>> {
        faces
            .iter()
            .map(|f| sorted(f.iter().map(|&k| c[k]).collect()))
            .collect()
    };
    if !simplicial {
        for r in 0..w.m.parents.len() {
            for key in faces_of(&corners_of(w.m.parents, r, nc), &w.faces) {
                *split_faces.entry(key).or_default() += 1;
            }
        }
    }
    let mut gone_cells = HashSet::new();
    let mut gone_parents = HashSet::new();
    let mut freed = Vec::new();
    let mut done_points = HashSet::new();
    let parent_ids: Vec<u64> = w.m.parents.ids().to_vec();
    let complete = |w: &Work, id: u64| -> Option<Vec<usize>> {
        let rows = kids.get(&id)?;
        (rows.len() == n_kids
            && rows
                .iter()
                .enumerate()
                .all(|(b, &r)| w.child(r) == b && held(r)))
        .then(|| rows.clone())
    };
    for (pr, &id) in parent_ids.iter().enumerate() {
        if gone_parents.contains(&pr) {
            continue;
        }
        let Some(rows) = complete(&w, id) else {
            continue;
        };
        let pc = corners_of(w.m.parents, pr, nc);
        let group: Vec<(usize, Vec<usize>)> = if simplicial {
            let k = tag(d, w.m.parents.column(nc)[pr] as usize);
            let z = w.corners(rows[0])[k];
            if !done_points.insert(z) {
                continue;
            }
            // Every cell around z: a child of a split at z, its sibling
            // there too.
            let around = w.by_point.get(&z).cloned().unwrap_or_default();
            let mut group = Vec::new();
            let mut ok = true;
            for &r in &around {
                let p = w.parent(r);
                let Some(&q) = (p >= 0.0).then(|| w.parent_row.get(&(p as u64))).flatten() else {
                    ok = false;
                    break;
                };
                let kq = tag(d, w.m.parents.column(nc)[q] as usize);
                if w.corners(r)[kq] != z {
                    ok = false;
                    break;
                }
                match complete(&w, p as u64) {
                    Some(rows) if !group.iter().any(|(g, _)| *g == q) => group.push((q, rows)),
                    Some(_) => {}
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            group.sort_by_key(|g| g.0);
            freed.push(z);
            group
        } else {
            // No child's outer side may be split: that neighbour would be
            // two levels finer than the parent.
            let finer = rows.iter().any(|&r| {
                let c = w.corners(r);
                let b = w.child(r);
                w.faces.iter().zip(&w.axes).any(|(f, &(i, s))| {
                    (b >> i) & 1 == s
                        && split_faces.contains_key(&sorted(f.iter().map(|&k| c[k]).collect()))
                })
            });
            if finer {
                continue;
            }
            for &r in &rows {
                for p in w.corners(r) {
                    if !pc.contains(&p) && !freed.contains(&p) {
                        freed.push(p);
                    }
                }
            }
            vec![(pr, rows)]
        };
        for (q, rows) in group {
            let parent = w.m.parents.row(q);
            let values: Vec<f64> =
                w.t.cells
                    .iter()
                    .enumerate()
                    .map(|(c, t)| {
                        let col = w.m.cells.column(nc + 3 + c);
                        match t {
                            Transfer::Mean => {
                                rows.iter().map(|&r| col[r]).sum::<f64>() / rows.len() as f64
                            }
                            Transfer::Sum => rows.iter().map(|&r| col[r]).sum(),
                            Transfer::Given(g) => g[rows[0]],
                            _ => col[rows[0]],
                        }
                    })
                    .collect();
            for &r in &rows {
                w.unindex(r);
            }
            let r0 = rows[0];
            for (k, v) in parent.iter().chain(&values).enumerate() {
                w.m.cells.column_mut(k)[r0] = *v;
            }
            w.index(r0);
            gone_cells.extend(rows[1..].iter().copied());
            gone_parents.insert(q);
            if !simplicial {
                for key in faces_of(
                    &parent[..nc].iter().map(|&p| p as u64).collect::<Vec<_>>(),
                    &w.faces,
                ) {
                    if let Some(n) = split_faces.get_mut(&key) {
                        *n -= 1;
                        if *n == 0 {
                            split_faces.remove(&key);
                        }
                    }
                }
            }
        }
    }
    m.cells.retain(|r| !gone_cells.contains(&r));
    m.parents.retain(|r| !gone_parents.contains(&r));
    freed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_sides_know_their_axes() {
        assert_eq!(
            box_side_axes(CellShape::Quad).unwrap(),
            [(1, 0), (0, 1), (1, 1), (0, 0)]
        );
        assert_eq!(
            box_side_axes(CellShape::Box(2)).unwrap(),
            [(0, 0), (0, 1), (1, 0), (1, 1)]
        );
        assert_eq!(box_side_axes(CellShape::Hexa).unwrap()[0], (2, 0));
    }

    use crate::mesh::GridStart;

    /// A grid's points (coordinates as states) and cells (one state `u`).
    fn grid(shape: CellShape, text: &str) -> (Table, Table, Table) {
        let g = GridStart::parse(text).unwrap();
        let coords = g.points();
        let d = coords.len();
        let mut points = Table::new(d);
        for k in 0..coords[0].len() {
            points.push(&coords.iter().map(|c| c[k]).collect::<Vec<_>>());
        }
        let nc = shape.corners();
        let mut cells = Table::new(nc + 4);
        for c in g.cells(shape).unwrap() {
            let mut row: Vec<f64> = c.iter().map(|&p| p as f64).collect();
            row.extend([0.0, -1.0, -1.0, 1.0]);
            cells.push(&row);
        }
        (points, cells, Table::new(nc + 3))
    }

    fn coords(points: &Table, id: u64) -> Vec<f64> {
        let r = points.ids().iter().position(|&i| i == id).unwrap();
        points.row(r)
    }

    /// Every side of a simplex mesh is a side of two cells, or lies on the
    /// unit box's boundary: no hanging points.
    fn conforming(shape: CellShape, points: &Table, cells: &Table) {
        let (_, faces) = shape.sides().unwrap();
        let mut count: HashMap<Vec<u64>, usize> = HashMap::new();
        for r in 0..cells.len() {
            let c = corners_of(cells, r, shape.corners());
            for f in &faces {
                *count
                    .entry(sorted(f.iter().map(|&k| c[k]).collect()))
                    .or_default() += 1;
            }
        }
        for (key, n) in count {
            assert!(n <= 2, "a side of {n} cells");
            if n == 1 {
                let x: Vec<Vec<f64>> = key.iter().map(|&p| coords(points, p)).collect();
                let on = (0..x[0].len())
                    .any(|i| x.iter().all(|p| p[i] == 0.0) || x.iter().all(|p| p[i] == 1.0));
                assert!(on, "a hanging side {x:?}");
            }
        }
    }

    fn mesh<'a>(
        shape: CellShape,
        points: &'a mut Table,
        cells: &'a mut Table,
        parents: &'a mut Table,
    ) -> Mesh<'a> {
        Mesh {
            shape,
            cells,
            parents,
            points,
            point_at: 0,
            max_cells: usize::MAX,
            max_points: usize::MAX,
        }
    }

    fn near(points: &Table, cells: &Table, shape: CellShape, at: &[f64], r2: f64) -> Vec<bool> {
        (0..cells.len())
            .map(|r| {
                let c = corners_of(cells, r, shape.corners());
                c.iter().any(|&p| {
                    let x = coords(points, p);
                    x.iter()
                        .zip(at)
                        .map(|(a, b)| (a - b) * (a - b))
                        .sum::<f64>()
                        < r2
                })
            })
            .collect()
    }

    /// Cells whose bounding box holds `at`.
    fn around(points: &Table, cells: &Table, shape: CellShape, at: &[f64]) -> Vec<bool> {
        (0..cells.len())
            .map(|r| {
                let x: Vec<Vec<f64>> = corners_of(cells, r, shape.corners())
                    .iter()
                    .map(|&p| coords(points, p))
                    .collect();
                (0..at.len()).all(|i| {
                    let lo = x.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min);
                    let hi = x.iter().map(|p| p[i]).fold(f64::NEG_INFINITY, f64::max);
                    lo <= at[i] && at[i] <= hi
                })
            })
            .collect()
    }

    fn total(cells: &Table, shape: CellShape) -> f64 {
        cells.column(shape.corners() + 3).iter().sum()
    }

    #[test]
    fn bisection_stays_conforming_in_two_three_and_four_dimensions() {
        let cases = [
            (CellShape::Triangle, "x 0 .. 1 by 2, y 0 .. 1 by 2", 10),
            (
                CellShape::Tetra,
                "x 0 .. 1 by 1, y 0 .. 1 by 1, z 0 .. 1 by 1",
                8,
            ),
            (
                CellShape::Simplex(4),
                "a 0 .. 1 by 1, b 0 .. 1 by 1, c 0 .. 1 by 1, d 0 .. 1 by 1",
                5,
            ),
        ];
        for (shape, text, rounds) in cases {
            let (mut points, mut cells, mut parents) = grid(shape, text);
            let n = cells.len() as f64;
            let at: Vec<f64> = (0..shape.dim()).map(|i| 0.3 + 0.01 * i as f64).collect();
            for round in 0..rounds {
                let marked = around(&points, &cells, shape, &at);
                let t = Transfers {
                    cells: vec![Transfer::Share],
                    points: vec![Transfer::Mean; shape.dim()],
                };
                split(
                    &mut mesh(shape, &mut points, &mut cells, &mut parents),
                    &marked,
                    &t,
                )
                .unwrap();
                conforming(shape, &points, &cells);
                // Shares halve exactly: the sum is kept.
                assert_eq!(total(&cells, shape), n, "{} round {round}", shape.name());
            }
            assert!(parents.len() >= rounds, "{}", shape.name());
        }
    }

    #[test]
    fn merging_undoes_a_split() {
        let shape = CellShape::Triangle;
        let (mut points, mut cells, mut parents) = grid(shape, "x 0 .. 1 by 2, y 0 .. 1 by 2");
        let before = (points.clone(), cells.clone());
        let t = Transfers {
            cells: vec![Transfer::Share],
            points: vec![Transfer::Mean; 2],
        };
        let mut levels = 0;
        for _ in 0..4 {
            let marked = near(&points, &cells, shape, &[0.3, 0.3], 0.02);
            split(
                &mut mesh(shape, &mut points, &mut cells, &mut parents),
                &marked,
                &t,
            )
            .unwrap();
            levels += 1;
        }
        let sum = Transfers {
            cells: vec![Transfer::Sum],
            points: vec![Transfer::Mean; 2],
        };
        for _ in 0..64 {
            if parents.is_empty() {
                break;
            }
            let all = vec![true; cells.len()];
            let freed = merge(
                &mut mesh(shape, &mut points, &mut cells, &mut parents),
                &all,
                &sum,
            );
            let used: HashSet<u64> = (0..cells.len())
                .flat_map(|r| corners_of(&cells, r, 3))
                .collect();
            let ids = points.ids().to_vec();
            points.retain(|r| !freed.contains(&ids[r]) || used.contains(&ids[r]));
            conforming(shape, &points, &cells);
            assert_eq!(total(&cells, shape), 8.0);
        }
        assert!(levels > 0 && parents.is_empty());
        // The same cells, ids and corners as at the start.
        assert_eq!(cells.ids(), before.1.ids());
        for c in 0..6 {
            assert_eq!(cells.column(c), before.1.column(c));
        }
        assert_eq!(points.ids(), before.0.ids());
    }

    /// Box levels across each side differ by at most one, and every side
    /// is two cells', a small one's on a large one's, or the boundary's.
    #[test]
    fn boxes_split_in_two_to_the_d_and_stay_balanced() {
        for (shape, text, rounds) in [
            (CellShape::Quad, "x 0 .. 1 by 2, y 0 .. 1 by 2", 6),
            (
                CellShape::Hexa,
                "x 0 .. 1 by 2, y 0 .. 1 by 2, z 0 .. 1 by 2",
                4,
            ),
            (
                CellShape::Box(3),
                "x 0 .. 1 by 1, y 0 .. 1 by 1, z 0 .. 1 by 1",
                4,
            ),
        ] {
            let (mut points, mut cells, mut parents) = grid(shape, text);
            let n = cells.len() as f64;
            let t = Transfers {
                cells: vec![Transfer::Share],
                points: vec![Transfer::Mean; shape.dim()],
            };
            for _ in 0..rounds {
                let marked = near(&points, &cells, shape, &vec![0.01; shape.dim()], 0.001);
                split(
                    &mut mesh(shape, &mut points, &mut cells, &mut parents),
                    &marked,
                    &t,
                )
                .unwrap();
                assert_eq!(total(&cells, shape), n);
                // Volumes from the coordinates add to 1, exactly.
                let nc = shape.corners();
                let codes = box_codes(shape).unwrap();
                let mut vol = 0.0;
                for r in 0..cells.len() {
                    let c = corners_of(&cells, r, nc);
                    let p0 = coords(&points, c[codes.iter().position(|&q| q == 0).unwrap()]);
                    let mut v = 1.0;
                    for i in 0..shape.dim() {
                        let pi =
                            coords(&points, c[codes.iter().position(|&q| q == 1 << i).unwrap()]);
                        v *= pi[i] - p0[i];
                    }
                    vol += v;
                }
                assert_eq!(vol, 1.0, "{}", shape.name());
            }
            let level = |r: usize| cells.column(shape.corners())[r];
            let deepest = (0..cells.len()).map(level).fold(0.0, f64::max);
            assert_eq!(deepest, rounds as f64, "{}", shape.name());
            // 2:1: cells sharing a corner differ by at most one level along
            // a side; check through the sides of the hanging-aware update.
            let (side_shape, faces) = shape.sides().unwrap();
            let mut sides = Table::new(side_shape.corners() + 2);
            let axes = box_side_axes(shape).unwrap();
            crate::mesh::update_sides(
                &cells,
                &mut sides,
                shape.corners(),
                &faces,
                Some((&parents, &axes)),
            );
            let row_of: HashMap<u64, usize> = cells
                .ids()
                .iter()
                .enumerate()
                .map(|(k, &i)| (i, k))
                .collect();
            let k = side_shape.corners();
            let mut boundary = 0.0;
            for s in 0..sides.len() {
                let (a, b) = (sides.column(k)[s], sides.column(k + 1)[s]);
                if b < 0.0 {
                    let x: Vec<Vec<f64>> = (0..k)
                        .map(|c| coords(&points, sides.column(c)[s] as u64))
                        .collect();
                    let on = (0..shape.dim())
                        .any(|i| x.iter().all(|p| p[i] == 0.0) || x.iter().all(|p| p[i] == 1.0));
                    assert!(on, "{} a boundary side inside: {x:?}", shape.name());
                    boundary += 1.0;
                    continue;
                }
                let (la, lb) = (level(row_of[&(a as u64)]), level(row_of[&(b as u64)]));
                assert!(
                    (la - lb).abs() <= 1.0,
                    "{} levels {la} and {lb}",
                    shape.name()
                );
            }
            assert!(boundary > 0.0);
        }
    }
}
