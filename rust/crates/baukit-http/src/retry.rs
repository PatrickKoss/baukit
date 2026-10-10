//! Classification of upstream HTTP responses and transport failures.
//!
//! Outbound clients all need the same decision: was this failure worth
//! retrying, and if so, after how long? [`classify_http_status`] answers that
//! from the status code and response headers alone, so it stays a pure function
//! that is easy to test and free of any particular HTTP client.

use std::time::Duration;

use axum::http::{HeaderMap, HeaderName, StatusCode, header::RETRY_AFTER};
use time::{OffsetDateTime, format_description::well_known::Rfc2822};

/// How a caller should react to a failed upstream request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryClass {
    /// The upstream asked for a specific wait before the next attempt.
    ///
    /// This is a rate limit that came with a usable retry delay.
    RetryAfter(Duration),
    /// The upstream rate limited the request without saying for how long.
    ///
    /// Back off with the client's own schedule.
    RateLimited,
    /// The upstream is failing or unreachable and the request may be retried.
    Unavailable,
    /// The request timed out before a response arrived.
    Timeout,
    /// The credential was rejected; re-authorize instead of retrying.
    Revoked,
    /// The request will fail the same way again; do not retry.
    Permanent,
}

/// Options for reading retry delays from response headers.
///
/// Vendor headers are read in order before the standard `Retry-After` header.
/// By default, each header value is parsed whole. Use
/// [`RetryHeaderOptions::with_first_field`] for APIs such as Polar that return
/// comma-separated quota windows and define the first field as the active one.
///
/// Parsed delays are uncapped by default, so the classifier reports what the
/// upstream sent. A client that schedules its own retries should set
/// [`RetryHeaderOptions::with_max_retry_after`].
/// Use [`RetryHeaderOptions::with_forbidden_rate_limit`] for APIs such as
/// GitHub that also report rate limits with `403`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetryHeaderOptions<'a> {
    extra_retry_after_headers: &'a [&'a str],
    first_field: bool,
    max_retry_after: Option<Duration>,
    forbidden_rate_limit: bool,
}

impl<'a> RetryHeaderOptions<'a> {
    /// Creates options that consult the named vendor headers in order.
    pub const fn new(extra_retry_after_headers: &'a [&'a str]) -> Self {
        Self {
            extra_retry_after_headers,
            first_field: false,
            max_retry_after: None,
            forbidden_rate_limit: false,
        }
    }

    /// Parses only the first comma-separated field of vendor header values.
    ///
    /// This does not split `Retry-After`, whose HTTP-date form contains a
    /// comma.
    #[must_use]
    pub const fn with_first_field(mut self) -> Self {
        self.first_field = true;
        self
    }

    /// Clamps parsed delays to `max`.
    ///
    /// An upstream that sends `Retry-After: 86400` then yields `max` instead of
    /// stalling a worker for a day.
    #[must_use]
    pub const fn with_max_retry_after(mut self, max: Duration) -> Self {
        self.max_retry_after = Some(max);
        self
    }

    /// Recognizes rate-limit headers on `403` responses.
    ///
    /// A usable retry header gives [`RetryClass::RetryAfter`]. Otherwise,
    /// `x-ratelimit-remaining: 0` gives [`RetryClass::RetryAfter`] when
    /// `x-ratelimit-reset` is a valid Unix epoch in seconds, or
    /// [`RetryClass::RateLimited`] without a usable reset. Other `403`
    /// responses and all `401` responses stay [`RetryClass::Revoked`].
    #[must_use]
    pub const fn with_forbidden_rate_limit(mut self) -> Self {
        self.forbidden_rate_limit = true;
        self
    }

    fn cap(self, delay: Duration) -> Duration {
        match self.max_retry_after {
            Some(max) => delay.min(max),
            None => delay,
        }
    }
}

impl RetryClass {
    /// Returns whether retrying the same request can succeed.
    ///
    /// [`RetryClass::Revoked`] and [`RetryClass::Permanent`] return `false`:
    /// both need an operator or a re-authorization, not another attempt.
    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::RetryAfter(_) | Self::RateLimited | Self::Unavailable | Self::Timeout
        )
    }

    /// Returns the wait the upstream asked for, if it named one.
    pub const fn retry_after(self) -> Option<Duration> {
        match self {
            Self::RetryAfter(delay) => Some(delay),
            _ => None,
        }
    }
}

