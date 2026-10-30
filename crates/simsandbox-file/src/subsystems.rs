//! Subsystems and declared parameters (ADR 0043).
//!
//! A model file may declare parameters (`params`, e.g. a field size shared
//! by two blocks) and define subsystems (`subsystems`): groups of blocks
//! with their own parameters and input and output ports, used like block
//! types by name. Any block parameter may be a number or a formula of the
//! declared parameters in its scope (`"K": "2*capacity"`).
//!
//! Loading flattens the file: each instance becomes its inner blocks,
//! named `instance/inner` (nested: `a/b/c`), wired through the instance's
//! ports, so the kernel, fusion, loops and views see ordinary blocks. The
//! declared parameters and formulas become a [`ParamPlan`] that interfaces
//! re-evaluate when a value changes.
//!
//! Units (ADR 0045): a declared parameter's `unit` must be a unit. A
//! formula that uses a declared parameter with a unit is checked like a
//! typed equation: it must balance and give the unit of the parameter it
//! sets (a name without a unit is dimensionless), and it is computed in SI,
//! so `"length": "size"` with `size` in km gives 1000 for a length in m.
//! A formula whose names have no units is not checked, as before.

use crate::{invalid, BlockEntry, ConnectionEntry, FileError, ModelFile};
use serde::{Deserialize, Serialize};
use simsandbox_core::units::Unit;
use simsandbox_expr::units::{dim_text, dimension, from_si, in_si, Dim, NO_DIM};
use simsandbox_expr::Expr;
use std::collections::BTreeMap;

/// A parameter's value in a file: a number, or a formula of the declared
/// parameters in scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Number(f64),
    Formula(String),
}

impl ParamValue {
    pub fn number(&self) -> Option<f64> {
        match self {
            ParamValue::Number(v) => Some(*v),
            ParamValue::Formula(_) => None,
        }
    }
}

impl From<f64> for ParamValue {
    fn from(v: f64) -> Self {
        ParamValue::Number(v)
    }
}

/// A declared parameter: the model's own, or one a subsystem offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamDecl {
    /// The model's value, or a subsystem's default; a number.
    pub value: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Suggested range, as for block parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
}

/// A subsystem's input: what it feeds inside.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputPort {
    pub name: String,
    /// Inner inputs, `"block.port"`.
    #[serde(default)]
    pub to: Vec<String>,
    /// Where its node sits on the subsystem's canvas (`at` before
    /// ADR 0052).
    #[serde(default, alias = "at", skip_serializing_if = "Option::is_none")]
    pub canvas: Option<[f64; 2]>,
}

/// A subsystem's output: which inner output it shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputPort {
    pub name: String,
    /// An inner output, `"block.port"`.
    pub from: String,
    /// Where its node sits on the subsystem's canvas (`at` before
    /// ADR 0052).
    #[serde(default, alias = "at", skip_serializing_if = "Option::is_none")]
    pub canvas: Option<[f64; 2]>,
}

/// A subsystem: blocks and wires with parameters and ports of their own.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subsystem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamDecl>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<InputPort>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<OutputPort>,
    pub blocks: Vec<BlockEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionEntry>,
}

/// A compiled formula.
#[derive(Debug, Clone, PartialEq)]
pub struct Formula {
    pub text: String,
    /// Variables are indices into [`ParamPlan::declared`]; in SI once the
    /// formula has units.
    expr: Expr,
    /// Its dimension, when a name in it has a unit; `None` for a formula
    /// that is not checked.
    dim: Option<Dim>,
    /// As written, without conversion: for a block that gives no units.
    raw: Expr,
}

impl Formula {
    /// Make the formula give `what`'s unit, `target` (dimensionless if
    /// `None`): an error at `path` if it measures something else.
    fn fit(&mut self, target: Option<&Unit>, what: &str, path: &str) -> Result<(), FileError> {
        let Some(dim) = self.dim else {
            return Ok(());
        };
        let target = target.copied().unwrap_or(Unit::DIMENSIONLESS);
        if dim != target.dim.0 {
            let shown = |d: Dim| {
                if d == NO_DIM {
                    "dimensionless".to_string()
                } else {
                    format!("in {}", dim_text(d))
                }
            };
            return Err(invalid(
                path,
                format!(
                    "{what} is {} but '{}' is {}",
                    shown(target.dim.0),
                    self.text,
                    shown(dim)
                ),
            ));
        }
        self.expr = from_si(
            std::mem::replace(&mut self.expr, Expr::Num(0.0)),
            target.scale,
        );
        // Fitted once: a formula is not converted twice.
        self.dim = None;
        Ok(())
    }

