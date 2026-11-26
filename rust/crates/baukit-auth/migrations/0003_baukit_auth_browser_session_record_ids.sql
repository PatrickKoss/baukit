-- Reference migration for baukit-auth's browser sessions, applied after
-- 0002_baukit_auth_browser_sessions.sql. Copy it into the product's own ordered
-- migrations; baukit-auth does not migrate on startup.
--
-- record_id is the credential-vault v2 record id. Rows written with it seal
-- refresh_token, id_token and grace_access_token as one record, so a column
-- cannot be dropped or replaced from another row. Rows written before this
-- migration keep record_id NULL and open through the v1 path until they expire.
--
-- A nullable column with no default does not rewrite the table.

SET lock_timeout = '5s';
SET statement_timeout = '1min';

ALTER TABLE baukit_auth_browser_sessions ADD COLUMN IF NOT EXISTS record_id UUID;
