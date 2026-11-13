# 0086 — Views bind fields by axis roles; meshes are drawn on the GPU

- **Status:** Accepted (owner, 2026-11-12: wgpu now, both together; the rest proposed here, mine)
- **Date:** 2026-11-12
- **Builds on:** [ADR 0081](0081-shapes-are-lists-of-axes.md) point 5 (views bind to axes by role: pick, reduce, facet), [ADR 0085](0085-meshes-are-tables-of-cells.md) point 8 (views of meshes), [ADR 0024](0024-heat-map-axes-and-coordinates.md) (axes and coordinates), [ADR 0041](0041-headless-interface.md) (headless rendering), [ADR 0008](0008-native-egui-ui.md) (wgpu paint callbacks)

## Context

ADR 0085 step 4 is to draw meshes: cells filled by a value in 2D, a 3D mesh's outer surface. ADR 0081's stage 3, views that bind fields by axis roles, was its own item (STATUS item 5). Asked on 2026-11-12, the owner chose to build both together, and to draw meshes with wgpu now rather than with egui's triangles.

**What exists:** fields over one or two axes are drawn (heat map, profile, surface, contour); tensors (three or more axes) and table rows are recorded but not drawn; cut filters make lines through 2D grids; headless renders rasterise egui's own triangles on the CPU (ADR 0041), so a wgpu paint callback does not appear in them; `plot.3d` and `plot.surface` project on the CPU and sort quads (the painter's algorithm, at most 40 000).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **wgpu paint callbacks in the window, the same triangles drawn by egui headless** | Large meshes at frame rate with a depth buffer; headless renders keep working from the same geometry | Two drawing paths from one geometry; the window's picture is checked by screenshot on a software driver |
| egui triangles everywhere | One path; headless exact | CPU-bound; no depth buffer (painter's sorting); slow past about 100 000 triangles |
| wgpu everywhere, headless rendered offscreen | One path | Headless needs a GPU or a software Vulkan driver everywhere (CI, Python users) |

## Decision

**1. Every field has axes in the history.** A recorded field carries its axes (each a name and a length, or a table's rows) from its port (`PortSpec::axis_names`, ADR 0081). Views may draw tensors and table rows.

**2. A view's field slot draws a number of axes**, as its role: a profile one, a heat map, contour or surface two, a mesh view a mesh. Axis slots (`x`, `y`) choose which of the field's axes are drawn, by binding a coordinate (as now) or the axis itself (`axis:NAME`). Unchosen, the field's first axes are drawn.

**3. Every axis left over** is, as saved with the view:
- **picked** (`"selection": {"pick": {"z": 3}}`), one place along it, chosen by a slider in the tile; by default the first place;
- **reduced** (`{"reduce": {"run": "mean"}}`), by `mean`, `minof`, `maxof` or `sum`;
- **faceted** (`{"facet": "c"}`), one small panel per place (at most 16); a profile draws one curve per place instead.

The slice is computed from the newest frame before drawing (`fields::slice`), so every field view takes fields of any rank. A profile of a 2D field along one axis with the other picked is an axis-aligned cut; cut filters stay for free lines and interpolation, which compute new fields.

**4. A field over a table's rows** is drawn by its rows: a profile against a coordinate over the same table (sorted by it), `plot.xy` cell by cell, and a mesh view.

**5. Meshes reach views through their ports.** Typed equations give every table of cells an output named after it: each cell's corners as ids of its points (`Rows { dim: corners }`), declaring its shape and its points' coordinate outputs (the states a starting mesh sets, `start c = grid …` or `file … at x, y`). The history lists meshes (`Available::meshes`: the cells field, the shape, the coordinates), so a field over cells is drawn on its mesh with nothing bound by hand.

**6. `plot.mesh`** draws a field over a mesh's cells: in 2D each cell filled by its value (cells split into triangles); in 3D the outer surface (the sides with one cell), shaded and turned with the mouse like `plot.3d`; a field over the points colours by corner instead. Cuts through 3D meshes come later, as picks of a coordinate.

**7. Drawing.** The view builds triangles on the CPU (positions, colours) from the newest frame. In the window they are drawn by a wgpu paint callback that renders into a texture of its own with a depth buffer (in the callback's `prepare`) and puts that texture on screen (in `paint`); the pipeline is installed when the app starts (`gpu::install`). Without it (headless, or a backend without wgpu) the same triangles are drawn as an egui mesh, sorted furthest first. The two pictures agree up to rasterisation.

**Order of work:**
- **4a.** Axes in the history; leftover axes picked, reduced or faceted for the heat map, profile, surface and contour; tensors offered to views; saved, checked and rendered headless.
- **4b.** Fields over table rows in views (profile by a coordinate over the same table, `plot.xy` cell by cell).
- **4c.** Mesh ports from typed equations, `Available::meshes`, `plot.mesh` in 2D and 3D drawn as egui triangles (the headless path).
- **4d.** The wgpu path in the window, checked by screenshot on a software driver.

## Consequences

- Tensors and table rows, recorded since ADR 0081 stage 2 and ADR 0079 stage 1c, become visible.
- Views gain three saved settings (`pick`, `reduce`, `facet`); old files are unchanged (a 2D field bound to a heat map has no leftover axes).
- The app depends on egui-wgpu through `simsandbox-views`' `gpu` feature; headless builds do not.
- Updated with the work: [visualization](../architecture/visualization.md), the [headless guide](../guides/headless.md), ADR 0081 (stage 3 as built), ADR 0085 (step 4 as built).

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none. Pick, reduce and facet are ADR 0081's; a mesh is ADR 0085's.

## As built

**Step 4a (2026-11-12): axis roles.** `crates/simsandbox-views/src/slice.rs` and the field views.
- **Axes in the history:** `History::set_axes` and `History::axes(field)` (names and lengths, the first fastest); the session names them from each output's port (`PortSpec::axis_names`), else `x`, `y`, `z`, `a3`, ….
- **Slicing:** `Slicing { pick, reduce, facet }` (`mean`, `minof`, `maxof`, `sum`), saved as the view's `selection`, and `slice(frame, dims, names, keep, slicing, facet_at)`, the values over the kept axes in their order. Each leftover axis gets a combo box and, when picked, a slider in the tile.
- **Views:** the heat map, surface and contour draw any field of two or more axes (the `x` and `y` slots choose two by coordinate, else the first two); the heat map draws facets in a grid of small panels on one colour scale; the profile draws any field along one axis (the one its `x` coordinate places, else the first), with facets as one curve per place. A field of one or two axes looks as before.
- **Offered:** tensors are offered to views as grids (`Available::grids`), and a profile's `y` takes any field; coordinates may place one axis of a tensor; view bindings in files may name tensors and rows.
- **Not yet:** fields over table rows in views (4b), meshes (4c, 4d); picks in the side panel (they are in the tile).
- **Tested:** slicing (picks, reductions, facets, keeping axes in another order, saving); a three-axis field in a heat map picked and reduced; headless renders of a typed-equations field over `[i, j, k]` in a heat map (first place, picked, reduced, faceted), surface and contour, and a profile of a 2D field picked and faceted. Every example unchanged byte for byte.

**Steps 4b and 4c (2026-11-12): rows and meshes in views.** `crates/simsandbox-views/src/mesh.rs`, the session and typed equations.
- **Rows (4b):** every field over a table's rows is a coordinate (axis 0) of the others over the same table (by the table's name in the ports' axis names), so a profile draws one against another, paired by id and sorted along `x`. `Available::rows` lists them; `plot.xy` cell by cell over rows waits.
- **Mesh ports (4c):** typed equations give each table of cells with a starting mesh an output named after the table, `Rows { dim: corners }`, each cell's corners as ids of its points, declaring `PortSpec::mesh` (`MeshOf`: shape, 2 or 3 coordinates, the polygons drawn: a 2D cell's own, a 3D cell's sides in order around). Segments, prisms, pyramids and higher dimensions have none.
- **History and `Available`:** `History::set_mesh`/`mesh` and `table`; `Available::meshes` (`MeshInfo`: the cells field, its coordinates, the fields over its cells and over its points).
- **`plot.mesh`:** slots `value` (rows over the cells or the points), `mesh`, `x`, `y`, `z`; styles `log_colour` and `edges` (outlines below 4000 cells). `triangles` builds a fan of triangles per polygon from the newest frame; 2D cells are filled in the plane with the axes' proportions kept; in 3D the sides of one cell only are drawn (a mesh of 2D cells in 3D draws every cell), shaded by a light above and in front, furthest first, each with its outline, turned like `plot.3d`. A model with a mesh and no saved views opens on it.
- **A fix:** rows outputs of states over cells, sides or edges read the table's first columns (the corners) instead of the states after them; they read past them now. Before this step, a view or `run` output of such a state was wrong.
- **Tested:** triangles of 2D cells (values per cell) and a solid's outer sides (two hexahedra: ten); a session over refining triangles and over hexahedra: its mesh info, each cell's own value recorded, rendered without warnings; a rows profile. Every example unchanged byte for byte.

**Step 4d (2026-11-12): meshes on the GPU in the window.** `crates/simsandbox-views/src/gpu.rs` (feature `gpu`, enabled by the app).
- **Install:** the app calls `gpu::install` with eframe's wgpu render state at start: two pipelines drawing triangles and line segments into an offscreen texture (the window's format) with a `Depth32Float` buffer, and one putting that texture on screen; `gpu::installed()` says whether it ran.
- **Drawing:** `plot.mesh` projects on the CPU as before (positions to the view's normalised device coordinates, depth from the eye to [0, 1], shading per triangle) and, when installed, hands the triangles and outlines to a `MeshCallback` instead of sorting them for the painter: `prepare` (re)creates the view's texture at its size in pixels and renders with depth testing, `paint` blits it into egui's pass. Colours are converted to linear for sRGB targets. Outlines sit just in front of their faces. Headless, and without the feature, the egui path draws the same triangles.
- **Checked:** by screenshots of the app (`--screenshot`) under Xvfb with a software driver, a 3D refined hexahedral mesh and a 2D refining triangle mesh; a unit test of the colour conversion. The GPU path is not exercised by `cargo test`, which runs without a window.
- **Step 4 of ADR 0085 is done.** Not yet: the heat map, surface and 3D plots on the GPU (deferred until a model needs them; see STATUS).

**Cuts and scatters of rows (2026-11-13).**
- **Cuts:** `plot.mesh` over a 3D mesh offers a cut in its tile (none, x, y, z, or any normal set by three numbers, and a slider over the points' extent along the normal), saved as its selection `{"cut": {"normal": [nx, ny, nz], "at": c}}`: the plane n̂·p = c, n̂ the normal made unit length (since 2026-11-13, at the owner's request: any direction; the first form named an axis). `mesh::section` slices each cell crossed by the plane along its edges (from its sides) into a convex polygon, ordered around its centre, drawn in the plane: across along the coordinate axis least along the normal made perpendicular to it, up along the normal across that (x and y for a normal along z, exactly, and the other two coordinates for any axis). A value over the cells fills it, one over the points is interpolated along the edges. A corner on the plane to within rounding (1e-12 of the cell's size) counts as above it, so a plane along cells' sides draws each side once (the cell below). The section is drawn as a 2D mesh, by the GPU in the window. An oblique plane is a view setting, not a pick of one of ADR 0081 point 5's axes.
- **Rows in `plot.xy`:** its slots take quantities over a table's rows; cell by cell, each is paired with `x`'s row of the same id (NaN where it has none). The "Cell by cell" preset stays for lines, so a model with a mesh still opens on its mesh.
- **Tested:** sections of two hexahedra (two squares of area 1 at z = 0.3; a plane along their shared side drawn once; none outside; the plane x + y = 1 a rectangle 1 by √2 in the first and nothing in the second, which it touches along an edge; a zero normal draws nothing) and of a cube of six tetrahedra (area 1 exactly to 1e-12, a value over the points interpolated to the plane's height); headless renders of two cuts differing, and of a scatter of rows. Every example unchanged byte for byte.

