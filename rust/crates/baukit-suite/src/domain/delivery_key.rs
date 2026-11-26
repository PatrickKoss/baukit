/// Deliveries to one peer app that may run at once; see [`suite_delivery_concurrency_key`].
pub const SUITE_DELIVERY_MAX_PER_KEY: u32 = 1;

/// The `concurrency_key` of every delivery job to `peer_app`.
///
/// A worker with `WorkerConfig::max_per_key` set to [`SUITE_DELIVERY_MAX_PER_KEY`]
/// gives a hanging peer at most that many slots.
pub fn suite_delivery_concurrency_key(peer_app: &str) -> String {
    format!("suite-peer:{peer_app}")
}
