use std::sync::Once;

use tracing_subscriber::EnvFilter;

static INIT: Once = Once::new();

/// Installs a compact, environment-filtered tracing subscriber for tests.
///
/// The first call attempts process-wide installation. Later calls are no-ops,
/// including when tests call this concurrently. If another subscriber was
/// installed first, this helper leaves it in place. No OpenTelemetry exporter
/// is configured. Like every global tracing subscriber, it cannot be reset;
/// tests that initialize full `baukit-telemetry` should instead consolidate all
/// telemetry assertions into one process-wide contract test.
pub fn init_test_tracing() {
    INIT.call_once(|| {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let _already_installed = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_test_writer()
            .compact()
            .try_init();
    });
}

#[cfg(test)]
mod tests {
    use super::init_test_tracing;

    #[test]
    fn tracing_init_is_idempotent() {
        init_test_tracing();
        init_test_tracing();
    }
}
