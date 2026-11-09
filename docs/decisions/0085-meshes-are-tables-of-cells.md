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

**3. Sides, derived.** `sides f of tri` declares the faces between cells (the (d−1)-dimensional sides): SimSandbox finds them from the corner lists, keeps each side's one or two cells (`inner`, `outer`; one for a boundary side) and rebuilds them when cells change. Where a large box meets two small ones, each small side is its own row (a hanging side). Edges and faces are not stored otherwise.

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
- **Still open:** whether to start with corner lists and derived sides (point 2–3) and add the full incidence structure later, or store every dimension's entities from the start; the owner asked what keeping that structure in step costs.

## Consequences

- One structure serves studies over any number of parameters, AMR, and meshes from files, in 1D, 2D and 3D; the edge tables of ADR 0079 stage 2 are its first case and keep working.
- Refinement over two parameters (the last of ADR 0078) waits for steps 1–2 here instead of a quadrilateral split.
- Fixed grids keep their stencils; a grid can later be read as a mesh where a model needs both.
- **Concepts added:** none at the top level: cells and sides are tables (**Members**), as edges are. The new words are the `cells … of …: shape` and `sides … of …` declarations, the shape names, and the geometry functions.