/// Classifies an upstream HTTP response by status and headers.
///
/// The mapping is:
///
/// - `401` and `403` are [`RetryClass::Revoked`]; the credential needs renewing.
/// - `408` and `504` are [`RetryClass::Timeout`].
/// - `425` is [`RetryClass::Unavailable`]. RFC 8470 section 5.2 lets a client
///   retry once the request is no longer sent as early data.
/// - `429` is [`RetryClass::RetryAfter`] when a retry-after header parses, and
///   [`RetryClass::RateLimited`] otherwise. The delay is uncapped; see
///   [`RetryHeaderOptions::with_max_retry_after`].
/// - Any other `5xx` is [`RetryClass::Unavailable`].
/// - Everything else, success codes included, is [`RetryClass::Permanent`];
///   callers only reach this function for responses they already treat as
///   failures.
///
/// `extra_retry_after_headers` names vendor headers to consult *before* the
/// standard `Retry-After`, in the order given. Fitbit, for instance, sends the
/// remaining window in `fitbit-rate-limit-reset`, so that integration passes
/// `&["fitbit-rate-limit-reset"]` while every other one passes `&[]`. Names that
/// are not valid header names are skipped.
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
///
/// use baukit_http::{RetryClass, classify_http_status};
/// use axum::http::{HeaderMap, HeaderValue, StatusCode, header::RETRY_AFTER};
///
/// let mut headers = HeaderMap::new();
/// headers.insert(RETRY_AFTER, HeaderValue::from_static("30"));
/// assert_eq!(
///     classify_http_status(StatusCode::TOO_MANY_REQUESTS, &headers, &[]),
///     RetryClass::RetryAfter(Duration::from_secs(30))
/// );
/// assert_eq!(
///     classify_http_status(StatusCode::FORBIDDEN, &HeaderMap::new(), &[]),
///     RetryClass::Revoked
/// );
/// ```
pub fn classify_http_status(
    status: StatusCode,
    headers: &HeaderMap,
    extra_retry_after_headers: &[&str],
) -> RetryClass {
    classify_http_status_with_options(
        status,
        headers,
        RetryHeaderOptions::new(extra_retry_after_headers),
    )
}

/// Classifies an upstream HTTP response with configurable retry header parsing.
///
/// See [`classify_http_status`] for the status mapping. The options control
/// which vendor retry headers are checked, whether their first
/// comma-separated field is used, and the largest delay reported.
/// [`RetryHeaderOptions::with_forbidden_rate_limit`] also recognizes rate
/// limits on `403` responses. `401` always stays [`RetryClass::Revoked`].
pub fn classify_http_status_with_options(
    status: StatusCode,
    headers: &HeaderMap,
    options: RetryHeaderOptions<'_>,
) -> RetryClass {
    classify_http_status_with_options_at(status, headers, options, OffsetDateTime::now_utc())
}

/// Classifies an upstream HTTP response using `options` at `now`.
///
/// See [`classify_http_status_with_options`] for the status mapping.
/// `now` is the reference instant for HTTP-date retry headers and the
/// `x-ratelimit-reset` epoch on opted-in `403` responses. Past deadlines
/// yield [`Duration::ZERO`], and delays are clamped to the configured cap.
pub fn classify_http_status_with_options_at(
    status: StatusCode,
    headers: &HeaderMap,
    options: RetryHeaderOptions<'_>,
    now: OffsetDateTime,
) -> RetryClass {
    match status {
        StatusCode::FORBIDDEN if options.forbidden_rate_limit => {
            classify_forbidden_rate_limit(headers, options, now)
        }
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => RetryClass::Revoked,
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => RetryClass::Timeout,
        StatusCode::TOO_EARLY => RetryClass::Unavailable,
        StatusCode::TOO_MANY_REQUESTS => {
            retry_after_from_headers_with_options_at(headers, options, now)
                .map_or(RetryClass::RateLimited, RetryClass::RetryAfter)
        }
        _ if status.is_server_error() => RetryClass::Unavailable,
        _ => RetryClass::Permanent,
    }
}

