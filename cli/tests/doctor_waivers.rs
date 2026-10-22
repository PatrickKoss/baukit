use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Result, ensure};
use baukit_cli::{
    AnalyticsAdapter, AuthProvider, DoctorWaiver, NewOptions, QualityProfile, doctor, generate_new,
    read_manifest,
};

const CONSOLE_SPEC: &str = "web/e2e/tests/qa-console.spec.ts";
const REASON: &str = "Every browser test uses the console fixture in e2e/tests/qa.ts";

fn options(parent: &Path) -> NewOptions {
    NewOptions {
        name: "waiver-product".to_owned(),
        directory: parent.to_owned(),
        backend: false,
        worker: false,
        mobile: false,
        web: true,
        pwa: false,
        mcp: false,
        platform: false,
        auth: None,
        force: false,
        into_existing: false,
        resolve_lockfiles: false,
        baukit_path: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rust")),
        port_offset: 0,
        quality: QualityProfile::Standard,
    }
}

fn waive(root: &Path, paths: &[&str]) -> Result<()> {
    let mut manifest = read_manifest(root)?;
    manifest.doctor.waive = paths
        .iter()
        .map(|path| DoctorWaiver {
            path: (*path).to_owned(),
            reason: REASON.to_owned(),
        })
        .collect();
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    Ok(())
}

#[test]
fn waived_browser_audits_print_notes_and_keep_unwaived_failures() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path()))?;
    let paths = [
        CONSOLE_SPEC,
        "web/e2e/tests/qa-submit-guards.spec.ts",
        "web/e2e/tests/qa-route-state.spec.ts",
        "web/e2e/tests/qa-geometry.spec.ts",
    ];
    for path in paths {
        fs::remove_file(root.join(path))?;
    }
    waive(&root, &paths)?;
    let output = Command::new(env!("CARGO_BIN_EXE_baukit"))
        .arg("doctor")
        .current_dir(&root)
        .output()?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    for path in paths {
        let note = format!("note: waived missing expected web file `{path}`: {REASON}");
        assert!(stdout.lines().any(|line| line == note), "{stdout}");
    }
    assert!(stdout.contains("doctor: product is healthy"));
    waive(&root, &paths[1..])?;
    let output = Command::new(env!("CARGO_BIN_EXE_baukit"))
        .arg("doctor")
        .current_dir(&root)
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains(&format!("- missing expected web file `{CONSOLE_SPEC}`")));
    for path in &paths[1..] {
        let note = format!("note: waived missing expected web file `{path}`: {REASON}");
        assert!(stderr.lines().any(|line| line == note), "{stderr}");
    }
    Ok(())
}

#[test]
fn waivers_cover_every_active_expected_file_group() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let mut combined = options(parent.path());
    combined.backend = true;
    combined.mobile = true;
    combined.auth = Some(AuthProvider::Oidc);
    combined.quality = QualityProfile::Strict;
    let root = generate_new(&combined)?;
    let cases = [
        ("CHANGELOG.md", "product"),
        ("scripts/quality-gate.sh", "strict quality"),
        (
            "scripts/check-migrations-immutable.test.sh",
            "strict backend",
        ),
        ("scripts/openapi-client.sh", "backend"),
        ("backend/Dockerfile", "backend"),
        ("backend/.dockerignore", "backend"),
        (
            "scripts/reconcile-env.test.py",
            "environment reconciliation",
        ),
        (
            "scripts/check-markdown-links.test.py",
            "Markdown link check",
        ),
        ("mobile/src/back-or-replace.test.ts", "mobile"),
        ("mobile/src/analytics.ts", "mobile"),
        (CONSOLE_SPEC, "web"),
        ("web/src/analytics.ts", "web"),
        ("web/src/auth.test.ts", "OIDC web"),
    ];
    doctor(&root)?;
    for (path, kind) in cases {
        let content = fs::read(root.join(path))?;
        fs::remove_file(root.join(path))?;
        let error = doctor(&root).expect_err("an unwaived expected file must fail");
        let finding = format!("missing expected {kind} file `{path}`");
        assert!(error.to_string().contains(&finding), "{error:#}");
        waive(&root, &[path])?;
        let notes = doctor(&root)?;
        assert!(notes.contains(&format!("note: waived {finding}: {REASON}")));
        fs::write(root.join(path), content)?;
        waive(&root, &[])?;
    }
    Ok(())
}

