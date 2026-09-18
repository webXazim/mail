# Post-deploy smoke test (WS8.4).
#
# Failure of any step exits non-zero so CI / deploy scripts can auto-fail.
#
# Usage:
#   ./deploy/smoke-test.ps1 [-BaseUrl 'http://127.0.0.1:8080'] [-WaitS]
#
# Env overrides:
#   HARBOR_SMOKE_DOMAIN   user domain to register (default crescentsphere.com,
#                         must be a domain the API accepts; for a fresh deploy
#                         start the api with HARBOR_REQUIRE_VERIFICATION=0 so
#                         login works without SMTP verification).
param(
    [string]$BaseUrl = 'http://127.0.0.1:8080',
    [int]$WaitS = 60
)

$ErrorActionPreference = 'Stop'

$domain = if ($env:HARBOR_SMOKE_DOMAIN) { $env:HARBOR_SMOKE_DOMAIN } else { 'crescentsphere.com' }

function Fail([string]$msg) {
    Write-Host "SMOKE FAIL: $msg" -ForegroundColor Red
    exit 1
}

Write-Host "Smoke testing $BaseUrl (domain=$domain)..."

# 1. Health gate must come up within WaitS.
$deadline = (Get-Date).AddSeconds($WaitS)
$healthy = $false
while ((Get-Date) -lt $deadline) {
    try {
        $res = Invoke-WebRequest -UseBasicParsing -Method Get -Uri "$BaseUrl/api/health" -TimeoutSec 5
        if ($res.StatusCode -eq 200) { $healthy = $true; break }
    } catch { }
    Start-Sleep -Seconds 2
}
if (-not $healthy) { Fail "/api/health did not return 200 within ${WaitS}s" }
Write-Host "ok: /api/health" -ForegroundColor Green

# 2. Register a throwaway account.
$stamp = (Get-Date -Format 'yyyyMMddHHmmss') + (Get-Random -Minimum 1000 -Maximum 9999)
$email = "smoke-$stamp@$domain"
$password = 'Smoke!#Test2026'
$body = @{ name = 'Smoke Test'; email = $email; password = $password } | ConvertTo-Json
$registered = $false
try {
    $resp = Invoke-RestMethod -Method Post -Uri "$BaseUrl/api/auth/register" -ContentType 'application/json' -Body $body -TimeoutSec 15
    # Either the verify-gated shape ({ ok }) or the auto-login shape
    # ({ access, refresh, user }) counts as a successful registration.
    if ($resp.ok -eq $true -or $null -ne $resp.access) { $registered = $true }
} catch {
    $registered = $false
}
if (-not $registered) { Fail "register failed for $email" }
Write-Host "ok: register $email" -ForegroundColor Green

# 3. Login round trip.
$login = $null
try {
    $r = Invoke-WebRequest -UseBasicParsing -Method Post -Uri "$BaseUrl/api/auth/login" -ContentType 'application/json' -Body $body -TimeoutSec 15
    $login = $r.Content | ConvertFrom-Json
} catch { }
if ($null -eq $login -or [string]::IsNullOrEmpty($login.access) -or [string]::IsNullOrEmpty($login.refresh)) {
    Fail 'login did not return access + refresh tokens (is the api running with HARBOR_REQUIRE_VERIFICATION=0?)'
}
Write-Host "ok: login -> access + refresh" -ForegroundColor Green

# 4. Refresh rotation.
$rotated = $null
try {
    $r = Invoke-WebRequest -UseBasicParsing -Method Post -Uri "$BaseUrl/api/auth/refresh" -ContentType 'application/json' -Body (@{ token = $login.refresh } | ConvertTo-Json) -TimeoutSec 15
    $rotated = $r.Content | ConvertFrom-Json
} catch { }
if ($null -eq $rotated -or [string]::IsNullOrEmpty($rotated.access) -or [string]::IsNullOrEmpty($rotated.refresh)) {
    Fail 'refresh did not rotate tokens'
}
if ($rotated.refresh -eq $login.refresh) {
    Fail 'refresh returned the same token (rotation broken)'
}
Write-Host "ok: refresh rotated token" -ForegroundColor Green

# 5. The rotated access token must actually authenticate.
try {
    $S = @{ Authorization = "Bearer $($rotated.access)" }
    $prof = Invoke-RestMethod -Method Get -Uri "$BaseUrl/api/profile" -Headers $S -TimeoutSec 15
    if ($prof.email -ne $email) { Fail "profile email mismatch: $($prof.email)" }
} catch {
    Fail 'rotated access token rejected by /api/profile'
}
Write-Host "ok: rotated token authenticates /api/profile" -ForegroundColor Green

Write-Host "SMOKE PASS" -ForegroundColor Green
exit 0