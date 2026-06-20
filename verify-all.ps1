# verify-all.ps1 — production verification gate for rustymelon.
#
# Runs the full behaviour-preservation suite and exits non-zero on any failure,
# so it can be used as a CI / pre-release gate:
#   1. melonink unit + differential-fuzz tests
#   2. whole-core stream parity (video+audio+saveRAM) across the game matrix
#   3. RA-visible main-RAM proof (masked snapshot) on RAM-deterministic games
#
# Requires a built `rusty` core + the stock `original` core in benchmarks/suite.cfg
# and a built melonbench. Edit the paths below if yours differ.

param(
    [string]$Melonbench = "E:\rustymelon-work\build\melonbench-target\release\melonbench.exe",
    [string]$Cargo      = "E:\rustymelon-work\rust\cargo\bin\cargo.exe",
    [string]$RamGames   = "sm64ds,mkds,nsmb"   # RAM-deterministic titles for ramverify
)
$ErrorActionPreference = "Stop"
$repo = $PSScriptRoot
$fail = $false

Write-Host "==> [1/3] melonink unit + fuzz tests"
$env:RUSTUP_HOME = "E:\rustymelon-work\rust\rustup"; $env:CARGO_HOME = "E:\rustymelon-work\rust\cargo"
$env:CARGO_TARGET_DIR = "E:\rustymelon-work\build\melonink-target"
Push-Location "$repo\melonink"
& $Cargo test --release
if ($LASTEXITCODE -ne 0) { $fail = $true; Write-Warning "melonink tests FAILED" }
Pop-Location

Write-Host "`n==> [2/3] whole-core stream parity (original vs rusty)"
Push-Location $repo
& $Melonbench suite --verify --frames 900 --cores original,rusty
if ($LASTEXITCODE -ne 0) { $fail = $true; Write-Warning "stream-parity gate FAILED" }
Pop-Location

Write-Host "`n==> [3/3] RetroAchievements-RAM proof (masked snapshots)"
$cfg = Get-Content "$repo\benchmarks\suite.cfg"
function CfgPath($key) { ($cfg | Where-Object { $_ -match "^$key\s" } | Select-Object -First 1) -replace "^$key\s+","" }
function GamePath($label) { (($cfg | Where-Object { $_ -match "^game\s+$label=" } | Select-Object -First 1) -replace "^game\s+$label=","" -split " ;")[0] }
$orig = ($cfg | Where-Object { $_ -match "^core\s+original=" }) -replace "^core\s+original=",""
$rusty = ($cfg | Where-Object { $_ -match "^core\s+rusty=" }) -replace "^core\s+rusty=",""
$sys = CfgPath "system_dir"; $sav = CfgPath "save_dir"; $opt = CfgPath "opt_file"
$tmp = Join-Path $env:TEMP "rustymelon-ramverify"; New-Item -ItemType Directory -Force $tmp | Out-Null
foreach ($g in ($RamGames -split ",")) {
    $rom = GamePath $g
    if (-not $rom -or -not (Test-Path $rom)) { Write-Warning "skip $g (ROM not found)"; continue }
    $base = @("--rom",$rom,"--system-dir",$sys,"--save-dir",$sav,"--opt",$opt,"--frames","500","--warmup","0","--verify")
    1..3 | ForEach-Object { & $Melonbench --core $orig @base --ram-dump "$tmp\$g-o$_.bin" 2>$null | Out-Null }
    & $Melonbench --core $rusty @base --ram-dump "$tmp\$g-r.bin" 2>$null | Out-Null
    Write-Host "  $g :" -NoNewline
    & $Melonbench ramverify --ref "$tmp\$g-o1.bin" --ref "$tmp\$g-o2.bin" --ref "$tmp\$g-o3.bin" --cand "$tmp\$g-r.bin" 2>&1 | Select-String "PASS|FAIL" | ForEach-Object { " $($_.Line.Trim())" }
    if ($LASTEXITCODE -ne 0) { $fail = $true }
}

Write-Host ""
if ($fail) { Write-Host "VERIFY-ALL: FAILED" -ForegroundColor Red; exit 1 }
Write-Host "VERIFY-ALL: PASS — rustymelon is behaviour-identical to stock." -ForegroundColor Green
