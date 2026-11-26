//! A scene's saved form (ADR 0093 point 8) and its check: the primitives
//! and their attributes, declared; frames, layers and bindings read from
//! the view's settings and checked against what the model offers, each
//! problem at its path.

use crate::history::{split_source, TIME};
use crate::view::{Available, Settings, SlotKind};
use egui::Color32;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// What an attribute is, which says what it may be bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A position or size in the scene's unit.
    Length,
    /// Radians, counter-clockwise.
    Angle,
    /// Screen points: a point's size, a line's width.
    Screen,
    /// A number through the scene's colour scale, or a palette name.
    Colour,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Length => "length",
            Kind::Angle => "angle",
            Kind::Screen => "screen",
            Kind::Colour => "colour",
        }
    }

    /// Its unit as `describe` gives it.
    pub fn unit(self) -> &'static str {
        match self {
            Kind::Length => "the scene's unit",
            Kind::Angle => "rad",
            Kind::Screen => "pt",
            Kind::Colour => "",
        }
    }
}

/// What an attribute is when it is not bound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unbound {
    /// It must be bound.
    Required,
    Number(f64),
    /// A palette colour.
    Colour(&'static str),
    /// Nothing is drawn for it (a disc's rotation mark).
    Nothing,
}

/// An attribute of a primitive or a frame, declared like a slot.
#[derive(Debug, Clone, Copy)]
pub struct Attribute {
    pub name: &'static str,
    pub kind: Kind,
    pub default: Unbound,
    pub description: &'static str,
}

/// A primitive a layer draws (ADR 0093 point 5).
#[derive(Debug)]
pub struct Primitive {
    pub name: &'static str,
    pub description: &'static str,
    pub attributes: &'static [Attribute],
}

