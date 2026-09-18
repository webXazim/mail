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

# 3. Login round trip via the HttpOnly session cookie.
$login = $null
$sess = $null
$setCookie = $null
try {
    $r = Invoke-WebRequest -UseBasicParsing -Method Post -Uri "$BaseUrl/api/auth/login" -ContentType 'application/json' -Body $body -TimeoutSec 15 -SessionVariable sess
    $login = $r.Content | ConvertFrom-Json
    $setCookie = $r.Headers['Set-Cookie']
} catch { }
if ($null -eq $login -or [string]::IsNullOrEmpty($login.access)) {
    Fail 'login did not return an access token (is the api running with HARBOR_REQUIRE_VERIFICATION=0?)'
}
if (-not ($setCookie -match 'harbor_session=')) { Fail 'login did not set a harbor_session cookie' }
if ($setCookie -notmatch 'HttpOnly') { Fail 'session cookie is not HttpOnly' }
if ($setCookie -notmatch 'SameSite=Lax') { Fail 'session cookie is not SameSite=Lax' }
Write-Host "ok: login -> access + HttpOnly session cookie" -ForegroundColor Green

# 4. Refresh rotates the cookie (no token in the request body).
$rotated = $null
$rotatedCookie = $null
try {
    $r = Invoke-WebRequest -UseBasicParsing -SkipHttpErrorCheck -Method Post -Uri "$BaseUrl/api/auth/refresh" -ContentType 'application/json' -Body '{}' -WebSession $sess -TimeoutSec 15
    $rotated = $r.Content | ConvertFrom-Json
    $rotatedCookie = $r.Headers['Set-Cookie']
} catch { }
if ($null -eq $rotated -or [string]::IsNullOrEmpty($rotated.access)) {
    Fail 'cookie-based refresh did not rotate tokens'
}
$m1 = [regex]::Match($setCookie, 'harbor_session=([^;]+)').Groups[1].Value
$m2 = [regex]::Match($rotatedCookie, 'harbor_session=([^;]+)').Groups[1].Value
if ($m1 -eq $m2 -or [string]::IsNullOrEmpty($m2)) {
    Fail 'refresh returned the same session cookie (rotation broken)'
}
Write-Host "ok: refresh rotated the session cookie" -ForegroundColor Green

# 5. CSRF probe: a cross-site POST carrying the cookie + forged Origin must be refused.
$csrfStatus = $null
try {
    $r = Invoke-WebRequest -UseBasicParsing -SkipHttpErrorCheck -Method Post -Uri "$BaseUrl/api/auth/refresh" -ContentType 'application/json' -Body '{}' -WebSession $sess -Headers @{ Origin = 'https://evil.example' } -TimeoutSec 15
    $csrfStatus = [int]$r.StatusCode
} catch { }
if ($csrfStatus -ne 403) { Fail "CSRF probe: expected 403, got $csrfStatus" }
Write-Host "ok: cross-site refresh blocked (CSRF probe 403)" -ForegroundColor Green

# 6. The rotated access token must actually authenticate.
try {
    $S = @{ Authorization = "Bearer $($rotated.access)" }
    $prof = Invoke-RestMethod -Method Get -Uri "$BaseUrl/api/profile" -Headers $S -TimeoutSec 15
    if ($prof.email -ne $email) { Fail "profile email mismatch: $($prof.email)" }
} catch {
    Fail 'rotated access token rejected by /api/profile'
}
Write-Host "ok: rotated token authenticates /api/profile" -ForegroundColor Green

# 7. Admin gate: if admin credentials are supplied, download the audit trail as
# CSV and verify RFC 4180 structure (a hard launch-gate row at WS5.5/8.3). The
# hard gate itself is also enforced in CI by `cargo test` (api_flows.rs), so this
# probe is belt-and-suspenders and skips when no HARBOR_SMOKE_ADMIN_* are set.
$adminEmail = $env:HARBOR_SMOKE_ADMIN_EMAIL
$adminPass = $env:HARBOR_SMOKE_ADMIN_PASSWORD
if ([string]::IsNullOrEmpty($adminEmail) -or [string]::IsNullOrEmpty($adminPass)) {
    Write-Host "skip: no HARBOR_SMOKE_ADMIN_EMAIL/PASSWORD set; audit CSV export probed by api_flows.rs in CI" -ForegroundColor Yellow
} else {
    $adminLogin = $null
    try {
        $B = @{ email = $adminEmail; password = $adminPass } | ConvertTo-Json
        $r = Invoke-WebRequest -UseBasicParsing -Method Post -Uri "$BaseUrl/api/auth/login" -ContentType 'application/json' -Body $B -TimeoutSec 15
        $adminLogin = $r.Content | ConvertFrom-Json
    } catch { }
    if ($null -eq $adminLogin -or [string]::IsNullOrEmpty($adminLogin.access)) { Fail "admin login failed for $adminEmail" }
    $adminHeaders = @{ Authorization = "Bearer $($adminLogin.access)" }
    $csv = $null
    try {
        $er = Invoke-WebRequest -UseBasicParsing -SkipHttpErrorCheck -Method Get -Uri "$BaseUrl/api/admin/audit/export" -Headers $adminHeaders -TimeoutSec 15
        if ($er.StatusCode -eq 200) { $csv = $er.Content }
    } catch { }
    if ([string]::IsNullOrEmpty($csv)) { Fail 'audit CSV export returned an empty/non-200 body' }
    if ($csv -notmatch 'text/csv') { Fail 'audit export is not served as text/csv' }
    $firstLine = ($csv -split "`r?`n" | Where-Object { $_ -ne "" } | Select-Object -First 1)
    if ($firstLine -ne 'id,time,actor,action,detail') {
        Fail "audit CSV header row missing (got: $firstLine)"
    }
    Write-Host "ok: /api/admin/audit/export returns RFC 4180 CSV" -ForegroundColor Green
}

Write-Host "SMOKE PASS" -ForegroundColor Green
exit 0