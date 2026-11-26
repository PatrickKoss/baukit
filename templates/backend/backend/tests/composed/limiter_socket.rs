use std::{error::Error, net::SocketAddr, time::Duration};

use axum::Router;
use baukit_runtime::{ShutdownToken, serve_listener_pair};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpSocket},
};

use crate::support::{self, Quota};

const DRAIN: Duration = Duration::from_secs(1);
const TOO_MANY_REQUESTS: u16 = 429;

/// G-02: the limiter keys on the real socket peer when served like the binary serves it.
#[tokio::test]
async fn exhausting_one_client_ip_leaves_another_unthrottled() -> Result<(), Box<dyn Error>> {
    let composed = support::compose(&Quota::ONE{% if context.auth_enabled %}, None).await{% else %}){% endif %}?;
    let api_listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = api_listener.local_addr()?;
    let ops_listener = TcpListener::bind("127.0.0.1:0").await?;
    let shutdown = ShutdownToken::new(DRAIN);
    let server = tokio::spawn(serve_listener_pair(
        api_listener,
        composed.api.router,
        ops_listener,
        Router::new(),
        shutdown.clone(),
    ));

    let first = status_from("127.0.0.1", address).await?;
    let exhausted = status_from("127.0.0.1", address).await?;
    let other = status_from("127.0.0.2", address).await?;
    shutdown.trigger();
    let _server = server.await;

    assert_ne!(first, TOO_MANY_REQUESTS, "the first request has quota left");
    assert_eq!(exhausted, TOO_MANY_REQUESTS, "127.0.0.1 spent its quota");
    assert_ne!(
        other, TOO_MANY_REQUESTS,
        "127.0.0.2 shares no bucket with 127.0.0.1"
    );
{% if not context.auth_enabled %}    assert_eq!(other, 200);
{% endif %}    Ok(())
}

async fn status_from(local: &str, server: SocketAddr) -> Result<u16, Box<dyn Error>> {
    let socket = TcpSocket::new_v4()?;
    socket.bind(SocketAddr::new(local.parse()?, 0))?;
    let mut stream = socket.connect(server).await?;
    stream
        .write_all(b"GET /items HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let head = String::from_utf8_lossy(&response);
    let status = head
        .split_whitespace()
        .nth(1)
        .ok_or("response has no status line")?;
    Ok(status.parse()?)
}
