//! "Show definition" (ADR 0108, the Stage): one step from what is picked
//! to where it is defined. A used model opens on the Overview at its own
//! level; typed equations open in their editor; a mechanism's bodies are
//! its table in the Inspector, with the picked body's row open; any other
//! block is shown on the diagram.

use super::{Focus, SimApp};
use crate::dock::Pane;
use serde_json::{Map, Value};

/// Where a definition was shown.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Shown {
    /// The Overview, at this use's level.
    Model(String),
    /// The equation editor.
    Equations,
    /// The Inspector's table of bodies.
    Bodies,
    /// The diagram, with the block selected.
    Diagram,
}

/// The row of a mechanism's bodies that a name read from it belongs to:
/// `pole_centre_x` to the body `pole`, `hinge_angle` to the body on the
/// joint `hinge`. The longest name that fits wins.
pub(super) fn body_row(rows: &[Map<String, Value>], key: &str) -> Option<usize> {
    let name = |r: &Map<String, Value>, k: &str| {
        r.get(k).and_then(Value::as_str).unwrap_or("").to_string()
    };
    rows.iter()
        .enumerate()
        .flat_map(|(k, r)| [(k, name(r, "name")), (k, name(r, "joint"))])
        .filter(|(_, n)| !n.is_empty() && key.starts_with(&format!("{n}_")))
        .max_by_key(|(_, n)| n.len())
        .map(|(k, _)| k)
}

/// Whether a view reads from `block` (its ports or parameters, or what a
/// use offers), so that its definition is one step from the view.
pub(super) fn draws(tile: &simsandbox_session::views::Tile, block: &str) -> bool {
    simsandbox_session::session::view_reads(&[simsandbox_session::views::shown_entry(tile)])
        .iter()
        .any(|r| {
            r.strip_prefix(block)
                .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('/'))
        })
}

impl SimApp {
    /// Show where `block` (a block or use of the model's own level) is
    /// defined, and bring that pane to the front.
    pub(super) fn show_definition(&mut self, block: &str) -> Option<Shown> {
        let live = self.session.as_ref()?;
        let m = live.model();
        let entry = m.file.blocks.iter().find(|b| b.name == block)?.clone();
        drop(m);
        let shown = if entry.is_use() {
            self.overview_part = block.to_string();
            Shown::Model(block.to_string())
        } else if entry.type_name == simsandbox_eq::registry::EQUATIONS {
            self.editor = Some(super::editors::Editor {
                block: Some(block.to_string()),
                text: entry.equations.join("\n"),
                error: None,
            });
            Shown::Equations
        } else if !entry.bodies.is_empty() {
            Shown::Bodies
        } else {
            Shown::Diagram
        };
        if !matches!(shown, Shown::Model(_)) {
            self.select(block);
            self.picked = vec![(None, block.to_string())];
            self.inspected_view = None;
        }
        self.focus = match shown {
            Shown::Model(_) => Some(Focus::Pane(Pane::Overview)),
            Shown::Bodies => Some(Focus::Pane(Pane::Inspector)),
            Shown::Diagram => Some(Focus::Pane(Pane::Diagram)),
            Shown::Equations => None,
        };
        Some(shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simsandbox_session::examples::EXAMPLES;

    fn open(name: &str) -> SimApp {
        let mut app = SimApp::new(None, None, None, None);
        let k = EXAMPLES
            .iter()
            .position(|e| e.name.starts_with(name))
            .unwrap_or_else(|| panic!("no example {name}"));
        app.open_example(k);
        app
    }

    #[test]
    fn a_name_read_from_a_mechanism_finds_its_body() {
        let rows: Vec<Map<String, Value>> = serde_json::from_value(serde_json::json!([
            {"name": "cart", "joint": "slider"},
            {"name": "pole", "joint": "hinge"},
            {"name": "pole_top", "joint": "weld"}
        ]))
        .unwrap();
        assert_eq!(body_row(&rows, "slider_position"), Some(0));
        assert_eq!(body_row(&rows, "pole_centre_x"), Some(1));
        assert_eq!(body_row(&rows, "pole_top_mass"), Some(2));
        assert_eq!(body_row(&rows, "weld_at_x"), Some(2));
        assert_eq!(body_row(&rows, "g"), None);
    }

    #[test]
    fn a_picked_body_opens_its_row_and_its_definition_is_one_step_away() {
        // ADR 0108, the Stage: selecting a drawn body selects its row;
        // "Show definition" goes to where each kind of part is defined.
        let mut app = open("Cart-pole");
        app.inspect_signal("cartpole.pole_centre_x");
        assert_eq!(app.mech_open, Some(("cartpole".into(), 1)));
        app.inspect_signal("cartpole.slider_position");
        assert_eq!(app.mech_open, Some(("cartpole".into(), 0)));
        // The cart-pole's scene draws it; the definition button shows.
        assert!(draws(&app.views.tiles[0], "cartpole"));
        assert!(!draws(&app.views.tiles[0], "cart"));
        assert_eq!(app.show_definition("cartpole"), Some(Shown::Bodies));
        assert_eq!(app.focus, Some(Focus::Pane(Pane::Inspector)));
        assert_eq!(
            app.diagram.canvas.selected,
            Some(crate::canvas::Selection::Block("cartpole".into()))
        );
        assert_eq!(app.show_definition("hand"), Some(Shown::Diagram));
        assert_eq!(app.focus, Some(Focus::Pane(Pane::Diagram)));
        assert_eq!(app.show_definition("nothing"), None);

        let mut app = open("Two balancing poles");
        assert_eq!(
            app.show_definition("left"),
            Some(Shown::Model("left".into()))
        );
        assert_eq!(app.overview_part, "left");
        assert_eq!(app.focus, Some(Focus::Pane(Pane::Overview)));
    }

    #[test]
    fn typed_equations_open_in_their_editor() {
        let mut app = open("Predator");
        let live = app.session.as_ref().unwrap();
        let block = live
            .model()
            .file
            .blocks
            .iter()
            .find(|b| b.type_name == simsandbox_eq::registry::EQUATIONS)
            .map(|b| b.name.clone())
            .expect("an equations block");
        assert_eq!(app.show_definition(&block), Some(Shown::Equations));
        let editor = app.editor.as_ref().unwrap();
        assert_eq!(editor.block.as_deref(), Some(block.as_str()));
        assert!(!editor.text.is_empty());
    }
}
