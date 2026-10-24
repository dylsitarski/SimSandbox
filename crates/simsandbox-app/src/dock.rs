//! The app's panes as dockable tabs (ADR 0053): the controls, the
//! inspector, the diagram, and views — each alone in a tab, or several tiled together in
//! one. Tabs can be dragged, stacked and split (egui_dock), and popped out
//! into their own windows. The arrangement is saved in the model file as
//! `controls.layout`, views by their place in `controls.views`; popped-out
//! tabs are saved as tabs of the first stack.

use egui_dock::{DockState, Node, NodeIndex, Split, TabIndex};
use simsandbox_file::layout::{Layout, Pane as FilePane, SplitKind};

/// One tab.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Pane {
    /// The pinned parameters.
    Controls,
    Inspector,
    Diagram,
    /// One view, by its tile's id.
    View(u64),
    /// Views tiled together; `id` names the tab.
    Views {
        id: u64,
        tiles: Vec<u64>,
    },
}

impl Pane {
    /// The view tiles it shows.
    pub fn tiles(&self) -> Vec<u64> {
        match self {
            Pane::View(t) => vec![*t],
            Pane::Views { tiles, .. } => tiles.clone(),
            _ => Vec::new(),
        }
    }
}

/// The dock and the tabs popped out of it.
pub struct Dock {
    pub state: DockState<Pane>,
    /// Tabs shown in their own windows.
    pub popped: Vec<Pane>,
    next_group: u64,
}

impl Dock {
    /// The arrangement a model without one opens with: the controls above
    /// the inspector on the left; on the right the views tiled together,
    /// with the diagram as a second tab.
    pub fn standard(tiles: &[u64]) -> Self {
        let mut d = Self {
            state: DockState::new(vec![Pane::Inspector]),
            popped: Vec::new(),
            next_group: 0,
        };
        let group = d.group(tiles.to_vec());
        d.state
            .main_surface_mut()
            .split_right(NodeIndex::root(), 0.3, vec![group, Pane::Diagram]);
        d.add_controls();
        d
    }

    /// Put the controls above the inspector (or in the first stack).
    fn add_controls(&mut self) {
        let leaf = self
            .find(|p| *p == Pane::Inspector)
            .filter(|path| path.surface.is_main())
            .map(|path| path.node_path().node);
        match leaf {
            Some(at) => {
                self.state.main_surface_mut().split(
                    at,
                    Split::Above,
                    // The new (upper) part's share.
                    0.3,
                    Node::leaf_with(vec![Pane::Controls]),
                );
            }
            None => self.state.push_to_first_leaf(Pane::Controls),
        }
    }

    /// A new tab of views tiled together.
    pub fn group(&mut self, tiles: Vec<u64>) -> Pane {
        self.next_group += 1;
        Pane::Views {
            id: self.next_group,
            tiles,
        }
    }

    /// The arrangement saved in a file; `tile` gives the tile showing each
    /// saved view (`None` if it could not be shown). Tiles not placed join
    /// the first tab of views; the inspector and the diagram are added if
    /// missing.
    pub fn from_layout(layout: &Layout, tile: &dyn Fn(usize) -> Option<u64>, all: &[u64]) -> Self {
        let mut d = Self {
            state: DockState::new(Vec::new()),
            popped: Vec::new(),
            next_group: 0,
        };
        let pane = |d: &mut Self, p: &FilePane| -> Option<Pane> {
            match p {
                FilePane::Named(n) if n == FilePane::CONTROLS => Some(Pane::Controls),
                FilePane::Named(n) if n == FilePane::INSPECTOR => Some(Pane::Inspector),
                FilePane::Named(n) if n == FilePane::DIAGRAM => Some(Pane::Diagram),
                p => {
                    let tiles: Vec<u64> = p.views().into_iter().filter_map(tile).collect();
                    match (p, tiles.len()) {
                        (_, 0) => None,
                        (FilePane::Named(_), _) => Some(Pane::View(tiles[0])),
                        _ => Some(d.group(tiles)),
                    }
                }
            }
        };
        // The first (top-left) stack starts the dock; each split then adds
        // its second part beside or below the first.
        fn leftmost(l: &Layout) -> &Layout {
            match l {
                Layout::Split { first, .. } => leftmost(first),
                tabs => tabs,
            }
        }
        fn tabs_of(
            d: &mut Dock,
            l: &Layout,
            pane: &dyn Fn(&mut Dock, &FilePane) -> Option<Pane>,
        ) -> (Vec<Pane>, usize) {
            match leftmost(l) {
                Layout::Tabs { tabs, active } => {
                    (tabs.iter().filter_map(|p| pane(d, p)).collect(), *active)
                }
                Layout::Split { .. } => unreachable!("leftmost is a stack"),
            }
        }
        fn place(
            d: &mut Dock,
            l: &Layout,
            at: NodeIndex,
            pane: &dyn Fn(&mut Dock, &FilePane) -> Option<Pane>,
        ) {
            match l {
                Layout::Tabs { active, .. } => {
                    if let Node::Leaf(leaf) = &mut d.state.main_surface_mut()[at] {
                        leaf.active = TabIndex((*active).min(leaf.tabs.len().saturating_sub(1)));
                    }
                }
                Layout::Split {
                    split,
                    fraction,
                    first,
                    second,
                } => {
                    let (tabs, _) = tabs_of(d, second, pane);
                    if tabs.is_empty() {
                        place(d, first, at, pane);
                        return;
                    }
                    let side = match split {
                        SplitKind::LeftRight => Split::Right,
                        SplitKind::TopBottom => Split::Below,
                    };
                    let f = (*fraction as f32).clamp(0.05, 0.95);
                    let [a, b] =
                        d.state
                            .main_surface_mut()
                            .split(at, side, f, Node::leaf_with(tabs));
                    place(d, first, a, pane);
                    place(d, second, b, pane);
                }
            }
        }
        let (first, _) = tabs_of(&mut d, layout, &pane);
        d.state = DockState::new(if first.is_empty() {
            vec![Pane::Inspector]
        } else {
            first
        });
        place(&mut d, layout, NodeIndex::root(), &pane);
        if d.find(|p| *p == Pane::Inspector).is_none() {
            d.state.push_to_first_leaf(Pane::Inspector);
        }
        if d.find(|p| *p == Pane::Diagram).is_none() {
            d.state.push_to_first_leaf(Pane::Diagram);
        }
        if d.find(|p| *p == Pane::Controls).is_none() {
            d.add_controls();
        }
        d.sync(all);
        d
    }

