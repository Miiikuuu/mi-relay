use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("CARGO_FEATURE_DESKTOP").is_none() {
        return;
    }
    let manifest = "packaging/linux/mirelay.gresource.xml";
    println!("cargo:rerun-if-changed={manifest}");
    println!("cargo:rerun-if-changed=assets/brand/MiRelay-brand-kit-v1/icons/png");
    println!(
        "cargo:rerun-if-changed=assets/brand/MiRelay-brand-kit-v1/wordmark/mirelay-wordmark.png"
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"))
        .join("mirelay.gresource");
    let status = Command::new("glib-compile-resources")
        .arg(manifest)
        .arg("--sourcedir=assets/brand/MiRelay-brand-kit-v1")
        .arg("--target")
        .arg(output)
        .status()
        .expect("Desktop builds require glib-compile-resources (libglib2.0-bin on Debian/Ubuntu)");
    assert!(
        status.success(),
        "Could not compile MiRelay's brand resources"
    );
}
