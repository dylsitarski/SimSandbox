"""The Python API as an agent uses it (ADR 0039): discover, build, check,
run, study and render, with results checked against known answers."""

import math
import os
import re
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


def test_a_parameter_driven_by_a_wire_follows_its_signal():
    # dx/dt = -k x with k = a t driven by a clock: x = exp(-a t^2 / 2).
    m = ss.Model()
    m.add_block("clock", "eq.equations", "rk4", equations="param a [1/s^2] = 0.5\nstate tau [s] = 0\ndtau/dt = 1\nk = a*tau [1/s]")
    m.add_block("decay", "eq.equations", "rk4", equations="param k [1/s] = 100\nstate x = 1\ndx/dt = -k*x")
    m.connect("clock.k", "decay.k")
    m.set_run(dt=0.01, t_end=2.0)
    report = m.check()
    assert report["ok"], report
    assert report["driven"] == [{"param": "decay.k", "from": "clock.k"}]
    r = m.run()
    assert np.max(np.abs(r["decay.x"] - np.exp(-0.25 * r.t**2))) < 1e-8
    # A value of its own stays, unused while the wire drives it.
    m.set("decay.k", 2.0)
    assert m.check()["ok"]
    assert np.array_equal(m.run()["decay.x"], r["decay.x"])


def test_connected_blocks_with_feedthrough_and_parameter_edits():
    car = ss.Model.example("Cruise control")
    car.set("cruise.ki", 0.0)  # P control only: settles below the target.
    r = car.run(t_end=200.0)
    kp, c, target = 400.0, 50.0, 25.0
    assert r["car.v"][-1] == pytest.approx(kp * target / (kp + c), rel=1e-6)
    # Edits are file edits: the saved file holds the new value.
    assert car.block("cruise")["params"]["ki"] == 0.0


def test_a_run_keeps_only_what_it_is_asked_for():
    car = ss.Model.example("Cruise control")
    full = car.run(t_end=20.0)
    some = car.run(t_end=20.0, signals=["car.v", "cruise"], every=7)
    # Every 7th step and the last, the same values bit for bit.
    rows = list(range(0, full.rows, 7))
    if rows[-1] != full.rows - 1:
        rows.append(full.rows - 1)
    assert np.array_equal(some.index["t"], full.index["t"][rows])
    assert np.array_equal(some["car.v"], full["car.v"][rows])
    kept = set(some.outputs)
    assert "car.v" in kept and all(n == "car.v" or n.startswith("cruise.") for n in kept)
    with pytest.raises(ss.ModelError, match="no output named nope"):
        car.run(t_end=1.0, signals=["nope"])
    # Models using models: one car's outputs, every other step.
    platoon = ss.Model.example("Platoon")
    whole = platoon.run(t_end=1.0)
    one = platoon.run(t_end=1.0, signals="car2", every=2)
    assert np.array_equal(one["car2/car.v"], whole["car2/car.v"][::2])
    with pytest.raises(KeyError):
        one["car1/car.v"]


def test_studies_and_grids_come_back_as_arrays():
    # The resonance curve is a study (ADR 0071): a row per run.
    curve = ss.Model.example("Resonance curve").run()
    w = curve["curve.w"]
    assert w.shape == (600,) and list(curve.index) == ["run"]
    expected = 1.0 / np.sqrt((1.0 - w**2) ** 2 + (0.2 * w) ** 2)
    assert np.allclose(curve["curve.spring_amplitude"], expected, rtol=1e-12)

    # Any model studied whole, as "Study this model" on the canvas.
    m = ss.Model()
    m.add_block("spring", "eq.equations", equations="param c = 0.2\nparam w = 1\namplitude = 1/sqrt((1 - w^2)^2 + (c*w)^2)")
    note = m.as_study(("spring.w", 0.5, 1.5, 11))
    assert "study" in note
    study = next(b for b in m.data["blocks"] if b.get("type") == "study")
    assert study["sample"]["params"][0]["points"] == 11
    top = m.run(t_end=0.1).model(".")
    assert top[f"{study['name']}.spring_w"][-1].size == 11

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


def test_rows_of_a_table_come_back_by_id():
    # ADR 0079: rows with ids, passed between blocks and followed by id.
    m = ss.Model()
    m.add_block("pop", "eq.equations", "iterate", equations=[
        "table pts max 8",
        "start pts = 3",
        "state age[pts] = id(pts)",
        "state size[pts] = 1 + id(pts)",
        "age[n+1] = age + 1",
        "size[n+1] = size*2",
        "remove pts where age > 4",
        "add pts where rows(pts) < 4: age = 0, size = 1",
        "of_two = of(size, 2)",
    ])
    m.add_block("acc", "eq.equations", "iterate", equations=[
        "table q", "input size[q]", "state seen[q] = 0", "seen[n+1] = seen + size",
    ])
    m.connect("pop.size", "acc.size")
    m.set_run(dt=1.0, t_end=8.0)
    r = m.run()
    size = r["pop.size"]
    assert isinstance(size, ss.Rows) and len(size) == r.rows
    assert list(size.ids[0]) == [0, 1, 2] and list(size.values[0]) == [1.0, 2.0, 3.0]
    assert np.all(np.diff(size.versions) >= 0)
    # Row 2 by id is what the model says it is, NaN once it is gone.
    assert np.array_equal(size.of(2), r["pop.of_two"], equal_nan=True)
    ids, table = size.by_id()
    assert list(ids) == sorted(set(np.concatenate(size.ids)))
    assert np.array_equal(table[:, list(ids).index(2)], r["pop.of_two"], equal_nan=True)
    # Downstream follows the same rows.
    seen = r["acc.seen"]
    assert all(np.array_equal(a, b) for a, b in zip(seen.ids, size.ids))


