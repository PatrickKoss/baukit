use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use baukit_cli::{
    AuthProvider, NewOptions, OpenApiCompatibility, QualityProfile, doctor, generate_new,
};
use sha2::{Digest, Sha256};

#[cfg(unix)]
use std::{
    env,
    os::unix::{fs::PermissionsExt, net::UnixListener},
};

fn options(parent: &Path, name: &str) -> NewOptions {
    NewOptions {
        name: name.to_owned(),
        directory: parent.to_path_buf(),
        backend: true,
        worker: false,
        mobile: false,
        web: false,
        pwa: false,
        mcp: false,
        auth: None,
        force: false,
        into_existing: false,
        resolve_lockfiles: false,
        baukit_path: None,
        port_offset: 0,
        quality: QualityProfile::Standard,
    }
}

fn frontend_options(parent: &Path, name: &str, mobile: bool, web: bool) -> NewOptions {
    NewOptions {
        name: name.to_owned(),
        directory: parent.to_path_buf(),
        backend: false,
        worker: false,
        mobile,
        web,
        pwa: false,
        mcp: false,
        auth: None,
        force: false,
        into_existing: false,
        resolve_lockfiles: false,
        baukit_path: None,
        port_offset: 0,
        quality: QualityProfile::Standard,
    }
}

fn verify_corepack_bootstrap(path: &Path) -> anyhow::Result<()> {
    let workflow: serde_yaml_ng::Value = serde_yaml_ng::from_str(&fs::read_to_string(path)?)?;
    let jobs = workflow["jobs"]
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("{} has no jobs", path.display()))?;
    let mut invocations = 0;
    for (name, job) in jobs {
        let Some(steps) = job["steps"].as_sequence() else {
            continue;
        };
        let mut installed = false;
        for step in steps {
            if step["uses"]
                .as_str()
                .is_some_and(|action| action.starts_with("actions/setup-node@"))
            {
                installed = false;
            }
            let Some(run) = step["run"].as_str() else {
                continue;
            };
            for command in run.lines().map(str::trim) {
                if command.starts_with("npm install --global corepack@") {
                    installed = true;
                }
                if command.contains("corepack ") {
                    anyhow::ensure!(
                        installed,
                        "{} job {name:?} uses Corepack before installing it: {command}",
                        path.display()
                    );
                    invocations += 1;
                }
            }
        }
    }
    anyhow::ensure!(invocations > 0, "{} never uses Corepack", path.display());
    Ok(())
}

#[test]
fn workflow_jobs_install_corepack_before_invoking_it() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut combined = options(parent.path(), "corepack-product");
    combined.web = true;
    combined.mobile = true;
    combined.mcp = true;
    combined.auth = Some(AuthProvider::Oidc);
    combined.quality = QualityProfile::Strict;
    let root = generate_new(&combined)?;
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for path in [
        repository.join(".github/workflows/ci.yml"),
        repository.join(".github/workflows/release.yml"),
        root.join(".github/workflows/ci.yml"),
        root.join(".github/workflows/native.yml"),
    ] {
        verify_corepack_bootstrap(&path)?;
    }
    Ok(())
}

#[test]
fn longest_application_name_generates_every_capability() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let name = "a".repeat(41);
    let mut generated = options(parent.path(), &name);
    generated.worker = true;
    generated.mobile = true;
    generated.web = true;
    generated.mcp = true;
    generated.auth = Some(AuthProvider::Oidc);
    let root = generate_new(&generated)?;
    let manifest = baukit_cli::read_manifest(&root)?;
    assert_eq!(manifest.app.name, name);
    for path in ["mobile/src/product.ts", "web/src/product.ts"] {
        assert!(
            fs::read_to_string(root.join(path))?
                .contains(&format!("export const PRODUCT_NAME = '{name}';"))
        );
    }
    assert!(
        root.join(format!("backend/crates/{name}-worker/src/lib.rs"))
            .is_file()
    );
    Ok(())
}

#[test]
fn application_name_over_the_service_limit_is_rejected() {
    let parent = tempfile::tempdir().expect("temporary directory");
    let name = "a".repeat(42);
    let error = generate_new(&options(parent.path(), &name))
        .expect_err("a 42-character product would overflow the worker operations Service name");
    assert_eq!(
        error.to_string(),
        format!(
            "invalid application name `{name}`; maximum length is 41 ASCII characters because generated Kubernetes Service names append `-baukit-app-worker-ops` and must fit the 63-character limit"
        )
    );
    assert!(!parent.path().join(name).exists());
}

#[test]
fn backend_generation_matches_golden_tree_and_is_deterministic() -> anyhow::Result<()> {
    let first_parent = tempfile::tempdir()?;
    let second_parent = tempfile::tempdir()?;
    let first = generate_new(&options(first_parent.path(), "snapshot-app"))?;
    let second = generate_new(&options(second_parent.path(), "snapshot-app"))?;

    let first_tree = read_tree(&first)?;
    let second_tree = read_tree(&second)?;
    assert_eq!(
        first_tree, second_tree,
        "same inputs must produce identical bytes"
    );

    let actual = render_hash_snapshot(&first_tree);
    let expected = include_str!("snapshots/backend.tree");
    assert_eq!(actual, expected, "generated backend tree changed");
    let compose = fs::read_to_string(first.join("compose.yaml"))?;
    assert!(compose.contains("image: postgres:18.6-alpine"));
    assert!(compose.contains("postgres-data:/var/lib/postgresql\n"));
    let dockerfile = fs::read_to_string(first.join("backend/Dockerfile"))?;
    assert!(dockerfile.contains("ARG GIT_COMMIT=unknown"));
    assert!(dockerfile.contains("ENV GIT_COMMIT=${GIT_COMMIT}"));
    Ok(())
}

#[test]
fn worker_generation_matches_golden_tree_and_records_capability() -> anyhow::Result<()> {
    let first_parent = tempfile::tempdir()?;
    let second_parent = tempfile::tempdir()?;
    let mut first_options = options(first_parent.path(), "snapshot-app");
    first_options.worker = true;
    let mut second_options = options(second_parent.path(), "snapshot-app");
    second_options.worker = true;

    let first = generate_new(&first_options)?;
    let second = generate_new(&second_options)?;
    let first_tree = read_tree(&first)?;
    assert_eq!(first_tree, read_tree(&second)?);
    assert_eq!(
        render_hash_snapshot(&first_tree),
        include_str!("snapshots/worker.tree")
    );

    let manifest = baukit_cli::read_manifest(&first)?;
    assert!(manifest.capabilities.backend);
    assert!(manifest.capabilities.worker);
    assert!(
        first
            .join("backend/crates/snapshot-app-worker/src/lib.rs")
            .is_file()
    );
    assert!(
        first
            .join("backend/crates/snapshot-app-bin/src/bin/worker.rs")
            .is_file()
    );
    assert!(
        first
            .join("backend/migrations/0003_baukit_jobs.sql")
            .is_file()
    );
    assert!(fs::read_to_string(first.join("deploy/values.yaml"))?.contains("enabled: true"));
    assert!(fs::read_to_string(first.join("Makefile"))?.contains("run-worker:"));
    assert!(
        fs::read_to_string(first.join("backend/crates/snapshot-app-bin/src/bin/migrate.rs"))?
            .contains("BaukitConfig<ProductConfig>")
    );
    Ok(())
}

#[test]
fn generated_backend_is_rustfmt_clean_across_product_names() -> anyhow::Result<()> {
    let maximum_name = "a".repeat(41);
    for name in [
        "aaa",
        "zeta",
        "solo-leveling-system-companion",
        &maximum_name,
    ] {
        let parent = tempfile::tempdir()?;
        let mut generated_options = options(parent.path(), name);
        generated_options.worker = true;
        generated_options.mcp = true;
        generated_options.auth = Some(AuthProvider::Oidc);
        let root = generate_new(&generated_options)?;
        let tree = read_tree(&root.join("backend"))?;
        let rust_sources = tree
            .keys()
            .filter(|path| path.extension().is_some_and(|extension| extension == "rs"));
        let output = Command::new("rustfmt")
            .args(["--edition", "2024", "--check"])
            .args(rust_sources.map(|path| root.join("backend").join(path)))
            .output()?;
        assert!(
            output.status.success(),
            "generated backend for {name} is not rustfmt-clean:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}

#[test]
fn mobile_generation_matches_golden_tree_and_is_deterministic() -> anyhow::Result<()> {
    assert_deterministic_snapshot(
        |parent| frontend_options(parent, "snapshot-app", true, false),
        include_str!("snapshots/mobile.tree"),
    )
}

#[test]
fn generated_native_qa_targets_are_platform_specific_and_shell_valid() -> anyhow::Result<()> {
    let mobile_parent = tempfile::tempdir()?;
    let mobile = generate_new(&frontend_options(
        mobile_parent.path(),
        "qa-mobile",
        true,
        false,
    ))?;
    let makefile = fs::read_to_string(mobile.join("mobile/Makefile"))?;
    assert!(makefile.contains("qa-android:"));
    assert!(makefile.contains("qa-ios:"));
    assert!(makefile.contains("e2e-android:"));
    assert!(makefile.contains("e2e-ios:"));
    assert!(mobile.join("mobile/.maestro/smoke.yaml").is_file());
    let app_config = fs::read_to_string(mobile.join("mobile/app.config.ts"))?;
    assert!(app_config.contains("with-qa-local-network.cjs"));
    assert!(
        mobile
            .join("mobile/plugins/with-qa-local-network.cjs")
            .is_file()
    );

    let combined_parent = tempfile::tempdir()?;
    let mut combined_options = options(combined_parent.path(), "qa-native");
    combined_options.mobile = true;
    combined_options.auth = Some(AuthProvider::Oidc);
    let combined = generate_new(&combined_options)?;
    let root_makefile = fs::read_to_string(combined.join("Makefile"))?;
    assert!(root_makefile.contains("qa-android:"));
    assert!(root_makefile.contains("qa-ios:"));
    let smoke = fs::read_to_string(combined.join("mobile/.maestro/smoke.yaml"))?;
    assert!(smoke.contains("appId: ${APP_ID}"));
    assert!(smoke.contains("Sign in with local Keycloak"));
    assert!(smoke.contains("development-password"));
    assert!(smoke.contains("- scrollUntilVisible:\n    element:\n      text: Allow\n"));
    let compose = fs::read_to_string(combined.join("mobile/scripts/qa/docker-compose.qa.yml"))?;
    assert!(compose.contains("BAUKIT_QA_POSTGRES_PORT"));
    assert!(compose.contains("BAUKIT_QA_REDIS_PORT"));
    assert!(compose.contains("BAUKIT_QA_KEYCLOAK_PORT"));

    for entry in fs::read_dir(combined.join("mobile/scripts/qa"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "sh") {
            let output = Command::new("bash").args(["-n"]).arg(&path).output()?;
            assert!(
                output.status.success(),
                "{} is not valid bash:\n{}",
                path.display(),
                String::from_utf8_lossy(&output.stderr),
            );
        }
    }
    for root in [&mobile, &combined] {
        let output = Command::new("python3")
            .arg(root.join("mobile/scripts/qa/test_android.py"))
            .output()?;
        assert!(
            output.status.success(),
            "Generated Android QA script tests failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}

#[test]
fn web_generation_matches_golden_tree_and_is_deterministic() -> anyhow::Result<()> {
    assert_deterministic_snapshot(
        |parent| frontend_options(parent, "snapshot-app", false, true),
        include_str!("snapshots/web.tree"),
    )
}

#[test]
fn generated_browser_qa_configures_authenticated_and_unauthenticated_cases() -> anyhow::Result<()> {
    let unauthenticated_parent = tempfile::tempdir()?;
    let unauthenticated = generate_new(&frontend_options(
        unauthenticated_parent.path(),
        "qa-public",
        false,
        true,
    ))?;
    let authenticated_parent = tempfile::tempdir()?;
    let mut authenticated_options = options(authenticated_parent.path(), "qa-private");
    authenticated_options.mobile = true;
    authenticated_options.web = true;
    authenticated_options.auth = Some(AuthProvider::Oidc);
    let authenticated = generate_new(&authenticated_options)?;

    let public_config = fs::read_to_string(unauthenticated.join("web/e2e/qa.config.ts"))?;
    assert!(public_config.contains("heading: new RegExp(`^${PRODUCT_NAME}$`, 'u')"));
    assert!(
        fs::read_to_string(unauthenticated.join("web/src/product.ts"))?
            .contains("PRODUCT_NAME = 'qa-public'")
    );
    assert!(public_config.contains("fields: ["));
    assert!(public_config.contains("invalidField: 'Example name'"));
    assert!(public_config.contains("recoveryRole: 'button'"));
    assert!(public_config.contains("recoveryRole: 'link'"));
    assert!(public_config.contains("apiStubs: ITEM_API_STUBS"));
    assert!(!public_config.contains("authenticated: true"));
    assert!(!public_config.contains("${PRODUCT_NAME}:oidc:tokens"));

    let private_config = fs::read_to_string(authenticated.join("web/e2e/qa.config.ts"))?;
    assert!(private_config.contains("authenticated: true"));
    assert!(private_config.contains("key: `${PRODUCT_NAME}:oidc:tokens`"));
    assert!(
        fs::read_to_string(authenticated.join("web/src/product.ts"))?
            .contains("PRODUCT_NAME = 'qa-private'")
    );
    assert!(private_config.contains("subject: 'qa-owner'"));
    assert!(private_config.contains("subject: 'qa-other'"));

    let keyboard = fs::read_to_string(authenticated.join("web/e2e/tests/qa-keyboard.spec.ts"))?;
    assert!(keyboard.contains("'inert' in HTMLElement.prototype"));
    assert!(keyboard.contains("MAX_FOCUS_SEARCH_PRESSES"));
    assert!(keyboard.contains("focus path:"));
    assert!(keyboard.contains("exact: true"));
    let route_state =
        fs::read_to_string(authenticated.join("web/e2e/tests/qa-route-state.spec.ts"))?;
    assert!(route_state.contains("withholds its settled state until the first load resolves"));
    let isolation =
        fs::read_to_string(authenticated.join("web/e2e/tests/qa-auth-isolation.spec.ts"))?;
    assert!(isolation.contains("Isolation needs two configured accounts."));
    let geometry = fs::read_to_string(authenticated.join("web/e2e/tests/geometry.ts"))?;
    assert!(geometry.contains("largest elements"));
    let console = fs::read_to_string(authenticated.join("web/e2e/tests/console-warnings.ts"))?;
    assert!(console.contains("Service Worker registration blocked by Playwright"));
    let playwright = fs::read_to_string(authenticated.join("web/e2e/playwright.config.ts"))?;
    assert!(playwright.contains("fileURLToPath(new URL('..', import.meta.url))"));
    assert!(playwright.contains("cwd: webRoot"));

    Ok(())
}

#[test]
fn combined_generation_matches_golden_tree_and_records_capabilities() -> anyhow::Result<()> {
    let first_parent = tempfile::tempdir()?;
    let second_parent = tempfile::tempdir()?;
    let mut first_options = options(first_parent.path(), "snapshot-app");
    first_options.mobile = true;
    first_options.web = true;
    let mut second_options = options(second_parent.path(), "snapshot-app");
    second_options.mobile = true;
    second_options.web = true;

    let first = generate_new(&first_options)?;
    let second = generate_new(&second_options)?;
    let first_tree = read_tree(&first)?;
    let second_tree = read_tree(&second)?;
    assert_eq!(
        first_tree, second_tree,
        "combined generation must be stable"
    );
    assert_eq!(
        render_hash_snapshot(&first_tree),
        include_str!("snapshots/combined.tree"),
        "generated combined tree changed"
    );

    let manifest = baukit_cli::read_manifest(&first)?;
    assert!(manifest.capabilities.backend);
    assert!(!manifest.capabilities.worker);
    assert!(manifest.capabilities.mobile);
    assert!(manifest.capabilities.web);
    assert!(!manifest.capabilities.pwa);
    assert_eq!(manifest.capabilities.auth, None);
    assert_eq!(manifest.quality.profile, QualityProfile::Standard);
    assert_eq!(manifest.quality.backend_coverage_lines, 70);
    assert_eq!(manifest.quality.webkit_repeats, 3);
    assert!(manifest.quality.critical_paths.is_empty());
    assert!(!manifest.quality.full_stack_e2e);
    assert_eq!(
        manifest.quality.openapi_compatibility,
        OpenApiCompatibility::Off
    );
    assert_eq!(manifest.openapi.consumers(), ["generated/openapi.d.ts"]);
    assert!(!fs::read_to_string(first.join("baukit.toml"))?.contains("auth"));
    assert!(!first.join("scripts/quality-gate.sh").exists());
    assert!(first.join("backend/Cargo.toml").is_file());
    assert!(first.join("mobile/app/_layout.tsx").is_file());
    assert!(first.join("mobile/app/(tabs)/index.tsx").is_file());
    assert!(!first.join("mobile/App.tsx").exists());
    assert!(first.join("web/src/App.tsx").is_file());
    Ok(())
}

#[test]
fn legacy_manifest_defaults_to_standard_quality_and_legacy_consumer() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path(), "legacy-app"))?;
    let path = root.join("baukit.toml");
    let source = fs::read_to_string(&path)?
        .replace(
            "[quality]\nprofile = \"standard\"\nbackend_coverage_lines = 70\ncritical_paths = []\nwebkit_repeats = 3\nfull_stack_e2e = false\nopenapi_compatibility = \"off\"\n\n",
            "",
        )
        .replace(
            "consumers = [\"generated/openapi.d.ts\"]",
            "typescript = \"generated/openapi.d.ts\"",
        );
    fs::write(path, source)?;

    let manifest = baukit_cli::read_manifest(&root)?;
    assert_eq!(manifest.quality.profile, QualityProfile::Standard);
    assert_eq!(manifest.openapi.consumers(), ["generated/openapi.d.ts"]);
    Ok(())
}

#[cfg(unix)]
#[test]
fn strict_coverage_creates_lcov_parent_with_an_external_target() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let parent = tempfile::tempdir()?;
    let mut strict = options(parent.path(), "external-coverage");
    strict.quality = QualityProfile::Strict;
    let root = generate_new(&strict)?;
    let runner = fs::read_to_string(root.join("scripts/quality-gate.sh"))?;
    let start = runner
        .find("cargo fmt --manifest-path backend/Cargo.toml")
        .expect("backend gate");
    let end = runner[start..]
        .find("rust_version=")
        .expect("coverage gate end")
        + start;
    let commands = format!(
        "set -eu\nmanifest_value() {{ echo 70; }}\n{}",
        &runner[start..end]
    );
    let tools = parent.path().join("tools");
    fs::create_dir(&tools)?;
    let cargo = tools.join("cargo");
    fs::write(
        &cargo,
        r#"#!/bin/sh
set -eu
mkdir -p "$CARGO_TARGET_DIR/llvm-cov"
while [ "$#" -gt 0 ]; do
  if [ "$1" = --output-path ]; then
    printf 'TN:external-target\n' > "$2"
    exit 0
  fi
  shift
done
"#,
    )?;
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755))?;
    let output = Command::new("sh")
        .args(["-c", &commands])
        .current_dir(&root)
        .env(
            "PATH",
            format!("{}:{}", tools.display(), std::env::var("PATH")?),
        )
        .env("CARGO_TARGET_DIR", parent.path().join("external-target"))
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.join("backend/target/llvm-cov/lcov.info"))?,
        "TN:external-target\n"
    );
    assert!(parent.path().join("external-target/llvm-cov").is_dir());
    Ok(())
}

