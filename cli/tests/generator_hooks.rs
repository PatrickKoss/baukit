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

fn render_mobile_with_manifest_edit(name: &str, extra: &str) -> anyhow::Result<std::path::PathBuf> {
    let mobile_options = |directory: &Path| NewOptions {
        components: Components::from([Component::Backend, Component::Mobile]),
        auth: Some(AuthProvider::Clerk),
        ..strict_auth_options(directory, name)
    };
    let seed = tempfile::tempdir()?;
    let seeded = generate_new(&mobile_options(seed.path()))?;
    let manifest = fs::read_to_string(seeded.join("baukit.toml"))?;
    let product = tempfile::tempdir()?.keep();
    fs::write(product.join("baukit.toml"), format!("{manifest}\n{extra}"))?;
    let mut options = mobile_options(&product);
    options.into_existing = true;
    generate_new(&options)
}

#[test]
fn mobile_client_and_redirect_path_come_from_the_manifest() -> anyhow::Result<()> {
    let root = render_mobile_with_manifest_edit(
        "hero-app",
        "[auth]\nmobile_client_id = \"hero-native\"\nmobile_redirect_path = \"auth/callback\"\n",
    )?;
    let product = fs::read_to_string(root.join("mobile/src/product.ts"))?;
    assert!(product.contains("export const MOBILE_OIDC_CLIENT_ID = 'hero-native';"));
    assert!(product.contains("export const OIDC_REDIRECT_PATH = 'auth/callback';"));
    for path in ["mobile/src/sign-in/lab.ts", "mobile/src/sign-in/local.ts"] {
        let source = fs::read_to_string(root.join(path))?;
        assert!(source.contains("path: OIDC_REDIRECT_PATH"), "{path}");
        assert!(!source.contains("'oauth'"), "{path}");
    }
    let variant = fs::read_to_string(root.join("mobile/src/build-variant.ts"))?;
    assert!(variant.contains("|| MOBILE_OIDC_CLIENT_ID"));
    let realm = fs::read_to_string(root.join("keycloak/realm.json"))?;
    assert!(realm.contains("\"clientId\": \"hero-native\""));
    assert!(realm.contains(
        "\"redirectUris\": [\"hero-app://auth/callback\", \"hero-app-dev://auth/callback\"]"
    ));
    let reconcile = fs::read_to_string(root.join("keycloak/reconcile.json"))?;
    assert!(reconcile.contains("\"clientId\": \"hero-native\""));
    assert!(!reconcile.contains("hero-app-mobile"));
    Ok(())
}

#[test]
fn mobile_client_and_redirect_path_default_to_the_template_names() -> anyhow::Result<()> {
    let root = render_mobile_with_manifest_edit("plain-mobile-app", "")?;
    let product = fs::read_to_string(root.join("mobile/src/product.ts"))?;
    assert!(product.contains("export const MOBILE_OIDC_CLIENT_ID = 'plain-mobile-app-mobile';"));
    assert!(product.contains("export const OIDC_REDIRECT_PATH = 'oauth';"));
    let realm = fs::read_to_string(root.join("keycloak/realm.json"))?;
    assert!(realm.contains("\"plain-mobile-app://oauth\", \"plain-mobile-app-dev://oauth\""));
    Ok(())
}

#[test]
fn unsafe_mobile_client_or_redirect_path_is_rejected() {
    for (extra, field) in [
        (
            "[auth]\nmobile_client_id = \"a'b\"\n",
            "auth.mobile_client_id",
        ),
        (
            "[auth]\nmobile_redirect_path = \"/oauth\"\n",
            "auth.mobile_redirect_path",
        ),
        (
            "[auth]\nmobile_redirect_path = \"a//b\"\n",
            "auth.mobile_redirect_path",
        ),
    ] {
        let error = render_mobile_with_manifest_edit("unsafe-mobile-app", extra)
            .err()
            .map(|error| error.to_string());
        assert!(
            error
                .as_deref()
                .is_some_and(|message| message.contains(field)),
            "{extra}: {error:?}"
        );
    }
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
    let edge = fs::read_to_string(root.join("deploy/gitops/base/edge.yaml"))?;
    assert!(
        edge.contains(
            "      - main: lab.home.test\n        sans:\n          - \"*.lab.home.test\"\n"
        )
    );
    assert!(!edge.contains("tls: {}"));
    let base = fs::read_to_string(root.join("deploy/gitops/base/helmrelease.yaml"))?;
    assert!(base.contains("repository: ghcr.io/owner/lab-app-api"));
    let release = fs::read_to_string(root.join("scripts/release.sh"))?;
    assert!(release.contains("registry=ghcr.io/owner\n"));
    Ok(())
}

