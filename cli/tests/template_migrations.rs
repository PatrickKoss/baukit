use std::{fs, path::Path};

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
        "0009_baukit_jobs_deferrals.sql",
        "0005_baukit_jobs_deferrals.sql",
    ),
    (
        "0010_baukit_jobs_concurrency_key.sql",
        "0006_baukit_jobs_concurrency_key.sql",
    ),
];

#[test]
fn template_jobs_migrations_match_the_crate_sql() -> anyhow::Result<()> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let crate_migrations = repository.join("rust/crates/baukit-jobs/migrations");
    for directory in TEMPLATE_MIGRATION_DIRS {
        for (template, source) in JOBS_MIGRATION_COPIES {
            let copy = fs::read_to_string(repository.join(directory).join(template))?;
            let original = fs::read_to_string(crate_migrations.join(source))?;
            assert_eq!(copy, original, "{directory}/{template} drifted from {source}");
        }
    }
    Ok(())
}
