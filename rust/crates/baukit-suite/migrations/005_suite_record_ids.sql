-- Copy after 004_platform.sql.
--
-- *_record_id is the credential-vault v2 record id of the matching ciphertext.
-- Rows written before this migration keep it NULL and open through the v1 path.
-- Clearing a sealed secret clears its record id with it.
--
-- Nullable columns with no default do not rewrite the tables.

SET lock_timeout = '5s';
SET statement_timeout = '1min';

ALTER TABLE suite_links ADD COLUMN IF NOT EXISTS secret_record_id uuid;
ALTER TABLE suite_link_requests ADD COLUMN IF NOT EXISTS verifier_record_id uuid;
ALTER TABLE suite_link_requests ADD COLUMN IF NOT EXISTS secret_record_id uuid;
