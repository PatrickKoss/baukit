-- Reference schema for baukit-sync tombstone purge horizons.
--
-- Copy this file into the product's own ordered migrations after
-- 0001_baukit_sync.sql. baukit-sync::purge reads and writes this table by its
-- fixed name.
--
-- The horizon is the greatest revision of any tombstone purged for the owner.
-- A missing row means nothing was purged, so every cursor is valid. The foreign
-- key removes the horizon when the owner's revision counter is erased.

CREATE TABLE sync_purge_horizons (
    owner_id UUID PRIMARY KEY REFERENCES sync_revisions (owner_id) ON DELETE CASCADE,
    horizon_revision BIGINT NOT NULL CHECK (horizon_revision > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE sync_purge_horizons IS
    'Greatest purged tombstone revision per owner, raised by baukit-sync::purge';