#[test]
fn strict_generation_is_capability_driven_and_matches_golden_tree() -> anyhow::Result<()> {
    let cases = [
        ("backend", true, false, false),
        ("web", false, false, true),
        ("mobile", false, true, false),
        ("combined", true, true, true),
    ];

    for (name, backend, mobile, web) in cases {
        let parent = tempfile::tempdir()?;
        let mut strict = if backend {
            options(parent.path(), &format!("strict-{name}"))
        } else {
            frontend_options(parent.path(), &format!("strict-{name}"), mobile, web)
        };
        strict.mobile = mobile;
        strict.web = web;
        strict.quality = QualityProfile::Strict;
        let root = generate_new(&strict)?;
        let manifest = baukit_cli::read_manifest(&root)?;
        assert_eq!(manifest.quality.profile, QualityProfile::Strict);

        let runner = fs::read_to_string(root.join("scripts/quality-gate.sh"))?;
        assert!(runner.contains("check-markdown-links.test.py"));
        assert!(runner.contains("check-markdown-links.py README.md CLAUDE.md AGENTS.md docs"));
        assert_eq!(runner.contains("cargo llvm-cov nextest"), backend);
        assert_eq!(runner.contains("check-migrations-immutable.sh"), backend);
        assert_eq!(runner.contains("playwright test"), web);
        assert_eq!(runner.contains("--repeat-each"), web);
        assert_eq!(runner.contains("expo-doctor"), mobile);
        assert_eq!(runner.contains("assembleDebug"), mobile);
        assert_eq!(runner.contains("--dir web run build:sw:check"), web);
        assert_eq!(
            runner.contains("--dir mobile run build:sw:check"),
            mobile && !web
        );
        assert_eq!(
            root.join("scripts/check-migrations-immutable.sh").is_file(),
            backend
        );
        assert!(root.join("scripts/check-markdown-links.py").is_file());
        assert!(root.join("scripts/check-markdown-links.test.py").is_file());

        let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml"))?;
        assert!(workflow.contains("  strict-quality:"));
        assert_eq!(
            workflow.contains("taiki-e/install-action@cargo-llvm-cov"),
            backend
        );
        assert_eq!(workflow.contains("playwright install --with-deps"), web);
        assert_eq!(workflow.contains("actions/setup-java@v6"), mobile);
    }

    let parent = tempfile::tempdir()?;
    let mut combined = options(parent.path(), "snapshot-app");
    combined.mobile = true;
    combined.web = true;
    combined.quality = QualityProfile::Strict;
    let root = generate_new(&combined)?;
    assert_eq!(
        render_hash_snapshot(&read_tree(&root)?),
        include_str!("snapshots/strict.tree")
    );
    Ok(())
}

#[test]
fn quality_flag_generates_the_strict_profile() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_baukit"))
        .args([
            "new",
            "strict-flag",
            "--web",
            "--quality",
            "strict",
            "--skip-lockfiles",
            "--dir",
        ])
        .arg(parent.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = baukit_cli::read_manifest(&parent.path().join("strict-flag"))?;
    assert_eq!(manifest.quality.profile, QualityProfile::Strict);
    Ok(())
}

#[test]
fn mcp_flag_generates_the_rust_server_and_records_the_capability() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_baukit"))
        .args([
            "new",
            "mcp-flag",
            "--backend",
            "--mcp",
            "--auth",
            "oidc",
            "--skip-lockfiles",
            "--dir",
        ])
        .arg(parent.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let root = parent.path().join("mcp-flag");
    let manifest = baukit_cli::read_manifest(&root)?;
    assert!(manifest.capabilities.mcp);
    assert!(
        root.join("backend/crates/mcp-flag-mcp/Cargo.toml")
            .is_file()
    );
    assert!(!root.join("mcp").exists());
    Ok(())
}

#[test]
fn generated_migration_guard_ports_failure_cases() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut strict = options(parent.path(), "strict-migrations");
    strict.quality = QualityProfile::Strict;
    let root = generate_new(&strict)?;
    let output = Command::new("sh")
        .arg("scripts/check-migrations-immutable.test.sh")
        .current_dir(root)
        .output()?;
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn generated_environment_reconciler_is_tested_and_setup_is_idempotent() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut generated_options = options(parent.path(), "env-setup");
    generated_options.mobile = true;
    generated_options.web = true;
    let root = generate_new(&generated_options)?;
    for package in ["web/package.json", "mobile/package.json"] {
        assert!(
            fs::read_to_string(root.join(package))?
                .contains("\"setup\": \"sh ../scripts/setup.sh\"")
        );
    }

    let tests = Command::new("python3")
        .arg("scripts/reconcile-env.test.py")
        .current_dir(&root)
        .output()?;
    assert!(
        tests.status.success(),
        "{}{}",
        String::from_utf8_lossy(&tests.stdout),
        String::from_utf8_lossy(&tests.stderr)
    );

    fs::write(root.join("web/.env"), "VITE_API_URL=http://local.test")?;
    let first = Command::new("sh")
        .arg("scripts/setup.sh")
        .current_dir(&root)
        .output()?;
    assert!(first.status.success());
    assert_eq!(
        fs::read(root.join("web/.env"))?,
        b"VITE_API_URL=http://local.test"
    );
    assert!(root.join("mobile/.env").is_file());
    let before = fs::read(root.join("mobile/.env"))?;
    let second = Command::new("sh")
        .arg("scripts/setup.sh")
        .current_dir(&root)
        .output()?;
    assert!(second.status.success());
    assert_eq!(fs::read(root.join("mobile/.env"))?, before);
    Ok(())
}

#[test]
fn generated_markdown_link_check_fails_for_a_committed_missing_target() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut strict = options(parent.path(), "strict-links");
    strict.quality = QualityProfile::Strict;
    let root = generate_new(&strict)?;

    let tests = Command::new("python3")
        .arg("scripts/check-markdown-links.test.py")
        .current_dir(&root)
        .output()?;
    assert!(
        tests.status.success(),
        "{}{}",
        String::from_utf8_lossy(&tests.stdout),
        String::from_utf8_lossy(&tests.stderr)
    );

    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .status()?
            .success()
    );
    assert!(
        Command::new("git")
            .args(["add", "."])
            .current_dir(&root)
            .status()?
            .success()
    );
    let passing = Command::new("python3")
        .args([
            "scripts/check-markdown-links.py",
            "README.md",
            "CLAUDE.md",
            "AGENTS.md",
            "docs",
        ])
        .current_dir(&root)
        .output()?;
    assert!(
        passing.status.success(),
        "{}{}",
        String::from_utf8_lossy(&passing.stdout),
        String::from_utf8_lossy(&passing.stderr)
    );

    fs::write(root.join("docs/broken.md"), "[missing](absent.md)\n")?;
    assert!(
        Command::new("git")
            .args(["add", "docs/broken.md"])
            .current_dir(&root)
            .status()?
            .success()
    );
    let broken = Command::new("python3")
        .args(["scripts/check-markdown-links.py", "docs"])
        .current_dir(&root)
        .output()?;
    assert!(!broken.status.success());
    assert!(String::from_utf8_lossy(&broken.stderr).contains("docs/broken.md:1 -> absent.md"));

    fs::write(root.join("docs/absent.md"), "# Present\n")?;
    let fixed = Command::new("python3")
        .args(["scripts/check-markdown-links.py", "docs"])
        .current_dir(&root)
        .status()?;
    assert!(fixed.success());
    Ok(())
}

#[test]
fn combined_generation_applies_port_offset_to_host_ports() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut generated_options = options(parent.path(), "offset-app");
    generated_options.mobile = true;
    generated_options.web = true;
    generated_options.auth = Some(AuthProvider::Oidc);
    generated_options.port_offset = 100;
    let root = generate_new(&generated_options)?;

    let expected = [
        ("baukit.toml", "port_offset = 100"),
        ("compose.yaml", "\"5532:5432\""),
        ("compose.yaml", "\"8181:8080\""),
        ("compose.yaml", "\"127.0.0.1:6479:6379\""),
        (
            "Makefile",
            "OFFSET_APP__RATE_LIMIT__REDIS_URL=redis://127.0.0.1:6479/",
        ),
        ("deploy/values.yaml", "http: 8180"),
        ("deploy/values.yaml", "ops: 9190"),
        (
            "mobile/.env.example",
            "EXPO_PUBLIC_API_URL=http://localhost:8180",
        ),
        (
            "mobile/.env.example",
            "EXPO_PUBLIC_OIDC_ISSUER=http://localhost:8181/realms/offset-app",
        ),
        ("mobile/app.config.ts", "http://localhost:8180"),
        (
            "mobile/app.config.ts",
            "http://localhost:8181/realms/${PRODUCT_NAME}",
        ),
        ("mobile/src/product.ts", "PRODUCT_NAME = 'offset-app'"),
        (
            "mobile/src/auth.ts",
            "http://localhost:8181/realms/${PRODUCT_NAME}",
        ),
        ("web/src/product.ts", "PRODUCT_NAME = 'offset-app'"),
        (
            "web/src/auth.ts",
            "http://localhost:8181/realms/${PRODUCT_NAME}",
        ),
        ("mobile/README.md", "http://localhost:8180"),
        ("web/.env.example", "VITE_API_URL=http://localhost:8180"),
        (
            "web/.env.example",
            "VITE_OIDC_ISSUER=http://localhost:8181/realms/offset-app",
        ),
        ("web/README.md", "http://localhost:8180"),
        ("README.md", "public API listens on port 8180"),
        ("README.md", "endpoints listen on port 9190"),
        ("README.md", "postgres@localhost:5532/offset_app"),
        ("README.md", "http://localhost:8181/realms/offset-app"),
        ("README.md", "Redis on `127.0.0.1:6479`"),
        ("docs/fake-providers.md", "FAKE_PROVIDER_PORT:-18181"),
        ("scripts/pkce-login.py", "http://localhost:8180/me"),
    ];
    for (relative, snippet) in expected {
        let contents = fs::read_to_string(root.join(relative))?;
        assert!(
            contents.contains(snippet),
            "{relative} did not contain {snippet:?}"
        );
    }
    Ok(())
}

#[test]
fn generation_rejects_a_port_offset_that_exceeds_u16() {
    let parent = tempfile::tempdir().expect("temporary directory");
    let mut generated_options = options(parent.path(), "invalid-offset");
    generated_options.port_offset = 47_455;
    let error = generate_new(&generated_options).expect_err("offset must fail");
    assert!(error.to_string().contains("above 65535"));
    assert!(!parent.path().join("invalid-offset").exists());
}

#[test]
fn oidc_generation_is_deterministic_and_records_the_optional_capability() -> anyhow::Result<()> {
    let first_parent = tempfile::tempdir()?;
    let second_parent = tempfile::tempdir()?;
    let mut first_options = options(first_parent.path(), "snapshot-app");
    first_options.mobile = true;
    first_options.web = true;
    first_options.auth = Some(AuthProvider::Oidc);
    let mut second_options = first_options.clone();
    second_options.directory = second_parent.path().to_path_buf();

    let first = generate_new(&first_options)?;
    let second = generate_new(&second_options)?;
    let first_tree = read_tree(&first)?;
    assert_eq!(first_tree, read_tree(&second)?);
    assert_eq!(
        render_hash_snapshot(&first_tree),
        include_str!("snapshots/auth.tree")
    );

    let manifest_source = fs::read_to_string(first.join("baukit.toml"))?;
    assert!(manifest_source.contains("auth = \"oidc\""));
    let manifest = baukit_cli::read_manifest(&first)?;
    assert_eq!(manifest.capabilities.auth, Some(AuthProvider::Oidc));
    assert!(first.join("keycloak/realm.json").is_file());
    assert!(first.join("backend/tests/auth_conformance.rs").is_file());
    assert!(first.join("web/src/auth.ts").is_file());
    assert!(first.join("web/src/local-data.ts").is_file());
    assert!(first.join("mobile/src/auth.ts").is_file());
    assert!(first.join("mobile/src/local-data.ts").is_file());
    assert!(first.join("mobile/app/(auth)/_layout.tsx").is_file());
    assert!(first.join("mobile/app/(auth)/sign-in.tsx").is_file());
    assert!(first.join("web/docs/local-data-retention.md").is_file());
    assert!(first.join("mobile/docs/local-data-retention.md").is_file());
    let mobile_package = fs::read_to_string(first.join("mobile/package.json"))?;
    assert!(mobile_package.contains("@baukit/auth-native"));
    assert!(mobile_package.contains("@baukit/data-contracts"));
    assert!(mobile_package.contains("\"main\": \"expo-router/entry\""));
    assert!(mobile_package.contains("\"expo-router\""));
    let web_package = fs::read_to_string(first.join("web/package.json"))?;
    assert!(web_package.contains("@baukit/auth-web"));
    assert!(web_package.contains("@baukit/data-contracts"));

    let api = fs::read_to_string(first.join("backend/crates/snapshot-app-api/src/lib.rs"))?;
    assert_eq!(api.matches("security((\"bearerAuth\" = []))").count(), 8);
    assert_eq!(api.matches("_principal: Principal").count(), 5);
    let openapi = fs::read_to_string(first.join("backend/openapi.json"))?;
    assert_eq!(openapi.matches("\"bearerAuth\": []").count(), 8);
    let realm = fs::read_to_string(first.join("keycloak/realm.json"))?;
    assert!(realm.contains("\"realmRoles\": [\"offline_access\"]"));
    assert!(realm.contains("snapshot-app-mobile"));
    assert!(realm.contains("\"loginTheme\": \"baukit-accessible\""));
    assert!(first.join("keycloak/realm-policy.json").is_file());
    assert!(first.join("keycloak/reconcile.json").is_file());
    assert!(
        first
            .join("keycloak/themes/baukit-accessible/login/theme.properties")
            .is_file()
    );
    assert!(
        first
            .join("keycloak/themes/baukit-accessible/login/resources/js/accessibility.js")
            .is_file()
    );
    assert!(
        first
            .join("keycloak/themes/baukit-accessible/login/resources/js/theme-preferences.js")
            .is_file()
    );
    assert!(
        first
            .join("keycloak/themes/baukit-accessible-test/login/theme.properties")
            .is_file()
    );
    assert!(
        first
            .join("keycloak/themes/baukit-accessible-test/login/resources/css/fixture.css")
            .is_file()
    );
    assert!(
        first
            .join("keycloak/themes/baukit-accessible-test/login/messages/messages_en.properties")
            .is_file()
    );
    assert!(first.join("scripts/keycloak-theme.browser.mjs").is_file());
    assert!(
        first
            .join("scripts/test-keycloak-theme-patches.sh")
            .is_file()
    );
    assert!(
        first
            .join("scripts/tests/keycloak_accessibility.test.mjs")
            .is_file()
    );
    assert!(
        first
            .join("scripts/tests/keycloak_theme_preferences.test.mjs")
            .is_file()
    );
    assert!(!first_tree.keys().any(|path| {
        path.starts_with("keycloak/themes")
            && path.extension().is_some_and(|extension| extension == "ftl")
    }));
    assert!(first.join("scripts/keycloak_policy.py").is_file());
    assert!(first.join("scripts/reconcile_keycloak.py").is_file());
    let compose = fs::read_to_string(first.join("compose.yaml"))?;
    assert!(compose.contains("keycloak-data:"));
    assert!(compose.contains("./keycloak/themes:/opt/keycloak/themes:ro"));
    assert!(compose.contains("KEYCLOAK_IMAGE:-quay.io/keycloak/keycloak:26.8.0"));
    let theme_runner = fs::read_to_string(first.join("scripts/test-keycloak-theme-patches.sh"))?;
    assert!(theme_runner.contains("in 26.7.5 26.8.0"));
    assert!(theme_runner.contains("127.0.0.1::8080"));
    assert!(theme_runner.contains("compose port keycloak 8080"));
    let reconcile = fs::read_to_string(first.join("keycloak/reconcile.json"))?;
    assert!(reconcile.contains("\"loginTheme\""));
    Ok(())
}

#[test]
fn oidc_realm_only_emits_selected_public_clients() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut selected = options(parent.path(), "web-product");
    selected.web = true;
    selected.auth = Some(AuthProvider::Oidc);
    let root = generate_new(&selected)?;
    let realm = fs::read_to_string(root.join("keycloak/realm.json"))?;
    assert!(realm.contains("web-product-web"));
    assert!(!realm.contains("web-product-mobile"));
    assert!(realm.contains("offline_access"));
    assert!(fs::read_to_string(root.join("compose.yaml"))?.contains("KC_HEALTH_ENABLED"));
    assert!(
        fs::read_to_string(root.join("scripts/pkce-login.py"))?
            .contains("parser.add_argument(\"--client-id\", required=True)")
    );
    Ok(())
}

#[test]
fn generated_keycloak_policy_and_reconciler_fixtures_pass() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let baukit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust");
    let mut selected = options(parent.path(), "keycloak-tools");
    selected.web = true;
    selected.mobile = true;
    selected.auth = Some(AuthProvider::Oidc);
    selected.baukit_path = Some(baukit_path);
    let root = generate_new(&selected)?;

    for arguments in [
        vec!["-m", "unittest", "discover", "-s", "scripts/tests"],
        vec![
            "scripts/keycloak_policy.py",
            "--environment-class",
            "development",
        ],
        vec![
            "scripts/keycloak_policy.py",
            "--realm",
            "scripts/tests/fixtures/production-realm.json",
            "--policy",
            "scripts/tests/fixtures/production-policy.json",
            "--environment-class",
            "production",
        ],
        vec!["scripts/reconcile_keycloak.py", "--check"],
    ] {
        let output = Command::new("python3")
            .args(arguments)
            .current_dir(&root)
            .output()?;
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let results = doctor(&root)?;
    assert!(
        results
            .iter()
            .any(|result| result.contains("development realm policy passed"))
    );
    assert!(
        results
            .iter()
            .any(|result| result.contains("reconciliation inputs passed"))
    );
    Ok(())
}

#[test]
fn generated_keycloak_policy_rejects_a_weakened_realm() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let baukit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust");
    let mut selected = options(parent.path(), "weak-realm");
    selected.web = true;
    selected.auth = Some(AuthProvider::Oidc);
    selected.baukit_path = Some(baukit_path);
    let root = generate_new(&selected)?;
    let realm_path = root.join("keycloak/realm.json");
    let weakened = fs::read_to_string(&realm_path)?
        .replace(
            "length(12) and notUsername and notEmail and maxLength(128)",
            "length(8) and maxLength(512)",
        )
        .replace(
            "\"bruteForceProtected\": true",
            "\"bruteForceProtected\": false",
        )
        .replace(
            "\"pkce.code.challenge.method\": \"S256\"",
            "\"pkce.code.challenge.method\": \"plain\"",
        )
        .replace(
            "\"directAccessGrantsEnabled\": false",
            "\"directAccessGrantsEnabled\": true",
        );
    fs::write(&realm_path, weakened)?;

    let output = Command::new("python3")
        .args([
            "scripts/keycloak_policy.py",
            "--environment-class",
            "development",
        ])
        .current_dir(&root)
        .output()?;
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "at least 12",
        "at most 128",
        "notUsername",
        "notEmail",
        "bruteForceProtected",
        "direct-access",
        "PKCE S256",
    ] {
        assert!(error.contains(expected), "missing {expected:?} in {error}");
    }
    let doctor_error = doctor(&root).expect_err("doctor must reject the weakened realm");
    assert!(doctor_error.to_string().contains("realm policy failed"));
    Ok(())
}

