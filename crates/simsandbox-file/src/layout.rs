//! How the app's panes are arranged (ADR 0053): `controls.layout`, a tree
//! of splits whose leaves are stacks of tabs. A tab is the controls (pinned
//! parameters), the inspector, the diagram, one view (`view:K`, its place in `controls.views`), or several
//! views tiled together (`{"views": [0, 2]}`). Like all of `controls` it
//! never changes results, and what cannot be used is dropped with a
//! warning.

use serde::{Deserialize, Serialize};

/// A node of the arrangement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Layout {
    /// Two parts, `first` left of or above `second`; `fraction` is the
    /// first's share.
    Split {
        split: SplitKind,
        fraction: f64,
        first: Box<Layout>,
        second: Box<Layout>,
    },
    /// Tabs stacked in one place; `active` is the one shown.
    Tabs {
        tabs: Vec<Pane>,
        #[serde(default, skip_serializing_if = "is_zero")]
        active: usize,
    },
}

fn is_zero(k: &usize) -> bool {
    *k == 0
}

/// How a split divides its space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitKind {
    /// Side by side.
    LeftRight,
    /// One above the other.
    TopBottom,
}

/// One tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Pane {
    /// `"controls"`, `"inspector"`, `"diagram"` or `"view:K"`.
    Named(String),
    /// Views tiled together in one tab.
    Views { views: Vec<usize> },
}

impl Pane {
    pub const CONTROLS: &'static str = "controls";
    pub const INSPECTOR: &'static str = "inspector";
    pub const DIAGRAM: &'static str = "diagram";

    pub fn view(k: usize) -> Self {
        Pane::Named(format!("view:{k}"))
    }

    /// The views it shows.
    pub fn views(&self) -> Vec<usize> {
        match self {
            Pane::Named(n) => n
                .strip_prefix("view:")
                .and_then(|k| k.parse().ok())
                .into_iter()
                .collect(),
            Pane::Views { views } => views.clone(),
        }
    }
}

impl Layout {
    /// Every pane, leaf by leaf.
    pub fn panes(&self) -> Vec<&Pane> {
        match self {
            Layout::Split { first, second, .. } => {
                let mut v = first.panes();
                v.extend(second.panes());
                v
            }
            Layout::Tabs { tabs, .. } => tabs.iter().collect(),
        }
    }

    /// Every view it shows, in order.
    pub fn views(&self) -> Vec<usize> {
        self.panes().iter().flat_map(|p| p.views()).collect()
    }

    /// Keep what can be used with `count` views, renumbered by `map` (a
    /// view's new place, `None` if it was dropped): unknown tabs, views
    /// that do not exist and a view shown twice are dropped, empty stacks
    /// and splits collapse. `None` if nothing is left. Problems go to
    /// `warnings`.
    pub fn checked(
        self,
        count: usize,
        map: &dyn Fn(usize) -> Option<usize>,
        warnings: &mut Vec<String>,
    ) -> Option<Layout> {
        let mut seen = Vec::new();
        self.check_inner(count, map, &mut seen, warnings)
    }

