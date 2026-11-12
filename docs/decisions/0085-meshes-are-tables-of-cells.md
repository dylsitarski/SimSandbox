# 0085 — Meshes are tables of cells over tables of points, in any dimension

- **Status:** Accepted (owner, 2026-11-09)
- **Date:** 2026-11-09
- **Amends:** [ADR 0079](0079-tables-of-members.md) stages 3 and 4 (cells split into four; AMR in two dimensions), [ADR 0078](0078-adaptive-studies.md) point 3 (refinement over two parameters by "the sides of cells")
- **Builds on:** [ADR 0079](0079-tables-of-members.md) stages 1–2 (tables with ids, edge tables, `split` and `merge` with transfers), [ADR 0081](0081-shapes-are-lists-of-axes.md) (rows ports, views by axis roles)

## Context

ADR 0079 planned its stage 3 as "`split` of cells into 4": quadrilaterals in two dimensions. Asked to build it, the owner asked on 2026-11-09 for meshing in general: refinement in two dimensions not tied to quadrilaterals, and three-dimensional meshes planned for from the start.

**Who needs meshes:**
- **Studies over two or more parameters** (ADR 0078): refining where a result changes, in as many dimensions as there are parameters.
- **Adaptive mesh refinement** of fields (ADR 0079 stage 4): finite volumes on cells that split and merge.
- **Unstructured meshes from files** (Gmsh, VTK): curved and irregular domains, in 2D and 3D.
- **Later:** finite elements (values at vertices), moving meshes, coupling fields across meshes ([block contract](../architecture/block-contract.md)), and drawing meshes with wgpu ([visualization](../architecture/visualization.md)).

**What exists:** tables of rows with stable ids; `edges segs of pts`, rows that each join two rows of another table; `split segs` (a new row in the middle of an edge, the edge replaced by two) and `merge`; transfers (`copy`, `mean`, an expression); rows ports, recording by id, studies over rows. An edge table is already a mesh of segments: a one-dimensional mesh.

