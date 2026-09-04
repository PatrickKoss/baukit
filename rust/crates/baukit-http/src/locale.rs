use std::collections::HashSet;

use axum::{
    extract::{FromRef, FromRequestParts},
    http::{header, request::Parts},
    response::{IntoResponse, Response},
};
use thiserror::Error;

use crate::ApiError;

/// Maximum raw query length inspected by [`RequestLocale`].
pub const MAX_LOCALE_QUERY_BYTES: usize = 2_048;
/// Maximum combined `Accept-Language` header length inspected by [`RequestLocale`].
pub const MAX_ACCEPT_LANGUAGE_BYTES: usize = 1_024;
/// Maximum number of locales in a [`RequestLocaleConfig`].
pub const MAX_SUPPORTED_LOCALES: usize = 64;

const MAX_LOCALE_TAG_BYTES: usize = 64;
const MAX_QUERY_PARAMETER_BYTES: usize = 32;

/// Controls whether a decoded query parameter overrides `Accept-Language`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocaleQueryOverride {
    /// Ignore query parameters when selecting a locale.
    Disabled,
    /// Use the named query parameter before `Accept-Language`.
    Parameter(String),
}

impl LocaleQueryOverride {
    /// Creates an enabled query override after validating its parameter name.
    pub fn parameter(name: impl Into<String>) -> Result<Self, RequestLocaleConfigError> {
        let name = name.into();
        if name.is_empty()
            || name.len() > MAX_QUERY_PARAMETER_BYTES
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(RequestLocaleConfigError::InvalidQueryParameter);
        }
        Ok(Self::Parameter(name))
    }
}

/// Invalid request-locale configuration.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RequestLocaleConfigError {
    /// No supported locale was supplied.
    #[error("at least one supported locale is required")]
    EmptySupportedLocales,
    /// More than [`MAX_SUPPORTED_LOCALES`] locales were supplied.
    #[error("at most {MAX_SUPPORTED_LOCALES} supported locales are allowed")]
    TooManySupportedLocales,
    /// A supported locale was empty, oversized, duplicated, or malformed.
    #[error("supported locale is empty, oversized, duplicated, or malformed")]
    InvalidSupportedLocale,
    /// The fallback was not an exact member of the supported set.
    #[error("fallback locale must be supported")]
    UnsupportedFallback,
    /// The query parameter name was empty, oversized, or malformed.
    #[error("locale query parameter name is empty, oversized, or malformed")]
    InvalidQueryParameter,
}

/// Product-owned locale choices used by [`RequestLocale`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestLocaleConfig {
    supported: Vec<String>,
    fallback_index: usize,
    query_override: LocaleQueryOverride,
}

impl RequestLocaleConfig {
    /// Validates supported locales, fallback, and query override behavior.
    pub fn new<I, L>(
        supported: I,
        fallback: impl AsRef<str>,
        query_override: LocaleQueryOverride,
    ) -> Result<Self, RequestLocaleConfigError>
    where
        I: IntoIterator<Item = L>,
        L: Into<String>,
    {
        let supported: Vec<String> = supported.into_iter().map(Into::into).collect();
        if supported.is_empty() {
            return Err(RequestLocaleConfigError::EmptySupportedLocales);
        }
        if supported.len() > MAX_SUPPORTED_LOCALES {
            return Err(RequestLocaleConfigError::TooManySupportedLocales);
        }

        let mut normalized = HashSet::with_capacity(supported.len());
        for locale in &supported {
            let lower = locale.to_ascii_lowercase();
            if locale.len() > MAX_LOCALE_TAG_BYTES
                || !valid_language_tag(locale)
                || !normalized.insert(lower)
            {
                return Err(RequestLocaleConfigError::InvalidSupportedLocale);
            }
        }

        let fallback_index = supported
            .iter()
            .position(|locale| locale.eq_ignore_ascii_case(fallback.as_ref()))
            .ok_or(RequestLocaleConfigError::UnsupportedFallback)?;

        if let LocaleQueryOverride::Parameter(name) = &query_override {
            LocaleQueryOverride::parameter(name.clone())?;
        }

        Ok(Self {
            supported,
            fallback_index,
            query_override,
        })
    }

