"""SimSandbox from Python: describe, check, build, run and render models
without the window (ADR 0038, ADR 0039).

A model is its file: :class:`Model` holds the model file as a dict
(``model.data``) that you may edit directly or through the helpers, and
every call sends it to SimSandbox as JSON. The same file gives the same
numbers in the app, the CLI and here.

>>> import simsandbox as ss
>>> m = ss.Model.example("Lorenz system")
>>> m.set("lorenz.rho", 14.0)
>>> r = m.run(t_end=5.0)
>>> r.t.shape, r["lorenz.xyz"].shape
((501,), (501, 3))
"""

from __future__ import annotations

import copy
import json
import os
from typing import Any, Iterable

import numpy as np

from . import _native
from ._native import ModelError

__version__ = _native.__version__
__all__ = [
    "Model",
    "Results",
    "ModelError",
    "describe",
    "schema",
    "check",
    "examples",
    "upgrade",
]


def describe(type_name: str | None = None) -> dict:
    """Every block, view and filter type SimSandbox has (with ports, units,
    parameters, methods and their speed and accuracy labels), or one type
    by name (e.g. ``"eq.equations"``, ``"plot.heatmap"``); ``"units"``
    gives every unit symbol and prefix files accept."""
    return json.loads(_native.describe(type_name))


def schema() -> dict:
    """The model file's JSON Schema."""
    return json.loads(_native.schema())


def check(model: "Model | dict | str | os.PathLike") -> dict:
    """Check a model (a :class:`Model`, a dict, JSON text or a path).
    Returns the report: ``ok``, ``errors`` (each with a ``path`` such as
    ``blocks[0].method``, or ``line`` and ``column``), ``warnings``,
    ``params``, what the model records and its views. Never raises for a
    bad model: read ``ok``."""
    return json.loads(_native.check(_text(model)))


def upgrade(model: "Model | dict | str | os.PathLike") -> tuple["Model", list[str]]:
    """The model in the current form of the file format, and the older
    forms it was converted from (ADR 0044). Files in older forms open
    anyway; this writes them as SimSandbox writes files today."""
    text, conversions, _warnings = _native.upgrade(_text(model))
    return Model.from_json(text), conversions


def examples() -> list[str]:
    """Names of the built-in example models, for :meth:`Model.example`."""
    return [name for name, _ in _native.examples()]


def _text(model: "Model | dict | str | os.PathLike") -> str:
    if isinstance(model, Model):
        return model.to_json()
    if isinstance(model, dict):
        return json.dumps(model)
    if isinstance(model, os.PathLike) or (
        isinstance(model, str) and not model.lstrip().startswith("{")
    ):
        with open(model, encoding="utf-8") as f:
            return f.read()
    return model


