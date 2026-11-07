use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};

use crate::{OPENAPI_TYPESCRIPT_PACKAGE, manifest::read_manifest};

/// Regenerates the TypeScript OpenAPI client for every configured consumer.
///
/// # Errors
///
/// Fails when `baukit.toml` cannot be read, the product has no backend or no
/// OpenAPI consumers, none of `corepack`, `pnpm` or `npx` is installed, an output
/// directory cannot be created, or the generator command fails.
pub fn generate_openapi_client(root: &Path) -> Result<()> {
    let manifest = read_manifest(root)?;
    if !manifest.capabilities.components.backend() {
        bail!("this product has no backend capability");
    }
    let consumers = manifest.openapi.consumers();
    if consumers.is_empty() {
        bail!("baukit.toml lists no OpenAPI consumers");
    }
    let corepack = command_exists("corepack");
    let pnpm = command_exists("pnpm");
    let npx = command_exists("npx");
    if !corepack && !pnpm && !npx {
        bail!(
            "TypeScript generation needs current Node.js LTS with corepack, pnpm, or npx; the committed OpenAPI schema was left unchanged"
        );
    }
    for consumer in consumers {
        let output = root.join(consumer);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut command = if corepack {
            let mut command = Command::new("corepack");
            command.args(["pnpm", "dlx", OPENAPI_TYPESCRIPT_PACKAGE]);
            command
        } else if pnpm {
            let mut command = Command::new("pnpm");
            command.args(["dlx", OPENAPI_TYPESCRIPT_PACKAGE]);
            command
        } else {
            let mut command = Command::new("npx");
            command.args(["--yes", OPENAPI_TYPESCRIPT_PACKAGE]);
            command
        };
        command
            .current_dir(root)
            .args([&manifest.openapi.schema, "-o", consumer]);
        run_checked(
            &mut command,
            &format!("TypeScript client generation for {consumer}"),
        )?;
    }
    Ok(())
}

pub(crate) fn command_exists(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn run_checked(command: &mut Command, label: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("could not start {label}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{label} failed with {status}")
    }
}
