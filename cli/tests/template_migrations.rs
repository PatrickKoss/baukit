use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};

const CRATE_MIGRATIONS: &str = "../rust/crates/baukit-auth/migrations";
const TEMPLATE_MIGRATIONS: &str = "../templates/backend/__auth__/backend/migrations";
const COPIED_PREFIX: &str = "baukit_auth_browser_session";

fn repository_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn statements(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn sql_files(directory: &Path) -> Result<Vec<(String, String)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).with_context(|| directory.display().to_string())? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "sql") {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .context("migration name is not UTF-8")?
                .to_owned();
            files.push((name, statements(&fs::read_to_string(&path)?)));
        }
    }
    Ok(files)
}

fn copied_name(crate_name: &str) -> Option<&str> {
    crate_name
        .split_once('_')
        .map(|(_, name)| name)
        .filter(|name| name.starts_with(COPIED_PREFIX))
}

const TEMPLATE_MIGRATION_DIRS: &[&str] = &[
    "templates/worker/backend/migrations",
    "templates/backend/__auth__/backend/migrations",
];

const JOBS_MIGRATION_COPIES: &[(&str, &str)] = &[
    (
        "0007_baukit_jobs_active_deduplication.sql",
        "0004_baukit_jobs_active_deduplication.sql",
    ),
    (
        "0012_baukit_jobs_deferrals.sql",
        "0005_baukit_jobs_deferrals.sql",
    ),
    (
        "0013_baukit_jobs_concurrency_key.sql",
        "0006_baukit_jobs_concurrency_key.sql",
    ),
    (
        "0014_baukit_jobs_validate_checks.sql",
        "0007_baukit_jobs_validate_checks.sql",
    ),
    (
        "0015_baukit_jobs_running_key_index.sql",
        "0008_baukit_jobs_running_key_index.sql",
    ),
];

#[test]
fn template_jobs_migrations_match_the_crate_sql() -> Result<()> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let crate_migrations = repository.join("rust/crates/baukit-jobs/migrations");
    for directory in TEMPLATE_MIGRATION_DIRS {
        for (template, source) in JOBS_MIGRATION_COPIES {
            let copy = fs::read_to_string(repository.join(directory).join(template))?;
            let original = fs::read_to_string(crate_migrations.join(source))?;
            assert_eq!(
                copy, original,
                "{directory}/{template} drifted from {source}"
            );
        }
    }
    Ok(())
}

#[test]
fn auth_template_copies_every_browser_session_migration() -> Result<()> {
    let template = sql_files(&repository_path(TEMPLATE_MIGRATIONS))?;
    let crate_files = sql_files(&repository_path(CRATE_MIGRATIONS))?;
    let mut checked = 0;
    for (crate_name, crate_sql) in &crate_files {
        let Some(name) = copied_name(crate_name) else {
            continue;
        };
        let copy = template
            .iter()
            .find(|(template_name, _)| copied_name(template_name) == Some(name));
        let (template_name, template_sql) =
            copy.with_context(|| format!("the auth template has no copy of {crate_name}"))?;
        ensure!(
            template_sql == crate_sql,
            "{template_name} differs from baukit-auth {crate_name}"
        );
        checked += 1;
    }
    ensure!(
        checked > 0,
        "no browser session migrations found in baukit-auth"
    );
    Ok(())
}
