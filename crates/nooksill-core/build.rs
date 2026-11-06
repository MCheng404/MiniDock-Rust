//! Stamps the build version so `nooksill_core::VERSION` matches the EXE
//! resource and the artifact name. See `scripts/version.rs` for the rules.

include!("../../scripts/version.rs");

fn main() {
    println!("cargo:rerun-if-env-changed=NOOKSILL_VERSION");
    println!("cargo:rerun-if-env-changed=NOOKSILL_BUILD");
    println!("cargo:rerun-if-changed=../../scripts/version.rs");
    // Commits move the number; watching the reflog is cheaper than a full scan.
    println!("cargo:rerun-if-changed=../../.git/logs/HEAD");

    let v = resolve(env!("CARGO_PKG_VERSION"));
    println!("cargo:rustc-env=NOOKSILL_VERSION={}", v.text);

    // Hand the resolved value to `scripts/package.sh` instead of re-deriving it
    // in shell: the artifact name must be the version actually stamped into the
    // binary. OUT_DIR is `target/<triple>/<profile>/build/<pkg>-<hash>/out`.
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    if let Some(dir) = out.ancestors().nth(3) {
        let _ = std::fs::write(
            dir.join("NOOKSILL_VERSION"),
            format!(
                "{}
",
                v.text
            ),
        );
    }
}