    /// The arrangement to save; `index` gives each tile's place among the
    /// views.
    pub fn to_layout(&self, index: &dyn Fn(u64) -> Option<usize>) -> Option<Layout> {
        let pane = |p: &Pane| -> Option<FilePane> {
            match p {
                Pane::Controls => Some(FilePane::Named(FilePane::CONTROLS.into())),
                Pane::Inspector => Some(FilePane::Named(FilePane::INSPECTOR.into())),
                Pane::Diagram => Some(FilePane::Named(FilePane::DIAGRAM.into())),
                Pane::View(t) => index(*t).map(FilePane::view),
                Pane::Views { tiles, .. } => {
                    let views: Vec<usize> = tiles.iter().filter_map(|t| index(*t)).collect();
                    (!views.is_empty()).then_some(FilePane::Views { views })
                }
            }
        };
        let tree = self.state.main_surface();
        fn walk(
            tree: &egui_dock::Tree<Pane>,
            at: NodeIndex,
            pane: &dyn Fn(&Pane) -> Option<FilePane>,
        ) -> Option<Layout> {
            if at.0 >= tree.len() {
                return None;
            }
            let split = |kind, fraction: f32| {
                let a = walk(tree, at.left(), pane);
                let b = walk(tree, at.right(), pane);
                match (a, b) {
                    (Some(a), Some(b)) => Some(Layout::Split {
                        split: kind,
                        fraction: ((fraction as f64) * 1000.0).round() / 1000.0,
                        first: Box::new(a),
                        second: Box::new(b),
                    }),
                    (a, b) => a.or(b),
                }
            };
            match &tree[at] {
                Node::Empty => None,
                Node::Leaf(leaf) => {
                    let tabs: Vec<FilePane> = leaf.tabs.iter().filter_map(pane).collect();
                    (!tabs.is_empty()).then(|| Layout::Tabs {
                        active: leaf.active.0.min(tabs.len() - 1),
                        tabs,
                    })
                }
                Node::Horizontal(s) => split(SplitKind::LeftRight, s.fraction),
                Node::Vertical(s) => split(SplitKind::TopBottom, s.fraction),
            }
        }
        let mut layout = walk(tree, NodeIndex::root(), &pane)?;
        // Popped-out tabs, and any in floating windows, go to the first
        // stack.
        let others: Vec<FilePane> = self
            .popped
            .iter()
            .chain(
                self.state
                    .iter_all_tabs()
                    .filter(|(path, _)| !path.surface.is_main())
                    .map(|(_, p)| p),
            )
            .filter_map(pane)
            .collect();
        if !others.is_empty() {
            let mut node = &mut layout;
            while let Layout::Split { first, .. } = node {
                node = first;
            }
            if let Layout::Tabs { tabs, .. } = node {
                tabs.extend(others);
            }
        }
        Some(layout)
    }

    /// Every tab, docked or popped out.
    pub fn panes(&self) -> Vec<Pane> {
        self.state
            .iter_all_tabs()
            .map(|(_, p)| p.clone())
            .chain(self.popped.iter().cloned())
            .collect()
    }

