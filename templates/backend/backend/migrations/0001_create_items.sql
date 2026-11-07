CREATE TABLE IF NOT EXISTS items (
    id UUID PRIMARY KEY,
    owner_id TEXT NOT NULL CHECK (length(owner_id) > 0),
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS items_owner_id_id_idx ON items (owner_id, id);
