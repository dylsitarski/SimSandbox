//! How the inspector organises a model's parameters (ADR 0053): each
//! value once, where it is set. Values set by formulas are not listed; the
//! value they come from says what uses it. A subsystem used several times
//! on the model level is one table, a column per use. Starting values
//! (initial conditions) come after a block's other parameters. Pinned
//! parameters and those with sliders form the Controls.

use simsandbox_file::ModelFile;
use simsandbox_session::session::ParamRow;
use std::collections::BTreeMap;

/// The parameters, organised.
#[derive(Debug, Default)]
pub struct Organised {
    /// Pinned, or with a slider.
    pub controls: Vec<ParamRow>,
    /// The model's own parameters.
    pub model: Vec<ParamRow>,
    /// Model-level blocks and subsystems used once, in file order.
    pub groups: Vec<Group>,
    /// Subsystems used more than once on the model level.
    pub tables: Vec<UsesTable>,
    /// For each key, the parameters whose formulas read it.
    pub used_by: BTreeMap<String, Vec<String>>,
    /// How many values set by formulas are not listed.
    pub computed: usize,
}

/// One block, or one use of a subsystem.
#[derive(Debug, Clone)]
pub struct Group {
    /// The block's name on the model level.
    pub name: String,
    /// Its type, or the subsystem's name.
    pub type_name: String,
    pub instance: bool,
    /// Its parameters (relative names inside a use: `car.c`), starting
    /// values last.
    pub rows: Vec<(String, ParamRow)>,
}

/// A subsystem's uses side by side.
#[derive(Debug, Clone)]
pub struct UsesTable {
    pub subsystem: String,
    pub uses: Vec<String>,
    /// A parameter by its name inside the subsystem (`mass`, `car.c`), with
    /// each use's row; starting values last.
    pub rows: Vec<(String, Vec<Option<ParamRow>>)>,
}

/// The model-level block a row belongs to: `car1` for `car1/car.c` and
/// for `car1.mass`; `""` for the model's own.
pub fn top(row: &ParamRow) -> &str {
    row.owner.split('/').next().unwrap_or("")
}

/// A row's name within its model-level block: `car.c` for `car1/car.c`,
/// `mass` for `car1.mass`, `T` for `leader.T`.
pub fn relative(row: &ParamRow) -> String {
    match row.owner.split_once('/') {
        Some((_, inner)) => format!("{inner}.{}", row.name),
        None => row.name.clone(),
    }
}

/// The key a formula's name refers to: names are the declared parameters
/// of the scope the row sits in.
fn source_key(row: &ParamRow, name: &str) -> String {
    // A use's own parameter reads the scope around the use; a block's
    // parameter the scope it is in.
    let scope = match row.block {
        None => row.owner.rsplit_once('/').map_or("", |(s, _)| s),
        Some(_) => row.owner.rsplit_once('/').map_or("", |(s, _)| s),
    };
    if scope.is_empty() {
        name.to_string()
    } else {
        format!("{scope}.{name}")
    }
}

