-- Upgrade 21: verified-domain provider provisioning and DNS readiness.
-- Ownership verification (0027) remains separate from provider activation.

-- Verified challenges are no longer needed after ownership has been proven.
UPDATE organization_domains
SET verification_token=NULL, verification_expires_at=NULL
WHERE verified_at IS NOT NULL;

ALTER TABLE organization_domains
  ADD COLUMN IF NOT EXISTS provider_marker TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS provider_synced_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS dns_zone_file TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS dns_expected JSONB NOT NULL DEFAULT '{}'::jsonb,
  ADD COLUMN IF NOT EXISTS dns_observed JSONB NOT NULL DEFAULT '{}'::jsonb,
  ADD COLUMN IF NOT EXISTS dns_mx_ready BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN IF NOT EXISTS dns_spf_ready BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN IF NOT EXISTS dns_dkim_ready BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN IF NOT EXISTS dns_dmarc_ready BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN IF NOT EXISTS dns_ready BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN IF NOT EXISTS last_dns_readiness_check TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS next_dns_check_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS dns_check_attempts INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS organization_domains_dns_due_idx
  ON organization_domains(next_dns_check_at, status)
  WHERE status IN ('dns_pending','active','degraded') AND provider_domain_id IS NOT NULL;

-- System/backfilled domains predate the public onboarding state machine. If a
-- provider id is already known they remain authoritative and are not queued for
-- customer DNS onboarding by this migration.
UPDATE organization_domains
SET provider_marker = CASE
      WHEN provider_marker='' AND is_system THEN 'cs-mail:system-domain:' || id::text
      ELSE provider_marker
    END
WHERE provider_marker='';

CREATE UNIQUE INDEX IF NOT EXISTS organization_domains_provider_id_unique_idx
  ON organization_domains(provider_domain_id)
  WHERE provider_domain_id IS NOT NULL AND provider_domain_id <> '';

-- Organization-domain changes are visible to every active member of that
-- business. Realtime remains an invalidation channel; clients re-fetch the
-- authoritative organization state after receiving the event.
CREATE OR REPLACE FUNCTION cs_realtime_organization_domain_change()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
  row_json JSONB;
  org_id UUID;
  member_row RECORD;
  event_seq BIGINT;
BEGIN
  IF TG_OP = 'UPDATE' THEN
    IF OLD.status IS NOT DISTINCT FROM NEW.status AND
       OLD.provider_domain_id IS NOT DISTINCT FROM NEW.provider_domain_id AND
       OLD.dns_zone_file IS NOT DISTINCT FROM NEW.dns_zone_file AND
       OLD.dns_mx_ready IS NOT DISTINCT FROM NEW.dns_mx_ready AND
       OLD.dns_spf_ready IS NOT DISTINCT FROM NEW.dns_spf_ready AND
       OLD.dns_dkim_ready IS NOT DISTINCT FROM NEW.dns_dkim_ready AND
       OLD.dns_dmarc_ready IS NOT DISTINCT FROM NEW.dns_dmarc_ready AND
       OLD.dns_ready IS NOT DISTINCT FROM NEW.dns_ready AND
       OLD.last_error IS NOT DISTINCT FROM NEW.last_error THEN
      RETURN NEW;
    END IF;
  END IF;

  row_json := CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
  org_id := NULLIF(row_json ->> 'organization_id','')::UUID;
  IF org_id IS NULL THEN
    RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
  END IF;

  FOR member_row IN
    SELECT user_id FROM organization_memberships
    WHERE organization_id=org_id AND status='active'
  LOOP
    IF NOT EXISTS (
      SELECT 1 FROM realtime_events
      WHERE transaction_id=txid_current()
        AND user_id=member_row.user_id
        AND kind='resource-changed'
        AND payload ->> 'resource'='business_domains'
    ) THEN
      INSERT INTO realtime_events(user_id, kind, payload)
      VALUES (
        member_row.user_id,
        'resource-changed',
        jsonb_build_object(
          'resource','business_domains',
          'organization_id',org_id::text,
          'domain_id',COALESCE(row_json ->> 'id',''),
          'action',lower(TG_OP)
        )
      ) RETURNING seq INTO event_seq;
      PERFORM pg_notify('cs_mail_realtime', event_seq::text);
    END IF;
  END LOOP;
  RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END;
$$;

DROP TRIGGER IF EXISTS organization_domains_realtime_change ON organization_domains;
CREATE TRIGGER organization_domains_realtime_change
AFTER INSERT OR UPDATE OR DELETE ON organization_domains
FOR EACH ROW EXECUTE FUNCTION cs_realtime_organization_domain_change();
