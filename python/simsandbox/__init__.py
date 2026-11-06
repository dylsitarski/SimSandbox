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

import contextlib
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
    "Probe",
    "ProbeRun",
    "Results",
    "Composed",
    "Run",
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
    with _in_folder(_folder(model)):
        return json.loads(_native.check(_text(model)))


def upgrade(model: "Model | dict | str | os.PathLike") -> tuple["Model", list[str]]:
    """The model in the current form of the file format, and the older
    forms it was converted from (ADR 0044). Files in older forms open
    anyway; this writes them as SimSandbox writes files today."""
    with _in_folder(_folder(model)):
        text, conversions, _warnings = _native.upgrade(_text(model))
    return Model.from_json(text), conversions


def examples() -> list[str]:
    """Names of the built-in example models, for :meth:`Model.example`."""
    return [name for name, _ in _native.examples()]


@contextlib.contextmanager
def _in_folder(folder: "str | None"):
    # Blocks that read files (``input.file``, ADR 0063) read them from the
    # model file's folder.
    previous = _native.set_folder(folder)
    try:
        yield
    finally:
        _native.set_folder(previous)


def _folder(model: "Model | dict | str | os.PathLike") -> "str | None":
    if isinstance(model, Model):
        return model.folder
    if isinstance(model, os.PathLike) or (
        isinstance(model, str) and not model.lstrip().startswith("{")
    ):
        return os.path.dirname(os.path.abspath(model))
    return None


