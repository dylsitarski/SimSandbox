//! Parameter rows and value fields, shared by the inspector, the Controls
//! and view styles: typed values and formulas (ADR 0056), sliders that
//! apply on release (ADR 0037), pins (ADR 0012), several uses side by side
//! and the values of blocks not running yet.

use super::*;

/// What the parameter panel collects from its rows this frame.
pub(super) struct Edits<'a> {
    /// New values to apply.
    pub(super) changes: &'a mut Vec<(ParamRow, f64)>,
    /// Slider choices and ranges to save in `controls` (ADR 0012).
    pub(super) controls: &'a mut Vec<(String, simsandbox_file::ParamControl)>,
    /// Text being typed, by field.
    pub(super) text: &'a mut HashMap<String, String>,
    /// Values of sliders being dragged that apply on release (ADR 0037).
    pub(super) pending: &'a mut HashMap<String, f64>,
    pub(super) error: &'a mut Option<String>,
    /// Formulas, and numbers in place of them: file edits (ADR 0056).
    pub(super) file: &'a mut Vec<(Scope, Edit)>,
}

/// Parameters side by side for several uses of a subsystem (or one use):
/// a row per parameter, a column per use. Values formulas set are shown,
/// not edited; right-click a name to give every use the first one's value.
pub(super) fn uses_table(
    ui: &mut egui::Ui,
    t: &crate::inspector::UsesTable,
    pins: &BTreeMap<String, simsandbox_file::ParamControl>,
    edits: Edits<'_>,
) {
    let Edits {
        changes,
        controls,
        text,
        error,
        file,
        ..
    } = edits;
    egui::Grid::new(("uses-table", &t.subsystem, t.uses.join(",")))
        .num_columns(t.uses.len() + 2)
        .spacing([8.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            if t.uses.len() > 1 {
                ui.label("");
                ui.label("");
                for u in &t.uses {
                    ui.strong(u);
                }
                ui.end_row();
            }
            let mut divided = false;
            for (name, cells) in &t.rows {
                let first = cells.iter().flatten().next();
                // A value inside a used model: read here, changed through
                // its model (ADR 0110 point 6).
                if let Some(f) = first.filter(|r| is_inside(r)) {
                    if !divided {
                        divided = true;
                        ui.weak("inside")
                            .on_hover_text("Values inside the model used: right-click one to expose it as a parameter of the model, or to change it in the model for every use");
                        ui.end_row();
                    }
                    let response = ui.label(name).on_hover_text(inside_hover(f));
                    response.context_menu(|ui| {
                        inside_menu(ui, f, text, file, error);
                        pin_items(ui, cells, pins, controls);
                    });
                    ui.weak(f.unit.as_str());
                    for cell in cells {
                        match cell {
                            Some(r) => {
                                ui.weak(shown_value(r)).on_hover_text(inside_hover(r));
                            }
                            None => {
                                ui.weak("—");
                            }
                        }
                    }
                    ui.end_row();
                    continue;
                }
                let starting = first.is_some_and(|r| r.initial_condition);
                let label = if starting {
                    format!("{name} ↻")
                } else {
                    name.clone()
                };
                let hover = first.map_or(String::new(), |r| {
                    let mut h = r.description.clone();
                    if starting {
                        h.push_str("\nSets the initial state: applies on Restart");
                    }
                    h
                });
                let first_value = cells
                    .iter()
                    .flatten()
                    .find(|r| r.formula.is_none() && r.driven.is_none())
                    .map(|r| r.value);
                let response = ui
                    .label(label)
                    .on_hover_text(format!("{hover}\nRight-click to pin or set every use"));
                response.context_menu(|ui| {
                    if let Some(v) = first_value.filter(|_| t.uses.len() > 1) {
                        if ui
                            .button(format!("Set every use to {}", format_value(v)))
                            .clicked()
                        {
                            for r in cells
                                .iter()
                                .flatten()
                                .filter(|r| r.formula.is_none() && r.driven.is_none())
                            {
                                changes.push((r.clone(), v));
                            }
                            ui.close();
                        }
                    }
                    pin_items(ui, cells, pins, controls);
                });
                ui.weak(first.map_or("", |r| r.unit.as_str()));
                for cell in cells {
                    match cell {
                        Some(r) if r.driven.is_some() => {
                            let from = r.driven.as_deref().unwrap_or_default();
                            ui.weak(format!("from {from}"))
                                .on_hover_text(format!("Driven by {from} (ADR 0075)"));
                        }
                        Some(r) => {
                            let key = format!("cell:{}", r.key);
                            let f = r.formula.as_deref();
                            if let Some(t) = value_entry(ui, &key, r.value, f, text, error) {
                                apply_typed(r, t, changes, file, error);
                            }
                        }
                        None => {
                            ui.weak("—");
                        }
                    }
                }
                ui.end_row();
            }
        });
}

