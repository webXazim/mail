-- Upgrade 50: attachment object-storage authority.
-- Existing rows are local-volume objects; new uploads record the backend that
-- owns their bytes so local and Cloudflare R2 objects can coexist safely.

ALTER TABLE staged_attachments
  ADD COLUMN IF NOT EXISTS storage_backend TEXT NOT NULL DEFAULT 'local';

ALTER TABLE staged_attachments
  DROP CONSTRAINT IF EXISTS staged_attachments_storage_backend_check;
ALTER TABLE staged_attachments
  ADD CONSTRAINT staged_attachments_storage_backend_check
  CHECK (storage_backend IN ('local', 'r2'));

COMMENT ON COLUMN staged_attachments.storage_backend IS
  'Physical attachment byte store. local=/srv/attachments, r2=private Cloudflare R2 bucket.';
