#!/usr/bin/env python3
"""CS Mail Upgrade 38 production launch certification.

Runs static and live production gates without logging credentials. A live PASS
requires real DNS/TLS, public API readiness, anti-relay, two-tenant IDOR probes,
app-password IMAP/SMTP round-trip, bounded read-only concurrency, and a fresh
backup + isolated restore drill. The final JSON report is hashed and recorded in
PostgreSQL's launch_certification_runs table.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import dataclasses
import datetime as dt
import email.message
import hashlib
import imaplib
import json
import os
from pathlib import Path
import re
import shutil
import smtplib
import socket
import ssl
import statistics
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from typing import Any, Callable

EXPECTED_CONTRACT = 32
EXPECTED_MIGRATION = "0042_launch_safety_defaults.sql"
DEFAULT_PUBLIC_ORIGIN = "https://mail.crescentsphere.com"


@dataclasses.dataclass
class Check:
    name: str
    mandatory: bool
    status: str
    detail: str
    duration_ms: int


class GateError(RuntimeError):
    pass


def parse_env_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    if not path.exists():
        return values
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in ("'", '"'):
            value = value[1:-1]
        values[key.strip()] = value
    return values


def run_cmd(cmd: list[str], *, cwd: Path | None = None, timeout: int = 120, input_text: str | None = None) -> str:
    proc = subprocess.run(
        cmd,
        cwd=cwd,
        input=input_text,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
    )
    if proc.returncode != 0:
        tail = "\n".join(proc.stdout.splitlines()[-16:])
        raise GateError(f"command failed ({proc.returncode}): {' '.join(cmd[:4])}\n{tail}")
    return proc.stdout


def json_request(url: str, *, method: str = "GET", token: str | None = None,
                 headers: dict[str, str] | None = None, body: Any = None,
                 timeout: float = 15.0) -> tuple[int, dict[str, Any], dict[str, str]]:
    request_headers = {"Accept": "application/json", "User-Agent": "cs-mail-launch-certifier/38"}
    if token:
        request_headers["Authorization"] = f"Bearer {token}"
    if headers:
        request_headers.update(headers)
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        request_headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, method=method, headers=request_headers, data=data)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as res:
            payload = res.read(1024 * 1024)
            parsed = json.loads(payload) if payload else {}
            return res.status, parsed, {k.lower(): v for k, v in res.headers.items()}
    except urllib.error.HTTPError as exc:
        payload = exc.read(1024 * 1024)
        try:
            parsed = json.loads(payload) if payload else {}
        except Exception:
            parsed = {"raw": payload.decode(errors="replace")[:1000]}
        return exc.code, parsed, {k.lower(): v for k, v in exc.headers.items()}


def require_env(env: dict[str, str], *keys: str) -> list[str]:
    missing = [key for key in keys if not env.get(key, "").strip()]
    if missing:
        raise GateError("missing required certification settings: " + ", ".join(missing))
    return [env[key].strip() for key in keys]


def dig(name: str, record: str) -> list[str]:
    if not shutil.which("dig"):
        raise GateError("dig is required for production DNS certification")
    out = run_cmd(["dig", "+short", record, name], timeout=20)
    return [line.strip().strip('"') for line in out.splitlines() if line.strip()]


def check_static(root: Path) -> str:
    required = [
        root / "backend/migrations" / EXPECTED_MIGRATION,
        root / "deploy/production/docker-compose.yml",
        root / "deploy/production/nginx-mail.crescentsphere.com.conf",
        root / "deploy/production/backup.sh",
        root / "deploy/production/restore-drill.sh",
        root / "deploy/production/deploy.sh",
        root / "deploy/production/deploy-from-git.sh",
        root / "deploy/production/bootstrap-vps.sh",
        root / "deploy/production/verify-release.sh",
        root / "deploy/production/CONFIGURATION.md",
                root / "deploy/production/validate-env.py",
        root / "deploy/production/setup-web-tls.sh",
        root / "deploy/production/nginx-mail.crescentsphere.com.bootstrap.conf",
        root / "deploy/production/CREDENTIALS.md",
        root / "frontend/Dockerfile.production",
    ]
    missing = [str(p.relative_to(root)) for p in required if not p.exists()]
    if missing:
        raise GateError("missing release files: " + ", ".join(missing))
    meta = (root / "backend/src/handlers/meta.rs").read_text()
    contract = (root / "backend/src/middleware/contract.rs").read_text()
    if f"contract_version: {EXPECTED_CONTRACT}" not in meta or f'HeaderValue::from_static("{EXPECTED_CONTRACT}")' not in contract:
        raise GateError("runtime API contract does not consistently report v32")
    compose = (root / "deploy/production/docker-compose.yml").read_text()
    if (root / "docker-compose.yml").exists():
        raise GateError("repository root must not contain an ambiguous compose file; local compose belongs under deploy/development")
    if not (root / "deploy/development/docker-compose.yml").exists():
        raise GateError("development compose must live under deploy/development")
    if "image: ${CS_MAIL_API_IMAGE:-cs-mail-api:production}" not in compose:
        raise GateError("production API must use a release-tagged image selected by deployment metadata")
    backend_dockerfile = (root / "backend/Dockerfile").read_text()
    frontend_dockerfile = (root / "frontend/Dockerfile.production").read_text()
    if "cargo build --release --locked" not in backend_dockerfile or "Cargo.lock" not in backend_dockerfile:
        raise GateError("backend production image must use the committed Cargo.lock")
    if "cargo clippy --locked --all-targets" not in backend_dockerfile or "cargo test --locked --all-targets" not in backend_dockerfile:
        raise GateError("backend production image must run blocking Rust quality/test gates before release build")
    if "npm ci" not in frontend_dockerfile or "npm run build" not in frontend_dockerfile:
        raise GateError("frontend production image must use the committed package-lock with npm ci")
    deploy_script = (root / "deploy/production/deploy.sh").read_text()
    for required_deploy_token in ("flock", "pre-deploy backup", "git archive", "www/current", "CS_MAIL_API_IMAGE", "nginx -t"):
        if required_deploy_token not in deploy_script:
            raise GateError(f"production deployment pipeline is missing: {required_deploy_token}")
    if re.search(r"(?m)^\s{2}mail:\s*$", compose):
        raise GateError("production compose must not define a Stalwart/mail service")
    for port in (25, 465, 587, 993):
        if re.search(rf'(?m)^\s*-\s*["\']?[^\n]*:{port}(?::|["\']?$)', compose):
            raise GateError(f"production compose must not bind host mail port {port}")
    nginx = (root / "deploy/production/nginx-mail.crescentsphere.com.conf").read_text()
    if "root /opt/cs-mail/www/current;" not in nginx:
        raise GateError("production Nginx must serve the atomically switched external frontend release")
    if not re.search(r"listen\s+443\s+ssl;", nginx) or "server_name mail.crescentsphere.com;" not in nginx:
        raise GateError("CS Mail web vhost must serve mail.crescentsphere.com directly on HTTPS :443")
    if not re.search(r"listen\s+80;", nginx) or "return 301 https://$host$request_uri;" not in nginx:
        raise GateError("CS Mail web vhost must own HTTP :80 for ACME and HTTPS redirect")
    if "ssl_certificate /etc/letsencrypt/live/mail.crescentsphere.com/fullchain.pem;" not in nginx:
        raise GateError("CS Mail web vhost must use the managed Let's Encrypt certificate")
    if "listen 127.0.0.1:18081" not in nginx:
        raise GateError("Platform Admin must remain bound to localhost :18081")
    if "real_ip_header CF-Connecting-IP" in nginx or "listen 127.0.0.1:18082" in nginx:
        raise GateError("legacy Cloudflare Tunnel origin directives must not remain")
    env_example = (root / "deploy/production/.env.production.example").read_text()
    if "CS_MAIL_WEB_PROXY_MODE=messenger" in env_example:
        edge = (root / "deploy/production/nginx-shared-edge.conf").read_text()
        public_inner = (root / "deploy/production/nginx-inner.conf").read_text()
        admin_inner = (root / "deploy/production/nginx-admin-inner.conf").read_text()
        if "cs-mail-web" not in compose or "cs-messenger_messenger" not in compose:
            raise GateError("Messenger shared edge network and CS Mail web alias are required")
        if "127.0.0.1:${CS_MAIL_ADMIN_HOST_PORT:-18081}:8081" not in compose:
            raise GateError("CS Mail admin web must bind only to VPS loopback")
        if "server_name mail.crescentsphere.com;" not in edge or "ssl_certificate /etc/letsencrypt/live/mail.crescentsphere.com/fullchain.pem;" not in edge:
            raise GateError("Messenger CS Mail edge must use its own public Let's Encrypt certificate")
        for blocked in ("/api/admin", "/mail/admin", "/api/metrics"):
            if blocked not in edge or blocked not in public_inner:
                raise GateError(f"public CS Mail web must block {blocked}")
        if 'proxy_set_header X-CS-Admin-Local "1";' not in admin_inner:
            raise GateError("localhost-only admin web must mark admin requests")
    if "CS_MAIL_CLIENT_HOST=smtp.crescentsphere.com" not in env_example or "CS_MAIL_EXPECTED_PTR=smtp.crescentsphere.com" not in env_example:
        raise GateError("production env contract must use the existing smtp.crescentsphere.com mail/PTR identity")
    if "CS_MAIL_REQUIRE_PUBLIC_HTTPS_HEALTH=true" not in env_example or "CS_MAIL_LETSENCRYPT_EMAIL=" not in env_example:
        raise GateError("production env contract must include direct HTTPS/TLS operational settings")
    if not re.search(r"location = /api/metrics\s*\{[^}]*return 404;", nginx):
        raise GateError("production nginx must block public /api/metrics")
    if "Content-Security-Policy" not in nginx or "frame-ancestors 'none'" not in nginx:
        raise GateError("production nginx must ship the outer SPA Content-Security-Policy")
    if not re.search(r"location = /api/mail-imports\s*\{[\s\S]*?client_max_body_size\s+2100m;", nginx):
        raise GateError("MBOX body-size exception must be scoped to /api/mail-imports")
    alertmanager = (root / "deploy/monitoring/alertmanager.yml").read_text()
    if "url_file: /run/secrets/cs_mail_alert_webhook_url" not in alertmanager:
        raise GateError("Alertmanager must use the mounted production webhook secret")
    if "127.0.0.1:9099" in alertmanager:
        raise GateError("Alertmanager still contains the legacy localhost receiver")
    audit = root / "backend/.cargo/audit.toml"
    if not audit.exists() or 'ignore = ["RUSTSEC-2023-0071"]' not in audit.read_text():
        raise GateError("cargo-audit policy must live at backend/.cargo/audit.toml")
    storage_migration = (root / "backend/migrations/0038_mailbox_storage_pool_allocations.sql").read_text()
    if "quota_override_bytes" not in storage_migration or "cs_mail_enforce_storage_pool_allocation" not in storage_migration:
        raise GateError("per-mailbox storage allocation migration is incomplete")
    payment_migration = (root / "backend/migrations/0039_payment_bound_plan_assignment_local_admin.sql").read_text()
    if "subscription_assignment_history" not in payment_migration or "orders_one_open_invoice_per_org_idx" not in payment_migration:
        raise GateError("payment-bound subscription assignment migration is incomplete")
    lifecycle_migration = (root / "backend/migrations/0040_platform_admin_operations.sql").read_text()
    if "renewal_grace_end" not in lifecycle_migration or "system_lifecycle" not in lifecycle_migration or "event_type" not in lifecycle_migration:
        raise GateError("platform-admin lifecycle/history migration is incomplete")
    if "Migrated existing subscription" not in lifecycle_migration or "assignment_source IN ('bootstrap'" not in lifecycle_migration:
        raise GateError("legacy/bootstrap subscriptions must receive operator-visible lifecycle history")
    migration = (root / "backend/migrations/0041_full_saas_control_plane.sql").read_text()
    if "CREATE TABLE IF NOT EXISTS platform_controls" not in migration or "mailbox_provisioning_enabled" not in migration:
        raise GateError("database-backed SaaS runtime controls migration is incomplete")
    if "status_reason" not in migration or "status_changed_by" not in migration or "status_changed_at" not in migration:
        raise GateError("auditable business lifecycle metadata is incomplete")
    mailbox_handler = (root / "backend/src/handlers/business_mailboxes.rs").read_text()
    if "pub async fn update_storage" not in mailbox_handler or "account_quotas" not in mailbox_handler or "enqueue_mailbox_quota_tx" not in mailbox_handler:
        raise GateError("mailbox storage allocation API/provider synchronization is incomplete")
    if "Current mailbox usage could not be verified" not in mailbox_handler or '"storage":if can_manage_storage' not in mailbox_handler:
        raise GateError("mailbox quota reductions must fail closed and business-wide pool data must be admin-only")
    entitlements = (root / "backend/src/services/entitlements.rs").read_text()
    if "pub fn scaled_storage_pool" not in entitlements or "A new mailbox requires the default allocation" not in entitlements:
        raise GateError("storage pool scaling/default mailbox allocation authority is incomplete")
    if "current_period_end" not in entitlements or "renewal_grace_end" not in entitlements:
        raise GateError("expired subscription periods must fail closed in entitlement reads")
    profile_handler = (root / "backend/src/handlers/profile.rs").read_text()
    if "mailbox_quota_bytes" not in profile_handler or "provider_total_bytes" not in profile_handler:
        raise GateError("signed-in mailbox storage indicator is not mailbox-authoritative")
    org_service = (root / "frontend/src/services/organizations.ts").read_text()
    business_page = (root / "frontend/src/pages/BusinessPage.tsx").read_text()
    if "updateMailboxStorage" not in org_service or "Mailboxes & storage" not in business_page or "business-mailbox-storage" not in business_page:
        raise GateError("business mailbox storage administration UI is incomplete")
    billing_handler = (root / "backend/src/handlers/billing.rs").read_text()
    if '"storage_allocated_bytes"' not in billing_handler:
        raise GateError("billing summary must expose allocated pooled storage")
    billing_service = (root / "backend/src/services/billing.rs").read_text()
    if "Payment must be submitted with a reference" not in billing_service or '"payment_approval", true' not in billing_service:
        raise GateError("reviewed payment must re-assert the exact ordered subscription")
    if "Invoice {invoice} is still open" not in billing_service:
        raise GateError("billing must prevent overlapping open invoices for one business")
    if "pub async fn reconcile_subscription_lifecycle" not in billing_service or "Renewal grace period expired" not in billing_service:
        raise GateError("subscription expiration/grace lifecycle worker is incomplete")
    auth_middleware = (root / "backend/src/middleware/auth.rs").read_text()
    if 'ADMIN_LOCAL_HEADER: &str = "x-cs-admin-local"' not in auth_middleware or "is_local_admin_request(&parts.headers)" not in auth_middleware:
        raise GateError("platform-admin API extractor is not localhost-gated")
    router = (root / "backend/src/router.rs").read_text()
    if "local_admin_gate" not in router:
        raise GateError("all /api/admin routes must be protected by the localhost route gate")
    if "/api/admin/subscriptions/:organization_id/history" not in router:
        raise GateError("platform admin must expose subscription history behind the localhost gate")
    admin_handler = (root / "backend/src/handlers/admin.rs").read_text()
    if "platform_role" not in admin_handler or "admin.user.platform_role" not in admin_handler:
        raise GateError("platform-role management is incomplete")
    if "business_memberships" not in admin_handler:
        raise GateError("platform user inventory must expose every business membership and inherited subscription")
    if "UserListQuery" not in admin_handler or "platform_role" not in admin_handler or '"total": total' not in admin_handler:
        raise GateError("platform user inventory must use server-side search/filter pagination")
    if "users(State(state), admin).await" in admin_handler or "Query(UserListQuery" not in admin_handler:
        raise GateError("admin user mutations must call the paged user inventory with its query contract")
    if 'Some("admin") => "platform_admin"' in admin_handler:
        raise GateError("legacy business/admin role must not implicitly grant platform-admin authority")
    if "The protected system organization subscription cannot be changed" not in billing_handler:
        raise GateError("backend must protect the internal system subscription from manual customer-plan edits")
    if "AdminSubscriptionsQuery" not in billing_handler or "organization_id" not in billing_handler or '"total": total' not in billing_handler:
        raise GateError("platform subscription inventory must use server-side search/filter pagination")
    billing_ui = (root / "frontend/src/pages/AdminBillingPage.tsx").read_text()
    if "subscriptionHistory" not in billing_ui or "Subscription history" not in billing_ui or "Expiration" not in billing_ui or "applySubscription" not in billing_ui:
        raise GateError("platform subscription lifecycle/history UI is incomplete")
    if "subscriptionsPage" not in billing_ui or "Search business, owner, billing email, plan or invoice" not in billing_ui or "subscriptionPageSize" not in billing_ui:
        raise GateError("platform subscription inventory UI must be server-filtered and paginated")
    admin_ui = (root / "frontend/src/pages/AdminPage.tsx").read_text()
    if "All business access" not in admin_ui or "Manage plan" not in admin_ui:
        raise GateError("platform user inventory must link every membership to authoritative business-plan management")
    if "platformRoleFilter" not in admin_ui or "userPageSize" not in admin_ui or ".usersPage" not in admin_ui:
        raise GateError("platform user inventory UI must be server-filtered and paginated")
    if 'aria-label="Search platform access"' not in admin_ui:
        raise GateError("platform access management must remain searchable and paginated at production scale")
    control_service = (root / "backend/src/services/platform_control.rs").read_text()
    for required_control in ("require_signup", "require_business_creation", "require_plan_ordering", "require_domain_onboarding", "require_mailbox_provisioning", "require_outbound_sending"):
        if required_control not in control_service:
            raise GateError(f"missing server-enforced platform control: {required_control}")
    control_handler = (root / "backend/src/handlers/platform_admin.rs").read_text()
    for authority in ("pub async fn businesses", "pub async fn business_members", "pub async fn domains", "pub async fn mailboxes", "pub async fn recovery", "pub async fn update_controls"):
        if authority not in control_handler:
            raise GateError(f"full SaaS control-plane backend is missing {authority}")
    if "Cancel this business subscription in Payments & plans before closing the business" not in control_handler:
        raise GateError("business closure must not silently override an uncancelled subscription")
    if "Protected system mailboxes are not managed through the customer control plane" not in control_handler or "The protected system organization cannot be modified here" not in control_handler:
        raise GateError("customer SaaS control-plane mutations must fail closed for protected system resources")
    if '"set_quota"' not in control_handler or "delete_customer_domain" not in control_handler:
        raise GateError("platform operator must be able to reconcile mailbox storage and safely release customer domains")
    platform_routes = (root / "backend/src/router.rs").read_text()
    for route in ("/api/admin/platform-controls", "/api/admin/businesses", "/api/admin/hosted-domains", "/api/admin/hosted-mailboxes", "/api/admin/recovery"):
        if route not in platform_routes:
            raise GateError(f"localhost control plane route missing: {route}")
    provisioning = (root / "backend/src/services/provisioning.rs").read_text()
    if "membership_status" not in provisioning or "mailbox_provisioning_enabled FROM platform_controls" not in provisioning:
        raise GateError("provider access/member suspension and queued mailbox-provisioning pause enforcement are incomplete")
    domain_onboarding = (root / "backend/src/services/domain_onboarding.rs").read_text()
    if "enqueue_mailbox_access_tx" not in domain_onboarding or 'next_status == "active"' not in domain_onboarding:
        raise GateError("domain recovery must re-enforce provider mailbox access when DNS becomes healthy")
    send_handler = (root / "backend/src/handlers/send.rs").read_text()
    schedule_handler = (root / "backend/src/handlers/schedule.rs").read_text()
    if "require_outbound_sending" not in send_handler or "outbound_sending_enabled" not in schedule_handler:
        raise GateError("outbound emergency pause must cover direct and scheduled customer sending")
    control_ui = (root / "frontend/src/pages/AdminControlPlanePage.tsx").read_text()
    if "Emergency service controls" not in control_ui or "Hosted domains" not in control_ui or "Hosted mailboxes" not in control_ui or "Durable job recovery" not in control_ui:
        raise GateError("localhost SaaS control-plane UI is incomplete")
    if "<Pager" not in control_ui or "set_quota" not in control_ui or "release this domain" not in control_ui:
        raise GateError("control-plane inventories must be paginated and expose safe operator storage/domain recovery")
    app_ui = (root / "frontend/src/App.tsx").read_text()
    if 'path="admin/control-plane"' not in app_ui or "RequireLocalAdminOrigin" not in app_ui:
        raise GateError("SaaS control plane must remain localhost-origin gated in the SPA")
    error_handler = (root / "backend/src/error.rs").read_text()
    if 'self.error != "platform_paused"' not in error_handler:
        raise GateError("operator-authored maintenance responses must be safe/public while other 5xx details stay redacted")
    if not re.search(r"location \^~ /api/admin/\s*\{\s*return 404;\s*\}", nginx):
        raise GateError("public Nginx must block /api/admin/*")
    if not re.search(r"location \^~ /mail/admin/\s*\{\s*return 404;\s*\}", nginx):
        raise GateError("public Nginx must block /mail/admin/*")
    if "listen 127.0.0.1:18081;" not in nginx or 'proxy_set_header X-CS-Admin-Local "1";' not in nginx:
        raise GateError("localhost-only admin reverse proxy is missing")
    if "CS_MAIL_BILLING_INSTANT_ACTIVATION:-false" not in compose:
        raise GateError("production billing must default to payment approval")
    safety = (root / "backend/migrations" / EXPECTED_MIGRATION).read_text()
    if "public_signup_enabled=FALSE" not in safety or "outbound_sending_enabled=FALSE" not in safety:
        raise GateError("production launch controls must default closed")
    return "contract v32, migration 0042, independent edge option, smtp.crescentsphere.com mail/PTR identity, private control plane, deterministic Docker builds, atomic frontend publishing, pre-migration backup, and closed public launch controls are coherent"


def check_env_file(env_file: Path, env: dict[str, str]) -> str:
    if not env_file.exists():
        raise GateError(f"missing {env_file}")
    mode = env_file.stat().st_mode & 0o777
    if mode & 0o077:
        raise GateError(f"{env_file} permissions are {mode:o}; require 600 or stricter")
    require_env(
        env,
        "POSTGRES_PASSWORD", "CS_MAIL_JWT_SECRET", "CS_MAIL_DELIVERY_EVENT_SECRET",
        "CS_MAIL_PROVISIONING_KEY", "CS_MAIL_TOTP_KEY", "CS_MAIL_MAIL_ADMIN_TOKEN",
        "CS_MAIL_MAIL_JMAP_USERNAME", "CS_MAIL_MAIL_JMAP_SECRET",
        "CS_MAIL_SHARED_PROVIDER_NETWORK", "CS_MAIL_RELEASE_SHA256",
        "CS_MAIL_DKIM_SELECTOR", "CS_MAIL_ALERT_WEBHOOK_FILE",
    )
    sha = env["CS_MAIL_RELEASE_SHA256"].lower()
    if not re.fullmatch(r"[0-9a-f]{64}", sha):
        raise GateError("CS_MAIL_RELEASE_SHA256 must be the 64-character deployed source-tree SHA-256")
    if env.get("CS_MAIL_PROVIDER_NAMESPACE", "cs-mail") != "cs-mail":
        raise GateError("CS_MAIL_PROVIDER_NAMESPACE must remain cs-mail")
    alert_file = Path(env["CS_MAIL_ALERT_WEBHOOK_FILE"])
    if not alert_file.is_file():
        raise GateError(f"missing Alertmanager webhook secret file: {alert_file}")
    alert_mode = alert_file.stat().st_mode & 0o777
    if alert_mode & 0o077:
        raise GateError(f"{alert_file} permissions are {alert_mode:o}; require 600 or stricter")
    alert_url = alert_file.read_text().strip()
    if not re.fullmatch(r"https://[^\s]+", alert_url):
        raise GateError("Alertmanager webhook secret must contain exactly one HTTPS URL")
    return f"production secrets/config present; mode={mode:o}; alert secret mode={alert_mode:o}; release={sha[:12]}…"


def check_subprocess_script(root: Path, env_file: Path, script: str, timeout: int) -> str:
    out = run_cmd([str(root / "deploy/production" / script), str(env_file)], cwd=root, timeout=timeout)
    return out.strip().splitlines()[-1] if out.strip() else f"{script} passed"


def check_public_api(env: dict[str, str]) -> str:
    origin = env.get("CS_MAIL_PUBLIC_ORIGIN", DEFAULT_PUBLIC_ORIGIN).rstrip("/")
    status, meta, headers = json_request(origin + "/api/meta")
    if status != 200 or meta.get("contract_version") != EXPECTED_CONTRACT:
        raise GateError(f"/api/meta expected contract {EXPECTED_CONTRACT}, got status={status} body={meta}")
    if headers.get("x-cs-contract-version") != str(EXPECTED_CONTRACT):
        raise GateError("x-cs-contract-version header does not match /api/meta")
    status, ready, _ = json_request(origin + "/api/health/ready")
    if status != 200:
        raise GateError(f"public readiness failed: status={status} body={ready}")
    return f"public API ready and contract v{EXPECTED_CONTRACT}"


def check_public_metrics_blocked(env: dict[str, str]) -> str:
    origin = env.get("CS_MAIL_PUBLIC_ORIGIN", DEFAULT_PUBLIC_ORIGIN).rstrip("/")
    status, _, _ = json_request(origin + "/api/metrics")
    if status != 404:
        raise GateError(f"public /api/metrics must return 404, got {status}")
    return "public /api/metrics is blocked at the reverse proxy"


def check_public_admin_blocked(env: dict[str, str]) -> str:
    origin = env.get("CS_MAIL_PUBLIC_ORIGIN", DEFAULT_PUBLIC_ORIGIN).rstrip("/")
    api_status, _, _ = json_request(origin + "/api/admin/overview")
    if api_status != 404:
        raise GateError(f"public /api/admin/* must return 404, got {api_status}")
    req = urllib.request.Request(origin + "/mail/admin", headers={"User-Agent": "cs-mail-launch-certifier/38"})
    try:
        urllib.request.urlopen(req, timeout=15)
        status = 200
    except urllib.error.HTTPError as exc:
        status = exc.code
    if status != 404:
        raise GateError(f"public /mail/admin must return 404, got {status}")
    return "public platform-admin API and SPA routes are blocked"


def check_https_headers(env: dict[str, str]) -> str:
    origin = env.get("CS_MAIL_PUBLIC_ORIGIN", DEFAULT_PUBLIC_ORIGIN).rstrip("/")
    req = urllib.request.Request(origin + "/", headers={"User-Agent": "cs-mail-launch-certifier/38"})
    with urllib.request.urlopen(req, timeout=15) as res:
        headers = {k.lower(): v for k, v in res.headers.items()}
    required = {
        "strict-transport-security": "max-age=",
        "x-content-type-options": "nosniff",
        "x-frame-options": "deny",
        "referrer-policy": "no-referrer",
        "content-security-policy": "frame-ancestors 'none'",
    }
    missing = [k for k, fragment in required.items() if fragment not in headers.get(k, "").lower()]
    if missing:
        raise GateError("missing/weak HTTPS security headers: " + ", ".join(missing))
    return "HSTS, nosniff, DENY framing, no-referrer and CSP headers verified"


def tls_certificate(host: str, port: int, *, smtp_starttls: bool = False) -> tuple[str, int]:
    context = ssl.create_default_context()
    if smtp_starttls:
        with smtplib.SMTP(host, port, timeout=15) as client:
            client.ehlo()
            client.starttls(context=context)
            client.ehlo()
            sock = client.sock
            if sock is None:
                raise GateError("SMTP STARTTLS did not establish a socket")
            cert = sock.getpeercert()
    else:
        with socket.create_connection((host, port), timeout=15) as raw:
            with context.wrap_socket(raw, server_hostname=host) as tls:
                cert = tls.getpeercert()
    expires = ssl.cert_time_to_seconds(cert["notAfter"])
    days = int((expires - time.time()) // 86400)
    if days < 14:
        raise GateError(f"certificate on {host}:{port} expires in {days} days (<14)")
    subject = dict(item[0] for item in cert.get("subject", []))
    return subject.get("commonName", host), days


def check_tls(env: dict[str, str]) -> str:
    mail_host = env.get("CS_MAIL_CLIENT_HOST", "smtp.crescentsphere.com")
    public_origin = env.get("CS_MAIL_PUBLIC_ORIGIN", DEFAULT_PUBLIC_ORIGIN)
    parsed = urllib.parse.urlparse(public_origin)
    if parsed.scheme != "https" or not parsed.hostname:
        raise GateError("CS_MAIL_PUBLIC_ORIGIN must be a valid HTTPS URL")
    web_port = parsed.port or 443
    _, web_days = tls_certificate(parsed.hostname, web_port)
    _, imap_days = tls_certificate(mail_host, int(env.get("CS_MAIL_CLIENT_IMAP_PORT", "993")))
    smtp_port = int(env.get("CS_MAIL_CLIENT_SMTP_PORT", "465"))
    _, smtp_days = tls_certificate(mail_host, smtp_port, smtp_starttls=smtp_port != 465)
    return (
        f"Direct web HTTPS and IMAPS/SMTP TLS certificates verify "
        f"(min {min(web_days, imap_days, smtp_days)} days remaining)"
    )


def check_dns(env: dict[str, str]) -> str:
    host = env.get("CS_MAIL_CLIENT_HOST", "smtp.crescentsphere.com").rstrip(".")
    domain = env.get("CS_MAIL_CERT_DOMAIN", env.get("CS_MAIL_MAIL_DEFAULT_DOMAIN", "crescentsphere.com")).rstrip(".")
    selector = env["CS_MAIL_DKIM_SELECTOR"].strip()
    addresses = dig(host, "A")
    if not addresses:
        raise GateError(f"{host} has no A record")
    mx = dig(domain, "MX")
    if not mx:
        raise GateError(f"{domain} has no MX record")
    mx_hosts = {record.split()[-1].rstrip(".").lower() for record in mx}
    if mx_hosts != {host.lower()}:
        raise GateError(f"{domain} MX must point only to {host}; found {', '.join(sorted(mx_hosts))}")
    spf = " ".join(dig(domain, "TXT"))
    if "v=spf1" not in spf.lower():
        raise GateError(f"{domain} has no SPF TXT policy")
    dmarc = " ".join(dig(f"_dmarc.{domain}", "TXT"))
    if "v=dmarc1" not in dmarc.lower():
        raise GateError(f"{domain} has no DMARC policy")
    dkim = " ".join(dig(f"{selector}._domainkey.{domain}", "TXT"))
    if "v=dkim1" not in dkim.lower() and "p=" not in dkim.lower():
        raise GateError(f"DKIM record missing for selector {selector} on {domain}")
    expected_ptr = env.get("CS_MAIL_EXPECTED_PTR", host).rstrip(".").lower()
    try:
        ptr = socket.gethostbyaddr(addresses[0])[0].rstrip(".").lower()
    except OSError as exc:
        raise GateError(f"PTR lookup failed for {addresses[0]}: {exc}") from exc
    if ptr != expected_ptr:
        raise GateError(f"PTR mismatch: {addresses[0]} -> {ptr}, expected {expected_ptr}")
    forward = {item[4][0] for item in socket.getaddrinfo(expected_ptr, 25, type=socket.SOCK_STREAM)}
    if addresses[0] not in forward:
        raise GateError(f"forward-confirmed PTR failed: {expected_ptr} does not resolve back to {addresses[0]}")
    return f"A/MX/SPF/DKIM/DMARC/PTR verified for {domain} ({addresses[0]} -> {ptr})"


def check_no_open_relay(env: dict[str, str]) -> str:
    host = env.get("CS_MAIL_CLIENT_HOST", "smtp.crescentsphere.com")
    port = int(env.get("CS_MAIL_CLIENT_SMTP_PORT", "465"))
    context = ssl.create_default_context()
    client_factory = smtplib.SMTP_SSL if port == 465 else smtplib.SMTP
    with client_factory(host, port, timeout=15, context=context) if port == 465 else client_factory(host, port, timeout=15) as client:
        client.ehlo()
        if port != 465:
            client.starttls(context=context)
            client.ehlo()
        mail_code, _ = client.mail("relay-probe@external.invalid")
        if 200 <= mail_code < 300:
            rcpt_code, response = client.rcpt(env.get("CS_MAIL_CERT_EXTERNAL_RCPT", "relay-probe@example.net"))
            if 200 <= rcpt_code < 300:
                raise GateError(f"unauthenticated external relay recipient accepted ({rcpt_code}); DATA was not sent")
            return f"unauthenticated relay rejected at RCPT ({rcpt_code})"
        return f"unauthenticated relay rejected at MAIL ({mail_code})"


def login(origin: str, email_addr: str, password: str) -> tuple[str, str]:
    status, body, _ = json_request(origin + "/api/auth/login", method="POST", body={"email": email_addr, "password": password})
    token = body.get("access") if isinstance(body, dict) else None
    user = body.get("user", {}) if isinstance(body, dict) else {}
    user_id = user.get("id") if isinstance(user, dict) else None
    if status != 200 or not isinstance(token, str) or not token or not isinstance(user_id, str) or not user_id:
        raise GateError(f"certification login failed for configured account ({status})")
    return token, user_id


def own_context(origin: str, token: str, user_id: str) -> tuple[str, str]:
    status, body, _ = json_request(origin + "/api/organizations", token=token)
    if status != 200:
        raise GateError(f"organizations lookup failed ({status})")
    orgs = body.get("organizations", [])
    org = next((o for o in orgs if o.get("status") == "active" and not o.get("is_system")), None) or next((o for o in orgs if o.get("status") == "active"), None)
    if not org:
        raise GateError("certification user has no active organization")
    org_id = str(org["id"])
    status, boxes, _ = json_request(origin + f"/api/organizations/{org_id}/mailboxes", token=token)
    if status != 200:
        raise GateError(f"mailbox lookup failed for certification organization ({status})")
    mailbox = next((m for m in boxes.get("mailboxes", []) if m.get("status") == "active" and str(m.get("user_id", "")) == user_id), None)
    if not mailbox:
        raise GateError("certification organization has no assigned active mailbox")
    return org_id, str(mailbox["id"])


def rejected(status: int) -> bool:
    return status in (403, 404)


def check_tenant_isolation(env: dict[str, str]) -> str:
    origin = env.get("CS_MAIL_PUBLIC_ORIGIN", DEFAULT_PUBLIC_ORIGIN).rstrip("/")
    a_email, a_pass, b_email, b_pass = require_env(
        env,
        "CS_MAIL_CERT_USER_A_EMAIL", "CS_MAIL_CERT_USER_A_PASSWORD",
        "CS_MAIL_CERT_USER_B_EMAIL", "CS_MAIL_CERT_USER_B_PASSWORD",
    )
    if a_email.lower() == b_email.lower():
        raise GateError("tenant certification accounts must be distinct")
    a_token, a_user_id = login(origin, a_email, a_pass)
    b_token, b_user_id = login(origin, b_email, b_pass)
    a_org, a_box = own_context(origin, a_token, a_user_id)
    b_org, b_box = own_context(origin, b_token, b_user_id)
    if a_org == b_org:
        raise GateError("tenant certification accounts must belong to different organizations")

    probes: list[tuple[str, int]] = []
    for token, foreign_org, foreign_box in ((a_token, b_org, b_box), (b_token, a_org, a_box)):
        probes.append(("foreign organization read", json_request(origin + f"/api/organizations/{foreign_org}", token=token)[0]))
        probes.append(("foreign mailbox list", json_request(origin + f"/api/organizations/{foreign_org}/mailboxes", token=token)[0]))
        probes.append(("foreign organization activation", json_request(origin + f"/api/organizations/{foreign_org}/activate", method="POST", token=token, body={})[0]))
        probes.append(("foreign mailbox activation", json_request(origin + f"/api/organizations/{foreign_org}/mailboxes/{foreign_box}/activate", method="POST", token=token, body={})[0]))
        probes.append(("foreign mailbox header", json_request(
            origin + "/api/settings", token=token,
            headers={"X-CS-Organization-ID": foreign_org, "X-CS-Mailbox-ID": foreign_box}
        )[0]))
    bad = [(name, code) for name, code in probes if not rejected(code)]
    if bad:
        raise GateError("cross-tenant probe unexpectedly succeeded: " + ", ".join(f"{n}={c}" for n, c in bad))
    return f"{len(probes)} symmetric cross-tenant organization/mailbox probes rejected"


def check_protocol_roundtrip(env: dict[str, str]) -> str:
    host = env.get("CS_MAIL_CLIENT_HOST", "smtp.crescentsphere.com")
    imap_port = int(env.get("CS_MAIL_CLIENT_IMAP_PORT", "993"))
    smtp_port = int(env.get("CS_MAIL_CLIENT_SMTP_PORT", "465"))
    a_user, a_password, b_user, b_password = require_env(
        env,
        "CS_MAIL_CERT_IMAP_A_USER", "CS_MAIL_CERT_IMAP_A_APP_PASSWORD",
        "CS_MAIL_CERT_IMAP_B_USER", "CS_MAIL_CERT_IMAP_B_APP_PASSWORD",
    )
    if a_user.lower() == b_user.lower():
        raise GateError("protocol certification mailboxes must be distinct")
    context = ssl.create_default_context()

    # Prove both app passwords can authenticate before sending anything.
    with imaplib.IMAP4_SSL(host, imap_port, ssl_context=context, timeout=20) as imap_a:
        typ, _ = imap_a.login(a_user, a_password)
        if typ != "OK":
            raise GateError("IMAP authentication failed for certification mailbox A")
    with imaplib.IMAP4_SSL(host, imap_port, ssl_context=context, timeout=20) as imap_b:
        typ, _ = imap_b.login(b_user, b_password)
        if typ != "OK":
            raise GateError("IMAP authentication failed for certification mailbox B")

    marker = f"cs-mail-launch-cert-{uuid.uuid4()}"
    msg = email.message.EmailMessage()
    msg["From"] = a_user
    msg["To"] = b_user
    msg["Subject"] = marker
    msg["Message-ID"] = f"<{marker}@{a_user.split('@',1)[-1]}>"
    msg.set_content(f"CS Mail Upgrade 38 production certification marker: {marker}")
    smtp_factory = smtplib.SMTP_SSL if smtp_port == 465 else smtplib.SMTP
    with smtp_factory(host, smtp_port, timeout=20, context=context) if smtp_port == 465 else smtp_factory(host, smtp_port, timeout=20) as smtp:
        smtp.ehlo()
        if smtp_port != 465:
            smtp.starttls(context=context)
            smtp.ehlo()
        smtp.login(a_user, a_password)
        refused = smtp.send_message(msg)
        if refused:
            raise GateError("SMTP submission refused one or more certification recipients")

    deadline = time.time() + 60
    found_id: bytes | None = None
    with imaplib.IMAP4_SSL(host, imap_port, ssl_context=context, timeout=20) as imap_b:
        imap_b.login(b_user, b_password)
        while time.time() < deadline:
            typ, _ = imap_b.select("INBOX")
            if typ != "OK":
                raise GateError("could not select certification mailbox B INBOX")
            typ, data = imap_b.search(None, "HEADER", "Message-ID", msg["Message-ID"])
            if typ == "OK" and data and data[0].strip():
                found_id = data[0].split()[-1]
                break
            time.sleep(2)
        if found_id is None:
            raise GateError("SMTP accepted certification message but IMAP B did not receive it within 60 seconds")
        # Remove only the unique certification message.
        imap_b.store(found_id, "+FLAGS", "\\Deleted")
        imap_b.expunge()
    return "app-password IMAP A/B auth + authenticated SMTP A→B + IMAP receipt passed"


def percentile(values: list[float], q: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    idx = min(len(ordered) - 1, max(0, int(round((len(ordered) - 1) * q))))
    return ordered[idx]


def check_load(env: dict[str, str]) -> str:
    origin = env.get("CS_MAIL_PUBLIC_ORIGIN", DEFAULT_PUBLIC_ORIGIN).rstrip("/")
    requests = int(env.get("CS_MAIL_CERT_LOAD_REQUESTS", "200"))
    concurrency = int(env.get("CS_MAIL_CERT_LOAD_CONCURRENCY", "20"))
    p95_limit = float(env.get("CS_MAIL_CERT_LOAD_P95_MS", "2000"))
    requests = max(20, min(requests, 2000))
    concurrency = max(2, min(concurrency, 100))

    def hit(i: int) -> tuple[int, float]:
        url = origin + ("/api/health/ready" if i % 2 == 0 else "/api/meta")
        start = time.perf_counter()
        status, _, _ = json_request(url, timeout=10)
        return status, (time.perf_counter() - start) * 1000

    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
        results = list(pool.map(hit, range(requests)))
    failures = [status for status, _ in results if status != 200]
    latencies = [lat for _, lat in results]
    p95 = percentile(latencies, 0.95)
    if failures:
        raise GateError(f"read-only load probe returned {len(failures)}/{requests} non-200 responses")
    if p95 > p95_limit:
        raise GateError(f"read-only load p95 {p95:.0f}ms exceeds configured {p95_limit:.0f}ms")
    return f"{requests} requests @ concurrency {concurrency}: p50={statistics.median(latencies):.0f}ms p95={p95:.0f}ms"


def record_ledger(root: Path, env_file: Path, env: dict[str, str], report: Path, summary: dict[str, int], passed: bool) -> None:
    compose = root / "deploy/production/docker-compose.yml"
    report_hash = hashlib.sha256(report.read_bytes()).hexdigest()
    release_hash = env["CS_MAIL_RELEASE_SHA256"].lower()
    label = env.get("CS_MAIL_RELEASE_LABEL", "cs-mail-upgrade-38-direct-nginx-secure-env")
    status = "passed" if passed else "failed"
    sql = r"""
