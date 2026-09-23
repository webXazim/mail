-- Track provider reconciliation per mailbox so large organizations cannot
-- starve later mailboxes when a worker processes a bounded batch.
ALTER TABLE mailboxes
  ADD COLUMN IF NOT EXISTS provider_reconciled_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS mailboxes_provider_reconcile_due_idx
  ON mailboxes(provider_reconciled_at, created_at)
  WHERE deleted_at IS NULL AND status <> 'deleting';