def test_strategy_helpers_write_models_that_search():
    # ADR 0078 point 7. Pattern search on a bowl: exactly its bottom.
    m = ss.Model()
    m.add_block("bowl", "eq.equations", "evaluate", equations=["y = (a - 3)^2 + (b + 1)^2"],
                params={"a": 0.0, "b": 0.0})
    note = m.minimize(None, {"bowl.a": {"start": 0, "span": 4}, "bowl.b": 0}, "final(bowl.y)",
                      budget=40)
    assert "search" in note
    r = m.run()
    assert r["search.best_bowl_a"][-1] == 3.0 and r["search.best_bowl_b"][-1] == -1.0
    assert r["search.best_score"][-1] == 0.0
    # Refinement of where a bistable state's fate changes: the cubic's
    # middle root, bracketed within the tolerance.
    b = ss.Model()
    b.add_block("bi", "eq.equations", "rk4", equations=["state x", "dx/dt = r + x - x^3"],
                params={"r": 0.2, "x0": 0.0})
    b.set_run(dt=0.01, t_end=30.0)
    b.refine(None, {"bi.x0": (-2, 2)}, "final(bi.x) > 0", budget=15)
    r = b.run()
    root = sorted(np.roots([1, 0, -1, -0.2]).real)[1]
    lo, hi = r["refine.lo"][-1], r["refine.hi"][-1]
    assert lo < root < hi and hi - lo <= 1e-3
    assert len(r["refine.bi_x0"].ids[-1]) == 18
    # Over two parameters: a mesh refined around a circle.
    c = ss.Model()
    c.add_block("f", "eq.equations", "evaluate", equations=["inside = (u^2 + v^2) < 0.25"],
                params={"u": 0.0, "v": 0.0})
    c.refine(None, {"f.u": (-1, 1), "f.v": (-1, 1)}, "final(f.inside)", points=5, tol=0.05,
             max=2000, budget=20)
    r = c.run()
    points = r["refine.points"]
    assert points[-1] > points[0] and points[-1] == points[-2]
    us, vs = r["refine.f_u"].values[-1], r["refine.f_v"].values[-1]
    assert len(us) == points[-1] and len(vs) == len(us)
    # A study as the target: the fitting example refit by a new strategy.
    f = ss.Model.example("Fitting a model")
    f.fit("trials", {"osc.k": 2.0, "osc.c": 1.0}, "at_samples(osc.x - data.x, data)")
    r = f.run()
    assert abs(r["fit1.fit_osc_k"][-1] - 3.991282) < 1e-5
    assert abs(r["fit1.fit_osc_c"][-1] - 0.501779) < 1e-5
    with pytest.raises(ss.ModelError, match="at least one"):
        ss.Model().minimize(None, {}, "final(x)")


def test_probes_keep_their_share_of_the_memory_budget():
    # ADR 0062 step 5: a quarter of the budget each, unless set.
    m = ss.Model()
    m.add_block("m", "eq.equations", "rk4",
                equations=["grid x = 0 .. 1, 50", "state T[x] = x", "dT/dt = 0.01*lap(T)"])
    m.set_run(dt=0.001, t_end=1.0)
    m.add_probe("rod", ["m.T"])
    full = m.run_probes()
    assert len(full["rod"]) == 1001 and full.memory["budget"] == 1024 * 10**6
    small = m.run_probes(memory=0.4)
    assert 125 <= len(small["rod"]) <= 250
    assert dict(small.memory["parts"])["probe rod"] >= 8 * 50 * len(small["rod"])


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


def _cart_pole() -> ss.Model:
    """A model giving what ADR 0093's cart-pole picture binds: cart.x and
    pole.theta, and a pointer to grab with."""
    m = ss.Model()
    m.add_block("hand", "input.pointer", settings={"min": "-3", "max": "3", "unit": "m"})
    m.add_block(
        "cart", "eq.equations", "rk4",
        equations="param w [1/s] = 1\nstate x [m] = 0\nstate v [m/s] = 1\ndx/dt = v\ndv/dt = -w^2*x",
    )
    m.add_block(
        "pole", "eq.equations", "rk4",
        equations="param g [1/s^2] = 9.81\nstate theta = 0.3\nstate omega [1/s] = 0\n"
        "dtheta/dt = omega\ndomega/dt = -g*sin(theta)",
    )
    return m


CART_POLE = dict(
    unit="m",
    extent={"x": [-3, 3], "y": [-0.5, 2]},
    frames={"cart": {"x": "cart.x", "y": 0}, "pole": {"in": "cart", "angle": "pole.theta"}},
    layers=[
        {"draw": "segments", "bind": {"x1": -3, "y1": 0, "x2": 3, "y2": 0}},
        {"draw": "boxes", "in": "cart", "bind": {"x": 0, "y": 0, "width": 0.4, "height": 0.2}, "grab": "hand"},
        {"draw": "boxes", "in": "pole", "bind": {"x": 0, "y": 0.5, "width": 0.04, "height": 1.0}},
        {"draw": "discs", "in": "pole", "bind": {"x": 0, "y": 1.0, "radius": 0.05}, "grab": "hand"},
    ],
)


