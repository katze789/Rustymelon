# build.ps1 — fetch sources, apply the rustymelon patch, build melonink + core.
#
# One-shot reproducible build for the rustymelon practice core on Windows.
# Requires: git, Rust (rustup, x86_64-pc-windows-gnu), and an MSYS2 MinGW-w64
# toolchain (gcc, cmake >= 3.19, ninja). Edit the paths below if yours differ.
#
#   pwsh ./build.ps1                 # build the rusty core (MELONINK on)
#   pwsh ./build.ps1 -Baseline       # build unmodified upstream (MELONINK off)

param(
    [string]$WorkDir = "$PSScriptRoot\build",
    [string]$Msys2   = "C:\msys64",
    [string]$MelonTag = "f6692dff8c0c53f77639a08e5e746a286312bb41",
    [switch]$Baseline
)
$ErrorActionPreference = "Stop"
$repo = $PSScriptRoot
$bash = "$Msys2\usr\bin\bash.exe"
function ToPosix([string]$p) { ($p -replace '\\','/' -replace '^([A-Za-z]):','/$1'.ToLower()) }

New-Item -ItemType Directory -Force $WorkDir | Out-Null

# 1. Fetch melonds-ds (the libretro core) and the pinned melonDS, apply patches.
$mdsds = "$WorkDir\melonds-ds"
if (-not (Test-Path $mdsds)) {
    git clone --recursive --depth 1 https://github.com/JesseTG/melonds-ds $mdsds
    if (-not $Baseline) {
        # rustymelon identity patch (distinct core name; only for the rusty build)
        git -C $mdsds apply "$repo\patches\melonds-ds-rustymelon.patch"
        Write-Host "Applied patches/melonds-ds-rustymelon.patch (distinct rustymelon identity)"
    }
}
$melon = "$WorkDir\melonDS-patched"
if (-not (Test-Path $melon)) {
    git clone https://github.com/JesseTG/melonDS $melon
    git -C $melon checkout $MelonTag
    git -C $melon apply "$repo\patches\melonds-rustymelon.patch"
    Write-Host "Applied patches/melonds-rustymelon.patch onto melonDS@$($MelonTag.Substring(0,7))"
}

# 2. Build the melonink Rust staticlib (no_std, panic=abort).
$lib = ""
if (-not $Baseline) {
    Push-Location "$repo\melonink"
    $env:CARGO_TARGET_DIR = "$WorkDir\melonink-target"
    cargo build --release --no-default-features
    Pop-Location
    $lib = "$WorkDir\melonink-target\release\libmelonink.a"
    if (-not (Test-Path $lib)) { throw "melonink build failed: $lib not found" }
}

# 3. Configure + build the core with the patched melonDS and (optionally) melonink.
$buildDir = if ($Baseline) { "$WorkDir\core-baseline" } else { "$WorkDir\core-rusty" }
$args = @(
    "cmake -S '$(ToPosix $mdsds)' -B '$(ToPosix $buildDir)' -G Ninja",
    "-DCMAKE_BUILD_TYPE=Release",
    "-DFETCHCONTENT_SOURCE_DIR_MELONDS='$(ToPosix $melon)'"
)
if (-not $Baseline) { $args += "-DMELONINK_LIB='$(ToPosix $lib)'" }
$cfg = $args -join ' '
$env:MSYSTEM = "MINGW64"
& $bash -lc "$cfg && cmake --build '$(ToPosix $buildDir)'"

$dll = "$buildDir\src\libretro\melondsds_libretro.dll"
Write-Host "`nBuilt: $dll"
Write-Host "(Verify before use:  melonbench suite --verify --cores original,rusty)"