    fn fallback(&self) -> &str {
        &self.supported[self.fallback_index]
    }
}

/// A locale selected from the configured supported set for one request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestLocale(String);

impl RequestLocale {
    /// Returns the configured spelling of the selected locale.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the extractor and returns the selected locale.
    pub fn into_string(self) -> String {
        self.0
    }
}

/// Predictable malformed-input failures from [`RequestLocale`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestLocaleRejection {
    /// The raw query exceeds [`MAX_LOCALE_QUERY_BYTES`].
    QueryTooLong,
    /// Percent decoding or UTF-8 decoding of the query failed.
    MalformedQuery,
    /// The configured locale query parameter appeared more than once.
    DuplicateQueryOverride,
    /// The explicit query locale is not in the supported set.
    UnsupportedQueryLocale,
    /// The combined `Accept-Language` values exceed [`MAX_ACCEPT_LANGUAGE_BYTES`].
    HeaderTooLong,
    /// `Accept-Language` contains invalid bytes, ranges, parameters, or quality values.
    MalformedHeader,
}

impl IntoResponse for RequestLocaleRejection {
    fn into_response(self) -> Response {
        let (field, message) = match self {
            Self::QueryTooLong => ("query", "is too long"),
            Self::MalformedQuery => ("query", "must use valid percent-encoded UTF-8"),
            Self::DuplicateQueryOverride => ("locale", "must be supplied at most once"),
            Self::UnsupportedQueryLocale => ("locale", "must be a supported locale"),
            Self::HeaderTooLong => ("accept_language", "is too long"),
            Self::MalformedHeader => ("accept_language", "is malformed"),
        };
        ApiError::validation_field(field, message).into_response()
    }
}

impl<S> FromRequestParts<S> for RequestLocale
where
    RequestLocaleConfig: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RequestLocaleRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let config = RequestLocaleConfig::from_ref(state);

        if let LocaleQueryOverride::Parameter(parameter) = &config.query_override
            && let Some(locale) = query_override(parts.uri.query(), parameter, &config)?
        {
            return Ok(Self(locale.to_owned()));
        }

        let locale = accept_language(parts, &config)?.unwrap_or_else(|| config.fallback());
        Ok(Self(locale.to_owned()))
    }
}

fn query_override<'a>(
    query: Option<&str>,
    parameter: &str,
    config: &'a RequestLocaleConfig,
) -> Result<Option<&'a str>, RequestLocaleRejection> {
    let Some(query) = query else {
        return Ok(None);
    };
    if query.len() > MAX_LOCALE_QUERY_BYTES {
        return Err(RequestLocaleRejection::QueryTooLong);
    }

    let mut selected = None;
    for pair in query.split('&') {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = decode_query_component(raw_key)?;
        let value = decode_query_component(raw_value)?;
        if key != parameter {
            continue;
        }
        if selected.is_some() {
            return Err(RequestLocaleRejection::DuplicateQueryOverride);
        }
        selected = Some(value);
    }

    let Some(selected) = selected else {
        return Ok(None);
    };
    lookup_supported(&selected, &config.supported)
        .map(Some)
        .ok_or(RequestLocaleRejection::UnsupportedQueryLocale)
}

