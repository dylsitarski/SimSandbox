//! A block's interface: its parameters, ports (with what each output
//! depends on at the same instant) and states, as its [`BlockSpec`].

use super::declare::Declared;
use super::layout::Registers;
use super::shaped::Shaped;
use super::statements::{EdgeKind, Lhs};
use super::units::Units;
use super::{element_name, Kind, RowsSource, TableRows};
use simsandbox_core::block::{BlockSpec, Domain, ParamSpec, PortSpec, Shape, TimeSemantics};
use simsandbox_core::table::TableSpec;
use simsandbox_core::units::Unit;

/// What the block shows and how its values map onto the kernel's: what
/// [`interface`] establishes.
pub(super) struct Interface {
    pub(super) spec: BlockSpec,
    /// For each state: its number of values, and the parameter holding
    /// its initial value (scalar states).
    pub(super) initial: Vec<(usize, Option<usize>)>,
    /// Each input: its values' number, and its table if over rows.
    pub(super) inputs_in: Vec<(usize, Option<usize>)>,
    /// Each output over a table's rows, in port order.
    pub(super) rows_out: Vec<(usize, RowsSource)>,
    /// Each state over rows: its table and column.
    pub(super) row_states: Vec<(usize, usize)>,
}

/// The parameters with their defaults; those that set where a run
/// starts are marked as initial conditions.
fn param_specs(d: &Declared, s: &Shaped, units: &Units) -> Vec<ParamSpec> {
    let mut specs: Vec<ParamSpec> = s
        .params
        .iter()
        .map(|p| ParamSpec {
            name: p.clone(),
            unit: units.of(p),
            default: d.params.iter().find(|q| &q.0 == p).map_or(1.0, |q| q.1),
            suggested_min: 0.0,
            suggested_max: 10.0,
            description: "Parameter of the equations".into(),
            initial_condition: false,
        })
        .collect();
    for (state, i) in d.states.iter().zip(&s.initial) {
        if let Some(i) = *i {
            let p = &mut specs[i];
            p.initial_condition = true;
            p.suggested_min = -10.0;
            p.description = format!("Initial value of {state}");
        }
    }
    // Parameters in array states' initial values mark them too:
    // changing them changes where the run starts.
    for init in &s.inits {
        for (p, _) in init.ast.names() {
            if let Some(spec) = specs.iter_mut().find(|q| q.name == p) {
                if !spec.initial_condition && !d.states.iter().any(|x| x == &p) {
                    let used_elsewhere = d
                        .equations
                        .iter()
                        .any(|l| l.ast.names().iter().any(|(m, _)| *m == p));
                    if !used_elsewhere {
                        spec.initial_condition = true;
                        spec.description = format!("Sets the initial value of {}", init.name);
                    }
                }
            }
        }
    }
    specs
}

/// What reading names depends on.
struct Reads<'a> {
    d: &'a Declared,
    s: &'a Shaped<'a>,
    params: &'a [ParamSpec],
}

impl Reads<'_> {
    /// The parameters `n` names: an array parameter is all its elements.
    fn param_names(&self, n: &str) -> Vec<String> {
        match self.d.array_params.iter().find(|a| a.name == n) {
            Some(a) => (0..a.cells).map(|k| element_name(n, k)).collect(),
            None if self.params.iter().any(|p| p.name == n) => vec![n.to_string()],
            None => Vec::new(),
        }
    }

    /// A table that follows an input holds that input's rows at each
    /// instant (ADR 0079 stage 1b): whatever reads its rows (a quantity
    /// over it, `id`, `rows`) depends on that input at the same instant.
    fn follower(&self, n: &str) -> Option<String> {
        let (d, s) = (self.d, self.s);
        let a = d
            .grid
            .iter()
            .position(|a| a.name == n)
            .or_else(|| s.arrays.row_table(n))
            .filter(|&a| s.followed[a])?;
        d.inputs
            .iter()
            .find(|i| s.arrays.row_table(i) == Some(a))
            .cloned()
    }

    /// Which inputs and parameters each defined quantity uses, directly
    /// or through quantities above it (ADR 0040, ADR 0075).
    fn feedthrough(&self) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
        let d = self.d;
        let mut uses: Vec<Vec<String>> = Vec::new();
        let mut reads: Vec<Vec<String>> = Vec::new();
        for line in &d.equations {
            let Lhs::Defined(name) = &line.lhs else {
                continue;
            };
            let mut u: Vec<String> = Vec::new();
            let mut r: Vec<String> = Vec::new();
            u.extend(self.follower(name));
            for (n, _) in line.ast.names() {
                if let Some(i) = self.follower(&n) {
                    if !u.contains(&i) {
                        u.push(i);
                    }
                }
                let (via, params): (Vec<String>, Vec<String>) =
                    match d.defined[..uses.len()].iter().position(|x| *x == n) {
                        Some(i) => (uses[i].clone(), reads[i].clone()),
                        None if d.inputs.contains(&n) => (vec![n], Vec::new()),
                        None => (Vec::new(), self.param_names(&n)),
                    };
                for i in via {
                    if !u.contains(&i) {
                        u.push(i);
                    }
                }
                for p in params {
                    if !r.contains(&p) {
                        r.push(p);
                    }
                }
            }
            // In input and parameter order, so the declaration reads
            // naturally.
            u.sort_by_key(|i| d.inputs.iter().position(|x| x == i));
            r.sort_by_key(|p| self.params.iter().position(|q| &q.name == p));
            uses.push(u);
            reads.push(r);
        }
        (uses, reads)
    }
}