impl PartialEq for Primitive {
    /// Primitives are told apart by name.
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Primitive {
    pub fn attribute(&self, name: &str) -> Option<usize> {
        self.attributes.iter().position(|a| a.name == name)
    }
}

const fn attr(
    name: &'static str,
    kind: Kind,
    default: Unbound,
    description: &'static str,
) -> Attribute {
    Attribute {
        name,
        kind,
        default,
        description,
    }
}

const COLOUR: Attribute = attr(
    "colour",
    Kind::Colour,
    Unbound::Colour(DEFAULT_COLOUR),
    "A number through the colour scale (0 dark to 1 bright, clamped), or a palette name",
);

/// The colour of anything not given one.
pub const DEFAULT_COLOUR: &str = "sky";

/// The primitives, in the order `describe` lists them.
pub const PRIMITIVES: &[Primitive] = &[
    Primitive {
        name: "points",
        description: "A dot of a size on screen at each position",
        attributes: &[
            attr("x", Kind::Length, Unbound::Required, "Across"),
            attr("y", Kind::Length, Unbound::Required, "Up"),
            attr(
                "size",
                Kind::Screen,
                Unbound::Number(6.0),
                "Diameter in screen points",
            ),
            COLOUR,
        ],
    },
    Primitive {
        name: "segments",
        description: "A straight line between two ends, of a width on screen",
        attributes: &[
            attr("x1", Kind::Length, Unbound::Required, "First end, across"),
            attr("y1", Kind::Length, Unbound::Required, "First end, up"),
            attr("x2", Kind::Length, Unbound::Required, "Second end, across"),
            attr("y2", Kind::Length, Unbound::Required, "Second end, up"),
            attr(
                "width",
                Kind::Screen,
                Unbound::Number(2.0),
                "Line width in screen points",
            ),
            COLOUR,
        ],
    },
    Primitive {
        name: "boxes",
        description: "A filled rectangle about its centre, turned by its angle",
        attributes: &[
            attr("x", Kind::Length, Unbound::Required, "Centre, across"),
            attr("y", Kind::Length, Unbound::Required, "Centre, up"),
            attr(
                "angle",
                Kind::Angle,
                Unbound::Number(0.0),
                "Turn, counter-clockwise",
            ),
            attr("width", Kind::Length, Unbound::Required, "Along its own x"),
            attr("height", Kind::Length, Unbound::Required, "Along its own y"),
            COLOUR,
        ],
    },
    Primitive {
        name: "discs",
        description: "A filled circle; bound, its angle draws a mark showing rotation",
        attributes: &[
            attr("x", Kind::Length, Unbound::Required, "Centre, across"),
            attr("y", Kind::Length, Unbound::Required, "Centre, up"),
            attr("radius", Kind::Length, Unbound::Required, "Radius"),
            attr(
                "angle",
                Kind::Angle,
                Unbound::Nothing,
                "Where a mark from the centre points; no mark if unbound",
            ),
            COLOUR,
        ],
    },
];

/// A frame's attributes: its pose in its parent (ADR 0093 point 4).
pub const FRAME: &[Attribute] = &[
    attr(
        "x",
        Kind::Length,
        Unbound::Number(0.0),
        "Origin in the parent, across",
    ),
    attr(
        "y",
        Kind::Length,
        Unbound::Number(0.0),
        "Origin in the parent, up",
    ),
    attr(
        "angle",
        Kind::Angle,
        Unbound::Number(0.0),
        "Turn in the parent, counter-clockwise",
    ),
];

/// The fixed palette of named colours: the Okabe–Ito colours, which stay
/// apart for colour-blind eyes, and three greys.
pub const PALETTE: &[(&str, Color32)] = &[
    ("black", Color32::from_rgb(0, 0, 0)),
    ("grey", Color32::from_rgb(150, 150, 150)),
    ("white", Color32::from_rgb(255, 255, 255)),
    ("orange", Color32::from_rgb(230, 159, 0)),
    ("sky", Color32::from_rgb(86, 180, 233)),
    ("green", Color32::from_rgb(0, 158, 115)),
    ("yellow", Color32::from_rgb(240, 228, 66)),
    ("blue", Color32::from_rgb(0, 114, 178)),
    ("red", Color32::from_rgb(213, 94, 0)),
    ("purple", Color32::from_rgb(204, 121, 167)),
];

pub fn palette(name: &str) -> Option<Color32> {
    PALETTE.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

/// The unit a scene's lengths are in when it names none.
pub const DEFAULT_UNIT: &str = "m";

/// What an attribute is bound to, checked.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Constant(f64),
    Colour(Color32),
    /// A scalar signal, read at its newest sample.
    Scalar(String),
    /// A quantity over a table's rows, one instance a row by id.
    Rows(String),
    /// A model parameter (`block.param`), no signal having the name
    /// (ADR 0102): read like a scalar signal, from the history, which
    /// records its value at every step.
    Param(String),
}

/// A frame, checked: its parent and its pose's bindings (x, y, angle).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub name: String,
    pub parent: Option<usize>,
    pub pose: [Source; 3],
    /// The table it is over, if any of its bindings is.
    pub table: Option<String>,
}

/// A layer, checked: its primitive, frame and each attribute's binding
/// (`None` where an optional attribute draws nothing).
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub primitive: &'static Primitive,
    pub frame: Option<usize>,
    pub attributes: Vec<Option<Source>>,
    /// The table its instances are rows of: its own bindings' or its
    /// frame's.
    pub table: Option<String>,
    /// The `input.pointer` block pressing on one of its instances sends
    /// the pointer to (ADR 0093 point 6).
    pub grab: Option<String>,
    /// The frame the pointer is sent in (`grab_in`): the scene's own if
    /// `None`. A template's layers grab in the frame the template is drawn
    /// in, so its model's pointer means what it means in its own scene
    /// (ADR 0113).
    pub grab_in: Option<usize>,
}

/// The region shown: x and y ranges in the scene's unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
    pub x: [f64; 2],
    pub y: [f64; 2],
}