fn decode_query_component(value: &str) -> Result<String, RequestLocaleRejection> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' => {
                let high = bytes
                    .get(index + 1)
                    .and_then(|byte| hex_value(*byte))
                    .ok_or(RequestLocaleRejection::MalformedQuery)?;
                let low = bytes
                    .get(index + 2)
                    .and_then(|byte| hex_value(*byte))
                    .ok_or(RequestLocaleRejection::MalformedQuery)?;
                decoded.push((high << 4) | low);
                index += 2;
            }
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8(decoded).map_err(|_| RequestLocaleRejection::MalformedQuery)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn accept_language<'a>(
    parts: &Parts,
    config: &'a RequestLocaleConfig,
) -> Result<Option<&'a str>, RequestLocaleRejection> {
    let mut combined = String::new();
    let mut saw_header = false;
    for value in parts.headers.get_all(header::ACCEPT_LANGUAGE) {
        saw_header = true;
        let value = value
            .to_str()
            .map_err(|_| RequestLocaleRejection::MalformedHeader)?;
        let separator_bytes = usize::from(!combined.is_empty());
        if combined.len() + separator_bytes + value.len() > MAX_ACCEPT_LANGUAGE_BYTES {
            return Err(RequestLocaleRejection::HeaderTooLong);
        }
        if !combined.is_empty() {
            combined.push(',');
        }
        combined.push_str(value);
    }
    if !saw_header {
        return Ok(None);
    }

    let mut preferences = Vec::new();
    for (position, item) in combined.split(',').enumerate() {
        let (range, quality) = parse_language_range(item)?;
        preferences.push((range, quality, position));
    }
    preferences.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.2.cmp(&right.2)));

    let excluded: Vec<&str> = preferences
        .iter()
        .filter_map(|(range, quality, _)| (*quality == Quality::ZERO).then_some(*range))
        .collect();
    for (range, quality, _) in preferences {
        if quality == Quality::ZERO {
            continue;
        }
        let matched = lookup_supported_excluding(range, &config.supported, &excluded);
        if let Some(matched) = matched {
            return Ok(Some(matched));
        }
    }
    Ok(None)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Quality(u16);

impl Quality {
    const ZERO: Self = Self(0);
}

fn parse_language_range(item: &str) -> Result<(&str, Quality), RequestLocaleRejection> {
    let mut parts = item.trim().split(';');
    let range = parts.next().unwrap_or_default().trim();
    if range != "*" && !valid_language_tag(range) {
        return Err(RequestLocaleRejection::MalformedHeader);
    }

    let quality = match parts.next() {
        None => Quality(1_000),
        Some(parameter) => {
            let (name, value) = parameter
                .trim()
                .split_once('=')
                .ok_or(RequestLocaleRejection::MalformedHeader)?;
            if !name.trim().eq_ignore_ascii_case("q") || parts.next().is_some() {
                return Err(RequestLocaleRejection::MalformedHeader);
            }
            parse_quality(value.trim())?
        }
    };
    Ok((range, quality))
}

fn parse_quality(value: &str) -> Result<Quality, RequestLocaleRejection> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() > 3 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RequestLocaleRejection::MalformedHeader);
    }
    let padded = format!("{fraction:0<3}");
    let fraction = padded
        .parse::<u16>()
        .map_err(|_| RequestLocaleRejection::MalformedHeader)?;
    match whole {
        "0" => Ok(Quality(fraction)),
        "1" if fraction == 0 => Ok(Quality(1_000)),
        _ => Err(RequestLocaleRejection::MalformedHeader),
    }
}

fn valid_language_tag(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_LOCALE_TAG_BYTES {
        return false;
    }
    value.split('-').enumerate().all(|(index, subtag)| {
        !subtag.is_empty()
            && subtag.len() <= 8
            && if index == 0 {
                subtag.bytes().all(|byte| byte.is_ascii_alphabetic())
            } else {
                subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
            }
    })
}

fn lookup_supported<'a>(range: &str, supported: &'a [String]) -> Option<&'a str> {
    let mut candidate = range;
    loop {
        if let Some(locale) = supported
            .iter()
            .find(|locale| locale.eq_ignore_ascii_case(candidate))
        {
            return Some(locale);
        }
        let prefix = format!("{candidate}-");
        if let Some(locale) = supported.iter().find(|locale| {
            locale.len() > prefix.len() && locale[..prefix.len()].eq_ignore_ascii_case(&prefix)
        }) {
            return Some(locale);
        }
        let separator = candidate.rfind('-')?;
        candidate = &candidate[..separator];
    }
}