/// The ports: inputs, states, defined quantities, the axes, and each
/// table of cells with a starting mesh (with these, by axis and number
/// of corners). A port has its quantity's axes (ADR 0080, ADR 0081); a
/// quantity over a table's rows carries them, with their ids and
/// version (ADR 0079 stage 1b).
fn ports(d: &Declared, units: &Units, reads: &Reads) -> (Vec<PortSpec>, Vec<(usize, usize)>) {
    let (grid, arrays) = (&d.grid, &reads.s.arrays);
    let row_table = |n: &str| arrays.row_table(n);
    let shape = |n: &str| {
        if row_table(n).is_some() {
            return Shape::Rows { dim: 1 };
        }
        let dims: Vec<usize> = arrays.of(n).iter().map(|&a| grid[a].cells).collect();
        Shape::of_axes(&dims)
    };
    let named = |p: PortSpec, n: &str| -> PortSpec {
        let own = arrays.of(n);
        let names: Vec<&str> = own.iter().map(|&a| grid[a].name.as_str()).collect();
        p.axes_named(&names)
    };
    let (uses, params_read) = reads.feedthrough();
    // A state's output is the state itself (ADR 0063), in state order.
    let mut state_at = 0;
    let mut ports: Vec<PortSpec> = d
        .inputs
        .iter()
        .map(|n| named(PortSpec::input(n, units.of(n), shape(n)), n))
        .chain(d.states.iter().map(|n| {
            let p = PortSpec::output(n, units.of(n), shape(n));
            if row_table(n).is_some() {
                let f: Vec<String> = reads.follower(n).into_iter().collect();
                let f: Vec<&str> = f.iter().map(String::as_str).collect();
                return named(p.depends_on(&f), n);
            }
            let p = p.from_state(state_at);
            state_at += shape(n).len();
            named(p, n)
        }))
        .chain(
            d.defined
                .iter()
                .zip(uses.iter().zip(&params_read))
                .map(|(n, (u, r))| {
                    let u: Vec<&str> = u.iter().map(String::as_str).collect();
                    let r: Vec<&str> = r.iter().map(String::as_str).collect();
                    named(PortSpec::output(n, units.of(n), shape(n)), n)
                        .depends_on(&u)
                        .depends_on_params(&r)
                }),
        )
        .collect();
    for (i, a) in grid.iter().enumerate().filter(|(_, a)| a.table.is_none()) {
        // Each axis is the coordinate of every output over it.
        let bounds: Vec<String> = [&a.lo, &a.hi]
            .iter()
            .flat_map(|e| e.names())
            .flat_map(|(n, _)| reads.param_names(&n))
            .collect();
        let bounds: Vec<&str> = bounds.iter().map(String::as_str).collect();
        let mut x = named(
            PortSpec::output(&a.name, units.of(&a.name), shape(&a.name)),
            &a.name,
        )
        .depends_on_params(&bounds);
        for n in d.states.iter().chain(&d.defined) {
            if let Some(k) = arrays.of(n).iter().position(|&a| a == i) {
                x = x.coordinate_of(n, k);
            }
        }
        ports.push(x);
    }
    // Each table of cells with a starting mesh is an output of its own:
    // each cell's corners, as ids of its points, for views (ADR 0086).
    let mut mesh_tables = Vec::new();
    for (i, a) in grid.iter().enumerate() {
        let (Some(shape), None, Some(_)) = (a.shape, a.sides_of, a.ends_of) else {
            continue;
        };
        let (Some(faces), Some((spec, _))) = (shape.drawn_faces(), d.starts.specs.get(&a.name))
        else {
            continue;
        };
        let coordinates = spec.names();
        if !(2..=3).contains(&coordinates.len()) || d.states.iter().any(|x| x == &a.name) {
            continue;
        }
        ports.push(
            PortSpec::output(
                &a.name,
                Unit::DIMENSIONLESS,
                Shape::Rows {
                    dim: shape.corners(),
                },
            )
            .axes_named(&[&a.name])
            .depends_on_params(&[])
            .mesh_of(simsandbox_core::block::MeshOf {
                shape: shape.name(),
                dim: coordinates.len(),
                faces,
                coordinates,
            }),
        );
        mesh_tables.push((i, shape.corners()));
    }
    (ports, mesh_tables)
}

