"""Type stubs for ``simsandbox._native``, the extension module built from
``python/src/lib.rs`` (PyO3). Keep them in step with it: every function,
argument name, default and return type. ``tests/test_stubs.py`` checks the
names and signatures against the built module.

Model files go in and reports come out as JSON text; numbers come back as
little-endian ``f64`` bytes. The ``_``-prefixed ``TypedDict`` classes exist
only here, to type the dicts the runs return.
"""

import os
from collections.abc import Sequence
from pathlib import Path
from typing import TypedDict

__all__ = [
    "ModelError",
    "describe",
    "promote",
    "set_folder",
    "run_composition",
    "schema",
    "check",
    "examples",
    "run",
    "run_probes",
    "model_hash",
    "made_with",
    "run_schema",
    "version_note",
    "utc_now",
    "run_file",
    "render",
    "render_diagram",
    "start_from",
    "group",
    "duplicate",
    "study",
    "study_model",
    "strategy",
    "upgrade",
    "connection_problem",
    "__version__",
]

__version__: str

class ModelError(ValueError):
    """A model file that cannot be loaded, run or rendered."""

# (t, key, value): a change made during a run (ADR 0058).
_Event = tuple[float, str, float]

class _RunResults(TypedDict):
    rows: int
    # (name, [rows], bytes)
    index: list[tuple[str, list[int], bytes]]
    # (name, shape as JSON, bytes)
    outputs: list[tuple[str, str, bytes]]
    # (name, dim, [bytes per row])
    populations: list[tuple[str, int, list[bytes]]]
    # (name, dim, [bytes per row]); each frame [version, n, ids…, values…]
    over_rows: list[tuple[str, int, list[bytes]]]
    # JSON text: null or the speed and accuracy labels.
    labels: str
    notes: list[str]
    warnings: list[str]

class _CompositionResults(TypedDict):
    models: list[tuple[str, _RunResults]]
    tick: float
    warnings: list[str]

# (name, times bytes, [(signal, bytes)], [(field, dims, bytes)],
#  [(population, dim, [bytes per sample])], [(over rows, dim, [bytes per sample])])
_ProbeRecord = tuple[
    str,
    bytes,
    list[tuple[str, bytes]],
    list[tuple[str, list[int], bytes]],
    list[tuple[str, int, list[bytes]]],
    list[tuple[str, int, list[bytes]]],
]

class _ProbeResults(TypedDict):
    probes: list[_ProbeRecord]
    files: list[Path]
    # JSON text: {"budget", "held", "parts"}.
    memory: str
    # JSON text: null or the speed and accuracy labels.
    labels: str
    warnings: list[str]

def describe(name: str | None = None) -> str: ...
def schema() -> str: ...
def check(text: str) -> str: ...
def model_hash(text: str) -> str: ...
def made_with() -> str: ...
def version_note(made_with: str) -> str | None: ...
def utc_now() -> str: ...
def run_file(text: str) -> str: ...
def run_schema() -> str: ...
def examples() -> list[tuple[str, str, str | None]]: ...
def run(
    text: str,
    t_end: float | None = None,
    dt: float | None = None,
    track: Sequence[_Event] | None = None,
    signals: Sequence[str] | None = None,
    every: int = 1,
) -> _RunResults: ...
def run_composition(
    text: str,
    t_end: float | None = None,
    signals: Sequence[str] | None = None,
    every: int = 1,
) -> _CompositionResults: ...
def run_probes(
    text: str,
    t_end: float | None = None,
    dt: float | None = None,
    track: Sequence[_Event] | None = None,
    names: Sequence[str] | None = None,
    save: str | os.PathLike[str] | None = None,
    memory: float | None = None,
) -> _ProbeResults: ...
def render(
    text: str,
    t_end: float | None = None,
    view: int | None = None,
    width: float = 1200.0,
    height: float = 800.0,
    scale: float = 1.0,
    light: bool = False,
    track: Sequence[_Event] | None = None,
) -> tuple[bytes, list[str]]: ...
def render_diagram(
    text: str,
    width: float = 1200.0,
    height: float = 800.0,
    scale: float = 1.0,
    light: bool = False,
    inside: str | None = None,
    data: bool = True,
    values: bool = False,
    expand: Sequence[str] = ...,
) -> bytes: ...
def start_from(text: str, t: float) -> tuple[str, list[str]]: ...
def group(text: str, blocks: Sequence[str]) -> tuple[str, str]: ...
def duplicate(text: str, blocks: Sequence[str]) -> tuple[str, str]: ...
def study(text: str, block: str) -> tuple[str, str]: ...
def strategy(text: str, spec: str, target: str | None = None) -> tuple[str, str]: ...
def study_model(text: str) -> tuple[str, str]: ...
def promote(text: str, filter: str, to: str | None = None) -> tuple[str, str]: ...
def set_folder(folder: str | os.PathLike[str] | None = None) -> Path | None: ...
def upgrade(text: str) -> tuple[str, list[str], list[str]]: ...
def connection_problem(text: str, source: str, target: str) -> str | None: ...
