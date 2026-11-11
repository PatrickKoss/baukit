//! Nested ruff configs that lint less than the root config.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use toml::Value;

use super::{Finding, relative, walk};

const NAME: &str = "suppressions";
const ALL: &str = "ALL";
const DEFAULT_SELECT: &[&str] = &["E4", "E7", "E9", "F"];
const MAX_EXTEND_DEPTH: usize = 8;
const DOT_RUFF: &str = ".ruff.toml";
const RUFF: &str = "ruff.toml";
const PYPROJECT: &str = "pyproject.toml";
const UNUSED_NOQA: &str = "RUF100";
const PER_FILE_KEYS: &[&str] = &["per-file-ignores", "extend-per-file-ignores"];
const HIDES_NOQA_MESSAGE: &str = "`per-file-ignores` ignores RUF100, which hides unused `# noqa` comments from `baukit check`; remove the entry";

/// The rule selectors a config turns on and off after `extend` and `extend-*` are applied.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Effective {
    select: Vec<String>,
    ignore: Vec<String>,
}

/// Findings for every ruff config that hides unused `# noqa` comments, and for every nested
/// config that selects fewer rules than the root config.
pub fn scan(root: &Path) -> Result<Vec<Finding>> {
    let base = config_in(root).and_then(|file| effective(&file, 0));
    let mut findings = Vec::new();
    for path in walk::files(root, |path| is_config_name(walk::file_name(path)))? {
        let Some(directory) = path.parent() else {
            continue;
        };
        if config_in(directory).as_deref() != Some(path.as_path()) {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap_or_default();
        let display = relative(root, &path);
        if ruff_table(&path).is_some_and(|table| hides_unused_noqa(&table)) {
            findings.push(
                Finding::error(NAME, display.clone(), HIDES_NOQA_MESSAGE)
                    .at_line(line_containing(&source, "RUF")),
            );
        }
        let (Some(base), true) = (&base, directory != root) else {
            continue;
        };
        let Some(nested) = effective(&path, 0) else {
            continue;
        };
        findings.extend(weakenings(base, &nested).into_iter().map(|message| {
            Finding::error(NAME, display.clone(), message)
                .at_line(line_containing(&source, "select"))
        }));
    }
    Ok(findings)
}

/// `per-file-ignores` entries are the one setting ruff applies over `--extend-select`.
fn hides_unused_noqa(table: &toml::Table) -> bool {
    let lint = table.get("lint").and_then(Value::as_table);
    [Some(table), lint].into_iter().flatten().any(|scope| {
        PER_FILE_KEYS
            .iter()
            .filter_map(|key| scope.get(*key).and_then(Value::as_table))
            .flat_map(|files| files.values())
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(Value::as_str)
            .any(|ignored| covers(ignored, UNUSED_NOQA))
    })
}

fn is_config_name(name: &str) -> bool {
    matches!(name, RUFF | DOT_RUFF | PYPROJECT)
}

/// The config ruff reads for files in `directory`: `.ruff.toml`, then `ruff.toml`, then a
/// `pyproject.toml` with a `[tool.ruff]` table.
fn config_in(directory: &Path) -> Option<PathBuf> {
    [DOT_RUFF, RUFF, PYPROJECT]
        .iter()
        .map(|name| directory.join(name))
        .find(|file| ruff_table(file).is_some())
}

fn ruff_table(file: &Path) -> Option<toml::Table> {
    let table: toml::Table = fs::read_to_string(file).ok()?.parse().ok()?;
    if walk::file_name(file) != PYPROJECT {
        return Some(table);
    }
    match table.get("tool")?.get("ruff")? {
        Value::Table(ruff) => Some(ruff.clone()),
        _ => None,
    }
}

fn effective(file: &Path, depth: usize) -> Option<Effective> {
    let table = ruff_table(file)?;
    let base = match table.get("extend").and_then(Value::as_str) {
        Some(parent) if depth < MAX_EXTEND_DEPTH => {
            effective(&file.parent()?.join(parent), depth + 1)?
        }
        _ => Effective {
            select: DEFAULT_SELECT.iter().map(ToString::to_string).collect(),
            ignore: Vec::new(),
        },
    };
    let mut select = option(&table, "select").unwrap_or(base.select);
    select.extend(option(&table, "extend-select").unwrap_or_default());
    let mut ignore = option(&table, "ignore").unwrap_or(base.ignore);
    ignore.extend(option(&table, "extend-ignore").unwrap_or_default());
    Some(Effective { select, ignore })
}

/// A list option from `[lint]` or, for the deprecated spelling, the top level.
fn option(table: &toml::Table, key: &str) -> Option<Vec<String>> {
    let value = table
        .get("lint")
        .and_then(|lint| lint.get(key))
        .or_else(|| table.get(key))?;
    Some(
        value
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect(),
    )
}

fn covers(selector: &str, code: &str) -> bool {
    selector == ALL || code.starts_with(selector)
}

fn overlaps(left: &str, right: &str) -> bool {
    left == ALL || right == ALL || left.starts_with(right) || right.starts_with(left)
}

fn weakenings(base: &Effective, nested: &Effective) -> Vec<String> {
    let mut messages = Vec::new();
    let dropped: Vec<&str> = base
        .select
        .iter()
        .map(String::as_str)
        .filter(|code| !base.ignore.iter().any(|ignored| covers(ignored, code)))
        .filter(|code| !nested.select.iter().any(|selector| covers(selector, code)))
        .collect();
    if !dropped.is_empty() {
        messages.push(format!(
            "nested ruff config drops rules the root config selects ({}); `extend` the root config and use `per-file-ignores` for the exceptions",
            quoted(&dropped)
        ));
    }
    let silenced: Vec<&str> = nested
        .ignore
        .iter()
        .map(String::as_str)
        .filter(|code| base.select.iter().any(|selector| overlaps(selector, code)))
        .filter(|code| !base.ignore.iter().any(|ignored| covers(ignored, code)))
        .collect();
    if !silenced.is_empty() {
        messages.push(format!(
            "nested ruff config ignores rules the root config enforces ({}); use `per-file-ignores` with a reason",
            quoted(&silenced)
        ));
    }
    messages
}

fn quoted(codes: &[&str]) -> String {
    codes
        .iter()
        .map(|code| format!("`{code}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn line_containing(source: &str, needle: &str) -> usize {
    source
        .lines()
        .position(|line| line.contains(needle))
        .map_or(1, |index| index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "[lint]\nselect = [\"E\", \"F\", \"S\"]\nignore = [\"S603\"]\n";

    fn run(files: &[(&str, &str)]) -> Result<Vec<String>> {
        let root = tempfile::tempdir()?;
        for (path, text) in files {
            let file = root.path().join(path);
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(file, text)?;
        }
        Ok(scan(root.path())?.iter().map(ToString::to_string).collect())
    }

    #[test]
    fn narrower_select_is_a_finding() -> Result<()> {
        let lines = run(&[
            ("ruff.toml", ROOT),
            ("animations/ruff.toml", "[lint]\nselect = [\"F\", \"E9\"]\n"),
        ])?;
        assert_eq!(
            lines,
            vec![
                "suppressions: animations/ruff.toml:2: nested ruff config drops rules the root config selects (`E`, `S`); `extend` the root config and use `per-file-ignores` for the exceptions"
            ]
        );
        Ok(())
    }

    #[test]
    fn extending_the_root_keeps_its_rules() -> Result<()> {
        let lines = run(&[
            ("ruff.toml", ROOT),
            (
                "tools/ruff.toml",
                "extend = \"../ruff.toml\"\n[lint]\nextend-select = [\"B\"]\n",
            ),
            (
                "scripts/ruff.toml",
                "extend = \"../ruff.toml\"\n[lint.per-file-ignores]\n\"x.py\" = [\"S101\"]\n",
            ),
        ])?;
        assert!(lines.is_empty(), "{lines:?}");
        Ok(())
    }

    #[test]
    fn same_or_broader_select_passes() -> Result<()> {
        let lines = run(&[
            ("ruff.toml", ROOT),
            (
                "a/ruff.toml",
                "[lint]\nselect = [\"E\", \"F\", \"S\", \"I\"]\nignore = [\"S603\"]\n",
            ),
            (
                "b/.ruff.toml",
                "[lint]\nselect = [\"ALL\"]\nignore = [\"S603\"]\n",
            ),
        ])?;
        assert!(lines.is_empty(), "{lines:?}");
        Ok(())
    }

    #[test]
    fn extra_ignores_of_enforced_rules_are_findings() -> Result<()> {
        let lines = run(&[
            ("ruff.toml", ROOT),
            (
                "a/ruff.toml",
                "extend = \"../ruff.toml\"\n[lint]\nextend-ignore = [\"S101\", \"S603\", \"W\"]\n",
            ),
        ])?;
        assert_eq!(
            lines,
            vec![
                "suppressions: a/ruff.toml:1: nested ruff config ignores rules the root config enforces (`S101`); use `per-file-ignores` with a reason"
            ]
        );
        Ok(())
    }

    #[test]
    fn pyproject_tables_and_top_level_keys_count() -> Result<()> {
        let lines = run(&[
            (
                "pyproject.toml",
                "[tool.ruff.lint]\nselect = [\"E\", \"S\"]\n",
            ),
            ("pkg/pyproject.toml", "[tool.ruff]\nselect = [\"E\"]\n"),
            ("other/pyproject.toml", "[project]\nname = \"x\"\n"),
        ])?;
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("suppressions: pkg/pyproject.toml:2: "),
            "{lines:?}"
        );
        assert!(lines[0].contains("(`S`)"), "{lines:?}");
        Ok(())
    }

    #[test]
    fn no_root_config_means_nothing_to_compare() -> Result<()> {
        let lines = run(&[("a/ruff.toml", "[lint]\nselect = [\"F\"]\n")])?;
        assert!(lines.is_empty());
        Ok(())
    }

    #[test]
    fn a_shadowed_pyproject_is_not_the_config_ruff_reads() -> Result<()> {
        let lines = run(&[
            ("ruff.toml", ROOT),
            ("a/ruff.toml", ROOT),
            ("a/pyproject.toml", "[tool.ruff.lint]\nselect = [\"F\"]\n"),
        ])?;
        assert!(lines.is_empty(), "{lines:?}");
        Ok(())
    }

    #[test]
    fn per_file_ignores_of_unused_noqa_are_findings_in_any_config() -> Result<()> {
        let lines = run(&[
            (
                "ruff.toml",
                "[lint]\nselect = [\"E\"]\n[lint.per-file-ignores]\n\"a.py\" = [\"RUF100\"]\n",
            ),
            (
                "pkg/pyproject.toml",
                "[tool.ruff.lint.extend-per-file-ignores]\n\"*\" = [\"RUF\"]\n",
            ),
            (
                "ok/ruff.toml",
                "extend = \"../ruff.toml\"\n[lint.per-file-ignores]\n\"a.py\" = [\"S101\"]\n",
            ),
            ("plain/ruff.toml", "[lint]\nignore = [\"RUF100\"]\n"),
        ])?;
        let lines: Vec<_> = lines
            .into_iter()
            .filter(|line| line.contains("ignores RUF100"))
            .collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines.iter().any(|line| line
                .starts_with("suppressions: ruff.toml:4: `per-file-ignores` ignores RUF100")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("suppressions: pkg/pyproject.toml:2: ")),
            "{lines:?}"
        );
        Ok(())
    }
}
