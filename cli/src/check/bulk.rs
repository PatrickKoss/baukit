use std::path::Path;

use anyhow::Result;

use super::{Finding, relative, walk};

const NAME: &str = "suppressions";
const ESLINT_SUPPRESSIONS_FILE: &str = "eslint-suppressions.json";
const SUPPRESSIONS_LOCATION_FLAG: &str = "--suppressions-location";
const ESLINT_BULK_FLAGS: &[&str] = &["--suppress-all", "--suppress-rule", "--prune-suppressions"];
const TEMPLATE_SUFFIXES: &[&str] = &[".j2", ".jinja"];
const GATE_SCRIPT_EXTENSIONS: &[&str] = &["sh", "bash", "mk", "yml", "yaml"];
const GATE_SCRIPT_NAMES: &[&str] = &["Makefile", "justfile", "Justfile"];
const LINT_TABLE_PREFIXES: &[&str] = &["[lints", "[workspace.lints"];
const BULK_FILE_MESSAGE: &str = "bulk suppression file; fix the findings instead";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Candidate {
    EslintSuppressions,
    PackageJson,
    CargoManifest,
    CargoConfig,
    GateScript,
}

/// Findings for every mechanism that silences lints wholesale instead of at one site.
pub fn scan(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for path in walk::files(root, |path| candidate(path).is_some())? {
        let Some(kind) = candidate(&path) else {
            continue;
        };
        let display = relative(root, &path);
        if kind == Candidate::EslintSuppressions {
            findings.push(Finding::error(NAME, display, BULK_FILE_MESSAGE));
            continue;
        }
        let Some(source) = walk::read_text(&path)? else {
            continue;
        };
        findings.extend(scan_source(kind, &display, &source, &path));
    }
    Ok(findings)
}

fn candidate(path: &Path) -> Option<Candidate> {
    let name = walk::file_name(path);
    let name = TEMPLATE_SUFFIXES
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
        .unwrap_or(name);
    let in_cargo_dir = path
        .parent()
        .is_some_and(|parent| walk::file_name(parent) == ".cargo");
    let extension = name.rsplit_once('.').map(|(_, extension)| extension);
    match name {
        ESLINT_SUPPRESSIONS_FILE => Some(Candidate::EslintSuppressions),
        "package.json" => Some(Candidate::PackageJson),
        "Cargo.toml" => Some(Candidate::CargoManifest),
        "config" | "config.toml" if in_cargo_dir => Some(Candidate::CargoConfig),
        _ if GATE_SCRIPT_NAMES.contains(&name) => Some(Candidate::GateScript),
        _ if extension.is_some_and(|extension| GATE_SCRIPT_EXTENSIONS.contains(&extension)) => {
            Some(Candidate::GateScript)
        }
        _ => None,
    }
}

fn scan_source(kind: Candidate, display: &str, source: &str, path: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut table = String::new();
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            trimmed.clone_into(&mut table);
        }
        let message = match kind {
            Candidate::PackageJson => {
                findings.extend(named_suppressions_file(line, path, display));
                eslint_bulk_flag(line)
            }
            Candidate::CargoManifest => lint_table_allow(&table, trimmed),
            Candidate::CargoConfig => rustflags_allow(trimmed),
            Candidate::GateScript => clippy_allow(trimmed),
            Candidate::EslintSuppressions => None,
        };
        if let Some(message) = message {
            findings.push(Finding::error(NAME, display, message).at_line(index + 1));
        }
    }
    findings
}

fn eslint_bulk_flag(line: &str) -> Option<String> {
    let flag = ESLINT_BULK_FLAGS
        .iter()
        .chain(std::iter::once(&SUPPRESSIONS_LOCATION_FLAG))
        .find(|flag| tokens(line).any(|token| token.starts_with(**flag)))?;
    Some(format!(
        "`{flag}` writes a bulk suppression file; fix the findings instead"
    ))
}

/// The file a `--suppressions-location` script names, reported when it exists.
fn named_suppressions_file(line: &str, package: &Path, display: &str) -> Option<Finding> {
    let mut words = tokens(line);
    let location = loop {
        let token = words.next()?;
        if let Some(value) = token.strip_prefix(SUPPRESSIONS_LOCATION_FLAG) {
            match value.strip_prefix('=') {
                Some(value) => break value.to_owned(),
                None => break words.next()?.to_owned(),
            }
        }
    };
    let file = package.parent()?.join(&location);
    if !file.is_file() || walk::file_name(&file) == ESLINT_SUPPRESSIONS_FILE {
        return None;
    }
    let shown = Path::new(display).parent()?.join(location);
    Some(Finding::error(
        NAME,
        shown.display().to_string(),
        BULK_FILE_MESSAGE,
    ))
}

