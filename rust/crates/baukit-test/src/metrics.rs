use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::LazyLock,
};

use regex::Regex;

const HTTP_REQUESTS_TOTAL: &str = "http_requests_total";
const HTTP_DURATION_BUCKET: &str = "http_request_duration_seconds_bucket";
const HTTP_DURATION_SUM: &str = "http_request_duration_seconds_sum";
const HTTP_DURATION_COUNT: &str = "http_request_duration_seconds_count";
const HTTP_REQUESTS_IN_FLIGHT: &str = "http_requests_in_flight";
const BUILD_INFO: &str = "build_info";

static APP_PREFIXED_HTTP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^.+_http_requests?_.*$").expect("app-prefixed HTTP regex is valid")
});
static STATUS_CLASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[1-5][xX]{2}$").expect("status-class regex is valid"));
static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}$").expect("email-label regex is valid")
});
static UUID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|[^0-9a-f])[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}(?:$|[^0-9a-f])",
    )
    .expect("UUID-label regex is valid")
});

/// All violations found in Prometheus exposition text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetricsConformanceError {
    violations: Vec<String>,
}

impl MetricsConformanceError {
    /// Returns violations in deterministic discovery order.
    #[must_use]
    pub fn violations(&self) -> &[String] {
        &self.violations
    }
}

impl fmt::Display for MetricsConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "metrics conformance failed:")?;
        for violation in &self.violations {
            writeln!(formatter, "- {violation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for MetricsConformanceError {}

/// Checks Prometheus exposition against telemetry-spec sections 2 and 6.
///
/// Set `http_serving` when the process serves HTTP. In that mode all canonical
/// HTTP counter, histogram, and gauge samples are required with exact label
/// sets. `build_info` and forbidden-name/value checks always apply.
///
/// The bounded label lint deliberately uses syntax-oriented heuristics. It
/// catches query strings (`?`), doubled slashes, email-shaped values, and
/// RFC-style UUIDs. Build labels (`version`, `commit`, `rust_version`) are
/// allowlisted from email/UUID shape checks. It cannot prove that an ordinary
/// path segment, opaque identifier, token, or error message is safe; product
/// tests must still review label sources and cardinality.
pub fn check_metrics_conformance(
    exposition: impl AsRef<str>,
    http_serving: bool,
) -> Result<(), MetricsConformanceError> {
    let mut violations = Vec::new();
    let samples = parse_samples(exposition.as_ref(), &mut violations);

    check_forbidden_names(exposition.as_ref(), &samples, &mut violations);
    check_label_values(&samples, &mut violations);
    check_build_info(&samples, &mut violations);
    if http_serving {
        check_http_metrics(&samples, &mut violations);
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(MetricsConformanceError { violations })
    }
}

/// Panics when Prometheus exposition violates the telemetry contract.
///
/// # Panics
///
/// Panics with every discovered violation. See [`check_metrics_conformance`]
/// when the caller should handle failures.
#[track_caller]
pub fn assert_metrics_conformance(exposition: impl AsRef<str>, http_serving: bool) {
    if let Err(error) = check_metrics_conformance(exposition, http_serving) {
        panic!("{error}");
    }
}

#[derive(Debug)]
struct Sample {
    line: usize,
    name: String,
    labels: BTreeMap<String, String>,
}

fn parse_samples(exposition: &str, violations: &mut Vec<String>) -> Vec<Sample> {
    exposition
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line_number = index + 1;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }

            match parse_sample(line_number, line) {
                Ok(sample) => Some(sample),
                Err(problem) => {
                    violations.push(problem);
                    None
                }
            }
        })
        .collect()
}

fn parse_sample(line_number: usize, line: &str) -> Result<Sample, String> {
    let name_end = line
        .find(|character: char| character == '{' || character.is_ascii_whitespace())
        .unwrap_or(line.len());
    let name = &line[..name_end];
    if name.is_empty() {
        return Err(format!("line {line_number}: missing metric name"));
    }

    let labels = if line.as_bytes().get(name_end) == Some(&b'{') {
        let end = line.rfind('}').ok_or_else(|| {
            format!("line {line_number}: sample `{name}` has an unterminated label set")
        })?;
        parse_labels(line_number, name, &line[name_end + 1..end])?
    } else {
        BTreeMap::new()
    };

    Ok(Sample {
        line: line_number,
        name: name.to_owned(),
        labels,
    })
}

