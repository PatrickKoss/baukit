use std::{net::SocketAddr, time::Duration};

use axum::{Router, extract::ConnectInfo, routing::get};
use baukit_runtime::{ShutdownToken, serve_listener_pair};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

async fn peer(ConnectInfo(addr): ConnectInfo<SocketAddr>) -> String {
    addr.ip().to_string()
}

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

async fn get_body(addr: SocketAddr, path: &str) -> TestResult<String> {
    let mut stream = TcpStream::connect(addr).await?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    Ok(response)
}

#[tokio::test]
async fn both_listeners_expose_peer_connect_info() -> TestResult {
    let api = TcpListener::bind("127.0.0.1:0").await?;
    let ops = TcpListener::bind("127.0.0.1:0").await?;
    let api_addr = api.local_addr()?;
    let ops_addr = ops.local_addr()?;
    let shutdown = ShutdownToken::new(Duration::from_secs(5));
    let server = tokio::spawn(serve_listener_pair(
        api,
        Router::new().route("/peer", get(peer)),
        ops,
        Router::new().route("/peer", get(peer)),
        shutdown.clone(),
    ));

    let api_response = get_body(api_addr, "/peer").await?;
    let ops_response = get_body(ops_addr, "/peer").await?;
    shutdown.trigger();
    server.await??;

    for response in [api_response, ops_response] {
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with("127.0.0.1"), "{response}");
    }
    Ok(())
}
