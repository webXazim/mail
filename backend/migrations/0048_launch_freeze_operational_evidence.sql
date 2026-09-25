-- Upgrade 05: production launch-freeze operational evidence.
--
-- Public launch must be tied to the exact deployed release and to recoverable
-- data, not just application/provider health.  This append-only ledger is
-- populated by the production backup/restore scripts and by an explicit
-- operator command that records proof produced by the independent offsite
-- backup systems for CS Mail and the shared Stalwart provider.

CREATE TABLE IF NOT EXISTS operational_evidence (
  id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  kind             TEXT NOT NULL CHECK (kind IN (
                     'local_backup',
                     'restore_drill',
                     'cs_mail_offsite_backup',
                     'stalwart_offsite_backup'
                   )),
  status           TEXT NOT NULL DEFAULT 'passed' CHECK (status IN ('passed','failed')),
  release_sha256   TEXT NOT NULL DEFAULT '',
  artifact_ref     TEXT NOT NULL DEFAULT '',
  artifact_sha256  TEXT NOT NULL DEFAULT '',
  detail           JSONB NOT NULL DEFAULT '{}'::jsonb,
  recorded_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT operational_evidence_release_sha256_format CHECK (
    release_sha256 = '' OR release_sha256 ~ '^[0-9a-f]{64}$'
  ),
  CONSTRAINT operational_evidence_artifact_sha256_format CHECK (
    artifact_sha256 = '' OR artifact_sha256 ~ '^[0-9a-f]{64}$'
  )
);

CREATE INDEX IF NOT EXISTS operational_evidence_kind_recorded_idx
  ON operational_evidence(kind, recorded_at DESC);
CREATE INDEX IF NOT EXISTS operational_evidence_release_idx
  ON operational_evidence(release_sha256, recorded_at DESC)
  WHERE release_sha256 <> '';

-- Evidence is intentionally append-only.  Recovery history must never be
-- rewritten in place after public launch.
CREATE OR REPLACE FUNCTION cs_mail_operational_evidence_immutable()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'operational_evidence is append-only';
END $$;

DROP TRIGGER IF EXISTS operational_evidence_no_update_delete ON operational_evidence;
CREATE TRIGGER operational_evidence_no_update_delete
BEFORE UPDATE OR DELETE ON operational_evidence
FOR EACH ROW EXECUTE FUNCTION cs_mail_operational_evidence_immutable();
