#!/bin/sh
# Fetch MuJoCo's C library and headers from the `mujoco-core` wheel on PyPI
# (the wheel ships libmujoco.so and include/mujoco/*.h) into ./mujoco/,
# which .gitignore keeps out of the repository. Linux x86-64 only as written;
# on macOS or Windows pick the matching wheel (--platform) or a release
# archive from GitHub, and set MUJOCO_DIR to where lib/ and include/ are.
set -eu
VERSION="${MUJOCO_VERSION:-4.0.1}"
here="$(cd "$(dirname "$0")" && pwd)"
dl="$here/mujoco/download"
mkdir -p "$dl" "$here/mujoco/lib" "$here/mujoco/include"
python3 -m pip download --no-deps --only-binary=:all: \
  --python-version 3.13 --platform manylinux_2_28_x86_64 \
  "mujoco-core==$VERSION" -d "$dl"
python3 -I - "$dl" "$here/mujoco" "$VERSION" <<'PY'
import sys, zipfile, pathlib, shutil
dl, out, ver = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3]
whl = next(dl.glob(f"mujoco_core-{ver}-*.whl"))
with zipfile.ZipFile(whl) as z:
    for name in z.namelist():
        if name.startswith("mujoco/include/mujoco/") and name.endswith(".h"):
            target = out / "include" / "mujoco" / pathlib.Path(name).name
        elif name == f"mujoco/libmujoco.so.{ver}":
            target = out / "lib" / pathlib.Path(name).name
        elif name.endswith("LICENSE") and "dist-info" in name:
            target = out / "LICENSE"
        else:
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        with z.open(name) as src, open(target, "wb") as dst:
            shutil.copyfileobj(src, dst)
lib = out / "lib"
for alias in ("libmujoco.so.4", "libmujoco.so"):
    p = lib / alias
    if p.exists() or p.is_symlink():
        p.unlink()
    p.symlink_to(f"libmujoco.so.{ver}")
print("MuJoCo", ver, "in", out)
PY
rm -rf "$dl"