    /// Not checked: computed from the values as written, as before units
    /// were checked (for a block that gives no units).
    fn unchecked(&mut self) {
        if self.dim.take().is_some() {
            self.expr = self.raw.clone();
        }
    }
}

/// A declared parameter at run time.
#[derive(Debug, Clone, PartialEq)]
pub struct Declared {
    /// `"size"` for the model's own, `"pasture.size"` for an instance's
    /// (`"farm/pasture.size"` nested).
    pub key: String,
    pub value: f64,
    /// Set for an instance parameter given as a formula of the enclosing
    /// scope: computed, not edited.
    pub formula: Option<Formula>,
    /// The subsystem's default, for an instance's parameter.
    pub default: Option<f64>,
    pub unit: String,
    pub description: String,
    pub range: Option<[f64; 2]>,
    /// Whether a person sets it: the model's parameters and those of
    /// instances in the model itself (not inside other subsystems), unless
    /// given as formulas.
    pub editable: bool,
}

/// A block parameter set by a formula.
#[derive(Debug, Clone, PartialEq)]
pub struct Derived {
    /// The flat block name, e.g. `pasture/grass`.
    pub block: String,
    pub param: String,
    pub formula: Formula,
    /// Where it is written, e.g. `subsystems.field.blocks[0].params.K`.
    pub path: String,
}

/// A parameter of a block inside an instance, with the value the
/// subsystem gives it, so saving writes only what differs.
#[derive(Debug, Clone, PartialEq)]
pub struct Inner {
    pub block: String,
    pub param: String,
    pub base: f64,
}

/// Declared parameters and formulas, in evaluation order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParamPlan {
    pub declared: Vec<Declared>,
    pub derived: Vec<Derived>,
    /// Numeric parameters of blocks inside instances.
    pub inner: Vec<Inner>,
}

impl ParamPlan {
    pub fn is_empty(&self) -> bool {
        self.declared.is_empty() && self.derived.is_empty()
    }

    pub fn declared(&self, key: &str) -> Option<&Declared> {
        self.declared.iter().find(|d| d.key == key)
    }

    /// The formula setting a block parameter, if one does.
    pub fn formula_of(&self, block: &str, param: &str) -> Option<&Formula> {
        self.derived
            .iter()
            .find(|d| d.block == block && d.param == param)
            .map(|d| &d.formula)
    }

    /// Recompute every formula. Returns the block parameters with their
    /// values.
    pub fn evaluate(&mut self) -> Vec<(String, String, f64)> {
        let mut values: Vec<f64> = self.declared.iter().map(|d| d.value).collect();
        for (i, d) in self.declared.iter_mut().enumerate() {
            if let Some(f) = &d.formula {
                values[i] = f.expr.eval(&values);
                d.value = values[i];
            }
        }
        self.derived
            .iter()
            .map(|d| {
                (
                    d.block.clone(),
                    d.param.clone(),
                    d.formula.expr.eval(&values),
                )
            })
            .collect()
    }

