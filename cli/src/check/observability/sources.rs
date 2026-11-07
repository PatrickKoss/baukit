use std::collections::BTreeMap;

use anyhow::Result;
use regex::Regex;

use super::promql::{CATALOG_METRICS, is_metric_name};

const HISTOGRAM_MARKER: &str = "histogram";
const COUNTER_SUFFIX: &str = "_total";
const TEST_MODULE_MARKER: &str = "#[cfg(test)]";
const TEST_DIRECTORIES: &[&str] = &["tests", "benches", "examples"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}

/// A metric a product emits, with where it was first seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmittedMetric {
    pub kind: MetricKind,
    pub path: String,
    pub line: usize,
}

/// Finds `counter!`, `gauge!` and `histogram!` calls (and their `describe_` forms) with a literal name.
pub struct MetricScanner {
    call: Regex,
}

impl MetricScanner {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call: Regex::new(
                r#"\b(?:describe_)?(counter|gauge|histogram)!\(\s*"([A-Za-z_:][A-Za-z0-9_:]*)""#,
            )?,
        })
    }

    /// Scans production code: comment lines and everything from the first `#[cfg(test)]` are skipped.
    pub fn scan(&self, path: &str, text: &str, metrics: &mut BTreeMap<String, EmittedMetric>) {
        let text = production_code(text);
        for captures in self.call.captures_iter(&text) {
            let (Some(kind), Some(name)) = (captures.get(1), captures.get(2)) else {
                continue;
            };
            let kind = match kind.as_str() {
                "counter" => MetricKind::Counter,
                "gauge" => MetricKind::Gauge,
                _ => MetricKind::Histogram,
            };
            let line = text
                .get(..name.start())
                .map_or(1, |before| before.matches('\n').count() + 1);
            metrics
                .entry(name.as_str().to_owned())
                .or_insert_with(|| EmittedMetric {
                    kind,
                    path: path.to_owned(),
                    line,
                });
        }
    }
}

