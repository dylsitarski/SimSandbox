//! Forms of particular blocks in the inspector, each returning the edits it
//! makes: a study's design, parameters and results (ADR 0070) and its
//! Search… (ADR 0078), a file reader (ADR 0063), a conversion's units (ADR
//! 0045) and when a used model acts (ADR 0088).

use super::*;

/// A text field for one setting of block `b`; a changed value is an edit
/// (empty clears the setting).
pub(super) fn setting_field(
    ui: &mut egui::Ui,
    b: &simsandbox_file::BlockEntry,
    key: &str,
    width: f32,
    hint: &str,
    edits: &mut HashMap<String, String>,
) -> Option<Edit> {
    let now = b.settings.get(key).cloned().unwrap_or_default();
    let id = format!("setting:{}:{key}", b.name);
    let mut text = edits.get(&id).cloned().unwrap_or_else(|| now.clone());
    let r = ui
        .add(egui::TextEdit::singleline(&mut text).desired_width(width))
        .on_hover_text(hint);
    if r.has_focus() {
        edits.insert(id.clone(), text.clone());
    }
    if r.lost_focus() {
        edits.remove(&id);
        if text.trim() != now {
            return Some(Edit::SetSetting {
                block: b.name.clone(),
                key: key.into(),
                text: text.trim().into(),
            });
        }
    }
    None
}

/// A study's form (ADR 0070): what it studies and how many runs, its
/// design, a row per parameter it varies (picked from `keys`, the studied
/// model's parameters with ranges), count and seed, replicates and seeds,
/// its named results, and the whole sample as JSON. Changes are kept as a
/// draft while typed or dragged and applied when a field is left, a
/// choice made or a button pressed. Nothing for other blocks.
/// When a used model acts (ADR 0088): always, or on its `activate` input
/// in a mode; and whether a `restart` input starts it again.
pub(super) fn use_activation(ui: &mut egui::Ui, b: &simsandbox_file::BlockEntry) -> Vec<Edit> {
    let mut out = Vec::new();
    ui.horizontal(|ui| {
        let now = b.activate.clone();
        let label = |m: &Option<String>| match m.as_deref() {
            None => "always".to_string(),
            Some("while") => "while 'activate' is high".to_string(),
            Some(m) => format!("starting again on 'activate' ({m})"),
        };
        ui.label("Runs");
        egui::ComboBox::from_id_salt(("use-activate", &b.name))
            .selected_text(label(&now))
            .show_ui(ui, |ui| {
                for m in [
                    None,
                    Some("rise"),
                    Some("fall"),
                    Some("change"),
                    Some("while"),
                ] {
                    let m = m.map(String::from);
                    if ui.selectable_label(now == m, label(&m)).clicked() && now != m {
                        out.push(Edit::SetActivate {
                            block: b.name.clone(),
                            mode: m,
                        });
                    }
                }
            })
            .response
            .on_hover_text(
                "Activated, it gets an 'activate' input: it starts again from its starting \
                 values when the input rises, falls or changes, or runs only while it is high \
                 and holds otherwise",
            );
        let mut restart = b.restart;
        if ui
            .checkbox(&mut restart, "restart input")
            .on_hover_text(
                "A 'restart' input: it starts again from its starting values when the input rises",
            )
            .changed()
        {
            out.push(Edit::SetRestart {
                block: b.name.clone(),
                on: restart,
            });
        }
    });
    out
}

/// A study's form (ADR 0070): when it runs, its design, what it varies,
/// its results, Search… and the sample as JSON. Typed changes wait in
/// `edits` until applied.
pub(super) fn study_fields(
    ui: &mut egui::Ui,
    b: &simsandbox_file::BlockEntry,
    edits: &mut HashMap<String, String>,
    keys: &[StudyKey],
) -> Vec<Edit> {
    use simsandbox_file::Sampling;
    let mut out = Vec::new();
    if b.type_name != simsandbox_file::STUDY {
        return out;
    }
    let saved = b.sample.clone().unwrap_or_default();
    let id = format!("study:{}", b.name);
    let mut s: Sampling = edits
        .get(&id)
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or_else(|| saved.clone());
    // When it runs (ADR 0088).
    study_activation(ui, b, &mut out);
    let mut t = Touched::default();
    study_design(ui, b, &mut s, &mut t);
    // What it varies.
    study_params(ui, b, &mut s, keys, &mut t);
    // Its named results.
    study_results(ui, &mut s, &mut t);
    if t.changed {
        edits.insert(id.clone(), serde_json::to_string(&s).unwrap_or_default());
    }
    if t.commit {
        edits.remove(&id);
        if s != saved {
            out.push(Edit::SetSample {
                block: b.name.clone(),
                sample: serde_json::to_string(&s).unwrap_or_default(),
            });
        }
    }
    out.extend(search_fields(ui, b, &s, edits, keys));
    // Everything, as written in the file.
    sample_json(ui, b, &saved, edits, &mut out);
    out
}

