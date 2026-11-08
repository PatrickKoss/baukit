mod agent_docs;
mod bulk;
mod comments;
mod config;
mod file_size;
mod hash_comments;
mod lints;
mod observability;
mod patterns;
mod suppressions;
mod template_files;
mod walk;

use std::{
    fmt,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};

pub use config::{CONFIG_FILE, CheckConfig};
pub(crate) use template_files::{
    MANIFEST_PATH as TEMPLATE_FILES_MANIFEST, render_manifest, sha256_hex,
};

/// A rule violation, or a note that does not fail the check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub path: Option<String>,
    pub line: Option<usize>,
    pub message: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Note,
}

impl Finding {
    fn error(rule: &'static str, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            rule,
            path: Some(path.into()),
            line: None,
            message: message.into(),
            severity: Severity::Error,
        }
    }

    fn note(rule: &'static str, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Note,
            ..Self::error(rule, path, message)
        }
    }

    #[must_use]
    fn at_line(mut self, line: usize) -> Self {
        self.line = Some(line);
        self
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: ", self.rule)?;
        if let Some(path) = &self.path {
            write!(formatter, "{path}")?;
            if let Some(line) = self.line {
                write!(formatter, ":{line}")?;
            }
            write!(formatter, ": ")?;
        }
        if self.severity == Severity::Note {
            write!(formatter, "note: ")?;
        }
        write!(formatter, "{}", self.message)
    }
}

/// Flags that change what a rule does beyond reporting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CheckOptions {
    pub verbose: bool,
    pub fix: bool,
    pub update: bool,
}

pub struct CheckContext<'a> {
    pub root: &'a Path,
    pub config: &'a CheckConfig,
    pub options: CheckOptions,
}

trait Rule {
    fn name(&self) -> &'static str;
    fn run(&self, context: &CheckContext<'_>) -> Result<Vec<Finding>>;
}

fn all_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(lints::Lints),
        Box::new(file_size::FileSize),
        Box::new(suppressions::Suppressions),
        Box::new(patterns::Patterns),
        Box::new(agent_docs::AgentDocs),
        Box::new(observability::Observability),
        Box::new(template_files::TemplateFiles),
    ]
}

/// Names of every rule, in run order.
#[must_use]
pub fn rule_names() -> Vec<&'static str> {
    all_rules().iter().map(|rule| rule.name()).collect()
}

pub struct CheckRequest {
    pub root: PathBuf,
    pub rules: Vec<String>,
    pub options: CheckOptions,
}

#[derive(Debug, Default)]
pub struct CheckReport {
    pub lines: Vec<String>,
    pub failed: bool,
}

/// Runs the selected rules (all when none are named) over `request.root`.
///
/// # Errors
///
/// Returns an error for an unknown rule, an unreadable or invalid `baukit-check.toml`, or a
/// rule that cannot run, such as a missing or mismatched ast-grep.
pub fn run_check(request: &CheckRequest) -> Result<CheckReport> {
    let rules = select_rules(&request.rules)?;
    let config = CheckConfig::load(&request.root)?;
    let context = CheckContext {
        root: &request.root,
        config: &config,
        options: request.options,
    };
    let mut report = CheckReport::default();
    let mut summaries = Vec::new();
    for rule in rules {
        let findings = rule.run(&context)?;
        let errors = findings
            .iter()
            .filter(|finding| finding.severity == Severity::Error)
            .count();
        report
            .lines
            .extend(findings.iter().map(ToString::to_string));
        report.failed |= errors > 0;
        summaries.push(summary_line(rule.name(), errors));
    }
    report.lines.extend(summaries);
    Ok(report)
}

fn summary_line(rule: &str, errors: usize) -> String {
    match errors {
        0 => format!("{rule}: ok"),
        1 => format!("{rule}: 1 finding"),
        count => format!("{rule}: {count} findings"),
    }
}

fn select_rules(names: &[String]) -> Result<Vec<Box<dyn Rule>>> {
    let rules = all_rules();
    if names.is_empty() {
        return Ok(rules);
    }
    if let Some(unknown) = names
        .iter()
        .find(|name| !rules.iter().any(|rule| rule.name() == name.as_str()))
    {
        bail!(
            "unknown rule `{unknown}`; known rules: {}",
            rule_names().join(", ")
        );
    }
    Ok(rules
        .into_iter()
        .filter(|rule| names.iter().any(|name| name == rule.name()))
        .collect())
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
