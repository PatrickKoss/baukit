use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use baukit_cli::{AuthProvider, NewOptions, doctor, generate_new};
use sha2::{Digest, Sha256};

#[cfg(unix)]
use std::{
    env,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    process::Command,
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
    assert_eq!(manifest.capabilities.auth, None);
    assert!(!fs::read_to_string(first.join("baukit.toml"))?.contains("auth"));
    assert!(first.join("backend/Cargo.toml").is_file());
    assert!(first.join("mobile/App.tsx").is_file());
    assert!(first.join("web/src/App.tsx").is_file());
    Ok(())
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
    assert!(first.join("mobile/src/auth.ts").is_file());
    assert!(fs::read_to_string(first.join("mobile/package.json"))?.contains("@baukit/auth-native"));

    let api = fs::read_to_string(first.join("backend/crates/snapshot-app-api/src/lib.rs"))?;
    assert_eq!(api.matches("security((\"bearerAuth\" = []))").count(), 6);
    assert_eq!(api.matches("_principal: Principal").count(), 5);
    let openapi = fs::read_to_string(first.join("backend/openapi.json"))?;
    assert_eq!(openapi.matches("\"bearerAuth\": []").count(), 6);
    let realm = fs::read_to_string(first.join("keycloak/realm.json"))?;
    assert!(realm.contains("\"realmRoles\": [\"offline_access\"]"));
    assert!(realm.contains("snapshot-app-mobile"));
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
fn release_emission_uses_private_ssh_tag_and_reproducibility_files() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut combined = options(parent.path(), "release-product");
    combined.web = true;
    combined.mobile = true;
    let root = generate_new(&combined)?;

    let cargo = fs::read_to_string(root.join("backend/Cargo.toml"))?;
    assert!(cargo.contains("ssh://git@github.com/PatrickKoss/baukit.git"));
    assert!(cargo.contains(&format!(
        "tag = \"baukit-v{}\"",
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
    assert!(fs::read_to_string(root.join("web/pnpm-workspace.yaml"))?.contains("allowBuilds"));
    assert!(fs::read_to_string(root.join("mobile/pnpm-workspace.yaml"))?.contains("allowBuilds"));
    let makefile = fs::read_to_string(root.join("Makefile"))?;
    assert!(
        makefile.contains("cargo test --manifest-path $(BACKEND_MANIFEST) -- --include-ignored")
    );
    assert!(makefile.contains("check: preflight fmt lint test check-web check-mobile"));
    assert!(makefile.contains("test: preflight"));
    assert!(!makefile.contains("baukit generate openapi-client"));
    let client = fs::read_to_string(root.join("scripts/openapi-client.sh"))?;
    assert!(client.contains("schema=backend/openapi.json"));
    assert!(!client.contains("cargo run"));
    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml"))?;
    assert_eq!(workflow.matches("BAUKIT_DEPLOY_KEY").count(), 3);
    assert!(
        workflow.contains("cargo test --manifest-path backend/Cargo.toml -- --include-ignored")
    );
    assert!(workflow.contains("working-directory: web"));
    assert!(workflow.contains("working-directory: mobile"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn generated_preflight_fails_without_an_agent_and_supports_prebuilt_images() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path(), "preflight-app"))?;

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
