-- Vault v2 record ids for browser sessions, copied from baukit-auth
-- migrations/0003_baukit_auth_browser_session_record_ids.sql.
--
-- Rows written with record_id seal refresh_token, id_token and
-- grace_access_token as one record, so a column cannot be dropped or replaced
-- from another row. Rows written before this migration keep record_id NULL and
-- open through the v1 path until they expire.
--
-- A nullable column with no default does not rewrite the table.

SET lock_timeout = '5s';
SET statement_timeout = '1min';

ALTER TABLE baukit_auth_browser_sessions ADD COLUMN IF NOT EXISTS record_id UUID;