/// The lines before the first `#[cfg(test)]`, with comment lines blanked so line numbers stay.
fn production_code(text: &str) -> String {
    text.lines()
        .take_while(|line| line.trim() != TEST_MODULE_MARKER)
        .map(|line| {
            if line.trim_start().starts_with("//") {
                ""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether a Rust file holds only tests, by the usual file and directory names.
pub fn is_test_file(relative_path: &str) -> bool {
    let mut components = relative_path.split('/');
    let file = components.next_back().unwrap_or_default();
    components.any(|directory| TEST_DIRECTORIES.contains(&directory))
        || file == "tests.rs"
        || file.ends_with("_tests.rs")
        || file.ends_with("_test.rs")
}

/// Naming problems of a product metric; catalog metrics are baukit's and pass.
pub fn naming_problems(name: &str, kind: MetricKind, prefix: Option<&str>) -> Vec<String> {
    if CATALOG_METRICS.contains(&name) {
        return Vec::new();
    }
    let mut problems = Vec::new();
    if let Some(prefix) = prefix
        && !name.starts_with(prefix)
    {
        problems.push(format!(
            "product metric `{name}` must start with `{prefix}`"
        ));
    }
    match kind {
        MetricKind::Counter if !name.ends_with(COUNTER_SUFFIX) => {
            problems.push(format!("counter `{name}` must end with `{COUNTER_SUFFIX}`"));
        }
        MetricKind::Gauge if name.ends_with(COUNTER_SUFFIX) => {
            problems.push(format!(
                "gauge `{name}` must not end with `{COUNTER_SUFFIX}`"
            ));
        }
        MetricKind::Histogram if name.ends_with(COUNTER_SUFFIX) => {
            problems.push(format!(
                "histogram `{name}` must not end with `{COUNTER_SUFFIX}`"
            ));
        }
        _ => {}
    }
    if name.contains("http_requests_duration_seconds") {
        problems.push(format!(
            "`{name}` uses the forbidden plural HTTP duration name"
        ));
    }
    problems
}

/// A product metric list in check-metric-names.py's `--allowlist` format.
#[derive(Debug, Default)]
pub struct MetricList {
    pub names: Vec<String>,
    pub histograms: Vec<String>,
    /// `(line, message)` for lines the Python linter rejects as a configuration error.
    pub problems: Vec<(usize, String)>,
}

pub fn parse_metric_list(text: &str) -> MetricList {
    let mut list = MetricList::default();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        let content = line.split_once('#').map_or(line, |(content, _)| content);
        let fields: Vec<&str> = content.split_whitespace().collect();
        let Some((name, kind)) = fields.split_first() else {
            continue;
        };
        if !is_metric_name(name) {
            list.problems
                .push((line_number, format!("invalid metric name '{name}'")));
            continue;
        }
        if !matches!(kind, [] | [HISTOGRAM_MARKER]) {
            list.problems.push((
                line_number,
                format!("expected a metric name and optional '{HISTOGRAM_MARKER}'"),
            ));
            continue;
        }
        if list.names.iter().any(|known| known == name) {
            list.problems
                .push((line_number, format!("duplicate metric name '{name}'")));
            continue;
        }
        list.names.push((*name).to_owned());
        if !kind.is_empty() {
            list.histograms.push((*name).to_owned());
        }
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanner_finds_multiline_macro_calls() -> Result<()> {
        let source = "fn f() {\n    metrics::counter!(\n        \"shop_orders_total\",\n    ).increment(1);\n    describe_histogram!(\"shop_latency_seconds\", \"x\");\n    counter!(NAME);\n}\n";
        let mut metrics = BTreeMap::new();
        MetricScanner::new()?.scan("src/lib.rs", source, &mut metrics);
        let found: Vec<(&str, MetricKind, usize)> = metrics
            .iter()
            .map(|(name, metric)| (name.as_str(), metric.kind, metric.line))
            .collect();
        assert_eq!(
            found,
            vec![
                ("shop_latency_seconds", MetricKind::Histogram, 5),
                ("shop_orders_total", MetricKind::Counter, 3),
            ]
        );
        Ok(())
    }

    #[test]
    fn scanner_skips_comments_and_tests() -> Result<()> {
        let source = "// counter!(\"commented_total\")\nfn f() { counter!(\"live_total\"); }\n#[cfg(test)]\nmod tests { fn t() { counter!(\"test_total\"); } }\n";
        let mut metrics = BTreeMap::new();
        MetricScanner::new()?.scan("src/lib.rs", source, &mut metrics);
        assert_eq!(metrics.keys().collect::<Vec<_>>(), vec!["live_total"]);
        assert!(is_test_file("backend/tests/a.rs"));
        assert!(is_test_file("src/foo_tests.rs"));
        assert!(!is_test_file("src/testsuite.rs"));
        Ok(())
    }

    #[test]
    fn naming_follows_the_prometheus_conventions() {
        assert!(
            naming_problems("shop_orders_total", MetricKind::Counter, Some("shop_")).is_empty()
        );
        assert!(
            naming_problems("http_requests_total", MetricKind::Counter, Some("shop_")).is_empty()
        );
        assert_eq!(
            naming_problems("orders", MetricKind::Counter, Some("shop_")),
            vec![
                "product metric `orders` must start with `shop_`",
                "counter `orders` must end with `_total`",
            ]
        );
        assert_eq!(
            naming_problems("shop_queue_total", MetricKind::Gauge, None),
            vec!["gauge `shop_queue_total` must not end with `_total`"]
        );
        assert!(naming_problems("shop_batch_size", MetricKind::Histogram, None).is_empty());
        assert_eq!(
            naming_problems("shop_batches_total", MetricKind::Histogram, None),
            vec!["histogram `shop_batches_total` must not end with `_total`"]
        );
    }

    #[test]
    fn metric_list_matches_the_python_allowlist_format() {
        let list = parse_metric_list(
            "# c\nshop_orders_total\nshop_latency_seconds histogram # x\n9bad\nshop_orders_total\nshop_x gauge\n",
        );
        assert_eq!(
            list.names,
            vec!["shop_orders_total", "shop_latency_seconds"]
        );
        assert_eq!(list.histograms, vec!["shop_latency_seconds"]);
        assert_eq!(
            list.problems,
            vec![
                (4, "invalid metric name '9bad'".to_owned()),
                (5, "duplicate metric name 'shop_orders_total'".to_owned()),
                (
                    6,
                    "expected a metric name and optional 'histogram'".to_owned()
                ),
            ]
        );
    }
}