def test_a_scene_is_added_checked_saved_and_drawn(tmp_path):
    # ADR 0093 point 8's cart-pole picture, on a model with its signals.
    m = _cart_pole().add_scene(**CART_POLE)
    view = m.data["controls"]["views"][0]
    assert view["type"] == "plot.scene" and view["frames"]["pole"]["in"] == "cart"
    report = m.check()
    assert report["ok"], report
    assert report["warnings"] == [], report["warnings"]
    path = tmp_path / "cart-pole.ssbx.json"
    m.save(path)
    assert ss.Model.load(path).data == m.data
    png = m.render(tmp_path / "scene.png", t_end=1.0, view=0, width=300, height=200)
    assert png[1:4] == b"PNG"
    # Its node on the canvas, wired from what it binds and grabs with.
    assert m.render(diagram=True, width=600, height=300)[1:4] == b"PNG"


def test_a_scenes_form_and_names_are_refused_at_once():
    m = _cart_pole()
    refused = [
        ([{"draw": "segments", "bind": {"y1": 0, "x2": 1, "y2": 0}}], r"layers\[0\]\.bind\.x1: required"),
        ([{"draw": "boxes", "bind": {"x": 0, "y": 0, "width": 1, "height": 1, "size": 2}}], r"layers\[0\]\.bind\.size"),
        ([{"draw": "points", "bind": {"x": "car.x", "y": 0}}], r"layers\[0\]\.bind\.x: 'car\.x' names no block"),
        ([{"draw": "points", "bind": {"x": 0, "y": 0}, "grab": "cart"}], r"layers\[0\]\.grab: no input\.pointer block 'cart'"),
        ([{"draw": "polygons"}], r"layers\[0\]\.draw: no primitive 'polygons'"),
        ([{"draw": "points", "in": "pole", "bind": {"x": 0, "y": 0}}], r"layers\[0\]\.in: no frame 'pole'"),
    ]
    for layers, message in refused:
        with pytest.raises(ss.ModelError, match=message):
            m.add_scene(layers)
    with pytest.raises(ss.ModelError, match=r"frames\.a\.in: the frames .* form a cycle"):
        m.add_scene([], frames={"a": {"in": "b"}, "b": {"in": "a"}})
    with pytest.raises(ss.ModelError, match=r"extent\.x"):
        m.add_scene([], extent={"x": [1, 0], "y": [0, 1]})
    assert "views" not in m.data.get("controls", {})
    # Units are the check's: a speed as a length is a warning there, and
    # the scene is not drawn.
    m.add_scene([{"draw": "points", "bind": {"x": "cart.v", "y": 0}}])
    warnings = m.check()["warnings"]
    assert any("layers[0].bind.x: 'cart.v' is in m/s" in w for w in warnings), warnings


def test_a_scene_binds_a_parameter(tmp_path):
    # ADR 0102: a parameter names a value as a signal does; units are the
    # check's, and a signal of the same name would win.
    m = ss.Model.example("Pendulum (with its own view)")
    m.data["controls"]["views"] = []
    m.add_scene(
        [
            {"draw": "segments", "in": "rod", "bind": {"x1": 0, "y1": 0, "x2": "bob.L", "y2": 0}},
            {"draw": "discs", "in": "rod", "bind": {"x": "bob.L", "y": 0, "radius": 0.08}},
        ],
        frames={"rod": {"angle": "bob.angle"}},
    )
    report = m.check()
    assert report["ok"] and report["warnings"] == [], report
    # The rod drawn after L changed (a scheduled change, recorded every
    # step like a signal) differs from the rod before.
    before = m.render(t_end=0.2, width=200, height=200)
    longer = m.copy().schedule(0.1, "bob.L", 0.5)
    assert longer.render(t_end=0.2, width=200, height=200) != before
    # A damping coefficient is no length.
    m.add_scene([{"draw": "points", "bind": {"x": "bob.c", "y": 0}}])
    warnings = m.check()["warnings"]
    assert any("layers[0].bind.x: 'bob.c' is in" in w for w in warnings), warnings


def test_every_example_loads_runs_briefly_and_round_trips():
    for name in ss.examples():
        m = ss.Model.example(name)
        assert m.check()["ok"], name
        again = ss.Model.from_json(m.to_json())
        assert again.data == m.data
        studied = any(b.get("type") == "study" for b in m.data.get("blocks", []))
        r = m.run() if studied else m.run(t_end=10 * m.data.get("run", {}).get("dt", 0.01))
        if isinstance(r, ss.Composed):
            # A model using models (ADR 0068): its first model's results.
            r = r.model(next(iter(r.models)))
        assert r.rows > 0 and all(math.isfinite(x) for x in next(iter(r.index.values())))


def test_the_diagram_renders_even_for_a_model_that_cannot_run(tmp_path):
    m = ss.Model().add_block("a", "eq.equations", "rk4", equations="input u\ndx/dt = u - x", canvas=(0, 0))
    assert not m.check()["ok"], "a.u is not connected"
    png = m.render(tmp_path / "d.png", diagram=True, width=300, height=200)
    assert png[1:4] == b"PNG"
    with pytest.raises(ValueError):
        m.render(diagram=True, t_end=1.0)
    # The nodes the file opens show their values; values=True opens all.
    m.data.setdefault("controls", {})["open"] = ["a"]
    opened = m.render(diagram=True, width=300, height=200)
    every = m.render(diagram=True, width=300, height=200, values=True)
    assert opened == every
    with pytest.raises(ValueError):
        m.render(values=True)


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