    /// Fit each block parameter's formula to the parameter's unit in the
    /// built `model` (ADR 0045), then set the values formulas give.
    pub(crate) fn fit_blocks(
        &mut self,
        model: &mut simsandbox_core::graph::Model,
        ids: &BTreeMap<String, simsandbox_core::graph::BlockId>,
    ) -> Result<(), FileError> {
        for d in &mut self.derived {
            let Some(&id) = ids.get(&d.block) else {
                continue;
            };
            let spec = model.block(id).map_err(|e| invalid(&d.path, e))?.spec();
            // A block that gives no units (typed equations without any) is
            // not checked, like its equations.
            let mut units = spec.params.iter().map(|p| p.unit);
            let mut ports = spec.ports.iter().map(|p| p.unit);
            if units.all(|u| u == Unit::DIMENSIONLESS) && ports.all(|u| u == Unit::DIMENSIONLESS) {
                d.formula.unchecked();
                continue;
            }
            let unit = spec
                .params
                .iter()
                .find(|p| p.name == d.param)
                .map(|p| p.unit);
            let what = format!("{}.{}", d.block.trim_start_matches("check/"), d.param);
            d.formula.fit(unit.as_ref(), &what, &d.path)?;
        }
        let paths: BTreeMap<(String, String), String> = self
            .derived
            .iter()
            .map(|d| ((d.block.clone(), d.param.clone()), d.path.clone()))
            .collect();
        for (block, param, value) in self.evaluate() {
            if let Some(&id) = ids.get(&block) {
                model
                    .set_param(id, &param, value)
                    .map_err(|e| invalid(&paths[&(block.clone(), param.clone())], e))?;
            }
        }
        Ok(())
    }

    /// Set a declared parameter and recompute. Returns the block parameters
    /// formulas set, to apply to the model.
    pub fn set(&mut self, key: &str, value: f64) -> Result<Vec<(String, String, f64)>, String> {
        if !value.is_finite() {
            return Err(format!("{key}: {value} is not a finite number"));
        }
        let d = self
            .declared
            .iter_mut()
            .find(|d| d.key == key)
            .ok_or_else(|| format!("no parameter '{key}'"))?;
        if let Some(f) = &d.formula {
            return Err(format!("{key} is computed from '{}'", f.text));
        }
        d.value = value;
        Ok(self.evaluate())
    }
}

/// A flattened model file: kernel blocks only.
pub(crate) struct Flat {
    pub file: ModelFile,
    pub plan: ParamPlan,
    /// For each flat block and connection, where it is written.
    pub block_origins: Vec<String>,
    pub connection_origins: Vec<String>,
    /// Each subsystem flattened alone with its defaults, to build and
    /// check even when nothing uses it.
    pub checks: Vec<Check>,
}

/// One subsystem on its own: its blocks, wires and ports, flat.
pub(crate) struct Check {
    pub file: ModelFile,
    pub block_origins: Vec<String>,
    pub connection_origins: Vec<String>,
    /// (path, flat port, is input) for each port of its interface.
    pub ports: Vec<(String, String, bool)>,
    /// Its parameters with their defaults, to check formula units.
    pub plan: ParamPlan,
}

/// What a name at one level of the model stands for.
enum Item {
    Block(String),
    Instance { inputs: Inputs, outputs: Outputs },
}

struct Ctx<'a> {
    defs: &'a BTreeMap<String, Subsystem>,
    blocks: Vec<BlockEntry>,
    block_origins: Vec<String>,
    connections: Vec<ConnectionEntry>,
    connection_origins: Vec<String>,
    plan: ParamPlan,
    /// Subsystems being expanded, to refuse one that contains itself.
    stack: Vec<String>,
}

/// Valid names: letters, digits and '_' (no '.', which separates ports,
/// or '/', which separates levels).
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

impl ModelFile {
    /// Whether the file uses subsystems or declared parameters.
    pub fn is_flat(&self) -> bool {
        self.subsystems.is_empty()
            && self.params.is_empty()
            && self.blocks.iter().all(|b| {
                b.params
                    .values()
                    .all(|v| matches!(v, ParamValue::Number(_)))
            })
    }

