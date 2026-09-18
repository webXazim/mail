# One-time Stalwart bootstrap for the compose `mail` service.
#
# First boot starts Stalwart in HTTP bootstrap mode (management/JMAP on :8080,
# responsecise endpoints). This script completes the setup wizard headlessly:
#   1. configures hostname / default domain / TLS / DKIM via x:Bootstrap/set
#   2. restarts the mail container so the new config takes effect
# Further runs are a no-op once bootstrap is complete.
#
# Usage (from repo root):
#   docker compose up -d mail
#   ./deploy/stalwart-bootstrap.ps1
#   docker compose up -d --build api
#
# Env overrides:
#   HARBOR_MAIL_ADMIN_PASSWORD  admin credential pin (default HarborDevMail!1)
#   HARBOR_MAIL_DEFAULT_DOMAIN  primary domain (default crescentsphere.com)
#   HARBOR_MAIL_HOSTNAME        server hostname (default mail.<domain>)
#   HARBOR_MAIL_PORT            host port for Stalwart HTTP (default 8081)
$ErrorActionPreference = 'Stop'

$domain = if ($env:HARBOR_MAIL_DEFAULT_DOMAIN) { $env:HARBOR_MAIL_DEFAULT_DOMAIN } else { 'crescentsphere.com' }
$hostname = if ($env:HARBOR_MAIL_HOSTNAME) { $env:HARBOR_MAIL_HOSTNAME } else { "mail.$domain" }
$port = if ($env:HARBOR_MAIL_PORT) { [int]$env:HARBOR_MAIL_PORT } else { 8081 }
$password = if ($env:HARBOR_MAIL_ADMIN_PASSWORD) { $env:HARBOR_MAIL_ADMIN_PASSWORD } else { 'HarborDevMail!1' }

$endpoint = "http://127.0.0.1:$port"
$auth = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("admin:$password"))

Write-Host "Using domain=$domain hostname=$hostname endpoint=$endpoint"

if ($null -eq (Get-Command curl.exe -ErrorAction SilentlyContinue)) {
    throw "curl.exe is required"
}

function Invoke-Jmap([string]$method, [string]$argsJson) {
    $body = '{"methodCalls":[["' + $method + '",' + $argsJson + ',"c1"]],"using":["urn:ietf:params:jmap:core","urn:stalwart:jmap"]}'
    & curl.exe -sS -X POST "$endpoint/jmap" -H "Authorization: Basic $auth" -H "Content-Type: application/json" --data-binary $body
}

Write-Host "Waiting for Stalwart bootstrap mode..."
$ok = $false
for ($i = 0; $i -lt 60; $i++) {
    try {
        $out = & curl.exe -sS -o NUL -w "%{http_code}" --max-time 2 "$endpoint/jmap" -H "Authorization: Basic $auth" -H "Content-Type: application/json" --data-binary '{"methodCalls":[["x:Bootstrap/get",{},"c1"]],"using":["urn:ietf:params:jmap:core","urn:stalwart:jmap"]}'
        if ($out -eq '200') { $ok = $true; break }
    } catch { }
    Start-Sleep -Seconds 2
}
if (-not $ok) {
    Write-Host "Bootstrap probe did not return 200. Is the mail container up and NOT yet bootstrapped?" -ForegroundColor Yellow
    Write-Host "If it was already bootstrapped this is fine (nothing to do). Exiting without changes."
    exit 0
}

Write-Host "Completing initial configuration..."
$resp = Invoke-Jmap 'x:Bootstrap/set' ('{"update":{"singleton":{"requestTlsCertificate":false,"generateDkimKeys":true,"serverHostname":"' + $hostname + '","defaultDomain":"' + $domain + '"}}}')
$resp | Write-Host

if ($resp -notmatch 'x:Bootstrap/set') {
    throw "Bootstrap/set did not return a JMAP response: $resp"
}

Write-Host "Restarting mail container to apply the new configuration..."
& docker compose restart mail
if ($LASTEXITCODE -ne 0) { throw "docker compose restart mail failed" }

Write-Host "Waiting for mail to restart in normal mode..."
$ready = $false
for ($i = 0; $i -lt 60; $i++) {
    try {
        $code = & curl.exe -sS -o NUL -w "%{http_code}" --max-time 2 "$endpoint/api/account" -H "Authorization: Basic $auth"
        if ($code -eq '200') { $ready = $true; break }
    } catch { }
    Start-Sleep -Seconds 2
}
if ($ready) {
    Write-Host "Done. Mail is up in normal mode; admin 'admin' is provisioned for" -ForegroundColor Green
    Write-Host "domain '$domain'. Bring up the API and sign up users to provision mailboxes."
} else {
    Write-Host "Mail restarted but not yet reachable on $endpoint — check 'docker logs mail'." -ForegroundColor Yellow
}