/// What a part of a study's form did this frame: changed the sample,
/// and whether to apply it now (a choice made, a field left).
#[derive(Default)]
struct Touched {
    changed: bool,
    commit: bool,
}

/// When a study runs (ADR 0088): on a change of its inputs, or on its
/// 'activate' input.
fn study_activation(ui: &mut egui::Ui, b: &simsandbox_file::BlockEntry, out: &mut Vec<Edit>) {
    ui.horizontal(|ui| {
        let now = b.activate.clone();
        let label = |m: &Option<String>| match m.as_deref() {
            None => "when its inputs change".to_string(),
            Some("while") => "while 'activate' is high".to_string(),
            Some(m) => format!("on 'activate' ({m})"),
        };
        ui.label("Runs");
        egui::ComboBox::from_id_salt(("activate", &b.name))
            .selected_text(label(&now))
            .show_ui(ui, |ui| {
                for m in [
                    None,
                    Some("rise"),
                    Some("fall"),
                    Some("change"),
                    Some("while"),
                ] {
                    let m = m.map(String::from);
                    if ui.selectable_label(now == m, label(&m)).clicked() && now != m {
                        out.push(Edit::SetActivate {
                            block: b.name.clone(),
                            mode: m,
                        });
                    }
                }
            })
            .response
            .on_hover_text(
                "Activated, a study gets an 'activate' input: it runs once when it rises, falls \
                 or changes, or keeps running while it is high, and holds its results otherwise",
            );
    });
}