    pub(crate) fn flatten(&self) -> Result<Flat, FileError> {
        let mut ctx = Ctx {
            defs: &self.subsystems,
            blocks: Vec::new(),
            block_origins: Vec::new(),
            connections: Vec::new(),
            connection_origins: Vec::new(),
            plan: ParamPlan::default(),
            stack: Vec::new(),
        };
        for name in self.subsystems.keys() {
            if !valid_name(name) {
                return Err(invalid(
                    format!("subsystems.{name}"),
                    "subsystem names are letters, digits and '_'",
                ));
            }
        }
        let mut scope = BTreeMap::new();
        for (name, d) in &self.params {
            let path = format!("params.{name}");
            if !valid_name(name) || simsandbox_expr::is_reserved(name) {
                return Err(invalid(
                    path,
                    "names are letters, digits and '_', not a function or 'pi'",
                ));
            }
            if !d.value.is_finite() {
                return Err(invalid(format!("{path}.value"), "must be a finite number"));
            }
            declared_unit(&d.unit, &format!("{path}.unit"))?;
            scope.insert(name.clone(), ctx.plan.declared.len());
            ctx.plan.declared.push(Declared {
                key: name.clone(),
                value: d.value,
                formula: None,
                default: None,
                unit: d.unit.clone(),
                description: d.description.clone(),
                range: d.range,
                editable: true,
            });
        }
        ctx.level(&self.blocks, &self.connections, "", "", &scope)?;
        // Every subsystem is also checked alone, with its defaults, so one
        // nobody uses yet is checked too.
        let mut checks = Vec::new();
        for (name, def) in &self.subsystems {
            let mut check = Ctx {
                defs: &self.subsystems,
                blocks: Vec::new(),
                block_origins: Vec::new(),
                connections: Vec::new(),
                connection_origins: Vec::new(),
                plan: ParamPlan::default(),
                stack: vec![name.clone()],
            };
            let scope = check.declare(
                def,
                &BlockEntry::instance(name, name),
                &BTreeMap::new(),
                "",
                "",
                false,
            )?;
            let path = format!("subsystems.{name}.");
            let items = check.level(&def.blocks, &def.connections, "check/", &path, &scope)?;
            let (inputs, outputs) = interface(def, &items, &path)?;
            let mut ports = Vec::new();
            for (k, p) in def.inputs.iter().enumerate() {
                for t in &inputs[&p.name] {
                    ports.push((format!("{path}inputs[{k}].to"), t.clone(), true));
                }
            }
            for (k, p) in def.outputs.iter().enumerate() {
                ports.push((
                    format!("{path}outputs[{k}].from"),
                    outputs[&p.name].clone(),
                    false,
                ));
            }
            checks.push(Check {
                file: ModelFile {
                    blocks: check.blocks,
                    connections: check.connections,
                    ..ModelFile::new()
                },
                block_origins: check.block_origins,
                connection_origins: check.connection_origins,
                ports,
                plan: check.plan,
            });
        }
        let file = ModelFile {
            blocks: ctx.blocks,
            connections: ctx.connections,
            params: BTreeMap::new(),
            subsystems: BTreeMap::new(),
            ..self.clone()
        };
        Ok(Flat {
            file,
            plan: ctx.plan,
            block_origins: ctx.block_origins,
            connection_origins: ctx.connection_origins,
            checks,
        })
    }
}

impl BlockEntry {
    /// An entry using a subsystem.
    pub fn instance(name: &str, subsystem: &str) -> Self {
        BlockEntry {
            name: name.into(),
            type_name: subsystem.into(),
            ..Default::default()
        }
    }
}

/// A declared parameter's unit: `None` if it gives none, an error at
/// `path` if it is not a unit.
fn declared_unit(text: &str, path: &str) -> Result<Option<Unit>, FileError> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    Unit::parse(text)
        .map(Some)
        .map_err(|e| invalid(path, format!("{e}; leave it out for none")))
}

/// Compile a formula of the declared parameters in `scope`; checked and
/// in SI when one of its names has a unit (ADR 0045).
fn compile(
    text: &str,
    scope: &BTreeMap<String, usize>,
    path: &str,
    declared: &[Declared],
) -> Result<Formula, FileError> {
    let ast = simsandbox_expr::parse(text).map_err(|e| invalid(path, format!("'{text}': {e}")))?;
    // Validated where each was declared.
    let units: Vec<Option<Unit>> = declared
        .iter()
        .map(|d| declared_unit(&d.unit, "").ok().flatten())
        .collect();
    let unit_of = |n: &str| scope.get(n).and_then(|&i| units[i]);
    let checked = ast.names().iter().any(|(n, _)| unit_of(n).is_some());
    let dim = if checked {
        let of = |n: &str| unit_of(n).map_or(NO_DIM, |u| u.dim.0);
        Some(dimension(&ast, &of).map_err(|(column, message)| {
            invalid(path, format!("'{text}', column {column}: {message}"))
        })?)
    } else {
        None
    };
    let expr = ast.resolve(&|n: &str| scope.get(n).copied()).map_err(|e| {
        let known: Vec<&str> = scope.keys().map(String::as_str).collect();
        let names = if known.is_empty() {
            "none are declared here".to_string()
        } else {
            format!("declared here: {}", known.join(", "))
        };
        invalid(path, format!("'{text}': {e} ({names})"))
    })?;
    let raw = expr.clone();
    let expr = if checked {
        let scales: Vec<f64> = units.iter().map(|u| u.map_or(1.0, |u| u.scale)).collect();
        in_si(expr, &scales)
    } else {
        expr
    };
    Ok(Formula {
        text: text.into(),
        expr,
        dim,
        raw,
    })
}

