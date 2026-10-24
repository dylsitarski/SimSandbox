# 0055 — Open nodes saved, values inside subsystems, popped-out windows drawn on their own

- **Status:** Accepted
- **Date:** 2026-10-24

## Context

After [ADR 0054](0054-controls-pane-pins-and-node-toggles.md) the owner reported (2026-10-23):

- Which nodes are open was lost on reopening ([Q32](../open-questions.md)). The owner asked to save it.
- Turning a slider off in the platoon also unpinned the value. The platoon's controls had `slider: true` and no `pin`, and a slider counted as a pin.
- The Controls had lost the range values beside their sliders.
- A popped-out tab still froze the main window, though only while the main window had focus ([Q33](../open-questions.md)). Turning vsync off had not helped.
- The owner asked whether subsystems should have value lists too. Uses on the model level already list their own parameters (`mass`). Blocks inside a subsystem showed none.

## Options considered

| Question | Options | Chosen |
| --- | --- | --- |
| Where open nodes are kept | Per user, outside the file; **in `controls`** | `controls.open`, like the arrangement of panes |
| Older sliders | Count a slider as a pin while reading the flag; **convert on reading** | A slider without a pin gains the pin when the file is read, reported as a conversion ([ADR 0044](0044-unsaved-changes-and-older-files.md)) |
| Values inside a subsystem | None (each use has its own); **the subsystem's own values, as every use's default** | Shown and edited inside the subsystem, with the uses that set their own marked |
| Pop-out | Immediate viewports, vsync off; **deferred viewports** | Deferred: each window draws on its own from the shared app |

## Decision

- **`controls.open`** lists the nodes whose values are shown: names on the model level, `subsystem::block` inside a subsystem. Unknown names are ignored. The list follows renames and deletions of blocks and subsystems. The app writes it as nodes are opened and closed, and reads it when a model opens. Headless diagrams show the nodes it lists. `render --diagram --values` (Python: `render(diagram=True, values=True)`) shows every node's values.
- **Sliders and pins.** On reading, a control with `slider: true` and no `pin` gains `pin: true`, reported as a conversion. From then on the two are independent. The examples are written in this form.
- **Range ends beside the slider.** In the Controls, a slider row has its lower end before it and its upper end after it, both typed in place.
- **Values inside a subsystem.** A block inside a subsystem shows the values its entry in the subsystem gives. These are every use's defaults: a use takes them unless it sets its own (`"car.c": 70` in the use's `params`). `*` marks a value that some uses set themselves, and the hover names them. Changing a value there changes the file (`Edit::SetParam` at that level) and restarts the run. Values per use stay in the inspector (one column per use).
- **Popped-out tabs are deferred viewports.** The app is shared between windows (`Arc<Mutex<SimApp>>`). The main window holds it while it draws, and each popped-out window holds it while drawing its tab. Actions in a popped-out window are carried out by the main window. The worker asks the main window for a frame, and the main window asks each popped-out window. Views are `Send` so the app can be shared. The vsync switch of ADR 0054 is removed.

## Consequences

- Under Xvfb both windows now draw independently, each at its own rate. Whether this fixes the freeze when the main window has focus must still be checked on the owner's desktop.
- `--screenshot` still does not finish while a tab is popped out: the screenshot event does not come back to the main window. It is only used with standard arrangements.
- Values inside a subsystem can be confused with values per use. The `*`, the hover and the restart on change keep them apart.
