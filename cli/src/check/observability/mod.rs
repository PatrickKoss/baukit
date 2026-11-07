mod dashboards;
mod promql;
mod sources;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use super::{CheckContext, Finding, Rule, config::ObservabilityConfig, relative, walk};
use promql::{Patterns, allowed_metric_names, base_metric};
use sources::{
    EmittedMetric, MetricKind, MetricScanner, is_test_file, naming_problems, parse_metric_list,
};

const NAME: &str = "observability";
const DASHBOARD_EXTENSIONS: &[&str] = &["json"];
const RULE_EXTENSIONS: &[&str] = &["yml", "yaml"];
const SOURCE_EXTENSIONS: &[&str] = &["rs"];

pub struct Observability;

impl Rule for Observability {
    fn name(&self) -> &'static str {
        NAME
    }

    fn run(&self, context: &CheckContext<'_>) -> Result<Vec<Finding>> {
        let Some(config) = &context.config.observability else {
            return Ok(vec![Finding::note(
                NAME,
                super::CONFIG_FILE,
                "not configured; add an [observability] table to check dashboards and rules",
            )]);
        };
        Check::new(context.root, config)?.run()
    }
}

struct Check<'a> {
    root: &'a Path,
    config: &'a ObservabilityConfig,
    patterns: Patterns,
    findings: Vec<Finding>,
}

/// A rule file's display path and content.
struct RuleFile {
    path: String,
    content: String,
}

impl<'a> Check<'a> {
    fn new(root: &'a Path, config: &'a ObservabilityConfig) -> Result<Self> {
        Ok(Self {
            root,
            config,
            patterns: Patterns::new()?,
            findings: Vec::new(),
        })
    }

    fn run(mut self) -> Result<Vec<Finding>> {
        let emitted = self.emitted_metrics()?;
        let (listed, listed_histograms) = self.listed_metrics()?;
        let rule_files = self.rule_files()?;
        let recordings: BTreeSet<String> = rule_files
            .iter()
            .flat_map(|file| self.patterns.recording_rule_names(&file.content))
            .collect();
        self.check_recordings_do_not_shadow(&rule_files, &emitted, &listed);
        let product_names = listed
            .iter()
            .map(String::as_str)
            .chain(emitted.keys().map(String::as_str));
        let histograms = listed_histograms.iter().map(String::as_str).chain(
            emitted
                .iter()
                .filter(|(_, metric)| metric.kind == MetricKind::Histogram)
                .map(|(name, _)| name.as_str()),
        );
        let allowed = allowed_metric_names(
            product_names,
            histograms,
            recordings.iter().map(String::as_str),
        );
        let referenced = self.check_dashboards(&allowed)?;
        self.check_rules(&rule_files, &allowed);
        self.check_dashboard_coverage(&emitted, &referenced);
        Ok(self.findings)
    }

    fn error(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.findings.push(Finding::error(NAME, path, message));
    }