#[test]
fn gitops_default_registry_keeps_the_ghcr_pull_secret() -> anyhow::Result<()> {
    let root = render_with_gitops(
        "ghcr-app",
        "[gitops]\ndomain = \"lab.example.com\"\nregistry = \"ghcr.io/example\"\n",
    )?;
    for environment in ["testing", "production"] {
        let overlay = root.join("deploy/gitops").join(environment);
        assert!(overlay.join("ghcr-pull-secret.enc.yaml").is_file());
        let kustomization = fs::read_to_string(overlay.join("kustomization.yaml"))?;
        assert!(kustomization.contains("  - ghcr-pull-secret.enc.yaml\n"));
    }
    let base = fs::read_to_string(root.join("deploy/gitops/base/helmrelease.yaml"))?;
    assert!(base.contains("      - name: ghcr-app-ghcr\n"));
    assert!(base.contains("repository: ghcr.io/example/ghcr-app-api"));
    let release = fs::read_to_string(root.join("scripts/release.sh"))?;
    assert!(!release.contains("lab_registry"));
    let workflow = fs::read_to_string(root.join(".github/workflows/release.yml"))?;
    assert!(!workflow.contains("LAB_REGISTRY"));
    Ok(())
}

#[test]
fn gitops_lab_registry_pulls_from_the_lab_and_still_publishes_to_ghcr() -> anyhow::Result<()> {
    let root = render_with_gitops(
        "lab-reg-app",
        "[gitops]\ndomain = \"lab.home.test\"\nregistry = \"ghcr.io/owner\"\nlab_registry = \"registry.lab.home.test/owner\"\n",
    )?;
    for environment in ["testing", "production"] {
        let overlay = root.join("deploy/gitops").join(environment);
        assert!(!overlay.join("ghcr-pull-secret.enc.yaml").exists());
        let kustomization = fs::read_to_string(overlay.join("kustomization.yaml"))?;
        assert!(!kustomization.contains("ghcr-pull-secret"));
    }
    let base = fs::read_to_string(root.join("deploy/gitops/base/helmrelease.yaml"))?;
    assert!(!base.contains("imagePullSecrets"));
    assert!(base.contains("repository: registry.lab.home.test/owner/lab-reg-app-api"));
    assert!(base.contains("repository: registry.lab.home.test/owner/lab-reg-app-migrate"));
    let release = fs::read_to_string(root.join("scripts/release.sh"))?;
    assert!(release.contains("registry=ghcr.io/owner\n"));
    assert!(release.contains("lab_registry=registry.lab.home.test/owner\n"));
    assert!(release.contains("run cosign copy --force"));
    let workflow = fs::read_to_string(root.join(".github/workflows/release.yml"))?;
    assert!(workflow.contains("LAB_REGISTRY_USER: ${{ vars.LAB_REGISTRY_USER }}"));
    assert!(workflow.contains("LAB_REGISTRY_TOKEN: ${{ secrets.LAB_REGISTRY_TOKEN }}"));
    assert!(workflow.contains("REGISTRY_TOKEN: ${{ github.token }}"));
    let manifest = fs::read_to_string(root.join("baukit.toml"))?;
    assert!(manifest.contains("lab_registry = \"registry.lab.home.test/owner\""));
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
            "[gitops]\ndomain = \"lab.test\"\nregistry = \"ghcr.io/owner\"\nlab_registry = \"Registry.lab\"\n",
            "gitops.lab_registry",
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

const OWNER_INITDB: &str = "postgres/initdb/10-owner.sql";
const OWNER_TEST: &str = "backend/tests/postgres_integration.rs";

fn dedicated_cluster(root: &Path, environment: &str) -> anyhow::Result<String> {
    Ok(fs::read_to_string(root.join(format!(
        "deploy/gitops/{environment}/backing/postgres-dedicated/postgres.yaml"
    )))?)
}

#[test]
fn migrations_run_as_a_non_superuser_owner_by_default() -> anyhow::Result<()> {
    let seed = tempfile::tempdir()?;
    let root = generate_new(&strict_auth_options(seed.path(), "owner-app"))?;
    let initdb = fs::read_to_string(root.join(OWNER_INITDB))?;
    assert!(initdb.contains("CREATE ROLE app LOGIN PASSWORD 'app' NOSUPERUSER NOCREATEDB NOREPLICATION NOBYPASSRLS NOCREATEROLE;"));
    assert!(
        initdb.contains("EXECUTE format('ALTER DATABASE %I OWNER TO app', current_database());")
    );
    assert!(!initdb.contains("owner_app"));
    let compose = fs::read_to_string(root.join("compose.yaml"))?;
    assert!(compose.contains("./postgres/initdb:/docker-entrypoint-initdb.d:ro"));
    assert!(!compose.contains("postgres://postgres:postgres@"));
    let test = fs::read_to_string(root.join(OWNER_TEST))?;
    assert!(test.contains("baukit_test::PostgresMigrationOwner::default(),"));
    assert!(test.contains("owner_connection_url()"));
    for environment in ["testing", "production"] {
        let cluster = dedicated_cluster(&root, environment)?;
        assert!(cluster.contains("      owner: app\n  plugins:\n"));
        assert!(!cluster.contains("postInitApplicationSQL"));
    }
    assert!(!fs::read_to_string(root.join("baukit.toml"))?.contains("[postgres]"));
    Ok(())
}

#[test]
fn owner_create_role_reaches_dev_tests_and_the_dedicated_cluster() -> anyhow::Result<()> {
    let root = render_with_manifest_edit("roles-app", "[postgres]\nowner_create_role = true\n")?;
    let initdb = fs::read_to_string(root.join(OWNER_INITDB))?;
    assert!(initdb.contains("NOBYPASSRLS CREATEROLE;"));
    let test = fs::read_to_string(root.join(OWNER_TEST))?;
    assert!(test.contains("baukit_test::PostgresMigrationOwner::default().with_create_role(),"));
    for environment in ["testing", "production"] {
        let cluster = dedicated_cluster(&root, environment)?;
        assert!(cluster.contains(
            "      owner: app\n      # `[postgres] owner_create_role`. Runs once at initdb; an existing cluster needs it run by hand.\n      postInitApplicationSQL:\n        - ALTER ROLE app CREATEROLE\n  plugins:\n"
        ));
        assert!(!cluster.contains("managed:"));
    }
    assert!(
        fs::read_to_string(root.join("baukit.toml"))?
            .contains("\n[postgres]\nowner_create_role = true\n")
    );
    Ok(())
}

#[test]
fn doctor_requires_dedicated_postgres_for_owner_create_role() -> anyhow::Result<()> {
    let root =
        render_with_manifest_edit("shared-roles-app", "[postgres]\nowner_create_role = true\n")?;
    let error = doctor(&root).expect_err("owner_create_role on shared Postgres must fail");
    let message = error.to_string();
    assert!(
        message.contains("postgres.owner_create_role needs dedicated Postgres"),
        "{message}"
    );
    assert!(
        message.contains("deploy/gitops/production, deploy/gitops/testing"),
        "{message}"
    );

    for environment in ["testing", "production"] {
        let overlay = root
            .join("deploy/gitops")
            .join(environment)
            .join("kustomization.yaml");
        let source = fs::read_to_string(&overlay)?;
        assert!(source.contains("components: []"), "{source}");
        fs::write(
            &overlay,
            source.replace(
                "components: []",
                "components:\n  - backing/postgres-dedicated",
            ),
        )?;
    }
    if let Err(error) = doctor(&root) {
        assert!(
            !error.to_string().contains("postgres.owner_create_role"),
            "{error}"
        );
    }
    Ok(())
}

#[test]
fn doctor_fails_when_the_gitops_chart_tag_lags_the_template_version() -> anyhow::Result<()> {
    let seed = tempfile::tempdir()?;
    let root = generate_new(&strict_auth_options(seed.path(), "chart-tag-app"))?;
    let chart_source = root.join("deploy/gitops/base/chart-source.yaml");
    let source = fs::read_to_string(&chart_source)?;
    let current = format!("tag: v{}\n", env!("CARGO_PKG_VERSION"));
    assert!(source.contains(&current), "{source}");
    if let Err(error) = doctor(&root) {
        assert!(!error.to_string().contains("chart-source.yaml"), "{error}");
    }
    fs::write(&chart_source, source.replace(&current, "tag: v0.28.7\n"))?;
    let message = doctor(&root)
        .expect_err("a stale chart tag must fail the doctor")
        .to_string();
    assert!(
        message.contains(&format!(
            "deploy/gitops/base/chart-source.yaml sets ref.tag v0.28.7, but template_version is {}",
            env!("CARGO_PKG_VERSION")
        )),
        "{message}"
    );
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
