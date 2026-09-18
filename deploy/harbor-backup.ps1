# WS6.2 hard-gate backup: automated `pg_dump` of the live `harbor` database.
#
# Covers every table in the schema (users, sessions, audit_log, settings,
# statistics counters, billing/orders, contacts, calendar, mail workflows...)
# in custom format (-Fc) so the companion restore drill can pg_restore it.
# Written to deploy/backups with a sidecar manifest (pg version, row counts
# on key tables, SHA-256) so a restore drill can prove data freshness, and a
# retention sweep keeps the newest N days.
#
# Fully local: runs via the api's own Postgres container. Examples:
#   .\harbor-backup.ps1
#   .\harbor-backup.ps1 -RetainDays 7
#   .\harbor-backup.ps1 -DbContainer harbor-mail-db-1 -DbUser harbor -DbName harbor
#
# Exit 0 on success, 1 on any failure (CI-gate safe).

param(
    [string]$DbContainer = 'harbor-mail-db-1',
    [string]$DbUser = 'harbor',
    [string]$DbName = 'harbor',
    [int]$RetainDays = 14,
    [string]$BackupDir = (Join-Path $PSScriptRoot 'backups')
)

$ErrorActionPreference = 'Stop'
function Fail([string]$msg) {
    Write-Host "BACKUP FAIL: $msg" -ForegroundColor Red
    exit 1
}

if (-not $DbContainer) { Fail 'DbContainer is empty' }
if (-not $DbName)      { Fail 'DbName is empty' }
if (-not (Get-Command docker -ErrorAction SilentlyContinue)) { Fail 'docker CLI not found' }

$running = docker inspect -f '{{.State.Running}}' $DbContainer 2>$null
if ($running.Trim() -ne 'true') { Fail "Postgres container '$DbContainer' is not running" }

# sanity: pg tools inside the container really reach the db
$probe = docker exec $DbContainer pg_isready -U $DbUser -d $DbName 2>&1
$probeText = $probe -join ' '
if (($probe -join '') -notmatch 'accepting') { Fail "pg_isready in ${DbContainer}: $probeText" }

if (-not (Test-Path $BackupDir)) {
    New-Item -ItemType Directory -Path $BackupDir -Force | Out-Null
}

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$dumpName = "harbor-$stamp.dump"
$manifestName = "harbor-$stamp.manifest.txt"

Write-Host "dumping $($DbContainer):/$DbName -> $dumpName ..." -ForegroundColor Yellow
$rc = docker exec $DbContainer sh -c "pg_dump -U '$DbUser' -d '$DbName' -Fc -f /tmp/$dumpName && echo PGDUMP_OK"
if (($rc -join '') -notmatch 'PGDUMP_OK') {
    Fail "pg_dump failed: $($rc -join ' ')"
}
docker cp "${DbContainer}:/tmp/$dumpName" (Join-Path $BackupDir $dumpName) 2>$null
if (-not (Test-Path (Join-Path $BackupDir $dumpName))) {
    Fail "dump file was not copied out of the container ($dumpName)"
}
docker exec $DbContainer rm -f "/tmp/$dumpName" 2>$null

$localDump = Join-Path $BackupDir $dumpName
if ((Get-Item $localDump).Length -eq 0) { Fail "dump file is empty after copy-out ($dumpName)" }

# manifest: tooling versions + freshness anchors (real table row counts)
$pgVer = docker exec $DbContainer psql -U $DbUser -d $DbName -tAc "show server_version" 2>$null
$counts = @{}
'users','sessions','audit_log','orders','contacts','calendar_events','send_counters' | ForEach-Object {
    $t = $_
    $n = docker exec $DbContainer psql -U $DbUser -d $DbName -tAc "select count(*) from $t" 2>$null
    $counts[$t] = ([string]$n).Trim()
}
$hash = (Get-FileHash -Path $localDump -Algorithm SHA256).Hash.ToLowerInvariant()
$manifest = @(
    "name=$dumpName",
    "created=$((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'))",
    "db=$($DbContainer):/$DbName user=$DbUser",
    "pg_version=$($pgVer.Trim())",
    "sha256=$hash",
    "bytes=$((Get-Item $localDump).Length)",
    $counts.GetEnumerator() | Sort-Object Name | ForEach-Object { "rows.$($_.Key)=$($_.Value)" }
) -join "`n"
$manifestPath = Join-Path $BackupDir $manifestName
Set-Content -Path $manifestPath -Value $manifest -Encoding utf8

# retention sweep (keep newest $RetainDays of dumps + manifests)
if ($RetainDays -gt 0) {
    $cutoff = (Get-Date).AddDays(-$RetainDays)
    Get-ChildItem -Path $BackupDir -Filter 'harbor-*.dump' |
        Where-Object { $_.LastWriteTime -lt $cutoff } | ForEach-Object {
            $base = $_.FullName -replace '\.dump$',''
            $_ | Remove-Item -Force
            foreach ($m in @("$base.manifest.txt")) { if (Test-Path $m) { Remove-Item -Force $m } }
            Write-Host "pruned $($_.Name)" -ForegroundColor DarkGray
        }
}

$mb = [math]::Round((Get-Item $localDump).Length / 1MB, 2)
Write-Host "done: $dumpName ($mb MB, pg $($pgVer.Trim()), sha256 $($hash.Substring(0,16))...)" -ForegroundColor Green
exit 0