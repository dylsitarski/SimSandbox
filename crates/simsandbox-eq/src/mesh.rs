//! Meshes as tables (ADR 0085): cells of a listed shape as ordered
//! corner lists over a table of points, sides between cells kept with
//! stable ids, geometry written as expressions of the corners' values,
//! and starting grids.

use simsandbox_core::table::Table;
use simsandbox_expr::{Ast, Func, Op};

/// A cell's shape: its corners in a fixed order (Gmsh's for the named
/// shapes, binary for `box d`), its sides, and its dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellShape {
    Segment,
    Triangle,
    Quad,
    Tetra,
    Hexa,
    Prism,
    Pyramid,
    /// A d-simplex, corners `0..=d` (`simplex 2` is a triangle).
    Simplex(usize),
    /// A d-box, axis-aligned, corner `k` at bit `i` of `k` along axis
    /// `i` (`box 2`: (0,0), (1,0), (0,1), (1,1)).
    Box(usize),
}

/// The most corners a shape may have (`box 6`).
pub const MAX_CORNERS: usize = 64;

impl CellShape {
    /// From its name as written: `triangle`, `simplex 4`, `box 3`, ….
    pub fn parse(text: &str) -> Option<Self> {
        let w: Vec<&str> = text.split_whitespace().collect();
        let dim = |n: &str| n.parse::<usize>().ok().filter(|d| (1..=6).contains(d));
        Some(match w[..] {
            ["segment"] => CellShape::Segment,
            ["triangle"] => CellShape::Triangle,
            ["quad"] => CellShape::Quad,
            ["tetra"] => CellShape::Tetra,
            ["hexa"] => CellShape::Hexa,
            ["prism"] => CellShape::Prism,
            ["pyramid"] => CellShape::Pyramid,
            ["simplex", d] => CellShape::Simplex(dim(d)?),
            ["box", d] => CellShape::Box(dim(d)?),
            _ => return None,
        })
    }

    pub fn name(self) -> String {
        match self {
            CellShape::Segment => "segment".into(),
            CellShape::Triangle => "triangle".into(),
            CellShape::Quad => "quad".into(),
            CellShape::Tetra => "tetra".into(),
            CellShape::Hexa => "hexa".into(),
            CellShape::Prism => "prism".into(),
            CellShape::Pyramid => "pyramid".into(),
            CellShape::Simplex(d) => format!("simplex {d}"),
            CellShape::Box(d) => format!("box {d}"),
        }
    }

    /// Its own dimension: 1 for a segment, 3 for a tetrahedron.
    pub fn dim(self) -> usize {
        match self {
            CellShape::Segment => 1,
            CellShape::Triangle | CellShape::Quad => 2,
            CellShape::Tetra | CellShape::Hexa | CellShape::Prism | CellShape::Pyramid => 3,
            CellShape::Simplex(d) | CellShape::Box(d) => d,
        }
    }

    pub fn corners(self) -> usize {
        match self {
            CellShape::Segment => 2,
            CellShape::Triangle => 3,
            CellShape::Quad | CellShape::Tetra => 4,
            CellShape::Pyramid => 5,
            CellShape::Prism => 6,
            CellShape::Hexa => 8,
            CellShape::Simplex(d) => d + 1,
            CellShape::Box(d) => 1 << d,
        }
    }