fn lint_table_allow(table: &str, line: &str) -> Option<String> {
    if !LINT_TABLE_PREFIXES
        .iter()
        .any(|prefix| table.starts_with(prefix))
        || line.starts_with('[')
        || line.starts_with('#')
    {
        return None;
    }
    let (lint, value) = line.split_once('=')?;
    let compact: String = value.split_whitespace().collect();
    let allows = compact == "\"allow\"" || compact.contains("level=\"allow\"");
    allows.then(|| {
        format!(
            "`{}` set to allow in `{table}`; fix the findings or suppress one site with `#[expect]`",
            lint.trim()
        )
    })
}

fn rustflags_allow(line: &str) -> Option<String> {
    if line.starts_with('#') {
        return None;
    }
    let flag = tokens(line).find(|token| is_allow_flag(token))?;
    Some(format!(
        "rustflags `{flag}` silences lints for the whole build; fix the findings instead"
    ))
}

fn clippy_allow(line: &str) -> Option<String> {
    if line.starts_with('#') {
        return None;
    }
    let flag = tokens(line)
        .skip_while(|token| *token != "clippy" && !token.contains("RUSTFLAGS"))
        .find(|token| is_allow_flag(token))?;
    Some(format!(
        "`{flag}` silences lints in the gate; fix the findings instead"
    ))
}

fn is_allow_flag(token: &str) -> bool {
    token.starts_with("-A") || token.starts_with("--allow") || token.starts_with("--cap-lints")
}

/// Whitespace-separated words with surrounding quotes, commas and brackets removed.
fn tokens(line: &str) -> impl Iterator<Item = &str> {
    line.split(|character: char| character.is_whitespace() || character == '=')
        .flat_map(|word| word.split(['"', '\'', ',', '[', ']']))
        .filter(|word| !word.is_empty())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn messages(root: &Path) -> Result<Vec<String>> {
        Ok(scan(root)?.iter().map(ToString::to_string).collect())
    }

    fn write(root: &Path, path: &str, text: &str) -> Result<()> {
        let file = root.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(file, text)?;
        Ok(())
    }

    #[test]
    fn eslint_suppression_files_and_flags_are_findings() -> Result<()> {
        let root = tempfile::tempdir()?;
        write(root.path(), "web/eslint-suppressions.json", "{}")?;
        write(root.path(), "web/lint-baseline.json", "{}")?;
        write(
            root.path(),
            "web/package.json",
            "{\n  \"scripts\": {\n    \"lint\": \"eslint . --suppressions-location lint-baseline.json\",\n    \"x\": \"eslint . --suppress-all\",\n    \"ok\": \"eslint .\"\n  }\n}\n",
        )?;
        let lines = messages(root.path())?;
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(lines.contains(&format!(
            "suppressions: web/eslint-suppressions.json: {BULK_FILE_MESSAGE}"
        )));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("lint-baseline.json: bulk"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("package.json:3: `--suppressions-location`"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("package.json:4: `--suppress-all`"))
        );
        Ok(())
    }

    #[test]
    fn clippy_allow_flags_in_gate_scripts_are_findings() -> Result<()> {
        let root = tempfile::tempdir()?;
        write(
            root.path(),
            "scripts/gate.sh",
            "cargo clippy --all-targets -- -D warnings -A clippy::unwrap_used\ncargo clippy -- --cap-lints=warn\n# cargo clippy -- -A x\ncargo clippy --all-targets -- -D warnings\ngrep -A 3 clippy log\n",
        )?;
        write(
            root.path(),
            "Makefile",
            "lint:\n\tRUSTFLAGS=\"-Adead_code\" cargo build\n",
        )?;
        let lines = messages(root.path())?;
        assert_eq!(
            lines,
            vec![
                "suppressions: Makefile:2: `-Adead_code` silences lints in the gate; fix the findings instead",
                "suppressions: scripts/gate.sh:1: `-A` silences lints in the gate; fix the findings instead",
                "suppressions: scripts/gate.sh:2: `--cap-lints` silences lints in the gate; fix the findings instead",
            ]
        );
        Ok(())
    }

    #[test]
    fn lint_tables_and_rustflags_that_allow_are_findings() -> Result<()> {
        let root = tempfile::tempdir()?;
        write(
            root.path(),
            "Cargo.toml",
            "[package]\nname = \"allow\"\n\n[lints.clippy]\nall = { level = \"warn\", priority = -1 }\nunwrap_used = \"allow\"\nexpect_used = { level = \"allow\" }\n\n[workspace.lints.rust]\nunsafe_code = \"deny\"\ndead_code = \"allow\"\n",
        )?;
        write(
            root.path(),
            ".cargo/config.toml",
            "[net]\ngit-fetch-with-cli = true\n[build]\nrustflags = [\"-D\", \"warnings\", \"--cap-lints\", \"warn\"]\n",
        )?;
        let lines = messages(root.path())?;
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|line| line.contains(".cargo/config.toml:4: rustflags `--cap-lints`"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Cargo.toml:6: `unwrap_used` set to allow"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Cargo.toml:7: `expect_used` set to allow"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Cargo.toml:11: `dead_code` set to allow"))
        );
        Ok(())
    }
}
