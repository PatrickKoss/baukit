-- Reference schema for baukit-push's PostgresPendingReceiptStore.
--
-- Only products that poll receipts after the send need this table. Copy this
-- file into the product's own ordered migrations; baukit-push deliberately
-- does not migrate on startup.
--
-- Rows carry no owner. They hold a device token for at most the receipt
-- retention (24 hours at Expo) until the product's purge removes them.

CREATE TABLE push_pending_receipts (
    ticket_id TEXT PRIMARY KEY CHECK (
        octet_length(ticket_id) BETWEEN 1 AND 128 AND ticket_id ~ '^[!-~]+$'
    ),
    token TEXT NOT NULL CHECK (
        octet_length(token) BETWEEN 1 AND 512 AND token ~ '^[!-~]+$'
    ),
    sent_at TIMESTAMPTZ NOT NULL,
    due_at TIMESTAMPTZ NOT NULL
);

COMMENT ON TABLE push_pending_receipts IS
    'Push tickets awaiting a receipt poll through baukit-push::PostgresPendingReceiptStore; the token must never be logged';

CREATE INDEX push_pending_receipts_due_idx ON push_pending_receipts (due_at, ticket_id);

CREATE INDEX push_pending_receipts_sent_idx ON push_pending_receipts (sent_at, ticket_id);