/// Pin or unpin each use's row.
fn pin_items(
    ui: &mut egui::Ui,
    cells: &[Option<ParamRow>],
    pins: &BTreeMap<String, simsandbox_file::ParamControl>,
    controls: &mut Vec<(String, simsandbox_file::ParamControl)>,
) {
    for r in cells.iter().flatten() {
        let mut c = pins.get(&r.key).cloned().unwrap_or_default();
        let pinned = c.pinned();
        let label = match pinned {
            true => format!("Unpin {} from the Controls", r.key),
            false => format!("Pin {} to the Controls", r.key),
        };
        if ui.button(label).clicked() {
            c.set_pinned(!pinned);
            controls.push((r.key.clone(), c));
            ui.close();
        }
    }
}

/// Whether a row is a value inside a used model (`car1/car.c`), not a
/// parameter the model declares (`car1.mass`).
pub(super) fn is_inside(r: &ParamRow) -> bool {
    r.owner.contains('/')
}

fn shown_value(r: &ParamRow) -> String {
    let v = format_value(r.value);
    match (&r.driven, &r.formula) {
        (Some(d), _) => format!("from {d}"),
        (None, Some(f)) => format!("{v} (= {f})"),
        _ => v,
    }
}

fn inside_hover(r: &ParamRow) -> String {
    let use_name = r.owner.split('/').next().unwrap_or("");
    format!(
        "{}\nInside {use_name}'s model: a use changes it only through its model's parameters (ADR 0110). Right-click to expose it as one, or to change it in the model for every use. Pinned, it is an experiment's slider (ADR 0112).",
        r.description
    )
}

/// Expose a value inside a used model, or change it there for every use.
fn inside_menu(
    ui: &mut egui::Ui,
    r: &ParamRow,
    text: &mut HashMap<String, String>,
    file: &mut Vec<(Scope, Edit)>,
    error: &mut Option<String>,
) {
    if ui
        .button("Expose as a parameter of its model")
        .on_hover_text("Each use then sets its own value")
        .clicked()
    {
        file.push((Scope::Model, Edit::ExposeInside { key: r.key.clone() }));
        ui.close();
    }
    ui.horizontal(|ui| {
        ui.label("In the model, for every use:");
        let key = format!("inside:{}", r.key);
        let entry = text
            .entry(key.clone())
            .or_insert_with(|| format_value(r.value));
        let response = ui.add(egui::TextEdit::singleline(entry).desired_width(70.0));
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            match entry.trim().parse::<f64>() {
                Ok(value) => {
                    file.push((
                        Scope::Model,
                        Edit::SetInside {
                            key: r.key.clone(),
                            value,
                        },
                    ));
                    text.remove(&key);
                    ui.close();
                }
                Err(_) => *error = Some(format!("'{}' is not a number", entry.trim())),
            }
        }
    });
}

/// A value inside a used model in the Inspector's tree: read-only, with
/// [`inside_menu`] (ADR 0110 point 6).
pub(super) fn inside_row(ui: &mut egui::Ui, r: &ParamRow, edits: Edits<'_>) {
    let Edits {
        controls,
        text,
        error,
        file,
        ..
    } = edits;
    ui.horizontal(|ui| {
        ui.label(&r.name)
            .on_hover_text(inside_hover(r))
            .context_menu(|ui| {
                inside_menu(ui, r, text, file, error);
                let mut c = simsandbox_file::ParamControl::default();
                if ui.button("Pin to the Controls").clicked() {
                    c.set_pinned(true);
                    controls.push((r.key.clone(), c));
                    ui.close();
                }
            });
        ui.weak(shown_value(r));
        ui.weak(&r.unit);
    });
}

/// Where a parameter row is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    /// The inspector's tree: a pin button.
    Tree,
    /// The Controls: the slider toggle and the pin button.
    Controls,
}