/// A subsystem's inputs as the flat inputs each feeds.
type Inputs = BTreeMap<String, Vec<String>>;
/// A subsystem's outputs as the flat output each shows.
type Outputs = BTreeMap<String, String>;

/// Resolve a subsystem's ports among its inner items.
fn interface(
    def: &Subsystem,
    items: &BTreeMap<String, Item>,
    path: &str,
) -> Result<(Inputs, Outputs), FileError> {
    let mut inputs = BTreeMap::new();
    for (k, p) in def.inputs.iter().enumerate() {
        let at = format!("{path}inputs[{k}]");
        if !valid_name(&p.name) || inputs.contains_key(&p.name) {
            return Err(invalid(
                format!("{at}.name"),
                format!(
                    "'{}': port names are unique letters, digits and '_'",
                    p.name
                ),
            ));
        }
        let mut targets = Vec::new();
        for (j, t) in p.to.iter().enumerate() {
            targets
                .extend(resolve(items, t, true).map_err(|m| invalid(format!("{at}.to[{j}]"), m))?);
        }
        inputs.insert(p.name.clone(), targets);
    }
    let mut outputs = BTreeMap::new();
    for (k, p) in def.outputs.iter().enumerate() {
        let at = format!("{path}outputs[{k}]");
        if !valid_name(&p.name) || outputs.contains_key(&p.name) {
            return Err(invalid(
                format!("{at}.name"),
                format!(
                    "'{}': port names are unique letters, digits and '_'",
                    p.name
                ),
            ));
        }
        let mut source =
            resolve(items, &p.from, false).map_err(|m| invalid(format!("{at}.from"), m))?;
        outputs.insert(p.name.clone(), source.remove(0));
    }
    Ok((inputs, outputs))
}

/// Flat ports for `"name.port"` at a level: one for an output, any number
/// for an instance input.
fn resolve(items: &BTreeMap<String, Item>, key: &str, input: bool) -> Result<Vec<String>, String> {
    let (name, port) = key
        .split_once('.')
        .ok_or_else(|| format!("'{key}' must have the form 'block.port'"))?;
    match items.get(name) {
        None => Err(format!("no block named '{name}'")),
        Some(Item::Block(flat)) => Ok(vec![format!("{flat}.{port}")]),
        Some(Item::Instance { inputs, outputs }) => {
            let found = if input {
                inputs.get(port).cloned()
            } else {
                outputs.get(port).map(|p| vec![p.clone()])
            };
            found.ok_or_else(|| {
                let (kind, names): (&str, Vec<&String>) = if input {
                    ("input", inputs.keys().collect())
                } else {
                    ("output", outputs.keys().collect())
                };
                let names: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
                format!(
                    "'{name}' has no {kind} '{port}' ({kind}s: {})",
                    names.join(", ")
                )
            })
        }
    }
}