    /// Its sides as lists of its corners, in a fixed order, if all have
    /// one shape (prisms and pyramids have triangles and quads: not yet).
    pub fn sides(self) -> Option<(CellShape, Vec<Vec<usize>>)> {
        let point = || CellShape::Simplex(0);
        Some(match self {
            CellShape::Segment | CellShape::Simplex(1) | CellShape::Box(1) => {
                (point(), vec![vec![0], vec![1]])
            }
            CellShape::Triangle => (CellShape::Segment, vec![vec![0, 1], vec![1, 2], vec![2, 0]]),
            CellShape::Quad => (
                CellShape::Segment,
                vec![vec![0, 1], vec![1, 2], vec![2, 3], vec![3, 0]],
            ),
            CellShape::Tetra => (
                CellShape::Triangle,
                vec![vec![0, 2, 1], vec![0, 1, 3], vec![0, 3, 2], vec![1, 2, 3]],
            ),
            CellShape::Hexa => (
                CellShape::Quad,
                vec![
                    vec![0, 3, 2, 1],
                    vec![0, 1, 5, 4],
                    vec![1, 2, 6, 5],
                    vec![2, 3, 7, 6],
                    vec![3, 0, 4, 7],
                    vec![4, 5, 6, 7],
                ],
            ),
            CellShape::Prism | CellShape::Pyramid => return None,
            // A d-simplex's sides: each corner left out in turn.
            CellShape::Simplex(d) => (
                simplex(d - 1),
                (0..=d)
                    .rev()
                    .map(|out| (0..=d).filter(|&k| k != out).collect())
                    .collect(),
            ),
            // A d-box's sides: each axis held at 0, then at 1.
            CellShape::Box(d) => (
                boxed(d - 1),
                (0..d)
                    .flat_map(|i| {
                        [0, 1].map(|v| {
                            (0..1usize << d)
                                .filter(|k| (k >> i) & 1 == v)
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect(),
            ),
        })
    }

    /// Whether it is a simplex (refined by bisection), else a box.
    pub fn simplicial(self) -> bool {
        matches!(
            self,
            CellShape::Segment | CellShape::Triangle | CellShape::Tetra | CellShape::Simplex(_)
        )
    }
}

/// The named shape for a d-simplex where there is one.
fn simplex(d: usize) -> CellShape {
    match d {
        1 => CellShape::Segment,
        2 => CellShape::Triangle,
        3 => CellShape::Tetra,
        d => CellShape::Simplex(d),
    }
}

fn boxed(d: usize) -> CellShape {
    match d {
        1 => CellShape::Segment,
        d => CellShape::Box(d),
    }
}

/// A starting grid: per axis its coordinate's name, ends and boxes.
#[derive(Debug, Clone, PartialEq)]
pub struct GridStart {
    pub axes: Vec<(String, f64, f64, usize)>,
}

impl GridStart {
    /// `x 0 .. 1 by 8, y 0 .. 2 by 4`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let usage = "write a starting grid as 'grid x 0 .. 1 by 8, y 0 .. 1 by 8' (each coordinate, its ends and its boxes)";
        let mut axes = Vec::new();
        for part in text.split(',') {
            let w: Vec<&str> = part.split_whitespace().collect();
            let [name, lo, "..", hi, "by", n] = w[..] else {
                return Err(usage.into());
            };
            let (Ok(lo), Ok(hi), Ok(n)) =
                (lo.parse::<f64>(), hi.parse::<f64>(), n.parse::<usize>())
            else {
                return Err(usage.into());
            };
            if !(lo.is_finite() && hi.is_finite() && lo < hi) || n == 0 {
                return Err(format!(
                    "{name}: the ends must increase and the boxes be 1 or more"
                ));
            }
            axes.push((name.to_string(), lo, hi, n));
        }
        Ok(Self { axes })
    }

    /// Its points, the first axis fastest: one coordinate list per axis.
    pub fn points(&self) -> Vec<Vec<f64>> {
        let counts: Vec<usize> = self.axes.iter().map(|a| a.3 + 1).collect();
        let total: usize = counts.iter().product();
        let mut coords = vec![Vec::with_capacity(total); self.axes.len()];
        for p in 0..total {
            let mut rest = p;
            for (i, &(_, lo, hi, n)) in self.axes.iter().enumerate() {
                let k = rest % counts[i];
                rest /= counts[i];
                // The ends exactly; between them evenly.
                coords[i].push(if k == n {
                    hi
                } else {
                    lo + (hi - lo) * k as f64 / n as f64
                });
            }
        }
        coords
    }

    /// Its cells of `shape`, as places among its points: boxes (each in
    /// the shape's corner order), or each box cut into d! simplices by
    /// the Kuhn construction (corner i+1 is corner i plus one step along
    /// the next axis of a permutation, permutations in lexicographic
    /// order), labelled so for newest-vertex bisection.
    pub fn cells(&self, shape: CellShape) -> Result<Vec<Vec<usize>>, String> {
        let d = self.axes.len();
        if shape.dim() != d {
            return Err(format!(
                "a grid over {d} coordinates makes cells of {d} dimensions, not {}",
                shape.name()
            ));
        }
        let counts: Vec<usize> = self.axes.iter().map(|a| a.3 + 1).collect();
        let place = |at: &[usize]| -> usize {
            let mut p = 0;
            for i in (0..d).rev() {
                p = p * counts[i] + at[i];
            }
            p
        };
        let boxes: usize = self.axes.iter().map(|a| a.3).product();
        let mut out = Vec::new();
        for b in 0..boxes {
            let mut rest = b;
            let origin: Vec<usize> = self
                .axes
                .iter()
                .map(|a| {
                    let k = rest % a.3;
                    rest /= a.3;
                    k
                })
                .collect();
            let at = |bits: &[usize]| -> usize {
                let p: Vec<usize> = origin.iter().zip(bits).map(|(o, b)| o + b).collect();
                place(&p)
            };
            if shape.simplicial() {
                for perm in permutations(d) {
                    let mut bits = vec![0; d];
                    let mut cell = vec![at(&bits)];
                    for &axis in &perm {
                        bits[axis] = 1;
                        cell.push(at(&bits));
                    }
                    out.push(cell);
                }
            } else {
                let order: Vec<Vec<usize>> = match shape {
                    CellShape::Quad => vec![vec![0, 0], vec![1, 0], vec![1, 1], vec![0, 1]],
                    CellShape::Hexa => [
                        [0, 0, 0],
                        [1, 0, 0],
                        [1, 1, 0],
                        [0, 1, 0],
                        [0, 0, 1],
                        [1, 0, 1],
                        [1, 1, 1],
                        [0, 1, 1],
                    ]
                    .iter()
                    .map(|c| c.to_vec())
                    .collect(),
                    CellShape::Box(_) => (0..1usize << d)
                        .map(|k| (0..d).map(|i| (k >> i) & 1).collect())
                        .collect(),
                    other => {
                        return Err(format!(
                            "a grid starts cells of simplices or boxes, not {}",
                            other.name()
                        ))
                    }
                };
                out.push(order.iter().map(|bits| at(bits)).collect());
            }
        }
        Ok(out)
    }
}

/// The permutations of `0..d` in lexicographic order.
fn permutations(d: usize) -> Vec<Vec<usize>> {
    if d == 0 {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for first in 0..d {
        for rest in permutations(d - 1) {
            let mut p = vec![first];
            p.extend(rest.into_iter().map(|r| if r >= first { r + 1 } else { r }));
            out.push(p);
        }
    }
    out
}

/// Keep `sides` up to date with `cells` (ADR 0085 point 3): each side is
/// a list of `corners` point ids, then its `inner` and `outer` cell ids
/// (`outer` -1 on the boundary). Sides are found in cell order, each
/// cell's sides in its shape's order; a side still there keeps its row
/// and id (its cells updated), one gone is removed, a new one added.
/// `cell_corners` is the cells' corner count, `faces` their sides.
pub fn update_sides(cells: &Table, sides: &mut Table, cell_corners: usize, faces: &[Vec<usize>]) {
    let k = faces.first().map_or(0, Vec::len);
    let mut found: Vec<(Vec<u64>, Vec<f64>, f64, f64)> = Vec::new();
    let mut at: std::collections::HashMap<Vec<u64>, usize> = Default::default();
    for (r, &id) in cells.ids().iter().enumerate() {
        for face in faces {
            let corners: Vec<f64> = face.iter().map(|&c| cells.column(c)[r]).collect();
            let mut key: Vec<u64> = corners.iter().map(|&c| c as u64).collect();
            key.sort_unstable();
            match at.get(&key) {
                Some(&s) if found[s].3 < 0.0 => found[s].3 = id as f64,
                Some(_) => {}
                None => {
                    at.insert(key.clone(), found.len());
                    found.push((key, corners, id as f64, -1.0));
                }
            }
        }
    }
    // The rows there, by their corners.
    let old: std::collections::HashMap<Vec<u64>, usize> = (0..sides.len())
        .map(|r| {
            let mut key: Vec<u64> = (0..k).map(|c| sides.column(c)[r] as u64).collect();
            key.sort_unstable();
            (key, r)
        })
        .collect();
    let keep: Vec<bool> = (0..sides.len())
        .map(|r| {
            let mut key: Vec<u64> = (0..k).map(|c| sides.column(c)[r] as u64).collect();
            key.sort_unstable();
            at.contains_key(&key)
        })
        .collect();
    // Cells of sides kept, as found now.
    for (key, _, inner, outer) in &found {
        if let Some(&r) = old.get(key) {
            sides.column_mut(k)[r] = *inner;
            sides.column_mut(k + 1)[r] = *outer;
        }
    }
    sides.retain(|r| keep[r]);
    let columns = sides.columns();
    for (key, corners, inner, outer) in found {
        if old.contains_key(&key) {
            continue;
        }
        let mut row = corners;
        row.push(inner);
        row.push(outer);
        row.resize(columns, 0.0);
        sides.push(&row);
    }
    let _ = cell_corners;
}

/// `corner(t, cells, k)` as written.
pub fn corner(t: &Ast, cells: &str, k: usize) -> Ast {
    Ast::Apply(
        "corner".into(),
        vec![t.clone(), Ast::Name(cells.into(), 1), Ast::Num(k as f64)],
        1,
    )
}

fn bin(op: Op, a: Ast, b: Ast) -> Ast {
    Ast::Bin(op, Box::new(a), Box::new(b))
}

fn call(f: Func, a: Ast) -> Ast {
    Ast::Call(f, vec![a])
}

fn sum(terms: Vec<Ast>) -> Ast {
    terms
        .into_iter()
        .reduce(|a, b| bin(Op::Add, a, b))
        .unwrap_or(Ast::Num(0.0))
}

/// `centre(t, cells)`: the mean of the corners' values.
pub fn centre(t: &Ast, cells: &str, shape: CellShape) -> Ast {
    let n = shape.corners();
    bin(
        Op::Div,
        sum((0..n).map(|k| corner(t, cells, k)).collect()),
        Ast::Num(n as f64),
    )
}

/// The measure of a cell from its corners' coordinates `xs` (one name per
/// coordinate): length, area or volume, in a fixed order of operations.
/// Hexahedra, prisms and pyramids add the tetrahedra about one diagonal
/// (exact when their faces are flat).
pub fn measure(cells: &str, shape: CellShape, xs: &[Ast]) -> Result<Ast, String> {
    let c = |x: &Ast, k: usize| corner(x, cells, k);
    // Corner k minus corner 0, per coordinate.
    let d = |a: usize, b: usize| -> Vec<Ast> {
        xs.iter().map(|x| bin(Op::Sub, c(x, b), c(x, a))).collect()
    };
    let dot = |u: &[Ast], v: &[Ast]| -> Ast {
        sum(u
            .iter()
            .zip(v)
            .map(|(a, b)| bin(Op::Mul, a.clone(), b.clone()))
            .collect())
    };
    let cross = |u: &[Ast], v: &[Ast]| -> Vec<Ast> {
        let m = |a: &Ast, b: &Ast| bin(Op::Mul, a.clone(), b.clone());
        vec![
            bin(Op::Sub, m(&u[1], &v[2]), m(&u[2], &v[1])),
            bin(Op::Sub, m(&u[2], &v[0]), m(&u[0], &v[2])),
            bin(Op::Sub, m(&u[0], &v[1]), m(&u[1], &v[0])),
        ]
    };
    let length = |v: &[Ast]| -> Ast {
        if v.len() == 1 {
            call(Func::Abs, v[0].clone())
        } else {
            call(Func::Sqrt, dot(v, v))
        }
    };
    // A triangle's area from corners a, b, c.
    let tri = |a: usize, b: usize, cc: usize| -> Result<Ast, String> {
        let (u, v) = (d(a, b), d(a, cc));
        Ok(bin(
            Op::Mul,
            Ast::Num(0.5),
            match xs.len() {
                2 => call(
                    Func::Abs,
                    bin(
                        Op::Sub,
                        bin(Op::Mul, u[0].clone(), v[1].clone()),
                        bin(Op::Mul, u[1].clone(), v[0].clone()),
                    ),
                ),
                3 => length(&cross(&u, &v)),
                n => {
                    return Err(format!(
                        "a triangle's area needs 2 or 3 coordinates, not {n}"
                    ))
                }
            },
        ))
    };
    // A tetrahedron's volume from corners a, b, c, e.
    let tet = |a: usize, b: usize, cc: usize, e: usize| -> Result<Ast, String> {
        if xs.len() != 3 {
            return Err(format!("a volume needs 3 coordinates, not {}", xs.len()));
        }
        let det = dot(&d(a, b), &cross(&d(a, cc), &d(a, e)));
        Ok(bin(Op::Div, call(Func::Abs, det), Ast::Num(6.0)))
    };
    let tets = |list: &[[usize; 4]]| -> Result<Ast, String> {
        Ok(sum(list
            .iter()
            .map(|t| tet(t[0], t[1], t[2], t[3]))
            .collect::<Result<_, _>>()?))
    };
    match shape {
        // A point (the side of a segment) counts 1.
        CellShape::Simplex(0) | CellShape::Box(0) => Ok(Ast::Num(1.0)),
        CellShape::Segment | CellShape::Simplex(1) | CellShape::Box(1) => Ok(length(&d(0, 1))),
        CellShape::Triangle | CellShape::Simplex(2) => tri(0, 1, 2),
        CellShape::Quad if xs.len() == 2 => {
            // The shoelace formula over the corners in order.
            let term = |i: usize| {
                let j = (i + 1) % 4;
                bin(
                    Op::Sub,
                    bin(Op::Mul, c(&xs[0], i), c(&xs[1], j)),
                    bin(Op::Mul, c(&xs[0], j), c(&xs[1], i)),
                )
            };
            Ok(bin(
                Op::Mul,
                Ast::Num(0.5),
                call(Func::Abs, sum((0..4).map(term).collect())),
            ))
        }
        CellShape::Quad => Ok(bin(Op::Add, tri(0, 1, 2)?, tri(0, 2, 3)?)),
        CellShape::Tetra | CellShape::Simplex(3) => tet(0, 1, 2, 3),
        CellShape::Hexa => tets(&[
            [0, 1, 2, 6],
            [0, 2, 3, 6],
            [0, 3, 7, 6],
            [0, 7, 4, 6],
            [0, 4, 5, 6],
            [0, 5, 1, 6],
        ]),
        CellShape::Prism => tets(&[[0, 1, 2, 3], [1, 2, 3, 4], [2, 3, 4, 5]]),
        CellShape::Pyramid => tets(&[[0, 1, 2, 4], [0, 2, 3, 4]]),
        // An axis-aligned box: the product of its extents.
        CellShape::Box(n) if xs.len() == n => Ok(xs
            .iter()
            .enumerate()
            .map(|(i, x)| call(Func::Abs, bin(Op::Sub, c(x, 1 << i), c(x, 0))))
            .reduce(|a, b| bin(Op::Mul, a, b))
            .expect("one axis or more")),
        // In more coordinates (a box's side): the product of its edges'
        // lengths.
        CellShape::Box(n) if xs.len() > n => Ok((0..n)
            .map(|i| length(&d(0, 1 << i)))
            .reduce(|a, b| bin(Op::Mul, a, b))
            .expect("one axis or more")),
        CellShape::Box(n) => Err(format!(
            "a box {n}'s measure takes at least {n} coordinates"
        )),
        CellShape::Simplex(n) => Err(format!(
            "the measure of a simplex of {n} dimensions comes later (up to 3 for now)"
        )),
    }
}

/// Component `k` of a side's unit normal, pointing from its inner cell to
/// its outer one: from the side's corners' coordinates `xs`, oriented by
/// the inner cell's centre.
pub fn normal(
    sides: &str,
    side: CellShape,
    cells: &str,
    shape: CellShape,
    k: usize,
    xs: &[Ast],
) -> Result<Ast, String> {
    let c = |x: &Ast, j: usize| corner(x, sides, j);
    let d = |a: usize, b: usize| -> Vec<Ast> {
        xs.iter().map(|x| bin(Op::Sub, c(x, b), c(x, a))).collect()
    };
    let m = |a: &Ast, b: &Ast| bin(Op::Mul, a.clone(), b.clone());
    let cross = |u: &[Ast], v: &[Ast]| -> Vec<Ast> {
        vec![
            bin(Op::Sub, m(&u[1], &v[2]), m(&u[2], &v[1])),
            bin(Op::Sub, m(&u[2], &v[0]), m(&u[0], &v[2])),
            bin(Op::Sub, m(&u[0], &v[1]), m(&u[1], &v[0])),
        ]
    };
    let raw: Vec<Ast> = match (side.corners(), xs.len()) {
        (1, 1) => vec![Ast::Num(1.0)],
        (2, 2) => {
            let u = d(0, 1);
            vec![u[1].clone(), Ast::Neg(Box::new(u[0].clone()))]
        }
        (3, 3) => cross(&d(0, 1), &d(0, 2)),
        // A box's side, corners in binary order: across its two edges.
        (4, 3) if matches!(side, CellShape::Box(2)) => cross(&d(0, 1), &d(0, 2)),
        // A quad, corners in turn: across its diagonals.
        (4, 3) => cross(&d(0, 2), &d(1, 3)),
        (n, dims) => {
            return Err(format!(
                "a normal is for sides of cells as many dimensions as coordinates ({n} corners, {dims} coordinates)"
            ))
        }
    };
    if k >= raw.len() {
        return Err(format!("a normal has components 0 to {}", raw.len() - 1));
    }
    // From the inner cell's centre towards the side's.
    let inner = |x: &Ast| {
        Ast::Apply(
            "inner".into(),
            vec![centre(x, cells, shape), Ast::Name(sides.into(), 1)],
            1,
        )
    };
    let towards: Vec<Ast> = xs
        .iter()
        .map(|x| bin(Op::Sub, centre(x, sides, side), inner(x)))
        .collect();
    let orient = call(
        Func::Sign,
        sum(raw.iter().zip(&towards).map(|(a, b)| m(a, b)).collect()),
    );
    let len = call(Func::Sqrt, sum(raw.iter().map(|a| m(a, a)).collect()));
    Ok(bin(Op::Div, m(&orient, &raw[k]), len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_have_their_corners_and_sides() {
        for (s, n, sides) in [
            (CellShape::Segment, 2, 2),
            (CellShape::Triangle, 3, 3),
            (CellShape::Quad, 4, 4),
            (CellShape::Tetra, 4, 4),
            (CellShape::Hexa, 8, 6),
            (CellShape::Simplex(4), 5, 5),
            (CellShape::Box(4), 16, 8),
        ] {
            assert_eq!(s.corners(), n, "{s:?}");
            let (side, list) = s.sides().unwrap();
            assert_eq!(list.len(), sides, "{s:?}");
            assert!(
                list.iter().all(|f| f.len() == side.corners().max(1)),
                "{s:?}"
            );
        }
        assert_eq!(CellShape::parse("simplex 3"), Some(CellShape::Simplex(3)));
        assert_eq!(CellShape::parse("hexagon"), None);
    }

    #[test]
    fn kuhn_grids_fill_the_box_with_simplices() {
        let g = GridStart::parse("x 0 .. 1 by 2, y 0 .. 1 by 1, z 0 .. 1 by 1").unwrap();
        let pts = g.points();
        assert_eq!(pts[0].len(), 3 * 2 * 2);
        let cells = g.cells(CellShape::Tetra).unwrap();
        // d! = 6 tetrahedra per box, two boxes.
        assert_eq!(cells.len(), 12);
        // Each Kuhn simplex runs from a box's low corner to its high one.
        for c in &cells {
            let lo = c[0];
            let hi = c[3];
            assert_eq!(pts[1][lo], 0.0);
            assert_eq!(pts[1][hi], 1.0);
        }
        assert_eq!(g.cells(CellShape::Hexa).unwrap().len(), 2);
        assert!(g.cells(CellShape::Triangle).is_err());
    }
}