fn lookup_supported_excluding<'a>(
    range: &str,
    supported: &'a [String],
    excluded: &[&str],
) -> Option<&'a str> {
    if range == "*" {
        return supported
            .iter()
            .find(|locale| !is_excluded(locale, excluded))
            .map(String::as_str);
    }
    let mut candidate = range;
    loop {
        if let Some(locale) = supported
            .iter()
            .find(|locale| locale.eq_ignore_ascii_case(candidate) && !is_excluded(locale, excluded))
        {
            return Some(locale);
        }
        let prefix = format!("{candidate}-");
        if let Some(locale) = supported.iter().find(|locale| {
            locale.len() > prefix.len()
                && locale[..prefix.len()].eq_ignore_ascii_case(&prefix)
                && !is_excluded(locale, excluded)
        }) {
            return Some(locale);
        }
        let separator = candidate.rfind('-')?;
        candidate = &candidate[..separator];
    }
}

fn is_excluded(locale: &str, excluded: &[&str]) -> bool {
    excluded.iter().any(|range| {
        if *range == "*" || locale.eq_ignore_ascii_case(range) {
            return true;
        }
        let prefix = format!("{range}-");
        locale.len() > prefix.len() && locale[..prefix.len()].eq_ignore_ascii_case(&prefix)
    })
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        extract::FromRef,
        http::{Request, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use serde_json::{Value, json};
    use tower::ServiceExt as _;

    use super::*;

    fn config(query_override: LocaleQueryOverride) -> RequestLocaleConfig {
        RequestLocaleConfig::new(["en", "de", "es-MX"], "en", query_override)
            .expect("valid locale config")
    }

    async fn extract(uri: &str, accept_language: Option<&str>) -> Result<String, u16> {
        async fn handler(locale: RequestLocale) -> String {
            locale.into_string()
        }

        let app = Router::new()
            .route("/locale", get(handler))
            .with_state(config(
                LocaleQueryOverride::parameter("locale").expect("valid parameter"),
            ));
        let mut request = Request::builder().uri(uri);
        if let Some(value) = accept_language {
            request = request.header(header::ACCEPT_LANGUAGE, value);
        }
        let response = app
            .oneshot(request.body(Body::empty()).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        if !status.is_success() {
            return Err(status.as_u16());
        }
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        Ok(String::from_utf8(body.to_vec()).expect("UTF-8 body"))
    }

    #[test]
    fn validates_configuration() {
        assert_eq!(
            RequestLocaleConfig::new(Vec::<String>::new(), "en", LocaleQueryOverride::Disabled),
            Err(RequestLocaleConfigError::EmptySupportedLocales)
        );
        assert_eq!(
            RequestLocaleConfig::new(["en", "EN"], "en", LocaleQueryOverride::Disabled),
            Err(RequestLocaleConfigError::InvalidSupportedLocale)
        );
        assert_eq!(
            RequestLocaleConfig::new(["en"], "de", LocaleQueryOverride::Disabled),
            Err(RequestLocaleConfigError::UnsupportedFallback)
        );
        assert_eq!(
            LocaleQueryOverride::parameter("bad parameter"),
            Err(RequestLocaleConfigError::InvalidQueryParameter)
        );
    }

    #[tokio::test]
    async fn percent_decoded_query_override_wins() {
        assert_eq!(
            extract("/locale?other=x&loc%61le=es%2DMX", Some("de;q=1")).await,
            Ok("es-MX".to_owned())
        );
        assert_eq!(
            extract("/locale?locale=de-DE", Some("es-MX")).await,
            Ok("de".to_owned())
        );
    }

    #[tokio::test]
    async fn quality_wins_and_header_order_breaks_ties() {
        assert_eq!(
            extract("/locale", Some("de;q=0.4, es;q=0.9, en;q=0.8")).await,
            Ok("es-MX".to_owned())
        );
        assert_eq!(
            extract("/locale", Some("es;q=0.8, de;q=0.8")).await,
            Ok("es-MX".to_owned())
        );
        assert_eq!(
            extract("/locale", Some("de;q=0, fr;q=1")).await,
            Ok("en".to_owned())
        );
        assert_eq!(
            extract("/locale", Some("*;q=1, en;q=0, de;q=0")).await,
            Ok("es-MX".to_owned())
        );
    }

    #[tokio::test]
    async fn rejects_malformed_and_oversized_input() {
        for uri in [
            "/locale?locale=%ZZ",
            "/locale?locale=de&locale=en",
            "/locale?locale=fr",
        ] {
            assert_eq!(extract(uri, None).await, Err(400));
        }
        for header in [
            "",
            "de;q=1.1",
            "de;q=0.1234",
            "de;level=1",
            "de,,en",
            "de_ DE",
        ] {
            assert_eq!(extract("/locale", Some(header)).await, Err(400));
        }
        let oversized_header = "a".repeat(MAX_ACCEPT_LANGUAGE_BYTES + 1);
        assert_eq!(extract("/locale", Some(&oversized_header)).await, Err(400));
        let oversized_query = format!("/locale?{}", "a".repeat(MAX_LOCALE_QUERY_BYTES + 1));
        assert_eq!(extract(&oversized_query, None).await, Err(400));
    }

    #[tokio::test]
    async fn query_override_can_be_disabled() {
        async fn handler(locale: RequestLocale) -> String {
            locale.into_string()
        }
        let app = Router::new()
            .route("/locale", get(handler))
            .with_state(config(LocaleQueryOverride::Disabled));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/locale?locale=de")
                    .header(header::ACCEPT_LANGUAGE, "es-MX")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(&body[..], b"es-MX");
    }

    #[tokio::test]
    async fn supports_locale_config_inside_application_state() {
        #[derive(Clone)]
        struct AppState {
            locales: RequestLocaleConfig,
        }

        impl FromRef<AppState> for RequestLocaleConfig {
            fn from_ref(state: &AppState) -> Self {
                state.locales.clone()
            }
        }

        async fn handler(locale: RequestLocale) -> String {
            locale.into_string()
        }

        let app = Router::new()
            .route("/locale", get(handler))
            .with_state(AppState {
                locales: config(LocaleQueryOverride::Disabled),
            });
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/locale")
                    .header(header::ACCEPT_LANGUAGE, "de")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(&body[..], b"de");
    }

    #[tokio::test]
    async fn rejection_uses_the_safe_validation_envelope() {
        let response = RequestLocaleRejection::MalformedHeader.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body: Value = serde_json::from_slice(&bytes).expect("JSON body");
        assert_eq!(body["error"]["code"], "validation_failed");
        assert_eq!(
            body["error"]["details"],
            json!({"accept_language": "is malformed"})
        );
    }

    #[test]
    fn parses_quality_without_floating_point() {
        assert_eq!(parse_quality("0"), Ok(Quality(0)));
        assert_eq!(parse_quality("0.5"), Ok(Quality(500)));
        assert_eq!(parse_quality("1.000"), Ok(Quality(1_000)));
        assert_eq!(
            parse_quality("1.001"),
            Err(RequestLocaleRejection::MalformedHeader)
        );
    }

    #[test]
    fn exact_regional_match_wins_before_configured_sibling() {
        let supported = vec!["en-GB".to_owned(), "en-US".to_owned()];
        assert_eq!(lookup_supported("en-US", &supported), Some("en-US"));
        assert_eq!(
            lookup_supported_excluding("en", &supported, &["en-GB"]),
            Some("en-US")
        );
    }
}
