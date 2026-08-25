-- One-shot upgrade for products whose sync_revisions table predates the
-- owner_id convention. Copy this into the product's ordered migrations only
-- when the table still has user_id and the conventional foreign-key name.

ALTER TABLE sync_revisions RENAME COLUMN user_id TO owner_id;
ALTER TABLE sync_revisions RENAME CONSTRAINT sync_revisions_user_id_fkey
    TO sync_revisions_owner_id_fkey;
ALTER TABLE sync_revisions
    ADD CONSTRAINT sync_revisions_last_revision_check CHECK (last_revision >= 0);

COMMENT ON TABLE sync_revisions IS
    'Per-owner monotonic revision counter allocated by baukit-sync::next_revision';