#[test]
fn oidc_dependencies_include_erasure_and_jobs_in_registry_and_path_modes() -> anyhow::Result<()> {
    let baukit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rust")
        .canonicalize()?;
    for local_path in [false, true] {
        for worker in [false, true] {
            let parent = tempfile::tempdir()?;
            let mut generated_options = options(parent.path(), "erasure-product");
            generated_options.auth = Some(AuthProvider::Oidc);
            generated_options.worker = worker;
            if local_path {
                generated_options.baukit_path = Some(baukit_path.clone());
            }
            let root = generate_new(&generated_options)?;
            let cargo: toml::Value =
                toml::from_str(&fs::read_to_string(root.join("backend/Cargo.toml"))?)?;
            let dependencies = &cargo["workspace"]["dependencies"];
            for name in ["baukit-auth", "baukit-erasure", "baukit-jobs"] {
                if local_path {
                    assert_eq!(
                        dependencies[name]["path"].as_str(),
                        Some(
                            baukit_path
                                .join("crates")
                                .join(name)
                                .to_str()
                                .ok_or_else(|| anyhow::anyhow!("non-UTF-8 path"))?
                        ),
                    );
                } else {
                    assert_eq!(
                        dependencies[name].as_str(),
                        Some(baukit_cli::TEMPLATE_VERSION)
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn release_emission_uses_registry_versions_and_reproducibility_files() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut combined = options(parent.path(), "release-product");
    combined.web = true;
    combined.mobile = true;
    let root = generate_new(&combined)?;

    let cargo = fs::read_to_string(root.join("backend/Cargo.toml"))?;
    assert!(!cargo.contains("ssh://git@github.com/PatrickKoss/baukit.git"));
    assert!(cargo.contains(&format!(
        "baukit-config = \"{}\"",
        baukit_cli::TEMPLATE_VERSION
    )));
    let web_manifest = fs::read_to_string(root.join("web/package.json"))?;
    assert!(!web_manifest.contains("git+ssh://"));
    assert!(web_manifest.contains(&format!(
        "\"@baukit/api-runtime\": \"{}\"",
        baukit_cli::TEMPLATE_VERSION
    )));
    assert_eq!(
        fs::read_to_string(root.join(".cargo/config.toml"))?,
        "[net]\ngit-fetch-with-cli = true\n"
    );
    assert!(!fs::read_to_string(root.join(".gitignore"))?.contains("/generated/"));
    let locks = fs::read_to_string(root.join("scripts/lockfiles.sh"))?;
    assert!(locks.contains("cargo generate-lockfile"));
    assert_eq!(
        locks
            .matches("install --lockfile-only --ignore-scripts")
            .count(),
        2
    );
    let preflight = fs::read_to_string(root.join("scripts/preflight.sh"))?;
    assert!(preflight.contains("BAUKIT_PREBUILT_IMAGES"));
    assert!(preflight.contains("ssh-add -l"));
    assert!(preflight.contains("PLAYWRIGHT_BROWSERS_PATH"));
    assert!(preflight.contains("executable resolved outside the repository cache"));
    // Registry tarballs ship prebuilt `dist/`, so only non-Baukit packages need build approval.
    let web_workspace = fs::read_to_string(root.join("web/pnpm-workspace.yaml"))?;
    assert!(!web_workspace.contains("@baukit/"));
    let mobile_workspace = fs::read_to_string(root.join("mobile/pnpm-workspace.yaml"))?;
    assert!(mobile_workspace.contains("unrs-resolver: true"));
    assert!(!mobile_workspace.contains("@baukit/"));
    let makefile = fs::read_to_string(root.join("Makefile"))?;
    assert!(
        makefile.contains("cargo test --manifest-path $(BACKEND_MANIFEST) -- --include-ignored")
    );
    assert!(
        makefile.contains("check: preflight fmt lint test test-scripts check-web check-mobile")
    );
    assert!(makefile.contains("test: preflight"));
    assert!(!makefile.contains("baukit generate openapi-client"));
    let client = fs::read_to_string(root.join("scripts/openapi-client.sh"))?;
    assert!(client.contains("openapi.get(\"consumers\")"));
    assert!(client.contains("tomllib.load(source)[\"openapi\"][\"schema\"]"));
    assert!(!client.contains("cargo run"));
    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml"))?;
    // Every job that builds product code needs the private Baukit dependency.
    assert_eq!(
        workflow.matches("ssh-private-key:").count(),
        workflow.matches("BAUKIT_DEPLOY_KEY").count()
    );
    for job in [
        "  backend:",
        "  backend-msrv:",
        "  api-drift:",
        "  docker-build:",
        "  web:",
        "  web-coverage:",
        "  e2e-web:",
        "  mobile:",
        "  mobile-coverage:",
        "  observability-lint:",
    ] {
        assert!(workflow.contains(job), "workflow is missing job {job}");
    }
    assert!(
        workflow.contains("cargo test --manifest-path backend/Cargo.toml -- --include-ignored")
    );
    // The MSRV floor is read from the manifest rather than restated here.
    assert!(workflow.contains("steps.msrv.outputs.version"));
    assert!(!workflow.contains("dtolnay/rust-toolchain@1."));
    assert!(workflow.contains("playwright install --with-deps"));
    assert!(workflow.contains("--project=webkit-desktop"));
    assert!(workflow.contains("--allowlist deploy/observability/product-metrics.txt"));
    assert!(workflow.contains("working-directory: web"));
    assert!(workflow.contains("working-directory: mobile"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn generated_preflight_fails_without_an_agent_and_supports_prebuilt_images() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path(), "preflight-app"))?;

    // Registry dependencies need no SSH agent, so preflight must pass without one.
    let registry_default = Command::new("sh")
        .arg("scripts/preflight.sh")
        .current_dir(&root)
        .env_remove("SSH_AUTH_SOCK")
        .output()?;
    assert!(registry_default.status.success());

    // The SSH checks below still guard products pinned to a Baukit git tag.
    let manifest_path = root.join("baukit.toml");
    let manifest = fs::read_to_string(&manifest_path)?;
    fs::write(
        &manifest_path,
        manifest.replace(
            "source = \"registry\"",
            "source = \"git\"\ngit = \"ssh://git@github.com/PatrickKoss/baukit.git\"",
        ),
    )?;

    let missing_agent = Command::new("sh")
        .arg("scripts/preflight.sh")
        .current_dir(&root)
        .env_remove("SSH_AUTH_SOCK")
        .output()?;
    assert!(!missing_agent.status.success());
    assert!(String::from_utf8_lossy(&missing_agent.stderr).contains("SSH_AUTH_SOCK is unset"));

    let prebuilt = Command::new("sh")
        .arg("scripts/preflight.sh")
        .current_dir(&root)
        .env_remove("SSH_AUTH_SOCK")
        .env("BAUKIT_PREBUILT_IMAGES", "true")
        .output()?;
    assert!(prebuilt.status.success());
    assert!(String::from_utf8_lossy(&prebuilt.stdout).contains("prebuilt-image mode"));

    let not_a_socket = root.join("not-an-agent");
    fs::write(&not_a_socket, "not a socket\n")?;
    let invalid_agent = Command::new("sh")
        .arg("scripts/preflight.sh")
        .current_dir(&root)
        .env("SSH_AUTH_SOCK", &not_a_socket)
        .output()?;
    assert!(!invalid_agent.status.success());
    let stderr = String::from_utf8_lossy(&invalid_agent.stderr);
    assert!(stderr.contains("does not point to a socket"));
    assert!(!stderr.contains(not_a_socket.to_string_lossy().as_ref()));

    let fake_bin = parent.path().join("fake-bin");
    fs::create_dir(&fake_bin)?;
    write_executable(
        &fake_bin.join("ssh-add"),
        "#!/bin/sh\nprintf '%s\\n' \"$BAUKIT_TEST_SSH_ADD_STATUS\" >> \"$BAUKIT_TEST_SSH_ADD_LOG\"\nexit \"$BAUKIT_TEST_SSH_ADD_STATUS\"\n",
    )?;
    for program in ["dirname", "grep", "sh"] {
        let resolved = Command::new("sh")
            .args(["-c", "command -v \"$1\"", "sh", program])
            .output()?;
        assert!(resolved.status.success(), "find {program}");
        std::os::unix::fs::symlink(
            String::from_utf8(resolved.stdout)?.trim(),
            fake_bin.join(program),
        )?;
    }
    let agent_socket = parent.path().join("agent.sock");
    let _agent = UnixListener::bind(&agent_socket)?;
    let calls = parent.path().join("ssh-add.calls");
    for (status, expected) in [("1", "no loaded identities"), ("2", "agent is unusable")] {
        let result = Command::new("sh")
            .arg("scripts/preflight.sh")
            .current_dir(&root)
            .env("PATH", &fake_bin)
            .env("SSH_AUTH_SOCK", &agent_socket)
            .env("BAUKIT_TEST_SSH_ADD_STATUS", status)
            .env("BAUKIT_TEST_SSH_ADD_LOG", &calls)
            .output()?;
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains(expected));
    }
    assert_eq!(fs::read_to_string(calls)?, "1\n2\n");
    Ok(())
}

#[cfg(unix)]
#[test]
fn generated_preflight_uses_one_playwright_cache_for_check_install_and_run() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&frontend_options(
        parent.path(),
        "playwright-app",
        false,
        true,
    ))?;
    fs::write(
        root.join("web/package.json"),
        "{\"devDependencies\":{\"@playwright/test\":\"1.0.0\"}}\n",
    )?;
    let fake_bin = parent.path().join("fake-bin");
    fs::create_dir(&fake_bin)?;
    write_executable(
        &fake_bin.join("corepack"),
        r#"#!/bin/sh
case "$*" in
  *"install --frozen-lockfile"*)
    mkdir -p "$BAUKIT_TEST_PLAYWRIGHT_MODULE"
    ;;
  *"exec node -e"*)
    [ -f "$BAUKIT_TEST_PLAYWRIGHT_MARKER" ]
    ;;
  *"exec playwright install chromium webkit"*)
    printf '%s\n' "$PLAYWRIGHT_BROWSERS_PATH" >>"$BAUKIT_TEST_INSTALL_LOG"
    mkdir -p "$PLAYWRIGHT_BROWSERS_PATH"
    : >"$BAUKIT_TEST_PLAYWRIGHT_MARKER"
    ;;
  *) exit 2 ;;
esac
"#,
    )?;
    write_executable(
        &fake_bin.join("record-playwright-cache"),
        "#!/bin/sh\nprintf '%s\\n' \"$PLAYWRIGHT_BROWSERS_PATH\" >\"$BAUKIT_TEST_RUN_LOG\"\n",
    )?;
    let marker = root.join("web/.playwright-browsers/browser-installed");
    let install_log = parent.path().join("install-cache");
    let run_log = parent.path().join("run-cache");
    let path = format!(
        "{}:{}",
        fake_bin.display(),
        env::var("PATH").unwrap_or_default()
    );
    for run in 0..2 {
        if run == 1 {
            fs::remove_dir_all(root.join("web/node_modules"))?;
            assert!(
                marker.is_file(),
                "dependency reinstalls must preserve the browser cache"
            );
        }
        let result = Command::new("sh")
            .args(["scripts/preflight.sh", "--", "record-playwright-cache"])
            .current_dir(&root)
            .env("PATH", &path)
            .env("BAUKIT_PREBUILT_IMAGES", "true")
            .env(
                "BAUKIT_TEST_PLAYWRIGHT_MODULE",
                root.join("web/node_modules/@playwright/test"),
            )
            .env("BAUKIT_TEST_PLAYWRIGHT_MARKER", &marker)
            .env("BAUKIT_TEST_INSTALL_LOG", &install_log)
            .env("BAUKIT_TEST_RUN_LOG", &run_log)
            .output()?;
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let expected_cache = format!("{}\n", root.join("web/.playwright-browsers").display());
    assert_eq!(fs::read_to_string(install_log)?, expected_cache);
    assert_eq!(fs::read_to_string(run_log)?, expected_cache);
    Ok(())
}

#[cfg(unix)]
fn write_executable(path: &Path, contents: &str) -> anyhow::Result<()> {
    fs::write(path, contents)?;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[test]
fn generation_can_render_directly_into_an_existing_repository_root() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    fs::create_dir_all(root.path().join(".git"))?;
    fs::write(root.path().join(".git/HEAD"), "ref: refs/heads/main\n")?;
    let mut existing = options(root.path(), "existing-product");
    existing.into_existing = true;

    assert_eq!(generate_new(&existing)?, root.path());
    assert!(root.path().join("baukit.toml").is_file());
    assert!(root.path().join(".git/HEAD").is_file());
    assert!(!root.path().join("existing-product").exists());
    Ok(())
}

#[test]
fn force_reports_conflicts_without_overwriting() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut options = options(parent.path(), "conflict-app");
    let root = generate_new(&options)?;
    let readme = root.join("README.md");
    fs::write(&readme, "user-owned content\n")?;

    assert!(generate_new(&options).is_err());
    options.force = true;
    let error = generate_new(&options).expect_err("modified file must be a conflict");
    assert!(error.to_string().contains("conflict"));
    assert_eq!(fs::read_to_string(readme)?, "user-owned content\n");
    let report = fs::read_to_string(root.join("baukit-conflicts.txt"))?;
    assert!(report.contains("README.md"));
    Ok(())
}

#[test]
fn at_least_one_capability_is_required() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let empty = frontend_options(parent.path(), "empty-app", false, false);
    let error = generate_new(&empty).expect_err("empty capability selection must fail");
    assert!(error.to_string().contains("at least one capability"));
    assert!(!parent.path().join("empty-app").exists());
    Ok(())
}

#[test]
fn worker_requires_backend() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut worker = frontend_options(parent.path(), "worker-only", false, false);
    worker.worker = true;
    let error = generate_new(&worker).expect_err("worker without backend must fail");
    assert!(error.to_string().contains("--worker requires --backend"));
    Ok(())
}

#[test]
fn raw_templates_do_not_contain_cargo_manifests() -> anyhow::Result<()> {
    let templates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../templates");
    let tree = read_tree(&templates)?;
    assert!(
        tree.keys()
            .all(|path| path.file_name().is_none_or(|name| name != "Cargo.toml")),
        "raw template Cargo.toml files are discovered and parsed by downstream Cargo commands"
    );
    assert_eq!(
        tree.keys()
            .filter(|path| path
                .file_name()
                .is_some_and(|name| name == "Cargo.toml.jinja"))
            .count(),
        9
    );
    Ok(())
}

#[test]
fn doctor_validates_a_local_generated_product() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let baukit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust");
    let mut local = options(parent.path(), "doctor-app");
    local.mobile = true;
    local.web = true;
    local.port_offset = 100;
    local.baukit_path = Some(baukit_path);
    let root = generate_new(&local)?;
    fs::rename(
        root.join("backend/migrations/0001_create_items.sql"),
        root.join("backend/migrations/0042_product_schema.sql"),
    )?;
    let results = doctor(&root)?;
    assert!(results.iter().any(|result| result.contains("schema")));
    assert!(
        results
            .iter()
            .any(|result| result.contains("Cargo workspace"))
    );
    assert!(results.iter().any(|result| result.contains("mobile")));
    assert!(results.iter().any(|result| result.contains("web")));
    assert!(
        results
            .iter()
            .any(|result| result.contains("SQL migration"))
    );
    assert!(
        results
            .iter()
            .any(|result| result.contains("port offset 100"))
    );
    assert!(
        results
            .iter()
            .any(|result| result.contains("environment reconciliation"))
    );

    fs::write(
        root.join("mobile/.env.example"),
        "EXPO_PUBLIC_API_URL=http://localhost:8080\n",
    )?;
    let error = doctor(&root).expect_err("doctor must find a stale generated port");
    assert!(
        error
            .to_string()
            .contains("mobile/.env.example` does not use port offset 100")
    );
    Ok(())
}

#[test]
fn doctor_validates_long_name_products_of_every_flavor() -> anyhow::Result<()> {
    for offset in [0, 100] {
        for (backend, mobile, web, mcp, auth) in [
            (true, false, false, false, None),
            (false, true, false, false, None),
            (false, false, true, false, None),
            (true, true, true, false, None),
            (true, false, false, true, Some(AuthProvider::Oidc)),
            (true, true, true, true, Some(AuthProvider::Oidc)),
        ] {
            let parent = tempfile::tempdir()?;
            let mut local = options(parent.path(), "long-product-name-fixture");
            local.backend = backend;
            local.worker = backend;
            local.mobile = mobile;
            local.web = web;
            local.mcp = mcp;
            local.auth = auth;
            local.port_offset = offset;
            local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
            let root = generate_new(&local)?;
            let results = doctor(&root)?;
            assert!(
                results
                    .iter()
                    .any(|result| result.contains("product identities"))
            );
            assert!(
                results
                    .iter()
                    .any(|result| result.contains("port offset")
                        || result.contains("no port offset"))
            );
        }
    }
    Ok(())
}

#[test]
fn doctor_rejects_drift_in_product_constants_and_url_defaults() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "long-product-name-fixture");
    local.worker = true;
    local.mobile = true;
    local.web = true;
    local.mcp = true;
    local.auth = Some(AuthProvider::Oidc);
    local.port_offset = 100;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    doctor(&root)?;
    for (relative, original, drifted, diagnostic) in [
        (
            "web/src/product.ts",
            "long-product-name-fixture",
            "other-product",
            "PRODUCT_NAME",
        ),
        (
            "mobile/src/product.ts",
            "long-product-name-fixture",
            "other-product",
            "PRODUCT_NAME",
        ),
        (
            "web/src/auth.ts",
            "localhost:8181",
            "localhost:9999",
            "port offset 100",
        ),
        (
            "mobile/src/auth.ts",
            "localhost:8181",
            "localhost:9999",
            "port offset 100",
        ),
        (
            "mobile/app.config.ts",
            "localhost:8181",
            "localhost:9999",
            "port offset 100",
        ),
        (
            "mobile/app.config.ts",
            "localhost:8180",
            "localhost:9999",
            "port offset 100",
        ),
        (
            "web/e2e/stack/keycloak.ts",
            "localhost:8181",
            "localhost:9999",
            "port offset 100",
        ),
        (
            "web/src/delete-profile.ts",
            "localhost:8180",
            "localhost:9999",
            "port offset 100",
        ),
        (
            "mobile/src/delete-profile.ts",
            "localhost:8180",
            "localhost:9999",
            "port offset 100",
        ),
        (
            "backend/crates/long-product-name-fixture-bin/src/lib.rs",
            "long-product-name-fixture",
            "other-product",
            "PRODUCT",
        ),
        (
            "backend/crates/long-product-name-fixture-bin/src/lib.rs",
            "8181\".to_owned()",
            "9999\".to_owned()",
            "port offset 100",
        ),
        (
            "backend/crates/long-product-name-fixture-bin/src/lib.rs",
            "8181/realms",
            "9999/realms",
            "port offset 100",
        ),
    ] {
        let path = root.join(relative);
        let source = fs::read_to_string(&path)?;
        assert!(
            source.contains(original),
            "missing test mutation in {relative}"
        );
        fs::write(&path, source.replace(original, drifted))?;
        let error = doctor(&root)
            .expect_err("doctor must reject configuration drift")
            .to_string();
        assert!(error.contains(diagnostic), "{error}");
        if diagnostic == "PRODUCT_NAME" || diagnostic == "ENV_PREFIX" {
            assert!(error.contains("consumed by"), "{error}");
        } else {
            assert!(error.contains(relative), "{error}");
        }
        fs::write(path, source)?;
    }
    doctor(&root)?;
    Ok(())
}

#[test]
fn doctor_accepts_variable_url_configuration_and_formatted_constants() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "long-product-name-fixture");
    local.mobile = true;
    local.web = true;
    local.mcp = true;
    local.auth = Some(AuthProvider::Oidc);
    local.port_offset = 100;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    for relative in ["mobile/src/product.ts", "web/src/product.ts"] {
        let path = root.join(relative);
        let source = fs::read_to_string(&path)?;
        fs::write(
            &path,
            source.replace(
                "export const PRODUCT_NAME = 'long-product-name-fixture';",
                "export const PRODUCT_NAME: string =\n  \"long-product-name-fixture\";",
            ),
        )?;
        doctor(&root)?;
        fs::remove_file(&path)?;
        let error = doctor(&root).expect_err("doctor must find missing product constants");
        assert!(
            error.to_string().contains("has no literal source"),
            "{error}"
        );
        fs::write(path, source)?;
    }
    for (relative, source) in [
        (
            "mobile/app.config.ts",
            "export default { scheme: 'product', extra: { apiBaseUrl: process.env.EXPO_PUBLIC_API_URL, oidcIssuer: process.env.EXPO_PUBLIC_OIDC_ISSUER } };",
        ),
        (
            "mobile/src/auth.ts",
            "export const issuer = process.env.EXPO_PUBLIC_OIDC_ISSUER;",
        ),
        (
            "web/src/auth.ts",
            "export const issuer = import.meta.env.VITE_OIDC_ISSUER;",
        ),
        (
            "web/e2e/stack/keycloak.ts",
            "export const url = process.env.E2E_KEYCLOAK_URL;",
        ),
    ] {
        let path = root.join(relative);
        let source = if relative == "mobile/src/auth.ts" {
            fs::read_to_string(&path)?.replace(
                "http://localhost:8181/realms/long-product-name-fixture",
                "${OIDC_ISSUER}",
            )
        } else {
            source.to_owned()
        };
        fs::write(path, source)?;
    }
    doctor(&root)?;
    Ok(())
}

#[test]
fn doctor_distinguishes_missing_constants_from_wrong_values() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "constant-diagnostics");
    local.mobile = true;
    local.web = true;
    local.mcp = true;
    local.auth = Some(AuthProvider::Oidc);
    local.port_offset = 100;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    for (relative, name, correct, wrong) in [
        (
            "mobile/src/product.ts",
            "PRODUCT_NAME",
            "constant-diagnostics",
            "wrong-product",
        ),
        (
            "web/src/product.ts",
            "PRODUCT_NAME",
            "constant-diagnostics",
            "wrong-product",
        ),
        (
            "backend/crates/constant-diagnostics-bin/src/bin/api.rs",
            "PRODUCT",
            "constant-diagnostics",
            "wrong-product",
        ),
    ] {
        let path = root.join(relative);
        let source = fs::read_to_string(&path)?;
        let declaration = format!("const {name}");
        assert!(source.contains(&declaration));
        fs::write(
            &path,
            source.replace(&declaration, &format!("const RENAMED_{name}")),
        )?;
        let error = doctor(&root)
            .expect_err("doctor must identify a missing constant")
            .to_string();
        if name == "KEYCLOAK_PORT" {
            assert!(
                error.contains(&format!(
                    "generated file `{relative}` does not define {name}"
                )),
                "{error}"
            );
        } else {
            assert!(
                error.contains(name) && error.contains("consumed by"),
                "{error}"
            );
            assert!(error.contains("has no literal source"), "{error}");
        }
        assert!(
            !error.contains("does not match application name"),
            "{error}"
        );
        fs::write(&path, source.replace(correct, wrong))?;
        let error = doctor(&root)
            .expect_err("doctor must identify a wrong consumed identity value")
            .to_string();
        if name == "KEYCLOAK_PORT" {
            assert!(
                error.contains(&format!(
                    "generated file `{relative}` {name} does not use port offset 100"
                )),
                "{error}"
            );
        } else {
            assert!(error.contains("does not match application name"), "{error}");
            assert!(error.contains(name), "{error}");
        }
        assert!(!error.contains("has no literal source"), "{error}");
        fs::write(path, source)?;
    }
    doctor(&root)?;
    Ok(())
}

