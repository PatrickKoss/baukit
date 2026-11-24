#![cfg(all(test, unix))]
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use baukit_cli::{AuthProvider, Component, Components, NewOptions, QualityProfile, generate_new};

fn mobile_options(parent: &Path, name: &str, auth: Option<AuthProvider>) -> NewOptions {
    NewOptions {
        name: name.to_owned(),
        directory: parent.to_path_buf(),
        components: Components::from([Component::Backend, Component::Mobile]),
        auth,
        force: false,
        into_existing: false,
        resolve_lockfiles: false,
        baukit_path: None,
        port_offset: 0,
        quality: QualityProfile::Strict,
    }
}

fn package_script(root: &Path, name: &str) -> anyhow::Result<String> {
    let package: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("mobile/package.json"))?)?;
    package["scripts"][name]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("mobile/package.json has no `{name}` script"))
}

fn node_test_files(root: &Path) -> anyhow::Result<Vec<String>> {
    let mut files = fs::read_dir(root.join("mobile/scripts"))?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<anyhow::Result<Vec<_>>>()?;
    files.retain(|name| name.ends_with(".test.mjs"));
    files.sort();
    Ok(files)
}

#[test]
fn mobile_test_scripts_cover_every_node_test() -> anyhow::Result<()> {
    for auth in [None, Some(AuthProvider::Oidc), Some(AuthProvider::Clerk)] {
        let parent = tempfile::tempdir()?;
        let root = generate_new(&mobile_options(parent.path(), "node-tests", auth))?;
        let command = package_script(&root, "test:scripts")?;
        let files = node_test_files(&root)?;
        assert!(!files.is_empty());
        for file in &files {
            assert!(
                command.contains(&format!("scripts/{file}")),
                "{auth:?}: {file} missing from `{command}`"
            );
        }
        assert_eq!(
            package_script(&root, "test")?,
            "jest && pnpm run test:scripts"
        );
    }
    Ok(())
}

fn gate_function(gate: &str, name: &str) -> anyhow::Result<String> {
    let start = gate
        .find(&format!("{name}() {{"))
        .ok_or_else(|| anyhow::anyhow!("{name} missing"))?;
    let end = gate[start..]
        .find("\n}\n")
        .ok_or_else(|| anyhow::anyhow!("{name} unterminated"))?;
    Ok(gate[start..start + end + 3].to_owned())
}

fn fake_corepack(directory: &Path) -> anyhow::Result<PathBuf> {
    let path = directory.join("corepack");
    fs::write(&path, "#!/bin/sh\necho \"$*\"\n")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

#[test]
fn mobile_unit_gate_step_runs_the_node_tests_in_both_modes() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&mobile_options(parent.path(), "unit-gate", None))?;
    let step = gate_function(
        &fs::read_to_string(root.join("scripts/quality-gate.sh"))?,
        "step_mobile_unit",
    )?;
    let tools = tempfile::tempdir()?;
    fake_corepack(tools.path())?;
    for mode in ["full", "affected"] {
        let script = format!(
            "set -eu\npnpm_package=pnpm workers=2 gate_mode={mode} BAUKIT_GATE_BASE=main\n{step}step_mobile_unit\n"
        );
        let output = Command::new("sh")
            .args(["-c", &script])
            .env(
                "PATH",
                format!("{}:{}", tools.path().display(), std::env::var("PATH")?),
            )
            .output()?;
        assert!(output.status.success(), "{mode}");
        let stdout = String::from_utf8(output.stdout)?;
        let mut lines = stdout.lines();
        assert_eq!(
            lines.next(),
            Some("pnpm --dir mobile run test:scripts"),
            "{mode}"
        );
        let jest = lines.next().unwrap_or_default();
        assert!(
            if mode == "full" {
                jest.contains("run test:coverage")
            } else {
                jest.contains("exec jest --changedSince=main")
            },
            "{mode}: {jest}"
        );
    }
    Ok(())
}

fn step_key(step: &serde_yaml_ng::Value) -> String {
    match step {
        serde_yaml_ng::Value::String(command) => command.clone(),
        serde_yaml_ng::Value::Mapping(mapping) => mapping
            .iter()
            .next()
            .map(|(command, argument)| {
                let argument = argument
                    .get("id")
                    .unwrap_or(argument)
                    .as_str()
                    .unwrap_or_default();
                format!("{} {argument}", command.as_str().unwrap_or_default())
            })
            .unwrap_or_default(),
        _ => String::new(),
    }
}

#[test]
fn keycloak_sign_in_flows_hide_the_keyboard_after_each_field() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&mobile_options(
        parent.path(),
        "maestro-login",
        Some(AuthProvider::Oidc),
    ))?;
    let mut sign_in_flows = 0;
    for entry in fs::read_dir(root.join("mobile/.maestro"))? {
        let source = fs::read_to_string(entry?.path())?;
        let Some(flow) = source.split("\n---\n").nth(1) else {
            continue;
        };
        let steps: Vec<serde_yaml_ng::Value> = serde_yaml_ng::from_str(flow)?;
        let keys = steps.iter().map(step_key).collect::<Vec<_>>();
        let Some(login) = keys.iter().position(|key| key == "tapOn kc-login") else {
            continue;
        };
        sign_in_flows += 1;
        let username = keys
            .iter()
            .position(|key| key == "tapOn username")
            .ok_or_else(|| anyhow::anyhow!("no username tap"))?;
        assert_eq!(
            keys[username..=login],
            [
                "tapOn username",
                "inputText test",
                "hideKeyboard",
                "tapOn password",
                "inputText development-password",
                "hideKeyboard",
                "tapOn kc-login",
            ]
        );
    }
    assert_eq!(sign_in_flows, 1);
    Ok(())
}