def _model_hash(model: "Model") -> str:
    with _in_folder(model.folder):
        return _native.model_hash(model.to_json())


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
    """A model file as a dict, with helpers to build and edit it.

    ``folder`` is where blocks read the files their settings name
    (``input.file``, ADR 0063): the folder the model was loaded from or
    saved to, else ``None`` (only absolute file names then)."""

    def __init__(self, data: dict | None = None, folder: "str | os.PathLike | None" = None):
        self.data: dict = (
            copy.deepcopy(data)
            if data is not None
            else {"format": 1, "blocks": [], "connections": []}
        )
        self.folder: str | None = None if folder is None else os.fspath(folder)

    # Opening and saving

    @classmethod
    def load(cls, path: str | os.PathLike) -> "Model":
        with open(path, encoding="utf-8") as f:
            m = cls.from_json(f.read())
        m.folder = os.path.dirname(os.path.abspath(path))
        return m

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
        self.folder = os.path.dirname(os.path.abspath(path))

    def copy(self) -> "Model":
        return Model(self.data, self.folder)

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
        """Connect an output to an input, ``connect("car.v", "cruise.v")``,
        or to a parameter, which the output then drives in place of its
        value (ADR 0075): ``connect("clock.k", "decay.k")``, a used
        model's ``"car1/mass"`` or a study's ``"curve.m"``."""
        self.data.setdefault("connections", []).append({"from": source, "to": target})
        return self

    def block(self, name: str) -> dict:
        for b in self.data.get("blocks", []):
            if b["name"] == name:
                return b
        raise KeyError(f"no block named {name!r}")

    # Parameters

    def set(self, key: str, value: "float | str") -> "Model":
        """Set a parameter: ``"block.param"``, a used model's
        ``"use.param"``, or a declared ``"name"``. A block parameter
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

    def run(
        self,
        t_end: float | None = None,
        dt: float | None = None,
        inputs: "Iterable[tuple[float, str, float]] | Run | None" = None,
        signals: "Iterable[str] | None" = None,
        every: int = 1,
    ) -> "Results | Composed":
        """Run to ``t_end`` in steps of ``dt`` (each defaulting to the file's
        ``run`` section). A model whose only runs are its studies'
        (ADR 0071) gives a row per run. ``signals`` keeps only those
        outputs (``"block.port"``, ``"block"`` for all its ports; in a
        model using models also a model's name, ``"."`` for the top
        level, or a path in one, ``"plant/car.v"``), and ``every`` only
        every so many steps and the last (ADR 0076): less memory and
        time, the same values. ``inputs`` are
        changes made during the run (ADR 0058): ``(t, key, value)``
        tuples, each applied at the first step boundary at or after ``t``
        along with the file's schedule (``run.inputs``); or a :class:`Run`,
        whose schedule and recorded input are applied to this model.
        Raises :class:`ModelError` with every problem :func:`check` finds
        if it cannot run. A model that uses model files (ADR 0066) runs each
        on its own clock and returns :class:`Composed`; given ``inputs``, it
        runs as one model on its clocks, as the app does, and returns
        :class:`Results` at every instant of any clock (keys by full name,
        ``"slow/u.y"``)."""
        # A use names a model and has no type; a study names one too (ADR 0070).
        uses = any(("file" in b or "model" in b) and "type" not in b for b in self.data.get("blocks", []))
        if isinstance(signals, str):
            signals = [signals]
        sig = None if signals is None else [str(s) for s in signals]
        if int(every) < 1:
            raise ValueError("every must be 1 or more steps")
        every = int(every)
        if uses and inputs is None:
            if dt is not None:
                raise ValueError("a model using models keeps each model's own clock: dt does not apply")
            try:
                with _in_folder(self.folder):
                    return Composed(_native.run_composition(self.to_json(), t_end, sig, every))
            except ModelError as e:
                with _in_folder(self.folder):
                    report = json.loads(_native.check(self.to_json()))
                raise ModelError(_errors(report) if not report["ok"] else str(e)) from None
        model, track = self._with_inputs(inputs)
        text = model.to_json()
        try:
            with _in_folder(self.folder):
                raw = _native.run(text, t_end, dt, track, sig, every)
        except ModelError as e:
            with _in_folder(self.folder):
                report = json.loads(_native.check(text))
            raise ModelError(_errors(report) if not report["ok"] else str(e)) from None
        return Results(raw)

    def _with_inputs(self, inputs) -> "tuple[Model, list[tuple[float, str, float]]]":
        if isinstance(inputs, Run):
            model = self.copy()
            model.data.setdefault("run", {})["inputs"] = copy.deepcopy(inputs.inputs)
            return model, list(inputs.track)
        if inputs is not None:
            return self, [(float(t), str(k), float(v)) for t, k, v in inputs]
        return self, []

    def add_model(
        self,
        name: str,
        file: "str | os.PathLike | None" = None,
        params: dict[str, float | str] | None = None,
        dt: float | None = None,
        shared: bool = False,
        files: "dict[str, str | os.PathLike] | None" = None,
        model: str | None = None,
    ) -> "Model":
        """Use a model file in this model (ADR 0066), under ``name``, which
        prefixes everything in it (``"plant/car.v"``). ``file`` is found
        from this model's folder (or is a full path) and may itself use
        model files. ``params`` set its parameter values for this use only,
        by path at any depth (``"car.m"``, ``"plant/car.m"``), each a number
        or a formula of this model's declared parameters (``"2*mass"``);
        it runs on its own clock, every ``dt`` (the file's ``run.dt`` if not
        given); link it to other models with :meth:`link`. With ``shared``,
        it runs on this model's clock instead, flattened into it like a
        subsystem: :meth:`connect` wires straight into its input blocks
        (``"plant/force"``) and from its blocks' outputs (``"plant/car.v"``).
        ``files`` replaces the files of models used inside it, by path
        (``{"plant": "car-2.ssbx.json"}``), found from this model's folder;
        only this use sees them, and the used file is left alone.
        ``model`` names a model defined in this file (:meth:`define`) in
        place of ``file``; without ``dt`` or ``shared`` it runs on its own
        clock if it writes ``run.dt``, else on this model's when wired
        (ADR 0068)."""
        if shared and dt is not None:
            raise ValueError("a use on a shared clock takes this model's step: no dt")
        if (file is None) == (model is None):
            raise ValueError("a use names a file or a model defined here, one of the two")
        use: dict = {"name": name}
        if file is not None:
            use["file"] = os.fspath(file)
        else:
            use["model"] = model
        if params:
            use["params"] = {k: v if isinstance(v, str) else float(v) for k, v in params.items()}
        if files:
            use["files"] = {k: os.fspath(v) for k, v in files.items()}
        if dt is not None:
            use["clock"] = {"dt": float(dt)}
        if shared:
            use["clock"] = "shared"
        self.data.setdefault("blocks", []).append(use)
        return self

    def define(self, name: str, model: "Model | dict") -> "Model":
        """Define a model in this file under ``name`` (ADR 0068), used by
        :meth:`add_model` with ``model=name``: written like a model file,
        its input blocks are its inputs and its declared ``outputs`` its
        outputs. Without ``run.dt`` it shares the clock of what uses it."""
        data = dict(model.data if isinstance(model, Model) else model)
        data.pop("format", None)
        self.data.setdefault("models", {})[name] = data
        return self

    def link(self, signal: str, to: str, every: dict | None = None, probe: str | None = None) -> "Model":
        """Link two models on their own clocks (ADR 0066): a probe records
        ``signal`` (``"plant/car.v"``) ``every`` as :meth:`add_probe` takes
        it (every step if not given), and drives the input block ``to``
        (``"controller/speed"``, an ``input.value``) with its samples, held
        between them."""
        name = probe or "link_" + signal.replace("/", "_").replace(".", "_")
        self.add_probe(name, [signal], every=every)
        self.data.setdefault("connections", []).append(
            {"from": f"probe:{name}.{signal}", "to": to}
        )
        return self

    def add_probe(
        self,
        name: str,
        record: Iterable[str],
        every: dict | None = None,
        keep: dict | None = None,
        to: str | None = None,
    ) -> "Model":
        """Add a probe (ADR 0062): it records the signals and fields in
        ``record`` (names as views bind them, ``"car.v"``), ``every``
        ``{"steps": n}``, ``{"seconds": dt}`` or ``{"times": [...]}``
        (every step if not given), and ``keep``s ``{"mb": size}``,
        ``{"last": n}`` or ``{"all": True}`` (thinned when long if not
        given). ``to`` names the CSV file it writes beside the model, with
        a ``.npy`` file per grid or point field."""
        probe: dict = {"name": name, "record": list(record)}
        if every is not None:
            probe["every"] = dict(every)
        if keep is not None:
            probe["keep"] = dict(keep)
        if to is not None:
            probe["to"] = to
        self.data.setdefault("controls", {}).setdefault("probes", []).append(probe)
        return self

    def run_probes(
        self,
        t_end: float | None = None,
        dt: float | None = None,
        inputs: "Iterable[tuple[float, str, float]] | Run | None" = None,
        probes: Iterable[str] | None = None,
        save: "str | os.PathLike | None" = None,
    ) -> "ProbeRun":
        """Run as the app does and return what the probes recorded
        (ADR 0062): all of them, or those named in ``probes``. ``t_end``,
        ``dt`` and ``inputs`` are as for :meth:`run`. With ``save``, the
        probes that have a file (``to``) write it into that directory."""
        model, track = self._with_inputs(inputs)
        text = model.to_json()
        names = None if probes is None else list(probes)
        try:
            with _in_folder(self.folder):
                raw = _native.run_probes(
                    text, t_end, dt, track, names, None if save is None else os.fspath(save)
                )
        except ModelError as e:
            with _in_folder(self.folder):
                report = json.loads(_native.check(text))
            raise ModelError(_errors(report) if not report["ok"] else str(e)) from None
        return ProbeRun(raw)

    def schedule(self, t: float, key: str, value: float) -> "Model":
        """Change ``key`` to ``value`` at time ``t`` in every run: the
        model's default schedule (``run.inputs``, ADR 0058). For recorded
        or one-off runs, use :class:`Run` instead."""
        self.data.setdefault("run", {}).setdefault("inputs", []).append(
            {"t": float(t), "set": key, "to": float(value)}
        )
        return self

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

    def duplicate(self, blocks: Iterable[str]) -> str:
        """Copy ``blocks`` (blocks or uses of models) beside themselves, with
        their values and the wires among them, as "Duplicate" does on the
        canvas (ADR 0069); the copies are named like ``car4``. Returns a
        note naming them."""
        text, note = _native.duplicate(self.to_json(), list(blocks))
        self.data = json.loads(text)
        return note

    def study(self, use: str) -> str:
        """Add a study block beside the used model ``use`` (ADR 0070), as
        "Study this model" does on the canvas: it runs the same model at 20
        values of its first parameter, its inputs fed as the use's are.
        Change its ``sample`` in ``data`` to study something else. Returns a
        note naming it."""
        text, note = _native.study(self.to_json(), use)
        self.data = json.loads(text)
        return note

    def as_study(self, *params: tuple[str, float, float, int], design: str = "grid", sample: dict | None = None) -> str:
        """Study the whole model (ADR 0071), as "Study this model" does on
        the canvas background: its blocks move into a model defined in the
        file, used in their place, with a study of it beside. Each of
        ``params`` is ``(key, start, stop, points)``, keys as inside the
        model (``"spring.w"``); or give the whole ``sample``. Without
        either, the study is a grid of 20 points over the model's first
        parameter. Returns a note naming the study block."""
        text, note = _native.study_model(self.to_json())
        self.data = json.loads(text)
        if params:
            sample = {
                "design": design,
                "params": [
                    {"param": k, "range": [float(a), float(b)], "points": int(n)}
                    for k, a, b, n in params
                ],
            }
        if sample is not None:
            study = next(b for b in reversed(self.data["blocks"]) if b.get("type") == "study")
            study["sample"] = sample
        return note

    def group(self, blocks: Iterable[str]) -> str:
        """Move ``blocks`` into a new model defined in this file (ADR 0068)
        and use it in their place, as "Make model" does on the canvas:
        wires that cross become its input blocks and declared outputs.
        Returns a note naming it."""
        text, note = _native.group(self.to_json(), list(blocks))
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
        with _in_folder(self.folder):
            text, notes = _native.start_from(self.to_json(), float(t))
        m = Model.from_json(text)
        m.folder = self.folder
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
        inside: str | None = None,
        data: bool = True,
        values: bool = False,
        expand: "list[str] | None" = None,
    ) -> bytes:
        """Run to ``t_end`` and draw the model's views (``controls.views``,
        or ``view`` alone) as a PNG, as the app shows them; with
        ``diagram``, draw its block diagram instead (no run needed), or
        with ``inside`` too, the inside of the model used at that path
        (``"plant"``, ``"vehicle/plant"``). ``expand`` lists uses drawn
        expanded in place, by path, one level each (ADR 0066), besides those
        the file lists in ``controls.expand``. The model's diagram
        includes its filters and views (ADR 0050) unless ``data`` is false.
        A diagram shows the values of the nodes the file opens
        (``controls.open``), or of every node with ``values``. Writes it to ``path`` if given; returns the PNG bytes."""
        if inside is not None and not diagram:
            raise ValueError("inside draws a used model's diagram: add diagram=True")
        if expand and not diagram:
            raise ValueError("expand draws uses in place on a diagram: add diagram=True")
        if values and not diagram:
            raise ValueError("values shows values on a diagram: add diagram=True")
        if diagram:
            if t_end is not None or view is not None:
                raise ValueError("t_end and view do not apply to a diagram")
            with _in_folder(self.folder):
                image = _native.render_diagram(
                    self.to_json(),
                    float(width),
                    float(height),
                    float(scale),
                    light,
                    inside,
                    data,
                    values,
                    list(expand or []),
                )
            warnings = []
        else:
            with _in_folder(self.folder):
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


class Run:
    """One run of a model (ADR 0059): the changes applied in it (a schedule
    and recorded input), the seed and settings used, and the model it was
    made with. Saved beside the model as ``<model>.<run>.run.json``;
    replaying it reproduces the run bit for bit."""

    def __init__(
        self,
        model: "Model",
        inputs: Iterable[dict] | None = None,
        track: Iterable[tuple[float, str, float]] | None = None,
        seed: int | None = None,
        dt: float | None = None,
        t_end: float | None = None,
        name: str = "",
        note: str = "",
        model_path: str | os.PathLike | None = None,
    ):
        run = model.data.get("run", {})
        self.model = model.copy()
        #: The schedule in effect: ``{"t", "set", "to"}`` dicts; the
        #: model's own if not given.
        self.inputs: list[dict] = (
            [dict(c) for c in inputs] if inputs is not None else copy.deepcopy(run.get("inputs", []))
        )
        #: Recorded input: ``(t, key, value)`` in the order it happened.
        self.track: list[tuple[float, str, float]] = [
            (float(t), str(k), float(v)) for t, k, v in (track or [])
        ]
        self.seed = seed if seed is not None else run.get("seed", 1)
        self.dt = dt if dt is not None else run.get("dt", 0.01)
        self.t_end = t_end if t_end is not None else run.get("t_end")
        self.name = name
        self.note = note
        self.created = ""
        #: Where the model file is, if known; a saved run refers to it.
        self.model_path = os.fspath(model_path) if model_path is not None else None
        #: The model changed since the run was made (set by :meth:`load`).
        self.stale = False

    @classmethod
    def load(cls, path: str | os.PathLike) -> "Run":
        """Read a run file and the model it names (relative to it). Warns
        if the model changed since the run was made."""
        with open(path, encoding="utf-8") as f:
            data = json.loads(f.read())
        if data.get("format") != 1:
            raise ModelError(f"{os.fspath(path)}: run file format {data.get('format')!r} is not 1")
        model_path = os.path.join(os.path.dirname(os.fspath(path)), data["model"])
        model = Model.load(model_path)
        settings = data.get("run", {})
        run = cls(
            model,
            inputs=data.get("inputs", []),
            track=data.get("track", []),
            seed=data.get("seed"),
            dt=settings.get("dt"),
            t_end=settings.get("t_end"),
            name=data.get("name", ""),
            note=data.get("note", ""),
            model_path=model_path,
        )
        run.created = data.get("created", "")
        with _in_folder(model.folder):
            run.stale = _native.model_hash(model.to_json()) != data["model_hash"]
        if run.stale:
            import warnings as _w

            _w.warn(
                f"{model_path} has changed since this run was recorded; replaying it anyway",
                stacklevel=2,
            )
        return run

    def replay(self, t_end: float | None = None) -> "Results":
        """Run the model with this run's seed, settings, schedule and
        recorded input."""
        m = self.model.copy()
        run = m.data.setdefault("run", {})
        run["seed"] = self.seed
        run["dt"] = self.dt
        if self.t_end is not None:
            run["t_end"] = self.t_end
        run["inputs"] = copy.deepcopy(self.inputs)
        return m.run(t_end=t_end, inputs=self.track)

    def to_json(self, model_path: str | None = None) -> str:
        """The run file's text; ``model_path`` is the model relative to
        where the run file goes."""
        target = model_path or (os.path.basename(self.model_path) if self.model_path else None)
        if target is None:
            raise ValueError("a run refers to its model file: save the model and pass model_path")
        data: dict[str, Any] = {
            "format": 1,
            "model": target,
            "model_hash": _model_hash(self.model),
        }
        if self.name:
            data["name"] = self.name
        if self.note:
            data["note"] = self.note
        import datetime

        data["created"] = self.created or datetime.datetime.now(datetime.timezone.utc).strftime(
            "%Y-%m-%dT%H:%M:%SZ"
        )
        data["seed"] = self.seed
        settings = {"dt": self.dt}
        if self.t_end is not None:
            settings["t_end"] = self.t_end
        data["run"] = settings
        if self.inputs:
            data["inputs"] = self.inputs
        if self.track:
            data["track"] = [list(e) for e in self.track]
        return json.dumps(data, indent=2)

    def save(self, path: str | os.PathLike) -> None:
        """Write the run file at ``path``. The model must be saved: the run
        refers to it by its path relative to ``path``."""
        if self.model_path is None:
            raise ValueError("save the model first and give Run(model_path=...)")
        rel = os.path.relpath(self.model_path, os.path.dirname(os.path.abspath(path)))
        with open(path, "w", encoding="utf-8") as f:
            f.write(self.to_json(rel) + "\n")


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


class Composed:
    """What a model using model files produced (ADR 0066): each model on
    its own clock as :class:`Results` at its own times, by name (``"."``
    for the top level's own blocks, else its path, ``"plant"`` or
    ``"vehicle/plant"``) in ``models`` or ``model(name)``, the common
    ``tick`` and ``warnings``. ``composed["plant/car.v"]`` finds a full
    name in the model holding it, and ``composed.t("plant")`` gives that
    model's times."""

    def __init__(self, raw: dict):
        self.models: dict[str, Results] = {name: Results(r) for name, r in raw["models"]}
        self.tick: float = raw["tick"]
        self.warnings: list[str] = list(raw["warnings"])

    def model(self, name: str) -> "Results":
        return self.models[name]

    def t(self, name: str) -> np.ndarray:
        return self.models[name]["t"]

    def __getitem__(self, name: str):
        for r in self.models.values():
            if name in r.outputs or name in r.populations:
                return r[name]
        raise KeyError(name)

    def __repr__(self) -> str:
        rows = ", ".join(f"{n}: {r.rows} rows" for n, r in self.models.items())
        return f"Composed({rows})"


