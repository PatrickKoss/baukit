use std::{fs, path::Path};

use anyhow::Result;

use crate::{Manifest, doctor_layout};

pub(super) fn validate(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let Some(platform) = &manifest.platform else {
        return Ok(());
    };
    let migration = doctor_layout::product_path(root, &platform.migration)?;
    if migration.parent() != Some(doctor_layout::migrations(root, manifest)?.as_path())
        || migration
            .extension()
            .is_none_or(|extension| extension != "sql")
    {
        failures.push(
            "platform.migration must be a SQL file in the declared migrations directory".to_owned(),
        );
    }
    let migration_sql = fs::read_to_string(&migration).unwrap_or_default();
    for table in ["platform_inbound_receipts", "platform_entitlement_cache"] {
        if !super::sql_creates_table(&migration_sql, table) {
            failures.push(format!(
                "platform migration {} must create {table}",
                platform.migration
            ));
        }
    }
    let mut source = String::new();
    let mut dependencies = String::new();
    for krate in doctor_layout::backend_crates(root, manifest)? {
        dependencies.push_str(&fs::read_to_string(&krate.manifest)?);
        for path in krate.sources {
            source.push_str(&doctor_layout::symbols(&fs::read_to_string(path)?, true));
        }
    }
    for symbol in [
        "PlatformConfig",
        "PostgresPlatformInbox",
        "PostgresEntitlementCache",
        "EntitlementEventHandler",
        "PlatformEvents",
        "purge_received_before",
    ] {
        if !source.contains(symbol) {
            failures.push(format!(
                "missing platform {symbol} in declared backend crates"
            ));
        }
    }
    if !dependencies.contains("baukit-suite")
        || !dependencies.contains("\"platform\"")
        || !dependencies.contains("\"postgres\"")
    {
        failures
            .push("platform requires baukit-suite with platform and postgres features".to_owned());
    }
    let schema = root.join(&manifest.openapi.schema);
    let document: serde_json::Value = serde_json::from_str(&fs::read_to_string(schema)?)?;
    if document
        .pointer("/paths/~1platform~1inbound/post")
        .is_none()
    {
        failures.push("platform OpenAPI must describe POST /platform/inbound".to_owned());
    }
    successes.push("platform generation metadata and backend wiring checked".to_owned());
    Ok(())
}