/// How many runs the study makes and of what, then its design: how it
/// picks points, how many, seeds and replicates.
fn study_design(
    ui: &mut egui::Ui,
    b: &simsandbox_file::BlockEntry,
    s: &mut simsandbox_file::Sampling,
    t: &mut Touched,
) {
    use simsandbox_file::{Design, Seeds};
    let runs = match s.design {
        Design::Rows => "a run per row".to_string(),
        _ => format!("{} runs", study_runs(s)),
    };
    ui.weak(match (&b.model, &b.file) {
        (Some(m), _) => {
            format!("Studies the model {m}: {runs} (open the node to edit the model)")
        }
        (None, Some(f)) => {
            format!("Studies the file {f}: {runs} (open the node to edit the model)")
        }
        _ => "Names no model to study".into(),
    });
    egui::Grid::new(("study-form", &b.name))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("design");
            let designs = [
                (Design::Grid, "grid: every combination"),
                (Design::List, "list of points"),
                (Design::Random, "random"),
                (Design::LatinHypercube, "Latin hypercube"),
                (Design::Sobol, "Sobol sequence"),
                (Design::File, "points from a file"),
                (Design::Inputs, "points from its inputs"),
                (Design::Rows, "a point per row of a table"),
            ];
            let label = designs.iter().find(|(d, _)| *d == s.design).map_or("", |(_, l)| l);
            egui::ComboBox::from_id_salt(("study-design", &b.name))
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (d, l) in designs {
                        if ui.selectable_value(&mut s.design, d, l).clicked() {
                            // Points from inputs need their number; a
                            // table's rows are as many as come.
                            if d == Design::Inputs && s.count.is_none() {
                                s.count = Some(simsandbox_file::DEFAULT_SAMPLE_COUNT);
                            }
                            if d == Design::Rows {
                                s.count = None;
                                s.replicates = None;
                            }
                            // ... and have no ranges: their values arrive.
                            if d.from_inputs() {
                                for p in &mut s.params {
                                    p.range = None;
                                    p.points = None;
                                }
                            }
                            t.changed = true;
                            t.commit = true;
                        }
                    }
                });
            ui.end_row();
            if matches!(
                s.design,
                Design::Random | Design::LatinHypercube | Design::Sobol | Design::Inputs
            ) {
                ui.label("points");
                let mut n = s.count.unwrap_or(simsandbox_file::DEFAULT_SAMPLE_COUNT);
                let r = ui.add(egui::DragValue::new(&mut n).range(1..=simsandbox_file::MAX_SAMPLES));
                if r.changed() {
                    s.count = Some(n);
                    t.changed = true;
                }
                t.commit |= r.drag_stopped() || r.lost_focus();
                ui.end_row();
            }
            if matches!(s.design, Design::Random | Design::LatinHypercube) {
                ui.label("seed");
                let mut seed = s.seed.unwrap_or(simsandbox_file::DEFAULT_SEED);
                let r = ui.add(egui::DragValue::new(&mut seed));
                if r.changed() {
                    s.seed = Some(seed);
                    t.changed = true;
                }
                t.commit |= r.drag_stopped() || r.lost_focus();
                ui.end_row();
            }
            if s.design == Design::File {
                ui.label("file");
                let mut f = s.file.clone().unwrap_or_default();
                let r = ui
                    .add(egui::TextEdit::singleline(&mut f).hint_text("points.csv"))
                    .on_hover_text("A CSV from the model's folder: a column per parameter, by path; an id column optional");
                if r.changed() {
                    s.file = (!f.trim().is_empty()).then_some(f);
                    t.changed = true;
                }
                t.commit |= r.lost_focus();
                ui.end_row();
            }
            // A table's rows run once each (ADR 0079 stage 1d).
            if s.design != Design::Rows {
                ui.label("replicates");
                let mut reps = s.replicates.unwrap_or(1);
                let r = ui
                    .add(egui::DragValue::new(&mut reps).range(1..=10_000))
                    .on_hover_text("Runs of each point, each replicate with a seed of its own");
                if r.changed() {
                    s.replicates = (reps > 1).then_some(reps);
                    t.changed = true;
                }
                t.commit |= r.drag_stopped() || r.lost_focus();
                ui.end_row();
            }
            ui.label("seeds");
            egui::ComboBox::from_id_salt(("study-seeds", &b.name))
                .selected_text(match s.seeds {
                    Seeds::Common => "common to every point",
                    Seeds::PerPoint => "one per point",
                })
                .show_ui(ui, |ui| {
                    for (v, l) in [
                        (Seeds::Common, "common to every point"),
                        (Seeds::PerPoint, "one per point"),
                    ] {
                        if ui.selectable_value(&mut s.seeds, v, l).clicked() {
                            t.changed = true;
                            t.commit = true;
                        }
                    }
                });
            ui.end_row();
        });
}