/// Organise `rows` (in the session's order) for `file`; with
/// `show_computed`, values set by formulas are listed too.
pub fn organise(rows: &[ParamRow], file: &ModelFile, show_computed: bool) -> Organised {
    let mut o = Organised::default();
    for r in rows {
        let Some(f) = &r.formula else {
            continue;
        };
        if let Ok(ast) = simsandbox_expr::parse(f) {
            for (n, _) in ast.names() {
                o.used_by
                    .entry(source_key(r, &n))
                    .or_default()
                    .push(r.key.clone());
            }
        }
    }
    let shown: Vec<&ParamRow> = rows
        .iter()
        .filter(|r| show_computed || r.formula.is_none())
        .collect();
    o.computed = rows.len() - shown.len();
    o.controls = rows
        .iter()
        .filter(|r| {
            file.controls
                .params
                .get(&r.key)
                .is_some_and(|c| c.slider || c.pin)
        })
        .cloned()
        .collect();
    // A control is listed there only.
    let control = |r: &ParamRow| o.controls.iter().any(|c| c.key == r.key);
    o.model = shown
        .iter()
        .filter(|r| r.owner.is_empty() && !control(r))
        .map(|r| (*r).clone())
        .collect();
    // Uses by subsystem, on the model level.
    let mut uses: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for b in &file.blocks {
        if file.subsystems.contains_key(&b.type_name) {
            uses.entry(b.type_name.as_str()).or_default().push(&b.name);
        }
    }
    let starting_last = |v: &mut Vec<(String, ParamRow)>| {
        v.sort_by_key(|(_, r)| r.initial_condition);
    };
    for b in &file.blocks {
        let instance = file.subsystems.contains_key(&b.type_name);
        let mine: Vec<(String, ParamRow)> = shown
            .iter()
            .filter(|r| !r.owner.is_empty() && top(r) == b.name && !control(r))
            .map(|r| (relative(r), (*r).clone()))
            .collect();
        let shared = instance && uses.get(b.type_name.as_str()).is_some_and(|u| u.len() > 1);
        if shared {
            continue;
        }
        let mut rows = mine;
        starting_last(&mut rows);
        if !rows.is_empty() || !instance {
            o.groups.push(Group {
                name: b.name.clone(),
                type_name: b.type_name.clone(),
                instance,
                rows,
            });
        }
    }
    for (subsystem, names) in uses.into_iter().filter(|(_, u)| u.len() > 1) {
        let mut keys: Vec<(String, bool)> = Vec::new();
        for r in shown.iter().filter(|r| names.contains(&top(r))) {
            let k = relative(r);
            if !keys.iter().any(|(x, _)| *x == k) {
                keys.push((k, r.initial_condition));
            }
        }
        keys.sort_by_key(|(_, ic)| *ic);
        let rows = keys
            .into_iter()
            .map(|(k, _)| {
                let cells = names
                    .iter()
                    .map(|u| {
                        shown
                            .iter()
                            .find(|r| top(r) == *u && relative(r) == k)
                            .map(|r| (*r).clone())
                    })
                    .collect();
                (k, cells)
            })
            .collect();
        o.tables.push(UsesTable {
            subsystem: subsystem.to_string(),
            uses: names.iter().map(|s| s.to_string()).collect(),
            rows,
        });
    }
    o
}

/// Rows whose key or description contains `query` (any case).
pub fn search(rows: &[ParamRow], query: &str) -> Vec<ParamRow> {
    let q = query.trim().to_lowercase();
    rows.iter()
        .filter(|r| r.key.to_lowercase().contains(&q) || r.description.to_lowercase().contains(&q))
        .cloned()
        .collect()
}