fn variable_port_makefile(assignment: &str, braced: bool, redis_port: u16) -> String {
    let source = format!(
        r#"export TIEFGANG_API_PORT {assignment} 8280
export TIEFGANG_OPS_PORT {assignment} 9290
export TIEFGANG_KEYCLOAK_PORT {assignment} 8281
export TIEFGANG_REDIS_PORT {assignment} {redis_port}

dev-backend:
	TIEFGANG__HTTP__PORT=$(TIEFGANG_API_PORT) \
	TIEFGANG__OPS__PORT=$(TIEFGANG_OPS_PORT) \
	TIEFGANG__AUTH__ISSUER=http://localhost:$(TIEFGANG_KEYCLOAK_PORT)/realms/tiefgang \
	TIEFGANG__RATE_LIMIT__REDIS_URL=redis://127.0.0.1:$(TIEFGANG_REDIS_PORT)/ \
	cargo run --manifest-path backend/Cargo.toml
"#,
    );
    if braced {
        source
            .replace("$(TIEFGANG_", "${TIEFGANG_")
            .replace(")", "}")
    } else {
        source
    }
}

#[test]
fn doctor_resolves_makefile_port_variables_and_rejects_drift() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "tiefgang");
    local.auth = Some(AuthProvider::Oidc);
    local.port_offset = 200;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let path = root.join("Makefile");
    for assignment in ["?=", ":=", "="] {
        for braced in [false, true] {
            let source = variable_port_makefile(assignment, braced, 6579);
            fs::write(&path, &source)?;
            doctor(&root)?;
            for port in [8280, 9290, 8281, 6579] {
                let drifted = source.replace(
                    &format!("{assignment} {port}"),
                    &format!("{assignment} 9999"),
                );
                fs::write(&path, drifted)?;
                let error = doctor(&root)
                    .expect_err("doctor must check resolved Makefile defaults")
                    .to_string();
                assert!(
                    error.contains("generated file `Makefile` does not use port offset 200"),
                    "{error}"
                );
                assert_eq!(
                    error.matches("generated file `Makefile`").count(),
                    1,
                    "{error}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn doctor_checks_makefile_redis_against_parameterized_compose_defaults() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "tiefgang");
    local.auth = Some(AuthProvider::Oidc);
    local.port_offset = 200;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let makefile = root.join("Makefile");
    let compose = root.join("compose.yaml");
    let original = fs::read_to_string(&compose)?;
    assert!(original.contains("127.0.0.1:6579:6379"));
    let source = variable_port_makefile("?=", false, 6389);
    for mapping in [
        "- \"127.0.0.1:${TIEFGANG_REDIS_PORT:-6389}:6379\"",
        "- \"127.0.0.1:${TIEFGANG_REDIS_PORT-6389}:6379\"",
        "- target: 6379\n        published: \"${TIEFGANG_REDIS_PORT:-6389}\"\n        host_ip: 127.0.0.1",
    ] {
        let configuration = original.replace("- \"127.0.0.1:6579:6379\"", mapping);
        fs::write(&compose, &configuration)?;
        fs::write(&makefile, &source)?;
        doctor(&root)?;
        fs::write(&makefile, source.replace("?= 6389", "?= 6390"))?;
        let error = doctor(&root).expect_err("Makefile and Compose Redis defaults must agree");
        assert!(
            error
                .to_string()
                .contains("Makefile` does not use port offset 200")
        );
        fs::write(&makefile, &source)?;
        fs::write(&compose, configuration.replace("6389", "6390"))?;
        let error = doctor(&root).expect_err("a changed Compose Redis default must cause drift");
        assert!(
            error
                .to_string()
                .contains("Makefile` does not use port offset 200")
        );
    }
    Ok(())
}

#[test]
fn doctor_checks_independent_host_and_container_port_declarations() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "declared-ports");
    local.auth = Some(AuthProvider::Oidc);
    local.mobile = true;
    local.web = true;
    local.port_offset = 100;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let manifest_path = root.join("baukit.toml");
    let manifest = fs::read_to_string(&manifest_path)?
        + r#"
[ports]
api = { host = 17001, container = 8080, service = "backend" }
ops = { host = 17002, container = 9090, service = "backend" }
postgres = { host = 17003, container = 5432 }
keycloak = { host = 17004, container = 8080 }
"#;
    fs::write(&manifest_path, &manifest)?;
    for relative in [
        "Makefile",
        "mobile/.env.example",
        "mobile/app.config.ts",
        "mobile/src/api.ts",
        "web/.env.example",
        "web/src/api.ts",
        "mobile/src/auth.ts",
        "mobile/src/delete-profile.ts",
        "web/src/auth.ts",
        "web/src/delete-profile.ts",
        "web/e2e/stack/keycloak.ts",
        "scripts/pkce-login.py",
        "backend/crates/declared-ports-bin/src/lib.rs",
    ] {
        let path = root.join(relative);
        let source = fs::read_to_string(&path)?
            .replace("8180", "17001")
            .replace("9190", "17002")
            .replace("8181", "17004");
        fs::write(path, source)?;
    }
    let values_path = root.join("deploy/values.yaml");
    fs::write(
        &values_path,
        fs::read_to_string(&values_path)?
            .replace("8180", "8080")
            .replace("9190", "9090"),
    )?;
    let compose_path = root.join("compose.yaml");
    let compose = fs::read_to_string(&compose_path)?.replace("5532:5432", "17003:5432").replace("8181:8080", "17004:8080").replace("volumes:\n  postgres-data:", "  backend:\n    ports:\n      - \"17001:8080\"\n      - \"17002:9090\"\n\nvolumes:\n  postgres-data:");
    fs::write(&compose_path, &compose)?;
    doctor(&root)?;
    for (source, diagnostic) in [
        (
            compose.replace("17003:5432", "17006:5432"),
            "host 17003 to container 5432",
        ),
        (
            compose.replace("17003:5432", "17003:5433"),
            "host 17003 to container 5432",
        ),
        (
            compose.replace("17004:8080", "17004:8181"),
            "host 17004 to container 8080",
        ),
        (
            compose.replace("17001:8080", "17001:8180"),
            "host 17001 to container 8080",
        ),
        (
            compose.replace("17003:5432", "${DB_PORT:-17006}:5432"),
            "host 17003 to container 5432",
        ),
    ] {
        fs::write(&compose_path, source)?;
        assert!(
            doctor(&root)
                .expect_err("declared mappings must agree")
                .to_string()
                .contains(diagnostic)
        );
    }
    fs::write(&compose_path, &compose)?;
    let api_path = root.join("mobile/src/api.ts");
    let api = fs::read_to_string(&api_path)?;
    fs::write(
        &api_path,
        format!("{api}\nconst wrong = 'http://127.0.0.1:17008';\n"),
    )?;
    assert!(
        doctor(&root)
            .expect_err("a wrong loopback port must be reported even alongside the correct one")
            .to_string()
            .contains("mobile/src/api.ts` does not match declared ports")
    );
    fs::write(&api_path, api)?;
    fs::write(
        &manifest_path,
        manifest.replace("container = 5432", "container = 5433"),
    )?;
    assert!(
        doctor(&root)
            .expect_err("the database image listens on 5432")
            .to_string()
            .contains("ports.postgres.container must be 5432")
    );
    Ok(())
}

#[test]
fn doctor_accepts_custom_literal_ports_without_an_offset() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "custom-ports");
    local.mobile = true;
    local.web = true;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    for relative in [
        "README.md",
        "docs/fake-providers.md",
        "Makefile",
        "deploy/values.yaml",
        "mobile/.env.example",
        "mobile/app.config.ts",
        "mobile/src/api.ts",
        "web/.env.example",
        "web/src/api.ts",
    ] {
        let path = root.join(relative);
        let source = fs::read_to_string(&path)?;
        fs::write(path, source.replace("8080", "8200").replace("9090", "9200"))?;
    }
    let compose = root.join("compose.yaml");
    fs::write(
        &compose,
        fs::read_to_string(&compose)?.replace("5432:5432", "127.0.0.1:5544:5432"),
    )?;
    doctor(&root)?;

    let mut manifest = baukit_cli::read_manifest(&root)?;
    manifest.port_offset = 100;
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    let error = doctor(&root).expect_err("configured offsets must reject stale ports");
    assert!(
        error
            .to_string()
            .contains("web/.env.example` does not use port offset 100")
    );
    assert!(
        error
            .to_string()
            .contains("mobile/.env.example` does not use port offset 100")
    );
    Ok(())
}

#[test]
fn doctor_accepts_loopback_compose_ports_and_checks_configured_offsets() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "loopback-ports");
    local.port_offset = 100;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let compose = root.join("compose.yaml");
    let source = fs::read_to_string(&compose)?.replace("5532:5432", "127.0.0.1:5532:5432");
    let source = source.replace("volumes:\n  postgres-data:", "  cache:\n    image: redis:8.10.2-alpine\n    ports:\n      - \"127.0.0.1:6379:6379\"\n\nvolumes:\n  postgres-data:");
    fs::write(&compose, &source)?;
    doctor(&root)?;
    fs::write(
        &compose,
        source.replace("127.0.0.1:5532:5432", "127.0.0.1:5432:5432"),
    )?;
    let error = doctor(&root).expect_err("configured offsets must reject stale compose ports");
    assert!(
        error
            .to_string()
            .contains("compose.yaml` does not use port offset 100")
    );
    Ok(())
}

#[test]
fn doctor_accepts_literal_auth_ports_without_environment_parameters() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "literal-auth-ports");
    local.auth = Some(AuthProvider::Oidc);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let path = root.join("compose.yaml");
    let source = fs::read_to_string(&path)?
        .replace("\"5432:5432\"", "\"127.0.0.1:5432:5432\"")
        .replace("\"8081:8080\"", "\"127.0.0.1:8081:8080\"");
    fs::write(path, source)?;
    doctor(&root)?;
    let mut manifest = baukit_cli::read_manifest(&root)?;
    manifest.port_offset = 100;
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    let error = doctor(&root).expect_err("configured offsets must reject stale Redis ports");
    assert!(error.to_string().contains("container port 6379"));
    Ok(())
}

#[test]
fn doctor_accepts_loopback_api_defaults_and_rejects_a_stale_port() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = frontend_options(parent.path(), "loopback-url", false, true);
    local.port_offset = 100;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let path = root.join("web/.env.example");
    fs::write(&path, "VITE_API_URL=http://127.0.0.1:8180\n")?;
    doctor(&root)?;
    fs::write(&path, "VITE_API_URL=http://127.0.0.1:8080\n")?;
    let error = doctor(&root).expect_err("configured offsets must check loopback URL ports");
    assert!(
        error
            .to_string()
            .contains("web/.env.example` does not use port offset 100")
    );
    Ok(())
}

#[test]
fn doctor_accepts_compose_port_parameters_and_long_loopback_mappings() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "compose-parameters");
    local.port_offset = 100;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let path = root.join("compose.yaml");
    let source = fs::read_to_string(&path)?;
    fs::write(
        &path,
        source.replace("5532:5432", "127.0.0.1:${POSTGRES_PORT:-5532}:5432"),
    )?;
    doctor(&root)?;
    let long = source.replace(
        "- \"5532:5432\"",
        "- target: 5432\n        published: \"5532\"\n        host_ip: 127.0.0.1",
    );
    fs::write(
        &path,
        long.replace(
            "published: \"5532\"",
            "published: \"${POSTGRES_PORT:-5532}\"",
        ),
    )?;
    doctor(&root)?;
    let long = long.replace(
        "volumes:\n  postgres-data:",
        "  reporting:\n    ports:\n      - \"127.0.0.1:5544:5432\"\n\nvolumes:\n  postgres-data:",
    );
    fs::write(&path, &long)?;
    doctor(&root)?;
    fs::write(
        &path,
        long.replace("published: \"5532\"", "published: \"5432\""),
    )?;
    let error = doctor(&root).expect_err("long mappings must still honor a configured offset");
    assert!(
        error
            .to_string()
            .contains("compose.yaml` does not use port offset 100")
    );
    Ok(())
}

#[test]
fn doctor_checks_pkce_ports_without_requiring_the_me_path() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "pkce-path");
    local.auth = Some(AuthProvider::Oidc);
    local.port_offset = 100;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let script = root.join("scripts/pkce-login.py");
    let source = fs::read_to_string(&script)?.replace("localhost:8180/me", "localhost:8180/v1/me");
    fs::write(&script, &source)?;
    doctor(&root)?;
    fs::write(
        &script,
        source.replace("localhost:8180/v1/me", "localhost:8080/v1/me"),
    )?;
    let error = doctor(&root).expect_err("configured offsets must reject a stale PKCE check URL");
    assert!(
        error
            .to_string()
            .contains("scripts/pkce-login.py` does not use port offset 100")
    );
    Ok(())
}

#[test]
fn doctor_accepts_web_without_unused_keycloak_test_helpers() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "web-without-e2e");
    local.web = true;
    local.auth = Some(AuthProvider::Oidc);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    fs::remove_dir_all(root.join("web/e2e/stack"))?;
    let path = root.join("web/package.json");
    let mut package: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
    assert!(
        package["devDependencies"]
            .as_object_mut()
            .expect("devDependencies")
            .remove("@baukit/auth-node")
            .is_some()
    );
    fs::write(path, serde_json::to_string_pretty(&package)?)?;
    doctor(&root)?;
    Ok(())
}

#[test]
fn doctor_accepts_auth_node_for_web_e2e_and_rejects_it_at_runtime() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "web-e2e");
    local.web = true;
    local.auth = Some(AuthProvider::Oidc);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    doctor(&root)?;
    let path = root.join("web/package.json");
    let mut package: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
    let dev_dependencies = package["devDependencies"]
        .as_object_mut()
        .expect("devDependencies");
    let auth_node = dev_dependencies
        .remove("@baukit/auth-node")
        .expect("auth-node devDependency");
    package["dependencies"]["@baukit/auth-node"] = auth_node;
    fs::write(path, serde_json::to_string_pretty(&package)?)?;
    let error = doctor(&root).expect_err("auth-node must not enter the browser runtime");
    assert!(
        error
            .to_string()
            .contains("only as a devDependency for Keycloak e2e")
    );
    Ok(())
}

#[test]
fn doctor_accepts_product_guidance_names_and_keeps_machine_read_docs() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "product-docs");
    local.auth = Some(AuthProvider::Oidc);
    local.mobile = true;
    local.web = true;
    local.mcp = true;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    for relative in [
        "docs/fake-providers.md",
        "docs/openapi-drift.md",
        "docs/syncable-tables.md",
        "docs/navigation-recipe.md",
        "docs/observability-lint.md",
        "docs/resource-budgets.md",
        "mobile/docs/local-data-retention.md",
        "web/docs/local-data-retention.md",
    ] {
        fs::remove_file(root.join(relative))?;
    }
    fs::write(
        root.join("docs/api-contract-policy.md"),
        "# API policy\n\nCheck generated clients against OpenAPI in CI.\n",
    )?;
    doctor(&root)?;
    fs::remove_file(root.join("docs/remote-mcp.md"))?;
    assert!(
        doctor(&root)
            .expect_err("remote MCP documentation is required")
            .to_string()
            .contains("missing remote MCP artifact `docs/remote-mcp.md`")
    );
    Ok(())
}

#[test]
fn doctor_requires_only_the_selected_mobile_analytics_adapter() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = frontend_options(parent.path(), "analytics-choice", true, false);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let manifest = fs::read_to_string(root.join("baukit.toml"))?;
    assert!(manifest.contains("analytics = \"posthog\""));
    doctor(&root)?;
    let package_path = root.join("mobile/package.json");
    let mut package: serde_json::Value = serde_json::from_str(&fs::read_to_string(&package_path)?)?;
    package["dependencies"]
        .as_object_mut()
        .expect("dependencies object")
        .remove("@baukit/analytics-posthog-native");
    fs::write(&package_path, serde_json::to_string_pretty(&package)?)?;
    let error = doctor(&root).expect_err("PostHog selection must require its adapter");
    assert!(
        error
            .to_string()
            .contains("missing dependency `@baukit/analytics-posthog-native`")
    );
    fs::write(
        root.join("baukit.toml"),
        manifest.replace("analytics = \"posthog\"", "analytics = \"none\""),
    )?;
    doctor(&root)?;
    fs::write(
        root.join("baukit.toml"),
        manifest.replace("analytics = \"posthog\"\n", ""),
    )?;
    doctor(&root)?;
    Ok(())
}

#[test]
fn doctor_accepts_root_workspaces_for_web_and_mobile_and_requires_a_workspace() -> anyhow::Result<()>
{
    let parent = tempfile::tempdir()?;
    let mut local = frontend_options(parent.path(), "root-workspace", true, true);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    doctor(&root)?;
    fs::remove_file(root.join("web/pnpm-workspace.yaml"))?;
    fs::remove_file(root.join("mobile/pnpm-workspace.yaml"))?;
    fs::write(
        root.join("pnpm-workspace.yaml"),
        "packages:\n  - web\n  - mobile\n",
    )?;
    doctor(&root)?;
    fs::remove_file(root.join("pnpm-workspace.yaml"))?;
    let error = doctor(&root).expect_err("both apps need a pnpm workspace");
    assert!(
        error
            .to_string()
            .contains("`web/pnpm-workspace.yaml` or `pnpm-workspace.yaml`")
    );
    assert!(
        error
            .to_string()
            .contains("`mobile/pnpm-workspace.yaml` or `pnpm-workspace.yaml`")
    );
    Ok(())
}

#[test]
fn doctor_requires_root_workspace_membership_for_every_app() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "workspace-members");
    local.mobile = true;
    local.web = true;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    doctor(&root)?;
    for capability in ["mobile", "web"] {
        fs::remove_file(root.join(capability).join("pnpm-workspace.yaml"))?;
    }
    for packages in [
        "['mobile', 'web']",
        "['*']",
        "['./mobile/', '{web}']",
        "['**/mobile', 'web']",
    ] {
        fs::write(
            root.join("pnpm-workspace.yaml"),
            format!("packages: {packages}\n"),
        )?;
        doctor(&root)?;
    }
    for capability in ["mobile", "web"] {
        for packages in [
            format!("['*', '!{capability}']"),
            format!(
                "['packages/*', '{}']",
                ["mobile", "web"]
                    .into_iter()
                    .filter(|name| *name != capability)
                    .collect::<Vec<_>>()
                    .join("', '")
            ),
        ] {
            fs::write(
                root.join("pnpm-workspace.yaml"),
                format!("packages: {packages}\n"),
            )?;
            let error = doctor(&root).expect_err("a root workspace must include each app");
            assert!(
                error
                    .to_string()
                    .contains(&format!("packages do not include `{capability}`"))
            );
        }
    }
    Ok(())
}

#[test]
fn generated_agent_guidance_is_a_regular_file() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path(), "agent-guidance"))?;
    assert!(
        fs::symlink_metadata(root.join("AGENTS.md"))?
            .file_type()
            .is_file()
    );
    assert_eq!(
        fs::read(root.join("AGENTS.md"))?,
        fs::read(root.join("CLAUDE.md"))?
    );
    Ok(())
}

#[test]
fn doctor_requires_generated_environment_and_strict_markdown_scripts() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let baukit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust");
    let mut strict = options(parent.path(), "doctor-scripts");
    strict.quality = QualityProfile::Strict;
    strict.baukit_path = Some(baukit_path);
    let root = generate_new(&strict)?;

    let results = doctor(&root)?;
    assert!(
        results
            .iter()
            .any(|result| result.contains("strict Markdown link check"))
    );

    fs::remove_file(root.join("scripts/reconcile-env.py"))?;
    fs::remove_file(root.join("scripts/check-markdown-links.py"))?;
    let error = doctor(&root).expect_err("doctor must require generated scripts");
    assert!(
        error
            .to_string()
            .contains("environment reconciliation file")
    );
    assert!(error.to_string().contains("Markdown link check file"));
    Ok(())
}