/// One parameter: name, typed entry, unit (↻ for a starting value), the
/// slider toggle among the Controls and a pin button; then the slider and
/// its range when enabled. `used_by` lists the formulas that read it.
pub(super) fn param_row(
    ui: &mut egui::Ui,
    m: &Model,
    row: &ParamRow,
    place: Place,
    used_by: Option<&Vec<String>>,
    edits: Edits<'_>,
) {
    let mut control = m
        .file
        .controls
        .params
        .get(&row.key)
        .cloned()
        .unwrap_or_default();
    let before = control.clone();
    let mut hover = format!(
        "{}\nValue: {}\nSuggested range: {} to {}",
        row.description, row.value, row.suggested[0], row.suggested[1]
    );
    if row.initial_condition {
        hover.push_str("\nSets the initial state: applies on Restart");
    }
    if let Some(f) = &row.formula {
        hover.push_str(&format!("\nComputed: {f}"));
    }
    hover.push_str("\nType a number, or '= expression' of the parameters for a formula");
    if let Some(v) = used_by {
        hover.push_str(&format!("\nRead by formulas: {}", v.join(", ")));
    }
    ui.label(&row.name).on_hover_text(hover).context_menu(|ui| {
        let label = if control.pinned() {
            "Unpin from the Controls"
        } else {
            "Pin to the Controls"
        };
        if ui.button(label).clicked() {
            let pinned = control.pinned();
            control.set_pinned(!pinned);
            ui.close();
        }
        // The model's own parameters can go (ADR 0056); formulas reading
        // one keep it.
        if row.owner.is_empty()
            && row.block.is_none()
            && ui
                .button("Remove this parameter")
                .on_hover_text("Refused while a formula reads it")
                .clicked()
        {
            edits.file.push((
                Scope::Model,
                Edit::RemoveParam {
                    name: row.key.clone(),
                },
            ));
            ui.close();
        }
    });

    // A value a formula sets shows the formula, which can be edited or
    // replaced by a number (ADR 0056). While a slider is dragged on a model
    // that is not real-time, its field shows the value that applies on
    // release (ADR 0037).
    let formula = row.formula.as_deref();
    // Driven by a wire (ADR 0075): its source, not a value to type.
    let driven = row.driven.as_deref();
    let typed = match (driven, edits.pending.get(&row.key)) {
        (Some(from), _) => {
            ui.weak(format!("from {from}")).on_hover_text(format!(
                "Driven by {from}: it takes that signal's value (ADR 0075). Drag the wire \
                 off its dot on the diagram to set it again"
            ));
            None
        }
        (None, Some(p)) => {
            pending_field(ui, *p);
            None
        }
        (None, None) => value_entry(ui, &row.key, row.value, formula, edits.text, edits.error),
    };
    if let Some(t) = typed {
        apply_typed(row, t, edits.changes, edits.file, edits.error);
    }

    ui.horizontal(|ui| {
        if formula.is_some() || driven.is_some() {
            ui.weak(format_value(row.value))
                .on_hover_text("Its value now");
        }
        ui.label(&row.unit);
        if row.initial_condition {
            ui.weak("↻")
                .on_hover_text("Initial condition: applies on Restart");
        }
    });
    ui.horizontal(|ui| {
        if place == Place::Controls && formula.is_none() && driven.is_none() {
            ui.checkbox(&mut control.slider, "")
                .on_hover_text("A slider under the value, between the two ends of its range");
        }
        pin_button(ui, &mut control);
    });
    ui.end_row();

    // The slider between its range's ends, which are typed in place.
    if place == Place::Controls && control.slider && formula.is_none() && driven.is_none() {
        let [mut lo, mut hi] = control.range.unwrap_or(row.suggested);
        let a = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::DragValue::new(&mut lo).speed(0.1))
                    .on_hover_text("The slider's lower end")
            })
            .inner;
        let mut v = edits.pending.get(&row.key).copied().unwrap_or(row.value);
        let r = ui.add(egui::Slider::new(&mut v, lo..=hi).show_value(false));
        if apply_now(m.real_time, r.changed(), r.dragged(), r.drag_stopped()) {
            edits.pending.remove(&row.key);
            edits.changes.push((row.clone(), v));
        } else if r.changed() {
            edits.pending.insert(row.key.clone(), v);
        }
        let b = ui
            .add(egui::DragValue::new(&mut hi).speed(0.1))
            .on_hover_text("The slider's upper end");
        if (a.changed() || b.changed()) && lo < hi {
            control.range = Some([lo, hi]);
        }
        ui.end_row();
    }
    if control != before {
        edits.controls.push((row.key.clone(), control));
    }
}

