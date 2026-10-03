use {{ context.app_crate }}_api::openapi_document;

/// Names a standard defines, such as OAuth 2.0 `access_token`, that the API must keep as they are.
const STANDARD_DEFINED_NAMES: &[&str] = &[];

#[test]
fn committed_openapi_has_no_drift() {
    baukit_test::assert_openapi_no_drift(
        &openapi_document(),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../openapi.json"),
    );
}

#[test]
fn openapi_names_are_camel_case() {
    let document = openapi_document();
    baukit_test::assert_openapi_camel_case(&document, STANDARD_DEFINED_NAMES);
}
