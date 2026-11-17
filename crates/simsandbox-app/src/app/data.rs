//! The Data window's contents (ADR 0030, ADR 0032): derived expressions
//! and cut filters through 2D grids, edited on a working copy.

use super::*;

/// The Data window's contents: expressions, then cuts.
pub(super) fn data_editor(ui: &mut egui::Ui, ed: &mut SignalsEditor, history: &History, m: &Model) {
    ui.heading("Signals");
    ui.label(
        "Each signal is computed from plotted signals (a.x, lorenz.xyz[0]), other signals \
         here, parameters (a.k) and t, using the same operators and functions as equations.",
    );
    let mut remove = None;
    // Fixed sizes: inside a grid the text boxes collapsed to a few
    // characters.
    let height = ui.spacing().interact_size.y;
    for (k, (name, text)) in ed.exprs.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add_sized([120.0, height], egui::TextEdit::singleline(name));
            ui.label("=");
            let width = (ui.available_width() - 30.0).max(200.0);
            ui.add_sized(
                [width, height],
                egui::TextEdit::singleline(text).code_editor(),
            );
            if ui.small_button("×").on_hover_text("Remove").clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        ed.exprs.remove(k);
    }
    if ui.button("Add signal").clicked() {
        ed.exprs
            .push((format!("signal{}", ed.exprs.len() + 1), String::new()));
    }
    ui.weak(format!("Signals: {}", m.scalars.join(", ")));

    let inputs = &m.cut_inputs;
    ui.add_space(8.0);
    ui.heading("Cuts");
    if inputs.is_empty() {
        ui.weak("Cuts take a 2D grid, such as the results of a study over two parameters; this model has none.");
        return;
    }
    ui.label(
        "Each cut filter cuts a 2D grid into lines, one per cut, which profiles and plots can \
         show. Cuts run along x at fixed y values, along y at fixed x values, or along free lines.",
    );
    let mut remove = None;
    for (k, c) in ed.cuts.iter_mut().enumerate() {
        if cut_form(ui, k, c, history, inputs, height) {
            remove = Some(k);
        }
    }
    if let Some(k) = remove {
        ed.cuts.remove(k);
    }
    ui.separator();
    if ui
        .button("Add cut filter")
        .on_hover_text("Cut a 2D grid into lines")
        .clicked()
    {
        let mut n = ed.cuts.len() + 1;
        while ed.cuts.iter().any(|c| c.name == format!("cuts{n}")) {
            n += 1;
        }
        let entry =
            simsandbox_session::session::default_cut(history, &format!("cuts{n}"), &inputs[0]);
        let object = serde_json::Value::Object(entry.settings.into_iter().collect());
        if let Ok(settings) = serde_json::from_value(object) {
            ed.cuts.push(CutEdit {
                name: format!("cuts{n}"),
                input: inputs[0].clone(),
                settings,
            });
        }
    }
}