fn parse_labels(
    line_number: usize,
    metric_name: &str,
    input: &str,
) -> Result<BTreeMap<String, String>, String> {
    let mut labels = BTreeMap::new();
    for pair in split_label_pairs(input)
        .map_err(|problem| format!("line {line_number}: sample `{metric_name}` has {problem}"))?
    {
        let (name, raw_value) = pair.split_once('=').ok_or_else(|| {
            format!("line {line_number}: sample `{metric_name}` has malformed label `{pair}`")
        })?;
        let name = name.trim();
        let raw_value = raw_value.trim();
        if name.is_empty() || !raw_value.starts_with('"') || !raw_value.ends_with('"') {
            return Err(format!(
                "line {line_number}: sample `{metric_name}` has malformed label `{pair}`"
            ));
        }
        let value = unescape_label_value(&raw_value[1..raw_value.len() - 1]);
        if labels.insert(name.to_owned(), value).is_some() {
            return Err(format!(
                "line {line_number}: sample `{metric_name}` repeats label `{name}`"
            ));
        }
    }
    Ok(labels)
}

fn split_label_pairs(input: &str) -> Result<Vec<&str>, &'static str> {
    if input.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut pairs = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    let mut start = 0;
    for (index, character) in input.char_indices() {
        if escaped {
            escaped = false;
        } else if quoted && character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character == ',' && !quoted {
            pairs.push(input[start..index].trim());
            start = index + 1;
        }
    }
    if quoted || escaped {
        return Err("an unterminated quoted label value");
    }
    pairs.push(input[start..].trim());
    Ok(pairs)
}

fn unescape_label_value(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            match characters.next() {
                Some('n') => output.push('\n'),
                Some(next) => output.push(next),
                None => output.push('\\'),
            }
        } else {
            output.push(character);
        }
    }
    output
}

fn check_forbidden_names(exposition: &str, samples: &[Sample], violations: &mut Vec<String>) {
    let mut reported = BTreeSet::new();
    let names = samples
        .iter()
        .map(|sample| (sample.line, sample.name.as_str()))
        .chain(exposition.lines().enumerate().filter_map(|(index, line)| {
            let line = line.trim();
            let declaration = line
                .strip_prefix("# HELP ")
                .or_else(|| line.strip_prefix("# TYPE "))?;
            declaration
                .split_ascii_whitespace()
                .next()
                .map(|name| (index + 1, name))
        }));
    for (line, name) in names {
        let problem = if name.starts_with("http_requests_duration_seconds") {
            Some(format!(
                "forbidden plural HTTP duration metric `{name}` appears on line {line}"
            ))
        } else if APP_PREFIXED_HTTP.is_match(name) {
            Some(format!(
                "forbidden app-prefixed HTTP metric `{name}` appears on line {line}"
            ))
        } else {
            None
        };
        if let Some(problem) = problem
            && reported.insert(name.to_owned())
        {
            violations.push(problem);
        }
    }
}

fn check_label_values(samples: &[Sample], violations: &mut Vec<String>) {
    const IDENTITY_SHAPE_ALLOWLIST: &[&str] = &["version", "commit", "rust_version"];

    for sample in samples {
        for (label, value) in &sample.labels {
            if label == "status" && STATUS_CLASS.is_match(value) {
                violations.push(format!(
                    "line {}: metric `{}` uses forbidden status-class value `{value}`",
                    sample.line, sample.name
                ));
            }
            if value.contains('?') {
                violations.push(format!(
                    "line {}: metric `{}` label `{label}` contains `?` and looks like a raw URL/query",
                    sample.line, sample.name
                ));
            }
            if value.contains("//") {
                violations.push(format!(
                    "line {}: metric `{}` label `{label}` contains `//` and looks like a raw URL/path",
                    sample.line, sample.name
                ));
            }
            if !IDENTITY_SHAPE_ALLOWLIST.contains(&label.as_str()) {
                if EMAIL.is_match(value) {
                    violations.push(format!(
                        "line {}: metric `{}` label `{label}` contains an email-shaped value",
                        sample.line, sample.name
                    ));
                }
                if UUID.is_match(value) {
                    violations.push(format!(
                        "line {}: metric `{}` label `{label}` contains a UUID-shaped value",
                        sample.line, sample.name
                    ));
                }
            }
        }
    }
}

fn check_build_info(samples: &[Sample], violations: &mut Vec<String>) {
    require_metric(samples, BUILD_INFO, violations);
    check_exact_labels(
        samples,
        BUILD_INFO,
        &["commit", "rust_version", "version"],
        violations,
    );
}

