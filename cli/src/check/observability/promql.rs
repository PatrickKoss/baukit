//! Port of the expression checks in `deploy/observability/lint/check-metric-names.py`, which stays
//! the source of truth. `cli/tests/check_observability.rs` runs both over the same fixtures.

use std::collections::BTreeSet;

use anyhow::Result;
use regex::Regex;

/// Baukit's metric catalog, `SPEC_METRICS` in check-metric-names.py.
pub const CATALOG_METRICS: &[&str] = &[
    "http_requests_total",
    "mcp_requests_total",
    "mcp_authentication_policy_decisions_total",
    "mcp_authentication_policy_duration_seconds",
    "mcp_introspection_cache_requests_total",
    "mcp_request_duration_seconds",
    "http_request_duration_seconds",
    "http_requests_in_flight",
    "http_rate_limit_decisions_total",
    "fixed_window_amount_budget_decisions_total",
    "build_info",
    "db_pool_connections_max",
    "db_pool_connections_idle",
    "db_pool_connections_in_use",
    "db_pool_acquire_duration_seconds",
    "db_pool_acquire_timeouts_total",
    "worker_job_runs_total",
    "worker_job_duration_seconds",
    "worker_queue_oldest_age_seconds",
];

/// `HISTOGRAM_METRICS` in check-metric-names.py.
pub const CATALOG_HISTOGRAMS: &[&str] = &[
    "mcp_authentication_policy_duration_seconds",
    "mcp_request_duration_seconds",
    "http_request_duration_seconds",
    "db_pool_acquire_duration_seconds",
    "worker_job_duration_seconds",
];

pub const PROMETHEUS_BUILTINS: &[&str] = &["up"];
pub const HISTOGRAM_SUFFIXES: &[&str] = &["_bucket", "_count", "_sum"];

const PROMQL_KEYWORDS: &[&str] = &[
    "and",
    "bool",
    "end",
    "group_left",
    "group_right",
    "ignoring",
    "json",
    "offset",
    "on",
    "or",
    "start",
    "unless",
];
const WORKER_JOB_LABELS: &[&str] = &["job", "exported_job"];
const BLOCK_SCALARS: &[&str] = &["", "|", ">", "|-", ">-", "|+", ">+"];

/// One `expr:` in a rule file, with its 1-based line and the rule it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleExpression {
    pub line: usize,
    pub rule: Option<RuleName>,
    pub expression: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleName {
    pub kind: String,
    pub name: String,
}

/// The compiled patterns of check-metric-names.py.
pub struct Patterns {
    quoted: Regex,
    braces: Regex,
    brackets: Regex,
    grouping: Regex,
    forbidden_plural: Regex,
    loki_service_name: Regex,
    status_class: Regex,
    record_name: Regex,
    rule_name: Regex,
    expr_line: Regex,
}