impl Ctx<'_> {
    /// Declare an instance's parameters: given in its entry (a number, or a
    /// formula of `outer`) or the subsystem's defaults. Returns the scope
    /// inside.
    fn declare(
        &mut self,
        def: &Subsystem,
        entry: &BlockEntry,
        outer: &BTreeMap<String, usize>,
        prefix: &str,
        epath: &str,
        top: bool,
    ) -> Result<BTreeMap<String, usize>, FileError> {
        for (k, v) in &entry.params {
            if !k.contains('.') && !def.params.contains_key(k) {
                let names: Vec<&str> = def.params.keys().map(String::as_str).collect();
                return Err(invalid(
                    format!("{epath}.params.{k}"),
                    format!(
                        "'{}' declares no parameter '{k}' (it has: {}); set an inner parameter as 'block.{k}'",
                        entry.type_name,
                        if names.is_empty() { "none".into() } else { names.join(", ") }
                    ),
                ));
            }
            if k.contains('.') && v.number().is_none() {
                return Err(invalid(
                    format!("{epath}.params.{k}"),
                    "inner parameters are set to numbers",
                ));
            }
        }
        let mut scope = BTreeMap::new();
        for (pname, decl) in &def.params {
            let dpath = format!("subsystems.{}.params.{pname}", entry.type_name);
            if !valid_name(pname) || simsandbox_expr::is_reserved(pname) {
                return Err(invalid(
                    dpath,
                    "names are letters, digits and '_', not a function or 'pi'",
                ));
            }
            if !decl.value.is_finite() {
                return Err(invalid(format!("{dpath}.value"), "must be a finite number"));
            }
            let unit = declared_unit(&decl.unit, &format!("{dpath}.unit"))?;
            let (value, formula) = match entry.params.get(pname) {
                None => (decl.value, None),
                Some(ParamValue::Number(v)) => (*v, None),
                Some(ParamValue::Formula(text)) => {
                    let fpath = format!("{epath}.params.{pname}");
                    let mut f = compile(text, outer, &fpath, &self.plan.declared)?;
                    f.fit(unit.as_ref(), &format!("{}.{pname}", entry.name), &fpath)?;
                    let values: Vec<f64> = self.plan.declared.iter().map(|d| d.value).collect();
                    (f.expr.eval(&values), Some(f))
                }
            };
            scope.insert(pname.clone(), self.plan.declared.len());
            self.plan.declared.push(Declared {
                key: format!("{prefix}{}.{pname}", entry.name),
                value,
                editable: top && formula.is_none(),
                formula,
                default: Some(decl.value),
                unit: decl.unit.clone(),
                description: decl.description.clone(),
                range: decl.range,
            });
        }
        Ok(scope)
    }

    /// Flatten one level (the model, or a subsystem's inside under
    /// `prefix`). `path` prefixes error paths, e.g. `subsystems.field.`.
    fn level(
        &mut self,
        blocks: &[BlockEntry],
        connections: &[ConnectionEntry],
        prefix: &str,
        path: &str,
        scope: &BTreeMap<String, usize>,
    ) -> Result<BTreeMap<String, Item>, FileError> {
        let mut items = BTreeMap::new();
        for (bi, entry) in blocks.iter().enumerate() {
            let epath = format!("{path}blocks[{bi}]");
            if !valid_name(&entry.name) {
                return Err(invalid(
                    format!("{epath}.name"),
                    format!(
                        "'{}': names are letters, digits and '_' (no '.', '/' or spaces)",
                        entry.name
                    ),
                ));
            }
            if items.contains_key(&entry.name) {
                return Err(invalid(
                    format!("{epath}.name"),
                    format!("duplicate block name '{}'", entry.name),
                ));
            }
            let flat = format!("{prefix}{}", entry.name);
            let item = match self.defs.get(&entry.type_name) {
                Some(def) => self.instance(def, entry, scope, prefix, &epath, &flat)?,
                None => {
                    if !entry.starts.is_empty() {
                        return Err(invalid(
                            format!("{epath}.starts"),
                            "only subsystems have 'starts'; a block has 'start'",
                        ));
                    }
                    let mut b = entry.clone();
                    b.name = flat.clone();
                    let values: Vec<f64> = self.plan.declared.iter().map(|d| d.value).collect();
                    for (k, v) in &mut b.params {
                        if let ParamValue::Formula(text) = v {
                            let path = format!("{epath}.params.{k}");
                            let f = compile(text, scope, &path, &self.plan.declared)?;
                            // Units are fitted once the block is built
                            // (`ParamPlan::fit_blocks`).
                            *v = ParamValue::Number(f.expr.eval(&values));
                            self.plan.derived.push(Derived {
                                block: flat.clone(),
                                param: k.clone(),
                                formula: f,
                                path,
                            });
                        } else if !prefix.is_empty() {
                            self.plan.inner.push(Inner {
                                block: flat.clone(),
                                param: k.clone(),
                                base: v.number().unwrap_or_default(),
                            });
                        }
                    }
                    self.blocks.push(b);
                    self.block_origins.push(epath.clone());
                    Item::Block(flat)
                }
            };
            items.insert(entry.name.clone(), item);
        }
        for (ci, c) in connections.iter().enumerate() {
            let cpath = format!("{path}connections[{ci}]");
            // A probe feeding the model (ADR 0063), on the model's level:
            // the loader puts a hold between them.
            let from = if c.from.starts_with("probe:") {
                if !path.is_empty() {
                    return Err(invalid(
                        format!("{cpath}.from"),
                        "probes feed blocks on the model's level, not inside a subsystem",
                    ));
                }
                vec![c.from.clone()]
            } else {
                resolve(&items, &c.from, false).map_err(|m| invalid(format!("{cpath}.from"), m))?
            };
            let to = resolve(&items, &c.to, true).map_err(|m| invalid(format!("{cpath}.to"), m))?;
            for t in to {
                self.connections.push(ConnectionEntry {
                    from: from[0].clone(),
                    to: t,
                });
                self.connection_origins.push(cpath.clone());
            }
        }
        Ok(items)
    }

    fn instance(
        &mut self,
        def: &Subsystem,
        entry: &BlockEntry,
        outer: &BTreeMap<String, usize>,
        prefix: &str,
        epath: &str,
        flat: &str,
    ) -> Result<Item, FileError> {
        let ty = &entry.type_name;
        if self.stack.contains(ty) {
            return Err(invalid(
                format!("{epath}.type"),
                format!(
                    "'{ty}' contains itself ({} -> {ty})",
                    self.stack.join(" -> ")
                ),
            ));
        }
        let extra = [
            (!entry.method.name.is_empty(), "method"),
            (!entry.equations.is_empty(), "equations"),
            (entry.update.is_some(), "update"),
            (!entry.settings.is_empty(), "settings"),
            (entry.start.is_some(), "start"),
        ];
        if let Some((_, what)) = extra.iter().find(|(set, _)| *set) {
            return Err(invalid(
                format!("{epath}.{what}"),
                format!(
                    "'{ty}' is a subsystem: its blocks have the {what}s ({})",
                    match *what {
                        "start" => "a subsystem's start states are 'starts', by inner block",
                        _ => "set them inside the subsystem",
                    }
                ),
            ));
        }
        let scope = self.declare(def, entry, outer, prefix, epath, prefix.is_empty())?;
        self.stack.push(ty.clone());
        let inner_prefix = format!("{flat}/");
        let path = format!("subsystems.{ty}.");
        let first = self.blocks.len();
        let items = self.level(&def.blocks, &def.connections, &inner_prefix, &path, &scope)?;
        self.stack.pop();
        // Inner values set for this instance only.
        for (k, v) in &entry.params {
            let Some((inner, param)) = k.rsplit_once('.') else {
                continue;
            };
            let target = format!("{inner_prefix}{inner}");
            let b = self.blocks[first..]
                .iter_mut()
                .find(|b| b.name == target)
                .ok_or_else(|| {
                    invalid(
                        format!("{epath}.params.{k}"),
                        format!("'{ty}' has no block '{inner}'"),
                    )
                })?;
            b.params.insert(param.into(), v.clone());
            self.plan
                .derived
                .retain(|d| !(d.block == target && d.param == param));
        }
        for (inner, start) in &entry.starts {
            let target = format!("{inner_prefix}{inner}");
            let b = self.blocks[first..]
                .iter_mut()
                .find(|b| b.name == target)
                .ok_or_else(|| {
                    invalid(
                        format!("{epath}.starts.{inner}"),
                        format!("'{ty}' has no block '{inner}'"),
                    )
                })?;
            b.start = Some(start.clone());
        }
        let (inputs, outputs) = interface(def, &items, &path)?;
        Ok(Item::Instance { inputs, outputs })
    }
}

