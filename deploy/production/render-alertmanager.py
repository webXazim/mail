#!/usr/bin/env python3
"""Render a private Alertmanager email config from the validated production env."""

import json
import os
from pathlib import Path
import re
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[2]
STATE = Path("/opt/cs-mail/secrets")
EMAIL = re.compile(r"^[^\s@,]+@[^\s@,]+\.[^\s@,]+$")


def write_private(path: Path, data: str) -> None:
    fd, tmp = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(tmp, path)
        os.chmod(path, 0o600)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)


def main() -> int:
    if os.geteuid() != 0:
        raise ValueError("run as root")
    username = os.environ.get("CS_MAIL_SMTP_USERNAME", "").strip()
    password = os.environ.get("CS_MAIL_SMTP_PASSWORD", "")
    sender = os.environ.get("CS_MAIL_ALERT_EMAIL_FROM", "").strip() or username
    recipient = os.environ.get("CS_MAIL_ALERT_EMAIL_TO", "").strip() or os.environ.get("CS_MAIL_LETSENCRYPT_EMAIL", "").strip()
    if not username or "\n" in username or "\r" in username or not password or "\n" in password or "\r" in password:
        raise ValueError("a single-line CS Mail SMTP submission credential is required")
    if not EMAIL.fullmatch(sender):
        raise ValueError("set CS_MAIL_ALERT_EMAIL_FROM to a permitted sender email address")
    if not EMAIL.fullmatch(recipient):
        raise ValueError("set CS_MAIL_ALERT_EMAIL_TO to one valid recipient email address")
    if not STATE.is_dir() or STATE.is_symlink():
        raise ValueError(f"private secrets directory is missing or unsafe: {STATE}")
    template = (ROOT / "deploy/monitoring/alertmanager.production.yml").read_text(encoding="utf-8")
    config = (template.replace("__ALERT_FROM__", json.dumps(sender))
              .replace("__SMTP_USERNAME__", json.dumps(username))
              .replace("__ALERT_TO__", json.dumps(recipient)))
    write_private(STATE / "alert-smtp-password", password)
    write_private(STATE / "alertmanager.yml", config)
    print("Alertmanager email configuration rendered with private SMTP credential")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except ValueError as exc:
        print(f"ALERT CONFIG FAIL: {exc}", file=sys.stderr)
        sys.exit(1)
