#!/usr/bin/env bash
set -euo pipefail
BASE_URL=${1:-http://127.0.0.1:8080}
WAIT_SECS=${WAIT_SECS:-60}

deadline=$((SECONDS + WAIT_SECS))
until curl -fsS "$BASE_URL/api/health/ready" >/dev/null 2>&1; do
  (( SECONDS < deadline )) || { echo "API readiness timeout" >&2; exit 1; }
  sleep 2
done

suffix="$(date +%s)-$$"
domain=${CS_MAIL_SMOKE_DOMAIN:-crescentsphere.com}
email="ci-smoke-$suffix@$domain"
password="Smoke-${suffix}-Aa9!"
jar=$(mktemp)
trap 'rm -f "$jar"' EXIT

register=$(curl -fsS -c "$jar" -H 'Content-Type: application/json' \
  -d "{\"email\":\"$email\",\"password\":\"$password\",\"name\":\"CI Smoke\"}" \
  "$BASE_URL/api/auth/register")
access=$(printf '%s' "$register" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("access", ""))')
[[ -n "$access" ]] || { echo "register did not return access token" >&2; exit 1; }

login=$(curl -fsS -c "$jar" -H 'Content-Type: application/json' \
  -d "{\"email\":\"$email\",\"password\":\"$password\"}" \
  "$BASE_URL/api/auth/login")
access=$(printf '%s' "$login" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("access", ""))')
[[ -n "$access" ]] || { echo "login did not return access token" >&2; exit 1; }

refresh=$(curl -fsS -b "$jar" -c "$jar" -X POST -H 'Content-Type: application/json' -d '{}' "$BASE_URL/api/auth/refresh")
rotated=$(printf '%s' "$refresh" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("access", ""))')
[[ -n "$rotated" ]] || { echo "refresh did not return rotated access token" >&2; exit 1; }

csrf_code=$(curl -sS -o /dev/null -w '%{http_code}' -b "$jar" -X POST \
  -H 'Content-Type: application/json' -H 'Origin: https://evil.example' -d '{}' \
  "$BASE_URL/api/auth/refresh")
[[ "$csrf_code" == 403 ]] || { echo "CSRF probe expected 403, got $csrf_code" >&2; exit 1; }

curl -fsS -H "Authorization: Bearer $rotated" "$BASE_URL/api/profile" >/dev/null
echo "CS Mail smoke test PASS"