/// The parameters a study varies, each with its range (and points on a
/// grid), or the note saying where they come from.
fn study_params(
    ui: &mut egui::Ui,
    b: &simsandbox_file::BlockEntry,
    s: &mut simsandbox_file::Sampling,
    keys: &[StudyKey],
    t: &mut Touched,
) {
    use simsandbox_file::{Design, SweepControl};
    let range_of = |key: &str| {
        keys.iter()
            .find(|(k, _, _)| k == key)
            .map_or([0.0, 1.0], |(_, r, _)| *r)
    };
    if s.design == Design::File {
        ui.weak("The file's columns are the parameters it varies.");
    } else if s.design == Design::List {
        ui.weak(format!(
            "{} parameters, {} points: edit the list as JSON below",
            s.params.len(),
            s.values.len()
        ));
    } else {
        // Points from inputs have no range: another model sends the values
        // (ADR 0078).
        let ranged = !s.design.from_inputs();
        ui.strong("varies");
        if s.design == Design::Rows {
            ui.weak("Each parameter's values arrive as a table's rows through the study's input of the same name; each row is a point, and the results leave as rows with the same ids.");
        } else if !ranged {
            ui.weak("Each parameter's values arrive through the study's input of the same name, one per point.");
        }
        let mut remove = None;
        egui::Grid::new(("study-params", &b.name))
            .num_columns(5)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                if !s.params.is_empty() && ranged {
                    for h in ["parameter", "from", "to"] {
                        ui.weak(h);
                    }
                    ui.weak(if s.design == Design::Grid { "points" } else { "" });
                    ui.end_row();
                }
                for (k, p) in s.params.iter_mut().enumerate() {
                    egui::ComboBox::from_id_salt(("study-param", &b.name, k))
                        .selected_text(&p.param)
                        .show_ui(ui, |ui| {
                            for (key, range, ic) in keys {
                                let l = if *ic { format!("{key} (start)") } else { key.clone() };
                                if ui.selectable_label(*key == p.param, l).clicked() && *key != p.param {
                                    p.param = key.clone();
                                    p.range = ranged.then_some(*range);
                                    t.changed = true;
                                    t.commit = true;
                                }
                            }
                        });
                    if !ranged {
                        if ui.small_button("×").on_hover_text("Stop varying it").clicked() {
                            remove = Some(k);
                        }
                        ui.end_row();
                        continue;
                    }
                    let [mut lo, mut hi] = p.range.unwrap_or_else(|| range_of(&p.param));
                    let speed = ((hi - lo).abs() / 200.0).max(1e-6);
                    let a = ui.add(egui::DragValue::new(&mut lo).speed(speed)).on_hover_text("From");
                    let z = ui.add(egui::DragValue::new(&mut hi).speed(speed)).on_hover_text("To");
                    if a.changed() || z.changed() {
                        p.range = Some([lo, hi]);
                        t.changed = true;
                    }
                    t.commit |= a.drag_stopped() || a.lost_focus() || z.drag_stopped() || z.lost_focus();
                    if s.design == Design::Grid {
                        let mut n = p.points.unwrap_or(0);
                        let r = ui
                            .add(egui::DragValue::new(&mut n).range(0..=100_000).custom_formatter(|v, _| {
                                if v == 0.0 { "auto".into() } else { format!("{v:.0}") }
                            }))
                            .on_hover_text("Values along this parameter (auto: 400 for one parameter, 100 each for several)");
                        if r.changed() {
                            p.points = (n >= 2).then_some(n);
                            t.changed = true;
                        }
                        t.commit |= r.drag_stopped() || r.lost_focus();
                    } else {
                        ui.label("");
                    }
                    if ui.small_button("×").on_hover_text("Stop varying it").clicked() {
                        remove = Some(k);
                    }
                    ui.end_row();
                }
            });
        if let Some(k) = remove {
            s.params.remove(k);
            t.changed = true;
            t.commit = true;
        }
        let unused = keys
            .iter()
            .find(|(k, _, _)| !s.params.iter().any(|p| &p.param == k));
        if let Some((key, range, _)) = unused {
            if ui
                .small_button("+ parameter")
                .on_hover_text("Vary another parameter of the studied model")
                .clicked()
            {
                s.params.push(SweepControl {
                    param: key.clone(),
                    range: ranged.then_some(*range),
                    points: None,
                });
                t.changed = true;
                t.commit = true;
            }
        }
    }
}

/// The study's named results: each run reduced to values.
fn study_results(ui: &mut egui::Ui, s: &mut simsandbox_file::Sampling, t: &mut Touched) {
    ui.strong("results").on_hover_text(
        "Each run reduced to named values, e.g. max(car.v) or first_time(abs(cruise.e) < 0.5); \
         without any, each output's final value",
    );
    let mut renamed: Option<(String, String)> = None;
    let mut dropped = None;
    for (name, formula) in s.reduce.iter_mut() {
        ui.horizontal(|ui| {
            let mut n = name.clone();
            let r = ui.add_sized([70.0, 20.0], egui::TextEdit::singleline(&mut n));
            if r.changed() {
                renamed = Some((name.clone(), n));
            }
            t.commit |= r.lost_focus();
            ui.label("=");
            let width = (ui.available_width() - 30.0).max(120.0);
            let r = ui.add_sized([width, 20.0], egui::TextEdit::singleline(formula));
            if r.changed() {
                t.changed = true;
            }
            t.commit |= r.lost_focus();
            if ui
                .small_button("×")
                .on_hover_text("Drop this result")
                .clicked()
            {
                dropped = Some(name.clone());
            }
        });
    }
    if let Some((from, to)) = renamed {
        if let Some(f) = s.reduce.remove(&from) {
            s.reduce.insert(to, f);
            t.changed = true;
        }
    }
    if let Some(n) = dropped {
        s.reduce.remove(&n);
        t.changed = true;
        t.commit = true;
    }
    if ui
        .small_button("+ result")
        .on_hover_text("Reduce each run to one more named value")
        .clicked()
    {
        let mut k = s.reduce.len() + 1;
        while s.reduce.contains_key(&format!("r{k}")) {
            k += 1;
        }
        s.reduce.insert(format!("r{k}"), "final(t)".into());
        t.changed = true;
        t.commit = true;
    }
}

