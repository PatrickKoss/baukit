use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use baukit_cli::{AuthProvider, NewOptions, QualityProfile, doctor, generate_new};

fn options(directory: &Path) -> NewOptions {
    NewOptions {
        name: "dev-ports-product".to_owned(),
        directory: directory.to_owned(),
        backend: true,
        worker: false,
        mobile: true,
        web: true,
        pwa: false,
        mcp: true,
        auth: Some(AuthProvider::Oidc),
        force: true,
        into_existing: true,
        resolve_lockfiles: false,
        baukit_path: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust")),
        port_offset: 0,
        quality: QualityProfile::Standard,
    }
}

const RENDERED_FILES: &[&str] = &[
    "web/vite.config.ts",
    "web/e2e/playwright.config.ts",
    "web/e2e/playwright.stack.config.ts",
    "web/README.md",
    "mobile/package.json",
    "mobile/README.md",
    "keycloak/realm.json",
    "keycloak/reconcile.json",
    "backend/crates/dev-ports-product-bin/src/lib.rs",
    "scripts/pkce-login.py",
    "scripts/test-keycloak-theme-patches.sh",
];

fn declared_product(root: &Path, declarations: &str) -> anyhow::Result<()> {
    let options = options(root);
    generate_new(&options)?;
    let manifest = root.join("baukit.toml");
    let source = fs::read_to_string(&manifest)? + "\n[ports]\n" + declarations;
    fs::write(&manifest, &source)?;
    for relative in RENDERED_FILES {
        fs::remove_file(root.join(relative))?;
    }
    generate_new(&options)?;
    assert_eq!(fs::read_to_string(&manifest)?, source);
    Ok(())
}

#[test]
fn declared_dev_ports_render_all_consumers_and_pass_doctor() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = parent.path();
    declared_product(
        root,
        "web = { host = 5673 }\nweb_preview = { host = 4773 }\nmetro = { host = 8093 }\n",
    )?;
    for (relative, snippet) in [
        ("web/vite.config.ts", "server: {\n    port: 5673"),
        ("web/vite.config.ts", "preview: {\n    port: 4773"),
        ("web/e2e/playwright.config.ts", "?? '4773'"),
        ("web/e2e/playwright.stack.config.ts", "?? '5673'"),
        ("keycloak/realm.json", "http://localhost:5673/*"),
        ("keycloak/realm.json", "http://localhost:5673\""),
        ("keycloak/reconcile.json", "http://localhost:5673/*"),
        ("keycloak/reconcile.json", "http://localhost:5673\""),
        (
            "backend/crates/dev-ports-product-bin/src/lib.rs",
            "authorized_parties: vec![\"http://localhost:5673\"",
        ),
        (
            "scripts/pkce-login.py",
            "default=\"http://localhost:5673/\"",
        ),
        (
            "scripts/test-keycloak-theme-patches.sh",
            "http://localhost:5673/",
        ),
        ("web/README.md", "defaults to 5673"),
    ] {
        assert!(
            fs::read_to_string(root.join(relative))?.contains(snippet),
            "{relative}: {snippet}"
        );
    }
    let package: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("mobile/package.json"))?)?;
    for script in ["start", "android", "ios"] {
        assert!(
            package["scripts"][script]
                .as_str()
                .expect("script")
                .ends_with("--port 8093")
        );
    }
    let results = doctor(root)?;
    for name in ["web", "web_preview", "metro"] {
        assert!(
            results
                .iter()
                .any(|result| result.contains(&format!("declared ports.{name} host")))
        );
    }
    Ok(())
}