/// The pin: highlighted while the value is among the Controls.
pub(super) fn pin_button(ui: &mut egui::Ui, control: &mut simsandbox_file::ParamControl) {
    let pinned = control.pinned();
    let text = if pinned {
        egui::RichText::new("📌")
    } else {
        egui::RichText::new("📌").weak()
    };
    let hover = if pinned {
        "Pinned: shown in the Controls. Click to unpin."
    } else {
        "Pin to the Controls, where it can also have a slider"
    };
    if ui
        .add(
            egui::Button::new(text)
                .small()
                .frame(pinned)
                .selected(pinned),
        )
        .on_hover_text(hover)
        .clicked()
    {
        control.set_pinned(!pinned);
    }
}

/// Parameter rows nested by path: this level's in a grid, then each
/// level inside, collapsed, by name (ADR 0066 step 4b).
pub(super) fn nested_rows(
    ui: &mut egui::Ui,
    n: &crate::inspector::Nested,
    salt: &str,
    row: &mut dyn FnMut(&mut egui::Ui, &str, &simsandbox_session::session::ParamRow),
) {
    if !n.rows.is_empty() {
        egui::Grid::new(("params", salt))
            .num_columns(4)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                for (name, r) in &n.rows {
                    row(ui, name, r);
                }
            });
    }
    for (name, inner) in &n.inner {
        let salt = format!("{salt}/{name}");
        egui::CollapsingHeader::new(name)
            .id_salt(("nested", &salt))
            .default_open(false)
            .show(ui, |ui| nested_rows(ui, inner, &salt, row));
    }
}

/// The values of a block not in the running model yet (the diagram's
/// draft), typed into the draft.
pub(super) fn draft_rows(
    ui: &mut egui::Ui,
    n: &simsandbox_session::diagram::Node,
    text: &mut HashMap<String, String>,
    error: &mut Option<String>,
) -> Vec<Edit> {
    let mut out = Vec::new();
    egui::Grid::new(("draft-rows", &n.name))
        .num_columns(3)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            for v in &n.values {
                let label = if v.initial {
                    format!("{} ↻", v.name)
                } else {
                    v.name.clone()
                };
                ui.label(label);
                let key = format!("draft:{}", v.key);
                let (block, param) = (n.name.clone(), v.name.clone());
                match value_entry(ui, &key, v.value, None, text, error) {
                    Some(Typed::Number(value)) => out.push(Edit::SetParam {
                        block,
                        param,
                        value,
                    }),
                    Some(Typed::Formula(formula)) => out.push(Edit::SetFormula {
                        block,
                        param,
                        formula,
                    }),
                    None => {}
                }
                ui.label(&v.unit);
                ui.end_row();
            }
        });
    out
}

/// Whether a slider's value applies now (ADR 0037): on every change for a
/// real-time model, otherwise when the drag ends (or on a click or key).
pub(super) fn apply_now(real_time: bool, changed: bool, dragged: bool, drag_stopped: bool) -> bool {
    if real_time {
        changed
    } else {
        drag_stopped || (changed && !dragged)
    }
}

/// A parameter field showing a slider's pending value: the same size as
/// the typed field, in the warning colour, not editable.
pub(super) fn pending_field(ui: &mut egui::Ui, value: f64) {
    let mut text = format_value(value);
    ui.add(
        egui::TextEdit::singleline(&mut text)
            .desired_width(90.0)
            .interactive(false)
            .text_color(ui.visuals().warn_fg_color),
    )
    .on_hover_text("Pending: this model is not real-time, so the value applies when you let go");
}

/// A typed number field. It shows a compact value and returns a new value
/// only when the user typed a different one, so the rounding of the display
/// never changes anything. `key` identifies the field's edit buffer.
pub(super) fn number_entry(
    ui: &mut egui::Ui,
    key: &str,
    value: f64,
    edits: &mut HashMap<String, String>,
    error: &mut Option<String>,
) -> Option<f64> {
    let shown = format_value(value);
    let mut text = edits.get(key).cloned().unwrap_or_else(|| shown.clone());
    let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(90.0));
    if r.has_focus() {
        edits.insert(key.into(), text.clone());
    }
    if !r.lost_focus() {
        return None;
    }
    edits.remove(key);
    let typed = text.trim();
    if typed == shown {
        return None;
    }
    match typed.parse::<f64>() {
        Ok(v) => Some(v),
        Err(_) => {
            *error = Some(format!("{key}: '{typed}' is not a number"));
            None
        }
    }
}

