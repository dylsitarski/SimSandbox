# 0101 — Scene primitives are parametric meshes; shape meshes come from models

- **Status:** Accepted (owner, 2026-11-18)
- **Date:** 2026-11-18
- **Builds on:** [ADR 0093](0093-scenes-from-primitive-layers.md) (scenes of layers, frames), [ADR 0085](0085-meshes-are-tables-of-cells.md) (meshes are tables of cells), [ADR 0086](0086-views-by-axis-roles-and-mesh-views.md) (meshes drawn as triangles, on the GPU in the window)

## Context

ADR 0093 gives a scene four primitives (`points`, `segments`, `boxes`, `discs`) and lists a `mesh` layer for after the gate. Asked how meshes fit (2026-11-18), two kinds were found that nothing told apart: **simulation meshes**, the cells a model solves on, whose points and values change; and **shape meshes**, the fixed outline of a body (a part from an STL file) moved by a rigid-body engine. The owner asked why the primitives should not themselves be shape meshes, and whether that would cost speed.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Primitives drawn by their own code, meshes a separate layer with its own drawing | Each drawn its best way | Two drawing paths and two hit tests; a 3D or GPU move done twice |
| Every shape a mesh the model builds (a box is four points and a cell) | One concept | Sizes cannot follow a signal (frames do not scale); many points recorded per instance; screen-sized points and hairlines are not meshes; a table in the model made only for the picture |
| **Primitives are parametric meshes: a generator whose parameters are the bound attributes; one drawing path below every layer** | Bound sizes, few numbers recorded, exact at any zoom; one path for primitives and meshes; instancing on the GPU later (one unit mesh, a few numbers per instance) | Each primitive provides a generator, bounds and a containment test |

Shape meshes from a file read by the view were also weighed: simplest at first, but a second source of geometry outside the model, beside units, replay, `check` and the rigid-body engine's own shapes.

## Decision

1. **A primitive is a parametric mesh.** It provides, from its bound attributes for one instance: its triangles and lines at a given scale (curves split finely enough for the pixels they cover), its bounding box, and whether a point lies inside it (exact: a disc by distance, a box in its own axes). The generated geometry is never recorded or saved; only the attributes are.
2. **One drawing path.** A scene turns its layers, through their frames, into one list of triangles and lines in scene coordinates (with screen-sized parts, point size and line width, kept as such), cached while nothing it was built from changes (the recordings' stamps, the bindings, the extent, the size in pixels). Before the gate egui's painter draws the list, in the window and headless alike; later a GPU path may draw the same layers by instancing without changing them.
3. **Grabbing tests primitives, not triangles**: the topmost layer with a `grab` whose instance contains the pointer (point 1's test, through the instance's frame), never the triangle list.
4. **Shape meshes come from models.** A body's fixed outline is a table of cells in a model (typed equations already read one from a file, `start c = file …`), and a `mesh` layer (after the gate, ADR 0093) draws a mesh port in a frame: a shape mesh in its body's frame, a simulation mesh in the scene's. A view reads no geometry files of its own.
5. **Primitives and meshes match rigid-body engines**: a few exact shapes (boxes, discs, later spheres, capsules, cylinders in 3D) plus meshes for the rest, so the rigid-body spike's bodies map to layers one to one.

## Consequences

- No change to ADR 0093's file form, primitives or frames; its build follows points 1–3 here.
- The scene's drawing and hit testing are unit-tested per primitive (geometry against the attributes, containment at edges) as well as by rendered pictures.
- Speed: building triangles costs what egui's own shapes cost (egui splits shapes into triangles too); the cache makes a paused scene nearly free; thousands of instances, if a model needs them, go to GPU instancing.
- A `mesh` layer, when built, reuses `plot.mesh`'s triangle building (`crates/simsandbox-views/src/mesh.rs`).

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none. A shape mesh is a mesh (ADR 0085); a parametric mesh is how a primitive (ADR 0093) is drawn.
