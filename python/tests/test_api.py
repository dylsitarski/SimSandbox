"""The Python API as an agent uses it (ADR 0039): discover, build, check,
run, sweep and render, with results checked against known answers."""

import math
import os
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
    # A state, so the model runs in time rather than as a sweep.
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
    # A change to a parameter that does not exist is an error.
    with pytest.raises(ss.ModelError, match="no parameter .z."):
        m.run(inputs=[(0.5, "push.z", 1.0)])


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
    heavy.data["blocks"][0]["params"]["m"] = 2000.0
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