class Results:
    """What a run produced, as numpy arrays.

    - ``index``: ``t``, or ``run`` for a model whose only runs are its
      studies' (ADR 0071).
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


class Probe:
    """What one probe recorded, as numpy arrays.

    - ``t``: the times of its samples.
    - ``signals``: each scalar signal, shape ``(samples,)``.
    - ``fields``: each grid ``(samples, ny, nx)`` or point set
      ``(samples, count, dim)``.
    - ``populations``: each population, a list over samples of
      ``(n, dim)`` arrays.

    ``probe[name]`` looks in ``t``, the signals, the fields, then the
    populations.
    """

    def __init__(self, raw: tuple):
        name, times, signals, fields, populations = raw
        self.name: str = name
        self.t = _array(times, (-1,))
        n = len(self.t)
        self.signals = {s: _array(data, (n,)) for s, data in signals}
        self.fields = {f: _array(data, (n, *dims)) for f, dims, data in fields}
        self.populations = {
            p: [_array(f, (-1, dim)) for f in frames] for p, dim, frames in populations
        }

    def __getitem__(self, name: str):
        if name == "t":
            return self.t
        for d in (self.signals, self.fields, self.populations):
            if name in d:
                return d[name]
        raise KeyError(f"{name!r}; recorded: {list(self.signals) + list(self.fields)}")

    def __len__(self) -> int:
        return len(self.t)

    def __repr__(self) -> str:
        return (
            f"Probe({self.name!r}, samples={len(self)}, signals={list(self.signals)}, "
            f"fields={list(self.fields)})"
        )


class ProbeRun:
    """What a run's probes recorded: ``probes`` by name (see
    :class:`Probe`), the ``files`` written, ``labels`` and ``warnings``.
    ``run[name]`` is a probe."""

    def __init__(self, raw: dict):
        self.probes: dict[str, Probe] = {p[0]: Probe(p) for p in raw["probes"]}
        self.files: list[str] = [str(f) for f in raw["files"]]
        self.labels: dict | None = json.loads(raw["labels"])
        self.warnings: list[str] = raw["warnings"]

    def __getitem__(self, name: str) -> Probe:
        if name not in self.probes:
            raise KeyError(f"{name!r}; probes: {list(self.probes)}")
        return self.probes[name]

    def __contains__(self, name: str) -> bool:
        return name in self.probes

    def __repr__(self) -> str:
        return f"ProbeRun(probes={list(self.probes)}, files={self.files})"


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