    fn check_inner(
        self,
        count: usize,
        map: &dyn Fn(usize) -> Option<usize>,
        seen: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) -> Option<Layout> {
        match self {
            Layout::Split {
                split,
                fraction,
                first,
                second,
            } => {
                let fraction = if fraction.is_finite() && fraction > 0.0 && fraction < 1.0 {
                    fraction
                } else {
                    warnings.push(format!(
                        "controls.layout: a split's fraction {fraction} is not between 0 and 1; \
                         using 0.5"
                    ));
                    0.5
                };
                let a = first.check_inner(count, map, seen, warnings);
                let b = second.check_inner(count, map, seen, warnings);
                match (a, b) {
                    (Some(a), Some(b)) => Some(Layout::Split {
                        split,
                        fraction,
                        first: Box::new(a),
                        second: Box::new(b),
                    }),
                    (a, b) => a.or(b),
                }
            }
            Layout::Tabs { tabs, active } => {
                let mut kept = Vec::new();
                for pane in tabs {
                    let pane = match pane {
                        Pane::Named(n)
                            if [Pane::CONTROLS, Pane::INSPECTOR, Pane::DIAGRAM]
                                .contains(&n.as_str()) =>
                        {
                            Some(Pane::Named(n))
                        }
                        Pane::Named(n) => match n.strip_prefix("view:").map(str::parse::<usize>) {
                            Some(Ok(k)) => match map(k).filter(|&j| j < count) {
                                Some(j) => Some(Pane::view(j)),
                                None => {
                                    warnings.push(format!(
                                        "controls.layout: {n} is not a view of this model; dropped"
                                    ));
                                    None
                                }
                            },
                            _ => {
                                warnings.push(format!(
                                    "controls.layout: unknown tab '{n}' (expected controls, \
                                     inspector, diagram or view:K); dropped"
                                ));
                                None
                            }
                        },
                        Pane::Views { views } => {
                            let mut v = Vec::new();
                            for k in views {
                                match map(k).filter(|&j| j < count) {
                                    Some(j) if !v.contains(&j) => v.push(j),
                                    Some(_) => {}
                                    None => warnings.push(format!(
                                        "controls.layout: view {k} is not a view of this model; \
                                         dropped"
                                    )),
                                }
                            }
                            (!v.is_empty()).then_some(Pane::Views { views: v })
                        }
                    };
                    let Some(pane) = pane else {
                        continue;
                    };
                    // Each tab and each view once.
                    let keys: Vec<String> = match &pane {
                        Pane::Named(n) if !n.starts_with("view:") => vec![n.clone()],
                        p => p.views().iter().map(|k| format!("view:{k}")).collect(),
                    };
                    if keys.iter().any(|k| seen.contains(k)) {
                        warnings.push(format!(
                            "controls.layout: {} is shown twice; the second dropped",
                            keys.join(", ")
                        ));
                        continue;
                    }
                    seen.extend(keys);
                    kept.push(pane);
                }
                if kept.is_empty() {
                    return None;
                }
                let active = active.min(kept.len() - 1);
                Some(Layout::Tabs { tabs: kept, active })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Layout {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn layouts_read_write_and_drop_what_cannot_be_shown() {
        let text = r#"{"split": "left_right", "fraction": 0.25,
            "first": {"tabs": ["inspector"]},
            "second": {"split": "top_bottom", "fraction": 0.6,
                "first": {"tabs": [{"views": [0, 2]}, "diagram"], "active": 1},
                "second": {"tabs": ["view:1", "view:7", "plots"]}}}"#;
        let l = parse(text);
        assert_eq!(l.views(), [0, 2, 1, 7]);
        let mut w = Vec::new();
        // Three views; view 2 was dropped, so 1 stays and later ones move up.
        let map = |k: usize| match k {
            2 => None,
            k if k > 2 => Some(k - 1),
            k => Some(k),
        };
        let l = l.checked(3, &map, &mut w).unwrap();
        assert_eq!(l.views(), [0, 1]);
        assert_eq!(w.len(), 3, "{w:?}");
        let again: Layout = parse(&serde_json::to_string(&l).unwrap());
        assert_eq!(again, l);
        // Nothing usable: no layout.
        let l = parse(r#"{"tabs": ["view:9"]}"#);
        assert!(l.checked(1, &|k| Some(k), &mut Vec::new()).is_none());
        // A stack that empties collapses its split.
        let l = parse(
            r#"{"split": "left_right", "fraction": 2.0,
            "first": {"tabs": ["nothing"]}, "second": {"tabs": ["diagram"]}}"#,
        );
        let l = l.checked(0, &|k| Some(k), &mut Vec::new()).unwrap();
        assert_eq!(l, parse(r#"{"tabs": ["diagram"]}"#));
    }
}
