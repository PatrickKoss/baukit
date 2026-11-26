-- Reference migration for baukit-push's PostgresDeliveryClaimStore, applied after
-- 0005_baukit_push_install_secrets.sql.
--
-- Copy this file into the product's own ordered migrations; baukit-push
-- deliberately does not migrate on startup.
--
-- Each claim stores the token its winner received. Release deletes only the row
-- that still carries that token, so a stale claimer cannot drop a newer claim.
-- Claims made before this migration have no token and are never released; they
-- expire through purge like any other past claim.
--
-- A nullable column with no default does not rewrite the table.

SET lock_timeout = '5s';
SET statement_timeout = '1min';

ALTER TABLE push_delivery_claims ADD COLUMN IF NOT EXISTS claim_id UUID;