#[test]
fn doctor_rejects_drift_in_every_web_port_consumer() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = parent.path();
    declared_product(
        root,
        "web = { host = 5673 }\nweb_preview = { host = 4773 }\n",
    )?;
    for (relative, correct, wrong, declaration) in [
        ("web/vite.config.ts", "port: 5673", "port: 5674", "web"),
        (
            "web/e2e/playwright.stack.config.ts",
            "'5673'",
            "'5674'",
            "web",
        ),
        (
            "web/e2e/playwright.config.ts",
            "'4773'",
            "'4774'",
            "web_preview",
        ),
        (
            "keycloak/realm.json",
            "localhost:5673/*",
            "localhost:5674/*",
            "web",
        ),
        (
            "keycloak/realm.json",
            "localhost:5673\"",
            "localhost:5674\"",
            "web",
        ),
        (
            "keycloak/reconcile.json",
            "localhost:5673/*",
            "localhost:5674/*",
            "web",
        ),
        (
            "keycloak/reconcile.json",
            "localhost:5673\"",
            "localhost:5674\"",
            "web",
        ),
        (
            "backend/crates/dev-ports-product-bin/src/lib.rs",
            "localhost:5673",
            "localhost:5674",
            "web",
        ),
        (
            "scripts/pkce-login.py",
            "localhost:5673",
            "localhost:5674",
            "web",
        ),
    ] {
        let path = root.join(relative);
        let source = fs::read_to_string(&path)?;
        assert!(source.contains(correct));
        fs::write(&path, source.replace(correct, wrong))?;
        let error = doctor(root)
            .expect_err("doctor must reject stale port")
            .to_string();
        assert!(
            error.contains(relative) && error.contains(&format!("ports.{declaration}")),
            "{error}"
        );
        if relative.ends_with("-bin/src/lib.rs") {
            assert_eq!(
                error.matches("does not match declared ports.web").count(),
                1,
                "{error}"
            );
        }
        assert!(
            error.contains(if declaration == "web" {
                "found port 5674"
            } else {
                "found port 4774"
            }),
            "{error}"
        );
        fs::write(path, source)?;
    }
    Ok(())
}

#[test]
fn doctor_checks_each_expo_command_including_compound_scripts() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = parent.path();
    declared_product(root, "metro = { host = 8093 }\n")?;
    let path = root.join("mobile/package.json");
    let mut package: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
    for command in [
        "expo start",
        "expo run:android",
        "expo run:ios",
        "expo start --web",
    ] {
        for suffix in [
            "",
            " --port nonsense",
            " --port 8093wrong",
            " --port 8081",
            " --port 8093 --port 8081",
            " --port 8093 && expo start",
            " --port 8093 && expo start --port 8081",
        ] {
            package["scripts"]["port-test"] = format!("{command}{suffix}").into();
            fs::write(&path, serde_json::to_string_pretty(&package)?)?;
            let error = doctor(root)
                .expect_err("every Expo command needs its declared port")
                .to_string();
            assert!(
                error.contains("mobile/package.json scripts.port-test")
                    && error.contains("ports.metro"),
                "{error}"
            );
        }
        package["scripts"]["port-test"] = format!("npx {command} --port=8093").into();
        fs::write(&path, serde_json::to_string_pretty(&package)?)?;
        doctor(root)?;
    }
    Ok(())
}

#[test]
fn doctor_checks_metro_forwarding_and_environment_in_scripts_and_makefile() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = parent.path();
    declared_product(root, "metro = { host = 8093 }\n")?;
    fs::create_dir_all(root.join("scripts/nested"))?;
    for relative in [
        "scripts/nested/metro.sh",
        "mobile/scripts/metro.sh",
        "Makefile",
    ] {
        let path = root.join(relative);
        let original = if path.is_file() {
            fs::read_to_string(&path)?
        } else {
            String::new()
        };
        for command in [
            "adb reverse tcp:8093 tcp:8093",
            "RCT_METRO_PORT=8093 expo start",
        ] {
            fs::write(&path, format!("{original}\n{command}\n"))?;
            doctor(root)?;
            fs::write(
                &path,
                format!("{original}\n{}\n", command.replace("8093", "8081")),
            )?;
            let error = doctor(root)
                .expect_err("stale Metro script port")
                .to_string();
            assert!(
                error.contains(relative) && error.contains("found port 8081"),
                "{error}"
            );
        }
        fs::write(
            &path,
            format!("{original}\nadb reverse tcp:8093 tcp:8093 && adb reverse tcp:8081 tcp:8081\n"),
        )?;
        let error = doctor(root)
            .expect_err("each adb reverse command must agree")
            .to_string();
        assert!(
            error.contains(relative) && error.contains("found port 8081"),
            "{error}"
        );
        fs::write(path, original)?;
    }
    Ok(())
}

