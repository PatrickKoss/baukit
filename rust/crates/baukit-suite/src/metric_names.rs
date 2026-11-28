//! Fixed metric names. Product identity comes from the scrape labels.

/// Peer delivery attempts, labelled `outcome`.
pub const SUITE_DELIVERIES_TOTAL: &str = "suite_deliveries_total";
/// Inbound event ingestion, labelled `outcome`.
pub const SUITE_EVENTS_INGESTED_TOTAL: &str = "suite_events_ingested_total";
/// Live emissions skipped before the outbox, labelled `reason`.
pub const SUITE_EMISSION_SKIPPED_TOTAL: &str = "suite_emission_skipped_total";
