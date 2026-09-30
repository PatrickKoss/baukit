-- Reference migration for baukit-push's PostgresPendingReceiptStore, applied
-- after 0003_baukit_push_pending_receipts.sql.
--
-- Copy this file into the product's own ordered migrations; baukit-push
-- deliberately does not migrate on startup.
--
-- Every pending ticket records the owner it was sent to, so erasing an owner
-- removes the device tokens their tickets hold at once. Tickets recorded
-- before this migration have no owner and are dropped; a dead token among
-- them is reported again by the next send to it.
--
-- Join ownership in a product migration so erasing an owner erases their
-- pending tickets:
--
--     ALTER TABLE push_pending_receipts
--         ADD CONSTRAINT push_pending_receipts_owner_fk
--         FOREIGN KEY (owner_id) REFERENCES <owner table> (id) ON DELETE CASCADE;

DELETE FROM push_pending_receipts;

ALTER TABLE push_pending_receipts ADD COLUMN owner_id UUID NOT NULL;

CREATE INDEX push_pending_receipts_owner_idx ON push_pending_receipts (owner_id);
