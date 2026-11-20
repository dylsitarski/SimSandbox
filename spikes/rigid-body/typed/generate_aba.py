#!/usr/bin/env python3
"""The equation-generator route: Featherstone's articulated-body algorithm
(ABA) unrolled into scalar typed equations, one group of lines per body,
from a bodies-and-joints description.

The lines are the same arithmetic as src/native_2d.rs (2D spatial algebra,
motion vectors (w, vx, vy) in each body's frame at its joint), written out
for a fixed tree: no arrays, no new functions, so the language as it is
runs them. States are the joint coordinates q<k>_ and rates qd<k>_; outputs
are each body's centre as rows (x<k>, y<k>, a<k>, vx<k>, vy<k>) and the
energy E.

    python3 typed/generate_aba.py     # writes typed/aba-chain-*.ssbx.json

Units are left out: the joint coordinates of one mechanism mix metres and
radians (see the research note).
"""

import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))


def num(x):
    return repr(float(x))


class Gen:
    def __init__(self):
        self.lines = []

    def let(self, name, expr):
        self.lines.append(f"{name} = {expr}")
        return name


def sum_terms(terms):
    """Join (coefficient-expression) terms, dropping zeros."""
    terms = [t for t in terms if t is not None]
    if not terms:
        return "0"
    out = terms[0]
    for t in terms[1:]:
        out += t if t.startswith("-") and False else f" + {t}"
    return out


def prod(a, b):
    """a*b for expressions or numbers, None when either is zero."""
    if a is None or b is None:
        return None
    if a == "0" or b == "0":
        return None
    if a == "1":
        return b
    if b == "1":
        return a
    return f"({a})*({b})"


