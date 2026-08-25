#[test]
fn committed_openapi_has_no_drift() {
    baukit_test::assert_openapi_no_drift(
        &{{ context.app_crate }}_api::openapi_document(),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../openapi.json"),
    );
}
