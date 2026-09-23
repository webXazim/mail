#!/bin/sh
set -eu
docker exec cs-platform-edge-mail_tls-1 nginx -t
docker exec cs-platform-edge-mail_tls-1 nginx -s reload