/// A scene as checked: what [`super::eval`] places and draws.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub unit: String,
    pub extent: Option<Extent>,
    pub frames: Vec<Frame>,
    pub layers: Vec<Layer>,
    pub view_from: Option<usize>,
}

/// The settings a scene takes.
const KEYS: [&str; 5] = ["unit", "extent", "frames", "layers", "view_from"];

fn object<'v>(v: &'v Value, path: &str) -> Result<&'v Map<String, Value>, String> {
    v.as_object()
        .ok_or_else(|| format!("{path}: an object, not {v}"))
}

fn string<'v>(v: &'v Value, path: &str) -> Result<&'v str, String> {
    v.as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{path}: a name, not {v}"))
}

fn range(v: &Value, path: &str) -> Result<[f64; 2], String> {
    let r = v
        .as_array()
        .filter(|a| a.len() == 2)
        .and_then(|a| Some([a[0].as_f64()?, a[1].as_f64()?]));
    match r {
        Some([lo, hi]) if lo.is_finite() && hi.is_finite() && lo < hi => Ok([lo, hi]),
        _ => Err(format!(
            "{path}: an increasing finite range [low, high], not {v}"
        )),
    }
}

/// What is checked against: the model's offer and the scene's unit.
struct Checker<'a, 'b> {
    available: &'b Available<'a>,
    unit: String,
}

impl Checker<'_, '_> {
    /// A binding of an attribute of `kind`, at `path`.
    fn source(&self, v: &Value, kind: Kind, path: &str) -> Result<Source, String> {
        if let Some(x) = v.as_f64() {
            return if x.is_finite() {
                Ok(Source::Constant(x))
            } else {
                Err(format!("{path}: not a finite number"))
            };
        }
        let Some(name) = v.as_str() else {
            return Err(format!("{path}: a number or a signal's name, not {v}"));
        };
        if kind == Kind::Colour {
            if let Some(c) = palette(name) {
                return Ok(Source::Colour(c));
            }
        }
        // A signal wins over a parameter of the same name (ADR 0102).
        let source = if self.available.accepts(SlotKind::Scalar, name) {
            Source::Scalar(name.to_string())
        } else if self.available.accepts(SlotKind::Rows, name) {
            Source::Rows(name.to_string())
        } else if self.available.params.iter().any(|p| p == name) {
            Source::Param(name.to_string())
        } else {
            let what = if kind == Kind::Colour {
                "signal, parameter or palette colour"
            } else {
                "signal or parameter"
            };
            return Err(format!("{path}: no {what} '{name}'"));
        };
        let signal = split_source(name).1;
        let unit = if signal == TIME {
            Some("s")
        } else {
            self.available.meta.and_then(|h| h.unit(signal))
        };
        let Some(unit) = unit else {
            return Ok(source);
        };
        let wrong = match kind {
            Kind::Length if unit != self.unit => Some(format!(
                "the scene's lengths are in {}: convert it in the model",
                self.unit
            )),
            Kind::Angle if unit != "rad" => {
                Some("an angle takes radians or a dimensionless number".to_string())
            }
            Kind::Screen => Some("it is in screen points, a dimensionless number".to_string()),
            _ => None,
        };
        match wrong {
            Some(why) => Err(format!("{path}: '{name}' is in {unit}; {why}")),
            None => Ok(source),
        }
    }

    /// The table a field over rows is over: its block and the table its
    /// port names.
    fn table(&self, name: &str) -> String {
        let signal = split_source(name).1;
        let block = signal.split('.').next().unwrap_or(signal);
        let table = self.available.meta.and_then(|h| h.table(signal));
        match table {
            Some(t) => format!("{block}.{t}"),
            None => block.to_string(),
        }
    }