def test_models_defined_in_the_file_and_model_parameters():
    platoon = ss.Model.example("Platoon")
    report = platoon.check()
    assert report["definitions"]["follower"]["uses"] == ["car1", "car2", "car3"]
    kp = next(p for p in report["params"] if p["key"] == "car2/control.kp")
    assert kp["formula"] == "kp"
    platoon.set("kp", 800.0)
    r = platoon.run(t_end=1.0)
    assert r["car3/car.v"].shape == (21,)
    assert platoon.render(diagram=True, inside="car1", width=300, height=200)[1:4] == b"PNG"
    assert platoon.render(diagram=True, expand=["car1", "car2"], width=300, height=200)[1:4] == b"PNG"
    with pytest.raises(ValueError):
        platoon.render(expand=["car1"])
    # Inputs reach a model inside by its full name (ADR 0066 step 5).
    pushed = platoon.run(t_end=2.0, inputs=[(1.0, "leader.target", 0.0)])
    assert isinstance(pushed, ss.Results) and pushed["car1/car.v"].shape[0] == 41
    assert "car4" in platoon.duplicate(["car3"])
    assert platoon.run(t_end=0.5)["car4/car.v"].shape == (11,)
    with pytest.raises(ss.ModelError, match="no model used as 'nope'"):
        platoon.render(diagram=True, inside="nope")

    # Q30: one field size for grass and sheep, then the sheep grouped.
    g = ss.Model.example("grazing")
    g.declare("size", 60, "m", "Side of the field")
    g.set("grass.size", "size").set("sheep.size", "size")
    assert {p["key"]: p["value"] for p in g.params()}["sheep.size"] == 60
    note = g.group(["sheep"])
    assert "model1" in note
    assert g.data["models"]["model1"]["params"]["size"]["value"] == 60
    assert g.check()["ok"]
    assert len(g.run(t_end=0.5).t(".")) == 6


def test_older_files_open_and_upgrade():
    old = os.path.join(os.path.dirname(__file__), "../../crates/simsandbox-file/tests/old/adr-0030-derived-signals.json")
    assert ss.check(old)["conversions"], "older forms are reported"
    m, conversions = ss.upgrade(old)
    assert any("signals" in c for c in conversions)
    assert "signals" not in m.data["controls"]
    assert ss.upgrade(m)[1] == []


def test_units_are_checked_and_converted_explicitly():
    m = ss.Model()
    m.add_block("shaker", "eq.equations", equations="param A [mm] = 50\nparam w [Hz] = 2\nx = A*sin(w*t) [mm]")
    m.add_block("spring", "eq.coupled_oscillator")
    m.connect("shaker.x", "spring.neighbor_x")
    report = m.check()
    assert not report["ok"]
    assert "unit mismatch" in str(report["errors"]) and "units.convert" in str(report["errors"])
    m.data["connections"] = []
    m.add_block("to_m", "units.convert", settings={"from": "mm", "to": "m"})
    m.connect("shaker.x", "to_m.u").connect("to_m.y", "spring.neighbor_x")
    m.set_run(dt=0.01, t_end=1.0)
    r = m.run()
    assert abs(r["to_m.y"][-1] - r["shaker.x"][-1] / 1000) < 1e-12
    units = {p["key"]: p["unit"] for p in m.check()["params"]}
    assert units["shaker.A"] == "mm"
    # Lines that do not balance are reported with the units.
    bad = ss.Model().add_block("b", "eq.equations", equations="input F [N]\nstate x [m]\ndx/dt = F")
    assert "dx/dt is in m/s but the right side is in" in str(bad.check()["errors"])


def test_the_unit_vocabulary_is_described():
    units = ss.describe("units")
    ft = next(u for u in units["symbols"] if u["symbol"] == "ft")
    assert (ft["si"], ft["factor"]) == ("m", 0.3048)
    m = ss.Model()
    m.add_block("a", "eq.equations", equations="param p [psi] = 100\nparam A [in^2] = 2\nF = p*A [lbf]\nparam v [m/s] = 0\nstate x [m] = 0\ndx/dt = v")
    assert abs(m.run(t_end=0.1, dt=0.1)["a.F"][0] - 200.0) < 1e-9


def test_filters_become_blocks_when_promoted(tmp_path):
    m = ss.Model.example("cruise")
    m.data["fusion"] = "off"
    m.data["controls"]["filters"] = [{"name": "push", "type": "expression", "expr": "cruise.F"}]
    before = m.run(t_end=5.0)["car.v"][-1]
    m.data["connections"] = [c for c in m.data["connections"] if c["to"] != "car.F"]
    note = m.promote("push", to="car.F")
    assert "push" in note
    assert any(b["name"] == "push" for b in m.data["blocks"])
    assert m.run(t_end=5.0)["car.v"][-1] == before
    # The diagram shows the data layer, or not.
    with_data = m.render(diagram=True)
    without = m.render(diagram=True, data=False)
    assert with_data != without
    with pytest.raises(ss.ModelError, match="no filter"):
        m.promote("nothing")


