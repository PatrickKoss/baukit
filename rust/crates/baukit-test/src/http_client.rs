/// Starts a raw `reqwest` client builder for driving a server under test.
///
/// Test code only. Production code sends outbound requests through
/// `baukit_egress::GuardedClient`.
#[expect(
    clippy::disallowed_methods,
    reason = "test-support client that drives local servers under test; it never ships in a production request path"
)]
pub fn test_http_client() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
}
