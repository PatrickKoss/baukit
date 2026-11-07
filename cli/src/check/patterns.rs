use std::{path::Path, process::Command};

use anyhow::{Context, Result, bail, ensure};
use include_dir::{Dir, include_dir};
use serde::Deserialize;

use super::{
    CheckContext, Finding, Rule,
    comments::SuppressionKind,
    suppressions::{missing_reason, scan_tree},
};

const NAME: &str = "patterns";
const AST_GREP: &str = "ast-grep";
const AST_GREP_VERSION: &str = "0.51.1";
const EXCLUDED_GLOBS: &[&str] = &[
    "!**/vendor/**",
    "!**/target/**",
    "!**/node_modules/**",
    "!**/.generated-fixture/**",
];
static RULES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../lint/ast-grep");

pub struct Patterns;

impl Rule for Patterns {
    fn name(&self) -> &'static str {
        NAME
    }

    fn run(&self, context: &CheckContext<'_>) -> Result<Vec<Finding>> {
        ensure_version()?;
        let rules =
            tempfile::tempdir().context("failed to create a temp dir for ast-grep rules")?;
        RULES
            .extract(rules.path())
            .context("failed to write the embedded ast-grep rules")?;
        let mut findings = Vec::new();
        for found in scan(context.root, &rules.path().join("sgconfig.yml"))? {
            let is_error = found.severity == "error";
            if is_error || context.options.verbose {
                let finding = if is_error {
                    Finding::error(
                        NAME,
                        found.file,
                        format!("{}: {}", found.rule_id, found.message),
                    )
                } else {
                    Finding::note(
                        NAME,
                        found.file,
                        format!("{}: {}", found.rule_id, found.message),
                    )
                };
                findings.push(finding.at_line(found.range.start.line + 1));
            }
        }
        findings.extend(
            scan_tree(context.root)?
                .iter()
                .filter(|(_, suppression)| {
                    suppression.kind == SuppressionKind::AstGrepIgnore && suppression.missing_reason
                })
                .map(|(path, suppression)| Finding {
                    rule: NAME,
                    ..missing_reason(path, suppression)
                }),
        );
        Ok(findings)
    }
}

fn ensure_version() -> Result<()> {
    let output = Command::new(AST_GREP)
        .arg("--version")
        .output()
        .with_context(|| {
            format!("`{AST_GREP}` not found; install ast-grep {AST_GREP_VERSION} (`mise install` or `cargo install ast-grep --locked --version {AST_GREP_VERSION}`)")
        })?;
    let version = String::from_utf8_lossy(&output.stdout);
    let version = version.trim();
    ensure!(
        version.rsplit(' ').next() == Some(AST_GREP_VERSION),
        "`{AST_GREP} --version` printed `{version}`; baukit check needs ast-grep {AST_GREP_VERSION}"
    );
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Match {
    file: String,
    rule_id: String,
    severity: String,
    message: String,
    range: Range,
}

#[derive(Debug, Deserialize)]
struct Range {
    start: Position,
}

#[derive(Debug, Deserialize)]
struct Position {
    line: usize,
}

fn scan(root: &Path, config: &Path) -> Result<Vec<Match>> {
    let mut command = Command::new(AST_GREP);
    command
        .current_dir(root)
        .arg("scan")
        .arg("--config")
        .arg(config)
        .arg("--json=stream");
    for glob in EXCLUDED_GLOBS {
        command.arg("--globs").arg(glob);
    }
    let output = command
        .arg(".")
        .output()
        .with_context(|| format!("failed to run `{AST_GREP} scan`"))?;
    if !output.status.success() && output.stdout.is_empty() {
        bail!(
            "`{AST_GREP} scan` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8(output.stdout).context("ast-grep printed non-UTF-8 output")?;
    let mut matches = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .with_context(|| format!("unexpected ast-grep output: {line}"))
        })
        .collect::<Result<Vec<Match>>>()?;
    for found in &mut matches {
        found.file = found.file.trim_start_matches("./").to_owned();
    }
    matches.sort_by(|left, right| {
        (&left.file, left.range.start.line).cmp(&(&right.file, right.range.start.line))
    });
    Ok(matches)
}