fn classify_forbidden_rate_limit(
    headers: &HeaderMap,
    options: RetryHeaderOptions<'_>,
    now: OffsetDateTime,
) -> RetryClass {
    if let Some(delay) = retry_after_from_headers_with_options_at(headers, options, now) {
        return RetryClass::RetryAfter(delay);
    }
    let remaining = headers
        .get("x-ratelimit-remaining")
        .and_then(|value| value.to_str().ok());
    if remaining.map(str::trim) != Some("0") {
        return RetryClass::Revoked;
    }
    let reset = headers
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<i64>().ok())
        .and_then(|epoch| OffsetDateTime::from_unix_timestamp(epoch).ok());
    reset.map_or(RetryClass::RateLimited, |deadline| {
        let delay = (deadline - now).try_into().unwrap_or(Duration::ZERO);
        RetryClass::RetryAfter(options.cap(delay))
    })
}

/// Reads the retry delay from the vendor headers first, then `Retry-After`.
///
/// Both `Retry-After` forms are accepted: delay seconds (`Retry-After: 120`)
/// and an HTTP-date (`Retry-After: Wed, 21 Oct 2026 07:28:00 GMT`). A date in
/// the past yields [`Duration::ZERO`], meaning retry now. Values that parse as
/// neither are ignored, and the next candidate header is tried.
///
/// `now` is the reference instant for HTTP-date values; pass
/// [`OffsetDateTime::now_utc`].
pub fn retry_after_from_headers_at(
    headers: &HeaderMap,
    extra_retry_after_headers: &[&str],
    now: OffsetDateTime,
) -> Option<Duration> {
    retry_after_from_headers_with_options_at(
        headers,
        RetryHeaderOptions::new(extra_retry_after_headers),
        now,
    )
}

/// Reads a retry delay using configurable vendor header parsing at `now`.
///
/// Vendor headers are checked before `Retry-After`. Invalid names and values
/// are skipped. The standard header always keeps its whole value so HTTP dates
/// continue to parse when first-field handling is enabled for vendor headers.
/// A delay above [`RetryHeaderOptions::with_max_retry_after`] is clamped.
pub fn retry_after_from_headers_with_options_at(
    headers: &HeaderMap,
    options: RetryHeaderOptions<'_>,
    now: OffsetDateTime,
) -> Option<Duration> {
    let vendor_delay = options
        .extra_retry_after_headers
        .iter()
        .filter_map(|name| HeaderName::try_from(*name).ok())
        .filter_map(|name| headers.get(&name).and_then(|value| value.to_str().ok()))
        .find_map(|value| {
            let value = if options.first_field {
                value.split(',').next().unwrap_or(value)
            } else {
                value
            };
            parse_retry_after(value, now)
        });

    vendor_delay
        .or_else(|| {
            headers
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| parse_retry_after(value, now))
        })
        .map(|delay| options.cap(delay))
}

/// Reads the retry delay relative to the current time.
///
/// See [`retry_after_from_headers_at`] for the accepted formats.
pub fn retry_after_from_headers(
    headers: &HeaderMap,
    extra_retry_after_headers: &[&str],
) -> Option<Duration> {
    retry_after_from_headers_at(
        headers,
        extra_retry_after_headers,
        OffsetDateTime::now_utc(),
    )
}

/// Reads a retry delay relative to the current time using `options`.
///
/// See [`retry_after_from_headers_with_options_at`] for the accepted formats.
pub fn retry_after_from_headers_with_options(
    headers: &HeaderMap,
    options: RetryHeaderOptions<'_>,
) -> Option<Duration> {
    retry_after_from_headers_with_options_at(headers, options, OffsetDateTime::now_utc())
}

/// Classifies a transport failure that never produced a response.
///
/// HTTP clients report timeouts differently, so the caller decides: pass
/// `is_timeout` from the client's own predicate, for example
/// `reqwest::Error::is_timeout`. Everything else is treated as a transient
/// connection problem.
pub const fn classify_transport_error(is_timeout: bool) -> RetryClass {
    if is_timeout {
        RetryClass::Timeout
    } else {
        RetryClass::Unavailable
    }
}

