# WS2.x hard-gate probe (fully local, no external DNS/MX — the code-level
# real-mail proof for production-readiness; VPS + crescentsphere.com DNS + true
# cross-host deliverability are wired by the WS7 deploy drill once we own the
# box and the domain, per the LAUNCH first-three + WS6 gating).
#
# Chain proven against the LIVE local stack:
#   register A + B  (crescentsphere.com, real Stalwart-backed mailbox; api
#     returns dev.verify_link)
#   consume verify token -> POST /api/auth/verify  for BOTH accounts
#   login A + B (verification now satisfied => real access tokens)
#   A -> B: POST /api/send with a UNIQUE subject (real SMTP submission via the
#     api's relay to the local Stalwart)
#   B: poll /api/mail/search for the unique subject; assert sender=to=subject
#
# Exit codes (CI-gate safe):
#   0  PASS — real A->B local round trip proven (the M3 core)
#   25 SKIP — no live stack reachable (docker/health)
#   1  FAIL
#
# Run:  .\deploy\harbor-mail-e2e.ps1
# Env overrides: HARBOR_SMOKE_BASE_URL, HARBOR_SMOKE_DOMAIN.

$ErrorActionPreference = 'Stop'
function Fail([string]$msg) { Write-Host "MAIL E2E FAIL: $msg" -ForegroundColor Red; exit 1 }
$BaseUrl = if ($env:HARBOR_SMOKE_BASE_URL) { $env:HARBOR_SMOKE_BASE_URL } else { 'http://127.0.0.1:8080' }
$Domain = if ($env:HARBOR_SMOKE_DOMAIN) { $env:HARBOR_SMOKE_DOMAIN } else { 'crescentsphere.com' }

function Call([string]$m, [string]$path, [hashtable]$headers = @{}, $body = $null) {
    $p = @{ Method = $m; Uri = "$BaseUrl$path"; Headers = $headers; TimeoutSec = 20 }
    if ($null -ne $body) { $p.ContentType = 'application/json'; $p.Body = ($body | ConvertTo-Json -Compress) }
    try {
        (Invoke-WebRequest -UseBasicParsing -SkipHttpErrorCheck @p) | ForEach-Object {
            [PSCustomObject]@{ StatusCode = $_.StatusCode; Content = $_.Content }
        }
    } catch { $null }
}

Write-Host "WS2.x mail e2e: base=$BaseUrl domain=$Domain" -ForegroundColor Yellow

$hb = Invoke-WebRequest -UseBasicParsing -Method Get -Uri "$BaseUrl/api/health" -TimeoutSec 8
if ($hb.StatusCode -ne 200) { Fail "health not 200 (stack down?) — got $($hb.StatusCode)" }

# 1. register two unique throwaway accounts on the real local domain
$stamp = (Get-Date -Format 'yyyyMMddHHmmss') + (Get-Random -Minimum 1000 -Maximum 9999)
$emailA = "e2ea-$stamp@$Domain"
$emailB = "e2eb-$stamp@$Domain"
$pass = 'HarborE2E!2026str0ng'
$aReg = Call POST '/api/auth/register' @{} @{ name = 'E2E A'; email = $emailA; password = $pass }
$bReg = Call POST '/api/auth/register' @{} @{ name = 'E2E B'; email = $emailB; password = $pass }
if ($null -eq $aReg -or $aReg.StatusCode -ge 400) { Fail "register A status=$($aReg.StatusCode) body=$($aReg.Content)" }
if ($null -eq $bReg -or $bReg.StatusCode -ge 400) { Fail "register B status=$($bReg.StatusCode) body=$($bReg.Content)" }
Write-Host "ok: registered A=$emailA" -ForegroundColor Green
Write-Host "ok: registered B=$emailB" -ForegroundColor Green

# 2. consume the dev verify token each register returned (real verify hop)
foreach ($pair in @(@{ r = $aReg; e = $emailA }, @{ r = $bReg; e = $emailB })) {
    $j = $pair.r.Content | ConvertFrom-Json
    $link = $null
    if ($null -ne $j.dev -and $null -ne $j.dev.verify_link) { $link = [string]$j.dev.verify_link }
    if ([string]::IsNullOrEmpty($link)) {
        # fallback: dev verify_link may live at top-level dev.verify_token
        if ($null -ne $j.dev -and $null -ne $j.dev.verify_token) { $link = [string]$j.dev.verify_token }
    }
    if ([string]::IsNullOrEmpty($link)) { Fail "no dev verification link returned for $($pair.e): $($pair.r.Content.Substring(0,[Math]::Min(180,$pair.r.Content.Length)))" }
    $tok = $link
    if ($link -match 'token=([0-9a-zA-Z]+)') { $tok = $matches[1] }
    $v = Call POST '/api/auth/verify' @{} @{ token = $tok }
    if ($null -eq $v -or $v.StatusCode -ge 400) { Fail "verify $($pair.e) status=$($v.StatusCode) body=$($v.Content)" }
    Write-Host "ok: verified $($pair.e)" -ForegroundColor Green
}

# 3. login both now that verification is satisfied
function Login([string]$email) {
    $l = Call POST '/api/auth/login' @{} @{ email = $email; password = $pass }
    if ($null -eq $l -or $l.StatusCode -ge 400) { Fail "login $email status=$($l.StatusCode) body=$($l.Content)" }
    $lj = $l.Content | ConvertFrom-Json
    if ([string]::IsNullOrEmpty($lj.access)) { Fail "login $email returned no access token" }
    $lj.access
}
$aTok = Login $emailA
$bTok = Login $emailB
Write-Host "ok: logged in A + B (access tokens minted)" -ForegroundColor Green

# 4. A -> B real SMTP submission with a UNIQUE subject
$subject = "local real mail round trip $stamp"
$s = Call POST '/api/send' @{ Authorization = "Bearer $aTok" } @{
    to      = @(@{ email = $emailB })
    subject = $subject
    body_text = "hello B — real local SMTP submission from A on $Domain"
}
if ($null -eq $s -or $s.StatusCode -ge 400) { Fail "send A->B status=$($s.StatusCode) body=$($s.Content)" }
Write-Host "ok: A -> B SMTP submitted (status $($s.StatusCode))" -ForegroundColor Green

# 5. B searched for its unique subject back (retry so async SMTP->JMAP settle)
$found = $false
for ($i = 0; $i -lt 8 -and -not $found; $i++) {
    $q = [uri]::EscapeDataString($subject)
    $m = Call GET "/api/mail/search?limit=20&q=$q" @{ Authorization = "Bearer $bTok" }
    if ($null -ne $m -and $m.StatusCode -eq 200 -and ($m.Content -match [regex]::Escape($emailA)) -and ($m.Content -match [regex]::Escape($subject))) {
        $found = $true
    } elseif ($i -lt 7) { Start-Sleep -Milliseconds 1200 }
}
if (-not $found) { Fail "B's mailbox never showed the unique subject after 8x1.2s" }

Write-Host "MAIL E2E: PASS — A($emailA) -> B($emailB) real local round trip (register+verify+login+SMTP send+JMAP search read-back, subject='$subject')" -ForegroundColor Green
exit 0