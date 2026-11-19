// Links libmujoco when the `mujoco` feature is on: from $MUJOCO_DIR/lib, or
// ./mujoco/lib as ./fetch-mujoco.sh leaves it, with an rpath so the binary
// finds the shared library where it was linked.
fn main() {
    println!("cargo:rerun-if-env-changed=MUJOCO_DIR");
    if std::env::var_os("CARGO_FEATURE_MUJOCO").is_none() {
        return;
    }
    let dir = std::env::var("MUJOCO_DIR")
        .unwrap_or_else(|_| format!("{}/mujoco", std::env::var("CARGO_MANIFEST_DIR").unwrap()));
    let lib = format!("{dir}/lib");
    if !std::path::Path::new(&lib).exists() {
        panic!("MuJoCo not found in {lib}: run ./fetch-mujoco.sh or set MUJOCO_DIR");
    }
    println!("cargo:rustc-link-search=native={lib}");
    println!("cargo:rustc-link-lib=dylib=mujoco");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{lib}");
}
