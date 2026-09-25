-- User quota fields are compatibility mirrors. Subscription and explicit
-- mailbox allocations are authoritative; a legacy user update must never
-- overwrite a mailbox's custom allocation or trip the storage-pool guard.
CREATE OR REPLACE FUNCTION sync_primary_mailbox_from_user() RETURNS trigger AS $$
BEGIN
  IF NEW.primary_mailbox_id IS NOT NULL THEN
    UPDATE mailboxes
    SET provider_account_id = NULLIF(NEW.mail_account_id, ''),
        sync_status = CASE NEW.mail_sync_status
          WHEN 'ready' THEN 'ready'
          WHEN 'retrying' THEN 'retrying'
          WHEN 'error' THEN 'error'
          WHEN 'none' THEN 'none'
          ELSE 'pending'
        END,
        sync_error = NEW.mail_sync_error,
        status = CASE
          WHEN NEW.status = 'suspended' THEN 'suspended'
          WHEN NEW.mail_sync_status = 'ready' THEN 'active'
          WHEN NEW.mail_sync_status = 'error' THEN 'error'
          ELSE 'pending'
        END,
        updated_at = now()
    WHERE id = NEW.primary_mailbox_id;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS users_primary_mailbox_compat_sync ON users;
CREATE TRIGGER users_primary_mailbox_compat_sync
AFTER UPDATE OF mail_account_id, mail_sync_status, mail_sync_error, status ON users
FOR EACH ROW EXECUTE FUNCTION sync_primary_mailbox_from_user();
