#!/usr/bin/env bash
set -euo pipefail

[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }
ROOT=${CS_MAIL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
ENV_FILE=${1:-$STATE/.env.production}
CERT_ENV=${2:-$STATE/.env.certification}
COMPOSE="$ROOT/deploy/production/docker-compose.yml"
RUNTIME="$STATE/runtime/current.env"

fail(){ echo "LAUNCH FREEZE FAIL: $*" >&2; exit 1; }
[[ -f "$ENV_FILE" && -f "$CERT_ENV" && -f "$RUNTIME" ]] || fail "production env, certification env, and current deployment metadata are required"
# shellcheck disable=SC1090
set -a; source "$ENV_FILE"; source "$RUNTIME"; set +a
[[ ${CS_MAIL_ENVIRONMENT:-} == production ]] || fail "runtime profile is not production"
[[ ${CS_MAIL_BILLING_INSTANT_ACTIVATION:-true} == false ]] || fail "instant billing activation must be false"
[[ ${CS_MAIL_RELEASE_SHA256:-} =~ ^[0-9a-f]{64}$ ]] || fail "deployed release SHA-256 is missing"

cd "$ROOT"
bash "$ROOT/deploy/production/verify-release.sh"
bash "$ROOT/deploy/production/preflight.sh" "$ENV_FILE"

# Freeze is performed while public mutation switches are still closed. The
# certification uses pre-created disposable businesses/mailboxes, so opening
# signup/ordering/provisioning is not required to prove tenant/protocol safety.
controls_closed=$(docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
  psql -U "${POSTGRES_USER:-csmail}" -d "${POSTGRES_DB:-csmail}" -tAc \
  "SELECT NOT(public_signup_enabled OR business_creation_enabled OR plan_ordering_enabled OR domain_onboarding_enabled OR mailbox_provisioning_enabled OR outbound_sending_enabled) FROM platform_controls WHERE singleton=TRUE" | tr -d '[:space:]')
[[ "$controls_closed" == t ]] || fail "close all public platform controls before creating launch-freeze evidence"

systemctl is-active --quiet cs-mail-backup.timer || fail "cs-mail-backup.timer is not active"

# The live certifier creates a fresh local backup and runs the isolated restore
# drill before recording a certification row for this exact release.
bash "$ROOT/deploy/production/certify-launch.sh" "$ENV_FILE" "$CERT_ENV"

psql_scalar() {
  docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
    psql -U "${POSTGRES_USER:-csmail}" -d "${POSTGRES_DB:-csmail}" -tAc "$1" | tr -d '[:space:]'
}

release=${CS_MAIL_RELEASE_SHA256}
cert_ok=$(psql_scalar "SELECT EXISTS(SELECT 1 FROM launch_certification_runs WHERE environment='production' AND release_sha256='$release' AND status='passed' AND mandatory_failed=0 AND completed_at>=now()-interval '24 hours')")
[[ "$cert_ok" == t ]] || fail "this exact release has no fresh passing certification"
local_ok=$(psql_scalar "SELECT EXISTS(SELECT 1 FROM operational_evidence WHERE kind='local_backup' AND status='passed' AND release_sha256='$release' AND recorded_at>=now()-interval '26 hours')")
[[ "$local_ok" == t ]] || fail "fresh local backup evidence for this release is missing"
restore_ok=$(psql_scalar "SELECT EXISTS(SELECT 1 FROM operational_evidence WHERE kind='restore_drill' AND status='passed' AND release_sha256='$release' AND recorded_at>=now()-interval '7 days')")
[[ "$restore_ok" == t ]] || fail "fresh restore-drill evidence for this release is missing"
cs_offsite_ok=$(psql_scalar "SELECT EXISTS(SELECT 1 FROM operational_evidence WHERE kind='cs_mail_offsite_backup' AND status='passed' AND recorded_at>=now()-interval '26 hours')")
[[ "$cs_offsite_ok" == t ]] || fail "fresh CS Mail offsite backup proof is missing; record the external backup manifest first"
stalwart_offsite_ok=$(psql_scalar "SELECT EXISTS(SELECT 1 FROM operational_evidence WHERE kind='stalwart_offsite_backup' AND status='passed' AND recorded_at>=now()-interval '26 hours')")
[[ "$stalwart_offsite_ok" == t ]] || fail "fresh shared Stalwart offsite backup proof is missing; record the provider backup manifest first"

latest_cert=$(docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
  psql -U "${POSTGRES_USER:-csmail}" -d "${POSTGRES_DB:-csmail}" -AtF '|' -c \
  "SELECT id::text,report_sha256,completed_at::text FROM launch_certification_runs WHERE environment='production' AND release_sha256='$release' AND status='passed' ORDER BY completed_at DESC LIMIT 1")
stamp=$(date -u +%Y%m%d-%H%M%S)
freeze_dir=${CS_MAIL_CERT_REPORT_DIR:-$STATE/certifications}
install -d -m 0700 "$freeze_dir"
freeze="$freeze_dir/launch-freeze-$stamp.manifest"
{
  echo "product=CS Mail"
  echo "status=passed"
  echo "created=$(date -u +%FT%TZ)"
  echo "release_sha256=$release"
  echo "release_label=${CS_MAIL_RELEASE_LABEL:-unknown}"
  echo "certification=$latest_cert"
  echo "public_controls=closed"
  echo "billing_instant_activation=false"
  echo "local_backup=fresh"
  echo "restore_drill=fresh"
  echo "cs_mail_offsite_backup=fresh"
  echo "stalwart_offsite_backup=fresh"
  echo "next_action=Open public controls deliberately in Platform Admin; do not edit PostgreSQL directly."
} > "$freeze"
chmod 600 "$freeze"
freeze_sha=$(sha256sum "$freeze" | awk '{print $1}')
printf 'PUBLIC LAUNCH FREEZE PASS\nEvidence: %s\nSHA-256: %s\n' "$freeze" "$freeze_sha"
printf 'Next: open public controls from localhost Platform Admin in the documented order and watch alerts during the first live orders.\n'
