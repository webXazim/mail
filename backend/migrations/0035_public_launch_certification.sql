-- Upgrade 28: public SaaS launch certification ledger.
--
-- Certification is performed by deploy/production/certify-launch.sh against
-- the real production topology. The ledger records the immutable release hash,
-- overall result and report digest so an operator can prove which build passed
-- the launch gate without storing protocol/account secrets in PostgreSQL.

CREATE TABLE IF NOT EXISTS launch_certification_runs (
  id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  release_label       TEXT NOT NULL,
  release_sha256      TEXT NOT NULL,
  environment         TEXT NOT NULL DEFAULT 'production',
  status              TEXT NOT NULL CHECK (status IN ('running','passed','failed','aborted')),
  report_sha256       TEXT NOT NULL DEFAULT '',
  report_path         TEXT NOT NULL DEFAULT '',
  mandatory_passed    INTEGER NOT NULL DEFAULT 0 CHECK (mandatory_passed >= 0),
  mandatory_failed    INTEGER NOT NULL DEFAULT 0 CHECK (mandatory_failed >= 0),
  optional_skipped    INTEGER NOT NULL DEFAULT 0 CHECK (optional_skipped >= 0),
  started_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
  completed_at        TIMESTAMPTZ,
  created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT launch_certification_release_sha256_format
    CHECK (release_sha256 ~ '^[0-9a-f]{64}$'),
  CONSTRAINT launch_certification_report_sha256_format
    CHECK (report_sha256 = '' OR report_sha256 ~ '^[0-9a-f]{64}$'),
  CONSTRAINT launch_certification_completion_consistency CHECK (
    (status = 'running' AND completed_at IS NULL)
    OR (status IN ('passed','failed','aborted') AND completed_at IS NOT NULL)
  )
);

CREATE INDEX IF NOT EXISTS launch_certification_runs_created_idx
  ON launch_certification_runs(created_at DESC);
CREATE INDEX IF NOT EXISTS launch_certification_runs_status_idx
  ON launch_certification_runs(status, created_at DESC);
CREATE INDEX IF NOT EXISTS launch_certification_runs_release_idx
  ON launch_certification_runs(release_sha256, created_at DESC);

-- Only an all-mandatory PASS may be recorded as passed.
CREATE OR REPLACE FUNCTION cs_mail_launch_certification_guard()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.status = 'passed' AND NEW.mandatory_failed <> 0 THEN
    RAISE EXCEPTION 'passed launch certification cannot contain mandatory failures';
  END IF;
  IF NEW.status = 'passed' AND (NEW.report_sha256 = '' OR NEW.report_path = '') THEN
    RAISE EXCEPTION 'passed launch certification requires a report digest and path';
  END IF;
  RETURN NEW;
END $$;

DROP TRIGGER IF EXISTS launch_certification_guard ON launch_certification_runs;
CREATE TRIGGER launch_certification_guard
BEFORE INSERT OR UPDATE ON launch_certification_runs
FOR EACH ROW EXECUTE FUNCTION cs_mail_launch_certification_guard();