impl Patterns {
    pub fn new() -> Result<Self> {
        Ok(Self {
            quoted: Regex::new(r#""(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'"#)?,
            braces: Regex::new(r"\{[^}]*\}")?,
            brackets: Regex::new(r"\[[^\]]*\]")?,
            grouping: Regex::new(
                r"\b(?:by|without|on|ignoring|group_left|group_right)\s*\(([^)]*)\)",
            )?,
            forbidden_plural: Regex::new(r"\bhttp_requests_duration_seconds(?:_[a-z]+)?\b")?,
            loki_service_name: Regex::new(r"\{[^}\n]*\bservice_name\s*(?:=|!=|=~|!~)")?,
            status_class: Regex::new(r#"\bstatus\s*(?:=|!=|=~|!~)\s*["'](?:[1-5][xX]{2})["']"#)?,
            record_name: Regex::new(r"(?m)^\s*-?\s*record:\s*([A-Za-z_:][A-Za-z0-9_:]*)\s*$")?,
            rule_name: Regex::new(r"^\s*-?\s*(record|alert):\s*(\S+)\s*$")?,
            expr_line: Regex::new(r"^(\s*)expr:\s*(.*)$")?,
        })
    }

    /// Metric identifiers in an expression, excluding functions, keywords and labels.
    pub fn metric_references(&self, expression: &str) -> BTreeSet<String> {
        let mut references: BTreeSet<String> = identifiers(expression)
            .into_iter()
            .filter(|(_, rest)| rest.trim_start().starts_with(['{', '[']))
            .map(|(name, _)| name.to_owned())
            .collect();
        let sanitized = self.quoted.replace_all(expression, " ");
        let sanitized = self.braces.replace_all(&sanitized, " ");
        let sanitized = self.brackets.replace_all(&sanitized, " ");
        let sanitized = self.grouping.replace_all(&sanitized, " ");
        for (name, rest) in identifiers(&sanitized) {
            if PROMQL_KEYWORDS.contains(&name) || rest.trim_start().starts_with('(') {
                continue;
            }
            references.insert(name.to_owned());
        }
        references
    }

    /// Problems in one expression, in the order and wording of check-metric-names.py.
    pub fn lint_expression(&self, expression: &str, allowed: &BTreeSet<String>) -> Vec<String> {
        let mut problems = Vec::new();
        if self.forbidden_plural.is_match(expression) {
            problems.push("forbidden plural HTTP duration metric".to_owned());
        }
        if self.loki_service_name.is_match(expression) {
            problems.push("service_name is forbidden in Loki label selectors".to_owned());
        }
        if self.status_class.is_match(expression) {
            problems.push("status classes are forbidden metric label values".to_owned());
        }
        let references = self.metric_references(expression);
        if references
            .iter()
            .any(|metric| metric.contains("worker_job_"))
        {
            let unquoted = self.quoted.replace_all(expression, " ");
            for captures in self.grouping.captures_iter(&unquoted) {
                let labels = captures.get(1).map_or("", |labels| labels.as_str());
                if labels
                    .split(',')
                    .any(|label| WORKER_JOB_LABELS.contains(&label.trim()))
                {
                    problems.push("worker handler types must use job_kind, not job".to_owned());
                }
            }
        }
        for metric in references.difference(allowed) {
            problems.push(format!("unknown metric '{metric}'"));
        }
        problems
    }

    /// Inline and block-scalar `expr:` values of a Prometheus rule file.
    pub fn rule_expressions(&self, content: &str) -> Vec<RuleExpression> {
        let lines: Vec<&str> = content.lines().collect();
        let mut expressions = Vec::new();
        let mut rule = None;
        let mut index = 0;
        while let Some(line) = lines.get(index) {
            let line_number = index + 1;
            index += 1;
            if let Some(captures) = self.rule_name.captures(line)
                && let (Some(kind), Some(name)) = (captures.get(1), captures.get(2))
            {
                rule = Some(RuleName {
                    kind: kind.as_str().to_owned(),
                    name: name.as_str().to_owned(),
                });
            }
            let Some(captures) = self.expr_line.captures(line) else {
                continue;
            };
            let indentation = captures.get(1).map_or(0, |indent| indent.as_str().len());
            let remainder = captures.get(2).map_or("", |rest| rest.as_str()).trim();
            let expression = if BLOCK_SCALARS.contains(&remainder) {
                let (block, next) = block_scalar(&lines, index, indentation);
                index = next;
                block
            } else {
                unquote(remainder).to_owned()
            };
            expressions.push(RuleExpression {
                line: line_number,
                rule: rule.clone(),
                expression,
            });
        }
        expressions
    }

    /// Names of the recording rules a rule file defines.
    pub fn recording_rule_names(&self, content: &str) -> Vec<String> {
        self.record_name
            .captures_iter(content)
            .filter_map(|captures| captures.get(1))
            .map(|name| name.as_str().to_owned())
            .collect()
    }
}

/// The indented lines after a block-scalar `expr:`, and the index of the first line after them.
fn block_scalar(lines: &[&str], start: usize, indentation: usize) -> (String, usize) {
    let mut block = Vec::new();
    let mut index = start;
    while let Some(line) = lines.get(index) {
        let trimmed = line.trim();
        if !trimmed.is_empty() && line.len() - line.trim_start().len() <= indentation {
            break;
        }
        if !trimmed.is_empty() {
            block.push(trimmed);
        }
        index += 1;
    }
    (block.join("\n"), index)
}

const fn is_identifier_start(character: char) -> bool {
    character.is_ascii_alphabetic() || character == '_' || character == ':'
}

const fn is_identifier_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || character == ':'
}

/// Whether `name` is a valid Prometheus metric name.
pub fn is_metric_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters.next().is_some_and(is_identifier_start) && characters.all(is_identifier_char)
}

/// Identifiers that begin a run of identifier characters, each with the text after it.
fn identifiers(text: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(is_identifier_char) {
        let (_, from_start) = rest.split_at(start);
        let end = from_start
            .find(|character: char| !is_identifier_char(character))
            .unwrap_or(from_start.len());
        let (token, after) = from_start.split_at(end);
        if token.starts_with(is_identifier_start) {
            found.push((token, after));
        }
        rest = after;
    }
    found
}

fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

/// Every series name a product may query: the catalog, product metrics and recording rules.
pub fn allowed_metric_names<'a>(
    product_metrics: impl IntoIterator<Item = &'a str>,
    product_histograms: impl IntoIterator<Item = &'a str>,
    recordings: impl IntoIterator<Item = &'a str>,
) -> BTreeSet<String> {
    let mut allowed: BTreeSet<String> = CATALOG_METRICS
        .iter()
        .chain(PROMETHEUS_BUILTINS)
        .map(|name| (*name).to_owned())
        .collect();
    allowed.extend(product_metrics.into_iter().map(str::to_owned));
    for histogram in CATALOG_HISTOGRAMS.iter().copied().chain(product_histograms) {
        allowed.extend(
            HISTOGRAM_SUFFIXES
                .iter()
                .map(|suffix| format!("{histogram}{suffix}")),
        );
    }
    allowed.extend(recordings.into_iter().map(str::to_owned));
    allowed
}

/// A histogram series name without its `_bucket`, `_count` or `_sum` suffix.
pub fn base_metric(name: &str) -> &str {
    HISTOGRAM_SUFFIXES
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
        .unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn references(expression: &str) -> Result<Vec<String>> {
        Ok(Patterns::new()?
            .metric_references(expression)
            .into_iter()
            .collect())
    }

    const PYTHON_LINTER: &str =
        include_str!("../../../../deploy/observability/lint/check-metric-names.py");

    fn python_names(start: &str, end: char) -> Vec<String> {
        let block = PYTHON_LINTER
            .split_once(start)
            .and_then(|(_, rest)| rest.split_once(end))
            .map_or("", |(block, _)| block);
        block
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn catalog_matches_check_metric_names_py() {
        assert_eq!(python_names("SPEC_METRICS = (", ')'), CATALOG_METRICS);
        assert_eq!(
            python_names("HISTOGRAM_METRICS = {", '}'),
            CATALOG_HISTOGRAMS
        );
        assert_eq!(
            python_names("PROMETHEUS_BUILTINS = {", '}'),
            PROMETHEUS_BUILTINS
        );
    }

    #[test]
    fn references_skip_functions_labels_and_durations() -> Result<()> {
        assert_eq!(
            references(
                r#"sum by (route) (rate(http_requests_total{status=~"5.."}[5m])) / on(route) group_left up"#
            )?,
            vec!["http_requests_total", "up"]
        );
        Ok(())
    }

    #[test]
    fn worker_job_label_is_rejected() -> Result<()> {
        let allowed = allowed_metric_names([], [], []);
        assert_eq!(
            Patterns::new()?.lint_expression("sum by (job) (worker_job_runs_total)", &allowed),
            vec!["worker handler types must use job_kind, not job"]
        );
        Ok(())
    }

    #[test]
    fn unknown_and_forbidden_names_are_reported() -> Result<()> {
        let allowed = allowed_metric_names(["shop_orders_total"], [], []);
        let problems = Patterns::new()?.lint_expression(
            r#"rate(http_requests_duration_seconds_bucket{status="5xx"}[5m]) + shop_orders_total + nope_total"#,
            &allowed,
        );
        assert_eq!(
            problems,
            vec![
                "forbidden plural HTTP duration metric",
                "status classes are forbidden metric label values",
                "unknown metric 'http_requests_duration_seconds_bucket'",
                "unknown metric 'nope_total'",
            ]
        );
        Ok(())
    }

    #[test]
    fn rule_expressions_read_inline_and_block_values() -> Result<()> {
        let patterns = Patterns::new()?;
        let content = "groups:\n  - name: a\n    rules:\n      - record: job:x:rate5m\n        expr: 'rate(x[5m])'\n      - alert: High\n        expr: |\n          sum(\n            y)\n        for: 5m\n";
        let found = patterns.rule_expressions(content);
        let summary: Vec<(usize, &str, Option<&str>)> = found
            .iter()
            .map(|rule| {
                (
                    rule.line,
                    rule.expression.as_str(),
                    rule.rule.as_ref().map(|name| name.name.as_str()),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (5, "rate(x[5m])", Some("job:x:rate5m")),
                (7, "sum(\ny)", Some("High")),
            ]
        );
        assert_eq!(patterns.recording_rule_names(content), vec!["job:x:rate5m"]);
        Ok(())
    }
}
