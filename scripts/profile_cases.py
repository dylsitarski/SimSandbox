#!/usr/bin/env python3
"""Write the profiling cases of docs/design/performance.md into a folder.

Each case is an example made heavier, or a small composition, so it runs
long enough to time and profile:

    python3 scripts/profile_cases.py OUT_DIR
    cargo build --release -p simsandbox-session --example profile_run
    for f in OUT_DIR/*.ssbx.json; do target/release/examples/profile_run $f; done
"""

import copy
import json
import sys
from pathlib import Path

EXAMPLES = Path(__file__).resolve().parent.parent / "examples"


def load(name):
    return json.loads((EXAMPLES / name).read_text())


def main(out):
    out.mkdir(parents=True, exist_ok=True)

    def save(name, model):
        (out / name).write_text(json.dumps(model))

    # One large grid (typed equations, 128 x 128, two fields).
    g = load("gray-scott.ssbx.json")
    g["run"]["t_end"] = 1500
    save("grid.ssbx.json", g)

    # Two such grids, each on its own clock, unlinked.
    spots = {k: v for k, v in g.items() if k in ("blocks", "connections", "params")}
    save(
        "two_grids.ssbx.json",
        {
            "format": 1,
            "models": {"spots": spots},
            "blocks": [
                {"name": "a", "model": "spots", "clock": {"dt": 1.0}},
                {"name": "b", "model": "spots", "clock": {"dt": 1.0}},
            ],
            "run": {"t_end": 200},
        },
    )

    # One small ODE: per-step overhead.
    lorenz = load("lorenz.ssbx.json")
    lorenz.setdefault("run", {})["t_end"] = 2000
    save("small_ode.ssbx.json", lorenz)

    # A fused group.
    c = load("coupled-oscillators.ssbx.json")
    c.setdefault("run", {})
    c["run"]["t_end"] = c["run"].get("t_end", 10) * 400
    save("fused.ssbx.json", c)

    # Many blocks on one clock: a platoon of 40 cars.
    p = load("platoon.ssbx.json")
    follower = next(b for b in p["blocks"] if b.get("model") == "follower")
    p["blocks"] = [b for b in p["blocks"] if b.get("model") != "follower"]
    p["connections"] = [k for k in p["connections"] if "car" not in k["to"]]
    previous = "leader.v"
    for i in range(1, 41):
        car = copy.deepcopy(follower)
        car["name"] = f"car{i}"
        car.pop("canvas", None)
        car["params"] = {"mass": 1000 + 20 * i}
        p["blocks"].append(car)
        p["connections"].append({"from": previous, "to": f"car{i}/lead"})
        previous = f"car{i}/v"
    p["run"]["t_end"] = 600
    p["controls"] = {}
    save("many_blocks.ssbx.json", p)

    # Models on their own clocks, linked by a probe.
    f = load("forced-spring.ssbx.json")
    spring = f["models"]["spring"]
    spring["blocks"].append(
        {"name": "drive", "type": "input.value", "settings": {"unit": "N"}}
    )
    spring["connections"] = [{"from": "drive.y", "to": "mass.F"}]
    spring["blocks"][0]["params"].pop("F")
    spring["params"].pop("F")
    save(
        "linked.ssbx.json",
        {
            "format": 1,
            "models": {"spring": spring},
            "blocks": [
                {
                    "name": "push",
                    "type": "eq.equations",
                    "equations": [
                        "param w [rad/s] = 1",
                        "param F0 [N] = 1",
                        "state phase [rad] = 0",
                        "dphase/dt = w",
                        "F = F0*sin(phase)",
                    ],
                },
                {"name": "a", "model": "spring", "clock": {"dt": 0.01}},
                {"name": "b", "model": "spring", "clock": {"dt": 0.02}},
            ],
            "connections": [
                {"from": "probe:pf.push.F", "to": "a/drive"},
                {"from": "probe:pf.push.F", "to": "b/drive"},
            ],
            "controls": {"probes": [{"name": "pf", "record": ["push.F"]}]},
            "run": {"t_end": 2000, "dt": 0.01},
        },
    )

    # Agents and a field.
    gz = load("grazing.ssbx.json")
    gz["run"]["t_end"] = gz["run"].get("t_end", 10) * 20
    save("agents.ssbx.json", gz)

    # A study of a model in time.
    save("study.ssbx.json", load("cruise-tuning.ssbx.json"))


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(Path(sys.argv[1]))