**How others represent meshes:**
- **By dimension and incidence.** PETSc's DMPlex keeps points of every dimension (vertices, edges, faces, cells) and each point's boundary (its "cone"), the same code for any dimension and any cell shape.
- **By cells as corner lists.** Gmsh, VTK and most file formats store each cell as an ordered list of its corners, with a shape that fixes the order. Faces and neighbours are derived.
- **Refinement families.** Simplices (segments, triangles, tetrahedra, d-simplices) refine without hanging nodes by newest-vertex bisection (Maubach; Stevenson), in any dimension, with a bounded loss of shape quality, provided the starting mesh is labelled so (a box cut into simplices by the Kuhn construction is). Boxes (quadrilaterals, hexahedra, d-boxes) refine by cutting into 2^d children, leaving hanging nodes where a large cell meets small ones (p4est's forests of octrees; deal.II). MFEM does both.

Constraints: the same bits on any machine (ADR 0077, ADR 0082); stable ids; snapshots, rewind, probes and run files keep working; fixed grids keep their fast stencils; the fewest concepts.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Quadtrees and octrees only (boxes split in 2^d) | Simple; exact geometry from integer places; good for structured AMR and parameter boxes | Boxes only: no triangles, no curved or irregular domains, no meshes from files |
| Simplices only, refined by bisection | Any dimension (studies over d parameters); no hanging nodes; files of triangles and tetrahedra | No quadrilaterals or hexahedra, which much CFD and many files use |
| Points re-triangulated each round (Delaunay) | Points added freely; no refinement rules | Delaunay is costly beyond three dimensions; cells change wholesale, so no transfer of cell values, no AMR |
| **Cells of any listed shape as corner lists over a table of points, sides derived; refinement for simplices (bisection) and boxes (2^d), both from the first build** | One structure for 1D, 2D, 3D and d-dimensional parameter spaces; triangles, quads, tetrahedra, hexahedra, prisms; reads Gmsh and VTK; ADR 0079's edge tables are its 1D case | Two refinement families to build and verify; geometry functions per shape; hanging sides for boxes |
| A full incidence structure (every dimension's entities stored, as DMPlex) | Most general (polyhedra, face-based finite volumes) | Edges and faces of a 3D mesh stored and kept in step on every change; more than any planned model needs now |

## Proposal

**1. Points.** A mesh's vertices are an ordinary table; their coordinates are states (or parameters) over it: `table pts`, `state x[pts]`, `state y[pts]`. A moving mesh changes them like any state.

**2. Cells.** `cells tri of pts: triangle` declares a table whose rows are cells of one shape, each the ordered list of its corners (ids of `pts`), in the shape's reference order as Gmsh writes it. Shapes: `segment`, `triangle`, `quad`, `tetra`, `hexa`, `prism`, `pyramid`, and for parameter spaces of any dimension `simplex d` and `box d`. A model may have several cell tables (a mixed mesh is one per shape). ADR 0079's `edges segs of pts` is `cells segs of pts: segment`, kept as a spelling.

**3. Sides, kept with stable ids.** `sides f of tri` declares the faces between cells (the (d−1)-dimensional sides): SimSandbox finds them from the corner lists, keeps each side's one or two cells (`inner`, `outer`; one for a boundary side), and keeps them up to date incrementally when cells change: a side that is still there keeps its row and id, a side cut by a refinement becomes new rows, a side that is gone is removed (each change a new version). Where a large box meets two small ones, each small side is its own row (a hanging side). Other entities (edges of a 3D mesh, cells described by their faces for general polyhedra) are not stored yet; they come as further tables on the same incremental machinery when a model needs them, without changing model files.

**4. Geometry**, from the points' coordinates, for any shape: `corner(x, tri, k)`, `centre(x, tri)` (mean of the corners), `measure(tri, x, y)` (length, area or volume), and for sides `normal(f, x, y)` and `across(T, f)` (a cell value on the other side). Exact in a fixed order (ADR 0077).

**5. Refinement**, between steps as rules (ADR 0079 point 3):
- `split tri where c` refines each cell where `c` holds by its shape's rule: simplices by newest-vertex bisection (one new point at the middle of the refinement edge; neighbours bisected as needed so no hanging nodes remain; the closure is fixed by ids, so the same cells split on any machine); boxes into 2^d children (new points at the middles of edges, faces and the centre; hanging sides allowed, at most one level apart across a side).
- `merge tri where c` undoes a split where all its children agree, using the parents SimSandbox keeps.
- **Transfers** (ADR 0079 point 4): cell values `copy`, `share` (by measure: conserved exactly), `mean` and `sum` on merge; point values for new points `mean` of the edge's ends (linear interpolation), or an expression.
- Shapes other than simplices and boxes (prisms, pyramids, general polygons) can be read, drawn and computed on, but not refined, until a model needs it.

**6. Starting meshes:** `start tri = grid x 0 .. 1 by 8, y 0 .. 1 by 8` (a box grid cut into simplices by the Kuhn construction, labelled for bisection; for boxes, the grid's boxes); `start tri = file "part.msh"` (Gmsh 4, or VTK unstructured), points and cells together; `start segs = chain` as now.

**7. Studies over d parameters:** a strategy holds the points as a `simplex d` mesh over parameter space, a study with `"design": "rows"` runs each new point, and `split` refines cells whose corners' results differ. Each new point runs once. This replaces ADR 0078's plan for two parameters, and works for three or more.

**8. Ports, recording and views:** cells, sides and points are tables, so they are rows ports, recorded by id and written by `run` as now (ADR 0079 stages 1b–1c). A mesh port carries its points' coordinates and the corner lists, so a view or a downstream block can draw or rebuild it. Views of meshes (filled cells in 2D, surfaces and cuts in 3D) come with ADR 0081 stage 3 and wgpu.

**Order of work** (replacing ADR 0079 stages 3 and 4):
1. Cell tables by corners with their shapes; derived sides; geometry functions; starting grids (Kuhn simplices, boxes) and Gmsh files. Verified: measures and normals exact on known meshes; sides of a mixed 3D mesh read from a file.
2. Refinement: newest-vertex bisection for simplices in any dimension (conforming), boxes into 2^d (hanging sides), merge, transfers. Verified: no hanging nodes after bisection; `share` conserves exactly; refinement of a study over two and three parameters around a known boundary (a circle, a sphere).
3. Time on meshes (ADR 0079 stage 4): finite volumes with sums over a cell's sides in rates; AMR on the heat equation and Gray–Scott against a fine fixed grid; a 3D case.
4. Views of meshes, with ADR 0081 stage 3.

## Decision

Accepted by the owner on 2026-11-09, with these choices:
- **Both refinement families** (simplices by bisection, boxes by 2^d split) in the first build.
- **Boxes at most one level apart across a side** (2:1 balance).
- **Corner lists, with sides kept incrementally with stable ids** (point 3): the middle path between deriving sides afresh (new ids at every change) and storing every dimension's entities now (every split re-linking edges and faces of every shape). The owner asked what keeping a full incidence structure in step would cost: for one tetrahedron bisected, a new point, two cells, the cut edge in halves and a new interior edge, two faces into four and a new interior face, and the same for every neighbour sharing the cut edge, exact for every shape and both rules, with snapshots and memory growing by the edges and faces stored. Stable side ids are what cannot be added cheaply later, so they come now; the other levels come later as tables, without changing model files.

## As built

**Step 1a (2026-11-09): cells, sides, geometry and starting grids.** `crates/simsandbox-eq/src/mesh.rs` (shapes, starting grids, sides, geometry as expressions) and the typed-equations parser.
- **Declarations:** `cells c of pts: triangle` (`[max N]` before the colon), shapes `segment`, `triangle`, `quad`, `tetra`, `hexa`, `prism`, `pyramid`, `simplex d` and `box d` (d up to 6); `sides f of c`; `edges segs of pts` is a table of segments, as before. A kernel table of cells holds its corners' ids (`c0`, …; edges keep `from` and `to`) before any states over it; one of sides holds its corners, `inner` and `outer` (−1 on the boundary).
- **Values on cells, sides and edges:** states over them (and inputs, matched by id) are allowed now; edges held none in ADR 0079 stage 2. A split edge's new half copies the edge's own values.
- **Starting grids:** `start c = grid x 0 .. 2 by 3, y 0 .. 1 by 2` makes the points (the first coordinate fastest; their coordinate states set by the grid, which must be declared over the points) and the cells: boxes in the shape's corner order, or each box cut into d! simplices by the Kuhn construction, permutations in lexicographic order. The points' table takes no `start` of its own then.
- **Sides with stable ids** (point 3): found from the corner lists in cell order, each cell's sides in its shape's order; kept after every step's rules: a side still there keeps its row and id (its cells updated), one gone is removed, a new one added. Prisms and pyramids have sides of two shapes: their sides come later.
- **Geometry**, written as expressions of corners (`corner(T, c, k)` reads T at corner k by `ArrayOp::Take`): `centre(T, c)`; `measure(c, x[, y[, z]])` (a point 1, lengths, areas by cross products or the shoelace formula, volumes by determinants; hexahedra, prisms and pyramids as tetrahedra about one diagonal, exact for flat faces; an axis-aligned `box d` the product of its extents; simplices above 3 dimensions later); for sides `normal(f, k, x, y[, z])`, component k of the unit normal oriented from the inner cell's centre outwards, `inner(T, f)`, `outer(T, f)` (NaN on the boundary) and `boundary(f)`. A model of formulas may read a mesh that no rule changes.
- **Not yet:** refinement of cells (step 2); the side ids of a mesh made again by a refinement rule (step 2).
- **Tested** (`crates/simsandbox-session/tests/meshes.rs`): segments, triangles, quads, tetrahedra, hexahedra, `simplex 2` and `box 3` on grids give their exact measures, side counts and boundary sides, and the divergence theorem along every coordinate (the boundary sum of x_k n_k |side| equals the volume), with the boundary closed; removing cells keeps every remaining side's id and place; values on cells and sides step as maps, and removing a point takes its cells and their sides; mistakes explained. Every example unchanged byte for byte.

**Step 1b (2026-11-12): meshes from Gmsh files.** `mesh::read_gmsh` and the parser's `start c = file "part.msh" at x, y, z`.
- **Format:** Gmsh's ASCII `.msh`, versions 4.1 and 2.2; first-order elements (segments, triangles, quads, tetrahedra, hexahedra, prisms, pyramids; corner order as Gmsh writes it); other elements skipped. A binary file is refused, saying to save it as ASCII (`Mesh.Binary = 0`). VTK files come later, when a model needs them.
- **Points and cells:** the points are the nodes the file's cells of the declared shapes use, in file order; their coordinates set the states named after `at` (declared over the points). Several cell tables over the same points may start from one file, one per shape (a mixed mesh). A relative path is from the model's folder; the file is read when the model is first built (a recompile keeps the rows it has), and its bytes are part of the run hash (`block.mesh:PATH` in the data digests), so a changed mesh is a changed model.
- **Tested** (`meshes.rs`, `mesh.rs`): a unit cube of five tetrahedra in 4.1 (volume 1, 16 sides of which 12 on the boundary, the divergence theorem exact); a quad and two triangles over shared points in 2.2 (6 points, area 2, ∫x dA of the triangles 1.5); the run hash changes with the file; a missing file, a file without the shape and a binary file explained. Every example unchanged byte for byte.

**Step 2 (2026-11-12): refinement.** `crates/simsandbox-eq/src/refine.rs` and the typed-equations rules.
- **Rules:** `split c where cond: u = share, x = mean` and `merge c where cond: u = sum` on a table of simplices (`triangle`, `tetra`, `simplex d`) or boxes (`quad`, `hexa`, `box d`); segments keep ADR 0079's edge split and merge. Each cell where the condition holds is refined once per step, in row order.
- **Simplices** by newest-vertex bisection (Maubach): a cell at level ℓ with corners (x0, …, xd) has tag k = d − (ℓ mod d) and refinement edge x0–xk; its children are (x0, …, xk−1, z, xk+1, …, xd) and (x1, …, xk, z, xk+1, …, xd), z the edge's middle. Every cell sharing that edge is bisected with it, after its own refinement edge is made the same by refining it first (the closure), so no hanging point is left. Grids cut by the Kuhn construction are labelled for this; triangles from a file are labelled by their longest side; tetrahedra and higher simplices from a file are refused for now.
- **Boxes** into 2^d children; a new point is made once, at the middle of the part of the parent it lies on (edge, face or centre), and reused by a neighbour split later. A neighbour across a side one level coarser is split first, so neighbours across a side are at most one level apart (2:1). A small cell's side on a large cell's side is a side of its own (inner the small cell, outer the large one); the large cell's whole side is then not a side.
- **Merge** gives back the parent where all its children hold the condition: for simplices only where every cell around the split's new point is a child of a split there (that point then goes), for boxes only where no neighbour would be two levels finer. One level per step. The points it frees go unless other cells or edges use them.
- **Ids and what is kept:** the first child keeps its parent's row and id, so a split then a merge gives the cell back. A refined cell table holds `level`, `parent` and `child` after its corners, and its parents are kept in a table of the block's own, `c~parents` (listed by `describe`), so snapshots, rewind and run files keep them.
- **Transfers:** a cell's state on split is `copy` (the default) or `share` (the parent's value over the number of children, exact: the sum is kept); on merge `mean` (the default), `sum` or `copy` (the first child's); a new point's state is `mean` (the default: the mean of the two points it lies between, exact for midpoints) or an expression over the cells (the value of the cell that makes it). Expressions for a cell's state are over the cells (both children take the parent's value). Merge leaves the points' states as they are.
- **Not yet:** prisms and pyramids; simplices of three or more dimensions from a file (they need a labelling); balance across edges and corners of 3D boxes (only across sides).
- **Tested** (`refine.rs`, `tests/refinement.rs`, `study.rs`): bisection conforming in 2, 3 and 4 dimensions (every side two cells' or on the boundary) with exact shares; triangles, tetrahedra, quads, hexahedra and `box 3` refined near a point then merged back to the starting mesh, with the divergence theorem exact every step (hanging sides included) and a shared value equal to each cell's measure; boxes balanced 2:1 with exact volumes; merge undoing splits to the same ids; triangles from a file; rewind; mistakes explained; a study over two parameters (375 runs) and three (2000 runs) refined around a circle and a sphere, each point run once, every cell across the boundary small. Every example unchanged byte for byte.

**Step 3 (2026-11-12): time on meshes.** Typed equations, the expression VM (`simsandbox-expr`) and the tests in `tests/finite_volumes.rs`.
- **Continuous states over rows:** `du/dt = …` over cells (or any table) is integrated by the block's method (`rk4`, `trbdf2`, …) with the tables fixed within each step; the rules run after the step, as for maps (ADR 0079 point 6). A new structure (any table's version changed) starts the method's workspace afresh, so an implicit method learns the new Jacobian's structure. A model of formulas alone cannot have a table that changes.
- **Sums over a cell's sides:** `outflow(F, f)` is, for each cell, the sum of `F` over the sides where it is the inner cell minus the sum where it is the outer one: the net outflow of a flux written per side, oriented as `normal`. It is `ArrayOp::Scatter` (the reverse of `Take`) at the sides' inner and outer places, added in side order, so the same bits on any machine. Hanging sides count for the small cell and the large one.
- **The value across a side:** `across(u, f)` is the outer cell's value, or on the boundary the inner cell's own: a flux `(inner(u, f) − across(u, f))·|side|/gap` is then zero on the boundary (insulated, as for grids), and other boundary conditions are written with `boundary(f)`.
- **Sparse Jacobians:** dependency analysis reads the places of corners and sides from the registers after a run (`Program::dependencies_at`): each value taken or scattered depends only on what is at its place, so an implicit method's Jacobian is as sparse as the mesh.
- **Starting values from the mesh:** an initial value may read a starting mesh's coordinates, directly or through `centre`, `measure` or `corner` of cells (`state u[c] = cos(pi*centre(x, c))`).
- **Not yet:** fluxes that are consistent on hanging sides and on meshes whose centres' joins are not normal to the sides (the two-point flux is first order there); meshes that move; views of meshes (step 4). Coordinates are states (with `dx/dt = 0` when fixed), which an implicit method carries as unknowns.
- **Tested** (`finite_volumes.rs`, `program.rs`): the heat equation's slowest mode on quads (8², 16², 32²) and hexahedra (4³, 8³) decays as exp(−dπ²t), second order, with heat kept to 1e-12; a bump of heat on quads refined two levels where it is (at most 613 cells) is within 14% (energy) and 12% (second moment) of a 32² grid's error from the coarse grid's, on hexahedra one level (288 of 512 cells) within 22% and 11%, heat kept exactly through splits and merges; Gray–Scott on quads refined where it reacts (364 of 1024 cells) is within 0.05% and 0.1% of a fine grid where the coarse grid is 10% and 15% off; rewinding a refining mesh gives the same bits; the scatter's sums and exact dependencies; mistakes explained. Every example unchanged byte for byte.

## Consequences

- One structure serves studies over any number of parameters, AMR, and meshes from files, in 1D, 2D and 3D; the edge tables of ADR 0079 stage 2 are its first case and keep working.
- Refinement over two parameters (the last of ADR 0078) waits for steps 1–2 here instead of a quadrilateral split.
- Fixed grids keep their stencils; a grid can later be read as a mesh where a model needs both.
- **Concepts added:** none at the top level: cells and sides are tables (**Members**), as edges are. The new words are the `cells … of …: shape` and `sides … of …` declarations, the shape names, and the geometry functions.
