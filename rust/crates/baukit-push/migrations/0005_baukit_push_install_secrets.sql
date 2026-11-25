-- Reference migration for baukit-push's PostgresDeviceRegistry, applied after
-- 0004_baukit_push_pending_receipt_owners.sql.
--
-- Copy this file into the product's own ordered migrations; baukit-push
-- deliberately does not migrate on startup.
--
-- Each device row can store the SHA-256 digest of its installation's secret.
-- A token moves to another owner only when that owner presents the same
-- secret. Existing rows keep a NULL digest and still move without one until
-- the installation registers again with a secret.
--
-- The column is nullable with no default, so adding it does not rewrite the
-- table. The check starts NOT VALID; 0007 validates it in its own transaction
-- without blocking writes.

SET lock_timeout = '5s';
SET statement_timeout = '1min';

ALTER TABLE push_devices ADD COLUMN IF NOT EXISTS install_secret_hash BYTEA;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'push_devices'::regclass
          AND conname = 'push_devices_install_secret_hash_length'
    ) THEN
        ALTER TABLE push_devices
            ADD CONSTRAINT push_devices_install_secret_hash_length
            CHECK (install_secret_hash IS NULL OR octet_length(install_secret_hash) = 32) NOT VALID;
    END IF;
END
$$;
