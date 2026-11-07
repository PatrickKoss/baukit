use std::{fs, path::Path};

use anyhow::{Context, Result};

use crate::{
    doctor_layout, doctor_mcp,
    manifest::{Manifest, OpenApiPaths},
};

pub(crate) fn validate_openapi_paths(
    root: &Path,
    openapi: &OpenApiPaths,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) {
    let initial_failure_count = failures.len();
    validate_openapi_file(root, "schema", &openapi.schema, failures);
    let consumers = openapi.consumers();
    if consumers.is_empty() {
        failures.push("openapi.consumers must list at least one output".to_owned());
    }
    for consumer in consumers {
        validate_openapi_file(root, "consumer", consumer, failures);
    }
    if failures.len() == initial_failure_count {
        successes.push("manifest-declared OpenAPI files are present".to_owned());
    }
}

pub(crate) fn validate_openapi_file(
    root: &Path,
    kind: &str,
    relative: &str,
    failures: &mut Vec<String>,
) {
    let path = Path::new(relative);
    if relative.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        failures.push(format!(
            "OpenAPI {kind} `{relative}` must stay inside the product root"
        ));
    } else if !root.join(path).is_file() {
        failures.push(format!("missing OpenAPI {kind} file `{relative}`"));
    }
}

pub(crate) fn validate_remote_mcp(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let before = failures.len();
    doctor_mcp::validate_wiring(root, manifest, failures)?;
    for relative in ["backend/mcp-tools.json", "docs/remote-mcp.md"] {
        if !root.join(relative).is_file() {
            failures.push(format!("missing remote MCP artifact `{relative}`"));
        }
    }
    let values = root.join("deploy/values.yaml");
    if values.is_file() {
        let values: serde_yaml_ng::Value = serde_yaml_ng::from_str(&fs::read_to_string(values)?)?;
        let mcp = values.get("mcp").or_else(|| {
            values
                .get("application")
                .and_then(|application| application.get("mcp"))
        });
        for key in ["resourceUrl", "allowedHosts"] {
            if mcp
                .and_then(|mcp| mcp.get(key))
                .is_none_or(serde_yaml_ng::Value::is_null)
            {
                failures.push(format!("remote MCP deployment is missing `{key}:`"));
            }
        }
    } else {
        failures.push("missing remote MCP deployment file `deploy/values.yaml`".to_owned());
    }
    if failures.len() == before {
        successes.push("remote MCP router, resource metadata, OAuth layer, scoped tools, and drift check are wired".to_owned());
    }
    Ok(())
}

pub(crate) fn validate_migrations(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let directory = doctor_layout::migrations(root, manifest)?;
    if !directory.is_dir() {
        failures.push(format!(
            "missing backend migration directory `{}`",
            directory.strip_prefix(root)?.display()
        ));
        return Ok(());
    }
    let mut migrations = Vec::new();
    for entry in fs::read_dir(&directory)
        .with_context(|| format!("could not inspect {}", directory.display()))?
    {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "sql") {
            migrations.push(path);
        }
    }
    migrations.sort();
    if migrations.is_empty() {
        failures.push("backend migration directory contains no `.sql` migrations".to_owned());
    } else {
        successes.push(format!(
            "backend migration directory contains {} SQL migration(s)",
            migrations.len()
        ));
    }
    Ok(())
}

pub(crate) fn validate_jobs_migration(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let directory = doctor_layout::migrations(root, manifest)?;
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&directory)
        .with_context(|| format!("could not inspect {}", directory.display()))?
    {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "sql") {
            let source = fs::read_to_string(&path)
                .with_context(|| format!("could not read {}", path.display()))?;
            if sql_creates_table(&source, "job_outbox") {
                successes.push(
                    "a backend migration creates the baukit-jobs `job_outbox` table".to_owned(),
                );
                return Ok(());
            }
        }
    }
    failures.push("no backend migration creates the baukit-jobs `job_outbox` table".to_owned());
    Ok(())
}

pub(crate) fn sql_creates_table(source: &str, table: &str) -> bool {
    let uncommented = source
        .lines()
        .map(|line| line.split_once("--").map_or(line, |(sql, _)| sql))
        .collect::<Vec<_>>()
        .join(" ");
    let tokens = uncommented
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();

    tokens.iter().enumerate().any(|(index, token)| {
        if token != "create" || tokens.get(index + 1).is_none_or(|token| token != "table") {
            return false;
        }
        let mut name_index = index + 2;
        if tokens.get(name_index).is_some_and(|token| token == "if")
            && tokens
                .get(name_index + 1)
                .is_some_and(|token| token == "not")
            && tokens
                .get(name_index + 2)
                .is_some_and(|token| token == "exists")
        {
            name_index += 3;
        }
        tokens.get(name_index).is_some_and(|token| token == table)
            || tokens
                .get(name_index + 1)
                .is_some_and(|token| token == table)
    })
}
