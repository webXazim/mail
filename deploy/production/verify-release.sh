#!/usr/bin/env bash
set -euo pipefail
ROOT=${CS_MAIL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}
fail(){ echo "RELEASE CHECK FAIL: $*" >&2; exit 1; }
ok(){ echo "ok: $*"; }

[[ -f "$ROOT/backend/Cargo.lock" ]] || fail "backend/Cargo.lock is required"
[[ -f "$ROOT/frontend/package-lock.json" ]] || fail "frontend/package-lock.json is required"
[[ -f "$ROOT/frontend/Dockerfile.production" ]] || fail "frontend production Dockerfile is missing"
[[ -f "$ROOT/deploy/production/docker-compose.yml" ]] || fail "production compose is missing"
[[ -f "$ROOT/deploy/production/nginx-mail.crescentsphere.com.conf" ]] || fail "production Nginx config is missing"
for script in deploy.sh deploy-from-git.sh bootstrap-vps.sh init-env.sh preflight.sh backup.sh restore-drill.sh rollback.sh status.sh certify-launch.sh clean-worktree.sh smoke-test.sh; do
  [[ -x "$ROOT/deploy/production/$script" ]] || fail "production script is missing/not executable: $script"
done
ok "production deployment scripts are present and executable"

for generated in frontend/node_modules frontend/dist frontend/coverage backend/target backend/.cs-mail-target .cache; do
  [[ ! -e "$ROOT/$generated" ]] || fail "generated path must not be committed: $generated"
done
ok "generated dependency/build directories are absent"

if find "$ROOT" -type f \( -name '.env.production' -o -name '.env.certification' -o -name '*.pem' -o -name '*.key' -o -name 'id_rsa' -o -name 'id_ed25519' \) -print -quit | grep -q .; then
  fail "private environment/key material is present in the repository"
fi
ok "no obvious production secret/key files are present"

# Production must never accidentally use the local/dev compose topology.
if grep -Eq '^[[:space:]]{2}(mail|stalwart):[[:space:]]*$' "$ROOT/deploy/production/docker-compose.yml"; then
  fail "production compose must not define a mail/Stalwart service"
fi
for port in 25 465 587 993; do
  if grep -Eq "^[[:space:]]*-[[:space:]]*['\"]?[^#]*:${port}([:/\"']|$)" "$ROOT/deploy/production/docker-compose.yml"; then
    fail "production compose must not bind host mail port $port"
  fi
done
ok "production compose preserves shared-Stalwart ownership"

# Lock files make GitHub -> VPS builds reproducible.
grep -q 'cargo build --release --locked' "$ROOT/backend/Dockerfile" || fail "backend image must build Cargo.lock with --locked"
grep -q 'cargo clippy --locked --all-targets -- -D warnings' "$ROOT/backend/Dockerfile" || fail "backend deploy image must run blocking Clippy"
grep -q 'cargo test --locked --all-targets' "$ROOT/backend/Dockerfile" || fail "backend deploy image must run Rust tests"
grep -q 'npm ci' "$ROOT/frontend/Dockerfile.production" || fail "frontend image must use npm ci"
ok "locked backend/frontend builds are configured"

python3 "$ROOT/deploy/production/certify_launch.py" --static-only --root "$ROOT" --report /tmp/cs-mail-release-static.json >/dev/null
ok "static production launch certification passes"

echo "CS Mail release verification PASS"
