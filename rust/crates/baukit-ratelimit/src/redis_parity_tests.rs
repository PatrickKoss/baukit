use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use proptest::{
    prelude::*,
    test_runner::{Config, TestCaseError, TestRunner},
};

use crate::{
    Quota,
    kernels::bucket::{ScaledQuota, admit},
    redis_scripts::TOKEN_BUCKET_SCRIPT_AT,
};

const CASES: u32 = 64;
const START_MS: u64 = 1_700_000_000_000;

fn trace() -> impl Strategy<Value = (u64, u64, u64, Vec<u64>)> {
    (
        1_u64..50,
        0_u64..10,
        1_u64..120_000,
        prop::collection::vec(0_u64..5_000, 1..60),
    )
}

async fn lua_step(
    connection: &mut redis::aio::MultiplexedConnection,
    key: &str,
    scaled: ScaledQuota,
    now_ms: u64,
) -> redis::RedisResult<(i64, u64, u64)> {
    redis::cmd("EVAL")
        .arg(TOKEN_BUCKET_SCRIPT_AT)
        .arg(1)
        .arg(key)
        .arg(scaled.capacity)
        .arg(scaled.rate)
        .arg(scaled.cost)
        .arg(60_000)
        .arg(now_ms)
        .query_async(connection)
        .await
}

/// P-04: the Lua script and `bucket::admit` make the same decisions on the same trace.
#[test]
#[ignore = "requires Docker; mandatory in the full local gate"]
fn lua_token_bucket_matches_the_rust_kernel() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let fixture = runtime.block_on(baukit_test::start_redis()).expect("redis");
    let client = redis::Client::open(fixture.connection_url()).expect("client");
    let connection = runtime
        .block_on(client.get_multiplexed_async_connection())
        .expect("connection");
    let cases = AtomicU64::new(0);

    let result = TestRunner::new(Config::with_cases(CASES)).run(
        &trace(),
        |(requests, burst, period_ms, steps)| {
            let key = format!("parity:{}", cases.fetch_add(1, Ordering::Relaxed));
            let mut connection = connection.clone();
            let quota = Quota::new(requests, Duration::from_millis(period_ms), burst)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let scaled = ScaledQuota::from_quota(quota);
            let (mut tokens, mut last_ms, mut now_ms) = (scaled.capacity, START_MS, START_MS);
            for step in steps {
                now_ms += step;
                let (allowed, after, retry_ms) = admit(
                    tokens,
                    last_ms,
                    now_ms,
                    scaled.rate,
                    scaled.capacity,
                    scaled.cost,
                );
                (tokens, last_ms) = (after, now_ms.max(last_ms));
                let lua = runtime
                    .block_on(lua_step(&mut connection, &key, scaled, now_ms))
                    .map_err(|error| TestCaseError::fail(error.to_string()))?;
                prop_assert_eq!(lua, (i64::from(allowed), after / scaled.cost, retry_ms));
            }
            Ok(())
        },
    );
    runtime.block_on(async move { drop(fixture) });
    result.expect("Lua and Rust token buckets agree");
}
