"""The Python API as an agent uses it (ADR 0039): discover, build, check,
run, sweep and render, with results checked against known answers."""

import math
import struct

import numpy as np
import pytest

import simsandbox as ss


def test_describe_lists_every_type_with_its_interface():
    catalogue = ss.describe()
    types = {t["type"]: t for t in catalogue["block_types"]}
    assert {"eq.equations", "eq.lorenz", "agents.grazers"} <= types.keys()
    grazers = types["agents.grazers"]
    assert grazers["settings"][0]["name"] == "update"
    assert any(p["name"] == "size" and p["unit"] == "m" for p in grazers["interface"]["params"])
    assert ss.describe("plot.heatmap")["type"] == "plot.heatmap"
    with pytest.raises(ss.ModelError, match="known: "):
        ss.describe("eq.nothing")
    assert "blocks" in ss.schema()["properties"]


def test_a_model_built_from_scratch_matches_its_exact_solution():
    # dx/dt = -k x from x0 = 1: x(t) = exp(-k t).
    m = ss.Model()
    m.add_block("decay", "eq.equations", "rk4", equations="dx/dt = -k*x", params={"k": 0.5, "x0": 1.0})
    m.set_run(dt=0.01, t_end=2.0)
    report = m.check()
    assert report["ok"], report
    assert m.get("decay.k") == 0.5
    r = m.run()
    assert r.t.shape == (201,)
    assert np.max(np.abs(r["decay.x"] - np.exp(-0.5 * r.t))) < 1e-9
    assert r.labels["speed"] == "RealTime"


def test_connected_blocks_with_feedthrough_and_parameter_edits():
    car = ss.Model.example("Cruise control")
    car.set("cruise.ki", 0.0)  # P control only: settles below the target.
    r = car.run(t_end=200.0)
    kp, c, target = 400.0, 50.0, 25.0
    assert r["car.v"][-1] == pytest.approx(kp * target / (kp + c), rel=1e-6)
    # Edits are file edits: the saved file holds the new value.
    assert car.block("cruise")["params"]["ki"] == 0.0


def test_sweeps_and_grids_come_back_as_arrays():
    curve = ss.Model.example("Resonance curve").run()
    w = curve["spring.w"]
    assert w.shape == (600,)
    expected = 1.0 / np.sqrt((1.0 - w**2) ** 2 + (0.2 * w) ** 2)
    assert np.allclose(curve["spring.amplitude"], expected, rtol=1e-12)

    swept = ss.Model.example("Resonance curve").sweep(("spring.w", 0.5, 1.5, 11)).run()
    assert swept.rows == 11

    rod = ss.Model.example("Heat along a rod").run(t_end=0.1)
    field = next(v for k, v in rod.outputs.items() if v.ndim == 3)
    assert field.shape[0] == rod.rows


def test_populations_have_a_varying_number_of_points():
    g = ss.Model.example("grazing").run(t_end=2.0)
    counts = g["sheep.count"]
    positions = g["sheep.position"]
    assert len(positions) == g.rows
    assert [len(p) for p in positions] == list(counts.astype(int))
    assert positions[0].shape[1] == 2
    # Same file, same seed: the same run.
    again = ss.Model.example("grazing").run(t_end=2.0)
    assert np.array_equal(again["sheep.position"][-1], positions[-1])


def test_problems_are_reported_where_they_are():
    m = ss.Model().add_block("a", "eq.lorenz", "leapfrog")
    report = m.check()
    assert not report["ok"]
    assert report["errors"][0]["path"] == "blocks[0].method"
    with pytest.raises(ss.ModelError, match=r"blocks\[0\]\.method"):
        m.run()
    with pytest.raises(ss.ModelError, match="already"):
        m.add_block("a", "eq.lorenz")
    syntax = ss.check('{"format": 1,,}')
    assert syntax["errors"][0]["line"] == 1


def test_views_render_to_png(tmp_path):
    out = tmp_path / "lorenz.png"
    png = ss.Model.example("Lorenz system").render(out, t_end=2.0, width=300, height=200, scale=2)
    assert png[1:4] == b"PNG"
    assert out.read_bytes() == png
    width, height = struct.unpack(">II", png[16:24])
    assert (width, height) == (600, 400)
    with pytest.raises(ss.ModelError, match="no view 5"):
        ss.Model.example("Lorenz system").render(view=5, t_end=0.1)


def test_every_example_loads_runs_briefly_and_round_trips():
    for name in ss.examples():
        m = ss.Model.example(name)
        assert m.check()["ok"], name
        again = ss.Model.from_json(m.to_json())
        assert again.data == m.data
        swept = "sample" in m.data
        r = m.run() if swept else m.run(t_end=10 * m.data.get("run", {}).get("dt", 0.01))
        assert r.rows > 0 and all(math.isfinite(x) for x in next(iter(r.index.values())))


def test_the_diagram_renders_even_for_a_model_that_cannot_run(tmp_path):
    m = ss.Model().add_block("a", "eq.equations", "rk4", equations="input u\ndx/dt = u - x", at=(0, 0))
    assert not m.check()["ok"], "a.u is not connected"
    png = m.render(tmp_path / "d.png", diagram=True, width=300, height=200)
    assert png[1:4] == b"PNG"
    with pytest.raises(ValueError):
        m.render(diagram=True, t_end=1.0)


def test_start_from_continues_from_a_reached_state():
    car = ss.Model.example("Cruise control")
    settled = car.start_from(60.0)
    assert any("v0 = " in n for n in settled.start_notes)
    before = car.run(t_end=60.0)["car.v"][-1]
    after = settled.run(t_end=0.1)["car.v"][0]
    assert after == before
    # Fields and populations are saved as a start state.
    grazing = ss.Model.example("grazing").start_from(2.0)
    assert "start" in grazing.block("sheep") and "start" in grazing.block("grass")
    assert grazing.check()["ok"]