#[test]
fn waivers_require_exactly_path_and_reason() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path()))?;
    let manifest = fs::read_to_string(root.join("baukit.toml"))?;
    for (entry, expected) in [
        (
            format!("{{ path = '{CONSOLE_SPEC}' }}"),
            "missing field `reason`",
        ),
        (
            "{ reason = 'Covered elsewhere' }".to_owned(),
            "missing field `path`",
        ),
        (
            format!("{{ path = '{CONSOLE_SPEC}', reason = 'Covered elsewhere', extra = true }}"),
            "unknown field `extra`",
        ),
    ] {
        fs::write(
            root.join("baukit.toml"),
            format!("{manifest}\n[doctor]\nwaive = [{entry}]\n"),
        )?;
        let error = doctor(&root).expect_err("invalid waiver fields must fail");
        assert!(format!("{error:#}").contains(expected), "{error:#}");
    }
    Ok(())
}

#[test]
fn waivers_reject_empty_and_whitespace_only_reasons() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path()))?;
    fs::remove_file(root.join(CONSOLE_SPEC))?;
    for reason in ["", " \t\n "] {
        let mut manifest = read_manifest(&root)?;
        manifest.doctor.waive = vec![DoctorWaiver {
            path: CONSOLE_SPEC.to_owned(),
            reason: reason.to_owned(),
        }];
        fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
        let error = doctor(&root).expect_err("blank waiver reasons must fail");
        assert!(
            error.to_string().contains(&format!(
                "doctor waiver `{CONSOLE_SPEC}` requires a non-empty reason"
            )),
            "{error:#}"
        );
    }
    Ok(())
}

#[test]
fn waivers_reject_existing_files_and_directories_as_stale() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path()))?;
    waive(&root, &[CONSOLE_SPEC])?;
    for directory in [false, true] {
        if directory {
            fs::remove_file(root.join(CONSOLE_SPEC))?;
            fs::create_dir(root.join(CONSOLE_SPEC))?;
        }
        let error = doctor(&root).expect_err("an existing waived path must fail");
        assert!(
            error.to_string().contains(&format!(
                "stale doctor waiver `{CONSOLE_SPEC}`: path exists"
            )),
            "{error:#}"
        );
    }
    #[cfg(unix)]
    {
        fs::remove_dir(root.join(CONSOLE_SPEC))?;
        std::os::unix::fs::symlink("missing-target", root.join(CONSOLE_SPEC))?;
        let error = doctor(&root).expect_err("a dangling symlink is still a stale waiver");
        assert!(
            error.to_string().contains(&format!(
                "stale doctor waiver `{CONSOLE_SPEC}`: path exists"
            )),
            "{error:#}"
        );
    }
    Ok(())
}

#[test]
fn waivers_reject_unknown_and_inactive_expected_paths() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path()))?;
    let mut manifest = read_manifest(&root)?;
    manifest.capabilities.analytics = AnalyticsAdapter::None;
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    for path in [
        "web/e2e/tests/product-only.spec.ts",
        "scripts/quality-gate.sh",
        "scripts/check-migrations-immutable.test.sh",
        "scripts/check-markdown-links.py",
        "mobile/src/back-or-replace.test.ts",
        "web/src/auth.test.ts",
        "compose.yaml",
        "web/src/analytics.ts",
    ] {
        waive(&root, &[path])?;
        let error = doctor(&root).expect_err("only active expected paths may be waived");
        assert!(
            error
                .to_string()
                .contains(&format!("unknown doctor waiver path `{path}`")),
            "{error:#}"
        );
    }
    Ok(())
}