/// The rows of what is selected on the diagram: a model-level block or
/// use (`scope` empty), or a block inside a subsystem (`scope` is the
/// subsystem; one column per use of it). Returns the uses and rows by
/// name, as a table.
pub fn selected(
    rows: &[ParamRow],
    file: &ModelFile,
    scope: Option<&str>,
    name: &str,
) -> Option<UsesTable> {
    let (uses, inner): (Vec<String>, Option<&str>) = match scope {
        None => (vec![name.to_string()], None),
        Some(s) => (
            file.blocks
                .iter()
                .filter(|b| b.type_name == s)
                .map(|b| b.name.clone())
                .collect(),
            Some(name),
        ),
    };
    let belongs = |r: &ParamRow, u: &str| match inner {
        None => !r.owner.is_empty() && top(r) == u,
        Some(i) => r.owner == format!("{u}/{i}"),
    };
    let mut keys: Vec<(String, bool)> = Vec::new();
    for u in &uses {
        for r in rows.iter().filter(|r| belongs(r, u)) {
            let k = match inner {
                None => relative(r),
                Some(_) => r.name.clone(),
            };
            if !keys.iter().any(|(x, _)| *x == k) {
                keys.push((k, r.initial_condition));
            }
        }
    }
    if keys.is_empty() {
        return None;
    }
    keys.sort_by_key(|(_, ic)| *ic);
    let table_rows = keys
        .into_iter()
        .map(|(k, _)| {
            let cells = uses
                .iter()
                .map(|u| {
                    rows.iter()
                        .find(|r| {
                            belongs(r, u)
                                && match inner {
                                    None => relative(r) == k,
                                    Some(_) => r.name == k,
                                }
                        })
                        .cloned()
                })
                .collect();
            (k, cells)
        })
        .collect();
    Some(UsesTable {
        subsystem: scope.unwrap_or(name).to_string(),
        uses,
        rows: table_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use simsandbox_session::examples::{registry, EXAMPLES};
    use simsandbox_session::Session;

    fn platoon() -> Session {
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Platoon"))
            .unwrap();
        Session::load(e.name, e.text, &registry()).unwrap()
    }

    #[test]
    fn each_value_is_listed_once_where_it_is_set() {
        let s = platoon();
        let rows = s.params();
        assert_eq!(rows.len(), 32);
        let o = organise(&rows, &s.file, false);
        // Fifteen values are set by formulas: not listed.
        assert_eq!(o.computed, 15);
        // ki and kp have sliders: listed among the controls only.
        assert!(o.model.is_empty());
        // kp is read by each use's kp.
        assert_eq!(o.used_by["kp"], ["car1.kp", "car2.kp", "car3.kp"]);
        assert_eq!(o.used_by["car1.kp"], ["car1/control.kp"]);
        // The three followers are one table; the leader a group.
        assert_eq!(o.tables.len(), 1);
        let t = &o.tables[0];
        assert_eq!((t.subsystem.as_str(), t.uses.len()), ("follower", 3));
        let names: Vec<&str> = t.rows.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["mass", "car.c", "car.v0", "control.I0"]);
        let mass: Vec<f64> = t.rows[0]
            .1
            .iter()
            .map(|c| c.as_ref().unwrap().value)
            .collect();
        assert_eq!(mass, [1000.0, 1500.0, 2500.0]);
        assert_eq!(o.groups.len(), 1);
        let leader: Vec<&str> = o.groups[0].rows.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(leader, ["T", "v0"]);
        // Sliders are the controls.
        let controls: Vec<&str> = o.controls.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(controls, ["ki", "kp", "leader.target"]);
        // 3 controls + 4 × 3 + 2 = 17 values to edit, as 3 + 4 + 2 lines.
        let shown =
            o.controls.len() + o.model.len() + o.tables[0].rows.len() * 3 + o.groups[0].rows.len();
        assert_eq!(shown, 17);
        // Everything, computed values too.
        assert_eq!(organise(&rows, &s.file, true).computed, 0);
    }

    #[test]
    fn search_and_selection_find_values_anywhere() {
        let s = platoon();
        let rows = s.params();
        let keys = |v: Vec<ParamRow>| v.into_iter().map(|r| r.key).collect::<Vec<_>>();
        assert_eq!(
            keys(search(&rows, "MASS")),
            ["car1.mass", "car2.mass", "car3.mass"]
        );
        // A use selected on the model level: its own and its blocks'.
        let t = selected(&rows, &s.file, None, "car2").unwrap();
        assert_eq!(t.uses, ["car2"]);
        assert!(t.rows.iter().any(|(k, _)| k == "car.c"));
        // A block inside the subsystem: every use side by side.
        let t = selected(&rows, &s.file, Some("follower"), "car").unwrap();
        assert_eq!(t.uses, ["car1", "car2", "car3"]);
        let names: Vec<&str> = t.rows.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["m", "c", "v0"]);
        assert!(selected(&rows, &s.file, None, "nothing").is_none());
    }
}