impl ModelFile {
    /// Copy what is running back into the file as written: the model's
    /// declared values, instance parameter values and inner values that
    /// differ from the subsystem's, block parameters (formulas stay
    /// formulas) and start states. `flat` is the running flat file with
    /// its parameters captured.
    pub fn capture_from(&mut self, flat: &ModelFile, plan: &ParamPlan) {
        let top: BTreeMap<String, usize> = self
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.name.clone(), i))
            .collect();
        for d in &plan.declared {
            if let Some(p) = self.params.get_mut(&d.key) {
                p.value = d.value;
            } else if let Some((inst, pname)) = d.key.split_once('.') {
                if !d.editable {
                    continue;
                }
                if let Some(&i) = top.get(inst) {
                    let e = &mut self.blocks[i];
                    // Values written stay written; defaults stay implicit.
                    if e.params.contains_key(pname) || d.default != Some(d.value) {
                        e.params.insert(pname.into(), d.value.into());
                    }
                }
            }
        }
        let inner_base: BTreeMap<(&str, &str), f64> = plan
            .inner
            .iter()
            .map(|n| ((n.block.as_str(), n.param.as_str()), n.base))
            .collect();
        for fb in &flat.blocks {
            match fb.name.split_once('/') {
                None => {
                    let Some(&i) = top.get(fb.name.as_str()) else {
                        continue;
                    };
                    let e = &mut self.blocks[i];
                    for (k, v) in &fb.params {
                        if !matches!(e.params.get(k), Some(ParamValue::Formula(_))) {
                            e.params.insert(k.clone(), v.clone());
                        }
                    }
                    e.start = fb.start.clone();
                }
                Some((inst, rest)) => {
                    let Some(&i) = top.get(inst) else {
                        continue;
                    };
                    let e = &mut self.blocks[i];
                    for (k, v) in &fb.params {
                        let key = format!("{rest}.{k}");
                        if plan.formula_of(&fb.name, k).is_some() {
                            continue;
                        }
                        let base = inner_base.get(&(fb.name.as_str(), k.as_str())).copied();
                        let given = e.params.contains_key(&key);
                        match (v.number(), base) {
                            (Some(x), Some(b)) if x == b && !given => {}
                            (Some(x), Some(b)) if x == b => {
                                e.params.remove(&key);
                            }
                            (Some(x), _) => {
                                e.params.insert(key, x.into());
                            }
                            _ => {}
                        }
                    }
                    match &fb.start {
                        Some(s) => {
                            e.starts.insert(rest.into(), s.clone());
                        }
                        None => {
                            e.starts.remove(rest);
                        }
                    }
                }
            }
        }
    }
}

