//! A field's axes left over by a view (ADR 0081 point 5, ADR 0086): each
//! picked at one place, reduced along, or shown as facets. A view draws
//! the slice over the axes it keeps.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How an axis left over is reduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reduce {
    Mean,
    Minof,
    Maxof,
    Sum,
}

impl Reduce {
    pub const ALL: [Reduce; 4] = [Reduce::Mean, Reduce::Minof, Reduce::Maxof, Reduce::Sum];

    pub fn name(self) -> &'static str {
        match self {
            Reduce::Mean => "mean",
            Reduce::Minof => "minof",
            Reduce::Maxof => "maxof",
            Reduce::Sum => "sum",
        }
    }

    fn apply(self, values: impl Iterator<Item = f64>) -> f64 {
        let (mut n, mut sum, mut lo, mut hi) = (0usize, 0.0, f64::INFINITY, f64::NEG_INFINITY);
        for v in values {
            n += 1;
            sum += v;
            lo = lo.min(v);
            hi = hi.max(v);
        }
        match self {
            Reduce::Mean if n > 0 => sum / n as f64,
            Reduce::Mean => f64::NAN,
            Reduce::Sum => sum,
            Reduce::Minof if n > 0 => lo,
            Reduce::Maxof if n > 0 => hi,
            _ => f64::NAN,
        }
    }
}

/// What a view does with the axes it does not draw, by axis name: picked
/// at a place (the first by default), reduced, or one facet each place.
/// Saved with the view.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Slicing {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pick: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reduce: BTreeMap<String, Reduce>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet: Option<String>,
}

/// The most facets a view shows.
pub const MAX_FACETS: usize = 16;

impl Slicing {
    pub fn is_empty(&self) -> bool {
        self.pick.is_empty() && self.reduce.is_empty() && self.facet.is_none()
    }

    /// The facet axis among `names`, if one is chosen and left over.
    pub fn facet_axis(&self, names: &[String], keep: &[usize]) -> Option<usize> {
        let f = self.facet.as_ref()?;
        names
            .iter()
            .position(|n| n == f)
            .filter(|a| !keep.contains(a))
    }
}

/// Values of `frame` (over axes of lengths `dims`, the first fastest) over
/// the axes `keep`, in that order (the first fastest): every other axis
/// picked, reduced or (the facet axis) taken at `facet_at`, as `s` says.
pub fn slice(
    frame: &[f64],
    dims: &[usize],
    names: &[String],
    keep: &[usize],
    s: &Slicing,
    facet_at: usize,
) -> Vec<f64> {
    if frame.len() != dims.iter().product::<usize>() {
        return vec![
            f64::NAN;
            keep.iter()
                .map(|&a| dims.get(a).copied().unwrap_or(1))
                .product()
        ];
    }
    let mut data = frame.to_vec();
    let mut left: Vec<(usize, usize)> = dims.iter().copied().enumerate().collect();
    // From the last axis down, so the places of those before stay.
    for axis in (0..dims.len()).rev() {
        if keep.contains(&axis) {
            continue;
        }
        let name = names.get(axis).map(String::as_str).unwrap_or("");
        let at = left
            .iter()
            .position(|&(a, _)| a == axis)
            .expect("an axis left");
        let len = left[at].1;
        let sizes: Vec<usize> = left.iter().map(|l| l.1).collect();
        data = if s.facet.as_deref() == Some(name) {
            take(&data, &sizes, at, facet_at.min(len.saturating_sub(1)))
        } else if let Some(&op) = s.reduce.get(name) {
            along(&data, &sizes, at, |vals| op.apply(vals))
        } else {
            let p = s.pick.get(name).copied().unwrap_or(0);
            take(&data, &sizes, at, p.min(len.saturating_sub(1)))
        };
        left.remove(at);
    }
    // The kept axes, in `keep`'s order.
    let order: Vec<usize> = keep
        .iter()
        .map(|a| left.iter().position(|l| l.0 == *a).expect("kept"))
        .collect();
    let sizes: Vec<usize> = left.iter().map(|l| l.1).collect();
    if order.windows(2).all(|w| w[0] < w[1]) {
        return data;
    }
    let strides: Vec<usize> = (0..sizes.len())
        .map(|i| sizes[..i].iter().product())
        .collect();
    let out_sizes: Vec<usize> = order.iter().map(|&i| sizes[i]).collect();
    let n: usize = out_sizes.iter().product();
    (0..n)
        .map(|mut k| {
            let mut at = 0;
            for (&i, &len) in order.iter().zip(&out_sizes) {
                at += (k % len) * strides[i];
                k /= len;
            }
            data[at]
        })
        .collect()
}