#[test]
fn doctor_checks_the_pwa_worker_build_in_a_mobile_only_product() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut mobile_only = frontend_options(parent.path(), "pwa-mobile", true, false);
    mobile_only.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    mobile_only.pwa = true;
    let root = generate_new(&mobile_only)?;
    assert!(
        doctor(&root)?
            .iter()
            .any(|result| result == "mobile PWA worker build uses the supported Baukit artifact")
    );
    let builder = root.join("mobile/scripts/build-sw.mjs");
    let source = fs::read(&builder)?;
    fs::remove_file(&builder)?;
    let error = doctor(&root).expect_err("doctor must require the Expo worker build");
    assert!(
        error
            .to_string()
            .contains("the PWA capability requires `mobile/scripts/build-sw.mjs`")
    );
    assert!(!error.to_string().contains("requires the web"));
    fs::write(&builder, source)?;
    let package_path = root.join("mobile/package.json");
    let mut package: serde_json::Value = serde_json::from_slice(&fs::read(&package_path)?)?;
    package["scripts"]
        .as_object_mut()
        .expect("scripts")
        .remove("build:sw:check");
    fs::write(package_path, serde_json::to_vec(&package)?)?;
    assert!(
        doctor(&root)
            .expect_err("missing check script")
            .to_string()
            .contains("requires the mobile `build:sw:check` script")
    );
    Ok(())
}

#[test]
fn doctor_rejects_a_pwa_without_an_app() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path(), "pwa-backend"))?;
    let path = root.join("baukit.toml");
    let manifest = fs::read_to_string(&path)?;
    fs::write(&path, manifest.replace("pwa = false", "pwa = true"))?;

    let error = doctor(&root).expect_err("doctor must reject a PWA without an app");
    assert!(
        error
            .to_string()
            .contains("the PWA capability requires the web or mobile capability")
    );
    Ok(())
}

#[test]
fn doctor_limits_openapi_compatibility_to_the_strict_profile() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path(), "compatibility-app"))?;
    let path = root.join("baukit.toml");
    let manifest = fs::read_to_string(&path)?;
    fs::write(
        &path,
        manifest.replace(
            "openapi_compatibility = \"off\"",
            "openapi_compatibility = \"enforce\"",
        ),
    )?;

    let error = doctor(&root).expect_err("doctor must reject the standard profile");
    assert!(
        error
            .to_string()
            .contains("quality.openapi_compatibility requires the strict profile")
    );
    Ok(())
}

#[test]
fn doctor_uses_manifest_declared_openapi_paths() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let baukit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust");
    let mut local = options(parent.path(), "doctor-openapi");
    local.baukit_path = Some(baukit_path);
    let root = generate_new(&local)?;
    fs::create_dir_all(root.join("contracts"))?;
    fs::create_dir_all(root.join("clients"))?;
    fs::rename(
        root.join("backend/openapi.json"),
        root.join("contracts/service.json"),
    )?;
    fs::rename(
        root.join("generated/openapi.d.ts"),
        root.join("clients/service.d.ts"),
    )?;
    let manifest_path = root.join("baukit.toml");
    let manifest = fs::read_to_string(&manifest_path)?
        .replace(
            "schema = \"backend/openapi.json\"",
            "schema = \"contracts/service.json\"",
        )
        .replace(
            "consumers = [\"generated/openapi.d.ts\"]",
            "typescript = \"clients/service.d.ts\"",
        );
    fs::write(manifest_path, manifest)?;

    let results = doctor(&root)?;
    assert!(
        results
            .iter()
            .any(|result| result.contains("manifest-declared OpenAPI"))
    );
    Ok(())
}

#[test]
fn doctor_accepts_a_timestamped_jobs_migration() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let baukit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust");
    let mut local = options(parent.path(), "doctor-worker");
    local.worker = true;
    local.baukit_path = Some(baukit_path);
    let root = generate_new(&local)?;
    fs::rename(
        root.join("backend/migrations/0003_baukit_jobs.sql"),
        root.join("backend/migrations/20260903120000_create_job_outbox.sql"),
    )?;

    let results = doctor(&root)?;
    assert!(
        results
            .iter()
            .any(|result| result.contains("creates the baukit-jobs"))
    );
    Ok(())
}

#[test]
fn doctor_accepts_an_env_only_api_source() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = frontend_options(parent.path(), "doctor-env", true, false);
    local.port_offset = 100;
    let root = generate_new(&local)?;
    fs::write(
        root.join("mobile/src/api.ts"),
        "export const apiUrl = process.env.EXPO_PUBLIC_API_URL;\n",
    )?;

    let results = doctor(&root)?;
    assert!(
        results
            .iter()
            .any(|result| result.contains("port offset 100"))
    );

    fs::write(
        root.join("mobile/src/api.ts"),
        "export const apiUrl = \"http://localhost:8080\";\n",
    )?;
    let error = doctor(&root).expect_err("doctor must find a stale localhost port");
    assert!(
        error
            .to_string()
            .contains("mobile/src/api.ts` does not use port offset 100")
    );
    Ok(())
}

#[test]
fn doctor_rejects_an_auth_redis_port_without_the_offset() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "doctor-redis");
    local.auth = Some(AuthProvider::Oidc);
    local.port_offset = 100;
    let root = generate_new(&local)?;
    let compose = fs::read_to_string(root.join("compose.yaml"))?;
    fs::write(
        root.join("compose.yaml"),
        compose.replace("127.0.0.1:6479:6379", "127.0.0.1:6379:6379"),
    )?;

    let error = doctor(&root).expect_err("doctor must find the stale Redis port");
    assert!(
        error
            .to_string()
            .contains("compose.yaml` does not use port offset 100")
    );
    Ok(())
}

#[test]
fn typescript_source_is_independent_of_product_name_length() -> anyhow::Result<()> {
    let maximum_name = "a".repeat(41);
    for auth in [None, Some(AuthProvider::Oidc)] {
        let parent = tempfile::tempdir()?;
        let mut baseline = None;
        for name in ["fixture", "solo-leveling-system-companion", &maximum_name] {
            let mut generated = options(parent.path(), name);
            generated.web = true;
            generated.mobile = true;
            generated.port_offset = 100;
            generated.auth = auth;
            let root = generate_new(&generated)?;
            let mut source = read_tree(&root)?;
            source.retain(|path, _| {
                ["web", "mobile"]
                    .iter()
                    .any(|flavor| path.starts_with(flavor))
                    && path
                        .extension()
                        .is_some_and(|ext| ext == "ts" || ext == "tsx" || ext == "js")
            });
            for flavor in ["web", "mobile"] {
                let path = PathBuf::from(format!("{flavor}/src/product.ts"));
                let product = source
                    .remove(&path)
                    .expect("frontend product constants must be generated");
                let product = String::from_utf8(product)?;
                assert_eq!(product, format!("export const PRODUCT_NAME = '{name}';\n"));
            }
            assert!(!source.is_empty());
            if let Some(expected) = &baseline {
                assert_eq!(
                    &source, expected,
                    "TypeScript source changed with product name {name} and auth {auth:?}"
                );
            } else {
                baseline = Some(source);
            }
        }
    }
    Ok(())
}

#[test]
fn mcp_requires_a_backend_and_auth_provider() {
    let parent = tempfile::tempdir().expect("temporary directory");
    let mut no_backend = frontend_options(parent.path(), "mcp-web", false, true);
    no_backend.mcp = true;
    assert!(
        generate_new(&no_backend)
            .expect_err("MCP without a backend must fail")
            .to_string()
            .contains("--mcp requires --backend")
    );

    let mut no_oidc = options(parent.path(), "mcp-no-oidc");
    no_oidc.mcp = true;
    assert!(
        generate_new(&no_oidc)
            .expect_err("MCP without OIDC must fail")
            .to_string()
            .contains("requires --auth oidc")
    );
}

#[test]
fn backend_without_mcp_has_no_mcp_files_dependencies_or_configuration() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path(), "plain-backend"))?;
    let tree = read_tree(&root)?;
    assert!(tree.keys().all(|path| !path.starts_with("mcp")));
    for contents in tree.values() {
        let source = String::from_utf8_lossy(contents).to_ascii_lowercase();
        assert!(!source.contains("modelcontextprotocol"));
        assert!(!source.contains("capabilities.mcp"));
    }
    assert!(!baukit_cli::read_manifest(&root)?.capabilities.mcp);
    Ok(())
}

fn assert_deterministic_snapshot(
    make_options: impl Fn(&Path) -> NewOptions,
    expected: &str,
) -> anyhow::Result<()> {
    let first_parent = tempfile::tempdir()?;
    let second_parent = tempfile::tempdir()?;
    let first = generate_new(&make_options(first_parent.path()))?;
    let second = generate_new(&make_options(second_parent.path()))?;
    let first_tree = read_tree(&first)?;
    let second_tree = read_tree(&second)?;
    assert_eq!(
        first_tree, second_tree,
        "same inputs must produce identical bytes"
    );
    assert_eq!(render_hash_snapshot(&first_tree), expected);
    Ok(())
}

fn read_tree(root: &Path) -> anyhow::Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn visit(
        root: &Path,
        directory: &Path,
        files: &mut BTreeMap<PathBuf, Vec<u8>>,
    ) -> anyhow::Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, files)?;
            } else {
                files.insert(path.strip_prefix(root)?.to_path_buf(), fs::read(path)?);
            }
        }
        Ok(())
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

fn render_hash_snapshot(tree: &BTreeMap<PathBuf, Vec<u8>>) -> String {
    let mut snapshot = String::new();
    for (path, contents) in tree {
        let digest = sha256_hex(contents);
        snapshot.push_str(&format!("{digest}  {}\n", path.display()));
    }
    snapshot
}

fn sha256_hex(contents: &[u8]) -> String {
    Sha256::digest(contents)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn doctor_follows_relocated_identity_modules_and_binding_names() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "relocated-identity");
    local.mobile = true;
    local.web = true;
    local.mcp = true;
    local.auth = Some(AuthProvider::Oidc);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    for component in ["mobile", "web"] {
        let directory = root.join(component).join("src");
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            if !path.is_file()
                || !matches!(
                    path.extension().and_then(|value| value.to_str()),
                    Some("ts" | "tsx")
                )
            {
                continue;
            }
            let source = fs::read_to_string(&path)?;
            fs::write(
                &path,
                source
                    .replace("PRODUCT_NAME", "APPLICATION_ID")
                    .replace("ENV_PREFIX", "CONFIG_PREFIX")
                    .replace("./product", "./identity"),
            )?;
        }
        fs::rename(directory.join("product.ts"), directory.join("identity.ts"))?;
    }
    let config = root.join("mobile/app.config.ts");
    fs::write(
        &config,
        fs::read_to_string(&config)?
            .replace("PRODUCT_NAME", "APPLICATION_ID")
            .replace("./src/product", "./src/identity"),
    )?;
    let api = root.join("backend/crates/relocated-identity-bin/src/bin/api.rs");
    fs::write(
        &api,
        fs::read_to_string(&api)?.replace("PRODUCT", "APPLICATION_ID"),
    )?;
    doctor(&root)?;
    let identity = root.join("web/src/identity.ts");
    let source = fs::read_to_string(&identity)?;
    fs::write(
        &identity,
        source.replace("relocated-identity", "different-product"),
    )?;
    let error = doctor(&root)
        .expect_err("a moved identity must still be checked")
        .to_string();
    assert!(error.contains("APPLICATION_ID"), "{error}");
    assert!(error.contains("web/src/analytics.ts"), "{error}");
    fs::write(&identity, source)?;
    fs::remove_file(&identity)?;
    let error = doctor(&root)
        .expect_err("an unresolved consumed identity must fail")
        .to_string();
    assert!(error.contains("has no literal source"), "{error}");
    Ok(())
}

#[test]
fn doctor_accepts_product_review_identity_layouts() -> anyhow::Result<()> {
    for (name, slug) in [
        ("eigenruhe", "eigenruhe"),
        ("tiefgang", "tiefgang"),
        ("sl", "solo-leveling"),
        ("finops", "finops"),
    ] {
        let parent = tempfile::tempdir()?;
        let mut local = options(parent.path(), name);
        local.mobile = true;
        local.web = true;
        local.mcp = name != "sl";
        local.auth = (name != "sl").then_some(AuthProvider::Oidc);
        local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
        let root = generate_new(&local)?;
        if name == "sl" {
            for path in ["src/bin/api.rs", "src/bin/migrate.rs"] {
                let path = root.join("backend/crates/sl-bin").join(path);
                fs::write(
                    &path,
                    fs::read_to_string(&path)?.replace("\"sl\"", "\"solo-leveling-system\""),
                )?;
            }
        }
        let mobile = root.join("mobile/app.config.ts");
        let source = fs::read_to_string(&mobile)?;
        fs::write(
            &mobile,
            source
                .replace("import { PRODUCT_NAME } from './src/product.ts';", "")
                .replace("PRODUCT_NAME", &format!("'{slug}'")),
        )?;
        let analytics = root.join("mobile/src/analytics-client.ts");
        fs::write(
            &analytics,
            fs::read_to_string(&analytics)?
                .replace(
                    "import { PRODUCT_NAME } from './product';",
                    &format!(
                        "const ANALYTICS_APP = '{}';",
                        if name == "sl" {
                            "solo-leveling-system"
                        } else {
                            name
                        }
                    ),
                )
                .replace("PRODUCT_NAME", "ANALYTICS_APP"),
        )?;
        fs::remove_file(root.join("mobile/src/product.ts"))?;
        let library = root.join(format!("backend/crates/{name}-bin/src/lib.rs"));
        if local.auth.is_some() {
            let source = fs::read_to_string(&library)?;
            fs::write(
                &library,
                source
                    .replace(&format!("const PRODUCT: &str = \"{name}\";"), "")
                    .replace("{PRODUCT}", name)
                    .replace("PRODUCT.to_owned()", &format!("\"{name}\".to_owned()")),
            )?;
        }
        let api = root.join(format!("backend/crates/{name}-bin/src/bin/api.rs"));
        fs::write(
            &api,
            fs::read_to_string(&api)?.replace("PRODUCT", "APP_NAME"),
        )?;
        doctor(&root)?;
        let source = fs::read_to_string(&api)?;
        fs::write(
            &api,
            source.replace(
                &format!(
                    "const APP_NAME: &str = \"{}\";",
                    if name == "sl" {
                        "solo-leveling-system"
                    } else {
                        name
                    }
                ),
                "",
            ),
        )?;
        let error = doctor(&root)
            .expect_err("removing the consumed backend identity must fail")
            .to_string();
        assert!(error.contains("APP_NAME"), "{error}");
        assert!(error.contains("has no literal source"), "{error}");
    }
    Ok(())
}

#[test]
fn doctor_ignores_identity_examples_in_comments_and_strings() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "identity-examples");
    local.mobile = true;
    local.web = true;
    local.mcp = true;
    local.auth = Some(AuthProvider::Oidc);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    for (relative, examples) in [
        (
            "web/src/analytics.ts",
            "const example = \"new AnalyticsClient({ app: 'wrong-product' })\";\n// new AnalyticsClient({ app: 'wrong-product' })\n",
        ),
        (
            "mobile/src/product.ts",
            "const example = \"const PRODUCT_NAME = 'wrong-product';\";\n",
        ),
        (
            "backend/crates/identity-examples-bin/src/bin/api.rs",
            "const EXAMPLE: &str = \"ConfigLoader::new(UNKNOWN, environment)\";\ntype Callback = fn(&'static str);\n// ConfigLoader::new(UNKNOWN, environment)\n",
        ),
    ] {
        let path = root.join(relative);
        let original = fs::read_to_string(&path)?;
        fs::write(&path, format!("{examples}{original}"))?;
        doctor(&root)?;
    }
    Ok(())
}

#[test]
fn doctor_accepts_renamed_crates_and_relocated_wiring() -> anyhow::Result<()> {
    let roles = [
        "domain", "ports", "services", "api", "postgres", "bin", "worker",
    ];
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "layout-product");
    local.worker = true;
    local.mobile = true;
    local.auth = Some(AuthProvider::Oidc);
    local.port_offset = 100;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    doctor(&root)?;
    for (relative, _) in read_tree(&root)? {
        let relative = relative.to_str().expect("generated paths use UTF-8");
        let path = root.join(relative);
        if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("rs" | "toml")
        ) {
            let mut source = fs::read_to_string(&path)?;
            for role in roles {
                source = source.replace(&format!("layout-product-{role}"), &format!("sl-{role}"));
            }
            fs::write(path, source.replace("layout_product_", "sl_"))?;
        }
    }
    for role in roles {
        fs::rename(
            root.join(format!("backend/crates/layout-product-{role}")),
            root.join(format!("backend/crates/sl-{role}")),
        )?;
    }
    let bin = root.join("backend/crates/sl-bin");
    fs::rename(root.join("backend/tests"), bin.join("verification"))?;
    let cargo = bin.join("Cargo.toml");
    fs::write(
        &cargo,
        fs::read_to_string(&cargo)?
            .replace("../../tests/", "verification/")
            .replace("src/bin/worker.rs", "src/bin/jobs.rs"),
    )?;
    fs::rename(bin.join("src/bin/worker.rs"), bin.join("src/bin/jobs.rs"))?;
    let domain = root.join("backend/crates/sl-domain/src");
    fs::rename(domain.join("limits.rs"), domain.join("policy.rs"))?;
    let lib = domain.join("lib.rs");
    fs::write(
        &lib,
        fs::read_to_string(&lib)?
            .replace("mod limits;", "mod policy;")
            .replace("limits::", "policy::"),
    )?;
    for (old, new) in [
        ("auth", "session"),
        ("local-data", "partition"),
        ("persistence-lifecycle", "identity-storage"),
    ] {
        fs::rename(
            root.join(format!("mobile/src/{old}.ts")),
            root.join(format!("mobile/src/{new}.ts")),
        )?;
        let test = root.join(format!("mobile/src/{old}.test.ts"));
        if test.is_file() {
            fs::rename(test, root.join(format!("mobile/src/{new}.test.ts")))?;
        }
        for (relative, _) in read_tree(&root)? {
            let relative = relative.to_str().expect("generated paths use UTF-8");
            if !relative.starts_with("mobile/")
                || !matches!(
                    Path::new(&relative)
                        .extension()
                        .and_then(|value| value.to_str()),
                    Some("ts" | "tsx")
                )
            {
                continue;
            }
            let path = root.join(relative);
            let source = fs::read_to_string(&path)?;
            fs::write(
                path,
                source
                    .replace(&format!("/{old}'"), &format!("/{new}'"))
                    .replace(&format!("/{old}\""), &format!("/{new}\"")),
            )?;
        }
    }
    fs::rename(
        root.join("mobile/app/(auth)/sign-in.tsx"),
        root.join("mobile/app/(auth)/login.tsx"),
    )?;
    let login = root.join("mobile/app/(auth)/login.tsx");
    fs::write(
        &login,
        fs::read_to_string(&login)?
            .replace("{ useAuth }", "{ useAuth, useLogin }")
            .replace(
                "const auth = useAuth();",
                "const auth = useAuth();\n  const login = useLogin();",
            )
            .replace("auth.signIn(mode)", "login(mode)"),
    )?;
    let session = root.join("mobile/src/session.ts");
    fs::write(
        &session,
        format!("{}\nexport function useLogin() {{ return useAuth().signIn; }}\n", fs::read_to_string(&session)?)
            .replace("createExpoOidcClient }", "createExpoOidcEnvironment }")
            .replace("  appearanceStateDecoration,", "  NativeOidcClient,\n  appearanceStateDecoration,")
            .replace("authClient = createExpoOidcClient(", "authClient = new NativeOidcClient(")
            .replace(
                "  {\n    randomBytes: (size) => Crypto.getRandomBytesAsync(size),\n    storage: authStorage,\n  },",
                "  createExpoOidcEnvironment({\n    randomBytes: (size) => Crypto.getRandomBytesAsync(size),\n    storage: authStorage,\n  }),",
            ),
    )?;
    let auth_test = root.join("mobile/src/oidc-auth.test.tsx");
    fs::write(
        &auth_test,
        fs::read_to_string(&auth_test)?
            .replace(
                "import { createExpoOidcClient } from '@baukit/auth-native/expo';",
                "import { NativeOidcClient } from '@baukit/auth-native';",
            )
            .replace(
                "jest.mock('@baukit/auth-native/expo', () => {",
                "jest.mock('@baukit/auth-native', () => {\n  const actual = jest.requireActual<typeof import('@baukit/auth-native')>('@baukit/auth-native');",
            )
            .replace(
                "return { completeExpoAuthSession: jest.fn(), createExpoOidcClient: jest.fn(() => client) };",
                "return { ...actual, NativeOidcClient: jest.fn(() => client) };",
            )
            .replace(
                "jest.mocked(createExpoOidcClient)",
                "jest.mocked(NativeOidcClient)",
            ),
    )?;
    fs::create_dir(root.join("docker"))?;
    fs::rename(root.join("keycloak"), root.join("docker/keycloak"))?;
    fs::rename(
        root.join("docker/keycloak/realm.json"),
        root.join("docker/keycloak/export.json"),
    )?;
    fs::rename(
        root.join("docker/keycloak/CHANGELOG.md"),
        root.join("docker/keycloak/DECISIONS.md"),
    )?;
    fs::create_dir(root.join("tools"))?;
    fs::rename(
        root.join("scripts/pkce-login.py"),
        root.join("tools/login.py"),
    )?;
    for script in ["keycloak_policy.py", "reconcile_keycloak.py"] {
        fs::rename(
            root.join("scripts").join(script),
            root.join("tools").join(script),
        )?;
    }
    fs::create_dir(root.join("database"))?;
    fs::rename(
        root.join("backend/migrations"),
        root.join("database/schema"),
    )?;
    for (relative, contents) in read_tree(&root)? {
        let Ok(source) = String::from_utf8(contents) else {
            continue;
        };
        let path = root.join(relative);
        let mut source = source
            .replace("/(auth)/sign-in", "/(auth)/login")
            .replace("../../migrations", "../../../database/schema")
            .replace("backend/migrations", "database/schema")
            .replace("keycloak/", "docker/keycloak/")
            .replace("docker/keycloak/realm.json", "docker/keycloak/export.json")
            .replace(
                "docker/keycloak/CHANGELOG.md",
                "docker/keycloak/DECISIONS.md",
            )
            .replace("scripts/pkce-login.py", "tools/login.py")
            .replace("scripts/keycloak_policy.py", "tools/keycloak_policy.py")
            .replace(
                "scripts/reconcile_keycloak.py",
                "tools/reconcile_keycloak.py",
            )
            .replace(
                "TEST_DIRECTORY.parent / \"keycloak_policy.py\"",
                "TEST_DIRECTORY.parents[1] / \"tools\" / \"keycloak_policy.py\"",
            )
            .replace(
                "SCRIPT_DIRECTORY = TEST_DIRECTORY.parent",
                "SCRIPT_DIRECTORY = TEST_DIRECTORY.parents[1] / \"tools\"",
            )
            .replace(
                "ROOT / \"scripts\" / \"keycloak_policy.py\"",
                "ROOT / \"tools\" / \"keycloak_policy.py\"",
            )
            .replace(
                "ROOT / \"scripts\" / \"reconcile_keycloak.py\"",
                "ROOT / \"tools\" / \"reconcile_keycloak.py\"",
            )
            .replace(
                "SCRIPT_DIRECTORY.parent / \"keycloak\"",
                "SCRIPT_DIRECTORY.parent / \"docker\" / \"keycloak\"",
            )
            .replace(
                "Path(__file__).resolve().parent.parent))",
                "Path(__file__).resolve().parents[2] / \"tools\"))",
            )
            .replace("ROOT / \"keycloak\"", "ROOT / \"docker\" / \"keycloak\"")
            .replace("/ \"realm.json\"", "/ \"export.json\"");
        for role in roles {
            source = source.replace(&format!("layout-product-{role}"), &format!("sl-{role}"));
        }
        fs::write(path, source)?;
    }
    let manifest = root.join("baukit.toml");
    fs::write(
        &manifest,
        format!(
            "{}\n[doctor]\nmigrations = \"database/schema\"\n",
            fs::read_to_string(&manifest)?
        ),
    )?;
    doctor(&root)?;

    let api = bin.join("src/bin/api.rs");
    let source = fs::read_to_string(&api)?;
    fs::write(
        &api,
        source.replace("\"layout-product\"", "\"wrong-product\""),
    )?;
    let error = doctor(&root)
        .expect_err("short crate names must not hide identity drift")
        .to_string();
    assert!(
        error.contains("backend/crates/sl-bin/src/bin/api.rs"),
        "{error}"
    );
    assert!(error.contains("does not match application name"), "{error}");
    fs::write(api, source)?;
    let policy = root.join("docker/keycloak/realm-policy.json");
    fs::copy(&policy, root.join("docker/other-policy.json"))?;
    assert!(
        doctor(&root)
            .expect_err("ambiguous policy needs a declaration")
            .to_string()
            .contains("multiple Keycloak policy files")
    );
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)?.replace(
            "[doctor]\n",
            "[doctor]\nkeycloak_policy = \"docker/keycloak/realm-policy.json\"\n",
        ),
    )?;
    doctor(&root)?;
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)?
            .replace("docker/keycloak/realm-policy.json", "docker/missing.json"),
    )?;
    assert!(
        doctor(&root)
            .expect_err("declared missing files must stay findings")
            .to_string()
            .contains("missing Keycloak policy file `docker/missing.json`")
    );
    Ok(())
}

