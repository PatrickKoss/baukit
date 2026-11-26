-- The store counts failed delivery attempts against SUITE_CIRCUIT_FAILURES when each delivery
-- finishes, so terminal job failures are no longer accounted in SQL.
DROP TRIGGER IF EXISTS suite_delivery_failed ON job_outbox;
DROP FUNCTION IF EXISTS account_suite_delivery_failures(uuid);
DROP FUNCTION IF EXISTS record_suite_delivery_failure();
DROP TABLE IF EXISTS suite_failed_delivery_jobs;

ALTER TABLE suite_links
    DROP CONSTRAINT IF EXISTS suite_links_consecutive_failures_check,
    ADD CONSTRAINT suite_links_consecutive_failures_check CHECK (consecutive_failures >= 0),
    ADD COLUMN next_probe_at timestamptz,
    ADD COLUMN failed_probes integer NOT NULL DEFAULT 0 CHECK (failed_probes >= 0);

UPDATE suite_links SET next_probe_at = now() + interval '1 hour'
    WHERE delivery_health = 'disabled' AND status <> 'revoked';