/// "+ parameter": a name, a value and a unit, declared as a parameter of
/// the model (ADR 0056).
pub(super) fn declare_form(
    ui: &mut egui::Ui,
    form: &mut [String; 3],
    error: &mut Option<String>,
) -> Option<Edit> {
    let mut add = false;
    ui.horizontal(|ui| {
        let [name, value, unit] = form;
        for (field, hint, width) in [
            (name, "name", 70.0),
            (value, "value", 60.0),
            (unit, "unit", 50.0),
        ] {
            let r = ui.add(
                egui::TextEdit::singleline(field)
                    .hint_text(hint)
                    .desired_width(width),
            );
            add |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        }
        add |= ui
            .small_button("+ parameter")
            .on_hover_text(
                "Declare a parameter of the model; type '= name' in a block's value to use it",
            )
            .clicked();
    });
    if !add || form[0].trim().is_empty() {
        return None;
    }
    let value = if form[1].trim().is_empty() {
        Ok(0.0)
    } else {
        form[1].trim().parse::<f64>()
    };
    match value {
        Ok(value) if value.is_finite() => {
            let e = Edit::DeclareParam {
                name: form[0].trim().to_string(),
                value,
                unit: form[2].trim().to_string(),
            };
            *form = Default::default();
            Some(e)
        }
        _ => {
            *error = Some(format!("'{}' is not a number", form[1].trim()));
            None
        }
    }
}

/// What was typed into a parameter's field (ADR 0056).
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Typed {
    Number(f64),
    /// `= expression`, without the `=`.
    Formula(String),
}

/// A parameter's field: a number, or `= expression` for a value a formula
/// sets. Like [`number_entry`], it returns only what was typed anew.
pub(super) fn value_entry(
    ui: &mut egui::Ui,
    key: &str,
    value: f64,
    formula: Option<&str>,
    edits: &mut HashMap<String, String>,
    error: &mut Option<String>,
) -> Option<Typed> {
    let shown = match formula {
        Some(f) => format!("= {f}"),
        None => format_value(value),
    };
    let mut text = edits.get(key).cloned().unwrap_or_else(|| shown.clone());
    let r = ui
        .add(egui::TextEdit::singleline(&mut text).desired_width(90.0))
        .on_hover_text("A number, or '= expression' of the model's parameters (e.g. '= 2*kp')");
    if r.has_focus() {
        edits.insert(key.into(), text.clone());
    }
    if !r.lost_focus() {
        return None;
    }
    edits.remove(key);
    let typed = text.trim();
    if typed == shown {
        return None;
    }
    if let Some(f) = typed.strip_prefix('=') {
        let f = f.trim();
        return (Some(f) != formula).then(|| Typed::Formula(f.to_string()));
    }
    match typed.parse::<f64>() {
        Ok(v) if v.is_finite() => Some(Typed::Number(v)),
        _ => {
            *error = Some(format!(
                "{key}: '{typed}' is not a number (or '= expression' for a formula)"
            ));
            None
        }
    }
}

/// Where a value's formula is written (ADR 0056): the block, or subsystem
/// use, on the model level, and the parameter. The model's own parameters
/// and values inside uses are numbers.
pub(super) fn formula_target(row: &ParamRow) -> Result<(String, String), String> {
    if row.owner.is_empty() {
        return Err(format!(
            "{}: a parameter of the model is a number; formulas go in block values, which \
             can read it",
            row.key
        ));
    }
    if row.owner.contains('/') {
        return Err(format!(
            "{}: a value inside a subsystem use is a number; open the subsystem to give every \
             use a formula, or declare a subsystem parameter",
            row.key
        ));
    }
    let param = row
        .key
        .strip_prefix(&format!("{}.", row.owner))
        .unwrap_or(&row.name);
    Ok((row.owner.clone(), param.to_string()))
}

/// Carry out what was typed into a row: a number on a value without a
/// formula applies at once; a formula, or a number in place of one,
/// changes the file (and restarts).
pub(super) fn apply_typed(
    row: &ParamRow,
    typed: Typed,
    changes: &mut Vec<(ParamRow, f64)>,
    file: &mut Vec<(Scope, Edit)>,
    error: &mut Option<String>,
) {
    if let (Typed::Number(v), None) = (&typed, &row.formula) {
        changes.push((row.clone(), *v));
        return;
    }
    match formula_target(row) {
        Ok((block, param)) => file.push((
            Scope::Model,
            match typed {
                Typed::Number(value) => Edit::SetParam {
                    block,
                    param,
                    value,
                },
                Typed::Formula(formula) => Edit::SetFormula {
                    block,
                    param,
                    formula,
                },
            },
        )),
        Err(e) => *error = Some(e),
    }
}

/// A compact display of a value: up to 6 significant digits.
pub(super) fn format_value(v: f64) -> String {
    simsandbox_session::diagram_paint::format_number(v)
}
