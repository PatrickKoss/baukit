use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result};

use super::{
    CONFIG_FILE, CheckContext, Finding, Rule, bulk,
    comments::{self, Suppression, SuppressionKind},
    relative, walk,
};

const NAME: &str = "suppressions";
const MAX_SECTION: &str = "[suppressions.max]";

pub struct Suppressions;

impl Rule for Suppressions {
    fn name(&self) -> &'static str {
        NAME
    }

    fn run(&self, context: &CheckContext<'_>) -> Result<Vec<Finding>> {
        let mut findings = bulk::scan(context.root)?;
        let mut counts: BTreeMap<SuppressionKind, usize> = BTreeMap::new();
        for (path, suppression) in scan_tree(context.root)? {
            *counts.entry(suppression.kind).or_default() += 1;
            if suppression.missing_reason {
                findings.push(missing_reason(&path, &suppression));
            }
        }
        let max = &context.config.suppressions.max;
        let mut lowered = BTreeMap::new();
        for kind in SuppressionKind::ALL {
            let count = counts.get(&kind).copied().unwrap_or(0);
            let limit = max.get(kind.key()).copied().unwrap_or(0);
            if count > limit {
                findings.push(Finding::error(
                    NAME,
                    CONFIG_FILE,
                    format!(
                        "{} count {count} exceeds max {limit}; remove suppressions instead of raising the max",
                        kind.key()
                    ),
                ));
            } else if count < limit {
                lowered.insert(kind.key(), count);
                if !context.options.update {
                    findings.push(Finding::note(
                        NAME,
                        CONFIG_FILE,
                        format!(
                            "{} count is {count}, lower max to {count} (or run `baukit check suppressions --update`)",
                            kind.key()
                        ),
                    ));
                }
            }
        }
        if context.options.update && !lowered.is_empty() {
            lower_maxima(context.root, &lowered)?;
        }
        Ok(findings)
    }
}

/// Every suppression under `root`, with its file path relative to `root`.
pub fn scan_tree(root: &Path) -> Result<Vec<(String, Suppression)>> {
    let mut found = Vec::new();
    for path in walk::files(root, comments::is_scanned)? {
        let Some(source) = walk::read_text(&path)? else {
            continue;
        };
        let display = relative(root, &path);
        found.extend(
            comments::scan(&path, &source)
                .into_iter()
                .map(|suppression| (display.clone(), suppression)),
        );
    }
    Ok(found)
}

pub fn missing_reason(path: &str, suppression: &Suppression) -> Finding {
    Finding::error(NAME, path, suppression.kind.missing_reason_message()).at_line(suppression.line)
}

/// Rewrites `[suppressions.max]` values in place; callers pass only values below the current max.
fn lower_maxima(root: &Path, lowered: &BTreeMap<&str, usize>) -> Result<()> {
    let path = root.join(CONFIG_FILE);
    let source =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut in_section = false;
    let mut output = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == MAX_SECTION;
        }
        output.push_str(&rewrite_line(line, in_section, lowered));
        output.push('\n');
    }
    fs::write(&path, output).with_context(|| format!("failed to write {}", path.display()))
}

fn rewrite_line(line: &str, in_section: bool, lowered: &BTreeMap<&str, usize>) -> String {
    if !in_section {
        return line.to_owned();
    }
    let Some((key, _)) = line.split_once('=') else {
        return line.to_owned();
    };
    let name = key.trim().trim_matches('"');
    match lowered.get(name) {
        Some(value) => format!("{} = {value}", key.trim_end()),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{CheckConfig, CheckOptions};

    const CONFIG: &str =
        "# budget\n[suppressions.max]\nrust-allow = 0\nrust-expect = 3\n\"eslint-disable\" = 1\n";

    fn run(root: &Path, options: CheckOptions) -> Result<Vec<String>> {
        let config = CheckConfig::load(root)?;
        let context = CheckContext {
            root,
            config: &config,
            options,
        };
        Ok(Suppressions
            .run(&context)?
            .iter()
            .map(ToString::to_string)
            .collect())
    }

    fn tree() -> Result<tempfile::TempDir> {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join(CONFIG_FILE), CONFIG)?;
        fs::write(
            root.path().join("a.rs"),
            "#[expect(dead_code, reason = \"r\")]\nfn a() {}\n",
        )?;
        fs::write(
            root.path().join("b.ts"),
            "// eslint-disable-next-line x -- reason\n",
        )?;
        Ok(root)
    }

    #[test]
    fn counts_below_max_ask_to_lower_it() -> Result<()> {
        let root = tree()?;
        let lines = run(root.path(), CheckOptions::default())?;
        assert_eq!(
            lines,
            vec![
                "suppressions: baukit-check.toml: note: rust-expect count is 1, lower max to 1 (or run `baukit check suppressions --update`)"
            ]
        );
        Ok(())
    }

    #[test]
    fn counts_above_max_and_missing_reasons_fail() -> Result<()> {
        let root = tree()?;
        fs::write(root.path().join("c.rs"), "#[allow(dead_code)]\nfn c() {}\n")?;
        let lines = run(root.path(), CheckOptions::default())?;
        assert!(
            lines.contains(
                &"suppressions: c.rs:1: `#[allow]` without `reason = \"...\"`".to_owned()
            )
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("rust-allow count 1 exceeds max 0"))
        );
        Ok(())
    }

    #[test]
    fn update_only_lowers_the_max() -> Result<()> {
        let root = tree()?;
        fs::write(root.path().join("d.rs"), "#[allow(x, reason = \"r\")]\n")?;
        let lines = run(
            root.path(),
            CheckOptions {
                update: true,
                ..CheckOptions::default()
            },
        )?;
        assert_eq!(lines.len(), 1, "{lines:?}");
        let rewritten = fs::read_to_string(root.path().join(CONFIG_FILE))?;
        assert_eq!(
            rewritten,
            "# budget\n[suppressions.max]\nrust-allow = 0\nrust-expect = 1\n\"eslint-disable\" = 1\n"
        );
        Ok(())
    }
}