    /// The one table these bindings' rows are over, if any; an error at
    /// the first that is over another.
    fn one_table<'s>(
        &self,
        bound: impl Iterator<Item = (String, &'s Source)>,
        mut table: Option<(String, String)>,
    ) -> Result<Option<(String, String)>, String> {
        for (path, s) in bound {
            let Source::Rows(name) = s else { continue };
            let t = self.table(name);
            match &table {
                Some((first, at)) if *first != t => {
                    return Err(format!(
                        "{path}: over the rows of '{t}', but {at} is over the rows of '{first}': rows pair by id within one table"
                    ))
                }
                Some(_) => {}
                None => table = Some((t, path)),
            }
        }
        Ok(table)
    }
}

impl Scene {
    /// Check a scene's settings against what the model offers.
    pub fn check(settings: &Settings, available: &Available<'_>) -> Result<Scene, String> {
        if let Some(k) = settings.keys().find(|k| !KEYS.contains(&k.as_str())) {
            return Err(format!(
                "'{}' takes no setting '{k}' (it takes {})",
                super::SCENE,
                KEYS.join(", ")
            ));
        }
        let unit = match settings.get("unit") {
            Some(v) => string(v, "unit")?.to_string(),
            None => DEFAULT_UNIT.to_string(),
        };
        let extent = match settings.get("extent") {
            None => None,
            Some(v) => {
                let e = object(v, "extent")?;
                if let Some(k) = e.keys().find(|k| *k != "x" && *k != "y") {
                    return Err(format!("extent.{k}: takes only x and y"));
                }
                let axis = |a: &str| {
                    let path = format!("extent.{a}");
                    e.get(a)
                        .ok_or_else(|| format!("{path}: required"))
                        .and_then(|v| range(v, &path))
                };
                Some(Extent {
                    x: axis("x")?,
                    y: axis("y")?,
                })
            }
        };
        let c = Checker { available, unit };
        let empty = Map::new();
        let given_frames = match settings.get("frames") {
            Some(v) => object(v, "frames")?,
            None => &empty,
        };
        let names: Vec<&String> = given_frames.keys().collect();
        let index = |name: &str| names.iter().position(|n| *n == name);
        let mut frames = Vec::new();
        for (name, v) in given_frames {
            let path = format!("frames.{name}");
            let f = object(v, &path)?;
            if let Some(k) = f
                .keys()
                .find(|k| !["in", "x", "y", "angle"].contains(&k.as_str()))
            {
                return Err(format!("{path}.{k}: a frame takes in, x, y and angle"));
            }
            let parent = match f.get("in") {
                None => None,
                Some(p) => {
                    let p = string(p, &format!("{path}.in"))?;
                    Some(index(p).ok_or_else(|| format!("{path}.in: no frame '{p}'"))?)
                }
            };
            let mut pose = Vec::new();
            for a in FRAME {
                let at = format!("{path}.{}", a.name);
                pose.push(match f.get(a.name) {
                    Some(v) => c.source(v, a.kind, &at)?,
                    None => match a.default {
                        Unbound::Number(x) => Source::Constant(x),
                        _ => unreachable!("frame attributes default to numbers"),
                    },
                });
            }
            let table = c.one_table(
                FRAME
                    .iter()
                    .zip(&pose)
                    .map(|(a, s)| (format!("{path}.{}", a.name), s)),
                None,
            )?;
            frames.push(Frame {
                name: name.clone(),
                parent,
                pose: pose.try_into().expect("three"),
                table: table.map(|(t, _)| t),
            });
        }
        // Parents: no cycles (ADR 0093 point 4).
        for (k, f) in frames.iter().enumerate() {
            let path = format!("frames.{}.in", f.name);
            let mut chain = vec![k];
            let mut at = f.parent;
            while let Some(p) = at {
                if chain.contains(&p) {
                    chain.push(p);
                    let names: Vec<&str> = chain.iter().map(|&i| frames[i].name.as_str()).collect();
                    return Err(format!(
                        "{path}: the frames {} form a cycle",
                        names.join(" in ")
                    ));
                }
                chain.push(p);
                at = frames[p].parent;
            }
        }
        // A frame in a frame over rows is over its rows too, one pose a
        // row (ADR 0093, "frames over rows as parents"); along a chain,
        // rows are of one table, paired by id.
        let own: Vec<Option<String>> = frames.iter().map(|f| f.table.clone()).collect();
        for k in 0..frames.len() {
            let mut table: Option<(String, String)> = None;
            let mut at = Some(k);
            while let Some(f) = at {
                if let Some(t) = &own[f] {
                    match &table {
                        Some((first, by)) if first != t => {
                            return Err(format!(
                                "frames.{}: over the rows of '{first}' (by {by}), but its frame '{}' is over the rows of '{t}': rows pair by id within one table",
                                frames[k].name, frames[f].name
                            ));
                        }
                        Some(_) => {}
                        None => table = Some((t.clone(), frames[f].name.clone())),
                    }
                }
                at = frames[f].parent;
            }
            frames[k].table = table.map(|(t, _)| t);
        }
        let given_layers: &[Value] = match settings.get("layers") {
            Some(v) => v
                .as_array()
                .ok_or_else(|| format!("layers: a list of layers, not {v}"))?,
            None => &[],
        };
        let mut layers = Vec::new();
        for (k, v) in given_layers.iter().enumerate() {
            let path = format!("layers[{k}]");
            let l = object(v, &path)?;
            if let Some(key) = l
                .keys()
                .find(|key| !["draw", "in", "bind", "grab", "grab_in"].contains(&key.as_str()))
            {
                return Err(format!(
                    "{path}.{key}: a layer takes draw, in, bind, grab and grab_in"
                ));
            }
            let draw = string(
                l.get("draw")
                    .ok_or_else(|| format!("{path}.draw: required"))?,
                &format!("{path}.draw"),
            )?;
            let primitive = PRIMITIVES.iter().find(|p| p.name == draw).ok_or_else(|| {
                let all: Vec<&str> = PRIMITIVES.iter().map(|p| p.name).collect();
                format!("{path}.draw: no primitive '{draw}' ({})", all.join(", "))
            })?;
            let frame = match l.get("in") {
                None => None,
                Some(f) => {
                    let f = string(f, &format!("{path}.in"))?;
                    Some(index(f).ok_or_else(|| format!("{path}.in: no frame '{f}'"))?)
                }
            };
            let bind = match l.get("bind") {
                Some(b) => object(b, &format!("{path}.bind"))?,
                None => &empty,
            };
            if let Some(a) = bind.keys().find(|a| primitive.attribute(a).is_none()) {
                let all: Vec<&str> = primitive.attributes.iter().map(|a| a.name).collect();
                return Err(format!(
                    "{path}.bind.{a}: '{draw}' has no attribute '{a}' ({})",
                    all.join(", ")
                ));
            }
            let mut attributes = Vec::new();
            for a in primitive.attributes {
                let at = format!("{path}.bind.{}", a.name);
                attributes.push(match (bind.get(a.name), a.default) {
                    (Some(v), _) => Some(c.source(v, a.kind, &at)?),
                    (None, Unbound::Required) => return Err(format!("{at}: required")),
                    (None, Unbound::Number(x)) => Some(Source::Constant(x)),
                    (None, Unbound::Colour(n)) => Some(Source::Colour(
                        palette(n).expect("defaults are palette colours"),
                    )),
                    (None, Unbound::Nothing) => None,
                });
            }
            // Rows of one table, the layer's own and its frame's.
            let in_frame = frame
                .and_then(|f| frames[f].table.clone())
                .map(|t| (t, format!("{path}.in")));
            let table = c.one_table(
                primitive
                    .attributes
                    .iter()
                    .zip(&attributes)
                    .filter_map(|(a, s)| Some((format!("{path}.bind.{}", a.name), s.as_ref()?))),
                in_frame,
            )?;
            let grab = match l.get("grab") {
                None => None,
                Some(g) => {
                    let g = string(g, &format!("{path}.grab"))?;
                    if available
                        .pointers
                        .is_some_and(|p| !p.iter().any(|n| n == g))
                    {
                        return Err(format!(
                            "{path}.grab: no input.pointer block '{g}' (grabbing sends the pointer to one)"
                        ));
                    }
                    Some(g.to_string())
                }
            };
            let grab_in = match l.get("grab_in") {
                None => None,
                Some(f) => {
                    let at = format!("{path}.grab_in");
                    let f = string(f, &at)?;
                    let k = index(f).ok_or_else(|| format!("{at}: no frame '{f}'"))?;
                    if frames[k].table.is_some() {
                        return Err(format!(
                            "{at}: '{f}' is over a table's rows: the pointer is sent in a frame of one pose"
                        ));
                    }
                    if grab.is_none() {
                        return Err(format!("{at}: the layer does not grab"));
                    }
                    Some(k)
                }
            };
            layers.push(Layer {
                primitive,
                frame,
                attributes,
                table: table.map(|(t, _)| t),
                grab,
                grab_in,
            });
        }
        let view_from = match settings.get("view_from") {
            None => None,
            Some(v) => {
                let f = string(v, "view_from")?;
                let k = index(f).ok_or_else(|| format!("view_from: no frame '{f}'"))?;
                if frames[k].table.is_some() {
                    return Err(format!(
                        "view_from: '{f}' is over a table's rows, one frame a row: view from a frame of one pose"
                    ));
                }
                Some(k)
            }
        };
        Ok(Scene {
            unit: c.unit,
            extent,
            frames,
            layers,
            view_from,
        })
    }