def generate(bodies, gravity):
    """bodies: dicts with parent (index or None), joint ('rev' or 'pris'),
    axis (for 'pris'), at, rot, mass, com, inertia. Returns equation lines
    and the state names."""
    g = Gen()
    n = len(bodies)
    L = g.lines
    L.append("# ABA unrolled by typed/generate_aba.py; body k's joint coordinate qk_, rate qdk_")
    for k in range(n):
        L.append(f"state q{k}_ = 0")
        L.append(f"state qd{k}_ = 0")
        L.append(f"param tau{k} = 0")
        L.append(f"param damp{k} = 0")
    # Pass 1: placement, velocity, velocity product, bias force.
    for k, b in enumerate(bodies):
        p = b["parent"]
        rot = b.get("rot", 0.0)
        if b["joint"] == "rev":
            th = f"q{k}_" if rot == 0 else f"(q{k}_ + {num(rot)})"
            g.let(f"c{k}", f"cos({th})")
            g.let(f"s{k}", f"sin({th})")
            rx, ry = num(b["at"][0]), num(b["at"][1])
        else:
            import math

            cr, sr = math.cos(rot), math.sin(rot)
            g.let(f"c{k}", num(cr))
            g.let(f"s{k}", num(sr))
            d = b["axis"]
            dx, dy = cr * d[0] - sr * d[1], sr * d[0] + cr * d[1]
            rx = g.let(f"rx{k}", f"{num(b['at'][0])} + {num(dx)}*q{k}_")
            ry = g.let(f"ry{k}", f"{num(b['at'][1])} + {num(dy)}*q{k}_")
        g.rx = getattr(g, "rx", {})
        g.rx[k] = (rx, ry)
        # Motion transform entries: X = [[1,0,0],[a,c,s],[bb,-s,c]].
        g.let(f"xa{k}", f"s{k}*{rx} - c{k}*{ry}")
        g.let(f"xb{k}", f"s{k}*{ry} + c{k}*{rx}")
        # Velocity.
        if p is None:
            wp, vxp, vyp = "0", "0", "0"
        else:
            wp, vxp, vyp = f"w{p}", f"vx{p}", f"vy{p}"
        if p is None:
            lx, ly = "0", "0"
        else:
            lx = g.let(f"lx{k}", f"{vxp} - {wp}*({ry})")
            ly = g.let(f"ly{k}", f"{vyp} + {wp}*({rx})")
        if b["joint"] == "rev":
            g.let(f"w{k}", f"{wp} + qd{k}_" if p is not None else f"qd{k}_")
            g.let(f"vx{k}", f"c{k}*{lx} + s{k}*{ly}" if p is not None else "0")
            g.let(f"vy{k}", f"-s{k}*{lx} + c{k}*{ly}" if p is not None else "0")
            # c = crm(v, (qd, 0, 0)) = (0, vy qd, -vx qd)
            g.let(f"cb1_{k}", f"vy{k}*qd{k}_")
            g.let(f"cb2_{k}", f"-vx{k}*qd{k}_")
        else:
            d = b["axis"]
            g.let(f"w{k}", wp)
            base_x = f"c{k}*{lx} + s{k}*{ly} + " if p is not None else ""
            base_y = f"-s{k}*{lx} + c{k}*{ly} + " if p is not None else ""
            g.let(f"vx{k}", f"{base_x}{num(d[0])}*qd{k}_")
            g.let(f"vy{k}", f"{base_y}{num(d[1])}*qd{k}_")
            # c = crm(v, (0, dx qd, dy qd)) = (0, -w dy qd, w dx qd)
            g.let(f"cb1_{k}", f"-w{k}*{num(d[1])}*qd{k}_")
            g.let(f"cb2_{k}", f"w{k}*{num(d[0])}*qd{k}_")
        # Spatial inertia at the joint.
        m, (cx, cy), ic = b["mass"], b["com"], b["inertia"]
        I = [
            [ic + m * (cx * cx + cy * cy), -m * cy, m * cx],
            [-m * cy, m, 0.0],
            [m * cx, 0.0, m],
        ]
        b["I"] = I
        # h = I v; bias = crf(v, h) = (-vy h1 + vx h2, -w h2, w h1)
        g.let(f"h0_{k}", f"{num(I[0][0])}*w{k} + {num(I[0][1])}*vx{k} + {num(I[0][2])}*vy{k}")
        g.let(f"h1_{k}", f"{num(I[0][1])}*w{k} + {num(m)}*vx{k}")
        g.let(f"h2_{k}", f"{num(I[0][2])}*w{k} + {num(m)}*vy{k}")
    # Pass 2: articulated inertias and bias forces, leaves to root.
    children = {k: [] for k in range(n)}
    for k, b in enumerate(bodies):
        if b["parent"] is not None:
            children[b["parent"]].append(k)
    for k in reversed(range(n)):
        b = bodies[k]
        I = b["I"]
        # IA = I + sum of children's transformed Ia (symmetric: 6 entries).
        idx = [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)]
        for r, c in idx:
            terms = [num(I[r][c])] + [f"T{r}{c}_{ch}" for ch in children[k]]
            g.let(f"A{r}{c}_{k}", " + ".join(terms))
        pa0 = " + ".join([f"-vy{k}*h1_{k} + vx{k}*h2_{k}"] + [f"F0_{ch}" for ch in children[k]])
        pa1 = " + ".join([f"-w{k}*h2_{k}"] + [f"F1_{ch}" for ch in children[k]])
        pa2 = " + ".join([f"w{k}*h1_{k}"] + [f"F2_{ch}" for ch in children[k]])
        g.let(f"P0_{k}", pa0)
        g.let(f"P1_{k}", pa1)
        g.let(f"P2_{k}", pa2)
        A = lambda r, c: f"A{min(r, c)}{max(r, c)}_{k}"
        if b["joint"] == "rev":
            S = ("1", "0", "0")
        else:
            S = ("0", num(b["axis"][0]), num(b["axis"][1]))
        for r in range(3):
            g.let(f"U{r}_{k}", sum_terms([prod(A(r, c), S[c]) for c in range(3)]))
        g.let(f"D_{k}", sum_terms([prod(S[r], f"U{r}_{k}") for r in range(3)]))
        g.let(f"u_{k}", f"tau{k} - damp{k}*qd{k}_ - (" + sum_terms([prod(S[r], f"P{r}_{k}") for r in range(3)]) + ")")
        if b["parent"] is None:
            continue
        # Ia = IA - U U^T / D; pa = P + Ia c + U u / D (c = (0, cb1, cb2))
        for r, c in idx:
            g.let(f"Ia{r}{c}_{k}", f"{A(r, c)} - U{r}_{k}*U{c}_{k}/D_{k}")
        Ia = lambda r, c: f"Ia{min(r, c)}{max(r, c)}_{k}"
        for r in range(3):
            g.let(
                f"pa{r}_{k}",
                f"P{r}_{k} + {Ia(r, 1)}*cb1_{k} + {Ia(r, 2)}*cb2_{k} + U{r}_{k}*u_{k}/D_{k}",
            )
        # To the parent: X^T Ia X and X^T pa, X = [[1,0,0],[a,c,s],[bb,-s,c]].
        X = [["1", "0", "0"], [f"xa{k}", f"c{k}", f"s{k}"], [f"xb{k}", f"-s{k}", f"c{k}"]]
        for r in range(3):
            for c in range(3):
                g.let(f"M{r}{c}_{k}", sum_terms([prod(Ia(r, j), X[j][c]) for j in range(3)]))
        for r, c in idx:
            g.let(f"T{r}{c}_{k}", sum_terms([prod(X[j][r], f"M{j}{c}_{k}") for j in range(3)]))
        # force to parent: fx = c f1 - s f2, fy = s f1 + c f2, n = f0 + rx fy - ry fx
        rx, ry = g.rx[k]
        g.let(f"F1_{k}", f"c{k}*pa1_{k} - s{k}*pa2_{k}")
        g.let(f"F2_{k}", f"s{k}*pa1_{k} + c{k}*pa2_{k}")
        g.let(f"F0_{k}", f"pa0_{k} + ({rx})*F2_{k} - ({ry})*F1_{k}")
    # Pass 3: accelerations, root to leaves. Base acceleration (0, -gx, -gy).
    for k, b in enumerate(bodies):
        p = b["parent"]
        if p is None:
            a0, a1, a2 = "0", num(-gravity[0]), num(-gravity[1])
        else:
            a0, a1, a2 = f"aa0_{p}", f"aa1_{p}", f"aa2_{p}"
        # X a_p: (a0, a*a0 + c a1 + s a2, bb*a0 - s a1 + c a2), plus c
        g.let(f"ab0_{k}", a0)
        g.let(f"ab1_{k}", f"xa{k}*{a0} + c{k}*{a1} + s{k}*{a2} + cb1_{k}")
        g.let(f"ab2_{k}", f"xb{k}*{a0} - s{k}*{a1} + c{k}*{a2} + cb2_{k}")
        g.let(f"qdd{k}", f"(u_{k} - U0_{k}*ab0_{k} - U1_{k}*ab1_{k} - U2_{k}*ab2_{k})/D_{k}")
        if b["joint"] == "rev":
            S = ("1", "0", "0")
        else:
            S = ("0", num(b["axis"][0]), num(b["axis"][1]))
        for r in range(3):
            extra = f" + {S[r]}*qdd{k}" if S[r] != "0" else ""
            g.let(f"aa{r}_{k}", f"ab{r}_{k}{extra}")
    for k in range(n):
        L.append(f"dq{k}_/dt = qd{k}_")
        L.append(f"dqd{k}_/dt = qdd{k}")
    # Rows: world pose of each joint frame, then the centre.
    L.append("# each body's centre as a row: x, y, angle, vx, vy (world)")
    for k, b in enumerate(bodies):
        p = b["parent"]
        rx, ry = g.rx[k]
        th = f"q{k}_" if b["joint"] == "rev" else "0"
        if b.get("rot", 0.0) != 0:
            th = f"{th} + {num(b['rot'])}"
        if p is None:
            g.let(f"a{k}", th)
            g.let(f"ox{k}", rx)
            g.let(f"oy{k}", ry)
        else:
            g.let(f"a{k}", f"a{p} + {th}")
            g.let(f"ox{k}", f"ox{p} + cos(a{p})*({rx}) - sin(a{p})*({ry})")
            g.let(f"oy{k}", f"oy{p} + sin(a{p})*({rx}) + cos(a{p})*({ry})")
        cx, cy = b["com"]
        g.let(f"x{k}", f"ox{k} + cos(a{k})*{num(cx)} - sin(a{k})*{num(cy)}")
        g.let(f"y{k}", f"oy{k} + sin(a{k})*{num(cx)} + cos(a{k})*{num(cy)}")
        g.let(f"bx{k}", f"vx{k} - w{k}*{num(cy)}")
        g.let(f"by{k}", f"vy{k} + w{k}*{num(cx)}")
        g.let(f"cvx{k}", f"cos(a{k})*bx{k} - sin(a{k})*by{k}")
        g.let(f"cvy{k}", f"sin(a{k})*bx{k} + cos(a{k})*by{k}")
    terms = []
    for k, b in enumerate(bodies):
        m, ic = b["mass"], b["inertia"]
        terms.append(
            f"{num(0.5 * m)}*(cvx{k}^2 + cvy{k}^2) + {num(0.5 * ic)}*w{k}^2"
            f" - {num(m)}*({num(gravity[0])}*x{k} + {num(gravity[1])}*y{k})"
        )
    # Split the sum so no expression nests too deep.
    parts = []
    for i in range(0, len(terms), 20):
        parts.append(g.let(f"E_{i // 20}", " + ".join(terms[i : i + 20])))
    g.let("E", " + ".join(parts))
    return L


def chain(n, length=0.1, mass=0.1):
    return [
        dict(
            parent=None if k == 0 else k - 1,
            joint="rev",
            at=[0.0 if k == 0 else length, 0.0],
            mass=mass,
            com=[length / 2, 0.0],
            inertia=mass * length * length / 12,
        )
        for k in range(n)
    ]


def write(name, bodies, desc):
    lines = generate(bodies, [0.0, -9.81])
    model = {
        "format": 1,
        "description": desc,
        "run": {"dt": 0.001, "t_end": 10.0},
        "blocks": [
            {
                "name": "mech",
                "type": "eq.equations",
                "equations": lines,
                "method": {"name": "rk4"},
            }
        ],
        "connections": [],
    }
    with open(os.path.join(HERE, name), "w") as f:
        f.write(json.dumps(model, indent=2) + "\n")
    return len(lines)


if __name__ == "__main__":
    for n in [2, 10, 50, 200]:
        count = write(
            f"aba-chain-{n}.ssbx.json",
            chain(n),
            f"The spike's chain of {n} links as typed equations generated by typed/generate_aba.py: "
            "Featherstone's ABA unrolled into scalar lines per body (relative joint angles q, rates qd).",
        )
        print(f"aba-chain-{n}: {count} lines")