#[test]
fn undeclared_dev_ports_are_unchecked_even_with_a_service_offset() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let mut options = options(parent.path());
    options.port_offset = 100;
    let root = generate_new(&options)?;
    for (relative, old, new) in [
        ("web/vite.config.ts", "5173", "5674"),
        ("web/e2e/playwright.config.ts", "4173", "4774"),
        ("keycloak/realm.json", "5173", "5674"),
        ("scripts/pkce-login.py", "5173", "5674"),
        (
            "mobile/package.json",
            "expo start",
            "expo start --port 8081",
        ),
    ] {
        let path = root.join(relative);
        fs::write(&path, fs::read_to_string(&path)?.replace(old, new))?;
    }
    fs::write(
        root.join("scripts/metro.sh"),
        "adb reverse tcp:8081 tcp:8081\nRCT_METRO_PORT=8081\n",
    )?;
    let results = doctor(&root)?;
    assert!(
        !results
            .iter()
            .any(|result| result.contains("declared ports."))
    );
    Ok(())
}

#[test]
fn dev_ports_reject_container_service_zero_and_collisions() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = parent.path();
    generate_new(&options(root))?;
    let path = root.join("baukit.toml");
    let original = fs::read_to_string(&path)?;
    for name in ["web", "web_preview", "metro"] {
        for (field, message) in [
            ("host = 8093, container = 8080", "accepts only `host`"),
            ("host = 8093, service = \"frontend\"", "accepts only `host`"),
            ("host = 0", "host must be nonzero"),
        ] {
            fs::write(
                &path,
                format!("{original}\n[ports]\n{name} = {{ {field} }}\n"),
            )?;
            let error = doctor(root)
                .expect_err("invalid dev port declaration")
                .to_string();
            assert!(
                error.contains(&format!("ports.{name}")) && error.contains(message),
                "{error}"
            );
            assert!(generate_new(&options(root)).is_err());
        }
    }
    for (declarations, names, port) in [
        ("metro = { host = 8081 }", "ports.keycloak", "8081"),
        (
            "metro = { host = 8091 }\nkeycloak = { host = 8091, container = 8080 }",
            "ports.keycloak",
            "8091",
        ),
        ("metro = { host = 8080 }", "ports.api", "8080"),
        ("metro = { host = 9090 }", "ports.ops", "9090"),
        ("metro = { host = 5432 }", "ports.postgres", "5432"),
        ("metro = { host = 6379 }", "ports.redis", "6379"),
        ("metro = { host = 18081 }", "ports.fake_provider", "18081"),
        (
            "metro = { host = 5673 }\nweb = { host = 5673 }",
            "ports.web",
            "5673",
        ),
        (
            "metro = { host = 4773 }\nweb_preview = { host = 4773 }",
            "ports.web_preview",
            "4773",
        ),
        (
            "api = { host = 5432, container = 8080 }",
            "ports.postgres",
            "5432",
        ),
    ] {
        fs::write(&path, format!("{original}\n[ports]\n{declarations}\n"))?;
        let error = doctor(root)
            .expect_err("effective hosts must be unique")
            .to_string();
        assert!(
            error.contains(names) && error.contains(&format!("effective host port {port}")),
            "{error}"
        );
    }
    fs::write(
        &path,
        format!("port_offset = 100\n{original}\n[ports]\nmetro = {{ host = 8181 }}\n"),
    )?;
    let error = doctor(root)
        .expect_err("offset-derived Keycloak collision")
        .to_string();
    assert!(
        error.contains("ports.keycloak") && error.contains("effective host port 8181"),
        "{error}"
    );
    Ok(())
}

