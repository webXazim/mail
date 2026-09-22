-- Upgrade 26: External Mail Clients & Mailbox Migration.
--
-- Adds CS Mail metadata for Stalwart-native application passwords and durable
-- mailbox import jobs. Application-password secrets are deliberately never
-- stored in PostgreSQL; Stalwart returns each generated secret once at create
-- time and stores only its own credential hash.

CREATE TABLE IF NOT EXISTS mailbox_app_passwords (
  id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id        UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  mailbox_id             UUID NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
  user_id                UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  label                   TEXT NOT NULL,
  provider_credential_id TEXT NOT NULL,
  allowed_ips            TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
  expires_at              TIMESTAMPTZ,
  status                  TEXT NOT NULL DEFAULT 'active'
                          CHECK (status IN ('active','revoking','revoked','error')),
  last_error              TEXT NOT NULL DEFAULT '',
  created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
  revoked_at              TIMESTAMPTZ,
  updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT mailbox_app_passwords_label_nonempty CHECK (length(trim(label)) BETWEEN 1 AND 120),
  CONSTRAINT mailbox_app_passwords_provider_id_nonempty CHECK (length(trim(provider_credential_id)) > 0)
);
CREATE UNIQUE INDEX IF NOT EXISTS mailbox_app_passwords_provider_unique_idx
  ON mailbox_app_passwords(mailbox_id, provider_credential_id);
CREATE INDEX IF NOT EXISTS mailbox_app_passwords_mailbox_active_idx
  ON mailbox_app_passwords(mailbox_id, created_at DESC)
  WHERE status='active';
CREATE INDEX IF NOT EXISTS mailbox_app_passwords_user_idx
  ON mailbox_app_passwords(user_id, created_at DESC);

CREATE OR REPLACE FUNCTION cs_mail_app_password_scope_guard()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  mailbox_org UUID;
  mailbox_user UUID;
BEGIN
  SELECT organization_id,user_id INTO mailbox_org,mailbox_user
  FROM mailboxes WHERE id=NEW.mailbox_id AND deleted_at IS NULL;
  IF mailbox_org IS NULL OR mailbox_org <> NEW.organization_id THEN
    RAISE EXCEPTION 'app-password mailbox does not belong to organization';
  END IF;
  IF mailbox_user IS NULL OR mailbox_user <> NEW.user_id THEN
    RAISE EXCEPTION 'app-password mailbox is not assigned to user';
  END IF;
  RETURN NEW;
END $$;
DROP TRIGGER IF EXISTS mailbox_app_passwords_scope_guard ON mailbox_app_passwords;
CREATE TRIGGER mailbox_app_passwords_scope_guard
BEFORE INSERT OR UPDATE OF organization_id,mailbox_id,user_id ON mailbox_app_passwords
FOR EACH ROW EXECUTE FUNCTION cs_mail_app_password_scope_guard();

CREATE TABLE IF NOT EXISTS mailbox_imports (
  id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id   UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  mailbox_id        UUID NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
  user_id           UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  source_type       TEXT NOT NULL DEFAULT 'mbox' CHECK (source_type IN ('mbox')),
  original_filename TEXT NOT NULL DEFAULT 'mailbox.mbox',
  storage_key       TEXT NOT NULL,
  file_sha256       TEXT NOT NULL,
  byte_size         BIGINT NOT NULL DEFAULT 0 CHECK (byte_size >= 0),
  status            TEXT NOT NULL DEFAULT 'queued'
                    CHECK (status IN ('queued','running','completed','failed','cancelled')),
  total_messages    BIGINT NOT NULL DEFAULT 0 CHECK (total_messages >= 0),
  imported_messages BIGINT NOT NULL DEFAULT 0 CHECK (imported_messages >= 0),
  failed_messages   BIGINT NOT NULL DEFAULT 0 CHECK (failed_messages >= 0),
  last_error        TEXT NOT NULL DEFAULT '',
  attempts          INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  max_attempts      INTEGER NOT NULL DEFAULT 5 CHECK (max_attempts BETWEEN 1 AND 20),
  next_attempt_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  locked_by         UUID,
  locked_until      TIMESTAMPTZ,
  started_at        TIMESTAMPTZ,
  completed_at      TIMESTAMPTZ,
  created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT mailbox_imports_storage_key_nonempty CHECK (length(trim(storage_key)) > 0),
  CONSTRAINT mailbox_imports_sha256_nonempty CHECK (length(file_sha256) = 64)
);
CREATE INDEX IF NOT EXISTS mailbox_imports_mailbox_created_idx
  ON mailbox_imports(mailbox_id, created_at DESC);
CREATE INDEX IF NOT EXISTS mailbox_imports_claim_idx
  ON mailbox_imports(status, next_attempt_at, locked_until, created_at)
  WHERE status IN ('queued','running');

CREATE TABLE IF NOT EXISTS mailbox_import_messages (
  import_id          UUID NOT NULL REFERENCES mailbox_imports(id) ON DELETE CASCADE,
  message_ordinal    BIGINT NOT NULL CHECK (message_ordinal > 0),
  message_sha256     TEXT NOT NULL,
  provider_email_id  TEXT NOT NULL DEFAULT '',
  byte_size          BIGINT NOT NULL DEFAULT 0 CHECK (byte_size >= 0),
  status             TEXT NOT NULL DEFAULT 'imported' CHECK (status IN ('imported','failed')),
  error              TEXT NOT NULL DEFAULT '',
  created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY(import_id, message_ordinal),
  CONSTRAINT mailbox_import_messages_sha256 CHECK (length(message_sha256)=64)
);
CREATE INDEX IF NOT EXISTS mailbox_import_messages_hash_idx
  ON mailbox_import_messages(import_id, message_sha256);

CREATE OR REPLACE FUNCTION cs_mail_import_scope_guard()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  mailbox_org UUID;
  mailbox_user UUID;
BEGIN
  SELECT organization_id,user_id INTO mailbox_org,mailbox_user
  FROM mailboxes WHERE id=NEW.mailbox_id AND deleted_at IS NULL;
  IF mailbox_org IS NULL OR mailbox_org <> NEW.organization_id THEN
    RAISE EXCEPTION 'mailbox import does not belong to organization';
  END IF;
  IF mailbox_user IS NULL OR mailbox_user <> NEW.user_id THEN
    RAISE EXCEPTION 'mailbox import mailbox is not assigned to user';
  END IF;
  RETURN NEW;
END $$;
DROP TRIGGER IF EXISTS mailbox_imports_scope_guard ON mailbox_imports;
CREATE TRIGGER mailbox_imports_scope_guard
BEFORE INSERT OR UPDATE OF organization_id,mailbox_id,user_id ON mailbox_imports
FOR EACH ROW EXECUTE FUNCTION cs_mail_import_scope_guard();

COMMENT ON TABLE mailbox_app_passwords IS 'Metadata for Stalwart-native app passwords. Raw app-password secrets are never persisted by CS Mail.';
COMMENT ON TABLE mailbox_imports IS 'Durable MBOX import jobs for one concrete organization mailbox.';
COMMENT ON TABLE mailbox_import_messages IS 'Per-source-record import ledger keyed by import + ordinal; SHA-256 supports diagnostics and crash recovery without collapsing legitimate duplicate records.';
