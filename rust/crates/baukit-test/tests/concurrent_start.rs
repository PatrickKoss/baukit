#![cfg(test)]
use std::error::Error;

const CONCURRENT_STARTS: usize = 8;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "requires Docker"]
async fn first_container_starts_race_free() -> Result<(), Box<dyn Error>> {
    let starts = (0..CONCURRENT_STARTS)
        .map(|_| tokio::spawn(baukit_test::start_redis()))
        .collect::<Vec<_>>();
    for start in starts {
        let container = start.await??;
        assert!(container.connection_url().starts_with("redis://"));
    }
    Ok(())
}
