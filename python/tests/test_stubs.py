"""The type stubs (simsandbox/_native.pyi) against the built module, and
the package's annotations under `mypy --strict`."""

import ast
import importlib.util
import inspect
import os
import subprocess
import sys
from pathlib import Path

import pytest

import simsandbox
from simsandbox import _native

PACKAGE = Path(simsandbox.__file__).parent
STUB = PACKAGE / "_native.pyi"
HERE = Path(__file__).parent


def _stub() -> ast.Module:
    return ast.parse(STUB.read_text(encoding="utf-8"))


def _public(name: str) -> bool:
    # Names starting with `_` in the stub are its own typing helpers
    # (TypedDicts, aliases); `__version__` is the module's.
    return not name.startswith("_") or name == "__version__"


def _stub_names(tree: ast.Module) -> dict[str, ast.AST]:
    names: dict[str, ast.AST] = {}
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.ClassDef)):
            names[node.name] = node
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            names[node.target.id] = node
        elif isinstance(node, ast.Assign):
            for t in node.targets:
                if isinstance(t, ast.Name):
                    names[t.id] = node
    return names


def test_the_package_is_typed():
    assert (PACKAGE / "py.typed").is_file()
    assert STUB.is_file()


def test_the_stub_names_what_the_module_has():
    stub = {n for n in _stub_names(_stub()) if _public(n)}
    runtime = {n for n in dir(_native) if _public(n)}
    assert stub == runtime, (
        f"in the stub only: {sorted(stub - runtime)}; in the module only: {sorted(runtime - stub)}"
    )
    stub_all = next(
        ast.literal_eval(node.value)
        for node in _stub().body
        if isinstance(node, ast.Assign) and any(getattr(t, "id", None) == "__all__" for t in node.targets)
    )
    assert stub_all == list(_native.__all__)


def test_each_function_has_the_signature_of_the_module():
    stub = _stub_names(_stub())
    checked = 0
    for name in dir(_native):
        obj = getattr(_native, name)
        if not _public(name) or not callable(obj) or isinstance(obj, type):
            continue
        node = stub[name]
        assert isinstance(node, ast.FunctionDef), name
        assert node.returns is not None, f"{name}: no return type"
        args = node.args
        assert not args.posonlyargs and not args.kwonlyargs and args.vararg is None and args.kwarg is None, name
        defaults = [None] * (len(args.args) - len(args.defaults)) + list(args.defaults)
        stub_params = [
            (a.arg, None if d is None else ast.literal_eval(d)) for a, d in zip(args.args, defaults)
        ]
        for a in args.args:
            assert a.annotation is not None, f"{name}({a.arg}): no type"
        if getattr(obj, "__text_signature__", None) is None:
            continue
        sig = inspect.signature(obj)
        runtime_params = []
        for p in sig.parameters.values():
            assert p.kind is inspect.Parameter.POSITIONAL_OR_KEYWORD, (name, p)
            runtime_params.append((p.name, None if p.default is inspect.Parameter.empty else p.default))
        assert stub_params == runtime_params, name
        checked += 1
    assert checked >= 25


def test_the_exception_is_a_value_error_in_both():
    cls = _stub_names(_stub())["ModelError"]
    assert isinstance(cls, ast.ClassDef)
    assert [ast.unparse(b) for b in cls.bases] == ["ValueError"]
    assert issubclass(_native.ModelError, ValueError)


def test_values_have_the_types_the_stub_gives():
    m = simsandbox.Model.example("Lorenz system")
    text = m.to_json()
    raw = _native.run(text, 0.05)
    assert set(raw) == {"rows", "index", "outputs", "populations", "over_rows", "labels", "notes", "warnings"}
    assert isinstance(raw["rows"], int)
    name, rows, data = raw["index"][0]
    assert isinstance(name, str) and rows == [raw["rows"]] and isinstance(data, bytes)
    name, shape, data = raw["outputs"][0]
    assert isinstance(shape, str) and isinstance(data, bytes)
    assert isinstance(raw["labels"], str)
    previous = _native.set_folder(os.getcwd())
    assert isinstance(_native.set_folder(previous), Path)
    name, model, folder = _native.examples()[0]
    assert isinstance(name, str) and isinstance(model, str) and (folder is None or isinstance(folder, str))
    assert _native.version_note(_native.made_with()) is None
    assert _native.connection_problem(text, "nope.x", "lorenz.u").startswith("the source")


def _mypy(*args: str, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-m", "mypy", "--strict", "--no-incremental", *args],
        cwd=cwd,
        capture_output=True,
        text=True,
    )


needs_mypy = pytest.mark.skipif(importlib.util.find_spec("mypy") is None, reason="mypy is not installed")


@needs_mypy
def test_stubtest_finds_the_stub_and_the_module_agree():
    # mypy's own check of a stub against what imports.
    r = subprocess.run(
        [sys.executable, "-m", "mypy.stubtest", "simsandbox._native"],
        capture_output=True,
        text=True,
    )
    assert r.returncode == 0, r.stdout + r.stderr


@needs_mypy
def test_typical_use_checks_under_mypy_strict(tmp_path):
    # From a temporary folder, so the installed package is the one checked.
    r = _mypy(str(HERE / "typing_usage.py"), cwd=tmp_path)
    assert r.returncode == 0, r.stdout + r.stderr


@needs_mypy
def test_mypy_strict_catches_a_wrong_call(tmp_path):
    # The stubs are used, not read as Any: wrong calls are errors.
    wrong = tmp_path / "wrong.py"
    wrong.write_text(
        "import simsandbox as ss\n"
        "from simsandbox import _native\n"
        "_native.run(1.0)\n"
        "_native.render('{}', widht=3.0)\n"
        "n: int = _native.check('{}')\n"
        "ss.Model().connect('a.y')\n",
        encoding="utf-8",
    )
    r = _mypy(str(wrong), cwd=tmp_path)
    assert r.returncode == 1, r.stdout
    for line in (3, 4, 5, 6):
        assert f"wrong.py:{line}: error" in r.stdout, r.stdout