/// One cut filter: its name, the grid it cuts, how, and between cells;
/// then where its lines run. Returns whether it is to be removed.
fn cut_form(
    ui: &mut egui::Ui,
    k: usize,
    c: &mut CutEdit,
    history: &History,
    inputs: &[String],
    height: f32,
) -> bool {
    use simsandbox_views::cut::{Interpolation, Mode};
    let mut removed = false;
    let g = simsandbox_session::session::cut_geometry(history, &c.input);
    ui.separator();
    ui.horizontal(|ui| {
        ui.add_sized([100.0, height], egui::TextEdit::singleline(&mut c.name));
        ui.label("cuts");
        egui::ComboBox::from_id_salt(("cut_input", k))
            .selected_text(&c.input)
            .show_ui(ui, |ui| {
                for i in inputs {
                    ui.selectable_value(&mut c.input, i.clone(), i);
                }
            });
        if ui
            .small_button("×")
            .on_hover_text("Remove this filter")
            .clicked()
        {
            removed = true;
        }
    });
    let s = &mut c.settings;
    ui.horizontal(|ui| {
        let mode_name = |m: Mode| match m {
            Mode::AlongX => format!("along {} at fixed {}", g.label(0), g.label(1)),
            Mode::AlongY => format!("along {} at fixed {}", g.label(1), g.label(0)),
            Mode::Free => "along free lines".to_string(),
        };
        let before = s.mode;
        egui::ComboBox::from_id_salt(("cut_mode", k))
            .selected_text(mode_name(s.mode))
            .show_ui(ui, |ui| {
                for m in [Mode::AlongX, Mode::AlongY, Mode::Free] {
                    ui.selectable_value(&mut s.mode, m, mode_name(m));
                }
            });
        if s.mode != before {
            // Start the new mode with one cut through the middle.
            let mid = |axis: usize| {
                let c = &g.centres[axis];
                c[c.len() / 2]
            };
            let first = |axis: usize| g.centres[axis][0];
            let last = |axis: usize| g.centres[axis][g.centres[axis].len() - 1];
            match s.mode {
                Mode::AlongX => s.at = vec![mid(1)],
                Mode::AlongY => s.at = vec![mid(0)],
                Mode::Free => s.lines = vec![[[first(0), first(1)], [last(0), last(1)]]],
            }
            s.range = None;
        }
        ui.label("between cells");
        ui.selectable_value(&mut s.interpolation, Interpolation::Bilinear, "bilinear")
            .on_hover_text("Interpolate between the four nearest cells");
        ui.selectable_value(&mut s.interpolation, Interpolation::Nearest, "nearest")
            .on_hover_text("Take the nearest cell");
    });
    cut_lines(ui, s, &g);
    removed
}

/// Where a cut filter's lines run: fixed values along an axis with the
/// range they cover, or free lines.
fn cut_lines(
    ui: &mut egui::Ui,
    s: &mut simsandbox_views::cut::CutSettings,
    g: &simsandbox_views::cut::Geometry,
) {
    use simsandbox_views::cut::Mode;
    match s.mode {
        Mode::AlongX | Mode::AlongY => {
            let (free, fixed) = if s.mode == Mode::AlongX {
                (0, 1)
            } else {
                (1, 0)
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("at {} =", g.label(fixed)));
                let mut gone = None;
                for (i, v) in s.at.iter_mut().enumerate() {
                    ui.add(egui::DragValue::new(v).speed(0.01));
                    if ui.small_button("×").clicked() {
                        gone = Some(i);
                    }
                }
                if let Some(i) = gone {
                    s.at.remove(i);
                }
                if ui.small_button("+").on_hover_text("Add a cut").clicked() {
                    let c = &g.centres[fixed];
                    s.at.push(c[c.len() / 2]);
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .checkbox(&mut s.whole, format!("whole {} range", g.label(free)))
                    .changed()
                    && !s.whole
                    && s.range.is_none()
                {
                    let c = &g.centres[free];
                    let (a, b) = (c[0], c[c.len() - 1]);
                    s.range = Some([a.min(b), a.max(b)]);
                }
                if let (false, Some(r)) = (s.whole, s.range.as_mut()) {
                    ui.label("from");
                    ui.add(egui::DragValue::new(&mut r[0]).speed(0.01));
                    ui.label("to");
                    ui.add(egui::DragValue::new(&mut r[1]).speed(0.01));
                }
            });
        }
        Mode::Free => {
            let mut gone = None;
            for (i, [a, b]) in s.lines.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("from ({}, {})", g.label(0), g.label(1)));
                    ui.add(egui::DragValue::new(&mut a[0]).speed(0.01));
                    ui.add(egui::DragValue::new(&mut a[1]).speed(0.01));
                    ui.label("to");
                    ui.add(egui::DragValue::new(&mut b[0]).speed(0.01));
                    ui.add(egui::DragValue::new(&mut b[1]).speed(0.01));
                    if ui.small_button("×").clicked() {
                        gone = Some(i);
                    }
                });
            }
            if let Some(i) = gone {
                s.lines.remove(i);
            }
            ui.horizontal(|ui| {
                if ui.small_button("+").on_hover_text("Add a line").clicked() {
                    let last = s.lines.last().copied().unwrap_or([[0.0, 0.0], [1.0, 1.0]]);
                    s.lines.push(last);
                }
                ui.checkbox(&mut s.whole, "extend each line to the edges of the grid");
            });
        }
    }
}