/// The whole sample as written in the file, edited as JSON.
fn sample_json(
    ui: &mut egui::Ui,
    b: &simsandbox_file::BlockEntry,
    saved: &simsandbox_file::Sampling,
    edits: &mut HashMap<String, String>,
    out: &mut Vec<Edit>,
) {
    egui::CollapsingHeader::new("sample as JSON")
        .id_salt(("study-json", &b.name))
        .show(ui, |ui| {
            let now = serde_json::to_string_pretty(saved).unwrap_or_default();
            let jid = format!("sample:{}", b.name);
            let mut text = edits.get(&jid).cloned().unwrap_or_else(|| now.clone());
            let r = ui.add(
                egui::TextEdit::multiline(&mut text)
                    .code_editor()
                    .desired_rows(6)
                    .desired_width(f32::INFINITY),
            );
            if r.has_focus() {
                edits.insert(jid.clone(), text.clone());
            }
            if r.lost_focus() {
                edits.remove(&jid);
                if text.trim() != now.trim() {
                    out.push(Edit::SetSample {
                        block: b.name.clone(),
                        sample: text,
                    });
                }
            }
        });
}

/// "Search…" (ADR 0078 point 7): a strategy model written to drive this
/// study over its parameters (their ranges as it has them): pattern
/// search, Levenberg–Marquardt, refinement or continuation, with the one
/// formula each needs. The study's sample is replaced and a strategy
/// block wired to it, which can be opened and changed like any other.
pub(super) fn search_fields(
    ui: &mut egui::Ui,
    b: &simsandbox_file::BlockEntry,
    s: &simsandbox_file::Sampling,
    edits: &mut HashMap<String, String>,
    keys: &[StudyKey],
) -> Option<Edit> {
    let mut out = None;
    egui::CollapsingHeader::new("Search…")
        .id_salt(("study-search", &b.name))
        .show(ui, |ui| {
            let field = |edits: &HashMap<String, String>, f: &str, default: &str| {
                edits
                    .get(&format!("search-{f}:{}", b.name))
                    .cloned()
                    .unwrap_or_else(|| default.to_string())
            };
            let kinds = [
                ("minimize", "lower an objective (pattern search)"),
                ("fit", "fit to data (Levenberg–Marquardt)"),
                ("refine", "find where a result changes (refinement)"),
                ("continue", "follow a branch (continuation)"),
            ];
            let mut kind = field(edits, "kind", "minimize");
            egui::ComboBox::from_id_salt(("search-kind", &b.name))
                .selected_text(kinds.iter().find(|k| k.0 == kind).map_or("", |k| k.1))
                .show_ui(ui, |ui| {
                    for (k, l) in kinds {
                        ui.selectable_value(&mut kind, k.to_string(), l);
                    }
                });
            edits.insert(format!("search-kind:{}", b.name), kind.clone());
            let first = s.reduce.values().next().cloned().unwrap_or_default();
            let (label, hint) = match kind.as_str() {
                "minimize" => ("objective", "lower is better, e.g. first_time(abs(cruise.e) < 0.5)"),
                "fit" => ("residuals", "a vector per run, e.g. at_samples(osc.x - data.x, data)"),
                "refine" => ("result", "split where it changes, e.g. final(bi.x) > 0"),
                _ => ("settled", "the state a run settles at, e.g. final(bi.x)"),
            };
            let mut formula = field(edits, "formula", &first);
            ui.horizontal(|ui| {
                ui.label(label);
                ui.add(egui::TextEdit::singleline(&mut formula).hint_text(hint));
            });
            edits.insert(format!("search-formula:{}", b.name), formula.clone());
            // Each swept parameter with its range: the study's, or the
            // model's suggested one.
            let params: Vec<(String, [f64; 2])> = s
                .params
                .iter()
                .map(|p| {
                    let range = p.range.or_else(|| {
                        keys.iter().find(|k| k.0 == p.param).map(|k| k.1)
                    });
                    (p.param.clone(), range.unwrap_or([0.0, 1.0]))
                })
                .collect();
            let mut state = field(edits, "state", "");
            let mut step = field(edits, "step", "0.02").parse::<f64>().unwrap_or(0.02);
            if kind == "continue" {
                ui.horizontal(|ui| {
                    ui.label("starting value");
                    egui::ComboBox::from_id_salt(("search-state", &b.name))
                        .selected_text(state.clone())
                        .show_ui(ui, |ui| {
                            for k in keys.iter().filter(|k| k.2) {
                                ui.selectable_value(&mut state, k.0.clone(), &k.0);
                            }
                        });
                    ui.label("step");
                    ui.add(egui::DragValue::new(&mut step).speed(0.001));
                });
                edits.insert(format!("search-state:{}", b.name), state.clone());
                edits.insert(format!("search-step:{}", b.name), step.to_string());
            }
            let mut budget = field(edits, "budget", "60").parse::<f64>().unwrap_or(60.0);
            ui.horizontal(|ui| {
                ui.label("rounds at most");
                ui.add(egui::DragValue::new(&mut budget).range(1.0..=100_000.0));
            });
            edits.insert(format!("search-budget:{}", b.name), budget.to_string());
            ui.weak(match kind.as_str() {
                "refine" => format!(
                    "Over {}, each across its range: refined where the result changes.",
                    params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", ")
                ),
                "continue" => format!(
                    "Over {} from {} to {}.",
                    params.first().map_or("(no parameter)", |p| p.0.as_str()),
                    params.first().map_or(0.0, |p| p.1[0]),
                    params.first().map_or(0.0, |p| p.1[1])
                ),
                _ => format!(
                    "Over {}, starting at the middle of each range.",
                    params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", ")
                ),
            });
            let mid = |r: [f64; 2]| 0.5 * (r[0] + r[1]);
            let spec = match kind.as_str() {
                "minimize" => serde_json::json!({"kind": "minimize", "objective": formula, "budget": budget,
                    "params": params.iter().map(|(k, r)| serde_json::json!(
                        {"param": k, "start": mid(*r), "span": r[1] - r[0], "range": r})).collect::<Vec<_>>()}),
                "fit" => serde_json::json!({"kind": "fit", "residuals": formula, "budget": budget,
                    "params": params.iter().map(|(k, r)| serde_json::json!(
                        {"param": k, "start": mid(*r)})).collect::<Vec<_>>()}),
                // A mesh of simplices over the parameters, segments over
                // one (ADR 0085 point 7).
                "refine" => serde_json::json!({"kind": "refine", "result": formula,
                    "budget": budget, "points": if params.len() > 1 { 5 } else { 9 },
                    "max": if params.len() > 1 { 2000 } else { 200 },
                    "params": params.iter().map(|(k, r)| serde_json::json!(
                        {"param": k, "range": r, "tol": (r[1] - r[0]) * 1e-3})).collect::<Vec<_>>()}),
                _ => serde_json::json!({"kind": "continue", "settled": formula, "budget": budget,
                    "state": state, "step": step,
                    "param": params.first().map(|p| p.0.clone()).unwrap_or_default(),
                    "range": params.first().map_or([0.0, 1.0], |p| p.1)}),
            };
            if ui
                .button("Write strategy")
                .on_hover_text("Replace this study's sample and add a strategy block that drives it, one round a step")
                .clicked()
            {
                out = Some(Edit::Search {
                    study: b.name.clone(),
                    strategy: spec.to_string(),
                });
            }
        });
    out
}

