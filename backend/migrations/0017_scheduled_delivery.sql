-- Upgrade 10: production scheduled-delivery queue.
--
-- scheduled_sends is now a durable leased job queue. Multiple API instances may
-- claim work concurrently through FOR UPDATE SKIP LOCKED. Delivery itself still
-- uses the Upgrade-09 mail_send_requests ledger with the deterministic key
-- `scheduled:<scheduled_sends.id>`, so reclaiming an expired lease never creates
-- a new logical SMTP submission.

ALTER TABLE scheduled_sends
  ADD COLUMN IF NOT EXISTS idempotency_key TEXT,
  ADD COLUMN IF NOT EXISTS request_hash TEXT,
  ADD COLUMN IF NOT EXISTS attempt_count INTEGER NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS last_attempt_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS claimed_by UUID,
  ADD COLUMN IF NOT EXISTS lease_until TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS completed_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS cancelled_at TIMESTAMPTZ;

-- Preserve history from the older pending -> sent|failed worker.
UPDATE scheduled_sends SET status = 'dead' WHERE status = 'failed';
UPDATE scheduled_sends
SET status = 'dead',
    error = CASE WHEN error = '' THEN 'Unknown legacy scheduled-send status' ELSE error END
WHERE status NOT IN ('pending','processing','retry','sent','dead','cancelled');

UPDATE scheduled_sends
SET idempotency_key = 'legacy:' || id::text
WHERE idempotency_key IS NULL OR btrim(idempotency_key) = '';

-- Existing rows were never created through an idempotent schedule request, so
-- this hash is only a stable migration fingerprint; new rows use the canonical
-- Rust request hash.
UPDATE scheduled_sends
SET request_hash = encode(
      digest(convert_to(send_at::text || E'\n' || compose::text, 'UTF8'), 'sha256'),
      'hex'
    )
WHERE request_hash IS NULL OR btrim(request_hash) = '';

UPDATE scheduled_sends
SET next_attempt_at = send_at
WHERE next_attempt_at IS NULL AND status IN ('pending','retry');

UPDATE scheduled_sends
SET completed_at = COALESCE(completed_at, updated_at)
WHERE completed_at IS NULL AND status IN ('sent','dead','cancelled');

ALTER TABLE scheduled_sends
  ALTER COLUMN idempotency_key SET NOT NULL,
  ALTER COLUMN request_hash SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS scheduled_sends_user_idempotency_idx
  ON scheduled_sends(user_id, idempotency_key);

DROP INDEX IF EXISTS scheduled_sends_due_idx;
CREATE INDEX IF NOT EXISTS scheduled_sends_claim_idx
  ON scheduled_sends(next_attempt_at, send_at)
  WHERE status IN ('pending','retry');
CREATE INDEX IF NOT EXISTS scheduled_sends_expired_lease_idx
  ON scheduled_sends(lease_until)
  WHERE status = 'processing';
CREATE INDEX IF NOT EXISTS scheduled_sends_status_updated_idx
  ON scheduled_sends(status, updated_at);

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'scheduled_sends_status_check'
  ) THEN
    ALTER TABLE scheduled_sends
      ADD CONSTRAINT scheduled_sends_status_check
      CHECK (status IN ('pending','processing','retry','sent','dead','cancelled'));
  END IF;
END $$;