class Model:
    """A model file as a dict, with helpers to build and edit it."""

    def __init__(self, data: dict | None = None):
        self.data: dict = (
            copy.deepcopy(data)
            if data is not None
            else {"format": 1, "blocks": [], "connections": []}
        )

    # Opening and saving

    @classmethod
    def load(cls, path: str | os.PathLike) -> "Model":
        with open(path, encoding="utf-8") as f:
            return cls.from_json(f.read())

    @classmethod
    def from_json(cls, text: str) -> "Model":
        return cls(json.loads(text))

    @classmethod
    def example(cls, name: str) -> "Model":
        """A built-in example, by its name or a unique part of it
        (``"grazing"``)."""
        found = [
            (n, t)
            for n, t in _native.examples()
            if n == name or name.lower() in n.lower()
        ]
        exact = [f for f in found if f[0] == name]
        if len(exact) == 1 or len(found) == 1:
            return cls.from_json((exact or found)[0][1])
        raise KeyError(f"{name!r} matches {[n for n, _ in found]}; examples: {examples()}")

    def to_json(self) -> str:
        return json.dumps(self.data, indent=2)

    def save(self, path: str | os.PathLike) -> None:
        with open(path, "w", encoding="utf-8") as f:
            f.write(self.to_json() + "\n")

    def copy(self) -> "Model":
        return Model(self.data)

    # Building

    def add_block(
        self,
        name: str,
        type: str,
        method: str | dict | None = None,
        equations: Iterable[str] | str | None = None,
        params: dict[str, float] | None = None,
        update: str | None = None,
        canvas: tuple[float, float] | None = None,
        settings: dict[str, str] | None = None,
    ) -> "Model":
        """Add a block. ``method`` is a name (``"rk4"``) or a dict with its
        options (``{"name": "dopri5", "rtol": 1e-9}``); without one, the
        type's first method (see :func:`describe`), or for ``eq.equations``
        the one its equations call for. ``equations`` are the
        lines of an ``eq.equations`` block (a string is split into
        lines). ``canvas`` places it on the node canvas (laid out automatically
        if absent). ``settings`` are a type's text settings, e.g.
        ``{"from": "mm", "to": "m"}`` for ``units.convert``."""
        if any(b["name"] == name for b in self.data.get("blocks", [])):
            raise ModelError(f"there is already a block named {name!r}")
        block: dict[str, Any] = {"name": name, "type": type}
        if method is not None:
            block["method"] = {"name": method} if isinstance(method, str) else dict(method)
        if equations is not None:
            block["equations"] = (
                equations.strip().splitlines()
                if isinstance(equations, str)
                else list(equations)
            )
        if params:
            block["params"] = {k: float(v) for k, v in params.items()}
        if update is not None:
            block["update"] = update
        if canvas is not None:
            block["canvas"] = [float(canvas[0]), float(canvas[1])]
        if settings:
            block["settings"] = {k: str(v) for k, v in settings.items()}
        self.data.setdefault("blocks", []).append(block)
        return self

    def connect(self, source: str, target: str) -> "Model":
        """Connect an output to an input: ``connect("car.v", "cruise.v")``."""
        self.data.setdefault("connections", []).append({"from": source, "to": target})
        return self

    def block(self, name: str) -> dict:
        for b in self.data.get("blocks", []):
            if b["name"] == name:
                return b
        raise KeyError(f"no block named {name!r}")

    # Parameters

    def set(self, key: str, value: "float | str") -> "Model":
        """Set a parameter: ``"block.param"``, a subsystem use's
        ``"instance.param"``, or a declared ``"name"``. A block parameter
        may be a formula of declared parameters (``"2*size"``)."""
        if key in self.data.get("params", {}):
            self.data["params"][key]["value"] = float(value)
            return self
        block, param = _split(key)
        v = value if isinstance(value, str) else float(value)
        self.block(block).setdefault("params", {})[param] = v
        return self

    def get(self, key: str) -> float:
        """A parameter's value: as set in the file, else the type's
        default."""
        for p in self.params():
            if p["key"] == key:
                return p["value"]
        raise KeyError(f"no parameter {key!r}")

    def params(self) -> list[dict]:
        """Every parameter, with its value, unit, suggested range,
        description and whether it is an initial condition."""
        report = self.check()
        if not report["ok"]:
            raise ModelError(_errors(report))
        return report["params"]

    def sweep(self, *params: tuple[str, float, float, int], design: str = "grid") -> "Model":
        """Evaluate the model at many parameter values instead of in time
        (a model with no states; ADR 0027): each of ``params`` is
        ``(key, start, stop, points)``."""
        self.data["sample"] = {
            "design": design,
            "params": [
                {"param": k, "range": [float(a), float(b)], "points": int(n)}
                for k, a, b, n in params
            ],
        }
        return self

    def set_run(self, dt: float | None = None, t_end: float | None = None, seed: int | None = None) -> "Model":
        run = self.data.setdefault("run", {})
        for key, value in (("dt", dt), ("t_end", t_end), ("seed", seed)):
            if value is not None:
                run[key] = value
        return self

    # Using it

    def check(self) -> dict:
        """See :func:`check`."""
        return check(self)

    def run(self, t_end: float | None = None, dt: float | None = None) -> "Results":
        """Run to ``t_end`` in steps of ``dt`` (each defaulting to the file's
        ``run`` section), or through the model's sweep. Raises
        :class:`ModelError` with every problem :func:`check` finds if it
        cannot run."""
        text = self.to_json()
        try:
            raw = _native.run(text, t_end, dt)
        except ModelError as e:
            report = json.loads(_native.check(text))
            raise ModelError(_errors(report) if not report["ok"] else str(e)) from None
        return Results(raw)

    def declare(
        self,
        name: str,
        value: float,
        unit: str = "",
        description: str = "",
        range: tuple[float, float] | None = None,
    ) -> "Model":
        """Declare a parameter of the model itself, which block parameters
        can use in formulas (ADR 0043): ``m.declare("size", 40, "m")`` then
        ``m.set("grass.size", "size")``."""
        d: dict[str, Any] = {"value": float(value)}
        if unit:
            d["unit"] = unit
        if description:
            d["description"] = description
        if range is not None:
            d["range"] = [float(range[0]), float(range[1])]
        self.data.setdefault("params", {})[name] = d
        return self

    def group(self, blocks: Iterable[str], subsystem: str | None = None) -> str:
        """Move ``blocks`` (of the model, or inside ``subsystem``) into a new
        subsystem used in their place, as "Make subsystem" does on the
        canvas: wires that cross become its ports. Returns a note naming
        it."""
        text, note = _native.group(self.to_json(), list(blocks), subsystem)
        self.data = json.loads(text)
        return note

    def promote(self, filter: str, to: str | None = None) -> str:
        """Turn the expression filter ``filter`` into a formula block that
        runs with the model (ADR 0050): an input for each output it reads,
        in that output's unit, wired from it. Views and filters that read
        the filter then read the block. With ``to`` (``"car.F"``), its
        output is also connected there; a filter itself can never feed
        the model. Returns a note naming the block."""
        text, note = _native.promote(self.to_json(), filter, to)
        self.data = json.loads(text)
        return note

    def start_from(self, t: float) -> "Model":
        """Run to ``t`` and return the model that starts from the state
        reached there, at time 0 (ADR 0042): e.g. a steady state from which
        to study a transient. Initial-condition parameters (``x0`` for a
        state ``x``) take the values where they can express the state;
        other blocks get a ``start`` entry with their state and tables.
        The notes say which (``.start_notes``)."""
        text, notes = _native.start_from(self.to_json(), float(t))
        m = Model.from_json(text)
        m.start_notes = notes
        return m

    def render(
        self,
        path: str | os.PathLike | None = None,
        t_end: float | None = None,
        view: int | None = None,
        width: float = 1200,
        height: float = 800,
        scale: float = 1.0,
        light: bool = False,
        diagram: bool = False,
        subsystem: str | None = None,
        data: bool = True,
        values: bool = False,
    ) -> bytes:
        """Run to ``t_end`` and draw the model's views (``controls.views``,
        or ``view`` alone) as a PNG, as the app shows them; with
        ``diagram``, draw its block diagram instead (no run needed), or
        with ``subsystem`` too, that subsystem's inside. The model's diagram
        includes its filters and views (ADR 0050) unless ``data`` is false.
        A diagram shows the values of the nodes the file opens
        (``controls.open``), or of every node with ``values``. Writes it to ``path`` if given; returns the PNG bytes."""
        if subsystem is not None and not diagram:
            raise ValueError("subsystem draws a subsystem's diagram: add diagram=True")
        if values and not diagram:
            raise ValueError("values shows values on a diagram: add diagram=True")
        if diagram:
            if t_end is not None or view is not None:
                raise ValueError("t_end and view do not apply to a diagram")
            image = _native.render_diagram(
                self.to_json(),
                float(width),
                float(height),
                float(scale),
                light,
                subsystem,
                data,
                values,
            )
            warnings = []
        else:
            image, warnings = _native.render(
                self.to_json(), t_end, view, float(width), float(height), float(scale), light
            )
        for w in warnings:
            import warnings as _w

            _w.warn(w, stacklevel=2)
        if path is not None:
            with open(path, "wb") as f:
                f.write(image)
        return image

    def __repr__(self) -> str:
        names = [b["name"] for b in self.data.get("blocks", [])]
        return f"Model(blocks={names})"