#[test]
fn doctor_checks_declared_backend_paths_and_source_contents() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "custom-paths");
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    fs::rename(root.join("backend"), root.join("server"))?;
    fs::create_dir(root.join("build"))?;
    fs::rename(
        root.join("server/Dockerfile"),
        root.join("build/backend.Dockerfile"),
    )?;
    fs::rename(
        root.join("server/.dockerignore"),
        root.join("build/backend.dockerignore"),
    )?;
    let mut manifest = baukit_cli::read_manifest(&root)?;
    manifest.openapi.schema = manifest.openapi.schema.replace("backend/", "server/");
    manifest.doctor.backend_manifest = Some("server/Cargo.toml".to_owned());
    manifest.doctor.backend_dockerfile = Some("build/backend.Dockerfile".to_owned());
    manifest.doctor.backend_dockerignore = Some("build/backend.dockerignore".to_owned());
    manifest.doctor.migrations = Some("server/migrations".to_owned());
    let limits = "server/crates/custom-paths-domain/src/limits.rs";
    manifest
        .doctor
        .sources
        .insert("backend_limits".to_owned(), limits.to_owned());
    let path = root.join("baukit.toml");
    fs::write(&path, toml::to_string(&manifest)?)?;
    doctor(&root)?;
    let source = fs::read_to_string(root.join(limits))?;
    fs::write(root.join(limits), "// check_measurement is not wired\n")?;
    assert!(
        doctor(&root)
            .expect_err("comments do not establish wiring")
            .to_string()
            .contains("missing backend_limits wiring")
    );
    fs::write(root.join(limits), source)?;
    for invalid in ["../Cargo.toml", "/tmp/Cargo.toml"] {
        manifest.doctor.backend_manifest = Some(invalid.to_owned());
        fs::write(&path, toml::to_string(&manifest)?)?;
        assert!(
            doctor(&root)
                .expect_err("doctor paths stay within the product")
                .to_string()
                .contains("must be relative to the product root")
        );
    }
    manifest.doctor.backend_manifest = Some("server/Cargo.toml".to_owned());
    manifest
        .doctor
        .sources
        .insert("limits_typo".to_owned(), limits.to_owned());
    fs::write(&path, toml::to_string(&manifest)?)?;
    assert!(
        doctor(&root)
            .expect_err("unknown source keys must not disable a check")
            .to_string()
            .contains("unknown doctor.sources key `limits_typo`")
    );
    Ok(())
}

#[test]
fn doctor_reports_missing_wiring_instead_of_template_filenames() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "missing-wiring");
    local.worker = true;
    local.mobile = true;
    local.auth = Some(AuthProvider::Oidc);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    doctor(&root)?;
    let mut manifest = baukit_cli::read_manifest(&root)?;
    manifest
        .doctor
        .sources
        .insert("pkce_login".to_owned(), "scripts/pkce-login.py".to_owned());
    manifest.doctor.sources.insert(
        "keycloak_reconcile_tests".to_owned(),
        "scripts/tests/test_reconcile_keycloak.py".to_owned(),
    );
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    for (relative, finding) in [
        (
            "backend/Dockerfile",
            "missing expected backend file `backend/Dockerfile`",
        ),
        (
            "backend/.dockerignore",
            "missing expected backend file `backend/.dockerignore`",
        ),
        (
            "backend/crates/missing-wiring-domain/src/limits.rs",
            "missing backend_limits wiring",
        ),
        (
            "backend/crates/missing-wiring-bin/src/bin/worker.rs",
            "missing worker_entry wiring",
        ),
        ("scripts/pkce-login.py", "missing pkce_login wiring"),
        ("keycloak/realm.json", "missing Keycloak realm;"),
        ("keycloak/realm-policy.json", "missing Keycloak policy;"),
        (
            "keycloak/reconcile.json",
            "missing Keycloak reconciliation config;",
        ),
        (
            "scripts/keycloak_policy.py",
            "missing Keycloak tool `keycloak_policy.py`",
        ),
        (
            "scripts/reconcile_keycloak.py",
            "missing Keycloak tool `reconcile_keycloak.py`",
        ),
        (
            "scripts/tests/test_reconcile_keycloak.py",
            "missing keycloak_reconcile_tests wiring",
        ),
        (
            "mobile/app/(auth)/sign-in.tsx",
            "missing mobile_sign_in wiring",
        ),
        ("mobile/src/auth.ts", "missing mobile_auth wiring"),
        (
            "mobile/src/local-data.ts",
            "missing mobile_local_data wiring",
        ),
        (
            "mobile/src/persistence-lifecycle.ts",
            "missing mobile_persistence wiring",
        ),
    ] {
        let path = root.join(relative);
        let source = fs::read_to_string(&path)?;
        fs::remove_file(&path)?;
        let error = doctor(&root).expect_err(finding).to_string();
        assert!(error.contains(finding), "{relative}: {error}");
        fs::write(path, source)?;
    }
    for (directory, extension, markers, finding) in [
        (
            "backend/tests",
            "rs",
            &["WorkerRunner"][..],
            "missing worker_tests wiring",
        ),
        (
            "backend/tests",
            "rs",
            &["MockOidcServer", "check_auth_router_conformance"][..],
            "missing auth_tests wiring",
        ),
        (
            "mobile/src",
            "ts",
            &["signIn", "signInWithOidc"][..],
            "missing mobile_auth_tests wiring",
        ),
        (
            "scripts/tests",
            "py",
            &["validate_realm"][..],
            "missing keycloak_policy_tests wiring",
        ),
    ] {
        let mut removed = Vec::new();
        for (relative, _) in read_tree(&root)? {
            let relative = relative.to_str().expect("generated paths use UTF-8");
            let path = root.join(relative);
            if !relative.starts_with(directory)
                || !path.extension().is_some_and(|value| {
                    value == extension || (extension == "ts" && value == "tsx")
                })
            {
                continue;
            }
            let source = fs::read_to_string(&path)?;
            if markers.iter().any(|marker| source.contains(marker))
                && (extension != "ts" || relative.contains(".test."))
            {
                fs::remove_file(&path)?;
                removed.push((path, source));
            }
        }
        assert!(!removed.is_empty());
        let error = doctor(&root).expect_err(finding).to_string();
        assert!(error.contains(finding), "{error}");
        for (path, source) in removed {
            fs::write(path, source)?;
        }
    }
    let migration = root.join("backend/migrations/0003_baukit_jobs.sql");
    fs::remove_file(&migration)?;
    assert!(
        doctor(&root)
            .expect_err("missing durable jobs schema")
            .to_string()
            .contains("no backend migration creates the baukit-jobs `job_outbox` table")
    );
    Ok(())
}

#[test]
fn doctor_analytics_none_does_not_require_analytics_code_or_dependencies() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = frontend_options(parent.path(), "no-analytics", true, true);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    for app in ["mobile", "web"] {
        fs::remove_file(root.join(app).join("src/analytics.ts"))?;
        let path = root.join(app).join("package.json");
        let mut package: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
        let dependencies = package["dependencies"]
            .as_object_mut()
            .expect("dependencies");
        dependencies.remove("@baukit/analytics-core");
        dependencies.remove("@baukit/analytics-posthog-native");
        fs::write(path, serde_json::to_string_pretty(&package)?)?;
    }
    let error = doctor(&root)
        .expect_err("selected analytics must still be checked")
        .to_string();
    assert!(error.contains("mobile/src/analytics.ts"), "{error}");
    assert!(error.contains("web/src/analytics.ts"), "{error}");
    assert!(
        error.contains("missing dependency `@baukit/analytics-core`"),
        "{error}"
    );
    let manifest = root.join("baukit.toml");
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)?.replace("analytics = \"posthog\"", "analytics = \"none\""),
    )?;
    doctor(&root)?;
    Ok(())
}

#[test]
fn doctor_requires_redis_only_for_redis_backed_features() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "postgres-limits");
    local.auth = Some(AuthProvider::Oidc);
    local.port_offset = 100;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let makefile = root.join("Makefile");
    let original = fs::read_to_string(&makefile)?;
    fs::write(
        &makefile,
        original.replace("REDIS_URL=redis://127.0.0.1:6479/", ""),
    )?;
    assert!(
        doctor(&root)
            .expect_err("Redis rate limiting needs a URL")
            .to_string()
            .contains("generated file `Makefile` does not use port offset 100")
    );
    let api = root.join("backend/crates/postgres-limits-bin/src/bin/api.rs");
    let source = fs::read_to_string(&api)?;
    let import_start = source
        .find("use baukit_ratelimit::{")
        .expect("rate limit imports");
    let import_end = source[import_start..].find("};").expect("import end") + import_start + 2;
    let mut postgres = source.clone();
    postgres.replace_range(import_start..import_end, "");
    let limiter_start = postgres
        .find("    let rate_limit_options =")
        .expect("rate limit setup");
    let limiter_end = postgres
        .find("    // Axum runs")
        .expect("authentication setup");
    postgres.replace_range(
        limiter_start..limiter_end,
        "    let api = api.layer(middleware::from_fn_with_state(pool.clone(), postgres_limit));\n",
    );
    postgres = postgres.replace("const ITEM_WRITE_GROUP: &str = \"item_writes\";\n", "");
    postgres = postgres.replace(
        "PostgresUserRepository::new(\n        pool,",
        "PostgresUserRepository::new(\n        pool.clone(),",
    );
    let postgres_limiter = r#"
async fn postgres_limit(
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
    request: Request,
    next: middleware::Next,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    use axum::http::StatusCode;
    if !is_item_write(&request) {
        return Ok(next.run(request).await);
    }
    let principal = request.extensions().get::<Principal>().ok_or(StatusCode::UNAUTHORIZED)?;
    let consumed: i64 = sqlx::query_scalar(
        "INSERT INTO product_rate_limits (subject, window_start, consumed) \
         VALUES ($1, date_trunc('minute', now()), 1) \
         ON CONFLICT (subject, window_start) DO UPDATE \
         SET consumed = product_rate_limits.consumed + 1 RETURNING consumed",
    )
    .bind(item_write_subject(principal))
    .fetch_one(&pool)
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let limit = i64::try_from(ITEM_WRITE_REQUESTS_PER_MINUTE)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if consumed > limit {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    Ok(next.run(request).await)
}
"#;
    fs::write(&api, format!("{postgres}{postgres_limiter}"))?;
    fs::write(
        root.join("backend/migrations/0009_product_rate_limits.sql"),
        "CREATE TABLE product_rate_limits (subject TEXT NOT NULL, window_start TIMESTAMPTZ NOT NULL, consumed BIGINT NOT NULL, PRIMARY KEY (subject, window_start));\n",
    )?;
    doctor(&root)?;
    fs::write(api, source)?;
    fs::write(&makefile, original)?;
    doctor(&root)?;
    Ok(())
}

#[test]
fn generated_preview_binds_the_ipv4_readiness_address() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&frontend_options(
        parent.path(),
        "preview-host",
        false,
        true,
    ))?;
    let config = fs::read_to_string(root.join("web/e2e/playwright.config.ts"))?;
    assert!(config.contains("http://127.0.0.1:"));
    assert!(config.contains("vite preview --host 127.0.0.1 --port"));
    Ok(())
}

#[test]
fn doctor_resolves_crate_identity_in_library_modules() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "crate-identity");
    local.auth = Some(AuthProvider::Oidc);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let library = root.join("backend/crates/crate-identity-bin/src/lib.rs");
    let original = fs::read_to_string(&library)?;
    fs::write(&library, format!("pub mod config;\n{original}"))?;
    let config = library.with_file_name("config.rs");
    for (import, binding) in [
        ("use crate::PRODUCT;", "PRODUCT"),
        ("", "crate::PRODUCT"),
        ("use crate::{PRODUCT as APP};", "APP"),
    ] {
        fs::write(
            &config,
            format!(
                "{import}\npub fn loader(environment: baukit_config::Environment) -> Result<baukit_config::ConfigLoader, baukit_config::LoadError> {{\n    baukit_config::ConfigLoader::new({binding}, environment)\n}}\n"
            ),
        )?;
        doctor(&root)?;
        fs::write(
            &library,
            fs::read_to_string(&library)?.replace(
                "const PRODUCT: &str = \"crate-identity\";",
                "const PRODUCT: &str = \"wrong-product\";",
            ),
        )?;
        let error = doctor(&root)
            .expect_err("crate references must not hide identity drift")
            .to_string();
        assert!(
            error.contains("config.rs") && error.contains("does not match application name"),
            "{error}"
        );
        fs::write(&library, format!("pub mod config;\n{original}"))?;
    }
    let binary = library
        .parent()
        .expect("crate source directory")
        .join("bin/api.rs");
    let source = fs::read_to_string(&binary)?;
    fs::write(
        &binary,
        source.replace(
            "ConfigLoader::new(PRODUCT,",
            "ConfigLoader::new(crate::PRODUCT,",
        ),
    )?;
    doctor(&root)?;
    fs::write(
        &binary,
        fs::read_to_string(&binary)?.replace(
            "const PRODUCT: &str = \"crate-identity\";",
            "const PRODUCT: &str = \"wrong-product\";",
        ),
    )?;
    let error = doctor(&root)
        .expect_err("binary crate constants belong to the binary")
        .to_string();
    assert!(error.contains("api.rs"), "{error}");
    Ok(())
}

#[test]
fn doctor_accepts_registry_dependencies_without_a_checkout() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "registry-product");
    local.mobile = true;
    let root = generate_new(&local)?;
    let manifest = baukit_cli::read_manifest(&root)?;
    assert!(matches!(
        manifest.dependencies.baukit,
        baukit_cli::BaukitDependency::Registry { .. }
    ));
    doctor(&root)?;
    Ok(())
}

#[test]
fn doctor_checks_optional_pkce_tools_and_selected_realms() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "realm-product");
    local.auth = Some(AuthProvider::Oidc);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    fs::remove_file(root.join("scripts/pkce-login.py"))?;
    doctor(&root)?;
    let mut manifest = baukit_cli::read_manifest(&root)?;
    manifest
        .doctor
        .sources
        .insert("pkce_login".to_owned(), "tools/login.py".to_owned());
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    assert!(
        doctor(&root)
            .expect_err("declared helpers must exist")
            .to_string()
            .contains("missing pkce_login wiring")
    );
    manifest.doctor.sources.remove("pkce_login");
    let library = root.join("backend/crates/realm-product-bin/src/lib.rs");
    fs::write(
        &library,
        fs::read_to_string(&library)?.replace(
            "identity_admin_realm: PRODUCT.to_owned()",
            "identity_admin_realm: \"identity-realm\".to_owned()",
        ),
    )?;
    fs::write(
        &library,
        fs::read_to_string(&library)?.replace(
            "issuer: format!(\"http://localhost:8081/realms/{PRODUCT}\")",
            "issuer: \"http://localhost:8081/realms/identity-realm\".to_owned()",
        ),
    )?;
    for (relative, contents) in read_tree(&root)? {
        if let Ok(source) = String::from_utf8(contents) {
            fs::write(
                root.join(relative),
                source.replace("/realms/realm-product", "/realms/identity-realm"),
            )?;
        }
    }
    for name in ["realm.json", "realm-policy.json"] {
        let path = root.join("keycloak").join(name);
        let mut document: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
        document["realm"] = serde_json::Value::String("identity-realm".to_owned());
        fs::write(path, serde_json::to_string_pretty(&document)?)?;
    }
    let mut other_realm: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("keycloak/realm.json"))?)?;
    other_realm["realm"] = serde_json::Value::String("external-sso".to_owned());
    fs::write(
        root.join("keycloak/other-realm.json"),
        serde_json::to_string_pretty(&other_realm)?,
    )?;
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    assert!(
        doctor(&root)
            .expect_err("ambiguous realms require a declaration")
            .to_string()
            .contains("multiple Keycloak realm files")
    );
    manifest.doctor.keycloak_realm = Some("keycloak/realm.json".to_owned());
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    doctor(&root)?;
    fs::write(
        &library,
        fs::read_to_string(&library)?.replace("identity-realm", "wrong-realm"),
    )?;
    assert!(
        doctor(&root)
            .expect_err("admin realm must match the selected realm")
            .to_string()
            .contains("does not match selected Keycloak realm")
    );
    Ok(())
}

