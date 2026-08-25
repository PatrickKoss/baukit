-- Reference schema for baukit-sync.
--
-- Copy this file into the product's own ordered migrations. Products own
-- migration execution; baukit-sync deliberately does not migrate on startup.
--
-- Replace `owner_id`'s REFERENCES clause with the product's own owner table.
-- Baukit does not own that table and cannot name it here.

CREATE TABLE sync_revisions (
    owner_id UUID PRIMARY KEY,
    last_revision BIGINT NOT NULL DEFAULT 0 CHECK (last_revision >= 0)
);

COMMENT ON TABLE sync_revisions IS
    'Per-owner monotonic revision counter allocated by baukit-sync::next_revision';

-- Every syncable table carries these four columns:
--
--     id          UUID PRIMARY KEY,
--     owner_id    UUID NOT NULL REFERENCES <owner table>(id),
--     updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
--     deleted_at  TIMESTAMPTZ,
--     revision    BIGINT NOT NULL
--
-- `deleted_at` makes deletion a tombstone rather than a row removal, so a pull
-- can deliver the deletion to clients that have not seen it yet. `revision` is
-- allocated by `next_revision` in the same transaction as the row write, and
-- every syncable table needs this index so an incremental pull is a range scan
-- rather than a sort:
--
--     CREATE INDEX <table>_sync_idx ON <table> (owner_id, revision);
--
-- The example below is the shape to copy, not a table to create as-is.
--
-- CREATE TABLE product_records (
--     id UUID PRIMARY KEY,
--     owner_id UUID NOT NULL REFERENCES users (id),
--     updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
--     deleted_at TIMESTAMPTZ,
--     revision BIGINT NOT NULL
-- );
-- CREATE INDEX product_records_sync_idx ON product_records (owner_id, revision);
