# 0045 — Units in typed equations; explicit conversion blocks

- **Status:** Accepted; the meaning of a unit at the end of a `dx/dt` line amended by [0049](0049-state-declarations-and-rate-units.md)
- **Date:** 2026-10-23

## Context

Typed equations ([ADR 0017](0017-typed-equations.md)) were dimensionless: their ports, parameters and states had unit `1`. Built-in blocks have SI units (`eq.coupled_oscillator`'s `neighbor_x` is in metres), and a connection needs compatible units, so a typed output could not drive them (the owner, 2026-10-23: "a typed equation output is non-dimensional while a coupled oscillator block accepts only units of meters"). The kernel also converted between scaled units of one dimension silently at connect time (`cm` into `m`), so a conversion never showed on the diagram.

## Options considered

| Question | Options | Chosen |
| --- | --- | --- |
| Where units are written | A `units:` section per block; **brackets where a name appears**; units in the side panel only | Brackets (owner): `input F [N]`, `param m [kg] = 1000`, `dv/dt = (F - c*v)/m [m/s]`; the unit sits beside the name it belongs to and lives in the equations text |
| What is checked | Only ports; **every equation**, names without a unit dimensionless | Every equation (owner), but a name without a unit is dimensionless, since many models have dimensionless parameters |
| Converting units | Silently at connect time (as before); **a conversion block** | A block (owner: "very explicit"): `units.convert` is the only conversion, so it is always visible |

## Decision

- **Syntax.** A unit is written in square brackets after a name, with whitespace before `[`: `input F [N], v [m/s]` (inputs with units are separated by commas), `param NAME [unit] = value` (a parameter with its default, the unit optional), and a trailing `[unit]` at the end of an equation gives the unit of the name it defines (`dx/dt = v [m]` gives `x` metres). Unit texts are those of `Unit::parse` (SI names and the scaled ones in `units.rs`, joined by `*` and `/` with `^n`). `[n]` and `[n+1]` keep meaning a map's step.
- **Checking.** Once any line of a block gives a unit, every line is checked: sums and differences, `min`, `max` and `atan2` need equal dimensions; products and quotients combine them; a power needs a constant whole (or half, for even dimensions) exponent unless its base is dimensionless; `sqrt` halves even dimensions; other functions need dimensionless arguments. A name without a unit is dimensionless; a defined quantity without one takes the dimension of its right side; `t` is in seconds; `x0` has `x`'s unit. A line that does not balance is an error at its line and column, naming both units ("dv/dt is in m·s^-2 but the right side is in m"). A block with no unit anywhere is not checked and behaves exactly as before, so older files are unchanged.
- **Evaluation.** Values are computed in SI: each name's value is scaled to SI where it is read and the result scaled back to the defined name's unit, so `param A [mm] = 50` stays 50 in the panel and in the file.
- **Connections need the same unit.** `Unit::same_as`: equal dimension and scale. `cm` into `m` is now refused like `kg` into `m`, with a message pointing to `units.convert`. No built-in block has a scaled unit, so no existing file changes meaning.
- **`units.convert`** (in `simsandbox-eq`): settings `from` and `to` (unit texts, dimensionless if absent), input `u` in `from`, output `y` in `to`, `y = factor·u` with direct feedthrough; method `exact`. `from` and `to` must measure the same kind of quantity (a conversion is not a computation; use typed equations for that). Displaying a signal in another unit is done the same way: convert it and plot the converted signal.
- **File format** (additive, [ADR 0044](0044-unsaved-changes-and-older-files.md)): blocks gain an optional `settings` object of text values; types list their settings in `describe`, and unknown settings are errors at `blocks[i].settings.KEY`. `method` may be left out: a type's first method, or for `eq.equations` the one its equations call for (`evaluate` for formulas, `iterate` for maps, `rk4` for states).
- **Interfaces** ([ADR 0039](0039-ai-agent-access.md)): units are in the equations text, so the CLI and Python see them through `check` (each parameter's `unit`) and `describe`; Python `Model.add_block(..., settings={...})`; the app's side panel lists conversion blocks with editable `from` and `to`, and the diagram shows each as `from → to`.

## Consequences

- Typed blocks can drive and be driven by built-in blocks; the units example drives springs in metres from a shaker in millimetres through a visible conversion.
- Equation errors now include unit mistakes, including `sin(t)` (time has units); a frequency parameter in `Hz` fixes it.
- Declared model and subsystem parameters ([ADR 0043](0043-subsystems.md)) and their formulas still carry descriptive units only.
- Displaying in other units means a conversion block per signal; a display-only unit choice on views can come later if that gets tedious.