#[test]
fn waivers_normalize_relative_paths_and_reject_normalized_duplicates() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path()))?;
    fs::remove_file(root.join(CONSOLE_SPEC))?;
    let alternate = ".\\web//e2e/./tests/qa-console.spec.ts";
    waive(&root, &[alternate])?;
    assert!(doctor(&root)?.contains(&format!(
        "note: waived missing expected web file `{CONSOLE_SPEC}`: {REASON}"
    )));
    for duplicate in [CONSOLE_SPEC, alternate] {
        waive(&root, &[CONSOLE_SPEC, duplicate])?;
        let error = doctor(&root).expect_err("duplicate waiver paths must fail");
        assert!(
            error
                .to_string()
                .contains(&format!("duplicate doctor waiver path `{CONSOLE_SPEC}`")),
            "{error:#}"
        );
    }
    Ok(())
}

#[test]
fn waivers_reject_absolute_parent_and_empty_paths() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path()))?;
    for path in [
        "",
        ".",
        "/web/e2e/tests/qa-console.spec.ts",
        "C:\\web\\e2e\\tests\\qa-console.spec.ts",
        "\\\\server\\share\\qa-console.spec.ts",
        "../web/e2e/tests/qa-console.spec.ts",
        "web/e2e/../e2e/tests/qa-console.spec.ts",
        "web\\e2e\\..\\e2e\\tests\\qa-console.spec.ts",
    ] {
        waive(&root, &[path])?;
        let error = doctor(&root).expect_err("invalid waiver paths must fail");
        assert!(
            error
                .to_string()
                .contains("must be relative to the product root without `..`"),
            "{error:#}"
        );
    }
    Ok(())
}

#[test]
fn waivers_do_not_silence_other_checks_on_the_same_path() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let root = generate_new(&options(parent.path()))?;
    fs::remove_file(root.join(CONSOLE_SPEC))?;
    waive(&root, &[CONSOLE_SPEC])?;
    let mut manifest = read_manifest(&root)?;
    manifest.quality.critical_paths = vec!["e2e/tests/qa-console.spec.ts".to_owned()];
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    let error = doctor(&root).expect_err("critical-path checks must still fail");
    assert!(
        error
            .to_string()
            .contains(&format!("critical path `{CONSOLE_SPEC}` does not exist")),
        "{error:#}"
    );
    assert!(
        !error
            .to_string()
            .lines()
            .any(|line| line.starts_with("- missing expected web file"))
    );
    assert!(error.to_string().contains(&format!(
        "note: waived missing expected web file `{CONSOLE_SPEC}`: {REASON}"
    )));
    Ok(())
}

#[test]
fn waivers_follow_declared_backend_file_paths() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let mut backend = options(parent.path());
    backend.backend = true;
    let root = generate_new(&backend)?;
    fs::rename(root.join("backend"), root.join("server"))?;
    let mut manifest = read_manifest(&root)?;
    manifest.doctor.backend_manifest = Some("server/Cargo.toml".to_owned());
    manifest.doctor.migrations = Some("server/migrations".to_owned());
    manifest.openapi.schema = "server/openapi.json".to_owned();
    fs::write(root.join("baukit.toml"), toml::to_string(&manifest)?)?;
    doctor(&root)?;
    fs::remove_file(root.join("server/Dockerfile"))?;
    waive(&root, &["server/Dockerfile"])?;
    assert!(doctor(&root)?.contains(&format!(
        "note: waived missing expected backend file `server/Dockerfile`: {REASON}"
    )));
    waive(&root, &["backend/Dockerfile"])?;
    let error = doctor(&root).expect_err("the former default path is inactive");
    assert!(
        error
            .to_string()
            .contains("unknown doctor waiver path `backend/Dockerfile`"),
        "{error:#}"
    );
    Ok(())
}