CART = {
    "format": 1,
    "run": {"dt": 0.01, "t_end": 2},
    "blocks": [
        {"name": "push", "type": "input.value", "settings": {"unit": "N", "min": "-50", "max": "50"}},
        {
            "name": "cart",
            "type": "eq.equations",
            "method": {"name": "rk4"},
            "equations": ["input F [N]", "param m [kg] = 2", "state v [m/s]", "dv/dt = F/m"],
        },
    ],
    "connections": [{"from": "push.y", "to": "cart.F"}],
}


def test_inputs_schedules_and_runs_replay_the_same(tmp_path):
    m = ss.Model(CART)
    # A change made during the run, and the same as the model's schedule.
    by_hand = m.run(inputs=[(0.5, "push.y", 10.0)])
    scheduled = m.copy().schedule(0.5, "push.y", 10.0).run()
    assert np.array_equal(by_hand["cart.v"], scheduled["cart.v"])
    # v = 1 until 0.5 s, then grows by F/m = 5 m/s^2.
    assert by_hand["cart.v"][-1] == pytest.approx(1 + 5 * 1.5)
    # Saved as a run beside the model, it replays bit for bit.
    m.save(tmp_path / "cart.ssbx.json")
    run = ss.Run(m, track=[(0.5, "push.y", 10.0)], name="push", model_path=tmp_path / "cart.ssbx.json")
    run.save(tmp_path / "cart.push.run.json")
    again = ss.Run.load(tmp_path / "cart.push.run.json")
    assert not again.stale and again.name == "push"
    assert np.array_equal(again.replay()["cart.v"], by_hand["cart.v"])
    # A run's input applies to another model, e.g. a heavier cart.
    heavy = ss.Model(CART).set("cart.m", 4.0)
    assert heavy.run(inputs=again)["cart.v"][-1] == pytest.approx(1 + 2.5 * 1.5)
    # Changing the model makes the run stale (with a warning).
    heavy.save(tmp_path / "cart.ssbx.json")
    with pytest.warns(UserWarning, match="changed since"):
        assert ss.Run.load(tmp_path / "cart.push.run.json").stale
    # A run says which SimSandbox made it; another version says so on load.
    text = (tmp_path / "cart.push.run.json").read_text()
    assert '"made_with": "simsandbox ' in text
    import json as _json

    data = _json.loads(text)
    data["made_with"] = "simsandbox 0.0.0"
    (tmp_path / "cart.old.run.json").write_text(_json.dumps(data))
    with pytest.warns(UserWarning) as caught:
        ss.Run.load(tmp_path / "cart.old.run.json")
    assert any("recorded with simsandbox 0.0.0" in str(w.message) for w in caught)
    # A change to a parameter that does not exist is an error.
    with pytest.raises(ss.ModelError, match="no parameter .z."):
        m.run(inputs=[(0.5, "push.z", 1.0)])


def test_run_files_are_read_and_written_as_every_interface_does(tmp_path):
    m = ss.Model(CART)
    m.save(tmp_path / "cart.ssbx.json")
    # A schedule written with whole numbers and keys in another order, and
    # a name outside ASCII: saved in the one form run files have.
    run = ss.Run(
        m,
        inputs=[{"to": 3, "set": "push.y", "t": 1}],
        track=[(0.5, "push.y", 10.0)],
        name="poussée",
        model_path=tmp_path / "cart.ssbx.json",
    )
    path = tmp_path / "cart.push.run.json"
    run.save(path)
    text = path.read_text(encoding="utf-8")
    assert '"name": "poussée"' in text
    assert text.index('"t": 1.0') < text.index('"set": "push.y"') < text.index('"to": 3.0')
    # Reading it and writing it again gives the same bytes.
    again = ss.Run.load(path)
    assert again.to_json(model_path="cart.ssbx.json") + "\n" == text
    assert again.inputs == [{"t": 1.0, "set": "push.y", "to": 3.0}]
    assert np.array_equal(again.replay()["cart.v"], run.replay()["cart.v"])
    # A field run files do not have, or a newer format, is refused on
    # reading, as the CLI and the app refuse it.
    import json as _json

    data = _json.loads(text)
    (tmp_path / "extra.run.json").write_text(_json.dumps({**data, "nope": 1}))
    with pytest.raises(ss.ModelError, match="extra.run.json.*nope"):
        ss.Run.load(tmp_path / "extra.run.json")
    (tmp_path / "newer.run.json").write_text(_json.dumps({**data, "format": 2}))
    with pytest.raises(ss.ModelError, match="newer.run.json"):
        ss.Run.load(tmp_path / "newer.run.json")


def test_views_render_with_a_runs_input():
    m = ss.Model(CART)
    m.data["controls"] = {"views": [{"type": "plot.xy", "bind": {"x": "t", "y": "cart.v"}}]}
    size = {"width": 300, "height": 200}
    still = m.render(**size)
    pushed = m.render(inputs=[(0.5, "push.y", 10.0)], **size)
    assert pushed[1:4] == b"PNG" and pushed != still
    # A run's schedule and recorded input draw the same as given by hand.
    run = ss.Run(m, track=[(0.5, "push.y", 10.0)])
    assert m.render(inputs=run, **size) == pushed
    with pytest.raises(ss.ModelError, match="no parameter 'push.z'"):
        m.render(inputs=[(0.5, "push.z", 1.0)], **size)
    with pytest.raises(ValueError, match="diagram"):
        m.render(diagram=True, inputs=run)