/// `data` (axes `sizes`) at place `p` along axis `axis`.
fn take(data: &[f64], sizes: &[usize], axis: usize, p: usize) -> Vec<f64> {
    along(data, sizes, axis, |vals| vals.nth(p).unwrap_or(f64::NAN))
}

/// `data` (axes `sizes`) with axis `axis` replaced by `f` of the values
/// along it.
fn along(
    data: &[f64],
    sizes: &[usize],
    axis: usize,
    f: impl Fn(&mut dyn Iterator<Item = f64>) -> f64,
) -> Vec<f64> {
    let inner: usize = sizes[..axis].iter().product();
    let len = sizes[axis];
    let outer: usize = sizes[axis + 1..].iter().product();
    let mut out = Vec::with_capacity(inner * outer);
    for o in 0..outer {
        for i in 0..inner {
            let mut vals = (0..len).map(|k| data[i + inner * (k + len * o)]);
            out.push(f(&mut vals));
        }
    }
    out
}

/// Controls for the axes `names` left over (not in `keep`): for each, pick
/// (with a slider), a reduction, or facets. Whether anything changed.
pub fn controls(
    ui: &mut egui::Ui,
    s: &mut Slicing,
    names: &[String],
    dims: &[usize],
    keep: &[usize],
    facets: bool,
) -> bool {
    let mut changed = false;
    for (axis, name) in names.iter().enumerate() {
        if keep.contains(&axis) || dims.get(axis).is_none_or(|&n| n < 2) {
            continue;
        }
        let len = dims[axis];
        ui.horizontal(|ui| {
            let now = if s.facet.as_deref() == Some(name) {
                "facets".to_string()
            } else if let Some(r) = s.reduce.get(name) {
                r.name().to_string()
            } else {
                "pick".to_string()
            };
            egui::ComboBox::from_id_salt(("slice", name))
                .selected_text(format!("{name}: {now}"))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(now == "pick", "pick").clicked() {
                        s.reduce.remove(name);
                        if s.facet.as_deref() == Some(name) {
                            s.facet = None;
                        }
                        changed = true;
                    }
                    for r in Reduce::ALL {
                        if ui.selectable_label(now == r.name(), r.name()).clicked() {
                            s.reduce.insert(name.clone(), r);
                            if s.facet.as_deref() == Some(name) {
                                s.facet = None;
                            }
                            changed = true;
                        }
                    }
                    if facets
                        && len <= MAX_FACETS
                        && ui.selectable_label(now == "facets", "facets").clicked()
                    {
                        s.reduce.remove(name);
                        s.facet = Some(name.clone());
                        changed = true;
                    }
                });
            if now == "pick" {
                let mut p = s.pick.get(name).copied().unwrap_or(0).min(len - 1);
                if ui.add(egui::Slider::new(&mut p, 0..=len - 1)).changed() {
                    s.pick.insert(name.clone(), p);
                    changed = true;
                }
            }
        });
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn leftover_axes_are_picked_reduced_or_faceted() {
        // v[i, j, k] = i + 10 j + 100 k over 2 × 3 × 4, i fastest.
        let dims = [2, 3, 4];
        let frame: Vec<f64> = (0..24)
            .map(|n| (n % 2) as f64 + 10.0 * ((n / 2) % 3) as f64 + 100.0 * (n / 6) as f64)
            .collect();
        let ax = names(&["i", "j", "k"]);
        let mut s = Slicing::default();
        // Keep i and j; k picked at its first place by default.
        assert_eq!(
            slice(&frame, &dims, &ax, &[0, 1], &s, 0),
            [0.0, 1.0, 10.0, 11.0, 20.0, 21.0]
        );
        s.pick.insert("k".into(), 3);
        assert_eq!(slice(&frame, &dims, &ax, &[0, 1], &s, 0)[0], 300.0);
        // Keep k and i, in that order: k fastest; j reduced.
        s.reduce.insert("j".into(), Reduce::Mean);
        assert_eq!(
            slice(&frame, &dims, &ax, &[2, 0], &s, 0),
            [10.0, 110.0, 210.0, 310.0, 11.0, 111.0, 211.0, 311.0]
        );
        s.reduce.insert("j".into(), Reduce::Maxof);
        assert_eq!(slice(&frame, &dims, &ax, &[0], &s, 0), [320.0, 321.0]);
        // A facet: k at the place asked.
        let f = Slicing {
            facet: Some("k".into()),
            ..Slicing::default()
        };
        assert_eq!(
            slice(&frame, &dims, &ax, &[1], &f, 2),
            [200.0, 210.0, 220.0]
        );
        assert_eq!(f.facet_axis(&ax, &[1]), Some(2));
        // Saved as written.
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"pick": {"k": 3}, "reduce": {"j": "maxof"}})
        );
        assert_eq!(serde_json::from_value::<Slicing>(json).unwrap(), s);
    }
}
