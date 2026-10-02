#[test]
fn the_dependency_graph_compiles_a_single_rustls_crypto_provider() {
    let _ = rustls::ClientConfig::builder();
    let _ = rustls::ServerConfig::builder();
}
