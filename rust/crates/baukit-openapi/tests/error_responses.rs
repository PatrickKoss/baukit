use std::fs;
use std::path::PathBuf;

use baukit_openapi::{ErrorResponseRules, OperationCondition as When, ResponseSelector};
use serde_json::Value;
use utoipa::openapi::header::{Header, HeaderBuilder};
use utoipa::openapi::path::HttpMethod;
use utoipa::openapi::{ObjectBuilder, OpenApi, Type};

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn decorate(input: &str, decorate: impl Fn(&mut OpenApi)) -> Value {
    let mut document: OpenApi =
        serde_json::from_str(&fixture(input)).expect("input fixture is an OpenAPI document");
    decorate(&mut document);
    serde_json::to_value(&document).expect("serialize document")
}

fn expected(name: &str) -> Value {
    serde_json::from_str(&fixture(name)).expect("expected fixture is JSON")
}

fn described(status: u16) -> String {
    format!("{status} response")
}

fn header(name: &str, schema: ObjectBuilder) -> Header {
    HeaderBuilder::new()
        .schema(schema)
        .description(Some(format!("{name} header")))
        .build()
}

fn string_header(name: &str) -> Header {
    header(name, ObjectBuilder::new().schema_type(Type::String))
}

#[test]
fn status_rules_reproduce_a_product_decorator_without_headers() {
    let always = [400, 429, 500, 503];
    let unsafe_method = [409, 413];
    let rules = always
        .into_iter()
        .map(|status| (When::Always, status))
        .chain([(When::Secured, 401), (When::Secured, 403)])
        .chain([(When::HasPathParameter, 404)])
        .chain(unsafe_method.map(|status| (When::UnsafeMethod, status)))
        .fold(ErrorResponseRules::new(), |rules, (condition, status)| {
            rules.status(condition, status, described(status))
        });
    let stale_revision = ErrorResponseRules::new().status(When::Always, 412, described(412));

    let actual = decorate("error-responses-status-only-input.json", |document| {
        rules.apply(document);
        stale_revision.apply_where(document, |path, method| {
            *method == HttpMethod::Patch && path.starts_with("/v1/widgets/")
        });
    });

    assert_eq!(
        actual,
        expected("error-responses-status-only-expected.json")
    );
}

#[test]
fn status_and_header_rules_reproduce_a_product_decorator_with_headers() {
    let rules = [400, 413, 429, 500, 504]
        .into_iter()
        .map(|status| (When::Always, status))
        .chain([(When::Secured, 401), (When::Secured, 403)])
        .fold(ErrorResponseRules::new(), |rules, (condition, status)| {
            rules.status(condition, status, described(status))
        })
        .header(
            When::Always,
            ResponseSelector::Every,
            "x-request-id",
            string_header("x-request-id"),
        )
        .header(
            When::Always,
            ResponseSelector::Status(401),
            "WWW-Authenticate",
            string_header("WWW-Authenticate"),
        )
        .header(
            When::Always,
            ResponseSelector::Status(429),
            "Retry-After",
            header(
                "Retry-After",
                ObjectBuilder::new()
                    .schema_type(Type::Integer)
                    .minimum(Some(0.0)),
            ),
        )
        .header(
            When::Secured,
            ResponseSelector::Every,
            "Cache-Control",
            string_header("Cache-Control"),
        );

    let actual = decorate("error-responses-with-headers-input.json", |document| {
        rules.apply(document);
    });

    assert_eq!(
        actual,
        expected("error-responses-with-headers-expected.json")
    );
}
