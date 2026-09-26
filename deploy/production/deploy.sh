#!/usr/bin/env bash
set -Eeuo pipefail

[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }

ROOT=${CS_MAIL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
ENV_FILE=${1:-$STATE/.env.production}
COMPOSE="$ROOT/deploy/production/docker-compose.yml"
NGINX_SOURCE="$ROOT/deploy/production/nginx-mail.crescentsphere.com.conf"
NGINX_SITE=/etc/nginx/sites-available/cs-mail.conf
NGINX_LINK=/etc/nginx/sites-enabled/cs-mail.conf
LOG_DIR=/var/log/cs-mail
RUNTIME_DIR="$STATE/runtime"

install -d -m 0700 "$RUNTIME_DIR" "$RUNTIME_DIR/deployments" /opt/backups/cs-mail "$LOG_DIR"
install -d -m 0755 "$STATE/www" "$STATE/www/releases"

exec 9>/run/lock/cs-mail-deploy.lock
flock -n 9 || { echo "another CS Mail deployment is already running" >&2; exit 1; }

stamp=$(date -u +%Y%m%d-%H%M%S)
LOG_FILE="$LOG_DIR/deploy-$stamp.log"
exec > >(tee -a "$LOG_FILE") 2>&1

on_error() {
  local rc=$?
  echo
  echo "DEPLOY FAILED (exit $rc). Log: $LOG_FILE" >&2
  if [[ -f "$RUNTIME_DIR/rollback-candidate.env" ]]; then
    echo "The previous successful release metadata is preserved at:" >&2
    echo "  $RUNTIME_DIR/rollback-candidate.env" >&2
    echo "If needed, review migration compatibility and run rollback.sh with its acknowledgement flag." >&2
  fi
  docker compose --env-file "$ENV_FILE" -f "$COMPOSE" ps 2>/dev/null || true
  docker compose --env-file "$ENV_FILE" -f "$COMPOSE" logs --tail=120 api 2>/dev/null || true
  docker compose --profile monitoring --env-file "$ENV_FILE" -f "$COMPOSE" logs --tail=80 alertmanager 2>/dev/null || true
  exit "$rc"
}
trap on_error ERR

[[ -f "$ENV_FILE" ]] || { echo "missing $ENV_FILE" >&2; exit 1; }
perm=$(stat -c '%a' "$ENV_FILE")
(( 10#$perm <= 600 )) || { echo "$ENV_FILE must be mode 600 or stricter (got $perm)" >&2; exit 1; }
set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a
KEEP_RELEASES=${CS_MAIL_KEEP_RELEASES:-5}
[[ "$KEEP_RELEASES" =~ ^[1-9][0-9]*$ ]] || { echo "CS_MAIL_KEEP_RELEASES must be a positive integer" >&2; exit 1; }
NGINX_SITE=${CS_MAIL_NGINX_SITE:-$NGINX_SITE}
NGINX_LINK=${CS_MAIL_NGINX_LINK:-$NGINX_LINK}
WEB_PROXY_MODE=${CS_MAIL_WEB_PROXY_MODE:-host}
case "$WEB_PROXY_MODE" in host|messenger|edge) ;; *) echo "CS_MAIL_WEB_PROXY_MODE must be host, messenger, or edge" >&2; exit 1 ;; esac

# API and platform-admin sockets remain loopback-only. Public web traffic is
# handled by this VPS's shared Nginx on the standard 80/443 virtual hosts.
[[ ${CS_MAIL_API_HOST_PORT:-18080} == 18080 ]] || { echo "CS_MAIL_API_HOST_PORT must remain 18080" >&2; exit 1; }
[[ ${CS_MAIL_ADMIN_HOST_PORT:-18081} == 18081 ]] || { echo "CS_MAIL_ADMIN_HOST_PORT must remain 18081" >&2; exit 1; }

bash "$ROOT/deploy/production/verify-release.sh"
bash "$ROOT/deploy/production/preflight.sh" "$ENV_FILE"
# Keep the scheduled backup service bound to this checkout, even when the repo
# lives outside the historical /opt/sites/cs-mail path.
bash "$ROOT/deploy/production/install-backup-timer.sh" "$ENV_FILE"

if [[ -d "$ROOT/.git" ]]; then
  cd "$ROOT"
  if [[ -n $(git status --porcelain --untracked-files=no) ]]; then
    echo "refusing to deploy modified tracked files from the production Git checkout" >&2
    git status --short >&2
    exit 1
  fi
  GIT_SHA=$(git rev-parse HEAD)
  SOURCE_SHA256=$(git archive --format=tar HEAD | sha256sum | awk '{print $1}')
else
  GIT_SHA=nogit
  SOURCE_SHA256=$(tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
    --exclude='.git' --exclude='frontend/node_modules' --exclude='frontend/dist' \
    --exclude='backend/target' --exclude='backend/.cs-mail-target' \
    -C "$ROOT" -cf - . | sha256sum | awk '{print $1}')
fi
SHORT_SHA=${GIT_SHA:0:12}
[[ "$GIT_SHA" == nogit ]] && SHORT_SHA=${SOURCE_SHA256:0:12}
RELEASE_TAG="git-${SHORT_SHA}-${SOURCE_SHA256:0:12}"
RELEASE_LABEL="cs-mail-${RELEASE_TAG}"
BUILD_DATE=$(date -u +%FT%TZ)
API_IMAGE="cs-mail-api:${RELEASE_TAG}"
FRONTEND_IMAGE="cs-mail-frontend-build:${RELEASE_TAG}"
WEB_RELEASE_DIR="$STATE/www/releases/$RELEASE_TAG"
WEB_STAGING_DIR="$STATE/www/releases/.${RELEASE_TAG}.tmp"

export CS_MAIL_BUILD_SHA="$GIT_SHA"
export CS_MAIL_BUILD_DATE="$BUILD_DATE"
export CS_MAIL_API_IMAGE="$API_IMAGE"
# The running API uses this immutable digest to bind launch certification and
# recovery evidence to the exact deployed source tree.
export CS_MAIL_RELEASE_SHA256="$SOURCE_SHA256"

printf '\n=== CS Mail production deployment ===\n'
printf 'release: %s\n' "$RELEASE_LABEL"
printf 'git: %s\n' "$GIT_SHA"
printf 'source sha256: %s\n' "$SOURCE_SHA256"
printf 'api image: %s\n\n' "$API_IMAGE"

# Preserve the last successful state for an explicit rollback if the new API
# starts migrations and later fails readiness.
if [[ -f "$RUNTIME_DIR/current.env" ]]; then
  cp -f "$RUNTIME_DIR/current.env" "$RUNTIME_DIR/rollback-candidate.env"
else
  rm -f "$RUNTIME_DIR/rollback-candidate.env"
fi

# Build everything before touching the running application.
echo "[1/8] Building and validating frontend in Docker..."
docker build --pull \
  -f "$ROOT/frontend/Dockerfile.production" \
  --build-arg VITE_DEMO_MODE=false \
  --build-arg BUILD_SHA="$GIT_SHA" \
  -t "$FRONTEND_IMAGE" \
  "$ROOT/frontend"

rm -rf "$WEB_STAGING_DIR"
install -d -m 0755 "$WEB_STAGING_DIR"
cid=$(docker create "$FRONTEND_IMAGE")
if ! docker cp "$cid:/app/dist/." "$WEB_STAGING_DIR/"; then
  docker rm -f "$cid" >/dev/null 2>&1 || true
  exit 1
fi
docker rm "$cid" >/dev/null
[[ -s "$WEB_STAGING_DIR/index.html" ]] || { echo "frontend build did not produce index.html" >&2; exit 1; }
rm -rf "$WEB_RELEASE_DIR"
mv "$WEB_STAGING_DIR" "$WEB_RELEASE_DIR"

echo "[2/8] Building release-tagged Rust API image..."
cd "$ROOT"
docker compose --env-file "$ENV_FILE" -f "$COMPOSE" build --pull api

# Back up the live state immediately before any container that can run SQLx
# migrations is replaced. First deploy has nothing to back up.
echo "[3/8] Creating pre-deploy backup when a live stack exists..."
if docker volume inspect cs-mail-prod_pgdata >/dev/null 2>&1; then
  # Starting only PostgreSQL is safe: migrations live in the API startup path.
  docker compose --env-file "$ENV_FILE" -f "$COMPOSE" up -d db
  for _ in $(seq 1 30); do
    db_health=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' \
      "$(docker compose --env-file "$ENV_FILE" -f "$COMPOSE" ps -q db)" 2>/dev/null || true)
    [[ "$db_health" == healthy || "$db_health" == running ]] && break
    sleep 2
  done
  CS_MAIL_BACKUP_REASON=predeploy bash "$ROOT/deploy/production/backup.sh" "$ENV_FILE"
else
  echo "No existing production PostgreSQL volume; pre-deploy backup skipped for first deployment."
fi

echo "[4/8] Starting database and new API (SQLx migrations run on API startup)..."
docker compose --env-file "$ENV_FILE" -f "$COMPOSE" up -d db
docker compose --env-file "$ENV_FILE" -f "$COMPOSE" up -d --no-build api

ready=0
for _ in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:${CS_MAIL_API_HOST_PORT:-18080}/api/health/ready" >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 2
done
[[ $ready -eq 1 ]] || { echo "new API failed readiness" >&2; exit 1; }

# Verify the forward migration and the provider-job operation contract that
# previously caused live 500s on `set_access` enqueue. Do this immediately
# after API startup, before publishing the frontend or declaring success.
echo "Verifying database migration head and provisioning operation contract..."
db_service=(docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db)
db_user=${POSTGRES_USER:-csmail}
db_name=${POSTGRES_DB:-csmail}
migration_head=$("${db_service[@]}" psql -U "$db_user" -d "$db_name" -Atqc   "SELECT COALESCE(max(version),0) FROM _sqlx_migrations WHERE success=TRUE")
[[ "$migration_head" =~ ^[0-9]+$ && "$migration_head" -ge 49 ]] || {
  echo "database migration head is $migration_head; expected at least 49" >&2
  exit 1
}
operation_constraint=$("${db_service[@]}" psql -U "$db_user" -d "$db_name" -Atqc   "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid='provisioning_jobs'::regclass AND conname='provisioning_jobs_operation_check'")
for operation in ensure_mailbox set_quota set_credentials set_access delete_mailbox; do
  grep -q "'$operation'" <<<"$operation_constraint" || {
    echo "provisioning_jobs_operation_check is missing operation: $operation" >&2
    exit 1
  }
done
echo "Database migration/operation contract PASS (head=$migration_head)"

echo "[5/8] Starting monitoring stack..."
python3 "$ROOT/deploy/production/render-alertmanager.py"
docker compose --profile monitoring --env-file "$ENV_FILE" -f "$COMPOSE" up -d --force-recreate alertmanager prometheus
alertmanager_ready=0
for _ in $(seq 1 30); do
  if curl -fsS http://127.0.0.1:19093/-/ready >/dev/null 2>&1; then
    alertmanager_ready=1
    break
  fi
  sleep 2
done
[[ $alertmanager_ready -eq 1 ]] || { echo "Alertmanager failed readiness" >&2; exit 1; }

echo "[6/8] Validating Nginx and atomically publishing frontend..."
if [[ -e "$STATE/www/current" && ! -L "$STATE/www/current" ]]; then
  echo "$STATE/www/current exists but is not a symlink; refusing unsafe frontend cutover" >&2
  exit 1
fi
if [[ "$WEB_PROXY_MODE" == messenger || "$WEB_PROXY_MODE" == edge ]]; then
  ln -sfn "$WEB_RELEASE_DIR" "$STATE/www/current"
  docker compose --profile shared_proxy --env-file "$ENV_FILE" -f "$COMPOSE" up -d --no-build web web_admin
  docker exec "$(docker compose --profile shared_proxy --env-file "$ENV_FILE" -f "$COMPOSE" ps -q web)" nginx -t
  docker exec "$(docker compose --profile shared_proxy --env-file "$ENV_FILE" -f "$COMPOSE" ps -q web_admin)" nginx -t
else
  nginx_backup=""
  if [[ -f "$NGINX_SITE" ]]; then
    nginx_backup=$(mktemp)
    cp -a "$NGINX_SITE" "$nginx_backup"
  fi
  install -m 0644 "$NGINX_SOURCE" "$NGINX_SITE"
  ln -sfn "$NGINX_SITE" "$NGINX_LINK"
  if ! nginx -t; then
    if [[ -n "$nginx_backup" ]]; then
      cp -a "$nginx_backup" "$NGINX_SITE"
    else
      rm -f "$NGINX_SITE" "$NGINX_LINK"
    fi
    [[ -z "$nginx_backup" ]] || rm -f "$nginx_backup"
    nginx -t >/dev/null 2>&1 || true
    echo "candidate Nginx configuration rejected; previous file restored" >&2
    exit 1
  fi
  [[ -z "$nginx_backup" ]] || rm -f "$nginx_backup"
  ln -sfn "$WEB_RELEASE_DIR" "$STATE/www/current"
  systemctl reload nginx
fi

echo "[7/8] Running loopback/local-TLS/public post-deploy health gates..."
curl -fsS "http://127.0.0.1:${CS_MAIL_API_HOST_PORT:-18080}/api/health/ready" >/dev/null
web_host=${CS_MAIL_WEB_HOST:-mail.crescentsphere.com}
public_origin=${CS_MAIL_PUBLIC_ORIGIN:-https://mail.crescentsphere.com}
curl -fsS --resolve "$web_host:443:127.0.0.1" "$public_origin/api/health/ready" >/dev/null
[[ $(curl -sS --resolve "$web_host:443:127.0.0.1" -o /dev/null -w '%{http_code}' "$public_origin/api/admin/overview") == 404 ]]
[[ $(curl -sS --resolve "$web_host:443:127.0.0.1" -o /dev/null -w '%{http_code}' "$public_origin/mail/admin") == 404 ]]
[[ $(curl -sS --resolve "$web_host:443:127.0.0.1" -o /dev/null -w '%{http_code}' "$public_origin/api/metrics") == 404 ]]
curl -fsS -H 'Host: localhost' "http://127.0.0.1:${CS_MAIL_ADMIN_HOST_PORT:-18081}/login" >/dev/null
if [[ ${CS_MAIL_REQUIRE_PUBLIC_HTTPS_HEALTH:-true} == true ]]; then
  curl -fsS "$public_origin/api/health/ready" >/dev/null
  [[ $(curl -sS -o /dev/null -w '%{http_code}' "$public_origin/api/admin/overview") == 404 ]]
  [[ $(curl -sS -o /dev/null -w '%{http_code}' "$public_origin/mail/admin") == 404 ]]
  [[ $(curl -sS -o /dev/null -w '%{http_code}' "$public_origin/api/metrics") == 404 ]]
else
  echo "Public HTTPS health gate skipped by CS_MAIL_REQUIRE_PUBLIC_HTTPS_HEALTH=false"
fi

# Persist deployment identity only after every health/security gate passes.
new_meta="$RUNTIME_DIR/current.env.new"
{
  printf 'CS_MAIL_RELEASE_LABEL=%q\n' "$RELEASE_LABEL"
  printf 'CS_MAIL_RELEASE_SHA256=%q\n' "$SOURCE_SHA256"
  printf 'CS_MAIL_DEPLOYED_GIT_SHA=%q\n' "$GIT_SHA"
  printf 'CS_MAIL_API_IMAGE=%q\n' "$API_IMAGE"
  printf 'CS_MAIL_FRONTEND_IMAGE=%q\n' "$FRONTEND_IMAGE"
  printf 'CS_MAIL_WEB_RELEASE_DIR=%q\n' "$WEB_RELEASE_DIR"
  printf 'CS_MAIL_DEPLOYED_AT=%q\n' "$BUILD_DATE"
} > "$new_meta"
chmod 600 "$new_meta"
if [[ -f "$RUNTIME_DIR/current.env" ]]; then
  cp -f "$RUNTIME_DIR/current.env" "$RUNTIME_DIR/previous.env"
fi
mv "$new_meta" "$RUNTIME_DIR/current.env"
cp -f "$RUNTIME_DIR/current.env" "$RUNTIME_DIR/deployments/${RELEASE_TAG}.env"
rm -f "$RUNTIME_DIR/rollback-candidate.env"

echo "[8/8] Cleaning old local release artifacts..."
# Keep the newest N frontend trees, but never delete current/previous targets.
mapfile -t web_releases < <(find "$STATE/www/releases" -mindepth 1 -maxdepth 1 -type d ! -name '.*' -printf '%T@ %p\n' | sort -rn | awk '{print $2}')
for ((i=KEEP_RELEASES; i<${#web_releases[@]}; i++)); do
  candidate=${web_releases[$i]}
  [[ $(readlink -f "$STATE/www/current") == $(readlink -f "$candidate") ]] && continue
  if [[ -f "$RUNTIME_DIR/previous.env" ]]; then
    prev_web=$(bash -c 'source "$1"; printf "%s" "${CS_MAIL_WEB_RELEASE_DIR:-}"' _ "$RUNTIME_DIR/previous.env")
    [[ -n "$prev_web" && $(readlink -f "$prev_web" 2>/dev/null || true) == $(readlink -f "$candidate") ]] && continue
  fi
  rm -rf -- "$candidate"
done

current_api="$API_IMAGE"
previous_api=""
previous_frontend=""
if [[ -f "$RUNTIME_DIR/previous.env" ]]; then
  previous_api=$(bash -c 'source "$1"; printf "%s" "${CS_MAIL_API_IMAGE:-}"' _ "$RUNTIME_DIR/previous.env")
  previous_frontend=$(bash -c 'source "$1"; printf "%s" "${CS_MAIL_FRONTEND_IMAGE:-}"' _ "$RUNTIME_DIR/previous.env")
fi
for repo in cs-mail-api cs-mail-frontend-build; do
  mapfile -t images < <(docker image ls "$repo" --format '{{.Repository}}:{{.Tag}}' | grep ':git-' || true)
  for ((i=KEEP_RELEASES; i<${#images[@]}; i++)); do
    image=${images[$i]}
    [[ "$image" == "$current_api" || "$image" == "$FRONTEND_IMAGE" || "$image" == "$previous_api" || "$image" == "$previous_frontend" ]] && continue
    docker image rm "$image" >/dev/null 2>&1 || true
  done
done
docker image prune -f --filter 'until=168h' >/dev/null || true
docker builder prune -f --filter 'until=168h' >/dev/null || true
bash "$ROOT/deploy/production/clean-worktree.sh" >/dev/null

trap - ERR
printf '\nCS Mail production deploy PASS\n'
printf 'Release: %s\n' "$RELEASE_LABEL"
printf 'Log: %s\n' "$LOG_FILE"
printf 'Status: %s/deploy/production/status.sh %s\n' "$ROOT" "$ENV_FILE"
