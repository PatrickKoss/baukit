//! `# noqa` comments that ruff itself reports as unused (RUF100).

use std::{collections::HashSet, path::Path, process::Command};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::Finding;

const RUFF: &str = "ruff";
const UNUSED_NOQA: &str = "RUF100";

#[derive(Debug, Deserialize)]
struct Diagnostic {
    code: Option<String>,
    filename: String,
    location: Location,
}

#[derive(Debug, Deserialize)]
struct Location {
    row: usize,
}

const TEMPLATES_DIR: &str = "templates/";
const TEMPLATE_PRODUCT_CONFIG: &str = "templates/common/ruff.toml";

pub fn finding(rule: &'static str, path: &str, line: usize) -> Finding {
    Finding::error(
        rule,
        path,
        "unused `# noqa`, remove it (`ruff check --extend-select RUF100 --fix`)",
    )
    .at_line(line)
}

/// `(path relative to root, line)` of every `# noqa` that ruff reports as unused under the
/// project's own ruff config. In a checkout that holds product templates, `templates/` is
/// skipped: it renders with the product's ruff config, which the generated-fixture check uses.
pub fn locations(root: &Path) -> Result<HashSet<(String, usize)>> {
    let output = Command::new(RUFF)
        .current_dir(root)
        .args(["check", "--extend-select", UNUSED_NOQA])
        .args(["--output-format", "json", "--no-cache", "--exit-zero", "."])
        .output()
        .with_context(|| {
            format!("`{RUFF}` not found; install ruff (`mise install`) so unused `# noqa` comments can be told apart")
        })?;
    if !output.status.success() {
        bail!(
            "`{RUFF} check` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8(output.stdout).context("ruff printed non-UTF-8 output")?;
    let mut unused = parse(root, &stdout)?;
    if root.join(TEMPLATE_PRODUCT_CONFIG).is_file() {
        unused.retain(|(path, _)| !path.starts_with(TEMPLATES_DIR));
    }
    Ok(unused)
}

fn parse(root: &Path, json: &str) -> Result<HashSet<(String, usize)>> {
    let diagnostics: Vec<Diagnostic> =
        serde_json::from_str(json).context("unexpected ruff JSON output")?;
    let roots: Vec<_> = std::iter::once(root.to_path_buf())
        .chain(root.canonicalize())
        .collect();
    Ok(diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.code.as_deref() == Some(UNUSED_NOQA))
        .filter_map(|diagnostic| {
            let file = Path::new(&diagnostic.filename);
            let relative = roots.iter().find_map(|base| file.strip_prefix(base).ok())?;
            Some((
                relative.to_string_lossy().replace('\\', "/"),
                diagnostic.location.row,
            ))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_unused_noqa_diagnostics_under_the_root() -> Result<()> {
        let json = r#"[
            {"code": "RUF100", "filename": "/repo/a/x.py", "location": {"row": 4, "column": 1}},
            {"code": "E402", "filename": "/repo/a/x.py", "location": {"row": 9, "column": 1}},
            {"code": "RUF100", "filename": "/elsewhere/y.py", "location": {"row": 2, "column": 1}},
            {"code": null, "filename": "/repo/z.py", "location": {"row": 1, "column": 1}}
        ]"#;
        let found = parse(Path::new("/repo"), json)?;
        assert_eq!(found, HashSet::from([("a/x.py".to_owned(), 4)]));
        Ok(())
    }

    #[test]
    fn rejects_output_that_is_not_ruff_json() {
        assert!(parse(Path::new("/repo"), "error: nope").is_err());
    }
}
