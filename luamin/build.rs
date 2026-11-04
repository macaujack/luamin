use std::{env, fs, path::PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    // Cargo places the external README in the package root when packaging.
    // In the workspace, the only maintained copy is in the repository root.
    let packaged_readme = manifest_dir.join("README.md");
    let readme = if packaged_readme.is_file() {
        packaged_readme
    } else {
        manifest_dir.join("../README.md")
    };

    println!("cargo::rerun-if-changed={}", readme.display());
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::copy(&readme, out_dir.join("README.md"))
        .expect("failed to copy the crate README for documentation");
}