def test_probes_record_on_their_own_schedule_and_write_files(tmp_path):
    m = ss.Model.example("Cruise control")
    m.add_probe("speed", ["car.v"], every={"steps": 10}, keep={"all": True}, to="speed.csv")
    r = m.run_probes(t_end=5.0, save=tmp_path)
    p = r["speed"]
    assert len(p) == 11
    # The same numbers as a plain run, at every tenth step.
    plain = m.run(t_end=5.0)
    assert np.array_equal(p["car.v"], plain["car.v"][::10])
    assert np.array_equal(p.t, plain.t[::10])
    assert r.files == [str(tmp_path / "speed.csv")]
    lines = (tmp_path / "speed.csv").read_text().splitlines()
    assert lines[0] == "t,car.v" and len(lines) == 12
    # Saved with the model, and loaded back.
    again = ss.Model.from_json(m.to_json())
    assert again.data["controls"]["probes"][0]["every"] == {"steps": 10}
    with pytest.raises(ss.ModelError, match="no probe 'x'"):
        m.run_probes(probes=["x"])


def test_grid_probes_come_back_as_arrays_and_npy_files(tmp_path):
    m = ss.Model.example("Gray")
    field = "gs.v"
    assert m.run(t_end=1.0)[field].ndim == 3
    m.add_probe("profile", [field], every={"times": [0.0, 30.0]}, to="gs.csv")
    p = m.run_probes(t_end=50.0, save=tmp_path)["profile"]
    assert p[field].shape == (2, 128, 128)
    saved = np.load(tmp_path / f"gs.{field}.npy")
    assert np.array_equal(saved, p[field])


def test_a_saved_model_reads_its_data_file_from_its_folder(tmp_path):
    (tmp_path / "push.csv").write_text("t,F [N]\n0,1\n1,3\n")
    m = ss.Model()
    m.add_block("data", "input.file", settings={"file": "push.csv", "interpolate": "linear"})
    m.add_block("gain", "eq.equations", equations=["input F [N]", "y = 2*F [N]"])
    m.connect("data.F", "gain.F")
    m.set_run(dt=0.25, t_end=1.5)
    # Not saved: the relative file has no folder to be read from.
    assert not m.check()["ok"]
    m.save(tmp_path / "model.ssbx.json")
    assert m.check()["ok"]
    r = m.run()
    assert list(r["gain.y"]) == [2.0, 3.0, 4.0, 5.0, 6.0, 6.0, 6.0]
    again = ss.Model.load(tmp_path / "model.ssbx.json")
    assert list(again.run()["data.F"]) == [1.0, 1.5, 2.0, 2.5, 3.0, 3.0, 3.0]
    assert ss.check(tmp_path / "model.ssbx.json")["ok"]


def test_a_composition_runs_models_on_their_own_clocks(tmp_path):
    cruise = ss.Model.example("Cruise control")
    car, pi = cruise.data["blocks"]
    plant = ss.Model({"format": 1, "run": {"dt": 0.05}, "blocks": [car], "connections": []})
    plant.add_block("force", "input.value", settings={"unit": "N"}).connect("force.y", "car.F")
    plant.save(tmp_path / "car.ssbx.json")
    controller = ss.Model({"format": 1, "run": {"dt": 0.05}, "blocks": [pi], "connections": []})
    controller.add_block("speed", "input.value", settings={"unit": "m/s"}).connect("speed.y", "cruise.v")
    controller.save(tmp_path / "cruise.ssbx.json")
    c = ss.Model({"format": 1, "run": {"t_end": 10}, "blocks": []})
    c.add_model("plant", "car.ssbx.json")
    c.add_model("controller", "cruise.ssbx.json", params={"cruise.kp": 600}, dt=0.5)
    c.link("plant/car.v", "controller/speed").link("controller/cruise.F", "plant/force")
    c.save(tmp_path / "both.ssbx.json")
    report = c.check()
    assert report["ok"], report
    assert report["tick"] == 0.05 and report["order"] == ["plant", "controller"]
    r = c.run()
    assert isinstance(r, ss.Composed)
    assert r.model("plant").rows == 201 and r.model("controller").rows == 21
    assert r.t("controller")[1] == 0.5
    # The force changes only at the controller's points, held between.
    force = r["plant/force.y"]
    assert len(set(force[1:10])) == 1 and force[10] != force[9]
    assert r["controller/speed.y"][1] == r["plant/car.v"][10]
    again = ss.Model.load(tmp_path / "both.ssbx.json").run()
    assert np.array_equal(again["plant/car.v"], r["plant/car.v"])
    # A model using model files is itself a model file others can use.
    outer = ss.Model({"format": 1, "run": {"t_end": 10}, "blocks": []})
    outer.add_model("vehicle", "both.ssbx.json")
    outer.save(tmp_path / "outer.ssbx.json")
    nested = outer.run()
    assert list(nested.models) == ["vehicle/plant", "vehicle/controller"]
    assert np.array_equal(nested["vehicle/plant/car.v"], r["plant/car.v"])
    # Values reach into it by path, as numbers or formulas of this model's
    # parameters.
    deep = ss.Model({"format": 1, "run": {"t_end": 10}, "blocks": [],
                     "params": {"half": {"value": 300, "unit": "N*s/m"}}})
    deep.add_model("vehicle", "both.ssbx.json", params={"controller/cruise.kp": "2*half"})
    deep.save(tmp_path / "deep.ssbx.json")
    assert np.array_equal(deep.run()["vehicle/plant/car.v"], r["plant/car.v"])
    # A copy of the car replaces the one inside, for this use only.
    heavy = ss.Model.load(tmp_path / "car.ssbx.json")
    heavy.set("car.m", 2000.0)
    heavy.save(tmp_path / "car-2.ssbx.json")
    swapped = ss.Model({"format": 1, "run": {"t_end": 10}, "blocks": []})
    swapped.add_model("vehicle", "both.ssbx.json", files={"plant": "car-2.ssbx.json"})
    swapped.save(tmp_path / "swapped.ssbx.json")
    assert swapped.check()["replaced_files"] == {"vehicle/plant": "car-2.ssbx.json"}
    by_params = ss.Model({"format": 1, "run": {"t_end": 10}, "blocks": []})
    by_params.add_model("vehicle", "both.ssbx.json", params={"plant/car.m": 2000})
    by_params.save(tmp_path / "by_params.ssbx.json")
    assert np.array_equal(swapped.run()["vehicle/plant/car.v"], by_params.run()["vehicle/plant/car.v"])
    # On one clock: the files flattened into this model, wired directly.
    one = ss.Model({"format": 1, "run": {"dt": 0.05, "t_end": 10}, "blocks": []})
    one.add_model("plant", "car.ssbx.json", shared=True)
    one.add_model("controller", "cruise.ssbx.json", shared=True)
    one.connect("plant/car.v", "controller/speed").connect("controller/cruise.F", "plant/force")
    one.save(tmp_path / "one.ssbx.json")
    flat = one.run()
    assert list(flat.models) == ["."]
    direct = cruise.run(t_end=10)
    assert np.array_equal(flat["plant/car.v"], direct["car.v"])