    fn find(&self, f: impl Fn(&Pane) -> bool) -> Option<egui_dock::TabPath> {
        self.state.find_tab_from(f)
    }

    /// Keep the tabs in step with the tiles: tiles that are gone leave
    /// their tabs; tiles not shown anywhere join the first tab of views
    /// (or a new one beside the diagram).
    pub fn sync(&mut self, tiles: &[u64]) {
        let keep = |p: &mut Pane| match p {
            Pane::View(t) => tiles.contains(t),
            Pane::Views { tiles: v, .. } => {
                v.retain(|t| tiles.contains(t));
                true
            }
            _ => true,
        };
        for (_, p) in self.state.iter_all_tabs_mut() {
            keep(p);
        }
        self.state.retain_tabs(|p| match p {
            Pane::View(t) => tiles.contains(t),
            _ => true,
        });
        for p in &mut self.popped {
            keep(p);
        }
        self.popped.retain(|p| match p {
            Pane::View(t) => tiles.contains(t),
            Pane::Views { tiles, .. } => !tiles.is_empty(),
            _ => true,
        });
        let shown: Vec<u64> = self.panes().iter().flat_map(Pane::tiles).collect();
        let missing: Vec<u64> = tiles
            .iter()
            .copied()
            .filter(|t| !shown.contains(t))
            .collect();
        let first_group = self.state.iter_all_tabs_mut().find_map(|(_, p)| match p {
            Pane::Views { tiles, .. } => Some(tiles),
            _ => None,
        });
        match first_group {
            Some(v) => v.extend(missing),
            None if self.popped.iter().any(|p| matches!(p, Pane::Views { .. }))
                && missing.is_empty() => {}
            // There is always a tab of views, empty if every view has a tab
            // of its own, so views can be tiled again.
            None => {
                let group = self.group(missing);
                self.beside_diagram(group);
            }
        }
    }

    /// Add a tab in front of the diagram's stack (or to the first stack).
    fn beside_diagram(&mut self, pane: Pane) {
        if let Some(path) = self.find(|p| *p == Pane::Diagram) {
            if let Some(leaf) = self
                .state
                .main_surface_mut()
                .iter_mut()
                .nth(path.node_path().node.0)
                .and_then(|n| n.get_leaf_mut())
            {
                leaf.tabs.insert(0, pane);
                leaf.active = TabIndex(0);
                return;
            }
        }
        self.state.push_to_first_leaf(pane);
    }

    /// Show a tab: bring it to the front of its stack (or of its window).
    pub fn focus(&mut self, f: impl Fn(&Pane) -> bool) {
        if let Some(path) = self.find(f) {
            let _ = self.state.set_active_tab(path);
            self.state.set_focused_node_and_surface(path.node_path());
        }
    }

    /// Move a tile out of its tab of views into a tab of its own, beside
    /// that one.
    pub fn own_tab(&mut self, tile: u64) {
        let mut found = None;
        for (path, p) in self.state.iter_all_tabs_mut() {
            if let Pane::Views { tiles, .. } = p {
                if let Some(k) = tiles.iter().position(|t| *t == tile) {
                    tiles.remove(k);
                    found = Some(path);
                }
            }
        }
        let Some(path) = found else {
            return;
        };
        let _ = self.state.set_active_tab(path);
        self.state.set_focused_node_and_surface(path.node_path());
        self.state.push_to_focused_leaf(Pane::View(tile));
        self.drop_empty_groups();
    }

    /// Put a tile back into the first tab of views, closing its own tab.
    pub fn tile_back(&mut self, tile: u64) {
        self.state.retain_tabs(|p| *p != Pane::View(tile));
        self.popped.retain(|p| *p != Pane::View(tile));
        let group = self.state.iter_all_tabs_mut().find_map(|(_, p)| match p {
            Pane::Views { tiles, .. } => Some(tiles),
            _ => None,
        });
        match group {
            Some(v) => v.push(tile),
            None => {
                let g = self.group(vec![tile]);
                self.state.push_to_first_leaf(g);
            }
        }
    }

    /// Every view tiled together again in one tab, where the first tab of
    /// views is (or beside the diagram).
    pub fn tile_all(&mut self, tiles: &[u64]) {
        self.state.retain_tabs(|p| !matches!(p, Pane::View(_)));
        self.popped.retain(|p| p.tiles().is_empty());
        let mut first = true;
        for (_, p) in self.state.iter_all_tabs_mut() {
            if let Pane::Views { tiles: v, .. } = p {
                *v = if first { tiles.to_vec() } else { Vec::new() };
                first = false;
            }
        }
        self.drop_empty_groups();
        self.sync(tiles);
    }