/// How many runs a study's sample makes: points times replicates.
pub(super) fn study_runs(s: &simsandbox_file::Sampling) -> usize {
    use simsandbox_file::Design;
    let points = match s.design {
        Design::Grid => {
            let auto = if s.params.len() == 1 {
                simsandbox_file::DEFAULT_SWEEP_POINTS
            } else {
                simsandbox_file::DEFAULT_GRID_POINTS
            };
            s.params
                .iter()
                .map(|p| p.points.unwrap_or(auto))
                .product::<usize>()
                * usize::from(!s.params.is_empty())
        }
        Design::List => s.values.len(),
        Design::Random | Design::LatinHypercube | Design::Sobol | Design::Inputs => {
            s.count.unwrap_or(simsandbox_file::DEFAULT_SAMPLE_COUNT)
        }
        // As many as arrive.
        Design::File | Design::Rows => 0,
    };
    points * s.replicates.unwrap_or(1)
}

/// A file reader's settings (ADR 0063): the file, typed or chosen in the
/// system dialog (relative to the model's `folder` when inside it), how
/// values go between samples, the unit, and for a `.npy` its times and
/// shape. Nothing for other blocks.
pub(super) fn file_fields(
    ui: &mut egui::Ui,
    b: &simsandbox_file::BlockEntry,
    edits: &mut HashMap<String, String>,
    folder: Option<&Path>,
) -> Vec<Edit> {
    let mut out = Vec::new();
    if b.type_name != simsandbox_eq::file_input::FILE {
        return out;
    }
    let set = |key: &str, text: String| Edit::SetSetting {
        block: b.name.clone(),
        key: key.into(),
        text,
    };
    let relative = |path: std::path::PathBuf| {
        folder
            .and_then(|f| path.strip_prefix(f).ok())
            .map_or(path.clone(), Path::to_path_buf)
            .to_string_lossy()
            .replace('\\', "/")
    };
    ui.horizontal(|ui| {
        ui.label("reads");
        let hint = if folder.is_some() {
            "A .csv (time, then a column per output) or .npy, in the model's folder or a full path"
        } else {
            "A .csv or .npy: save the model first to name it from the model's folder, or give a full path"
        };
        out.extend(setting_field(ui, b, "file", 160.0, hint, edits));
        if ui.small_button("Choose…").clicked() {
            let mut dialog = rfd::FileDialog::new()
                .set_title("Data to read")
                .add_filter("Data", &["csv", "npy"]);
            if let Some(dir) = folder {
                dialog = dialog.set_directory(dir);
            }
            if let Some(path) = dialog.pick_file() {
                out.push(set("file", relative(path)));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("between samples");
        let now = b.settings.get("interpolate").map_or("hold", String::as_str);
        for (choice, hint) in [
            (
                "hold",
                "The last sample, as a probe feeding a block gives it",
            ),
            ("linear", "A straight line between samples"),
        ] {
            if ui
                .selectable_label(now == choice, choice)
                .on_hover_text(hint)
                .clicked()
                && now != choice
            {
                out.push(set("interpolate", choice.into()));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("unit");
        out.extend(setting_field(
            ui,
            b,
            "unit",
            70.0,
            "For columns whose header gives none (\"car.v [m/s]\" does), e.g. m/s",
            edits,
        ));
    });
    if b.settings.get("file").is_some_and(|f| f.ends_with(".npy")) {
        ui.horizontal(|ui| {
            ui.label("times from");
            out.extend(setting_field(
                ui,
                b,
                "times",
                110.0,
                "A CSV whose first column gives the samples' times, e.g. the probe's CSV",
                edits,
            ));
            ui.label("or every");
            out.extend(setting_field(
                ui,
                b,
                "dt",
                50.0,
                "Seconds between samples, from 0",
                edits,
            ));
            ui.label("s");
        });
    }
    out
}

/// A conversion block's `from` and `to` units (ADR 0045) as text fields;
/// a changed one is an edit, which restarts the run. Nothing for other
/// blocks.
pub(super) fn unit_fields(
    ui: &mut egui::Ui,
    b: &simsandbox_file::BlockEntry,
    edits: &mut HashMap<String, String>,
) -> Vec<Edit> {
    let mut out = Vec::new();
    if b.type_name != simsandbox_eq::convert::CONVERT {
        return out;
    }
    ui.horizontal(|ui| {
        ui.label("converts")
            .on_hover_text("Connections need the same unit on both ends; this block converts");
        for (k, key) in ["from", "to"].into_iter().enumerate() {
            if k == 1 {
                ui.label("to");
            }
            let now = b.settings.get(key).cloned().unwrap_or_else(|| "1".into());
            let id = format!("convert:{}:{key}", b.name);
            let mut text = edits.get(&id).cloned().unwrap_or_else(|| now.clone());
            let r = ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(70.0))
                .on_hover_text("A unit, e.g. km/h, m/s, mm, N*s/m; changing it restarts the run");
            if r.has_focus() {
                edits.insert(id.clone(), text.clone());
            }
            if r.lost_focus() {
                edits.remove(&id);
                let typed = text.trim().to_string();
                if !typed.is_empty() && typed != now {
                    out.push(Edit::SetSetting {
                        block: b.name.clone(),
                        key: key.to_string(),
                        text: typed,
                    });
                }
            }
        }
    });
    out
}