def test_models_defined_in_the_file_are_used_by_name():
    gain = ss.Model({"format": 1, "blocks": []})
    gain.add_block("u", "input.value")
    gain.add_block("g", "eq.equations", equations=["input x", "y = 2*x"]).connect("u.y", "g.x")
    gain.data["outputs"] = [{"name": "y", "from": "g.y"}]
    m = ss.Model({"format": 1, "run": {"dt": 0.1, "t_end": 0.3}, "blocks": []})
    m.add_block("src", "eq.equations", equations=["state s", "ds/dt = 1"], params={"s0": 1})
    m.define("gain", gain).add_model("a", model="gain").add_model("b", model="gain")
    m.connect("src.s", "a/u").connect("a/y", "b/u")
    report = m.check()
    assert report["ok"], report
    assert report["definitions"]["gain"]["uses"] == ["a", "b"]
    r = m.run()
    assert np.array_equal(r["b/g.y"], 4 * r["src.s"])
    with pytest.raises(ValueError):
        m.add_model("c", "x.ssbx.json", model="gain")


def test_a_study_block_runs_a_definition_at_each_point():
    cruise = ss.Model.example("Cruise control").data
    for k in ("format", "description", "controls"):
        cruise.pop(k, None)
    sample = {"design": "list", "params": [{"param": "cruise.kp"}], "values": [[200.0], [800.0]],
              "reduce": {"settle": "first_time(abs(cruise.e) < 0.5)"}}
    m = ss.Model({"format": 1, "models": {"cruise": cruise},
                  "blocks": [{"name": "s", "type": "study", "model": "cruise", "sample": sample}],
                  "run": {"dt": 1.0, "t_end": 1.0}})
    assert m.check()["ok"]
    r = m.run()
    # Nothing else runs in time: a row per run. The firmer controller settles sooner.
    settle = list(r["s.settle"])
    assert len(settle) == 2 and settle[1] < settle[0]
    assert list(r["s.cruise_kp"]) == [200.0, 800.0]


def test_studying_a_used_model_as_on_the_canvas():
    m = ss.Model.example("Platoon")
    note = m.study("car1")
    assert "car1_study" in note
    block = next(b for b in m.data["blocks"] if b["name"] == "car1_study")
    assert block["model"] == "follower"
    block["sample"] = {"design": "list", "params": [{"param": "mass"}], "values": [[800.0], [3200.0]],
                       "reduce": {"v_max": "max(car.v)"}}
    r = m.run(t_end=0.2)
    v_max = r["car1_study.v_max"]
    assert v_max[-1].size == 2  # a line of cells, one per run


def test_levenberg_marquardt_as_a_model_matches_scipy():
    # ADR 0078 step 2: the fitting example, against SciPy's least_squares on
    # the oscillator's exact solution and the example's own measurements.
    optimize = pytest.importorskip("scipy.optimize")
    m = ss.Model.example("Fitting a model")
    data = m.data["models"]["oscillator"]["blocks"][1]["settings"]["data"]
    rows = [line.split(",") for line in data.strip().splitlines()[1:]]
    t = np.array([float(a) for a, _ in rows])
    obs = np.array([float(b) for _, b in rows])

    def x_of(p):
        k, c = p
        w = np.sqrt(k - c * c / 4)
        return np.exp(-c * t / 2) * (np.cos(w * t) + c / (2 * w) * np.sin(w * t))

    best = optimize.least_squares(
        lambda p: x_of(p) - obs, [2.0, 1.0], xtol=1e-15, ftol=1e-15, gtol=1e-15
    )
    r = m.run()
    # RK4 at dt = 0.01 against the exact solution: agreement to 1e-7.
    assert abs(r["fit.k"][-1] - best.x[0]) < 1e-7
    assert abs(r["fit.c"][-1] - best.x[1]) < 1e-7
    assert r["fit.done"][-1] == 1.0