#[test]
fn doctor_follows_route_reexports_and_reconciliation_tests() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "route-product");
    local.mobile = true;
    local.auth = Some(AuthProvider::Oidc);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let route = root.join("mobile/app/(auth)/sign-in.tsx");
    fs::rename(&route, root.join("mobile/src/sign-in.tsx"))?;
    let screen = root.join("mobile/src/sign-in.tsx");
    fs::write(
        &screen,
        fs::read_to_string(&screen)?.replace("../../src/", "./"),
    )?;
    fs::write(&route, "export { default } from '../../src/sign-in';\n")?;
    let tests = root.join("scripts/tests/test_reconcile_keycloak.py");
    fs::write(
        &tests,
        fs::read_to_string(&tests)?.replace("load_reconcile_config", "load_config"),
    )?;
    let tool = root.join("scripts/reconcile_keycloak.py");
    fs::write(
        &tool,
        format!(
            "{}\nload_config = load_reconcile_config\n",
            fs::read_to_string(&tool)?
        ),
    )?;
    doctor(&root)?;
    fs::write(
        &route,
        "export default function SignInScreen() { return null; }\n",
    )?;
    assert!(
        doctor(&root)
            .expect_err("disconnected auth code does not wire a screen")
            .to_string()
            .contains("missing mobile_sign_in wiring")
    );
    Ok(())
}

#[test]
fn doctor_follows_imported_screen_wrappers() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "wrapped-route");
    local.mobile = true;
    local.auth = Some(AuthProvider::Oidc);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let route = root.join("mobile/app/(auth)/sign-in.tsx");
    let screen = root.join("mobile/src/login-screen.tsx");
    fs::rename(&route, &screen)?;
    fs::write(
        &screen,
        fs::read_to_string(&screen)?.replace("../../src/", "./"),
    )?;
    fs::write(
        root.join("mobile/src/login-wrapper.tsx"),
        "import Screen from './login-screen';\nexport default function Wrapper() { return <Screen />; }\n",
    )?;
    fs::write(
        &route,
        "import Wrapper from '../../src/login-wrapper';\nexport default function Route() { return <Wrapper />; }\n",
    )?;
    doctor(&root)?;
    fs::write(
        &route,
        "import Wrapper from '../../src/login-wrapper';\nexport default function Route() { return null; }\n",
    )?;
    let error = doctor(&root)
        .expect_err("an unused screen import does not connect sign-in")
        .to_string();
    assert!(error.contains("missing mobile_sign_in wiring"), "{error}");
    fs::write(
        &route,
        "import Wrapper from '../../src/login-wrapper';\nexport default function Route() { return <Wrapper />; }\n",
    )?;
    fs::write(
        root.join("mobile/src/login-wrapper.tsx"),
        "import Route from '../app/(auth)/sign-in';\nexport default function Wrapper() { return <Route />; }\n",
    )?;
    let error = doctor(&root)
        .expect_err("an import cycle cannot connect the login screen")
        .to_string();
    assert!(error.contains("missing mobile_sign_in wiring"), "{error}");
    Ok(())
}

#[test]
fn doctor_ignores_archives_only_inside_git() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "archive-product");
    local.auth = Some(AuthProvider::Oidc);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    fs::create_dir(root.join(".upgrade"))?;
    fs::copy(
        root.join("scripts/keycloak_policy.py"),
        root.join(".upgrade/keycloak_policy.py"),
    )?;
    fs::write(
        root.join(".gitignore"),
        format!(
            "{}\n.upgrade/\n",
            fs::read_to_string(root.join(".gitignore"))?
        ),
    )?;
    let error = doctor(&root)
        .expect_err("outside Git the filesystem walk includes archives")
        .to_string();
    assert!(
        error.contains("multiple Keycloak tools `keycloak_policy.py`"),
        "{error}"
    );
    let toolchain_bin = Path::new(env!("CARGO"))
        .parent()
        .expect("Cargo has a toolchain directory");
    let error = Command::new("git")
        .env("PATH", toolchain_bin)
        .output()
        .expect_err("the isolated toolchain path has no Git executable");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    let output = Command::new(env!("CARGO_BIN_EXE_baukit"))
        .arg("doctor")
        .current_dir(&root)
        .env("PATH", toolchain_bin)
        .output()?;
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr)?;
    assert!(
        error.contains("multiple Keycloak tools `keycloak_policy.py`"),
        "{error}"
    );
    for arguments in [["init", "--quiet"], ["add", "."]] {
        assert!(
            Command::new("git")
                .args(arguments)
                .current_dir(&root)
                .status()?
                .success()
        );
    }
    doctor(&root)?;
    let output = Command::new(env!("CARGO_BIN_EXE_baukit"))
        .arg("doctor")
        .current_dir(&root)
        .env("PATH", toolchain_bin)
        .output()?;
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr)?;
    assert!(
        error.contains("Git is required to scan files in a Git repository"),
        "{error}"
    );
    fs::copy(
        root.join("scripts/keycloak_policy.py"),
        root.join("extra-policy.py"),
    )?;
    let error = doctor(&root)
        .expect_err("untracked product files still count")
        .to_string();
    assert!(
        error.contains("multiple Keycloak tools `keycloak_policy.py`"),
        "{error}"
    );
    Ok(())
}

#[test]
fn doctor_accepts_shared_measurements_in_product_limit_validators() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "measured-limits");
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let mut manifest = baukit_cli::read_manifest(&root)?;
    let relative = "backend/crates/measured-limits-domain/src/custom_limits.rs";
    manifest
        .doctor
        .sources
        .insert("backend_limits".to_owned(), relative.to_owned());
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    let library = root.join("backend/crates/measured-limits-domain/src/lib.rs");
    fs::write(
        &library,
        format!("pub mod custom_limits;\n{}", fs::read_to_string(&library)?),
    )?;
    let path = root.join(relative);
    for (parameter, measured) in [
        (
            "text: &str",
            "baukit_core::limits::trimmed_unicode_scalar_count(text)",
        ),
        (
            "document: &serde_json::Value",
            "baukit_core::limits::compact_json_utf8_bytes(document).map_err(|_| \"jsonb_invalid\")?",
        ),
    ] {
        fs::write(
            &path,
            format!(
                "pub fn validate({parameter}, maximum: usize) -> Result<(), &'static str> {{\n    if {measured} > maximum {{ Err(\"limit_exceeded\") }} else {{ Ok(()) }}\n}}\n"
            ),
        )?;
        doctor(&root)?;
    }
    fs::write(
        &path,
        "pub fn validate(text: &str, maximum: usize) -> Result<(), &'static str> {\n    if text.len() > maximum { Err(\"limit_exceeded\") } else { Ok(()) }\n}\n",
    )?;
    assert!(
        doctor(&root)
            .expect_err("text bytes do not implement the shared scalar measurement")
            .to_string()
            .contains("missing backend_limits wiring")
    );
    Ok(())
}

#[test]
fn doctor_accepts_redis_url_environment_fallback() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "redis-fallback");
    local.auth = Some(AuthProvider::Oidc);
    local.port_offset = 10;
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    let makefile = root.join("Makefile");
    let original = fs::read_to_string(&makefile)?;
    let literal = "REDIS_URL=redis://127.0.0.1:6389/";
    assert!(original.contains(literal));
    fs::write(
        &makefile,
        original.replace(literal, "REDIS_URL=$${REDIS_URL:-redis://127.0.0.1:6389/}"),
    )?;
    doctor(&root)?;
    fs::write(
        &makefile,
        original.replace(literal, "REDIS_URL=$${REDIS_URL:-redis://127.0.0.1:6379/}"),
    )?;
    assert!(
        doctor(&root)
            .expect_err("wrong fallback port")
            .to_string()
            .contains("Makefile")
    );
    Ok(())
}

#[test]
fn generated_oidc_workers_retain_failed_identity_jobs() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "retained-erasure");
    local.worker = true;
    local.auth = Some(AuthProvider::Oidc);
    local.baukit_path = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust"));
    let root = generate_new(&local)?;
    for binary in ["api", "worker"] {
        let source = fs::read_to_string(root.join(format!(
            "backend/crates/retained-erasure-bin/src/bin/{binary}.rs"
        )))?;
        assert!(
            source.contains("retain_failed_kinds(&[baukit_erasure::IDENTITY_DELETE_JOB_TYPE])")
        );
    }
    doctor(&root)?;
    Ok(())
}

#[test]
fn native_ios_workflow_embeds_the_bundle_in_the_installed_app() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&frontend_options(parent.path(), "ios-release", true, false))?;
    let workflow: serde_yaml_ng::Value =
        serde_yaml_ng::from_slice(&fs::read(root.join(".github/workflows/native.yml"))?)?;
    let steps = workflow["jobs"]["ios"]["steps"]
        .as_sequence()
        .expect("iOS steps");
    let run = |name: &str| {
        steps
            .iter()
            .find(|step| step["name"].as_str() == Some(name))
            .and_then(|step| step["run"].as_str())
            .expect("workflow step")
    };
    assert!(
        run("Compile for iOS Simulator").contains("-configuration Release -sdk iphonesimulator")
    );
    assert!(
        run("Run product-owned Maestro critical paths when configured")
            .contains("Build/Products/Release-iphonesimulator")
    );
    Ok(())
}

#[test]
fn auth_conformance_does_not_wait_for_access_token_expiry() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut product = options(parent.path(), "auth-lifetime");
    product.auth = Some(AuthProvider::Oidc);
    let root = generate_new(&product)?;
    let test = fs::read_to_string(root.join("backend/tests/auth_conformance.rs"))?;
    assert!(!test.contains("tokio::time::sleep"));
    assert!(!test.contains("Duration::from_secs(1)"));
    assert!(test.contains(".expires_at(0)"));
    assert!(test.contains("authorization_header(&expired)"));
    assert!(test.contains("refresh_session(session.refresh_token())"));
    Ok(())
}

#[test]
fn pwa_requires_an_app_and_selects_the_web_host_first() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut backend = options(parent.path(), "pwa-backend");
    backend.pwa = true;
    assert_eq!(
        generate_new(&backend)
            .expect_err("PWA needs an app")
            .to_string(),
        "--pwa requires --mobile or --web to serve the worker"
    );
    for (name, mobile, web, host) in [
        ("web-pwa", false, true, "web"),
        ("both-pwa", true, true, "web"),
        ("auth-pwa", true, false, "mobile"),
    ] {
        let mut product = frontend_options(parent.path(), name, mobile, web);
        product.pwa = true;
        if name == "auth-pwa" {
            product.auth = Some(AuthProvider::Oidc);
        }
        let root = generate_new(&product)?;
        assert!(baukit_cli::read_manifest(&root)?.capabilities.pwa);
        let package: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(host).join("package.json"))?)?;
        assert_eq!(
            package["dependencies"]["@baukit/pwa-web"],
            baukit_cli::TEMPLATE_VERSION
        );
        let output = if host == "mobile" {
            " --output-dir dist"
        } else {
            ""
        };
        assert_eq!(
            package["scripts"]["build:sw"],
            format!("node scripts/build-sw.mjs{output}")
        );
        assert_eq!(
            package["scripts"]["build:sw:check"],
            format!("node scripts/build-sw.mjs{output} --check")
        );
        assert!(
            fs::read_to_string(root.join(host).join("scripts/build-sw.mjs"))?
                .contains("@baukit/pwa-web/worker")
        );
    }
    Ok(())
}

#[test]
fn mobile_pwa_host_generates_expo_web_export_configuration() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    for (name, pwa, web, auth) in [
        ("mobile-pwa", true, false, false),
        ("auth-mobile-pwa", true, false, true),
        ("native-mobile", false, false, false),
        ("web-hosted-pwa", true, true, false),
    ] {
        let mut product = frontend_options(parent.path(), name, true, web);
        product.pwa = pwa;
        product.auth = auth.then_some(AuthProvider::Oidc);
        let root = generate_new(&product)?;
        let package: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("mobile/package.json"))?)?;
        let mobile_hosts_pwa = pwa && !web;
        for (dependency, version) in [
            ("@expo/metro-runtime", "57.0.16"),
            ("react-dom", "19.2.3"),
            ("react-native-web", "0.21.2"),
        ] {
            if mobile_hosts_pwa {
                assert_eq!(package["dependencies"][dependency], version, "{name}");
            } else {
                assert!(package["dependencies"][dependency].is_null(), "{name}");
            }
        }
        let output = if mobile_hosts_pwa {
            " --output-dir dist"
        } else {
            ""
        };
        assert_eq!(
            package["scripts"]["build:sw"],
            format!("node scripts/build-sw.mjs{output}")
        );
        assert_eq!(
            package["scripts"]["build:sw:check"],
            format!("node scripts/build-sw.mjs{output} --check")
        );
        let config = fs::read_to_string(root.join("mobile/app.config.ts"))?;
        assert_eq!(config.contains("output: 'single'"), mobile_hosts_pwa);
        assert_eq!(config.contains("bundler: 'metro'"), mobile_hosts_pwa);
        let metro = fs::read_to_string(root.join("mobile/metro.config.js"))?;
        assert_eq!(
            metro.contains("config.resolver.assetExts.push('wasm')"),
            mobile_hosts_pwa
        );
        let workflow: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(&fs::read_to_string(root.join(".github/workflows/ci.yml"))?)?;
        let steps = workflow["jobs"]["mobile"]["steps"]
            .as_sequence()
            .expect("mobile CI steps");
        assert_eq!(
            steps.iter().any(|step| step["run"]
                .as_str()
                .is_some_and(|run| run.contains("expo export --platform web"))),
            mobile_hosts_pwa
        );
        product.name = format!("{name}-strict");
        product.quality = QualityProfile::Strict;
        let strict_root = generate_new(&product)?;
        let gate = fs::read_to_string(strict_root.join("scripts/quality-gate.sh"))?;
        assert_eq!(
            gate.contains("--dir mobile exec expo export --platform web"),
            mobile_hosts_pwa
        );
    }
    Ok(())
}

#[test]
fn mobile_pwa_generation_matches_golden_tree() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut product = frontend_options(parent.path(), "snapshot-app", true, false);
    product.pwa = true;
    let root = generate_new(&product)?;
    assert_eq!(
        render_hash_snapshot(&read_tree(&root)?),
        include_str!("snapshots/mobile-pwa.tree")
    );
    Ok(())
}

#[test]
fn remote_mcp_generation_matches_the_golden_tree() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut generated = options(parent.path(), "snapshot-app");
    generated.auth = Some(AuthProvider::Oidc);
    generated.mcp = true;
    let root = generate_new(&generated)?;
    assert_eq!(
        render_hash_snapshot(&read_tree(&root)?),
        include_str!("snapshots/mcp-remote.tree")
    );
    let manifest = baukit_cli::read_manifest(&root)?;
    assert!(manifest.capabilities.mcp);
    assert!(!root.join("mcp").exists());
    let api = fs::read_to_string(root.join("backend/crates/snapshot-app-bin/src/bin/api.rs"))?;
    assert!(api.contains("mcp_policy("));
    assert!(api.contains("let mcp_services = snapshot_app_mcp::services(item_reads);"));
    let tools = fs::read_to_string(root.join("backend/crates/snapshot-app-mcp/src/lib.rs"))?;
    assert!(tools.contains("pub fn authentication_policy() -> Arc<dyn AuthenticationPolicy>"));
    assert!(tools.contains("Arc::new(JwtOnlyPolicy)"));
    assert!(tools.contains("pub fn services(items: Arc<dyn ItemReadService>) -> McpServices"));
    assert!(tools.contains("McpServices::new(Arc::new(ItemTools::new(items)))"));
    assert_eq!(manifest.openapi.consumers(), ["generated/openapi.d.ts"]);
    doctor(&root)?;
    Ok(())
}

#[test]
fn remote_mcp_requires_auth_and_backend() {
    let parent = tempfile::tempdir().expect("tempdir");
    let mut generated = options(parent.path(), "remote");
    generated.mcp = true;
    assert!(
        generate_new(&generated)
            .expect_err("OIDC required")
            .to_string()
            .contains("requires --auth oidc")
    );
    generated.auth = Some(AuthProvider::Oidc);
    generated.backend = false;
    assert!(
        generate_new(&generated)
            .expect_err("backend required")
            .to_string()
            .contains("requires --backend")
    );
}

#[test]
fn remote_realm_preserves_oidc_scopes_and_binds_the_resource_audience() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut generated = options(parent.path(), "remote");
    generated.auth = Some(AuthProvider::Oidc);
    generated.mcp = true;
    generated.web = true;
    generated.mobile = true;
    let root = generate_new(&generated)?;
    let realm: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("keycloak/realm.json"))?)?;
    let scopes = realm["clientScopes"].as_array().expect("client scopes");
    for name in [
        "basic",
        "profile",
        "email",
        "roles",
        "offline_access",
        "items:read",
    ] {
        assert!(scopes.iter().any(|scope| scope["name"] == name), "{name}");
    }
    let basic = scopes
        .iter()
        .find(|scope| scope["name"] == "basic")
        .expect("basic scope");
    assert!(
        basic["protocolMappers"]
            .as_array()
            .expect("basic mappers")
            .iter()
            .any(|mapper| {
                mapper["protocolMapper"] == "oidc-sub-mapper"
                    && mapper["config"]["access.token.claim"] == "true"
            })
    );
    assert!(
        realm["defaultDefaultClientScopes"]
            .as_array()
            .expect("default scopes")
            .contains(&serde_json::json!("basic"))
    );
    assert!(
        realm["defaultOptionalClientScopes"]
            .as_array()
            .expect("optional scopes")
            .contains(&serde_json::json!("offline_access"))
    );
    let client = realm["clients"]
        .as_array()
        .expect("clients")
        .iter()
        .find(|client| client["clientId"] == "remote-mcp")
        .expect("MCP client");
    assert_eq!(client["publicClient"], true);
    assert_eq!(client["directAccessGrantsEnabled"], false);
    assert_eq!(client["attributes"]["pkce.code.challenge.method"], "S256");
    assert_eq!(client["defaultClientScopes"], serde_json::json!(["basic"]));
    assert_eq!(
        client["optionalClientScopes"],
        serde_json::json!(["items:read"])
    );
    assert!(
        client["protocolMappers"]
            .as_array()
            .expect("MCP mappers")
            .iter()
            .any(|mapper| {
                mapper["protocolMapper"] == "oidc-audience-mapper"
                    && mapper["config"]["included.custom.audience"] == "http://localhost:8080/mcp"
                    && mapper["config"]["id.token.claim"] == "false"
            })
    );
    Ok(())
}

#[test]
fn doctor_reports_each_missing_remote_mcp_connection() -> anyhow::Result<()> {
    for (relative, symbol, finding) in [
        (
            "backend/crates/remote-bin/src/bin/api.rs",
            "baukit_mcp::router(",
            "router mount and auth layer",
        ),
        (
            "backend/crates/remote-bin/src/bin/api.rs",
            "api.merge(mcp)",
            "router mount and auth layer",
        ),
        (
            "backend/crates/remote-mcp/src/lib.rs",
            "Self::definitions()",
            "tool registration and scope enforcement",
        ),
        (
            "backend/crates/remote-mcp/Cargo.toml",
            "baukit-mcp.workspace = true",
            "auth layer",
        ),
        (
            "deploy/values.yaml",
            "allowedHosts:",
            "deployment is missing",
        ),
    ] {
        let parent = tempfile::tempdir()?;
        let mut generated = options(parent.path(), "remote");
        generated.mcp = true;
        generated.auth = Some(AuthProvider::Oidc);
        let root = generate_new(&generated)?;
        let path = root.join(relative);
        let source = fs::read_to_string(&path)?;
        assert!(source.contains(symbol));
        fs::write(path, source.replace(symbol, ""))?;
        let error = doctor(&root).expect_err("missing remote wiring");
        assert!(error.to_string().contains(finding), "{relative}: {error}");
    }
    Ok(())
}

#[test]
fn doctor_reports_migration_for_every_retired_mcp_manifest_shape() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut generated = options(parent.path(), "migration");
    generated.mcp = true;
    generated.auth = Some(AuthProvider::Oidc);
    let root = generate_new(&generated)?;
    let path = root.join("baukit.toml");
    let manifest = fs::read_to_string(&path)?;
    for capability in [
        "{ authentication = \"personal-token\" }",
        "{ authentication = \"node-oidc\" }",
        "{ authentication = \"caller-supplied\" }",
        "{ authentication = \"resource-oauth\", transport = \"remote\" }",
    ] {
        fs::write(
            &path,
            manifest.replace("mcp = true", &format!("mcp = {capability}")),
        )?;
        let finding = doctor(&root)
            .expect_err("retired capability needs migration")
            .to_string();
        assert!(
            finding.contains("retired MCP capability table"),
            "{finding}"
        );
        assert!(finding.contains("capabilities.mcp = true"), "{finding}");
        assert!(
            finding.contains("docs/migrations/mcp-stdio-to-remote.md"),
            "{finding}"
        );
        assert!(!finding.contains("could not parse"), "{finding}");
    }
    Ok(())
}

