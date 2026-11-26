-- Reference migration for baukit-push's PostgresDeviceRegistry, applied after
-- 0006_baukit_push_delivery_claim_ids.sql.
--
-- Copy this file into the product's own ordered migrations; baukit-push
-- deliberately does not migrate on startup.
--
-- Validates the install secret length check that 0005 added NOT VALID. It runs
-- in its own transaction, so the scan holds only a SHARE UPDATE EXCLUSIVE lock
-- and writes continue.

SET lock_timeout = '5s';
SET statement_timeout = '15min';

ALTER TABLE push_devices VALIDATE CONSTRAINT push_devices_install_secret_hash_length;