#[test]
fn new_and_existing_renders_without_declarations_are_identical() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = parent.path();
    let options = options(root);
    generate_new(&options)?;
    let tree = read_tree(root)?;
    generate_new(&options)?;
    assert_eq!(read_tree(root)?, tree);
    assert!(fs::read_to_string(root.join("web/vite.config.ts"))?.contains("port: 5173"));
    assert!(!fs::read_to_string(root.join("mobile/package.json"))?.contains("--port"));
    Ok(())
}

fn read_tree(root: &Path) -> anyhow::Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn visit(
        root: &Path,
        directory: &Path,
        tree: &mut BTreeMap<PathBuf, Vec<u8>>,
    ) -> anyhow::Result<()> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                visit(root, &path, tree)?;
            } else {
                tree.insert(path.strip_prefix(root)?.to_owned(), fs::read(path)?);
            }
        }
        Ok(())
    }
    let mut tree = BTreeMap::new();
    visit(root, root, &mut tree)?;
    Ok(tree)
}

#[test]
fn missing_dev_port_files_do_not_produce_port_findings() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = parent.path();
    declared_product(
        root,
        "web = { host = 5673 }\nweb_preview = { host = 4773 }\nmetro = { host = 8093 }\n",
    )?;
    for relative in RENDERED_FILES {
        fs::remove_file(root.join(relative))?;
    }
    let error = doctor(root)
        .expect_err("separate expected-file checks still apply")
        .to_string();
    assert!(error.contains("missing"), "{error}");
    assert!(!error.contains("declared ports."), "{error}");
    Ok(())
}

#[test]
fn dev_port_declarations_do_not_shift_with_manifest_offset() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = parent.path();
    let mut options = options(root);
    options.port_offset = 100;
    generate_new(&options)?;
    let path = root.join("baukit.toml");
    fs::write(
        &path,
        fs::read_to_string(&path)?
            + "\n[ports]\nweb = { host = 5673 }\nweb_preview = { host = 4773 }\nmetro = { host = 8093 }\n",
    )?;
    for relative in RENDERED_FILES {
        fs::remove_file(root.join(relative))?;
    }
    options.port_offset = 0;
    generate_new(&options)?;
    doctor(root)?;
    assert!(fs::read_to_string(root.join("scripts/pkce-login.py"))?.contains("localhost:8180/me"));
    assert!(fs::read_to_string(root.join("web/vite.config.ts"))?.contains("port: 5673"));
    assert!(fs::read_to_string(root.join("mobile/package.json"))?.contains("--port 8093"));
    Ok(())
}

#[test]
fn metro_renders_in_every_mobile_auth_flavor() -> anyhow::Result<()> {
    for auth in [
        None,
        Some(AuthProvider::Oidc),
        Some(AuthProvider::Clerk),
        Some(AuthProvider::Workos),
    ] {
        let parent = tempfile::tempdir()?;
        let root = parent.path();
        let mut options = options(root);
        options.backend = false;
        options.mcp = false;
        options.web = false;
        options.auth = auth;
        options.pwa = true;
        generate_new(&options)?;
        let path = root.join("baukit.toml");
        fs::write(
            &path,
            fs::read_to_string(&path)? + "\n[ports]\nmetro = { host = 8093 }\n",
        )?;
        fs::remove_file(root.join("mobile/package.json"))?;
        fs::remove_file(root.join("mobile/README.md"))?;
        generate_new(&options)?;
        let source = fs::read_to_string(root.join("mobile/package.json"))?;
        for command in [
            "expo start --port 8093",
            "expo run:android --port 8093",
            "expo run:ios --port 8093",
        ] {
            assert!(source.contains(command), "{auth:?}: {command}");
        }
        doctor(root)?;
    }
    Ok(())
}

