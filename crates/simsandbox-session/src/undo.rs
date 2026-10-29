//! Undo for canvas edits (ADR 0050). The app keeps the model file as it
//! was before each edit; undoing restores the parts the canvas edits
//! (blocks, wires, subsystems, declared parameters, groups, filters, views,
//! probes and their places) while keeping what changed elsewhere since: parameter
//! values, sliders, speed and the run settings. How much changed decides
//! how it applies: a layout alone or filters and views apply to the
//! running model, anything else rebuilds it.

use simsandbox_file::{BlockEntry, ModelFile, ParamValue};

/// What separates two model files, as far as applying one goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Only where nodes sit.
    Layout,
    /// Filters and views (and places), not the model.
    Data,
    /// The model itself: it is rebuilt and the run restarts.
    Model,
}

/// `now` with the canvas-edited parts of `target`, keeping `now`'s
/// parameter values wherever a block (or declared parameter) of the same
/// name and type still has them.
pub fn canvas_state(now: &ModelFile, target: &ModelFile) -> ModelFile {
    let mut out = now.clone();
    out.blocks = carried(&now.blocks, &target.blocks);
    out.connections = target.connections.clone();
    out.subsystems = target.subsystems.clone();
    for (name, def) in &mut out.subsystems {
        if let Some(old) = now.subsystems.get(name) {
            def.blocks = carried(&old.blocks, &def.blocks);
            for (p, d) in &mut def.params {
                if let Some(o) = old.params.get(p) {
                    d.value = o.value;
                }
            }
        }
    }
    out.params = target.params.clone();
    for (p, d) in &mut out.params {
        if let Some(o) = now.params.get(p) {
            d.value = o.value;
        }
    }
    out.groups = target.groups.clone();
    out.controls.filters = target.controls.filters.clone();
    out.controls.views = target.controls.views.clone();
    out.controls.probes = target.controls.probes.clone();
    out
}

/// `target`'s blocks with the numeric parameter values of `now`'s blocks
/// of the same name and type.
fn carried(now: &[BlockEntry], target: &[BlockEntry]) -> Vec<BlockEntry> {
    target
        .iter()
        .map(|b| {
            let mut b = b.clone();
            if let Some(o) = now
                .iter()
                .find(|o| o.name == b.name && o.type_name == b.type_name)
            {
                for (k, v) in &mut b.params {
                    if let (ParamValue::Number(_), Some(ParamValue::Number(x))) =
                        (&*v, o.params.get(k))
                    {
                        *v = ParamValue::Number(*x);
                    }
                }
            }
            b
        })
        .collect()
}

/// A file without places: where nodes sit does not change the model.
fn unplaced(f: &ModelFile) -> ModelFile {
    let mut f = f.clone();
    for b in &mut f.blocks {
        b.canvas = None;
    }
    for def in f.subsystems.values_mut() {
        for b in &mut def.blocks {
            b.canvas = None;
        }
        for p in &mut def.inputs {
            p.canvas = None;
        }
        for p in &mut def.outputs {
            p.canvas = None;
        }
    }
    for x in &mut f.controls.filters {
        x.canvas = None;
    }
    for v in &mut f.controls.views {
        v.canvas = None;
    }
    for p in &mut f.controls.probes {
        p.canvas = None;
    }
    f
}

/// How `next` differs from `now`.
pub fn change(now: &ModelFile, next: &ModelFile) -> Change {
    let (mut a, mut b) = (unplaced(now), unplaced(next));
    if a == b {
        return Change::Layout;
    }
    for f in [&mut a, &mut b] {
        f.controls.filters.clear();
        f.controls.views.clear();
        f.controls.probes.clear();
    }
    if a == b {
        Change::Data
    } else {
        Change::Model
    }
}

/// Copy where every model node sits (blocks and subsystem ports at every
/// level) from `from` into `to`.
pub fn copy_layout(from: &ModelFile, to: &mut ModelFile) {
    let place = |src: &[BlockEntry], dst: &mut [BlockEntry]| {
        for b in dst {
            b.canvas = src.iter().find(|s| s.name == b.name).and_then(|s| s.canvas);
        }
    };
    place(&from.blocks, &mut to.blocks);
    for (name, def) in &mut to.subsystems {
        if let Some(src) = from.subsystems.get(name) {
            place(&src.blocks, &mut def.blocks);
            for p in &mut def.inputs {
                p.canvas = src
                    .inputs
                    .iter()
                    .find(|q| q.name == p.name)
                    .and_then(|q| q.canvas);
            }
            for p in &mut def.outputs {
                p.canvas = src
                    .outputs
                    .iter()
                    .find(|q| q.name == p.name)
                    .and_then(|q| q.canvas);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::{apply, Edit};
    use crate::examples::{registry, EXAMPLES};

    fn cruise() -> ModelFile {
        let e = EXAMPLES.iter().find(|e| e.name.contains("Cruise")).unwrap();
        ModelFile::from_json(e.text).unwrap().0
    }

    #[test]
    fn undoing_tells_layout_data_and_model_apart_and_keeps_values() {
        let r = registry();
        let before = cruise();
        let edited = |e: Edit| {
            let mut f = before.clone();
            apply(&mut f, &r, &e).unwrap();
            f
        };
        let moved = edited(Edit::Move {
            block: "car".into(),
            at: [5.0, 5.0],
        });
        assert_eq!(
            change(&moved, &canvas_state(&moved, &before)),
            Change::Layout
        );
        let shown = edited(Edit::Bind {
            from: "car.v".into(),
            to: "view:1.y".into(),
            many: true,
        });
        assert_eq!(change(&shown, &canvas_state(&shown, &before)), Change::Data);
        let mut cut = edited(Edit::Disconnect { to: "car.F".into() });
        assert_eq!(change(&cut, &canvas_state(&cut, &before)), Change::Model);
        // A slider moved after the edit keeps its value when undoing.
        cut.blocks[0]
            .params
            .insert("m".into(), ParamValue::Number(1234.0));
        let undone = canvas_state(&cut, &before);
        assert_eq!(undone.connections, before.connections);
        assert_eq!(undone.blocks[0].params["m"], ParamValue::Number(1234.0));
        // Layouts copy level by level.
        let mut back = moved.clone();
        copy_layout(&before, &mut back);
        assert_eq!(back.blocks[0].canvas, before.blocks[0].canvas);
    }
}
