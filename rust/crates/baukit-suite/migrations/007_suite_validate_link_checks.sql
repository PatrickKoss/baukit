-- Copy after 006_suite_delivery_probes.sql.
--
-- Validates the checks that 006 added NOT VALID. It runs in its own transaction, so the scan
-- holds only a SHARE UPDATE EXCLUSIVE lock and writes continue.

SET lock_timeout = '5s';
SET statement_timeout = '15min';

ALTER TABLE suite_links VALIDATE CONSTRAINT suite_links_consecutive_failures_nonnegative;
ALTER TABLE suite_links VALIDATE CONSTRAINT suite_links_failed_probes_check;