#[test]
fn doctor_reports_retired_typescript_mcp_even_without_a_capability() -> anyhow::Result<()> {
    for artifact in [
        "mcp/package.json",
        "mcp/src/server.ts",
        "mcp/src/transports/server.ts",
    ] {
        let parent = tempfile::tempdir()?;
        let root = generate_new(&options(parent.path(), "old-server"))?;
        let path = root.join(artifact);
        fs::create_dir_all(path.parent().expect("parent"))?;
        fs::write(path, "{}")?;
        let finding = doctor(&root)
            .expect_err("retired server needs migration")
            .to_string();
        assert!(
            finding.contains("retired TypeScript MCP server found in mcp/"),
            "{finding}"
        );
        assert!(
            finding.contains("docs/migrations/mcp-stdio-to-remote.md"),
            "{finding}"
        );
    }
    Ok(())
}

#[test]
fn cli_rejects_removed_mcp_selection_flags() -> anyhow::Result<()> {
    for flag in ["--mcp-transport", "--mcp-auth"] {
        let output = Command::new(env!("CARGO_BIN_EXE_baukit"))
            .args([
                "new",
                "removed-flags",
                "--backend",
                "--mcp",
                "--auth",
                "oidc",
                flag,
                "remote",
            ])
            .output()?;
        assert!(!output.status.success());
        let finding = String::from_utf8(output.stderr)?;
        assert!(finding.contains("unexpected argument"), "{finding}");
        assert!(finding.contains(flag), "{finding}");
    }
    Ok(())
}

#[test]
fn doctor_accepts_product_named_remote_tool_adapters() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut generated = options(parent.path(), "product-tools");
    generated.mcp = true;
    generated.auth = Some(AuthProvider::Oidc);
    let root = generate_new(&generated)?;
    for (relative, before, after) in [
        (
            "backend/crates/product-tools-mcp/src/lib.rs",
            "ItemTools",
            "ProductTools",
        ),
        (
            "backend/crates/product-tools-mcp/src/lib.rs",
            "pub fn services(",
            "pub fn product_services(",
        ),
        (
            "backend/crates/product-tools-bin/src/bin/api.rs",
            "product_tools_mcp::services(",
            "product_tools_mcp::product_services(",
        ),
        (
            "backend/crates/product-tools-mcp/src/bin/mcp-tools.rs",
            "ItemTools",
            "ProductTools",
        ),
        ("backend/tests/tool_drift.rs", "ItemTools", "ProductTools"),
    ] {
        let path = root.join(relative);
        let source = fs::read_to_string(&path)?;
        assert!(source.contains(before), "{relative}");
        fs::write(path, source.replace(before, after))?;
    }
    let findings = doctor(&root)?;
    assert!(
        findings
            .iter()
            .any(|finding| finding.contains("scoped tools, and drift check are wired"))
    );
    Ok(())
}

#[test]
fn doctor_accepts_non_typescript_files_in_a_root_mcp_directory() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path(), "rust-files"))?;
    fs::create_dir_all(root.join("mcp/src"))?;
    fs::write(
        root.join("mcp/src/lib.rs"),
        "pub const NAME: &str = \"rust-files\";",
    )?;
    doctor(&root)?;
    assert!(!baukit_cli::read_manifest(&root)?.capabilities.mcp);
    Ok(())
}

#[test]
fn managed_auth_providers_generate_every_flavor_and_mcp_without_keycloak() -> anyhow::Result<()> {
    for (provider, sdk, snapshot) in [
        (
            AuthProvider::Clerk,
            "@clerk/clerk-js",
            include_str!("snapshots/clerk.tree"),
        ),
        (
            AuthProvider::Workos,
            "@workos-inc/authkit-js",
            include_str!("snapshots/workos.tree"),
        ),
    ] {
        for (backend, web, mobile, mcp, worker, pwa, quality) in [
            (
                true,
                false,
                false,
                false,
                false,
                false,
                QualityProfile::Standard,
            ),
            (
                false,
                true,
                false,
                false,
                false,
                false,
                QualityProfile::Standard,
            ),
            (
                false,
                false,
                true,
                false,
                false,
                false,
                QualityProfile::Standard,
            ),
            (
                true,
                true,
                true,
                true,
                false,
                false,
                QualityProfile::Standard,
            ),
            (
                false,
                false,
                true,
                false,
                false,
                true,
                QualityProfile::Standard,
            ),
            (
                true,
                false,
                false,
                true,
                true,
                false,
                QualityProfile::Standard,
            ),
            (true, true, true, true, true, true, QualityProfile::Strict),
        ] {
            let parent = tempfile::tempdir()?;
            let mut generated = options(parent.path(), "snapshot-app");
            generated.backend = backend;
            generated.web = web;
            generated.mobile = mobile;
            generated.mcp = mcp;
            generated.worker = worker;
            generated.pwa = pwa;
            generated.quality = quality;
            generated.auth = Some(provider);
            let root = generate_new(&generated)?;
            let tree = read_tree(&root)?;
            assert!(
                tree.keys()
                    .all(|path| !path.to_string_lossy().contains("keycloak"))
            );
            assert!(!root.join("keycloak").exists());
            assert_eq!(
                baukit_cli::read_manifest(&root)?.capabilities.auth,
                Some(provider)
            );
            if backend {
                let compose = fs::read_to_string(root.join("compose.yaml"))?;
                assert!(compose.contains("redis:"));
                assert!(!compose.contains("keycloak"));
            }
            if web {
                assert!(fs::read_to_string(root.join("web/package.json"))?.contains(sdk));
            }
            if mcp && web && mobile && !worker {
                assert_eq!(render_hash_snapshot(&tree), snapshot);
            }
            doctor(&root)?;
        }
    }
    Ok(())
}

#[test]
fn doctor_reports_provider_config_sdk_and_keycloak_mismatches() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut generated = options(parent.path(), "provider-wiring");
    generated.auth = Some(AuthProvider::Clerk);
    generated.web = true;
    generated.mcp = true;
    let root = generate_new(&generated)?;
    fs::create_dir(root.join("keycloak"))?;
    fs::write(root.join("keycloak/realm.json"), "{}")?;
    let compose_path = root.join("compose.yaml");
    let mut compose: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&fs::read_to_string(&compose_path)?)?;
    compose["services"]["keycloak"] =
        serde_yaml_ng::from_str("image: quay.io/keycloak/keycloak:26.8.0")?;
    fs::write(compose_path, serde_yaml_ng::to_string(&compose)?)?;
    fs::write(
        root.join("config/local.toml"),
        "[auth]\nprovider = \"workos\"\nissuer = \"https://identity.example/realms/product\"\n[mcp]\nintrospection_client_id = \"backend\"\n",
    )?;
    let path = root.join("web/package.json");
    let mut package: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
    package["dependencies"]
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("missing dependencies"))?
        .remove("@clerk/clerk-js");
    fs::write(path, serde_json::to_string_pretty(&package)?)?;
    let error = doctor(&root)
        .expect_err("provider mismatches must fail doctor")
        .to_string();
    for finding in [
        "Keycloak realm left over",
        "compose.yaml has a Keycloak service",
        "auth.provider is `workos`",
        "auth.issuer points to a Keycloak realm",
        "Keycloak MCP introspection policy",
        "missing `@clerk/clerk-js`",
    ] {
        assert!(
            error.contains(finding),
            "missing finding {finding}: {error}"
        );
    }
    Ok(())
}

#[test]
fn doctor_accepts_external_oidc_without_the_bundled_realm() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut generated = options(parent.path(), "external-provider");
    generated.auth = Some(AuthProvider::Oidc);
    let root = generate_new(&generated)?;
    fs::remove_dir_all(root.join("keycloak"))?;
    let path = root.join("compose.yaml");
    let mut compose: serde_yaml_ng::Value = serde_yaml_ng::from_str(&fs::read_to_string(&path)?)?;
    compose["services"]
        .as_mapping_mut()
        .ok_or_else(|| anyhow::anyhow!("missing services"))?
        .remove(serde_yaml_ng::Value::String("keycloak".into()));
    fs::write(path, serde_yaml_ng::to_string(&compose)?)?;
    for issuer in [
        "https://external.example/tenant",
        "http://localhost:8181/tenant",
    ] {
        fs::write(
            root.join("config/local.toml"),
            format!(
                "[auth]\nprovider = \"oidc\"\nissuer = {issuer:?}\naudience = \"external-provider-backend\"\n",
            ),
        )?;
        doctor(&root)?;
    }
    Ok(())
}

#[test]
fn doctor_accepts_remote_mcp_modules_in_declared_crates() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "redemut-shaped");
    local.auth = Some(AuthProvider::Oidc);
    local.mcp = true;
    let root = generate_new(&local)?;
    doctor(&root)?;
    let bin = root.join("backend/crates/redemut-shaped-bin/src");
    fs::rename(bin.join("bin/api.rs"), bin.join("application.rs"))?;
    let library_path = bin.join("lib.rs");
    let library = fs::read_to_string(&library_path)?;
    let start = library
        .find("#[derive(Clone, Debug, Default, Deserialize)]")
        .expect("config start");
    let end = library
        .find("#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]")
        .expect("config end");
    fs::write(
        bin.join("config.rs"),
        format!("use super::*;\n{}", &library[start..end]),
    )?;
    fs::write(
        library_path,
        format!(
            "{}mod config;\nmod application;\npub use config::ProductConfig;\n{}",
            &library[..start],
            &library[end..]
        ),
    )?;
    let mcp = root.join("backend/crates/redemut-shaped-mcp/src");
    let source = fs::read_to_string(mcp.join("lib.rs"))?;
    let start = source.find("impl ItemTools {").expect("definitions start");
    let end = source
        .find("impl ToolService for ItemTools")
        .expect("definitions end");
    fs::write(
        mcp.join("definitions.rs"),
        format!("use super::*;\n{}", &source[start..end]),
    )?;
    fs::write(
        mcp.join("lib.rs"),
        format!("{}mod definitions;\n{}", &source[..start], &source[end..]),
    )?;
    doctor(&root)?;
    let mcp_manifest = root.join("backend/crates/redemut-shaped-mcp/Cargo.toml");
    let cargo = fs::read_to_string(&mcp_manifest)?;
    fs::write(
        mcp_manifest,
        cargo.replace(
            "name = \"redemut-shaped-mcp\"",
            "name = \"custom-tool-adapter\"",
        ),
    )?;
    doctor(&root)?;
    let manifest_path = root.join("baukit.toml");
    let manifest = fs::read_to_string(&manifest_path)?;
    fs::write(
        &manifest_path,
        format!(
            "{manifest}\n[doctor.sources]\nmcp_router = \"backend/crates/redemut-shaped-bin/src/application.rs\"\nmcp_config = \"backend/crates/redemut-shaped-bin/src/config.rs\"\nmcp_drift = \"backend/tests/tool_drift.rs\"\n"
        ),
    )?;
    doctor(&root)?;
    let manifest = fs::read_to_string(&manifest_path)?;
    fs::write(
        &manifest_path,
        manifest.replace(
            "mcp_config = \"backend/crates/redemut-shaped-bin/src/config.rs\"",
            "mcp_config = \"backend/crates/redemut-shaped-bin/src/lib.rs\"",
        ) + "mcp_tools = \"backend/crates/redemut-shaped-mcp/src/lib.rs\"\n",
    )?;
    doctor(&root)?;
    for (relative, symbol, finding) in [
        (
            "backend/crates/redemut-shaped-bin/src/application.rs",
            "baukit_mcp::router",
            "router mount",
        ),
        (
            "backend/crates/redemut-shaped-bin/src/application.rs",
            "api.merge(mcp)",
            "router merge",
        ),
        (
            "backend/crates/redemut-shaped-mcp/src/lib.rs",
            "impl ToolService for",
            "tool registration",
        ),
        (
            "backend/crates/redemut-shaped-mcp/src/definitions.rs",
            "required_scopes:",
            "scope enforcement",
        ),
        (
            "backend/crates/redemut-shaped-bin/src/config.rs",
            "pub mcp: baukit_mcp::McpConfig",
            "resource configuration",
        ),
        (
            "backend/crates/redemut-shaped-bin/src/config.rs",
            "self.mcp.validate()",
            "configuration validation",
        ),
        (
            "backend/tests/tool_drift.rs",
            "assert_eq!",
            "schema drift check",
        ),
    ] {
        let path = root.join(relative);
        let source = fs::read_to_string(&path)?;
        assert!(source.contains(symbol), "{relative}: {symbol}");
        let missing = source.replace(symbol, "missing_wiring");
        fs::write(&path, format!("{missing}\n// {symbol}\n"))?;
        let error = match doctor(&root) {
            Err(error) => error,
            Ok(findings) => panic!("missing {symbol} accepted: {findings:?}"),
        };
        assert!(error.to_string().contains(finding), "{symbol}: {error}");
        fs::write(path, source)?;
    }
    Ok(())
}

#[test]
fn doctor_accepts_mcp_composition_and_validation_across_backend_crates() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "graph-mcp");
    local.auth = Some(AuthProvider::Oidc);
    local.mcp = true;
    let root = generate_new(&local)?;
    let entry = root.join("backend/crates/graph-mcp-bin/src/bin/api.rs");
    let source = fs::read_to_string(&entry)?;
    assert!(source.contains("api.merge(mcp)"));
    fs::write(
        &entry,
        source.replace(
            "api.merge(mcp)",
            "graph_mcp_api::router_with_routes(api, mcp)",
        ),
    )?;
    let api = root.join("backend/crates/graph-mcp-api/src/lib.rs");
    let source = fs::read_to_string(&api)?;
    fs::write(
        &api,
        format!(
            "{source}\npub fn router_with_routes(api: axum::Router, routes: axum::Router) -> axum::Router {{ api.merge(routes) }}\npub fn validate_mcp(config: &baukit_mcp::McpConfig) -> Result<(), baukit_mcp::McpConfigError> {{ config.validate() }}\n"
        ),
    )?;
    let config = root.join("backend/crates/graph-mcp-bin/src/lib.rs");
    let source = fs::read_to_string(&config)?;
    assert!(source.contains("self.mcp.validate()"));
    fs::write(
        &config,
        source.replace(
            "self.mcp.validate()",
            "graph_mcp_api::validate_mcp(&self.mcp)",
        ),
    )?;
    let values = root.join("deploy/values.yaml");
    let mut deployment: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&fs::read_to_string(&values)?)?;
    deployment["mcp"]["enabled"] = false.into();
    deployment["mcp"]["resourceUrl"] = "".into();
    fs::write(&values, serde_yaml_ng::to_string(&deployment)?)?;
    assert!(!fs::read_to_string(&values)?.contains("/mcp"));
    doctor(&root)?;
    for (path, symbol, finding) in [
        (&api, "api.merge(routes)", "router merge"),
        (&api, "config.validate()", "configuration validation"),
        (
            &config,
            "graph_mcp_api::validate_mcp(&self.mcp)",
            "configuration validation",
        ),
    ] {
        let source = fs::read_to_string(path)?;
        assert!(source.contains(symbol));
        fs::write(path, source.replace(symbol, "missing_wiring"))?;
        assert!(
            doctor(&root)
                .expect_err("missing wiring")
                .to_string()
                .contains(finding)
        );
        fs::write(path, source)?;
    }
    deployment["mcp"]
        .as_mapping_mut()
        .expect("MCP values")
        .remove("allowedHosts");
    fs::write(values, serde_yaml_ng::to_string(&deployment)?)?;
    assert!(
        doctor(&root)
            .expect_err("missing MCP values")
            .to_string()
            .contains("allowedHosts:")
    );
    Ok(())
}

#[test]
fn doctor_follows_mcp_definitions_and_configuration_from_library_entry_points() -> anyhow::Result<()>
{
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "module-mcp");
    local.auth = Some(AuthProvider::Oidc);
    local.mcp = true;
    let root = generate_new(&local)?;
    let mcp = root.join("backend/crates/module-mcp-mcp/src");
    let library = fs::read_to_string(mcp.join("lib.rs"))?;
    let start = library.find("impl ItemTools {").expect("tool definitions");
    let end = library
        .find("impl ToolService for ItemTools")
        .expect("tool service");
    fs::write(
        mcp.join("definitions.rs"),
        format!("use super::*;\n{}", &library[start..end]),
    )?;
    fs::write(
        mcp.join("lib.rs"),
        format!(
            "{}mod definitions;\nmod config;\n{}",
            &library[..start],
            &library[end..]
        ),
    )?;
    fs::write(
        mcp.join("config.rs"),
        "pub struct RemoteConfig { pub resource: baukit_mcp::McpConfig }\nimpl RemoteConfig { pub fn validate(&self) -> Result<(), baukit_mcp::McpConfigError> { self.resource.validate() } }\n",
    )?;
    let manifest = root.join("baukit.toml");
    fs::write(
        &manifest,
        format!(
            "{}\n[doctor.sources]\nmcp_tools = \"backend/crates/module-mcp-mcp/src/lib.rs\"\nmcp_config = \"backend/crates/module-mcp-mcp/src/lib.rs\"\n",
            fs::read_to_string(&manifest)?
        ),
    )?;
    doctor(&root)?;
    for (file, symbol, finding) in [
        ("lib.rs", "mod definitions;", "scope enforcement"),
        ("definitions.rs", "required_scopes:", "scope enforcement"),
        ("lib.rs", "mod config;", "resource configuration"),
        (
            "config.rs",
            "resource: baukit_mcp::McpConfig",
            "resource configuration",
        ),
        (
            "config.rs",
            "self.resource.validate()",
            "configuration validation",
        ),
    ] {
        let path = mcp.join(file);
        let source = fs::read_to_string(&path)?;
        assert!(source.contains(symbol));
        fs::write(&path, source.replace(symbol, "missing_wiring"))?;
        assert!(
            doctor(&root)
                .expect_err("missing module wiring")
                .to_string()
                .contains(finding)
        );
        fs::write(path, source)?;
    }
    Ok(())
}

#[test]
fn remote_mcp_compose_keeps_public_issuers_and_fetches_internal_keys() -> anyhow::Result<()> {
    for provider in [
        AuthProvider::Oidc,
        AuthProvider::Clerk,
        AuthProvider::Workos,
    ] {
        let parent = tempfile::tempdir()?;
        let mut local = options(parent.path(), "container-auth");
        local.auth = Some(provider);
        local.mcp = true;
        let root = generate_new(&local)?;
        let compose: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(&fs::read_to_string(root.join("compose.yaml"))?)?;
        let backend = &compose["services"]["backend"];
        assert_eq!(backend["profiles"][0], "backend");
        let environment = &backend["environment"];
        assert_eq!(
            environment["CONTAINER_AUTH__MCP__ENABLED"],
            "${CONTAINER_AUTH__MCP__ENABLED:-false}"
        );
        if provider == AuthProvider::Oidc {
            assert_eq!(
                environment["CONTAINER_AUTH__MCP__JWKS_URI"],
                "${CONTAINER_AUTH__MCP__JWKS_URI:-http://keycloak:8080/realms/container-auth/protocol/openid-connect/certs}"
            );
            assert_eq!(
                environment["CONTAINER_AUTH__AUTH__JWKS_URI"],
                "${CONTAINER_AUTH__AUTH__JWKS_URI:-http://keycloak:8080/realms/container-auth/protocol/openid-connect/certs}"
            );
            assert_eq!(
                environment["CONTAINER_AUTH__MCP__ISSUER"],
                "${CONTAINER_AUTH__MCP__ISSUER:-http://localhost:8081/realms/container-auth}"
            );
        } else {
            assert!(environment["CONTAINER_AUTH__MCP__JWKS_URI"].is_null());
            assert!(compose["services"]["keycloak"].is_null());
        }
    }
    Ok(())
}

#[test]
fn doctor_requires_a_consumed_remote_mcp_dependency() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut local = options(parent.path(), "mcp-dependency");
    local.auth = Some(AuthProvider::Oidc);
    local.mcp = true;
    let root = generate_new(&local)?;
    for suffix in ["bin", "mcp", "api"] {
        let path = root.join(format!("backend/crates/mcp-dependency-{suffix}/Cargo.toml"));
        let source = fs::read_to_string(&path)?;
        assert!(source.contains("baukit-mcp.workspace = true"));
        fs::write(path, source.replace("baukit-mcp.workspace = true", ""))?;
    }
    assert!(
        doctor(&root)
            .expect_err("unused workspace dependency is not MCP wiring")
            .to_string()
            .contains("remote MCP auth layer")
    );
    Ok(())
}
