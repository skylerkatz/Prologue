fn main() {
    // With bundle.externalBin configured, tauri_build refuses to run when the
    // staged sidecar is absent — which would break plain `cargo build`/`cargo
    // test` (they never run scripts/build-sidecar.sh). An empty placeholder
    // satisfies the check; the tauri CLI's beforeDevCommand/beforeBuildCommand
    // always stage the real binary over it before anything is copied/bundled.
    let triple = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    let staged = std::path::Path::new("binaries").join(format!("prologue-{triple}"));
    if !staged.exists() {
        std::fs::create_dir_all("binaries").expect("create src-tauri/binaries");
        std::fs::write(&staged, []).expect("write sidecar placeholder");
    }

    tauri_build::build()
}
