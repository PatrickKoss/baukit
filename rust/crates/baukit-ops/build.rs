use std::{env, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=RUSTC");

    let rustc = env::var_os("RUSTC").expect("Cargo always supplies RUSTC to build scripts");
    let output = Command::new(rustc)
        .arg("--version")
        .output()
        .expect("failed to query rustc version");
    assert!(output.status.success(), "rustc --version failed");
    let output = String::from_utf8(output.stdout).expect("rustc --version returned non-UTF-8");
    let version = output
        .split_whitespace()
        .nth(1)
        .expect("rustc --version omitted its semantic version");

    println!("cargo:rustc-env=BAUKIT_RUST_VERSION={version}");
}
