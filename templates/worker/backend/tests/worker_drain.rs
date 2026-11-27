use std::{error::Error, time::Duration};

use baukit_config::ShutdownConfig;

use {{ context.app_crate }}_bin::WorkerProductConfig;
use {{ context.app_crate }}_bin::worker_drain_budget;

const CHART_WORKER_DRAIN: Duration = Duration::from_secs(45);

#[test]
fn default_worker_settings_fit_the_default_drain() -> Result<(), Box<dyn Error>> {
    let worker = WorkerProductConfig::default().worker_config();
    worker_drain_budget(ShutdownConfig::default().drain_timeout, &worker)?;
    worker_drain_budget(CHART_WORKER_DRAIN, &worker)?;
    Ok(())
}

#[test]
fn a_drain_shorter_than_the_release_reserve_is_rejected() {
    let worker = WorkerProductConfig::default().worker_config();
    assert!(worker_drain_budget(worker.release_reserve, &worker).is_err());
}