def test_an_output_over_three_axes_is_an_array_with_named_axes():
    # ADR 0081: q = x + 10y + 100z over axes of 2, 3 and 4 cells, the first
    # fastest, so last in the array.
    m = ss.Model()
    m.add_block("m", "eq.equations", equations="grid x = 2\ngrid y = 3\ngrid z = 4\nq = x + 10*y + 100*z")
    r = m.run(t_end=1.0)
    q = r["m.q"]
    assert q.shape == (r.rows, 4, 3, 2)
    assert q[-1, 3, 2, 1] == 1 + 20 + 300
    assert r.axes("m.q") == [("z", 4), ("y", 3), ("x", 2)]


def test_connect_refuses_names_the_model_does_not_have():
    # Checked when the wire is written, not when the model runs.
    m = ss.Model()
    m.add_block("clock", "eq.equations", "rk4", equations="param a = 0.5\nstate tau = 0\ndtau/dt = 1\nk = a*tau")
    m.add_block("decay", "eq.equations", "rk4", equations="param k = 1\ninput u\nstate x = 1\ndx/dt = -k*x + u")
    wrong = {
        ("clok.k", "decay.u"): "the source 'clok.k' names no block 'clok'; blocks: clock, decay",
        ("clock.q", "decay.u"): "the block 'clock' has no output 'q'; outputs: tau, k",
        ("clock.k", "decay.zz"): "has no input or parameter 'zz'; inputs: u; parameters: k, x0",
        ("clock.k", "dekay.u"): "the target 'dekay.u' names no block 'dekay'",
        ("clock", "decay.u"): "is not 'block.port'",
        ("clock.k", "plant/u"): "names no used model 'plant'",
    }
    for (source, target), message in wrong.items():
        with pytest.raises(ss.ModelError, match=re.escape(message)):
            m.connect(source, target)
    assert m.data["connections"] == []
    with pytest.raises(TypeError):
        m.connect("clock.k", None)  # type: ignore[arg-type]
    # An input or a parameter takes one wire.
    m.connect("clock.k", "decay.u")
    with pytest.raises(ss.ModelError, match="already wired from 'clock.k'"):
        m.connect("clock.tau", "decay.u")
    m.connect("clock.tau", "decay.k")
    assert m.check()["ok"]
    assert m.data["connections"] == [
        {"from": "clock.k", "to": "decay.u"},
        {"from": "clock.tau", "to": "decay.k"},
    ]


def test_add_block_refuses_an_unknown_type():
    m = ss.Model()
    with pytest.raises(ss.ModelError, match="unknown block type 'eq.nothing'; known: .*eq.equations"):
        m.add_block("x", "eq.nothing")
    assert m.data["blocks"] == []


def test_connect_accepts_every_wire_of_every_example():
    # Each example rebuilt wire by wire: connect refuses none of the wires
    # the model files need.
    examples = os.path.join(os.path.dirname(__file__), "..", "..", "examples")
    paths = sorted(
        os.path.join(d, f)
        for d, _, files in os.walk(examples)
        for f in files
        if f.endswith(".ssbx.json")
    )
    if not paths:
        pytest.skip("the repository's examples are not here")
    wires = 0
    for path in paths:
        m = ss.Model.load(path)
        connections = m.data.get("connections", [])
        m.data["connections"] = []
        for c in connections:
            m.connect(c["from"], c["to"])
            wires += 1
        assert m.data["connections"] == connections, path
    assert wires > 20



def test_a_mechanism_is_built_from_rows_checked_at_the_call():
    # ADR 0105: one row per body; every number a parameter; poses as rows.
    m = ss.Model()
    m.add_block(
        "cartpole",
        "mech.planar",
        bodies=[
            {"name": "cart", "parent": "ground", "joint": "slider", "kind": "prismatic", "mass": 1.0},
            {"name": "pole", "parent": "cart", "joint": "hinge", "kind": "revolute",
             "mass": 0.1, "centre": [0, 0.5], "inertia": 0.0083, "start": [0.1, 0]},
        ],
    )
    m.add_block("push", "eq.equations", equations=["f = 0 [N]"])
    m.connect("push.f", "cartpole.slider_force")
    run = m.run(t_end=0.5, dt=0.001)
    angle = run["cartpole.hinge_angle"]
    assert angle[0] == 0.1 and angle[-1] > 0.1  # it falls
    with pytest.raises(ss.ModelError, match=r"bodies\[1\]\.parent"):
        ss.Model().add_block(
            "m",
            "mech.planar",
            bodies=[
                {"name": "a", "parent": "ground", "joint": "ja", "kind": "revolute", "mass": 1, "centre": [0, -1]},
                {"name": "b", "parent": "c", "joint": "jb", "kind": "revolute", "mass": 1},
            ],
        )
    with pytest.raises(ss.ModelError, match="only mechanisms"):
        ss.Model().add_block("l", "eq.lorenz", bodies=[])
