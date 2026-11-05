//! `show.values`: the newest value of each bound signal as a number, and
//! lines of cells (a study's results) as a table, cell by cell. For models
//! whose results are a few numbers, such as a model evaluated, not run
//! (ADR 0072).

use crate::builtin::bindings;
use crate::history::{label, SignalRef};
use crate::view::{always, Preset, Slot, SlotKind, View, ViewInput, ViewRegistry, ViewType};

/// At most this many rows of a line of cells are listed.
const MAX_ROWS: usize = 2_000;

pub fn register(r: &mut ViewRegistry) -> Result<(), String> {
    r.register(ViewType {
        name: "show.values",
        label: "Values",
        description: "The newest value of each signal as a number; lines of cells (a study's results) as a table",
        slots: &[Slot {
            name: "values",
            kind: SlotKind::Values,
            of: None,
            applies: always,
            many: true,
            required: true,
            description: "Signals shown as numbers, or lines of cells shown as a table",
        }],
        style: &[],
        applies_to: &[],
        swap: None,
        presets: &[Preset {
            label: "Values",
            description: "Every signal's newest value, as a number",
            bindings: |a| {
                let s = a.scalars;
                (!s.is_empty() && s.len() <= 12).then(|| {
                    let mut b = bindings(&[]);
                    b.insert("values".into(), s.to_vec());
                    b
                })
            },
        }],
        create: || Box::new(Values),
    })
}

/// A number in at most 6 significant digits, without trailing zeros.
pub fn number(v: f64) -> String {
    if v.is_nan() {
        return "–".into();
    }
    if v.is_infinite() {
        return if v > 0.0 {
            "∞".into()
        } else {
            "−∞".into()
        };
    }
    let a = v.abs();
    if a != 0.0 && !(1e-3..1e6).contains(&a) {
        let s = format!("{v:.5e}");
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let m = m.trim_end_matches('0').trim_end_matches('.');
        return format!("{m}e{e}");
    }
    let digits = if a == 0.0 {
        0
    } else {
        (5 - a.log10().floor() as i32).clamp(0, 8) as usize
    };
    let s = format!("{v:.digits$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

struct Values;

impl View for Values {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let bound = input.many_any("values");
        let newest = |h: &crate::History, f: usize| -> Vec<f64> {
            if h.is_empty() {
                Vec::new()
            } else {
                h.fields()[f].frame(h.len() - 1).to_vec()
            }
        };
        let mut lines: Vec<(String, Vec<f64>)> = Vec::new();
        let unit = |b: &crate::view::Bound<'_>| b.history.unit(b.name).unwrap_or("").to_string();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("values")
                    .num_columns(3)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        for b in bound {
                            let v = match b.signal {
                                SignalRef::Field(f) => {
                                    let cells = newest(b.history, f);
                                    if cells.len() != 1 {
                                        let u = unit(b);
                                        let name = if u.is_empty() {
                                            label(b.name)
                                        } else {
                                            format!("{} [{u}]", label(b.name))
                                        };
                                        lines.push((name, cells));
                                        continue;
                                    }
                                    cells[0]
                                }
                                s => b.history.values(s).last().copied().unwrap_or(f64::NAN),
                            };
                            ui.label(label(b.name));
                            ui.label(egui::RichText::new(number(v)).monospace().size(20.0));
                            ui.label(unit(b));
                            ui.end_row();
                        }
                    });
                if lines.is_empty() {
                    return;
                }
                ui.add_space(8.0);
                let rows = lines.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
                egui::Grid::new("cells")
                    .num_columns(lines.len() + 1)
                    .striped(true)
                    .spacing([16.0, 2.0])
                    .show(ui, |ui| {
                        ui.weak("cell");
                        for (name, _) in &lines {
                            ui.strong(name);
                        }
                        ui.end_row();
                        for k in 0..rows.min(MAX_ROWS) {
                            ui.weak(k.to_string());
                            for (_, v) in &lines {
                                ui.monospace(v.get(k).map_or(String::new(), |x| number(*x)));
                            }
                            ui.end_row();
                        }
                    });
                if rows > MAX_ROWS {
                    ui.weak(format!("… {} more cells", rows - MAX_ROWS));
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::number;

    #[test]
    fn numbers_are_short_and_readable() {
        assert_eq!(number(0.0), "0");
        assert_eq!(number(1.0), "1");
        assert_eq!(number(119.473_684_2), "119.474");
        assert_eq!(number(-0.612_345_67), "-0.612346");
        assert_eq!(number(2.5e-7), "2.5e-7");
        assert_eq!(number(1.234_567e9), "1.23457e9");
        assert_eq!(number(f64::NAN), "–");
    }
}