/// The kernel's tables: each table's columns before its states (edges:
/// `from`, `to`; cells: their corners; sides: corners, `inner`, `outer`),
/// then its states; after them the parents of the cells split and not
/// merged back (ADR 0085 step 2).
fn table_specs(tables: &[TableRows]) -> Vec<TableSpec> {
    let own = tables.iter().map(|t| {
        let mut cols: Vec<String> = match (t.shape, t.sides_of) {
            (Some(crate::mesh::CellShape::Segment), None) => {
                vec!["from".into(), "to".into()]
            }
            (Some(s), _) => (0..s.corners()).map(|k| format!("c{k}")).collect(),
            (None, _) => Vec::new(),
        };
        if t.sides_of.is_some() {
            cols.extend(["inner".to_string(), "outer".to_string()]);
        }
        if t.parents.is_some() {
            cols.extend(["level", "parent", "child"].map(String::from));
        }
        cols.extend(t.columns.iter().cloned());
        let cols: Vec<&str> = cols.iter().map(String::as_str).collect();
        TableSpec::new(&t.name, &cols)
    });
    let parents = tables.iter().filter(|t| t.parents.is_some()).map(|t| {
        let mut cols: Vec<String> = (0..t.corners()).map(|k| format!("c{k}")).collect();
        cols.extend(["level", "parent", "child"].map(String::from));
        let cols: Vec<&str> = cols.iter().map(String::as_str).collect();
        TableSpec::new(&format!("{}~parents", t.name), &cols)
    });
    own.chain(parents).collect()
}

/// Whether any equation, initial value, edge value or rule reads `t`.
fn reads_time(d: &Declared, s: &Shaped) -> bool {
    d.equations
        .iter()
        .map(|l| &l.ast)
        .chain(s.inits.iter().map(|i| &i.ast))
        .chain(d.edges.iter().filter_map(|e| match &e.kind {
            EdgeKind::Value(a) | EdgeKind::Gradient(a) => Some(a),
            EdgeKind::Periodic => None,
        }))
        .chain(d.rules.iter().flat_map(|r| {
            std::iter::once(&r.cond).chain(r.add.iter().flat_map(|(_, v)| v.iter().map(|g| &g.1)))
        }))
        .any(|a| a.names().iter().any(|(n, _)| n == "t"))
}

/// The block's interface, as a Block named `name`.
pub(super) fn interface(
    name: &str,
    d: &Declared,
    s: &Shaped,
    units: &Units,
    r: &Registers,
    tables: &[TableRows],
) -> Interface {
    let arrays = &s.arrays;
    let row_table = |n: &str| arrays.row_table(n);
    let params = param_specs(d, s, units);
    let reads = Reads {
        d,
        s,
        params: &params,
    };
    let (ports, mesh_tables) = ports(d, units, &reads);
    let states = d
        .states
        .iter()
        .filter(|n| row_table(n).is_none())
        .flat_map(|n| {
            if !arrays.of(n).is_empty() {
                (0..arrays.len_of(n))
                    .map(|k| element_name(n, k))
                    .collect::<Vec<_>>()
            } else {
                vec![n.clone()]
            }
        })
        .collect();
    let initial = d
        .states
        .iter()
        .zip(&s.initial)
        .map(|(n, i)| (arrays.len_of(n), *i))
        .collect();
    let table_of = |a: usize| tables.iter().position(|t| t.axis == a);
    let inputs_in = d
        .inputs
        .iter()
        .map(|n| (arrays.len_of(n), row_table(n).and_then(table_of)))
        .collect();
    // Each state over rows: its table and its column there.
    let column_of = |n: &String| {
        let t = table_of(row_table(n)?)?;
        Some((t, tables[t].columns.iter().position(|c| c == n)?))
    };
    // In port order: states, then defined quantities, then meshes.
    let rows_out = d
        .states
        .iter()
        .filter_map(|n| column_of(n).map(|(t, c)| (t, RowsSource::Column(c))))
        .chain(d.defined.iter().enumerate().filter_map(|(k, n)| {
            let t = table_of(row_table(n)?)?;
            Some((t, RowsSource::Registers(r.spans[r.defined[k]])))
        }))
        .chain(
            mesh_tables
                .iter()
                .filter_map(|&(i, n)| Some((table_of(i)?, RowsSource::Corners(n)))),
        )
        .collect();
    let row_states = d.states.iter().filter_map(column_of).collect();
    Interface {
        spec: BlockSpec {
            name: name.into(),
            domain: Domain::Equations,
            ports,
            params,
            states,
            tables: table_specs(tables),
            time: match d.kind {
                Kind::Continuous => TimeSemantics::Continuous,
                Kind::Map => TimeSemantics::DiscreteEvent,
                Kind::Formulas => TimeSemantics::Stateless,
            },
            reads_time: Some(reads_time(d, s)),
        },
        initial,
        inputs_in,
        rows_out,
        row_states,
    }
}
