#![cfg(test)]
use std::{fs, path::Path};

use baukit_cli::{
    AuthProvider, Component, Components, NewOptions, QualityProfile, doctor, generate_gitops,
    generate_new,
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

fn render_with_gitops(name: &str, gitops: &str) -> anyhow::Result<std::path::PathBuf> {
    let seed = tempfile::tempdir()?;
    let seeded = generate_new(&strict_auth_options(seed.path(), name))?;
    let manifest = fs::read_to_string(seeded.join("baukit.toml"))?;
    let default = "[gitops]\ndomain = \"lab.example.com\"\nregistry = \"ghcr.io/example\"\n";
    assert!(manifest.contains(default), "{manifest}");
    let product = tempfile::tempdir()?.keep();
    fs::write(
        product.join("baukit.toml"),
        manifest.replace(default, gitops),
    )?;
    let mut options = strict_auth_options(&product, name);
    options.into_existing = true;
    generate_new(&options)
}

#[test]
fn gitops_overlays_take_domain_registry_and_realm_from_the_manifest() -> anyhow::Result<()> {
    let root = render_with_gitops(
        "lab-app",
        "[gitops]\ndomain = \"lab.home.test\"\nregistry = \"ghcr.io/owner\"\nrealm = \"shared\"\n",
    )?;
    let testing = fs::read_to_string(root.join("deploy/gitops/testing/kustomization.yaml"))?;
    assert!(testing.contains("Host(`lab-app-testing.lab.home.test`)"));
    assert!(testing.contains("namespace: lab-app-testing"));
    let production = fs::read_to_string(root.join("deploy/gitops/production/kustomization.yaml"))?;
    assert!(production.contains("Host(`lab-app.lab.home.test`)"));
    let environment = fs::read_to_string(root.join("deploy/gitops/production/environment.yaml"))?;
    assert!(environment.contains("value: https://id.lab.home.test/realms/shared"));
    assert!(environment.contains("value: lab-app-production"));
    let base = fs::read_to_string(root.join("deploy/gitops/base/helmrelease.yaml"))?;
    assert!(base.contains("repository: ghcr.io/owner/lab-app-api"));
    let release = fs::read_to_string(root.join("scripts/release.sh"))?;
    assert!(release.contains("registry=ghcr.io/owner\n"));
    Ok(())
}

#[test]
fn gitops_identity_url_overrides_the_default_host() -> anyhow::Result<()> {
    let root = render_with_gitops(
        "idp-app",
        "[gitops]\ndomain = \"lab.home.test\"\nregistry = \"ghcr.io/owner\"\nidentity_url = \"https://sso.home.test/\"\n",
    )?;
    let environment = fs::read_to_string(root.join("deploy/gitops/testing/environment.yaml"))?;
    assert!(environment.contains("value: https://sso.home.test/realms/idp-app"));
    Ok(())
}

#[test]
fn gitops_values_with_unsafe_characters_are_rejected() -> anyhow::Result<()> {
    for (gitops, field) in [
        (
            "[gitops]\ndomain = \"Lab.Example\"\nregistry = \"ghcr.io/owner\"\n",
            "gitops.domain",
        ),
        (
            "[gitops]\ndomain = \"lab.test\"\nregistry = \"ghcr.io/$(id)\"\n",
            "gitops.registry",
        ),
        (
            "[gitops]\ndomain = \"lab.test\"\nregistry = \"ghcr.io/owner\"\nidentity_url = \"http://id.lab.test\"\n",
            "gitops.identity_url",
        ),
    ] {
        let error = render_with_gitops("unsafe-gitops-app", gitops)
            .expect_err("unsafe gitops values must not reach generated files");
        assert!(error.to_string().contains(field), "{error}");
    }
    Ok(())
}

#[test]
fn linux_jobs_take_runners_from_repository_variables() -> anyhow::Result<()> {
    let seed = tempfile::tempdir()?;
    let root = generate_new(&strict_auth_options(seed.path(), "runner-app"))?;
    for workflow in ["ci.yml", "release.yml", "promote.yml"] {
        let source = fs::read_to_string(root.join(".github/workflows").join(workflow))?;
        let runners: Vec<&str> = source
            .lines()
            .filter_map(|line| line.trim().strip_prefix("runs-on: "))
            .collect();
        assert!(!runners.is_empty(), "{workflow}");
        for runner in runners {
            assert!(
                runner == "${{ fromJSON(vars.BAUKIT_RUNS_ON || '\"ubuntu-latest\"') }}"
                    || runner == "${{ fromJSON(vars.BAUKIT_GATE_RUNS_ON || '\"ubuntu-latest\"') }}",
                "{workflow}: {runner}"
            );
        }
    }
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml"))?;
    assert!(ci.contains("BAUKIT_GATE_RUNS_ON"));
    Ok(())
}

#[test]
fn generate_gitops_adds_missing_files_and_keeps_product_edits() -> anyhow::Result<()> {
    let seed = tempfile::tempdir()?;
    let root = generate_new(&strict_auth_options(seed.path(), "adopt-app"))?;
    fs::remove_dir_all(root.join("deploy/gitops/testing"))?;
    fs::remove_file(root.join(".github/workflows/release.yml"))?;
    fs::write(root.join("deploy/gitops/base/edge.yaml"), "product edit\n")?;
    let lines = generate_gitops(&root)?;
    assert!(
        lines.contains(&"added .github/workflows/release.yml".to_owned()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"added deploy/gitops/testing/release.yaml".to_owned()),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("kept deploy/gitops/base/edge.yaml"))
    );
    assert!(
        !lines.iter().any(|line| line.contains("ci.yml")),
        "{lines:?}"
    );
    assert_eq!(
        fs::read_to_string(root.join("deploy/gitops/base/edge.yaml"))?,
        "product edit\n"
    );
    Ok(())
}

#[test]
fn generate_gitops_needs_a_backend() -> anyhow::Result<()> {
    let seed = tempfile::tempdir()?;
    let mut options = strict_auth_options(seed.path(), "web-only-app");
    options.components = Components::from([Component::Web]);
    options.auth = None;
    let root = generate_new(&options)?;
    assert!(!root.join("deploy/gitops").exists());
    assert!(!root.join(".github/workflows/release.yml").exists());
    let error = generate_gitops(&root).expect_err("a product without backend has no images");
    assert!(error.to_string().contains("backend = false"));
    Ok(())
}