fn check_http_metrics(samples: &[Sample], violations: &mut Vec<String>) {
    for metric in [
        HTTP_REQUESTS_TOTAL,
        HTTP_DURATION_BUCKET,
        HTTP_DURATION_SUM,
        HTTP_DURATION_COUNT,
        HTTP_REQUESTS_IN_FLIGHT,
    ] {
        require_metric(samples, metric, violations);
    }

    check_exact_labels(
        samples,
        HTTP_REQUESTS_TOTAL,
        &["method", "route", "status"],
        violations,
    );
    check_exact_labels(
        samples,
        HTTP_DURATION_BUCKET,
        &["le", "method", "route", "status"],
        violations,
    );
    for metric in [HTTP_DURATION_SUM, HTTP_DURATION_COUNT] {
        check_exact_labels(samples, metric, &["method", "route", "status"], violations);
    }
    check_exact_labels(
        samples,
        HTTP_REQUESTS_IN_FLIGHT,
        &["method", "route"],
        violations,
    );
}

fn require_metric(samples: &[Sample], name: &str, violations: &mut Vec<String>) {
    if !samples.iter().any(|sample| sample.name == name) {
        violations.push(format!("required metric sample `{name}` is missing"));
    }
}

fn check_exact_labels(
    samples: &[Sample],
    metric_name: &str,
    expected: &[&str],
    violations: &mut Vec<String>,
) {
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    for sample in samples.iter().filter(|sample| sample.name == metric_name) {
        let actual = sample
            .labels
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if actual != expected {
            violations.push(format!(
                "line {}: metric `{metric_name}` labels are {actual:?}; expected exactly {expected:?}",
                sample.line
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{Router, body::Body, http::Request, routing::get};
    use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
    use tower::ServiceExt as _;

    use super::*;

    #[tokio::test]
    async fn accepts_metrics_from_baukit_http_and_the_local_recorder() {
        let recorder = PrometheusBuilder::new()
            .set_buckets_for_metric(
                Matcher::Full("http_request_duration_seconds".to_owned()),
                baukit_telemetry::HTTP_DURATION_BUCKETS,
            )
            .expect("spec buckets are valid")
            .build_recorder();
        let handle = recorder.handle();
        metrics::set_global_recorder(recorder).expect("this is the only recorder-owning test");
        metrics::gauge!(
            "build_info",
            "version" => "1.0.0",
            "commit" => "abc123",
            "rust_version" => "1.95.0"
        )
        .set(1.0);

        let app = baukit_http::layers(
            Router::new().route("/widgets/{id}", get(|| async { "ok" })),
            baukit_http::HttpOptions::default(),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/widgets/42")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router request succeeds");
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let rendered = handle.render();
        check_metrics_conformance(&rendered, true).expect("Baukit metrics conform");
        for boundary in [
            "0.005", "0.01", "0.025", "0.05", "0.1", "0.25", "0.5", "1", "2.5", "5", "10",
        ] {
            assert!(
                rendered.contains(&format!(
                    "http_request_duration_seconds_bucket{{le=\"{boundary}\""
                )) || rendered.contains(&format!("le=\"{boundary}\"")),
                "missing spec bucket le=\"{boundary}\" in:\n{rendered}"
            );
        }
    }

    #[test]
    fn rejects_plural_status_class_and_app_prefix() {
        let exposition = r#"
build_info{version="1",commit="abc",rust_version="1.95"} 1
http_requests_duration_seconds_bucket{method="GET",route="/x",status="2xx",le="1"} 1
orders_http_requests_total{method="GET",route="/x",status="200"} 1
"#;
        let error =
            check_metrics_conformance(exposition, false).expect_err("must reject violations");
        let rendered = error.to_string();
        assert!(rendered.contains("plural HTTP duration"), "{rendered}");
        assert!(rendered.contains("status-class"), "{rendered}");
        assert!(rendered.contains("app-prefixed"), "{rendered}");
    }

    #[test]
    fn label_lint_flags_bounded_high_signal_shapes() {
        let exposition = r#"
build_info{version="1",commit="abc",rust_version="1.95"} 1
domain_total{kind="person@example.com",subject="550e8400-e29b-41d4-a716-446655440000",route="/users?id=1"} 1
"#;
        let error = check_metrics_conformance(exposition, false).expect_err("must reject labels");
        let rendered = error.to_string();
        assert!(rendered.contains("email-shaped"), "{rendered}");
        assert!(rendered.contains("UUID-shaped"), "{rendered}");
        assert!(rendered.contains("contains `?`"), "{rendered}");
    }

    #[test]
    fn duration_bucket_constants_cannot_drift() {
        assert_eq!(
            baukit_telemetry::HTTP_DURATION_BUCKETS,
            baukit_http::DURATION_BUCKETS
        );
    }
}