def _split(key: str) -> tuple[str, str]:
    block, dot, rest = key.partition(".")
    if not dot or not rest:
        raise KeyError(f"{key!r} is not 'block.name'")
    return block, rest


def _errors(report: dict) -> str:
    lines = []
    for e in report["errors"]:
        where = e.get("path") or (
            f"line {e['line']}, column {e['column']}" if "line" in e else ""
        )
        lines.append(f"{where}: {e['message']}" if where else e["message"])
    return "; ".join(lines)


class Results:
    """What a run produced, as numpy arrays.

    - ``index``: ``t``, or for a sweep ``sample`` and each swept parameter.
    - ``outputs``: every fixed-size output by ``"block.port"``: shape
      ``(rows,)`` for a scalar, ``(rows, n)`` for a vector,
      ``(rows, ny, nx)`` for a grid, ``(rows, count, dim)`` for points.
    - ``populations``: every population output: a list over rows of
      ``(n, dim)`` arrays (n varies).
    - ``labels`` (speed and accuracy), ``notes`` (algebraic loops) and
      ``warnings``.

    ``results[name]`` looks in the index, then the outputs, then the
    populations.
    """

    def __init__(self, raw: dict):
        self.rows: int = raw["rows"]
        self.index = {
            name: _array(data, (self.rows,)) for name, _, data in raw["index"]
        }
        self.outputs: dict[str, np.ndarray] = {}
        self.shapes: dict[str, dict] = {}
        for name, shape, data in raw["outputs"]:
            shape = json.loads(shape)
            self.shapes[name] = shape
            self.outputs[name] = _array(data, (self.rows, *_dims(shape)))
        self.populations: dict[str, list[np.ndarray]] = {
            name: [_array(f, (-1, dim)) for f in frames]
            for name, dim, frames in raw["populations"]
        }
        self.labels: dict | None = json.loads(raw["labels"])
        self.notes: list[str] = raw["notes"]
        self.warnings: list[str] = raw["warnings"]

    @property
    def t(self) -> np.ndarray:
        """Times of a timed run."""
        return self.index["t"]

    def __getitem__(self, name: str):
        for d in (self.index, self.outputs, self.populations):
            if name in d:
                return d[name]
        raise KeyError(f"{name!r}; outputs: {list(self.outputs)}")

    def __contains__(self, name: str) -> bool:
        return name in self.index or name in self.outputs or name in self.populations

    def __repr__(self) -> str:
        return (
            f"Results(rows={self.rows}, index={list(self.index)}, "
            f"outputs={list(self.outputs)}, populations={list(self.populations)})"
        )


def _dims(shape: dict) -> tuple[int, ...]:
    kind = shape["kind"]
    if kind == "scalar":
        return ()
    if kind == "vector":
        return (shape["len"],)
    if kind == "grid":
        return (shape["ny"], shape["nx"])
    if kind == "points":
        return (shape["count"], shape["dim"])
    raise ValueError(f"unexpected shape {shape}")


def _array(data: bytes, shape: tuple[int, ...]) -> np.ndarray:
    a = np.frombuffer(data, dtype="<f8").reshape(shape)
    a.flags.writeable = False
    return a
