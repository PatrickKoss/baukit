use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use anyhow::{Context, Result};

use crate::{
    AnalyticsAdapter, AuthProvider, ENV_RECONCILIATION_FILES, EXPECTED_AUTH_WEB_FILES,
    EXPECTED_BACKEND_FILES, EXPECTED_COMMON_FILES, EXPECTED_MOBILE_FILES,
    EXPECTED_STRICT_BACKEND_FILES, EXPECTED_STRICT_FILES, EXPECTED_WEB_FILES,
    MARKDOWN_LINK_CHECK_FILES, Manifest, QualityProfile, doctor_layout,
};

type ExpectedFiles = BTreeMap<String, &'static str>;

fn add_files(expected: &mut ExpectedFiles, kind: &'static str, paths: &[&str]) {
    expected.extend(paths.iter().map(|path| ((*path).to_owned(), kind)));
}

fn expected_files(root: &Path, manifest: &Manifest) -> Result<ExpectedFiles> {
    let mut expected = ExpectedFiles::new();
    add_files(&mut expected, "product", EXPECTED_COMMON_FILES);
    add_files(
        &mut expected,
        "environment reconciliation",
        ENV_RECONCILIATION_FILES,
    );
    if manifest.quality.profile == QualityProfile::Strict {
        add_files(&mut expected, "strict quality", EXPECTED_STRICT_FILES);
        add_files(
            &mut expected,
            "Markdown link check",
            MARKDOWN_LINK_CHECK_FILES,
        );
        if manifest.capabilities.backend {
            add_files(
                &mut expected,
                "strict backend",
                EXPECTED_STRICT_BACKEND_FILES,
            );
        }
    }
    if manifest.capabilities.backend {
        for path in EXPECTED_BACKEND_FILES {
            expected.insert(path.replace("__APP__", &manifest.app.name), "backend");
        }
        let cargo = doctor_layout::backend_manifest(root, manifest)?;
        let directory = cargo.parent().context("backend manifest has no parent")?;
        for (declared, name) in [
            (manifest.doctor.backend_dockerfile.as_deref(), "Dockerfile"),
            (
                manifest.doctor.backend_dockerignore.as_deref(),
                ".dockerignore",
            ),
        ] {
            let path = match declared {
                Some(relative) => doctor_layout::product_path(root, relative)?,
                None => directory.join(name),
            };
            let relative = normalize_path(&path.strip_prefix(root)?.to_string_lossy())
                .context("invalid expected doctor path")?;
            expected.insert(relative, "backend");
        }
    }
    for (active, kind, files) in [
        (
            manifest.capabilities.mobile,
            "mobile",
            EXPECTED_MOBILE_FILES,
        ),
        (manifest.capabilities.web, "web", EXPECTED_WEB_FILES),
    ] {
        if !active {
            continue;
        }
        add_files(&mut expected, kind, files);
        if manifest.capabilities.analytics != AnalyticsAdapter::None {
            expected.insert(format!("{kind}/src/analytics.ts"), kind);
        }
    }
    if manifest.capabilities.web && manifest.capabilities.auth == Some(AuthProvider::Oidc) {
        add_files(&mut expected, "OIDC web", EXPECTED_AUTH_WEB_FILES);
    }
    Ok(expected)
}

fn normalize_path(relative: &str) -> Option<String> {
    let path = relative.replace('\\', "/");
    if path.starts_with('/')
        || path.as_bytes().get(1) == Some(&b':')
        || path.split('/').any(|part| part == "..")
    {
        return None;
    }
    let normalized = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/");
    (!normalized.is_empty()).then_some(normalized)
}

fn validated_waivers<'a>(
    root: &Path,
    manifest: &'a Manifest,
    expected: &ExpectedFiles,
    failures: &mut Vec<String>,
) -> BTreeMap<String, &'a str> {
    let mut seen = BTreeSet::new();
    let mut waivers = BTreeMap::new();
    for waiver in &manifest.doctor.waive {
        let Some(path) = normalize_path(&waiver.path) else {
            failures.push(format!(
                "doctor waiver path `{}` must be relative to the product root without `..`",
                waiver.path
            ));
            continue;
        };
        if !seen.insert(path.clone()) {
            failures.push(format!("duplicate doctor waiver path `{path}`"));
            continue;
        }
        if waiver.reason.trim().is_empty() {
            failures.push(format!(
                "doctor waiver `{path}` requires a non-empty reason"
            ));
            continue;
        }
        if !expected.contains_key(&path) {
            failures.push(format!(
                "unknown doctor waiver path `{path}` is not an expected file for this product"
            ));
            continue;
        }
        if root.join(&path).symlink_metadata().is_ok() {
            failures.push(format!("stale doctor waiver `{path}`: path exists"));
            continue;
        }
        waivers.insert(path, waiver.reason.trim());
    }
    waivers
}

pub(super) fn validate(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let expected = expected_files(root, manifest)?;
    let waivers = validated_waivers(root, manifest, &expected, failures);
    for (path, kind) in expected {
        if root.join(&path).is_file() {
            continue;
        }
        let finding = format!("missing expected {kind} file `{path}`");
        if let Some(reason) = waivers.get(&path) {
            successes.push(format!("note: waived {finding}: {reason}"));
        } else {
            failures.push(finding);
        }
    }
    if ENV_RECONCILIATION_FILES
        .iter()
        .all(|path| root.join(path).is_file())
    {
        successes.push("environment reconciliation scripts are present".to_owned());
    }
    if manifest.quality.profile == QualityProfile::Strict
        && MARKDOWN_LINK_CHECK_FILES
            .iter()
            .all(|path| root.join(path).is_file())
    {
        successes.push("strict Markdown link check scripts are present".to_owned());
    }
    Ok(())
}