INSERT INTO launch_certification_runs(
  release_label,release_sha256,environment,status,report_sha256,report_path,
  mandatory_passed,mandatory_failed,optional_skipped,completed_at
) VALUES (
  :'release_label', :'release_sha256', 'production', :'status', :'report_sha256', :'report_path',
  :'mandatory_passed'::integer, :'mandatory_failed'::integer, :'optional_skipped'::integer, now()
);
"""
    cmd = [
        "docker", "compose", "--env-file", str(env_file), "-f", str(compose),
        "exec", "-T", "db", "psql", "-U", env.get("POSTGRES_USER", "csmail"), "-d", env.get("POSTGRES_DB", "csmail"), "-v", "ON_ERROR_STOP=1",
        "-v", f"release_label={label}", "-v", f"release_sha256={release_hash}", "-v", f"status={status}",
        "-v", f"report_sha256={report_hash}", "-v", f"report_path={report}",
        "-v", f"mandatory_passed={summary['mandatory_passed']}",
        "-v", f"mandatory_failed={summary['mandatory_failed']}",
        "-v", f"optional_skipped={summary['optional_skipped']}",
    ]
    run_cmd(cmd, cwd=root, timeout=60, input_text=sql)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--env-file", type=Path, default=Path("/opt/cs-mail/.env.production"))
    parser.add_argument("--cert-env-file", type=Path, default=Path("/opt/cs-mail/.env.certification"))
    parser.add_argument("--report", type=Path)
    parser.add_argument("--static-only", action="store_true")
    args = parser.parse_args()
    root = args.root.resolve()
    env_file = args.env_file.resolve()
    cert_env_file = args.cert_env_file.resolve()
    env = parse_env_file(env_file)
    if not args.static_only:
        if not cert_env_file.exists():
            print(f"[FAIL] certification_secrets: missing {cert_env_file}")
            return 1
        cert_mode = cert_env_file.stat().st_mode & 0o777
        if cert_mode & 0o077:
            print(f"[FAIL] certification_secrets: {cert_env_file} permissions are {cert_mode:o}; require 600 or stricter")
            return 1
        env.update(parse_env_file(cert_env_file))
    env.update({k: v for k, v in os.environ.items() if v is not None})
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%d-%H%M%S")
    report = args.report or Path(env.get("CS_MAIL_CERT_REPORT_DIR", "/opt/cs-mail/certifications")) / f"launch-{stamp}.json"
    report = report.resolve()
    report.parent.mkdir(parents=True, exist_ok=True)

    checks: list[Check] = []

    def gate(name: str, fn: Callable[[], str], *, mandatory: bool = True) -> None:
        start = time.perf_counter()
        try:
            detail = fn()
            status = "passed"
        except Exception as exc:  # every gate becomes report evidence
            detail = str(exc)
            status = "failed" if mandatory else "skipped"
        checks.append(Check(name, mandatory, status, detail[:4000], int((time.perf_counter() - start) * 1000)))
        marker = "PASS" if status == "passed" else ("FAIL" if status == "failed" else "SKIP")
        print(f"[{marker}] {name}: {detail}")

    gate("release_static_integrity", lambda: check_static(root))
    if args.static_only:
        summary = {
            "mandatory_passed": sum(c.mandatory and c.status == "passed" for c in checks),
            "mandatory_failed": sum(c.mandatory and c.status == "failed" for c in checks),
            "optional_skipped": sum((not c.mandatory) and c.status == "skipped" for c in checks),
        }
    else:
        gate("production_environment", lambda: check_env_file(env_file, env))
        gate("shared_vps_preflight", lambda: check_subprocess_script(root, env_file, "preflight.sh", 120))
        gate("public_api_contract_and_readiness", lambda: check_public_api(env))
        gate("public_metrics_blocked", lambda: check_public_metrics_blocked(env))
        gate("public_admin_blocked", lambda: check_public_admin_blocked(env))
        gate("https_security_headers", lambda: check_https_headers(env))
        gate("public_protocol_tls", lambda: check_tls(env))
        gate("mail_dns_alignment", lambda: check_dns(env))
        gate("smtp_no_open_relay", lambda: check_no_open_relay(env))
        gate("cross_tenant_idor", lambda: check_tenant_isolation(env))
        gate("app_password_protocol_roundtrip", lambda: check_protocol_roundtrip(env))
        gate("bounded_public_concurrency", lambda: check_load(env))
        gate("fresh_backup", lambda: check_subprocess_script(root, env_file, "backup.sh", 900))
        gate("isolated_restore_and_domain_race_drill", lambda: check_subprocess_script(root, env_file, "restore-drill.sh", 1200))
        summary = {
            "mandatory_passed": sum(c.mandatory and c.status == "passed" for c in checks),
            "mandatory_failed": sum(c.mandatory and c.status == "failed" for c in checks),
            "optional_skipped": sum((not c.mandatory) and c.status == "skipped" for c in checks),
        }

    passed = summary["mandatory_failed"] == 0
    payload = {
        "schema": 1,
        "product": "CS Mail",
        "upgrade": 35,
        "api_contract": EXPECTED_CONTRACT,
        "migration_head": EXPECTED_MIGRATION,
        "started_at": (dt.datetime.now(dt.timezone.utc) - dt.timedelta(milliseconds=sum(c.duration_ms for c in checks))).isoformat(),
        "completed_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "status": "passed" if passed else "failed",
        "release_sha256": env.get("CS_MAIL_RELEASE_SHA256", "") if not args.static_only else "",
        "summary": summary,
        "checks": [dataclasses.asdict(c) for c in checks],
    }
    report.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    report_hash = hashlib.sha256(report.read_bytes()).hexdigest()
    print(f"report: {report}")
    print(f"report_sha256: {report_hash}")

    if not args.static_only:
        try:
            record_ledger(root, env_file, env, report, summary, passed)
            print("ledger: recorded in launch_certification_runs")
        except Exception as exc:
            print(f"[FAIL] certification_ledger: {exc}")
            passed = False
            checks.append(Check("certification_ledger", True, "failed", str(exc)[:4000], 0))
            summary["mandatory_failed"] += 1
            payload["status"] = "failed"
            payload["summary"] = summary
            payload["checks"] = [dataclasses.asdict(c) for c in checks]
            payload["completed_at"] = dt.datetime.now(dt.timezone.utc).isoformat()
            report.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
            report_hash = hashlib.sha256(report.read_bytes()).hexdigest()
            print(f"updated_failed_report_sha256: {report_hash}")

    print("LAUNCH CERTIFICATION: " + ("PASS" if passed else "FAIL"))
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
