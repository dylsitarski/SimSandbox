"""Typical use of the package, for `mypy --strict` in test_stubs.py (it is
type-checked, not run). Each line should check without an ignore: if it
needs one, a stub or an annotation in simsandbox/__init__.py is wrong."""

from __future__ import annotations

import os
from pathlib import Path

import numpy as np
from numpy.typing import NDArray

import simsandbox as ss
from simsandbox import _native


def build() -> ss.Model:
    m = ss.Model()
    m.add_block("decay", "eq.equations", "rk4", equations="dx/dt = -k*x", params={"k": 0.5, "x0": 1.0})
    m.add_block("gain", "eq.equations", equations=["input u", "y = 2*u"])
    m.connect("decay.x", "gain.u").set_run(dt=0.01, t_end=2.0)
    m.set("decay.k", 0.25)
    m.declare("size", 40.0, "m", range=(1.0, 100.0))
    m.add_probe("p", ["decay.x"], every={"steps": 10}, to="p.csv")
    return m


def use(m: ss.Model, folder: Path) -> float:
    report: dict[str, object] = ss.check(m)
    ok = bool(report["ok"])
    k: float = m.get("decay.k")
    params = m.params()
    names: list[str] = [str(p["key"]) for p in params]
    r = m.run(t_end=1.0, signals=["decay.x"], every=2)
    total = 0.0
    if isinstance(r, ss.Results):
        t: NDArray[np.float64] = r.t
        x = r.outputs["decay.x"]
        y = r["gain.y"]
        total += len(y)
        total += float(np.sum(x)) + float(t[-1])
        axes: list[tuple[str | None, int | str]] = r.axes("decay.x")
        total += len(axes) + len(r.warnings) + len(r.notes)
    else:
        total += r.tick
    probes: ss.ProbeRun = m.run_probes(t_end=1.0, save=folder)
    files: list[str] = probes.files
    p: ss.Probe = probes["p"]
    total += len(p) + len(files)
    png: bytes = m.render(diagram=True, width=400, height=300)
    image: bytes = m.render(t_end=0.5, inputs=[(0.1, "decay.k", 0.5)])
    later: ss.Model = m.start_from(0.5)
    notes: list[str] = later.start_notes
    upgraded, conversions = ss.upgrade(m.to_json())
    total += len(png) + len(image) + len(notes) + len(conversions) + len(upgraded.data)
    run = ss.Run(m, track=[(0.2, "decay.k", 1.0)], name="faster")
    replayed: ss.Results = run.replay()
    total += replayed.rows + len(names) + k + ok
    return total


def native(text: str, folder: str | os.PathLike[str]) -> int:
    raw = _native.run(text, t_end=1.0, track=[(0.1, "decay.k", 0.5)])
    rows: int = raw["rows"]
    names = [name for name, _shape, _data in raw["outputs"]]
    problem: str | None = _native.connection_problem(text, "decay.x", "gain.u")
    previous: Path | None = _native.set_folder(folder)
    _native.set_folder(previous)
    examples: list[tuple[str, str, str | None]] = _native.examples()
    image, warnings = _native.render(text, view=0, light=True)
    note: str | None = _native.version_note(_native.made_with())
    try:
        _native.check(text)
    except _native.ModelError as e:
        names.append(str(e))
    return rows + len(names) + len(examples) + len(image) + len(warnings) + (problem is None) + (note is None)


def describe() -> list[str]:
    catalogue = ss.describe()
    return [str(t["type"]) for t in catalogue["block_types"]] + ss.examples()
