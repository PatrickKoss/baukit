use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use baukit_cli::{AuthProvider, NewOptions, QualityProfile, doctor, generate_new};
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
        auth: None,
        force: false,
        into_existing: false,
        resolve_lockfiles: false,
        baukit_path: None,
        port_offset: 0,
        quality: QualityProfile::Standard,
    }
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
fn generated_backend_is_rustfmt_clean_across_product_name_sort_positions() -> anyhow::Result<()> {
    for name in ["aaa", "zeta"] {
        let parent = tempfile::tempdir()?;
        let mut generated_options = options(parent.path(), name);
        generated_options.worker = true;
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
    assert!(public_config.contains("heading: /^qa-public$/u"));
    assert!(public_config.contains("fields: ["));
    assert!(public_config.contains("invalidField: 'Example name'"));
    assert!(public_config.contains("recoveryRole: 'button'"));
    assert!(public_config.contains("recoveryRole: 'link'"));
    assert!(public_config.contains("apiStubs: ITEM_API_STUBS"));
    assert!(!public_config.contains("authenticated: true"));
    assert!(!public_config.contains("qa-public:oidc:tokens"));

    let private_config = fs::read_to_string(authenticated.join("web/e2e/qa.config.ts"))?;
    assert!(private_config.contains("authenticated: true"));
    assert!(private_config.contains("qa-private:oidc:tokens"));
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
            "[quality]\nprofile = \"standard\"\nbackend_coverage_lines = 70\ncritical_paths = []\nwebkit_repeats = 3\nfull_stack_e2e = false\n\n",
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
        assert_eq!(workflow.contains("actions/setup-java@v4"), mobile);
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
            "http://localhost:8181/realms/offset-app",
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
    assert_eq!(api.matches("security((\"bearerAuth\" = []))").count(), 6);
    assert_eq!(api.matches("_principal: Principal").count(), 5);
    let openapi = fs::read_to_string(first.join("backend/openapi.json"))?;
    assert_eq!(openapi.matches("\"bearerAuth\": []").count(), 6);
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
    assert!(!first_tree.keys().any(|path| {
        path.starts_with("keycloak/themes")
            && path.extension().is_some_and(|extension| extension == "ftl")
    }));
    assert!(first.join("scripts/keycloak_policy.py").is_file());
    assert!(first.join("scripts/reconcile_keycloak.py").is_file());
    let compose = fs::read_to_string(first.join("compose.yaml"))?;
    assert!(compose.contains("keycloak-data:"));
    assert!(compose.contains("./keycloak/themes:/opt/keycloak/themes:ro"));
    assert!(compose.contains("KEYCLOAK_IMAGE:-quay.io/keycloak/keycloak:26.7.0"));
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
    assert!(workflow.contains("scripts/observability-lint.py"));
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
        "#!/bin/sh\nexit \"$BAUKIT_TEST_SSH_ADD_STATUS\"\n",
    )?;
    let agent_socket = parent.path().join("agent.sock");
    let _agent = UnixListener::bind(&agent_socket)?;
    let path = format!(
        "{}:{}",
        fake_bin.display(),
        env::var("PATH").unwrap_or_default()
    );
    for (status, expected) in [("1", "no loaded identities"), ("2", "agent is unusable")] {
        let result = Command::new("sh")
            .arg("scripts/preflight.sh")
            .current_dir(&root)
            .env("PATH", &path)
            .env("SSH_AUTH_SOCK", &agent_socket)
            .env("BAUKIT_TEST_SSH_ADD_STATUS", status)
            .output()?;
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains(expected));
    }
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
    printf '%s\n' "$PLAYWRIGHT_BROWSERS_PATH" >"$BAUKIT_TEST_INSTALL_LOG"
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
    let marker = parent.path().join("browser-installed");
    let install_log = parent.path().join("install-cache");
    let run_log = parent.path().join("run-cache");
    let path = format!(
        "{}:{}",
        fake_bin.display(),
        env::var("PATH").unwrap_or_default()
    );
    let result = Command::new("sh")
        .args(["scripts/preflight.sh", "--", "record-playwright-cache"])
        .current_dir(&root)
        .env("PATH", path)
        .env("BAUKIT_PREBUILT_IMAGES", "true")
        .env(
            "BAUKIT_TEST_PLAYWRIGHT_MODULE",
            root.join("web/node_modules/@playwright/test"),
        )
        .env("BAUKIT_TEST_PLAYWRIGHT_MARKER", marker)
        .env("BAUKIT_TEST_INSTALL_LOG", &install_log)
        .env("BAUKIT_TEST_RUN_LOG", &run_log)
        .output()?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let expected_cache = format!(
        "{}\n",
        root.join("web/node_modules/.cache/playwright-browsers")
            .display()
    );
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
        8
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
        let digest = Sha256::digest(contents);
        snapshot.push_str(&format!("{digest:x}  {}\n", path.display()));
    }
    snapshot
}
