#![no_main]

use std::sync::LazyLock;

use axum::extract::FromRequestParts;
use baukit_http::{
    IdempotencyKeyRule, LocaleQueryOverride, MAX_IDEMPOTENCY_KEY_BYTES, RequestLocale,
    RequestLocaleConfig, RevisionEtag,
};
use http::{HeaderValue, Request, header};
use libfuzzer_sys::fuzz_target;
use tokio::runtime::{Builder, Runtime};

const SUPPORTED_LOCALES: [&str; 3] = ["en", "de-DE", "pt-BR"];
const ETAG: RevisionEtag<'static> = RevisionEtag::new("rev-");
const MIN_IDEMPOTENCY_KEY_BYTES: usize = 8;

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    Builder::new_current_thread()
        .build()
        .expect("tokio runtime")
});

static LOCALE_CONFIG: LazyLock<RequestLocaleConfig> = LazyLock::new(|| {
    RequestLocaleConfig::new(
        SUPPORTED_LOCALES,
        "en",
        LocaleQueryOverride::parameter("locale").expect("query parameter"),
    )
    .expect("locale config")
});

fn check_idempotency_key(value: &HeaderValue) {
    let rule = IdempotencyKeyRule::new(MIN_IDEMPOTENCY_KEY_BYTES, MAX_IDEMPOTENCY_KEY_BYTES);
    if let Ok(key) = rule.parse(value) {
        assert_eq!(key.as_str().as_bytes(), value.as_bytes());
        assert!(key.as_str().len() >= MIN_IDEMPOTENCY_KEY_BYTES);
        assert!(key.as_str().len() <= MAX_IDEMPOTENCY_KEY_BYTES);
    }
}

fn check_if_match(value: &HeaderValue) {
    if let Ok(revision) = ETAG.parse(value) {
        let trimmed = value.as_bytes().trim_ascii_start().trim_ascii_end();
        assert_eq!(
            ETAG.header_value(revision).as_bytes(),
            trimmed,
            "accepted ETag is not in canonical form"
        );
    }
}

fn check_locale(text: &str, value: &HeaderValue) {
    let uri = format!("/?{}", text.replace(['#', ' '], ""));
    let Ok(request) = Request::builder()
        .uri(uri)
        .header(header::ACCEPT_LANGUAGE, value)
        .body(())
    else {
        return;
    };
    let (mut parts, ()) = request.into_parts();
    let result = RUNTIME.block_on(RequestLocale::from_request_parts(
        &mut parts,
        &*LOCALE_CONFIG,
    ));
    if let Ok(locale) = result {
        assert!(SUPPORTED_LOCALES.contains(&locale.as_str()));
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(value) = HeaderValue::from_bytes(data) else {
        return;
    };
    check_idempotency_key(&value);
    check_if_match(&value);
    if let Ok(text) = std::str::from_utf8(data) {
        check_locale(text, &value);
    }
});