    /// Files named by the configured entries: a file as is, a directory's direct children by extension.
    fn configured_files(
        &mut self,
        entries: &[String],
        extensions: &[&str],
    ) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        for entry in entries {
            let path = self.root.join(entry);
            if path.is_file() {
                files.push(path);
            } else if path.is_dir() {
                files.extend(directory_files(&path, extensions)?);
            } else {
                self.error(
                    entry.as_str(),
                    "configured in [observability] but does not exist",
                );
            }
        }
        Ok(files)
    }

    fn emitted_metrics(&mut self) -> Result<BTreeMap<String, EmittedMetric>> {
        let scanner = MetricScanner::new()?;
        let mut metrics = BTreeMap::new();
        for entry in &self.config.sources {
            let directory = self.root.join(entry);
            if !directory.is_dir() {
                self.error(
                    entry.as_str(),
                    "configured in [observability] but is no directory",
                );
                continue;
            }
            let files = walk::files(&directory, |path| has_extension(path, SOURCE_EXTENSIONS))?;
            for file in files {
                let display = relative(self.root, &file);
                if is_test_file(&display) {
                    continue;
                }
                if let Some(text) = walk::read_text(&file)? {
                    scanner.scan(&display, &text, &mut metrics);
                }
            }
        }
        let prefix = self.config.metric_prefix.as_deref();
        for (name, metric) in &metrics {
            for problem in naming_problems(name, metric.kind, prefix) {
                self.findings
                    .push(Finding::error(NAME, metric.path.as_str(), problem).at_line(metric.line));
            }
        }
        Ok(metrics)
    }

    fn listed_metrics(&mut self) -> Result<(Vec<String>, Vec<String>)> {
        let mut names = Vec::new();
        let mut histograms = Vec::new();
        let entries = self.config.metrics.clone();
        for path in self.configured_files(&entries, &[])? {
            let text = read(&path)?;
            let list = parse_metric_list(&text);
            let display = relative(self.root, &path);
            for (line, problem) in list.problems {
                self.findings
                    .push(Finding::error(NAME, display.as_str(), problem).at_line(line));
            }
            names.extend(list.names);
            histograms.extend(list.histograms);
        }
        Ok((names, histograms))
    }

    fn rule_files(&mut self) -> Result<Vec<RuleFile>> {
        let entries = self.config.rules.clone();
        self.configured_files(&entries, RULE_EXTENSIONS)?
            .into_iter()
            .map(|path| {
                Ok(RuleFile {
                    path: relative(self.root, &path),
                    content: read(&path)?,
                })
            })
            .collect()
    }

    fn check_recordings_do_not_shadow(
        &mut self,
        rule_files: &[RuleFile],
        emitted: &BTreeMap<String, EmittedMetric>,
        listed: &[String],
    ) {
        for file in rule_files {
            for expression in self.patterns.rule_expressions(&file.content) {
                let Some(rule) = expression.rule.filter(|rule| rule.kind == "record") else {
                    continue;
                };
                let shadows = emitted.contains_key(&rule.name)
                    || listed.contains(&rule.name)
                    || promql::CATALOG_METRICS.contains(&rule.name.as_str());
                if shadows {
                    self.findings.push(
                        Finding::error(
                            NAME,
                            file.path.as_str(),
                            format!("recording rule '{}' shadows an emitted metric", rule.name),
                        )
                        .at_line(expression.line),
                    );
                }
            }
        }
    }

    /// Lints every dashboard and returns the base names of the metrics they reference.
    fn check_dashboards(&mut self, allowed: &BTreeSet<String>) -> Result<BTreeSet<String>> {
        let mut referenced = BTreeSet::new();
        let entries = self.config.dashboards.clone();
        for path in self.configured_files(&entries, DASHBOARD_EXTENSIONS)? {
            let display = relative(self.root, &path);
            let document: serde_json::Value = match serde_json::from_str(&read(&path)?) {
                Ok(document) => document,
                Err(error) => {
                    self.error(display, format!("invalid dashboard JSON: {error}"));
                    continue;
                }
            };
            for (location, expression) in dashboards::expressions(&document, &display) {
                for problem in self.patterns.lint_expression(&expression, allowed) {
                    self.error(location.as_str(), problem);
                }
                referenced.extend(
                    self.patterns
                        .metric_references(&expression)
                        .iter()
                        .map(|name| base_metric(name).to_owned()),
                );
            }
            for (location, problem) in
                dashboards::datasource_problems(&document, &display, &self.config.datasource_uids)
            {
                self.error(location, problem);
            }
        }
        Ok(referenced)
    }

    fn check_rules(&mut self, rule_files: &[RuleFile], allowed: &BTreeSet<String>) {
        for file in rule_files {
            for expression in self.patterns.rule_expressions(&file.content) {
                for problem in self
                    .patterns
                    .lint_expression(&expression.expression, allowed)
                {
                    self.findings.push(
                        Finding::error(NAME, file.path.as_str(), problem).at_line(expression.line),
                    );
                }
            }
        }
    }

    fn check_dashboard_coverage(
        &mut self,
        emitted: &BTreeMap<String, EmittedMetric>,
        referenced: &BTreeSet<String>,
    ) {
        if self.config.dashboards.is_empty() {
            return;
        }
        for (name, metric) in emitted {
            if promql::CATALOG_METRICS.contains(&name.as_str()) || referenced.contains(name) {
                continue;
            }
            self.findings.push(
                Finding::error(
                    NAME,
                    metric.path.as_str(),
                    format!("emitted metric `{name}` appears on no dashboard"),
                )
                .at_line(metric.line),
            );
        }
    }
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extensions.contains(&extension))
}

fn directory_files(directory: &Path, extensions: &[&str]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)
        .with_context(|| format!("failed to list {}", directory.display()))?
    {
        let path = entry?.path();
        if path.is_file() && has_extension(&path, extensions) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}
