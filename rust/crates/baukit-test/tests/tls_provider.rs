#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    reason = "integration test code: a failed expectation is the test failure, and scenario tests read top to bottom"
)]
#[test]
fn the_dependency_graph_compiles_a_single_rustls_crypto_provider() {
    let _ = rustls::ClientConfig::builder();
    let _ = rustls::ServerConfig::builder();
}
