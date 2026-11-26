#![cfg(feature = "sqlx-postgres")]
use std::{
    error::Error,
    time::{Duration, Instant},
};

use baukit_test::{
    PostgresTestOptions, Toxic, ToxicDirection, start_redis, start_redis_with_stable_port,
    start_toxiproxy,
};
use sqlx::{Connection as _, PgConnection};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
};

type TestResult = Result<(), Box<dyn Error>>;

const STALLED_QUERY_BUDGET: Duration = Duration::from_secs(2);
const INJECTED_LATENCY: Duration = Duration::from_millis(500);
const TIMEOUT_TOXIC_CLOSE: Duration = Duration::from_millis(300);
const PONG: &[u8] = b"+PONG\r\n";

async fn select_one(connection: &mut PgConnection) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar("SELECT 1").fetch_one(connection).await
}

async fn ping(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    stream.write_all(b"PING\r\n").await?;
    let mut reply = vec![0; PONG.len()];
    stream.read_exact(&mut reply).await?;
    Ok(reply)
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn postgres_query_stalls_while_paused_and_works_after_resume() -> TestResult {
    let postgres = PostgresTestOptions::new().start().await?;
    let mut connection = PgConnection::connect(postgres.connection_url()).await?;
    assert_eq!(select_one(&mut connection).await?, 1);

    postgres.fault()?.pause().await?;
    let stalled = tokio::time::timeout(STALLED_QUERY_BUDGET, select_one(&mut connection)).await;
    assert!(stalled.is_err(), "a paused server must not answer");
    postgres.fault()?.resume().await?;

    let mut fresh = PgConnection::connect(postgres.connection_url()).await?;
    assert_eq!(select_one(&mut fresh).await?, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn postgres_restart_keeps_url_and_data() -> TestResult {
    let postgres = PostgresTestOptions::new()
        .with_stable_host_port()
        .start()
        .await?;
    let mut connection = PgConnection::connect(postgres.connection_url()).await?;
    sqlx::query("CREATE TABLE survives_restart (id int)")
        .execute(&mut connection)
        .await?;
    drop(connection);

    postgres.fault()?.stop().await?;
    assert!(
        PgConnection::connect(postgres.connection_url())
            .await
            .is_err(),
        "a stopped server must refuse connections"
    );
    postgres.fault()?.start().await?;

    let mut connection = PgConnection::connect(postgres.connection_url()).await?;
    let tables: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE tablename = 'survives_restart'")
            .fetch_one(&mut connection)
            .await?;
    assert_eq!(tables, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn redis_restart_keeps_url() -> TestResult {
    let redis = start_redis_with_stable_port().await?;
    let address = redis
        .connection_url()
        .trim_start_matches("redis://")
        .trim_end_matches('/')
        .to_owned();
    redis.fault().stop().await?;
    assert!(TcpStream::connect(&address).await.is_err());
    redis.fault().start().await?;
    let mut stream = TcpStream::connect(&address).await?;
    assert_eq!(ping(&mut stream).await?, PONG);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn redis_ping_stalls_while_paused() -> TestResult {
    let redis = start_redis().await?;
    let address = redis
        .connection_url()
        .trim_start_matches("redis://")
        .trim_end_matches('/')
        .to_owned();
    let mut stream = TcpStream::connect(&address).await?;
    assert_eq!(ping(&mut stream).await?, PONG);
    redis.fault().pause().await?;
    assert!(
        tokio::time::timeout(STALLED_QUERY_BUDGET, ping(&mut stream))
            .await
            .is_err()
    );
    redis.fault().resume().await?;
    let mut fresh = TcpStream::connect(&address).await?;
    assert_eq!(ping(&mut fresh).await?, PONG);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn toxiproxy_latency_delays_replies() -> TestResult {
    let redis = start_redis().await?;
    let toxiproxy = start_toxiproxy().await?;
    let proxy = toxiproxy
        .proxy_container("redis", redis.container(), 6379)
        .await?;
    let mut stream = TcpStream::connect(proxy.address()).await?;
    assert_eq!(ping(&mut stream).await?, PONG);

    let toxic = proxy
        .add_toxic(
            Toxic::Latency {
                latency: INJECTED_LATENCY,
                jitter: Duration::ZERO,
            },
            ToxicDirection::Downstream,
        )
        .await?;
    let started = Instant::now();
    assert_eq!(ping(&mut stream).await?, PONG);
    assert!(started.elapsed() >= INJECTED_LATENCY);

    proxy.remove_toxic(&toxic).await?;
    let started = Instant::now();
    assert_eq!(ping(&mut stream).await?, PONG);
    assert!(started.elapsed() < INJECTED_LATENCY);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn toxiproxy_reset_peer_resets_the_connection() -> TestResult {
    let redis = start_redis().await?;
    let toxiproxy = start_toxiproxy().await?;
    let proxy = toxiproxy
        .proxy_container("redis", redis.container(), 6379)
        .await?;
    proxy
        .add_toxic(
            Toxic::ResetPeer {
                after: Duration::ZERO,
            },
            ToxicDirection::Upstream,
        )
        .await?;
    let mut stream = TcpStream::connect(proxy.address()).await?;
    let error = ping(&mut stream).await.err().ok_or("ping must fail")?;
    assert!(
        matches!(
            error.kind(),
            std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::UnexpectedEof
        ),
        "unexpected error kind: {error}"
    );

    toxiproxy.reset().await?;
    let mut stream = TcpStream::connect(proxy.address()).await?;
    assert_eq!(ping(&mut stream).await?, PONG);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn toxiproxy_timeout_holds_then_closes_the_connection() -> TestResult {
    let redis = start_redis().await?;
    let toxiproxy = start_toxiproxy().await?;
    let proxy = toxiproxy
        .proxy_container("redis", redis.container(), 6379)
        .await?;
    proxy
        .add_toxic(
            Toxic::Timeout {
                after: TIMEOUT_TOXIC_CLOSE,
            },
            ToxicDirection::Downstream,
        )
        .await?;
    let started = Instant::now();
    let mut stream = TcpStream::connect(proxy.address()).await?;
    let reply = tokio::time::timeout(STALLED_QUERY_BUDGET, ping(&mut stream)).await?;
    assert!(
        reply.is_err(),
        "no reply must arrive through a timeout toxic"
    );
    assert!(started.elapsed() >= TIMEOUT_TOXIC_CLOSE);
    Ok(())
}
