#!/usr/bin/env python3
"""Validate CS Mail's root-only shell env file without printing secret values."""
from __future__ import annotations
import re
import sys
from pathlib import Path

KEY = re.compile(r"^[A-Z][A-Z0-9_]*$")
DANGEROUS = ("`", "$(", "${")
REQUIRED_SECRET_MIN = {
    "POSTGRES_PASSWORD": 32,
    "CS_MAIL_JWT_SECRET": 32,
    "CS_MAIL_DELIVERY_EVENT_SECRET": 32,
    "CS_MAIL_PROVISIONING_KEY": 32,
    "CS_MAIL_TOTP_KEY": 32,
}
REQUIRED_NONEMPTY = (
    "CS_MAIL_SHARED_PROVIDER_NETWORK",
    "CS_MAIL_MAIL_ADMIN_TOKEN",
    "CS_MAIL_MAIL_JMAP_USERNAME",
    "CS_MAIL_MAIL_JMAP_SECRET",
    "CS_MAIL_LETSENCRYPT_EMAIL",
    "CS_MAIL_SMTP_USERNAME",
    "CS_MAIL_SMTP_PASSWORD",
)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: validate-env.py /opt/cs-mail/.env.production", file=sys.stderr)
        return 2
    path = Path(sys.argv[1])
    seen: dict[str, int] = {}
    values: dict[str, str] = {}
    for n, raw in enumerate(path.read_text().splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export ") or "=" not in line:
            print(f"invalid env syntax on line {n}", file=sys.stderr); return 1
        key, value = line.split("=", 1)
        if not KEY.fullmatch(key):
            print(f"invalid environment key on line {n}: {key!r}", file=sys.stderr); return 1
        if key in seen:
            print(
                f"duplicate environment key on line {n}: {key} "
                f"(first defined on line {seen[key]}); keep exactly one definition",
                file=sys.stderr,
            )
            return 1
        if any(token in value for token in DANGEROUS):
            print(f"shell command/interpolation syntax is forbidden in value for {key}", file=sys.stderr); return 1
        if "\x00" in value or "\r" in value:
            print(f"invalid control character in value for {key}", file=sys.stderr); return 1
        seen[key] = n; values[key] = value

    for key, minimum in REQUIRED_SECRET_MIN.items():
        if len(values.get(key, "")) < minimum:
            print(f"{key} must be set and at least {minimum} characters", file=sys.stderr); return 1
    for key in REQUIRED_NONEMPTY:
        if not values.get(key, "").strip():
            print(f"{key} must be configured before production deployment", file=sys.stderr); return 1
    mailer_url = values.get("CS_MAILER_API_URL", "").strip()
    mailer_key = values.get("CS_MAILER_API_KEY", "").strip()
    if mailer_url != "https://mailer.crescentsphere.com/api/v1/emails":
        print("CS_MAILER_API_URL must use the public Mailer HTTPS endpoint", file=sys.stderr); return 1
    if not mailer_key.startswith("cs_live_"):
        print("CS_MAILER_API_KEY must be a production sending key", file=sys.stderr); return 1

    # Backward-compatible production profile: older root-owned env files may
    # predate CS_MAIL_ENVIRONMENT. This production-only deployment stack always
    # injects the runtime profile as production, so absence is safe; an explicit
    # non-production value is still rejected.
    environment = values.get("CS_MAIL_ENVIRONMENT", "production").strip().lower()
    if environment != "production":
        print("CS_MAIL_ENVIRONMENT must be production when set", file=sys.stderr); return 1
    instant_activation = values.get("CS_MAIL_BILLING_INSTANT_ACTIVATION", "false").strip().lower()
    if instant_activation not in {"true", "false"}:
        print("CS_MAIL_BILLING_INSTANT_ACTIVATION must be true or false", file=sys.stderr); return 1

    if values.get("CS_MAIL_WEB_HOST", "mail.crescentsphere.com") != "mail.crescentsphere.com":
        print("CS_MAIL_WEB_HOST must be mail.crescentsphere.com", file=sys.stderr); return 1
    if values.get("CS_MAIL_CLIENT_HOST", "smtp.crescentsphere.com") != "smtp.crescentsphere.com":
        print("CS_MAIL_CLIENT_HOST must be smtp.crescentsphere.com for this VPS", file=sys.stderr); return 1
    if values.get("CS_MAIL_EXPECTED_PTR", "smtp.crescentsphere.com") != "smtp.crescentsphere.com":
        print("CS_MAIL_EXPECTED_PTR must remain smtp.crescentsphere.com", file=sys.stderr); return 1
    if values.get("CS_MAIL_SMTP_HOST") != "smtp.crescentsphere.com" or values.get("CS_MAIL_SMTP_PORT") != "465":
        print("private SMTP must use smtp.crescentsphere.com:465 with verified TLS", file=sys.stderr); return 1
    if values.get("CS_MAIL_WEB_HOST") == values.get("CS_MAIL_CLIENT_HOST"):
        print("web and mail protocol hostnames must be different", file=sys.stderr); return 1
    if values.get("CS_MAIL_PUBLIC_ORIGIN", "").rstrip("/") != "https://mail.crescentsphere.com":
        print("CS_MAIL_PUBLIC_ORIGIN must be https://mail.crescentsphere.com", file=sys.stderr); return 1
    if values.get("CS_MAIL_LISTEN_ADDR", "0.0.0.0:8080") != "0.0.0.0:8080":
        print("CS_MAIL_LISTEN_ADDR must remain 0.0.0.0:8080 inside the API container", file=sys.stderr); return 1
    if values.get("CS_MAIL_ATTACHMENT_STORE_DIR", "/srv/attachments") != "/srv/attachments":
        print("CS_MAIL_ATTACHMENT_STORE_DIR must remain /srv/attachments to match the persistent volume", file=sys.stderr); return 1
    try:
        db_capacity = int(values.get("CS_MAIL_DB_CAPACITY_BYTES", "21474836480"))
    except ValueError:
        print("CS_MAIL_DB_CAPACITY_BYTES must be an integer byte count", file=sys.stderr); return 1
    if db_capacity < 1073741824:
        print("CS_MAIL_DB_CAPACITY_BYTES must be at least 1 GiB", file=sys.stderr); return 1

    object_backend = values.get("CS_MAIL_OBJECT_STORAGE_BACKEND", "local").strip().lower()
    if object_backend not in {"local", "r2"}:
        print("CS_MAIL_OBJECT_STORAGE_BACKEND must be local or r2", file=sys.stderr); return 1
    if object_backend == "r2":
        for key in ("CS_MAIL_R2_ACCOUNT_ID", "CS_MAIL_R2_BUCKET", "CS_MAIL_R2_ACCESS_KEY_ID", "CS_MAIL_R2_SECRET_ACCESS_KEY"):
            if not values.get(key, "").strip():
                print(f"{key} is required when CS_MAIL_OBJECT_STORAGE_BACKEND=r2", file=sys.stderr); return 1
        endpoint = values.get("CS_MAIL_R2_ENDPOINT", "").strip()
        if endpoint and not endpoint.startswith("https://"):
            print("CS_MAIL_R2_ENDPOINT must use HTTPS when set", file=sys.stderr); return 1
        try:
            transfers = int(values.get("CS_MAIL_R2_MAX_CONCURRENT_TRANSFERS", "4"))
        except ValueError:
            print("CS_MAIL_R2_MAX_CONCURRENT_TRANSFERS must be an integer", file=sys.stderr); return 1
        if not 1 <= transfers <= 16:
            print("CS_MAIL_R2_MAX_CONCURRENT_TRANSFERS must be between 1 and 16", file=sys.stderr); return 1
    expected_paths = {
        "CS_MAIL_WEB_TLS_CERT": "/etc/letsencrypt/live/mail.crescentsphere.com/fullchain.pem",
        "CS_MAIL_WEB_TLS_KEY": "/etc/letsencrypt/live/mail.crescentsphere.com/privkey.pem",
        "CS_MAIL_ACME_WEBROOT": "/var/www/letsencrypt",
        "CS_MAIL_NGINX_SITE": "/etc/nginx/sites-available/cs-mail.conf",
        "CS_MAIL_NGINX_LINK": "/etc/nginx/sites-enabled/cs-mail.conf",
    }
    for key, expected in expected_paths.items():
        if values.get(key, expected) != expected:
            print(f"{key} must remain {expected} for the managed production layout", file=sys.stderr); return 1
    email=values.get("CS_MAIL_LETSENCRYPT_EMAIL", "")
    if not re.fullmatch(r"[^@\s]+@[^@\s]+\.[^@\s]+", email):
        print("CS_MAIL_LETSENCRYPT_EMAIL must be a valid email address", file=sys.stderr); return 1
    alert_from = values.get("CS_MAIL_ALERT_EMAIL_FROM", "") or values.get("CS_MAIL_SMTP_USERNAME", "")
    alert_to = values.get("CS_MAIL_ALERT_EMAIL_TO", "") or email
    for label, address in (("CS_MAIL_ALERT_EMAIL_FROM", alert_from), ("CS_MAIL_ALERT_EMAIL_TO", alert_to)):
        if not re.fullmatch(r"[^\s@,]+@[^\s@,]+\.[^\s@,]+", address):
            print(f"{label} must resolve to one valid email address", file=sys.stderr); return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
