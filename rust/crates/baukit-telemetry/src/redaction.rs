//! One redaction pass shared by the stdout JSON logs and the OTLP span export.

use std::{fmt, sync::LazyLock};

use opentelemetry::{KeyValue, Value as OtelValue};
use opentelemetry_sdk::{
    Resource,
    error::OTelSdkResult,
    trace::{SpanData, SpanExporter},
};
use regex::Regex;
use serde_json::Value;

pub(crate) const REDACTED: &str = "[REDACTED]";

static EMAIL_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| scrubber(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b"));
static BEARER_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| scrubber(r"(?i)\bbearer\s+[A-Z0-9._~+/=-]+"));
static SECRET_ASSIGNMENT_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    scrubber(
        r#"(?i)\b(authorization|cookie|(?:access_|refresh_|id_)?token|password|(?:client_)?secret|api[_-]?key|private[_-]?key|session(?:[_-]?id)?|credentials?|code_verifier)\s*[:=]\s*(?:"[^"]*"|'[^']*'|[^\s,;&]+)"#,
    )
});
static QUERY_CODE_PATTERN: LazyLock<Regex> = LazyLock::new(|| scrubber(r"(?i)([?&]code=)[^&\s#]+"));

/// Substrings of a normalized field name that mark it sensitive.
const SENSITIVE_SUBSTRINGS: [&str; 15] = [
    "authorization",
    "cookie",
    "token",
    "password",
    "email",
    "requestbody",
    "responsebody",
    "providerpayload",
    "secret",
    "credential",
    "apikey",
    "privatekey",
    "authcode",
    "oauthcode",
    "codeverifier",
];

/// Whole words of a field name that mark it sensitive. Bare `code` is not one:
/// Baukit logs stable error codes under `code`; OAuth codes use `auth_code`.
const SENSITIVE_WORDS: [&str; 3] = ["key", "session", "sessions"];

#[expect(
    clippy::expect_used,
    reason = "scrubber patterns are string literals in this file, compiled by every redaction test"
)]
fn scrubber(pattern: &str) -> Regex {
    Regex::new(pattern).expect("scrubber patterns are valid regexes")
}

pub(crate) fn scrub_text(value: &str) -> String {
    let value = EMAIL_PATTERN.replace_all(value, REDACTED);
    let value = BEARER_PATTERN.replace_all(&value, "Bearer [REDACTED]");
    let value = QUERY_CODE_PATTERN.replace_all(&value, "${1}[REDACTED]");
    SECRET_ASSIGNMENT_PATTERN
        .replace_all(&value, "$1=[REDACTED]")
        .into_owned()
}

pub(crate) fn is_sensitive_field(name: &str) -> bool {
    let normalized = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect::<String>();
    SENSITIVE_SUBSTRINGS
        .iter()
        .any(|sensitive| normalized.contains(sensitive))
        || words(name).any(|word| SENSITIVE_WORDS.contains(&word.as_str()))
}

fn words(name: &str) -> impl Iterator<Item = String> + '_ {
    let mut spaced = String::with_capacity(name.len() * 2);
    let mut previous_lower = false;
    for character in name.chars() {
        if character.is_ascii_uppercase() && previous_lower {
            spaced.push(' ');
        }
        previous_lower = character.is_ascii_lowercase() || character.is_ascii_digit();
        spaced.push(if character.is_ascii_alphanumeric() {
            character.to_ascii_lowercase()
        } else {
            ' '
        });
    }
    spaced
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .into_iter()
}

pub(crate) fn redact_json(name: &str, value: Value) -> Value {
    if is_sensitive_field(name) {
        return Value::String(REDACTED.to_owned());
    }
    match value {
        Value::String(text) => Value::String(scrub_text(&text)),
        other => other,
    }
}

fn redact_otel(attribute: KeyValue) -> KeyValue {
    let KeyValue { key, value, .. } = attribute;
    if is_sensitive_field(key.as_str()) {
        return KeyValue::new(key, REDACTED);
    }
    let value = match value {
        OtelValue::String(text) => OtelValue::from(scrub_text(text.as_str())),
        other => other,
    };
    KeyValue::new(key, value)
}

