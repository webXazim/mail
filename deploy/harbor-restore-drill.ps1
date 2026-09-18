# WS6.2 hard-gate restore drill: prove the latest backup actually restores and
# that the restored data is fresh, using only local resources.
#
# The drill is NON-DESTRUCTIVE by construction: it restores into a throwaway
# database (harbor_restore_drill) on the same Postgres server, never into the
# live `harbor` database. It:
#   1. takes the newest deploy/backups/harbor-*.dump
#   2. pg_restore's it into the throwaway db (structural failures fail the drill)
#   3. asserts data freshness: row counts on the REAL tables must be identical
#      between the live db and the restored throwaway db (users, sessions,
#      audit_log, orders, contacts, calendar_events, send_counters)
#   4. drops the throwaway db and exits 0
#
# Example:
#   .\harbor-restore-drill.ps1                 # newest dump
#   .\harbor-restore-drill.ps1 -Dump .\backups\harbor-20260101-100000.dump
#
# Exit 0 on success, 1 if anything fails (CI-gate safe).

param(
    [string]$DbContainer = 'harbor-mail-db-1',
    [string]$DbUser = 'harbor',
    [string]$DbName = 'harbor',
    [string]$DrillDb = 'harbor_restore_drill',
    [string]$BackupDir = (Join-Path $PSScriptRoot 'backups'),
    [string]$Dump
)

$ErrorActionPreference = 'Stop'
function Fail([string]$msg) {
    Write-Host "RESTORE DRILL FAIL: $msg" -ForegroundColor Red
    exit 1
}

if (-not $Dump) {
    $candidates = Get-ChildItem -Path $BackupDir -Filter 'harbor-*.dump' |
        Sort-Object LastWriteTime -Descending
    if (-not $candidates) { Fail "no dump found under $BackupDir (run .\harbor-backup.ps1 first)" }
    $Dump = $candidates[0].FullName
}
if (-not (Test-Path $Dump)) { Fail "dump not found: $Dump" }

$running = docker inspect -f '{{.State.Running}}' $DbContainer 2>$null
if ($running.Trim() -ne 'true') { Fail "Postgres container '$DbContainer' is not running" }

$remote = Split-Path $Dump -Leaf

# 0. tear down any stale drill db (idempotent, never touches $DbName)
docker exec $DbContainer sh -c "dropdb -U '$DbUser' --if-exists '$DrillDb'" 2>$null | Out-Null
docker exec $DbContainer sh -c "createdb -U '$DbUser' '$DrillDb'" 2>$null
if ($LASTEXITCODE -ne 0) { Fail "could not create throwaway db '$DrillDb'" }

# copy the dump in + restore (fail fast on any structural restore error)
docker cp $Dump "${DbContainer}:/tmp/$remote" 2>$null
if ($LASTEXITCODE -ne 0) { Fail "failed to copy dump into $DbContainer" }
$restoreOut = docker exec $DbContainer sh -c "pg_restore -U '$DbUser' -d '$DrillDb' --no-owner --no-privileges /tmp/$remote && echo RESTORE_OK"
if (($restoreOut -join '') -notmatch 'RESTORE_OK') {
    Fail "pg_restore reported errors (structural): $($restoreOut -join ' ')"
}
docker exec $DbContainer rm -f "/tmp/$remote" 2>$null

# 1. freshness: live vs restored counts must be identical on real tables
$tables = @('users','sessions','audit_log','orders','contacts','calendar_events','send_counters')
$bad = @()
foreach ($t in $tables) {
    $liveN = docker exec $DbContainer psql -U $DbUser -d $DbName -tAc "select count(*) from $t" 2>$null
    $drillN = docker exec $DbContainer psql -U $DbUser -d $DrillDb -tAc "select count(*) from $t" 2>$null
    $liveN = ([string]$liveN).Trim(); $drillN = ([string]$drillN).Trim()
    if ($liveN -ne $drillN) { $bad += "$t live=$liveN restored=$drillN" }
}
if ($bad.Count -gt 0) {
    Fail "restore freshness mismatch: $($bad -join ', ')"
}

$total = (Get-Content (Join-Path $BackupDir (([IO.Path]::GetFileNameWithoutExtension($Dump))) -ErrorAction SilentlyContinue) -ErrorAction SilentlyContinue | Measure-Object).Count
Write-Host "freshness ok: live == restored on $($tables -join ', ')" -ForegroundColor Green

# 2. always drop the throwaway db so the drill is repeatable + non-destructive
docker exec $DbContainer sh -c "dropdb -U '$DbUser' '$DrillDb'" 2>$null

Write-Host "restore drill PASS: $([IO.Path]::GetFileName($Dump))" -ForegroundColor Green
exit 0