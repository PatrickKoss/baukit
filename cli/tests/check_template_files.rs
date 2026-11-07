use std::{fs, path::Path, process::Command};

use anyhow::{Context, Result, ensure};
use baukit_cli::{Component, Components, NewOptions, QualityProfile, generate_new};

const FINDINGS_EXIT: i32 = 1;

fn check_template_files(root: &Path, extra: &[&str]) -> Result<(i32, String)> {
    let output = Command::new(env!("CARGO_BIN_EXE_baukit"))
        .args(["check", "template-files"])
        .args(extra)
        .arg("--root")
        .arg(root)
        .output()?;
    let code = output.status.code().context("baukit was killed")?;
    Ok((code, String::from_utf8(output.stdout)?))
}

#[test]
fn fresh_product_passes_and_an_edited_script_fails_until_updated() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&NewOptions {
        name: "owned-files".to_owned(),
        directory: parent.path().to_path_buf(),
        components: Components::from([Component::Backend, Component::Web]),
        auth: None,
        force: false,
        into_existing: false,
        resolve_lockfiles: false,
        baukit_path: None,
        port_offset: 0,
        quality: QualityProfile::Strict,
    })?;
    let manifest = fs::read_to_string(root.join(".baukit/template-files.toml"))?;
    for owned in [
        "scripts/quality-gate.sh",
        "scripts/reconcile-env.py",
        ".github/workflows/ci.yml",
        "docs/quality-gate.md",
        "web/scripts/build-sw.mjs",
    ] {
        ensure!(
            manifest.contains(&format!("\"{owned}\" = ")),
            "{owned} missing from\n{manifest}"
        );
    }
    ensure!(
        !manifest.contains("\"backend/"),
        "backend code must stay product-owned:\n{manifest}"
    );

    let (code, output) = check_template_files(&root, &[])?;
    ensure!(code == 0, "{output}");
    assert_eq!(output, "template-files: ok\n");

    let gate = root.join("scripts/quality-gate.sh");
    let mut forked = fs::read_to_string(&gate)?;
    forked.push_str("echo product step\n");
    fs::write(&gate, forked)?;
    let (code, output) = check_template_files(&root, &[])?;
    ensure!(code == FINDINGS_EXIT, "{output}");
    assert_eq!(
        output,
        "template-files: scripts/quality-gate.sh: template-owned file differs from the template; move the change into scripts/quality-gate.local.sh and restore the file\ntemplate-files: 1 finding\n"
    );

    let (code, output) = check_template_files(&root, &["--update"])?;
    ensure!(code == 0, "{output}");
    let (code, output) = check_template_files(&root, &[])?;
    ensure!(code == 0, "{output}");
    Ok(())
}
