# 0030 — Filters feed views; native visualization grows with need

- **Status:** Superseded by 0032
- **Date:** 2026-10-18

## Context

Views have been absorbing data transformation: the heat map works out coordinates and resamples, the profile cuts and interpolates ([ADR 0029](0029-profile-cuts.md)), sweeps reshape samples into grids ([ADR 0028](0028-sweeps-as-samples.md)). Each view grows complicated and a transformation cannot feed anything else (a cut cannot be followed by an FFT). Pipeline tools such as ParaView separate sources, filters and representations. The owner asked whether SimSandbox is going about visualization the wrong way and whether native visualization is worth the effort; the live loop is central for the owner, including capable 3D views and renders ([Q24](../open-questions.md#resolved)).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Lightweight filters: named transformations producing new signals or fields that any view can show; views only draw | Views stay simple; transformations compose; one place for derived data | Filters need a home in files and the UI |
| A full node-editor pipeline (ParaView style) | Fully general | Heavy for hobbyists; competes with the model canvas |
| Keep transformations inside views | Nothing to change | Views keep growing; nothing composes |
| Rely on an external viewer (e.g. Rerun) | Less to build | Owner judged Rerun too narrow; external tools are not part of the live loop |

## Decision

- **Data flows sources → filters → views.** Sources are the recorded outputs (and swept parameters). **Filters** are named, ordered entries in the model's `controls`, each turning existing signals or fields into new ones: derived expressions (today's `controls.signals` become the first filter type), cuts and slices ([ADR 0031](0031-cut-filters.md)), reshaping a sweep into grids, and later FFT, smoothing, Poincaré sections and reductions. A filter's output is an ordinary signal or field, with coordinates where it has them ([ADR 0024](0024-heat-map-axes-and-coordinates.md)), so any view or later filter can use it.
- **Views only draw.** Transformations move out of views into filters; a view's own state is limited to presentation (bindings, style, camera, swap, interactive picks such as where to place a cut, which edit the filter).
- **The UI is a list, not a node graph:** a "Data" panel lists sources and filters in order (like ParaView's pipeline browser, flat), each filter with its settings. Filters change only what is shown, never results, like the rest of `controls` ([ADR 0012](0012-controls-in-model-files.md)); filters that approximate (interpolation, FFT windows) have method choices ([ADR 0020](0020-methods-for-approximations.md)).
- **Native visualization is the live loop's, and is not capped.** Its scope grows with production needs, including capable 3D views and rendering (meshes, surfaces, volumes, lighting) through wgpu ([ADR 0008](0008-native-egui-ui.md)), tested on software drivers. Each capability is added when a model or workflow needs it, with the verification and labelling rules that apply to numerical code where relevant.
- **Export complements, never replaces,** native views: CSV today, VTK and HDF5 later, so ParaView or Python can take over deep post-processing.
- No external viewer is embedded; Rerun was considered and judged too narrow by the owner.

## Consequences

- `controls.signals` is read and converted into derived-expression filters; the view contract loses the transformation code that moves into filters.
- The cut work of [ADR 0029](0029-profile-cuts.md) becomes a filter ([ADR 0031](0031-cut-filters.md)); the profile view plots line fields against their coordinates.
- Sweeps ([ADR 0028](0028-sweeps-as-samples.md)) record samples; reshaping into grids is a filter, so the surface and contour views read grids from it.
- [Visualization architecture](../architecture/visualization.md) gains the filter layer when built.
