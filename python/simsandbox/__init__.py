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
import functools
import json
import os
from typing import TYPE_CHECKING, Any, Iterable, Iterator, Literal, cast, overload

import numpy as np
from numpy.typing import NDArray

from . import _native
from ._native import ModelError

if TYPE_CHECKING:
    from ._native import _CompositionResults, _ProbeRecord, _ProbeResults, _RunResults

__version__ = _native.__version__
__all__ = [
    "Model",
    "Probe",
    "ProbeRun",
    "Results",
    "Rows",
    "Composed",
    "Run",
    "ModelError",
    "describe",
    "schema",
    "check",
    "explain",
    "agent_mode",
    "is_agent_mode",
    "examples",
    "upgrade",
]


def describe(type_name: str | None = None) -> dict[str, Any]:
    """Every block, view and filter type SimSandbox has (with ports, units,
    parameters, methods and their speed and accuracy labels), or one type
    by name (e.g. ``"eq.equations"``, ``"plot.heatmap"``); ``"units"``
    gives every unit symbol and prefix files accept."""
    return cast("dict[str, Any]", json.loads(_native.describe(type_name)))


def schema() -> dict[str, Any]:
    """The model file's JSON Schema."""
    return cast("dict[str, Any]", json.loads(_native.schema()))


def check(model: "Model | dict[str, Any] | str | os.PathLike[str]", agent: bool = False) -> dict[str, Any]:
    """Check a model (a :class:`Model`, a dict, JSON text or a path).
    Returns the report: ``ok``, ``errors`` (each with a ``path`` such as
    ``blocks[0].method``, or ``line`` and ``column``), ``warnings``,
    ``params``, what the model records and its views. Never raises for a
    bad model: read ``ok``.

    With ``agent=True`` (ADR 0107), also not ``ok`` where a part an agent
    made (``"agent": true``) does not explain itself: a model's
    description, a block's purpose (:meth:`Model.note`), a reason for
    each value set on it, a description for each parameter and input of
    typed equations (``# …`` after its line). Agents run this before
    handing a model over."""
    with _in_folder(_folder(model)):
        return cast("dict[str, Any]", json.loads(_native.check(_text(model), agent)))


_AGENT = os.environ.get("SIMSANDBOX_AGENT", "").strip().lower() in ("1", "true", "yes")


def agent_mode(on: bool = True) -> None:
    """Whether this program acts for an AI agent (ADR 0107). While on,
    what a :class:`Model` adds or changes is marked ``"agent": true``
    (blocks, wires, values, and the model for its settings, views and
    declarations), so a person sees what an agent made and
    ``check(model, agent=True)`` asks for its notes. Starts on if the
    environment variable ``SIMSANDBOX_AGENT`` is ``1``."""
    global _AGENT
    _AGENT = bool(on)


def is_agent_mode() -> bool:
    """Whether :func:`agent_mode` is on."""
    return _AGENT


@overload
def explain(
    model: "Model | dict[str, Any] | str | os.PathLike[str]",
    format: Literal["markdown"] = "markdown",
    part: str = "",
    depth: int = 1,
) -> str: ...
@overload
def explain(
    model: "Model | dict[str, Any] | str | os.PathLike[str]",
    format: Literal["json"],
    part: str = "",
    depth: int = 1,
) -> dict[str, Any]: ...
def explain(
    model: "Model | dict[str, Any] | str | os.PathLike[str]",
    format: str = "markdown",
    part: str = "",
    depth: int = 1,
) -> "str | dict[str, Any]":
    """The model's card (ADR 0107): what it is, its parts by role, each
    part's summary, equations or bodies, method and labels, parameters by
    role, inputs and outputs; how the parts connect; its key parameters
    (pinned, else suggested by the types); what its views show; how far
    to trust it. Markdown (default) or, with ``format="json"``, a dict.

    One level at a time (ADR 0110): each model used inside is one part,
    with its interface for that use. ``part`` gives the card of a model
    used inside, by its path of uses (``"car1"``, ``"vehicle/plant"``);
    ``depth`` includes the cards of used models that many levels down
    (1: this level only). What reads inside a used model by path is listed
    under ``reaching_inside``.

    Raises :class:`ModelError` if the model cannot load or has no such
    part."""
    if format not in ("markdown", "json"):
        raise ValueError(f"format is 'markdown' or 'json', not {format!r}")
    with _in_folder(_folder(model)):
        if depth < 1:
            raise ValueError(f"depth is a whole number from 1, not {depth!r}")
        text = _native.explain(_text(model), format == "json", part, depth)
    return cast("dict[str, Any]", json.loads(text)) if format == "json" else text


def upgrade(model: "Model | dict[str, Any] | str | os.PathLike[str]") -> tuple["Model", list[str]]:
    """The model in the current form of the file format, and the older
    forms it was converted from (ADR 0044). Files in older forms open
    anyway; this writes them as SimSandbox writes files today."""
    with _in_folder(_folder(model)):
        text, conversions, _warnings = _native.upgrade(_text(model))
    return Model.from_json(text), conversions


def examples() -> list[str]:
    """Names of the built-in example models, for :meth:`Model.example`."""
    return [name for name, _, _ in _native.examples()]


@functools.cache
def _block_types_cached() -> tuple[str, ...]:
    return tuple(t["type"] for t in describe()["block_types"])


def _block_types() -> list[str]:
    """The registered block types, as :func:`describe` lists them."""
    return list(_block_types_cached())


@contextlib.contextmanager
def _in_folder(folder: "str | None") -> Iterator[None]:
    # Blocks that read files (``input.file``, ADR 0063) read them from the
    # model file's folder.
    previous = _native.set_folder(folder)
    try:
        yield
    finally:
        _native.set_folder(previous)


def _folder(model: "Model | dict[str, Any] | str | os.PathLike[str]") -> "str | None":
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


