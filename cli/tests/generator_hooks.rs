#![cfg(test)]
use std::{fs, path::Path};

use baukit_cli::{
    AuthProvider, Component, Components, NewOptions, QualityProfile, doctor, generate_new,
};

fn strict_auth_options(directory: &Path, name: &str) -> NewOptions {
    NewOptions {
        name: name.to_owned(),
        directory: directory.to_path_buf(),
        components: Components::from([Component::Backend, Component::Web]),
        auth: Some(AuthProvider::Oidc),
        force: false,
        into_existing: false,
        resolve_lockfiles: false,
        baukit_path: Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../rust")),
        port_offset: 0,
        quality: QualityProfile::Strict,
    }
}

fn render_with_manifest_edit(name: &str, extra: &str) -> anyhow::Result<std::path::PathBuf> {
    let seed = tempfile::tempdir()?;
    let seeded = generate_new(&strict_auth_options(seed.path(), name))?;
    let manifest = fs::read_to_string(seeded.join("baukit.toml"))?;
    let product = tempfile::tempdir()?.keep();
    fs::write(product.join("baukit.toml"), format!("{manifest}\n{extra}"))?;
    let mut options = strict_auth_options(&product, name);
    options.into_existing = true;
    generate_new(&options)
}

#[test]
fn ci_settings_and_login_theme_come_from_the_manifest() -> anyhow::Result<()> {
    let root = render_with_manifest_edit(
        "hooked-app",
        "[auth]\nlogin_theme = \"hooked\"\n\n[quality.ci]\ntimeout_minutes = 180\n\n[quality.ci.env]\nBAUKIT_QA_HEADLESS = \"1\"\nPLAYWRIGHT_BROWSERS_PATH = \"${{ github.workspace }}/web/.playwright-browsers\"\n",
    )?;
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml"))?;
    assert!(ci.contains("timeout-minutes: 180"));
    assert!(ci.contains("      BAUKIT_QA_HEADLESS: \"1\"\n"));
    assert!(ci.contains(
        "      PLAYWRIGHT_BROWSERS_PATH: \"${{ github.workspace }}/web/.playwright-browsers\"\n"
    ));
    assert!(!ci.contains("PLAYWRIGHT_BROWSERS_PATH: web/node_modules"));
    assert!(!ci.contains("pnpm@"));
    let realm = fs::read_to_string(root.join("keycloak/realm.json"))?;
    assert!(realm.contains("\"loginTheme\": \"hooked\""));
    let script = fs::read_to_string(root.join("scripts/test-keycloak-theme-patches.sh"))?;
    assert!(script.contains("KEYCLOAK_LOGIN_THEME=\"hooked\""));
    Ok(())
}

#[test]
fn defaults_keep_the_template_timeout_theme_and_browser_cache() -> anyhow::Result<()> {
    let seed = tempfile::tempdir()?;
    let root = generate_new(&strict_auth_options(seed.path(), "plain-app"))?;
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml"))?;
    assert!(ci.contains("timeout-minutes: 90"));
    assert!(ci.contains("PLAYWRIGHT_BROWSERS_PATH: web/node_modules/.cache/playwright-browsers"));
    let realm = fs::read_to_string(root.join("keycloak/realm.json"))?;
    assert!(realm.contains("\"loginTheme\": \"baukit-accessible\""));
    let script = fs::read_to_string(root.join("scripts/test-keycloak-theme-patches.sh"))?;
    assert!(script.contains("KEYCLOAK_LOGIN_THEME=\"baukit-accessible\""));
    Ok(())
}

#[test]
fn a_login_theme_with_shell_characters_is_rejected() -> anyhow::Result<()> {
    let error = render_with_manifest_edit("bad-theme-app", "[auth]\nlogin_theme = \"a\\\"; rm\"\n")
        .expect_err("unsafe theme names must not reach generated scripts");
    assert!(error.to_string().contains("auth.login_theme"));
    Ok(())
}

#[test]
fn doctor_rejects_invalid_ci_settings() -> anyhow::Result<()> {
    let seed = tempfile::tempdir()?;
    let root = generate_new(&strict_auth_options(seed.path(), "doctor-ci-app"))?;
    let manifest = fs::read_to_string(root.join("baukit.toml"))?;
    fs::write(
        root.join("baukit.toml"),
        format!(
            "{manifest}\n[quality.ci]\ntimeout_minutes = 0\n\n[quality.ci.env]\nlower = \"x\"\n"
        ),
    )?;
    let error = doctor(&root).expect_err("invalid ci settings must fail the doctor");
    let message = error.to_string();
    assert!(message.contains("quality.ci.timeout_minutes"));
    assert!(message.contains("quality.ci.env key `lower`"));
    Ok(())
}