    /// The signals it reads, as written, once each.
    pub fn reads(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .frames
            .iter()
            .flat_map(|f| f.pose.iter())
            .chain(
                self.layers
                    .iter()
                    .flat_map(|l| l.attributes.iter().flatten()),
            )
            .filter_map(|s| match s {
                Source::Scalar(n) | Source::Rows(n) | Source::Param(n) => Some(n.clone()),
                _ => None,
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }
}

/// Where a scene's settings bind signals, as written, without checking
/// them: each binding's path as `check` names it (`frames.cart.x`,
/// `layers[1].bind.x`) and the signal, frames first (by name), then
/// layers in order. Numbers and palette colours are not signals. A
/// layer's `grab` is not a signal; see [`grabs`].
pub fn bound(settings: &Settings) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(frames) = settings.get("frames").and_then(Value::as_object) {
        for (name, f) in frames {
            let Some(f) = f.as_object() else { continue };
            for a in FRAME {
                if let Some(s) = f.get(a.name).and_then(Value::as_str) {
                    out.push((format!("frames.{name}.{}", a.name), s.to_string()));
                }
            }
        }
    }
    if let Some(layers) = settings.get("layers").and_then(Value::as_array) {
        for (k, l) in layers.iter().enumerate() {
            let Some(bind) = l.get("bind").and_then(Value::as_object) else {
                continue;
            };
            // In the primitive's order where it is known.
            let mut names: Vec<&String> = bind.keys().collect();
            if let Some(p) = l
                .get("draw")
                .and_then(Value::as_str)
                .and_then(|d| PRIMITIVES.iter().find(|p| p.name == d))
            {
                names.sort_by_key(|n| p.attribute(n).unwrap_or(usize::MAX));
            }
            for a in names {
                let Some(s) = bind[a].as_str() else { continue };
                if a == "colour" && palette(s).is_some() {
                    continue;
                }
                out.push((format!("layers[{k}].bind.{a}"), s.to_string()));
            }
        }
    }
    out
}

/// The `input.pointer` blocks a scene's layers grab with, as written:
/// each layer's path (`layers[3].grab`) and the block.
pub fn grabs(settings: &Settings) -> Vec<(String, String)> {
    settings
        .get("layers")
        .and_then(Value::as_array)
        .map(|layers| {
            layers
                .iter()
                .enumerate()
                .filter_map(|(k, l)| {
                    let g = l.get("grab")?.as_str()?;
                    Some((format!("layers[{k}].grab"), g.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The signals a scene's settings bind, as written, without checking
/// them: what the history must record for it (ADR 0063), once each.
pub fn reads(settings: &Settings) -> Vec<String> {
    let mut out: Vec<String> = bound(settings).into_iter().map(|(_, s)| s).collect();
    out.sort();
    out.dedup();
    out
}

/// The path a `check` error is at (`layers[1].bind.x` of
/// `"layers[1].bind.x: required"`), if it names one.
pub fn error_path(error: &str) -> Option<&str> {
    let (path, _) = error.split_once(": ")?;
    let first = path.split(['.', '[']).next()?;
    (KEYS.contains(&first) && !path.contains(' ')).then_some(path)
}

fn describe_attributes(attributes: &[Attribute]) -> Vec<Value> {
    attributes
        .iter()
        .map(|a| {
            let default = match a.default {
                Unbound::Required => Value::Null,
                Unbound::Number(x) => json!(x),
                Unbound::Colour(n) => json!(n),
                Unbound::Nothing => json!("none"),
            };
            json!({
                "name": a.name,
                "kind": a.kind.name(),
                "unit": a.kind.unit(),
                "required": a.default == Unbound::Required,
                "default": default,
                "description": a.description,
            })
        })
        .collect()
}

/// What a scene takes, for `describe` (ADR 0093 point 5).
pub fn describe() -> Value {
    let bindings: BTreeMap<&str, &str> = [
        ("number", "a constant"),
        ("signal", "a scalar signal ('block.port'): one instance, its newest value"),
        ("parameter", "a model parameter ('block.param') where no signal has that name (a signal wins): its value at the drawn instant, recorded at every step like a signal, so a rewound or replayed run draws as it was"),
        ("rows", "a quantity over a table's rows: one instance a row, paired with the layer's other rows (and its frame's) by id"),
        ("colour", "colour also takes a palette name"),
    ]
    .into();
    json!({
        "settings": {
            "unit": format!("the length unit of every position and size ('{DEFAULT_UNIT}' if absent); a binding in another length unit is refused"),
            "extent": "{\"x\": [low, high], \"y\": [low, high]}: the region shown; absent, it follows what is drawn. Drawn with equal scales",
            "frames": "name to {\"in\": parent, \"x\", \"y\", \"angle\"}: a point p drawn in the frame lies at (x, y) + R(angle)·p in its parent (the scene's if 'in' is absent). Frames chain; a frame in a frame over a table's rows is over its rows too, one pose a row, so a template drawn in it is drawn once a row",
            "layers": "a list of {\"draw\": primitive, \"in\": frame, \"bind\": attribute to binding, \"grab\": input.pointer block}, drawn first to last. Pressing on an instance of a layer with 'grab' sends that block x and y in the scene's own frame (whatever the layer's frame or view_from), or in the frame 'grab_in' names (a template's layers grab in the frame the template is drawn in, ADR 0113), down and picked (the row id, 0 for a layer of one instance; -1 once let go) until released; the topmost such layer wins, and pan and zoom pause while it is held",
            "view_from": "a frame: the picture is drawn in its coordinates",
        },
        "bindings": bindings,
        "palette": PALETTE.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        "frame": describe_attributes(FRAME),
        "primitives": PRIMITIVES.iter().map(|p| json!({
            "name": p.name,
            "description": p.description,
            "attributes": describe_attributes(p.attributes),
        })).collect::<Vec<_>>(),
    })
}