fn redact_span(mut span: SpanData) -> SpanData {
    span.attributes = span.attributes.into_iter().map(redact_otel).collect();
    for event in &mut span.events.events {
        event.name = scrub_text(&event.name).into();
        event.attributes = std::mem::take(&mut event.attributes)
            .into_iter()
            .map(redact_otel)
            .collect();
    }
    span
}

/// Applies the log redaction rules to every span before the wrapped exporter sees it.
pub(crate) struct RedactingExporter<E>(E);

impl<E> RedactingExporter<E> {
    pub(crate) const fn new(inner: E) -> Self {
        Self(inner)
    }
}

impl<E> fmt::Debug for RedactingExporter<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RedactingExporter")
    }
}

impl<E: SpanExporter> SpanExporter for RedactingExporter<E> {
    fn export(
        &self,
        batch: Vec<SpanData>,
    ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        self.0.export(batch.into_iter().map(redact_span).collect())
    }

    fn shutdown_with_timeout(&self, timeout: std::time::Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.0.set_resource(resource);
    }
}

#[cfg(test)]
mod tests {
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use tracing_subscriber::{Registry, layer::SubscriberExt as _};

    use super::*;

    const SENSITIVE_NAMES: [&str; 12] = [
        "secret",
        "key",
        "auth_code",
        "session",
        "credential",
        "client_secret",
        "api_key",
        "apiKey",
        "session_id",
        "password",
        "refresh_token",
        "authorization",
    ];

    #[test]
    fn sensitive_names_cover_secrets_but_not_error_codes() {
        for name in SENSITIVE_NAMES {
            assert!(is_sensitive_field(name), "{name}");
        }
        for name in [
            "code",
            "http.response.status_code",
            "request_id",
            "keycloak_realm",
            "job_id",
        ] {
            assert!(!is_sensitive_field(name), "{name}");
        }
    }

    #[test]
    fn assignments_and_query_codes_are_scrubbed() {
        for (input, expected) in [
            ("client_secret=abc123 next", "client_secret=[REDACTED] next"),
            ("api-key: k1", "api-key=[REDACTED]"),
            ("session_id=s1;", "session_id=[REDACTED];"),
            ("credentials='x y'", "credentials=[REDACTED]"),
            (
                "GET /cb?code=xyz&state=1",
                "GET /cb?code=[REDACTED]&state=1",
            ),
        ] {
            assert_eq!(scrub_text(input), expected, "{input}");
        }
    }

    #[test]
    fn otlp_export_redacts_every_sensitive_span_and_event_field() {
        let memory = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(RedactingExporter::new(memory.clone()))
            .build();
        let subscriber = Registry::default()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("redaction-test")));
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(
                "request",
                secret = "s-1",
                key = "s-2",
                auth_code = "s-3",
                session = "s-4",
                credential = "s-5",
                client_secret = "s-6",
                api_key = "s-7",
                session_id = "s-8",
                password = "s-9",
                refresh_token = "s-10",
                authorization = "s-11",
                note = "mail private@example.com",
                code = "internal_error",
            );
            let _entered = span.enter();
            tracing::info!(client_secret = "s-12", "callback ?code=s-13 done");
        });
        provider.force_flush().expect("flush");
        let spans = memory.get_finished_spans().expect("spans");
        let span = spans.first().expect("one span");
        let attribute = |name: &str| {
            span.attributes
                .iter()
                .find(|kv| kv.key.as_str() == name)
                .map(|kv| kv.value.to_string())
        };
        for name in SENSITIVE_NAMES.iter().filter(|name| **name != "apiKey") {
            assert_eq!(attribute(name).as_deref(), Some(REDACTED), "{name}");
        }
        assert_eq!(attribute("note").as_deref(), Some("mail [REDACTED]"));
        assert_eq!(attribute("code").as_deref(), Some("internal_error"));
        let rendered = format!("{span:?}");
        assert!(!rendered.contains("s-1"), "{rendered}");
        assert!(!rendered.contains("private@example.com"), "{rendered}");
        provider.shutdown().expect("shutdown");
    }
}