def _text(model: "Model | dict[str, Any] | str | os.PathLike[str]") -> str:
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

    def __init__(self, data: dict[str, Any] | None = None, folder: "str | os.PathLike[str] | None" = None):
        self.data: dict[str, Any] = (
            copy.deepcopy(data)
            if data is not None
            else {"format": 1, "blocks": [], "connections": []}
        )
        self.folder: str | None = None if folder is None else os.fspath(folder)
        #: What :meth:`start_from` noted of each block, on the model it
        #: returns; empty otherwise.
        self.start_notes: list[str] = []

    # Opening and saving

    @classmethod
    def load(cls, path: str | os.PathLike[str]) -> "Model":
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
            (n, t, d)
            for n, t, d in _native.examples()
            if n == name or name.lower() in n.lower()
        ]
        exact = [f for f in found if f[0] == name]
        if len(exact) == 1 or len(found) == 1:
            # An example that reads files reads them from a folder holding
            # them (ADR 0069 step 3).
            _, text, folder = (exact or found)[0]
            return cls(json.loads(text), folder)
        raise KeyError(f"{name!r} matches {[n for n, _, _ in found]}; examples: {examples()}")

    def to_json(self) -> str:
        return json.dumps(self.data, indent=2)

    def save(self, path: str | os.PathLike[str]) -> None:
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
        method: str | dict[str, Any] | None = None,
        equations: Iterable[str] | str | None = None,
        params: dict[str, float] | None = None,
        update: str | None = None,
        canvas: tuple[float, float] | None = None,
        settings: dict[str, str] | None = None,
        bodies: Iterable[dict[str, Any]] | None = None,
    ) -> "Model":
        """Add a block. ``method`` is a name (``"rk4"``) or a dict with its
        options (``{"name": "dopri5", "rtol": 1e-9}``); without one, the
        type's first method (see :func:`describe`), or for ``eq.equations``
        the one its equations call for. ``equations`` are the
        lines of an ``eq.equations`` block (a string is split into
        lines). ``canvas`` places it on the node canvas (laid out automatically
        if absent). ``settings`` are a type's text settings, e.g.
        ``{"from": "mm", "to": "m"}`` for ``units.convert``. ``bodies`` are
        a mechanism's (``mech.planar``) rows, one dict per body and its
        joint (ADR 0105; ``describe("mech.planar")`` lists the columns),
        checked here."""
        if any(b["name"] == name for b in self.data.get("blocks", [])):
            raise ModelError(f"there is already a block named {name!r}")
        # Groups of blocks the file defines are used like types (ADR 0043).
        known = _block_types() + sorted(self.data.get("subsystems", {}))
        if type not in known:
            raise ModelError(f"unknown block type {type!r}; known: {', '.join(known)}")
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
        if _AGENT:
            block["agent"] = True
        if bodies is not None:
            if not type.startswith("mech."):
                raise ModelError(f"a {type} block has no bodies: only mechanisms (mech.*) do")
            block["bodies"] = [dict(b) for b in bodies]
            problem = _native.block_problem(json.dumps(block))
            if problem is not None:
                raise ModelError(problem)
        self.data.setdefault("blocks", []).append(block)
        return self

    def connect(self, source: str, target: str) -> "Model":
        """Connect an output to an input, ``connect("car.v", "cruise.v")``,
        or to a parameter, which the output then drives in place of its
        value (ADR 0075): ``connect("clock.k", "decay.k")``, a used
        model's ``"car1/mass"`` or a study's ``"curve.m"``.

        Raises :class:`ModelError` at once if either end names a block,
        used model, port or parameter the model does not have (add the
        blocks first), or if ``target`` is already wired."""
        for what, key in (("source", source), ("target", target)):
            if not isinstance(key, str):
                raise TypeError(f"the {what} is a name such as 'block.port', not {type(key).__name__}")
        self._not_inside(source)
        for c in self.data.get("connections", []):
            if c.get("to") == target:
                raise ModelError(
                    f"{target!r} is already wired from {c.get('from')!r}; an input or parameter takes one wire"
                )
        with _in_folder(self.folder):
            problem = _native.connection_problem(self.to_json(), source, target)
        if problem is not None:
            raise ModelError(problem)
        wire: dict[str, Any] = {"from": source, "to": target}
        if _AGENT:
            wire["agent"] = True
        self.data.setdefault("connections", []).append(wire)
        return self

    def add_scene(
        self,
        layers: Iterable[dict[str, Any]],
        frames: dict[str, dict[str, Any]] | None = None,
        unit: str | None = None,
        extent: dict[str, Any] | None = None,
        view_from: str | None = None,
        canvas: tuple[float, float] | None = None,
    ) -> "Model":
        """Add a scene view (``plot.scene``, ADR 0093) after the model's
        other views: ``layers`` of primitives drawn first to last, each
        ``{"draw": "boxes", "in": frame, "bind": {attribute: binding},
        "grab": pointer_block}``, a binding being a number, a signal
        (``"cart.x"``), a quantity over a table's rows, or for ``colour`` a
        palette name; named ``frames``, each ``{"in": parent, "x", "y",
        "angle"}`` bound the same way; the length ``unit`` (``"m"`` if not
        given); the ``extent`` shown, ``{"x": [lo, hi], "y": [lo, hi]}``
        (following what is drawn if not given); and the frame it is seen
        from. ``describe("plot.scene")`` lists the primitives and their
        attributes. ``canvas`` places its node on the canvas's data layer.

        Raises :class:`ModelError` at once if the form is wrong (an
        unknown key, primitive or attribute, a required attribute unbound,
        a frame that is not there or a cycle of frames, a bad extent), if a
        binding names no block, filter or probe of the model, or if
        ``grab`` names no ``input.pointer`` block. Whether a block has that
        output, and units, are checked by :meth:`check` and when it is
        drawn."""
        view: dict[str, Any] = {"type": "plot.scene"}
        if unit is not None:
            view["unit"] = unit
        if extent is not None:
            view["extent"] = copy.deepcopy(dict(extent))
        if frames:
            view["frames"] = copy.deepcopy(dict(frames))
        view["layers"] = copy.deepcopy(list(layers))
        if view_from is not None:
            view["view_from"] = view_from
        if canvas is not None:
            view["canvas"] = [float(canvas[0]), float(canvas[1])]
        problem = _native.scene_problem(self.to_json(), json.dumps(view))
        if problem is not None:
            raise ModelError(problem)
        self.data.setdefault("controls", {}).setdefault("views", []).append(view)
        self._marked()
        return self

    def expose(self, model: str | None, key: str, name: str | None = None) -> str:
        """Make a value inside ``model`` (a model defined in this file, or
        ``None`` for this model) a parameter of that model (ADR 0110):
        ``key`` is ``"block.param"`` or a path into a model it uses
        (``"plant/k"``, ``"plant/car.m"``). It is named ``name``, or after
        the parameter, with the value the model gives it as its default;
        returns the name, which each use then sets
        (``add_model(..., params={name: value})``). A use sets only what
        its model exposes."""
        with _in_folder(self.folder):
            text, made = _native.expose(self.to_json(), model, key, name)
        self.data = json.loads(text)
        self._marked()
        return made

    def offer(self, model: str | None, signal: str, name: str | None = None) -> str:
        """Make an output inside ``model`` (a model defined in this file, or
        ``None`` for this model) an output of that model (ADR 0110):
        ``signal`` is ``"block.port"`` or a path into a model it uses
        (``"plant/car.v"``). It is named ``name``, or after the port;
        returns the name, which a use's wires and links read as
        ``"use/name"``. Views and probes that only record may still read
        inside by path."""
        text, made = _native.offer_output(self.to_json(), model, signal, name)
        self.data = json.loads(text)
        self._marked()
        return made

    def _model_of(self, use: str) -> str | None:
        for b in self.data.get("blocks", []):
            if b.get("name") == use and _is_use(b):
                return cast("str | None", b.get("model"))
        return None

    def _not_inside(self, signal: str) -> None:
        """Refuse a signal read from inside a used model (ADR 0110)."""
        use, slash, rest = signal.partition("/")
        if not slash or ("." not in rest and "/" not in rest):
            return
        if any(b.get("name") == use and _is_use(b) for b in self.data.get("blocks", [])):
            model = self._model_of(use)
            fix = f"m.offer({model!r}, {rest!r})" if model else "offer it as an output in that model file"
            raise ModelError(
                f"{signal!r} reads inside {use!r}: what feeds a result reads a model's outputs "
                f"(ADR 0110); {fix}, then read {use}/<name>"
            )

    def define_template(
        self,
        name: str,
        view: dict[str, Any],
        inputs: dict[str, dict[str, Any]] | None = None,
        params: dict[str, dict[str, Any]] | None = None,
        offers: Iterable[str] | None = None,
        description: str | None = None,
    ) -> "Model":
        """Define a template (ADR 0113): ``view`` is a view (a scene, a
        plot) that names its ``inputs`` (``{"x": {"unit": "m"}}``; ``kind``
        ``"rows"`` or ``"pointer"`` for a grab) and ``params`` (``{"colour":
        {"value": "blue"}}``) with a ``$`` (``"$x"``) and binds no signal of
        its own; a scene may ``offer`` frames a parent draws in. Use it with
        :meth:`add_view` (``template=``) or as a scene's layer
        (``{"draw": "template", "name": ..., "template": name, "bind":
        {...}}``). Problems are reported by :meth:`check`."""
        t: dict[str, Any] = dict(copy.deepcopy(view))
        if inputs:
            t["inputs"] = copy.deepcopy(dict(inputs))
        if params:
            t["params"] = copy.deepcopy(dict(params))
        if offers:
            t["offers"] = list(offers)
        if description:
            t["description"] = description
        self.data.setdefault("templates", {})[name] = t
        self._marked()
        return self

    def add_view(
        self,
        type: str | None = None,
        bind: dict[str, "str | list[str]"] | None = None,
        template: str | None = None,
        params: dict[str, "float | str"] | None = None,
    ) -> "Model":
        """Add a view after the model's others: of a registered ``type``
        with ``bind`` binding its slots, or a ``template`` of the model
        (:meth:`define_template`) with ``bind`` binding its inputs and
        ``params`` setting its parameters (ADR 0113)."""
        if (type is None) == (template is None):
            raise ValueError("a view has a type or a template, one of the two")
        view: dict[str, Any] = {"type": type} if type is not None else {"template": template}
        if bind:
            view["bind"] = copy.deepcopy(dict(bind))
        if params:
            view["params"] = dict(params)
        self.data.setdefault("controls", {}).setdefault("views", []).append(view)
        self._marked()
        return self

    def add_views(self, file: "str | os.PathLike[str]") -> "Model":
        """Add a view file's templates and views (``*.ssbv.json``, ADR
        0113) to the model's, after its own views. :meth:`render` with
        ``views=`` draws a view file in place of the model's views without
        changing the model."""
        with open(file, encoding="utf-8") as f:
            text = _native.apply_views(self.to_json(), f.read(), False)
        self.data = json.loads(text)
        self._marked()
        return self

    def block(self, name: str) -> dict[str, Any]:
        for b in self.data.get("blocks", []):
            if b["name"] == name:
                return cast("dict[str, Any]", b)
        raise KeyError(f"no block named {name!r}")

    # Parameters

    def set(self, key: str, value: "float | str") -> "Model":
        """Set a parameter: ``"block.param"``, a used model's
        ``"use.param"``, or a declared ``"name"``. A block parameter
        may be a formula of declared parameters (``"2*size"``)."""
        if key in self.data.get("params", {}):
            self.data["params"][key]["value"] = float(value)
            self._marked()
            return self
        block, param = _split(key)
        if "/" in block:
            use = block.split("/")[0]
            raise ModelError(_reaching(use, key.split("/", 1)[1], self._model_of(use)))
        entry = self.block(block)
        if _is_use(entry) and ("." in param or "/" in param):
            raise ModelError(_reaching(block, param, entry.get("model")))
        v = value if isinstance(value, str) else float(value)
        entry.setdefault("params", {})[param] = v
        self._marked(block)
        return self

    def note(
        self,
        part: str | None = None,
        purpose: str | None = None,
        assumptions: Iterable[str] | None = None,
        reasons: dict[str, str] | None = None,
    ) -> "Model":
        """Say what a block (``part``) or, without one, the model is for,
        what it assumes and why its values were chosen (ADR 0107): shown
        on its card (:func:`explain`). A model's purpose is its
        ``description``. Agents must write these for what they make;
        ``check(model, agent=True)`` says what is missing."""
        if part is None:
            if purpose is not None:
                self.data["description"] = purpose
            notes = self.data.setdefault("notes", {})
        else:
            notes = self.block(part).setdefault("notes", {})
            if purpose is not None:
                notes["purpose"] = purpose
        if assumptions is not None:
            notes["assumptions"] = list(assumptions)
        if reasons:
            notes.setdefault("reasons", {}).update({k: str(v) for k, v in reasons.items()})
        if not notes:
            (self.data if part is None else self.block(part)).pop("notes")
        return self

    def _marked(self, block: str | None = None) -> None:
        """In :func:`agent_mode`, mark the block (or the model) as made or
        changed by an agent."""
        if _AGENT:
            (self.data if block is None else self.block(block))["agent"] = True

    def get(self, key: str) -> float:
        """A parameter's value: as set in the file, else the type's
        default."""
        for p in self.params():
            if p["key"] == key:
                return float(p["value"])
        raise KeyError(f"no parameter {key!r}")

    def params(self) -> list[dict[str, Any]]:
        """Every parameter, with its value, unit, suggested range,
        description and whether it is an initial condition."""
        report = self.check()
        if not report["ok"]:
            raise ModelError(_errors(report))
        return cast("list[dict[str, Any]]", report["params"])

    def set_run(self, dt: float | None = None, t_end: float | None = None, seed: int | None = None) -> "Model":
        run = self.data.setdefault("run", {})
        for key, value in (("dt", dt), ("t_end", t_end), ("seed", seed)):
            if value is not None:
                run[key] = value
        self._marked()
        return self

    # Using it

    def check(self, agent: bool = False) -> dict[str, Any]:
        """See :func:`check`."""
        return check(self, agent)

    @overload
    def explain(self, format: Literal["markdown"] = "markdown", part: str = "", depth: int = 1) -> str: ...
    @overload
    def explain(self, format: Literal["json"], part: str = "", depth: int = 1) -> dict[str, Any]: ...
    def explain(self, format: str = "markdown", part: str = "", depth: int = 1) -> "str | dict[str, Any]":
        """See :func:`explain`."""
        return cast("str | dict[str, Any]", explain(self, format, part, depth))  # type: ignore[call-overload]

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

    def _with_inputs(
        self, inputs: "Iterable[tuple[float, str, float]] | Run | None"
    ) -> "tuple[Model, list[tuple[float, str, float]]]":
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
        file: "str | os.PathLike[str] | None" = None,
        params: dict[str, float | str] | None = None,
        dt: float | None = None,
        shared: bool = False,
        files: "dict[str, str | os.PathLike[str]] | None" = None,
        model: str | None = None,
    ) -> "Model":
        """Use a model file in this model (ADR 0066), under ``name``, which
        prefixes everything in it (``"plant/car.v"``). ``file`` is found
        from this model's folder (or is a full path) and may itself use
        model files. ``params`` set its declared parameters for this use
        only, each a number or a formula of this model's declared parameters
        (``"2*mass"``); a value inside it is set by exposing it first
        (:meth:`expose`, ADR 0110);
        it runs on its own clock, every ``dt`` (the file's ``run.dt`` if not
        given); link it to other models with :meth:`link`. With ``shared``,
        it runs on this model's clock instead, flattened into it like a
        subsystem: :meth:`connect` wires straight into its input blocks
        (``"plant/force"``) and from the outputs its model offers
        (``"plant/v"``; :meth:`offer` offers one, ADR 0110).
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
        use: dict[str, Any] = {"name": name}
        if file is not None:
            use["file"] = os.fspath(file)
        else:
            use["model"] = model
        if params:
            for k in params:
                if "." in k or "/" in k:
                    raise ModelError(_reaching(name, k, model))
            use["params"] = {k: v if isinstance(v, str) else float(v) for k, v in params.items()}
        if files:
            use["files"] = {k: os.fspath(v) for k, v in files.items()}
        if dt is not None:
            use["clock"] = {"dt": float(dt)}
        if shared:
            use["clock"] = "shared"
        self.data.setdefault("blocks", []).append(use)
        return self

    def define(self, name: str, model: "Model | dict[str, Any]") -> "Model":
        """Define a model in this file under ``name`` (ADR 0068), used by
        :meth:`add_model` with ``model=name``: written like a model file,
        its input blocks are its inputs and its declared ``outputs`` its
        outputs. Without ``run.dt`` it shares the clock of what uses it."""
        data = dict(model.data if isinstance(model, Model) else model)
        data.pop("format", None)
        self.data.setdefault("models", {})[name] = data
        return self

    def link(self, signal: str, to: str, every: dict[str, Any] | None = None, probe: str | None = None) -> "Model":
        """Link two models on their own clocks (ADR 0066): a probe records
        ``signal`` (``"plant/car.v"``) ``every`` as :meth:`add_probe` takes
        it (every step if not given), and drives the input block ``to``
        (``"controller/speed"``, an ``input.value``) with its samples, held
        between them."""
        self._not_inside(signal)
        name = probe or "link_" + signal.replace("/", "_").replace(".", "_")
        self.add_probe(name, [signal], every=every)
        self.data.setdefault("connections", []).append(
            {"from": f"probe:{name}.{signal}", "to": to}
        )
        self._marked()
        return self

    def add_probe(
        self,
        name: str,
        record: Iterable[str],
        every: dict[str, Any] | None = None,
        keep: dict[str, Any] | None = None,
        to: str | None = None,
    ) -> "Model":
        """Add a probe (ADR 0062): it records the signals and fields in
        ``record`` (names as views bind them, ``"car.v"``), ``every``
        ``{"steps": n}``, ``{"seconds": dt}`` or ``{"times": [...]}``
        (every step if not given), and ``keep``s ``{"mb": size}``,
        ``{"last": n}`` or ``{"all": True}`` (thinned when long if not
        given). ``to`` names the CSV file it writes beside the model, with
        a ``.npy`` file per grid or point field."""
        probe: dict[str, Any] = {"name": name, "record": list(record)}
        if every is not None:
            probe["every"] = dict(every)
        if keep is not None:
            probe["keep"] = dict(keep)
        if to is not None:
            probe["to"] = to
        self.data.setdefault("controls", {}).setdefault("probes", []).append(probe)
        self._marked()
        return self

    def run_probes(
        self,
        t_end: float | None = None,
        dt: float | None = None,
        inputs: "Iterable[tuple[float, str, float]] | Run | None" = None,
        probes: Iterable[str] | None = None,
        save: "str | os.PathLike[str] | None" = None,
        memory: float | None = None,
    ) -> "ProbeRun":
        """Run as the app does and return what the probes recorded
        (ADR 0062): all of them, or those named in ``probes``. ``t_end``,
        ``dt`` and ``inputs`` are as for :meth:`run`. With ``save``, the
        probes that have a file (``to``) write it into that directory.
        ``memory`` is the app's memory budget in MB (1024 by default):
        probes that keep what they record by default thin past a quarter
        of it (ADR 0062 step 5); ``ProbeRun.memory`` says what was held."""
        model, track = self._with_inputs(inputs)
        text = model.to_json()
        names = None if probes is None else list(probes)
        try:
            with _in_folder(self.folder):
                raw = _native.run_probes(
                    text, t_end, dt, track, names, None if save is None else os.fspath(save),
                    memory,
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
        self._marked()
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
        self._marked()
        return self

    def duplicate(self, blocks: Iterable[str]) -> str:
        """Copy ``blocks`` (blocks or uses of models) beside themselves, with
        their values and the wires among them, as "Duplicate" does on the
        canvas (ADR 0069); the copies are named like ``car4``. Returns a
        note naming them."""
        text, note = _native.duplicate(self.to_json(), list(blocks))
        self.data = json.loads(text)
        return note

    def arrange(self) -> int:
        """Place every block and use on the canvas by role, as "Arrange by
        role" does (the Structure lens, ADR 0108): interaction on the left,
        the plant (systems and used models) in the middle, controllers below
        it so their loop is drawn as a loop, what only reads it to the
        right. Changes only positions (``canvas``). Returns how many moved; see
        it with ``render(diagram=True)``."""
        text, moved = _native.arrange(self.to_json())
        self.data = json.loads(text)
        return int(moved)

    def study(self, use: str) -> str:
        """Add a study block beside the used model ``use`` (ADR 0070), as
        "Study this model" does on the canvas: it runs the same model at 20
        values of its first parameter, its inputs fed as the use's are.
        Change its ``sample`` in ``data`` to study something else. Returns a
        note naming it."""
        text, note = _native.study(self.to_json(), use)
        self.data = json.loads(text)
        return note

    def as_study(self, *params: tuple[str, float, float, int], design: str = "grid", sample: dict[str, Any] | None = None) -> str:
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

    # Strategies as models (ADR 0078 point 7). Each writes a study whose
    # points come from its inputs and a strategy block, a map whose rounds
    # are its steps, wired to it, the same models as the examples. The
    # target is a study block, a used model, or None for the whole model
    # (moved into a definition the study runs). Run the model to search;
    # open the strategy block to change it. Returns a note naming it.

    def _strategy(self, target: str | None, spec: dict[str, Any]) -> str:
        text, note = _native.strategy(self.to_json(), json.dumps(spec), target)
        self.data = json.loads(text)
        return note

    @staticmethod
    def _varied(params: dict[str, Any]) -> list[dict[str, Any]]:
        """``{"car.kp": 400}`` or ``{"car.kp": {"start": 400, "span": 1000,
        "range": [1, 2000]}}`` as the strategy takes them."""
        out = []
        for key, v in params.items():
            if isinstance(v, dict):
                out.append({"param": key, **{k: v[k] for k in v}})
            else:
                out.append({"param": key, "start": float(v)})
        return out

    def minimize(
        self,
        target: str | None,
        params: dict[str, Any],
        objective: str,
        budget: float = 60,
        tol: float = 1e-3,
    ) -> str:
        """Pattern search (``examples/cruise-search.ssbx.json``): lower the
        ``objective`` (a reduce formula of each run, e.g.
        ``"first_time(abs(cruise.e) < 0.5)"``) over ``params``, each a start
        or ``{"start", "span", "range"}`` (the first step is a quarter of
        the span; values stay in the range). Each round runs the centre and
        a step either way on each parameter, moves to the best or halves
        the step, and holds still once the step is below ``tol``; at most
        ``budget`` rounds. Read ``search.best_<param>`` and
        ``search.best_score``."""
        return self._strategy(
            target,
            {"kind": "minimize", "params": self._varied(params), "objective": objective,
             "budget": budget, "tol": tol},
        )

    def fit(
        self,
        target: str | None,
        params: dict[str, Any],
        residuals: str,
        budget: float = 60,
        tol: float = 1e-9,
    ) -> str:
        """Levenberg–Marquardt least squares
        (``examples/oscillator-fit.ssbx.json``): ``params`` from their
        starts, ``residuals`` a vector per run (e.g.
        ``"at_samples(osc.x - data.x, data)"``). Holds still at the fit;
        read ``fit.fit_<param>``, ``fit.cost`` and ``fit.done``."""
        return self._strategy(
            target,
            {"kind": "fit", "params": self._varied(params), "residuals": residuals,
             "budget": budget, "tol": tol},
        )

    def refine(
        self,
        target: str | None,
        params: dict[str, Any],
        result: str,
        points: int = 9,
        change: float = 0.5,
        tol: float = 1e-3,
        max: int = 200,
        budget: float = 60,
    ) -> str:
        """Refinement (``examples/bistable-refinement.ssbx.json``) of where
        ``result`` (a reduce formula) changes, over ``params``: each key
        its range, ``{"bi.x0": (-2, 2)}``, or ``{"range": (-2, 2), "tol":
        1e-4}``. The points are a mesh of simplices over the parameters
        (segments over one), ``points`` a side to start; each round every
        cell whose corners' results differ by more than ``change`` is
        bisected, until it is narrower than ``tol`` along every parameter.
        Read ``refine.changing`` (cells still changing), over one parameter
        ``refine.lo`` and ``refine.hi`` (where it changes), and the points
        as rows (``Results.over_rows``)."""
        ranged = []
        for key, v in params.items():
            r = v if not isinstance(v, dict) else v["range"]
            entry: dict[str, Any] = {"param": key, "range": [float(r[0]), float(r[1])]}
            if isinstance(v, dict) and "tol" in v:
                entry["tol"] = float(v["tol"])
            ranged.append(entry)
        return self._strategy(
            target,
            {"kind": "refine", "params": ranged, "result": result, "points": int(points),
             "change": change, "tol": tol, "max": int(max), "budget": budget},
        )

    def continue_(
        self,
        target: str | None,
        param: str,
        range: tuple[float, float],
        step: float,
        state: str,
        settled: str,
        state_start: float = 0.0,
        budget: float = 200,
    ) -> str:
        """Natural-parameter continuation
        (``examples/bistable-continuation.ssbx.json``): one run a round,
        ``param`` stepping by ``step`` across ``range`` and back, each run
        starting from where the last settled: its starting-value parameter
        ``state`` (e.g. ``"bi.x0"``) set to ``settled`` (e.g.
        ``"final(bi.x)"``) of the last run, from ``state_start``. Plot
        ``continuation.settled`` against ``continuation.value``."""
        return self._strategy(
            target,
            {"kind": "continue", "param": param, "range": [float(range[0]), float(range[1])],
             "step": step, "state": state, "settled": settled,
             "state_start": state_start, "budget": budget},
        )

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
        path: str | os.PathLike[str] | None = None,
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
        components: bool = False,
        wired: bool = False,
        inputs: "Iterable[tuple[float, str, float]] | Run | None" = None,
        views: "str | os.PathLike[str] | None" = None,
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
        (``controls.open``), or of every node with ``values``. With
        ``components`` a diagram draws each node as its header and the wires
        between two nodes as one bundle, named by its ports' group or count
        (the Structure lens, ADR 0108); with ``wired``, each block's wired
        ports only, with how many more it has. ``inputs`` are
        changes made during the run, as for :meth:`run`: ``(t, key, value)``
        tuples or a :class:`Run` (the CLI's ``--inputs``). ``views`` names a
        view file (``*.ssbv.json``, ADR 0113) whose views are drawn in place
        of the model's (the CLI's ``--views``); the model is left as it is.
        Writes it to ``path`` if given; returns the PNG bytes."""
        if inside is not None and not diagram:
            raise ValueError("inside draws a used model's diagram: add diagram=True")
        if expand and not diagram:
            raise ValueError("expand draws uses in place on a diagram: add diagram=True")
        if values and not diagram:
            raise ValueError("values shows values on a diagram: add diagram=True")
        if components and not diagram:
            raise ValueError("components draws a diagram's components: add diagram=True")
        if wired and not diagram:
            raise ValueError("wired draws a diagram's wired ports: add diagram=True")
        if diagram:
            if t_end is not None or view is not None:
                raise ValueError("t_end and view do not apply to a diagram")
            if inputs is not None:
                raise ValueError("inputs change a run; a diagram is drawn without one")
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
                    components,
                    wired,
                )
            warnings: list[str] = []
        else:
            model, track = self._with_inputs(inputs)
            if views is not None:
                with open(views, encoding="utf-8") as f:
                    shown = _native.apply_views(model.to_json(), f.read(), True)
                model = Model.from_json(shown)
                model.folder = self.folder
            with _in_folder(self.folder):
                image, warnings = _native.render(
                    model.to_json(), t_end, view, float(width), float(height), float(scale), light, track
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
        inputs: Iterable[dict[str, Any]] | None = None,
        track: Iterable[tuple[float, str, float]] | None = None,
        seed: int | None = None,
        dt: float | None = None,
        t_end: float | None = None,
        name: str = "",
        note: str = "",
        model_path: str | os.PathLike[str] | None = None,
    ):
        run = model.data.get("run", {})
        self.model = model.copy()
        #: The schedule in effect: ``{"t", "set", "to"}`` dicts; the
        #: model's own if not given.
        self.inputs: list[dict[str, Any]] = (
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
        #: The SimSandbox that made it (``simsandbox 0.0.1``); this build's
        #: for a new run, empty for a run file written before it was
        #: recorded.
        self.made_with = ""
        #: Where the model file is, if known; a saved run refers to it.
        self.model_path = os.fspath(model_path) if model_path is not None else None
        #: The model changed since the run was made (set by :meth:`load`).
        self.stale = False

    @classmethod
    def load(cls, path: str | os.PathLike[str]) -> "Run":
        """Read a run file and the model it names (relative to it). Warns
        if the model changed since the run was made."""
        with open(path, encoding="utf-8") as f:
            text = f.read()
        # Read as the CLI and the app read it: unknown fields and a newer
        # format are refused.
        try:
            data = json.loads(_native.run_file(text))
        except ModelError as e:
            raise ModelError(f"{os.fspath(path)}: {e}") from None
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
        run.made_with = data.get("made_with", "")
        note = _native.version_note(run.made_with)
        if note is not None:
            import warnings as _w

            _w.warn(f"{os.fspath(path)}: {note}", stacklevel=2)
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
        # With inputs given, even none, a model runs as one: Results.
        return cast(Results, m.run(t_end=t_end, inputs=self.track))

    def to_json(self, model_path: str | None = None) -> str:
        """The run file's text; ``model_path`` is the model relative to
        where the run file goes. Written by the same code as the app's and
        checked as a run file is when read, so a run saved here and one
        saved in the app with the same contents are the same bytes."""
        target = model_path or (os.path.basename(self.model_path) if self.model_path else None)
        if target is None:
            raise ValueError("a run refers to its model file: save the model and pass model_path")
        data: dict[str, Any] = {
            "format": 1,
            "model": target,
            "model_hash": _model_hash(self.model),
            "made_with": self.made_with or _native.made_with(),
        }
        if self.name:
            data["name"] = self.name
        if self.note:
            data["note"] = self.note
        data["created"] = self.created or _native.utc_now()
        data["seed"] = self.seed
        settings = {"dt": self.dt}
        if self.t_end is not None:
            settings["t_end"] = self.t_end
        data["run"] = settings
        if self.inputs:
            data["inputs"] = self.inputs
        if self.track:
            data["track"] = [list(e) for e in self.track]
        return _native.run_file(json.dumps(data)).rstrip("\n")

    def save(self, path: str | os.PathLike[str]) -> None:
        """Write the run file at ``path``. The model must be saved: the run
        refers to it by its path relative to ``path``."""
        if self.model_path is None:
            raise ValueError("save the model first and give Run(model_path=...)")
        rel = os.path.relpath(self.model_path, os.path.dirname(os.path.abspath(path)))
        with open(path, "w", encoding="utf-8") as f:
            f.write(self.to_json(rel) + "\n")


def _is_use(entry: dict[str, Any]) -> bool:
    return "type" not in entry and ("model" in entry or "file" in entry)


def _reaching(use: str, key: str, model: str | None) -> str:
    """Why a use cannot set a value inside its model, and the fix."""
    fix = (
        f"m.expose({model!r}, {key!r}) declares it, then set {use}.<name>"
        if model
        else "expose it as a parameter in that model file, then set that"
    )
    return (
        f"{use} sets {key!r} inside its model: a use sets only what its model exposes "
        f"(ADR 0110); {fix}"
    )


def _split(key: str) -> tuple[str, str]:
    block, dot, rest = key.partition(".")
    if not dot or not rest:
        raise KeyError(f"{key!r} is not 'block.name'")
    return block, rest


def _errors(report: dict[str, Any]) -> str:
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

    def __init__(self, raw: "_CompositionResults"):
        self.models: dict[str, Results] = {name: Results(r) for name, r in raw["models"]}
        self.tick: float = raw["tick"]
        self.warnings: list[str] = list(raw["warnings"])

    def model(self, name: str) -> "Results":
        return self.models[name]

    def t(self, name: str) -> NDArray[np.float64]:
        return self.models[name].t

    def __getitem__(self, name: str) -> Any:
        for r in self.models.values():
            if name in r.outputs or name in r.populations or name in r.over_rows:
                return r[name]
        raise KeyError(name)

    def __repr__(self) -> str:
        rows = ", ".join(f"{n}: {r.rows} rows" for n, r in self.models.items())
        return f"Composed({rows})"


class Rows:
    """A quantity over a table's rows, followed by id (ADR 0079), at each
    row of the results (or sample of a probe).

    - ``ids``: a list over rows of int arrays, the ids of the rows there.
    - ``values``: a list over rows of arrays, ``(n,)`` (or ``(n, dim)``),
      in the order of ``ids``.
    - ``versions``: the table's structure version at each row (-1 where
      nothing was recorded); equal versions mean the same rows.

    :meth:`of` follows one row by its id; :meth:`by_id` gives every id
    seen and a matrix over them, NaN where a row is not there.
    """

    def __init__(self, name: str, dim: int, frames: list[bytes]):
        self.name = name
        self.dim = dim
        self.ids: list[NDArray[np.int64]] = []
        self.values: list[NDArray[np.float64]] = []
        versions = []
        for f in frames:
            v = _array(f, (-1,))
            if len(v) < 2:
                versions.append(-1)
                self.ids.append(np.zeros(0, dtype=np.int64))
                self.values.append(np.zeros((0,) if dim == 1 else (0, dim)))
                continue
            n = int(v[1])
            versions.append(int(v[0]))
            self.ids.append(v[2 : 2 + n].astype(np.int64))
            vals = v[2 + n :]
            self.values.append(vals if dim == 1 else vals.reshape(n, dim))
        self.versions = np.array(versions, dtype=np.int64)

    def __len__(self) -> int:
        return len(self.ids)

    def of(self, id: int) -> NDArray[np.float64]:
        """The row ``id`` at each row of the results, NaN where it is not
        there: shape ``(rows,)`` (or ``(rows, dim)``)."""
        out = np.full((len(self),) if self.dim == 1 else (len(self), self.dim), np.nan)
        for k, (ids, vals) in enumerate(zip(self.ids, self.values)):
            at = np.nonzero(ids == id)[0]
            if len(at):
                out[k] = vals[at[0]]
        return out

    def by_id(self) -> tuple[NDArray[np.int64], NDArray[np.float64]]:
        """Every id seen, increasing, and the values over them:
        ``(rows, ids)`` (or ``(rows, ids, dim)``), NaN where a row is not
        there."""
        seen = np.unique(np.concatenate(self.ids)) if self.ids else np.zeros(0, np.int64)
        shape = (len(self), len(seen)) + (() if self.dim == 1 else (self.dim,))
        out = np.full(shape, np.nan)
        for k, (ids, vals) in enumerate(zip(self.ids, self.values)):
            out[k, np.searchsorted(seen, ids)] = vals
        return seen, out

    def __repr__(self) -> str:
        return f"Rows({self.name!r}, rows={len(self)}, dim={self.dim})"


class Results:
    """What a run produced, as numpy arrays.

    - ``index``: ``t``, or ``run`` for a model whose only runs are its
      studies' (ADR 0071).
    - ``outputs``: every fixed-size output by ``"block.port"``: shape
      ``(rows,)`` for a scalar, ``(rows, n)`` for a vector,
      ``(rows, ny, nx)`` for a grid, ``(rows, count, dim)`` for points,
      ``(rows, nk, …, n1)`` for one over three or more axes (its first
      axis varying fastest, so last in the array; ADR 0081).
      :meth:`axes` names them.
    - ``populations``: every population output: a list over rows of
      ``(n, dim)`` arrays (n varies).
    - ``over_rows``: every quantity over a table's rows, as :class:`Rows`
      (ids, values and versions at each row; ADR 0079).
    - ``labels`` (speed and accuracy), ``notes`` (algebraic loops) and
      ``warnings``.

    ``results[name]`` looks in the index, then the outputs, the
    populations and the quantities over rows.
    """

    def __init__(self, raw: "_RunResults"):
        self.rows: int = raw["rows"]
        self.index = {
            name: _array(data, (self.rows,)) for name, _, data in raw["index"]
        }
        self.outputs: dict[str, NDArray[np.float64]] = {}
        self.shapes: dict[str, dict[str, Any]] = {}
        for name, shape, data in raw["outputs"]:
            shape = json.loads(shape)
            self.shapes[name] = shape
            self.outputs[name] = _array(data, (self.rows, *_dims(shape)))
        self.populations: dict[str, list[NDArray[np.float64]]] = {
            name: [_array(f, (-1, dim)) for f in frames]
            for name, dim, frames in raw["populations"]
        }
        self.over_rows: dict[str, Rows] = {
            name: Rows(name, dim, frames) for name, dim, frames in raw.get("over_rows", [])
        }
        self.labels: dict[str, Any] | None = json.loads(raw["labels"])
        self.notes: list[str] = raw["notes"]
        self.warnings: list[str] = raw["warnings"]

    @property
    def t(self) -> NDArray[np.float64]:
        """Times of a timed run."""
        return self.index["t"]

    def axes(self, name: str) -> list[tuple[str | None, int | str]]:
        """An output's axes after the rows, in the array's order: each as
        (name, length), the name ``None`` where its block gives none and
        the length ``"rows"`` for a table's rows (ADR 0081)."""
        shape = self.shapes[name]
        axes = [(a.get("name"), a["length"]) for a in shape.get("axes", [])]
        return list(reversed(axes))

    def __getitem__(self, name: str) -> Any:
        """An array, or for a population a list of arrays, or :class:`Rows`:
        the type depends on the name."""
        for d in (self.index, self.outputs, self.populations, self.over_rows):
            if name in d:
                return d[name]
        raise KeyError(f"{name!r}; outputs: {list(self.outputs)}")

    def __contains__(self, name: str) -> bool:
        return any(
            name in d for d in (self.index, self.outputs, self.populations, self.over_rows)
        )

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
    - ``over_rows``: each quantity over a table's rows, as :class:`Rows`.

    ``probe[name]`` looks in ``t``, the signals, the fields, the
    populations, then the quantities over rows.
    """

    def __init__(self, raw: "_ProbeRecord"):
        name, times, signals, fields, populations, over_rows = raw
        self.name: str = name
        self.t = _array(times, (-1,))
        n = len(self.t)
        self.signals = {s: _array(data, (n,)) for s, data in signals}
        self.fields = {f: _array(data, (n, *dims)) for f, dims, data in fields}
        self.populations = {
            p: [_array(f, (-1, dim)) for f in frames] for p, dim, frames in populations
        }
        self.over_rows = {r: Rows(r, dim, frames) for r, dim, frames in over_rows}

    def __getitem__(self, name: str) -> Any:
        if name == "t":
            return self.t
        for d in (self.signals, self.fields, self.populations, self.over_rows):
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
    :class:`Probe`), the ``files`` written, ``labels``, ``warnings`` and
    ``memory`` (bytes held at the end against the budget: ``budget``,
    ``held``, ``parts`` as [name, bytes]). ``run[name]`` is a probe."""

    def __init__(self, raw: "_ProbeResults"):
        self.probes: dict[str, Probe] = {p[0]: Probe(p) for p in raw["probes"]}
        self.files: list[str] = [str(f) for f in raw["files"]]
        self.memory: dict[str, Any] = json.loads(raw["memory"])
        self.labels: dict[str, Any] | None = json.loads(raw["labels"])
        self.warnings: list[str] = raw["warnings"]

    def __getitem__(self, name: str) -> Probe:
        if name not in self.probes:
            raise KeyError(f"{name!r}; probes: {list(self.probes)}")
        return self.probes[name]

    def __contains__(self, name: str) -> bool:
        return name in self.probes

    def __repr__(self) -> str:
        return f"ProbeRun(probes={list(self.probes)}, files={self.files})"


def _dims(shape: dict[str, Any]) -> tuple[int, ...]:
    kind = shape["kind"]
    if kind == "scalar":
        return ()
    if kind == "vector":
        return (shape["len"],)
    if kind == "grid":
        return (shape["ny"], shape["nx"])
    if kind == "points":
        return (shape["count"], shape["dim"])
    if kind == "axes":
        # Three or more axes, the first fastest: numpy's last (ADR 0081).
        return tuple(a["length"] for a in reversed(shape["axes"]))
    raise ValueError(f"unexpected shape {shape}")


def _array(data: bytes, shape: tuple[int, ...]) -> NDArray[np.float64]:
    a = np.frombuffer(data, dtype="<f8").reshape(shape)
    a.flags.writeable = False
    return a