#[test]
fn re_render_preserves_product_changes_and_reports_port_conflicts() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = parent.path();
    let options = options(root);
    generate_new(&options)?;
    let path = root.join("baukit.toml");
    let manifest = fs::read_to_string(&path)? + "\n[ports]\nweb = { host = 5673 }\n";
    fs::write(&path, &manifest)?;
    let vite = root.join("web/vite.config.ts");
    let source = fs::read_to_string(&vite)? + "// product-owned setting\n";
    fs::write(&vite, &source)?;
    let error = generate_new(&options).expect_err("modified files must remain conflicts");
    assert!(error.to_string().contains("conflict"));
    assert_eq!(fs::read_to_string(vite)?, source);
    assert_eq!(fs::read_to_string(path)?, manifest);
    assert!(fs::read_to_string(root.join("baukit-conflicts.txt"))?.contains("web/vite.config.ts"));
    Ok(())
}

#[test]
fn declaring_one_dev_port_leaves_other_dev_ports_unchecked() -> anyhow::Result<()> {
    for declaration in [
        "web = { host = 5673 }",
        "web_preview = { host = 4773 }",
        "metro = { host = 8093 }",
    ] {
        let parent = tempfile::tempdir()?;
        let root = parent.path();
        declared_product(root, declaration)?;
        for (relative, old, new) in [
            ("web/vite.config.ts", "5173", "5674"),
            ("web/e2e/playwright.config.ts", "4173", "4774"),
            (
                "mobile/package.json",
                "expo start\"",
                "expo start --port 8081\"",
            ),
        ] {
            let path = root.join(relative);
            fs::write(&path, fs::read_to_string(&path)?.replace(old, new))?;
        }
        doctor(root)?;
    }
    Ok(())
}

#[test]
fn seeded_manifest_renders_dev_ports_and_keycloak_without_backend_offset() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let source = parent.path().join("source");
    generate_new(&options(&source))?;
    let manifest = fs::read_to_string(source.join("baukit.toml"))?
        + "\n[ports]\nweb = { host = 5673 }\nweb_preview = { host = 4773 }\nmetro = { host = 8093 }\nkeycloak = { host = 8091, container = 8080 }\n";
    let root = parent.path().join("rendered");
    fs::create_dir(&root)?;
    fs::write(root.join("baukit.toml"), &manifest)?;
    let options = options(&root);
    generate_new(&options)?;
    doctor(&root)?;
    assert_eq!(fs::read_to_string(root.join("baukit.toml"))?, manifest);
    assert!(fs::read_to_string(root.join("compose.yaml"))?.contains("8091:8080"));
    assert!(fs::read_to_string(root.join("web/e2e/stack/keycloak.ts"))?.contains("localhost:8091"));
    assert!(
        fs::read_to_string(root.join("mobile/package.json"))?.contains("expo start --port 8093")
    );
    Ok(())
}

#[test]
fn external_oidc_still_checks_declared_backend_web_origins() -> anyhow::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = parent.path();
    declared_product(root, "web = { host = 5673 }\n")?;
    fs::write(
        root.join("config/local.toml"),
        "[auth]\nissuer = \"https://identity.example.com/realms/dev-ports-product\"\n",
    )?;
    doctor(root)?;
    let path = root.join("backend/crates/dev-ports-product-bin/src/lib.rs");
    fs::write(
        &path,
        fs::read_to_string(&path)?.replace("localhost:5673", "localhost:5674"),
    )?;
    let error = doctor(root)
        .expect_err("external OIDC must still use the declared web origin")
        .to_string();
    assert!(
        error.contains("-bin/src/lib.rs") && error.contains("found port 5674"),
        "{error}"
    );
    Ok(())
}