fn parse_retry_after(value: &str, now: OffsetDateTime) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let deadline = OffsetDateTime::parse(value, &Rfc2822).ok()?;
    Some((deadline - now).try_into().unwrap_or(Duration::ZERO))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    const FITBIT_RESET: &str = "fitbit-rate-limit-reset";

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                HeaderName::try_from(*name).expect("header name should be valid"),
                HeaderValue::from_str(value).expect("header value should be valid"),
            );
        }
        map
    }

    fn at(text: &str) -> OffsetDateTime {
        OffsetDateTime::parse(text, &Rfc2822).expect("reference date should parse")
    }

    #[test]
    fn credentials_rejected_by_upstream_are_revoked() {
        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            assert_eq!(
                classify_http_status(status, &HeaderMap::new(), &[]),
                RetryClass::Revoked
            );
        }
    }

    #[test]
    fn forbidden_rate_limit_reads_retry_after_seconds() {
        let options = RetryHeaderOptions::default().with_forbidden_rate_limit();
        for seconds in [0, 120, 86_400] {
            assert_eq!(
                classify_http_status_with_options(
                    StatusCode::FORBIDDEN,
                    &headers(&[("retry-after", &seconds.to_string())]),
                    options,
                ),
                RetryClass::RetryAfter(Duration::from_secs(seconds))
            );
        }
    }

    #[test]
    fn forbidden_rate_limit_reads_retry_after_http_dates() {
        let now = at("Wed, 21 Oct 2026 07:28:00 GMT");
        let options = RetryHeaderOptions::default().with_forbidden_rate_limit();
        let cases = [
            ("Wed, 21 Oct 2026 07:30:00 GMT", Duration::from_secs(120)),
            ("Wed, 21 Oct 2026 07:00:00 GMT", Duration::ZERO),
        ];
        for (date, delay) in cases {
            assert_eq!(
                classify_http_status_with_options_at(
                    StatusCode::FORBIDDEN,
                    &headers(&[("retry-after", date)]),
                    options,
                    now,
                ),
                RetryClass::RetryAfter(delay)
            );
        }
    }

    #[test]
    fn forbidden_rate_limit_reads_reset_epochs_and_saturates_past_resets() {
        let now = OffsetDateTime::from_unix_timestamp(1_000).expect("reference epoch is valid");
        let options = RetryHeaderOptions::default().with_forbidden_rate_limit();
        for (reset, delay) in [
            ("1120", Duration::from_secs(120)),
            ("1000", Duration::ZERO),
            ("880", Duration::ZERO),
            ("-1", Duration::ZERO),
        ] {
            assert_eq!(
                classify_http_status_with_options_at(
                    StatusCode::FORBIDDEN,
                    &headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", reset)]),
                    options,
                    now,
                ),
                RetryClass::RetryAfter(delay),
                "reset: {reset}"
            );
        }
    }

    #[test]
    fn forbidden_rate_limit_without_a_usable_reset_has_no_delay() {
        let now = OffsetDateTime::from_unix_timestamp(1_000).expect("reference epoch is valid");
        let options = RetryHeaderOptions::default().with_forbidden_rate_limit();
        for reset in [
            None,
            Some("later"),
            Some("Wed, 21 Oct 2026 07:30:00 GMT"),
            Some("1120.5"),
            Some("9223372036854775807"),
            Some("18446744073709551616"),
        ] {
            let mut map = headers(&[("x-ratelimit-remaining", "0"), ("retry-after", "soon")]);
            if let Some(reset) = reset {
                map.insert(
                    "x-ratelimit-reset",
                    HeaderValue::from_str(reset).expect("reset should be a valid header value"),
                );
            }
            assert_eq!(
                classify_http_status_with_options_at(StatusCode::FORBIDDEN, &map, options, now),
                RetryClass::RateLimited,
                "reset: {reset:?}"
            );
        }
    }

    #[test]
    fn forbidden_without_rate_limit_evidence_stays_revoked() {
        let now = OffsetDateTime::from_unix_timestamp(1_000).expect("reference epoch is valid");
        let options = RetryHeaderOptions::default().with_forbidden_rate_limit();
        let cases = [
            HeaderMap::new(),
            headers(&[("retry-after", "soon")]),
            headers(&[("x-ratelimit-reset", "1120")]),
            headers(&[
                ("x-ratelimit-remaining", "1"),
                ("x-ratelimit-reset", "1120"),
            ]),
            headers(&[
                ("x-ratelimit-remaining", "invalid"),
                ("x-ratelimit-reset", "1120"),
            ]),
        ];
        for map in cases {
            assert_eq!(
                classify_http_status_with_options_at(StatusCode::FORBIDDEN, &map, options, now),
                RetryClass::Revoked,
                "headers: {map:?}"
            );
        }
    }

    #[test]
    fn unauthorized_stays_revoked_with_forbidden_rate_limit_enabled() {
        let options = RetryHeaderOptions::new(&[FITBIT_RESET]).with_forbidden_rate_limit();
        let map = headers(&[
            ("retry-after", "120"),
            (FITBIT_RESET, "45"),
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", "1120"),
        ]);
        assert_eq!(
            classify_http_status_with_options(StatusCode::UNAUTHORIZED, &map, options),
            RetryClass::Revoked
        );
    }

    #[test]
    fn forbidden_rate_limit_headers_are_ignored_by_default() {
        let now = OffsetDateTime::from_unix_timestamp(1_000).expect("reference epoch is valid");
        let cases = [
            headers(&[("retry-after", "120")]),
            headers(&[("retry-after", "Wed, 21 Oct 2026 07:30:00 GMT")]),
            headers(&[(FITBIT_RESET, "45")]),
            headers(&[("ratelimit-reset", "30, 3600")]),
            headers(&[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", "1120"),
            ]),
            headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "880")]),
            headers(&[("x-ratelimit-remaining", "0")]),
            headers(&[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", "later"),
            ]),
            headers(&[
                ("x-ratelimit-remaining", "1"),
                ("x-ratelimit-reset", "1120"),
            ]),
            headers(&[
                ("retry-after", "120"),
                (FITBIT_RESET, "45"),
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", "1120"),
            ]),
            HeaderMap::new(),
        ];
        for options in [
            RetryHeaderOptions::default(),
            RetryHeaderOptions::new(&[FITBIT_RESET, "ratelimit-reset"]).with_first_field(),
        ] {
            for map in &cases {
                assert_eq!(
                    classify_http_status_with_options_at(StatusCode::FORBIDDEN, map, options, now),
                    RetryClass::Revoked,
                    "headers: {map:?}"
                );
            }
        }
    }

    #[test]
    fn forbidden_rate_limit_delays_are_capped_for_every_header_source() {
        let now = OffsetDateTime::from_unix_timestamp(1_000).expect("reference epoch is valid");
        let cases = [
            headers(&[("retry-after", "3600")]),
            headers(&[("retry-after", "Thu, 01 Jan 1970 01:16:40 GMT")]),
            headers(&[(FITBIT_RESET, "3600")]),
            headers(&[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", "4600"),
            ]),
        ];
        for cap in [Duration::ZERO, Duration::from_secs(60)] {
            let options = RetryHeaderOptions::new(&[FITBIT_RESET])
                .with_forbidden_rate_limit()
                .with_max_retry_after(cap);
            for map in &cases {
                assert_eq!(
                    classify_http_status_with_options_at(StatusCode::FORBIDDEN, map, options, now),
                    RetryClass::RetryAfter(cap),
                    "headers: {map:?}"
                );
            }
        }
    }

    #[test]
    fn forbidden_rate_limit_keeps_vendor_parsing_and_header_precedence() {
        let options = RetryHeaderOptions::new(&[FITBIT_RESET]).with_forbidden_rate_limit();
        for (vendor, delay) in [("45", 45), ("later", 120)] {
            let map = headers(&[
                (FITBIT_RESET, vendor),
                ("retry-after", "120"),
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", "0"),
            ]);
            assert_eq!(
                classify_http_status_with_options(StatusCode::FORBIDDEN, &map, options),
                RetryClass::RetryAfter(Duration::from_secs(delay))
            );
        }
        let map = headers(&[("ratelimit-reset", "30, 3600")]);
        let options = RetryHeaderOptions::new(&["ratelimit-reset"]).with_forbidden_rate_limit();
        assert_eq!(
            classify_http_status_with_options(StatusCode::FORBIDDEN, &map, options),
            RetryClass::Revoked
        );
        assert_eq!(
            classify_http_status_with_options(
                StatusCode::FORBIDDEN,
                &map,
                options.with_first_field()
            ),
            RetryClass::RetryAfter(Duration::from_secs(30))
        );
    }

    #[test]
    fn reset_epoch_handling_does_not_change_other_statuses_or_vendor_parsing() {
        let now = OffsetDateTime::from_unix_timestamp(1_000).expect("reference epoch is valid");
        let map = headers(&[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", "1120"),
        ]);
        let options = RetryHeaderOptions::default().with_forbidden_rate_limit();
        assert_eq!(
            classify_http_status_with_options_at(StatusCode::TOO_MANY_REQUESTS, &map, options, now),
            RetryClass::RateLimited
        );
        let options = RetryHeaderOptions::new(&["x-ratelimit-reset"]).with_forbidden_rate_limit();
        for status in [StatusCode::TOO_MANY_REQUESTS, StatusCode::FORBIDDEN] {
            assert_eq!(
                classify_http_status_with_options_at(status, &map, options, now),
                RetryClass::RetryAfter(Duration::from_secs(1_120))
            );
        }
    }

    #[test]
    fn timeout_statuses_are_classified_as_timeouts() {
        for status in [StatusCode::REQUEST_TIMEOUT, StatusCode::GATEWAY_TIMEOUT] {
            assert_eq!(
                classify_http_status(status, &HeaderMap::new(), &[]),
                RetryClass::Timeout
            );
        }
    }

    #[test]
    fn too_early_is_retryable() {
        let class = classify_http_status(StatusCode::TOO_EARLY, &HeaderMap::new(), &[]);
        assert_eq!(class, RetryClass::Unavailable);
        assert!(class.is_retryable());
    }

    #[test]
    fn retry_after_is_uncapped_by_default() {
        assert_eq!(
            classify_http_status(
                StatusCode::TOO_MANY_REQUESTS,
                &headers(&[("retry-after", "86400")]),
                &[],
            ),
            RetryClass::RetryAfter(Duration::from_secs(86_400))
        );
    }

    #[test]
    fn max_retry_after_clamps_longer_delays() {
        let options = RetryHeaderOptions::default().with_max_retry_after(Duration::from_secs(300));
        assert_eq!(
            classify_http_status_with_options(
                StatusCode::TOO_MANY_REQUESTS,
                &headers(&[("retry-after", "86400")]),
                options,
            ),
            RetryClass::RetryAfter(Duration::from_secs(300))
        );
        assert_eq!(
            retry_after_from_headers_with_options(&headers(&[("retry-after", "120")]), options),
            Some(Duration::from_secs(120))
        );
    }

    #[test]
    fn max_retry_after_clamps_http_dates_and_vendor_headers() {
        let now = at("Wed, 21 Oct 2026 07:28:00 GMT");
        let cap = Duration::from_secs(60);
        let date = headers(&[("retry-after", "Thu, 22 Oct 2026 07:28:00 GMT")]);
        assert_eq!(
            retry_after_from_headers_with_options_at(
                &date,
                RetryHeaderOptions::default().with_max_retry_after(cap),
                now,
            ),
            Some(cap)
        );
        let vendor = headers(&[(FITBIT_RESET, "3600")]);
        assert_eq!(
            retry_after_from_headers_with_options_at(
                &vendor,
                RetryHeaderOptions::new(&[FITBIT_RESET]).with_max_retry_after(cap),
                now,
            ),
            Some(cap)
        );
    }

    #[test]
    fn rate_limit_without_a_usable_header_has_no_delay() {
        assert_eq!(
            classify_http_status(StatusCode::TOO_MANY_REQUESTS, &HeaderMap::new(), &[]),
            RetryClass::RateLimited
        );
        assert_eq!(
            classify_http_status(
                StatusCode::TOO_MANY_REQUESTS,
                &headers(&[("retry-after", "soon")]),
                &[],
            ),
            RetryClass::RateLimited
        );
    }

    #[test]
    fn rate_limit_reads_retry_after_seconds() {
        assert_eq!(
            classify_http_status(
                StatusCode::TOO_MANY_REQUESTS,
                &headers(&[("retry-after", "120")]),
                &[],
            ),
            RetryClass::RetryAfter(Duration::from_secs(120))
        );
    }

    #[test]
    fn retry_after_accepts_an_http_date() {
        let now = at("Wed, 21 Oct 2026 07:28:00 GMT");
        let map = headers(&[("retry-after", "Wed, 21 Oct 2026 07:30:00 GMT")]);
        assert_eq!(
            retry_after_from_headers_at(&map, &[], now),
            Some(Duration::from_secs(120))
        );
    }

    #[test]
    fn a_retry_after_date_in_the_past_means_retry_now() {
        let now = at("Wed, 21 Oct 2026 07:28:00 GMT");
        let map = headers(&[("retry-after", "Wed, 21 Oct 2026 07:00:00 GMT")]);
        assert_eq!(
            retry_after_from_headers_at(&map, &[], now),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn vendor_header_wins_over_retry_after() {
        let map = headers(&[(FITBIT_RESET, "45"), ("retry-after", "120")]);
        assert_eq!(
            classify_http_status(StatusCode::TOO_MANY_REQUESTS, &map, &[FITBIT_RESET]),
            RetryClass::RetryAfter(Duration::from_secs(45))
        );
        assert_eq!(
            classify_http_status(StatusCode::TOO_MANY_REQUESTS, &map, &[]),
            RetryClass::RetryAfter(Duration::from_secs(120))
        );
    }

    #[test]
    fn an_unparseable_vendor_header_falls_through_to_retry_after() {
        let map = headers(&[(FITBIT_RESET, "later"), ("retry-after", "9")]);
        assert_eq!(
            classify_http_status(StatusCode::TOO_MANY_REQUESTS, &map, &[FITBIT_RESET]),
            RetryClass::RetryAfter(Duration::from_secs(9))
        );
    }

    #[test]
    fn comma_separated_vendor_header_is_unchanged_by_default() {
        let map = headers(&[("ratelimit-reset", "30, 3600")]);
        assert_eq!(
            classify_http_status(StatusCode::TOO_MANY_REQUESTS, &map, &["ratelimit-reset"],),
            RetryClass::RateLimited
        );
    }

    #[test]
    fn first_field_option_reads_a_comma_separated_vendor_header() {
        let map = headers(&[("ratelimit-reset", "30, 3600")]);
        let options = RetryHeaderOptions::new(&["ratelimit-reset"]).with_first_field();
        assert_eq!(
            classify_http_status_with_options(StatusCode::TOO_MANY_REQUESTS, &map, options),
            RetryClass::RetryAfter(Duration::from_secs(30))
        );
    }

    #[test]
    fn first_field_option_trims_whitespace() {
        let map = headers(&[("ratelimit-reset", " 30 ,  3600 ")]);
        let options = RetryHeaderOptions::new(&["ratelimit-reset"]).with_first_field();
        assert_eq!(
            retry_after_from_headers_with_options(&map, options),
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn first_field_option_accepts_a_single_value() {
        let map = headers(&[("ratelimit-reset", "30")]);
        let options = RetryHeaderOptions::new(&["ratelimit-reset"]).with_first_field();
        assert_eq!(
            retry_after_from_headers_with_options(&map, options),
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn first_field_option_does_not_split_retry_after_http_dates() {
        let now = at("Wed, 21 Oct 2026 07:28:00 GMT");
        let map = headers(&[("retry-after", "Wed, 21 Oct 2026 07:30:00 GMT")]);
        let options = RetryHeaderOptions::default().with_first_field();
        assert_eq!(
            retry_after_from_headers_with_options_at(&map, options, now),
            Some(Duration::from_secs(120))
        );
    }

    #[test]
    fn an_invalid_extra_header_name_is_skipped() {
        let map = headers(&[("retry-after", "5")]);
        assert_eq!(
            classify_http_status(StatusCode::TOO_MANY_REQUESTS, &map, &["not a header"]),
            RetryClass::RetryAfter(Duration::from_secs(5))
        );
    }

    #[test]
    fn other_server_errors_are_unavailable() {
        for status in [
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            assert_eq!(
                classify_http_status(status, &HeaderMap::new(), &[]),
                RetryClass::Unavailable
            );
        }
    }

    #[test]
    fn other_statuses_are_permanent() {
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::NOT_FOUND,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::OK,
        ] {
            assert_eq!(
                classify_http_status(status, &HeaderMap::new(), &[]),
                RetryClass::Permanent
            );
        }
    }

    #[test]
    fn transport_errors_split_timeouts_from_connection_failures() {
        assert_eq!(classify_transport_error(true), RetryClass::Timeout);
        assert_eq!(classify_transport_error(false), RetryClass::Unavailable);
    }

    #[test]
    fn retryability_and_delay_are_exposed_per_class() {
        assert!(RetryClass::RetryAfter(Duration::from_secs(1)).is_retryable());
        assert!(RetryClass::RateLimited.is_retryable());
        assert!(RetryClass::Unavailable.is_retryable());
        assert!(RetryClass::Timeout.is_retryable());
        assert!(!RetryClass::Revoked.is_retryable());
        assert!(!RetryClass::Permanent.is_retryable());

        assert_eq!(
            RetryClass::RetryAfter(Duration::from_secs(3)).retry_after(),
            Some(Duration::from_secs(3))
        );
        assert_eq!(RetryClass::RateLimited.retry_after(), None);
    }
}