    /// Close empty tabs of views but the first.
    fn drop_empty_groups(&mut self) {
        let first = self.state.iter_all_tabs().find_map(|(_, p)| match p {
            Pane::Views { id, .. } => Some(*id),
            _ => None,
        });
        self.state.retain_tabs(|p| match p {
            Pane::Views { id, tiles } => !tiles.is_empty() || Some(*id) == first,
            _ => true,
        });
    }

    /// Show a tab in its own window.
    pub fn pop_out(&mut self, pane: &Pane) {
        if let Some(path) = self.find(|p| p == pane) {
            if let Some(p) = self.state.remove_tab(path) {
                self.popped.push(p);
            }
        }
    }

    /// Bring a popped-out tab back into the dock.
    pub fn dock_back(&mut self, pane: &Pane) {
        if let Some(k) = self.popped.iter().position(|p| p == pane) {
            let p = self.popped.remove(k);
            if self.state.iter_all_tabs().next().is_none() {
                self.state = DockState::new(vec![p]);
            } else {
                self.state.push_to_first_leaf(p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(json: &str) -> Layout {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn arrangements_round_trip_through_the_dock() {
        let l = layout(
            r#"{"split": "left_right", "fraction": 0.3,
                "first": {"tabs": ["inspector", "controls"]},
                "second": {"split": "top_bottom", "fraction": 0.6,
                    "first": {"tabs": [{"views": [0, 2]}, "diagram"], "active": 1},
                    "second": {"tabs": ["view:1"]}}}"#,
        );
        // Views 0, 1, 2 are tiles 10, 11, 12.
        let tile = |k: usize| Some(10 + k as u64);
        let d = Dock::from_layout(&l, &tile, &[10, 11, 12]);
        let index = |t: u64| Some((t - 10) as usize);
        assert_eq!(d.to_layout(&index), Some(l));
        // An arrangement saved before the controls had a pane gets them
        // above the inspector.
        let old = layout(
            r#"{"split": "left_right", "fraction": 0.3,
            "first": {"tabs": ["inspector"]}, "second": {"tabs": ["diagram"]}}"#,
        );
        let d = Dock::from_layout(&old, &tile, &[]);
        let expected = layout(
            r#"{"split": "left_right", "fraction": 0.3,
                "first": {"split": "top_bottom", "fraction": 0.3,
                    "first": {"tabs": ["controls"]}, "second": {"tabs": ["inspector"]}},
                "second": {"tabs": ["diagram"]}}"#,
        );
        assert_eq!(d.to_layout(&index), Some(expected));
    }

    #[test]
    fn tabs_follow_the_tiles_and_move_between_tabs() {
        let mut d = Dock::standard(&[1, 2, 3]);
        let index = |t: u64| Some((t - 1) as usize);
        let saved = d.to_layout(&index).unwrap();
        assert_eq!(saved.views(), [0, 1, 2]);
        // A view in a tab of its own, and back.
        d.own_tab(2);
        assert!(d.panes().contains(&Pane::View(2)));
        assert_eq!(d.to_layout(&index).unwrap().views(), [0, 2, 1]);
        d.tile_back(2);
        assert!(!d.panes().contains(&Pane::View(2)));
        // Every view in a tab of its own: the tab of views stays, empty,
        // and comes back after the arrangement is saved and read.
        let mut alone = Dock::standard(&[7]);
        alone.own_tab(7);
        let groups = |d: &Dock| {
            d.panes()
                .into_iter()
                .filter(|p| matches!(p, Pane::Views { .. }))
                .count()
        };
        assert_eq!(groups(&alone), 1);
        alone.sync(&[]);
        assert_eq!(groups(&alone), 1);
        let saved = alone.to_layout(&|_| Some(0)).unwrap();
        assert_eq!(groups(&Dock::from_layout(&saved, &|_| Some(7), &[7])), 1);
        // A tile closed elsewhere leaves; a new one joins the views.
        d.own_tab(3);
        d.sync(&[1, 2, 4]);
        let tiles: Vec<u64> = d.panes().iter().flat_map(Pane::tiles).collect();
        assert_eq!(tiles, [1, 2, 4]);
        // Popped out, it is still saved, in the first stack.
        d.pop_out(&Pane::Diagram);
        assert_eq!(d.popped, [Pane::Diagram]);
        let saved = d.to_layout(&|t| Some(t as usize)).unwrap();
        assert!(saved
            .panes()
            .contains(&&FilePane::Named(FilePane::DIAGRAM.into())));
        d.dock_back(&Pane::Diagram);
        assert!(d.popped.is_empty());
        // Everything tiled together again.
        d.own_tab(1);
        d.tile_all(&[1, 2, 4]);
        let groups: Vec<Pane> = d
            .panes()
            .into_iter()
            .filter(|p| !p.tiles().is_empty())
            .collect();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].tiles(), [1, 2, 4]);
    }
}