impl Check {
    /// Build the subsystem alone and check its ports name real ports.
    pub(crate) fn verify(
        &self,
        registry: &simsandbox_core::registry::Registry,
        folder: Option<&std::path::Path>,
    ) -> Result<(), FileError> {
        let (mut model, ids) = self
            .file
            .build(registry, folder)
            .map_err(|e| crate::origin(e, &self.block_origins, &self.connection_origins))?;
        self.plan.clone().fit_blocks(&mut model, &ids)?;
        for (path, port, input) in &self.ports {
            let (block, name) = port.split_once('.').expect("flat ports are block.port");
            let spec = model
                .block(ids[block])
                .map_err(|e| invalid(path, e))?
                .spec();
            let want = if *input {
                simsandbox_core::block::Causality::Input
            } else {
                simsandbox_core::block::Causality::Output
            };
            if !spec
                .ports
                .iter()
                .any(|p| p.name == name && p.causality == want)
            {
                let kind = if *input { "input" } else { "output" };
                let names: Vec<&str> = spec
                    .ports
                    .iter()
                    .filter(|p| p.causality == want)
                    .map(|p| p.name.as_str())
                    .collect();
                let inner = block.trim_start_matches("check/");
                return Err(invalid(
                    path,
                    format!(
                        "'{inner}' has no {kind} '{name}' ({kind}s: {})",
                        names.join(", ")
                    ),
                ));
            }
        }
        Ok(())
    }
}
