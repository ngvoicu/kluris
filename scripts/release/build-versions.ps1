# Build this tree as several releases that differ only in their version, for
# the Windows life-cycle smoke (lifecycle-smoke.ps1); build-versions.sh is the
# macOS/Linux one. Each version is a copy of the tree with
# `[workspace.package] version` changed, built by build-local.ps1, its
# target\distrib copied to <Out>\v<version>\.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release\build-versions.ps1 -Out <dir> -Versions 3.0.0,3.0.1
#
# -Versions is one comma-separated word: `-File` hands a script its
# arguments as strings, never as arrays.
#
# The copies build into this tree's target folder (CARGO_TARGET_DIR, else
# target\) and use its tools (KLURIS_TOOLS_DIR, else target\tools).
param(
    [Parameter(Mandatory = $true)][string]$Out,
    [Parameter(Mandatory = $true)][string]$Versions
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $root 'target' }
if (-not $env:KLURIS_TOOLS_DIR) { $env:KLURIS_TOOLS_DIR = Join-Path $root 'target\tools' }
$null = New-Item -ItemType Directory -Force -Path $Out
$Out = (Resolve-Path -LiteralPath $Out).Path
$work = Join-Path $root 'target\release-versions'
Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
$null = New-Item -ItemType Directory -Force -Path $work
$powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$tar = Join-Path $env:SystemRoot 'System32\tar.exe'
$utf8 = New-Object Text.UTF8Encoding($false)
# Native commands from here on, their status checked by hand (Windows
# PowerShell 5.1 turns a native command's redirected stderr into an error).
$ErrorActionPreference = 'Continue'

foreach ($version in @($Versions -split ',' | ForEach-Object { $_.Trim() } | Where-Object { $_ })) {
    if ($version -notmatch '^[0-9][0-9A-Za-z.+-]*$') { throw "not a version: '$version'" }
    $copy = Join-Path $work "kluris-$version"
    $null = New-Item -ItemType Directory -Force -Path $copy
    $archive = Join-Path $work 'tree.tar'
    Push-Location -LiteralPath $root
    & $tar -cf $archive --exclude=./target --exclude=./tmp --exclude=./.git --exclude=./parity/.oracle --exclude=./parity/.venv .
    $code = $LASTEXITCODE
    Pop-Location
    if ($code -ne 0) { throw "tar of the tree failed ($code)" }
    & $tar -xf $archive -C $copy
    if ($LASTEXITCODE -ne 0) { throw "tar -x into $copy failed ($LASTEXITCODE)" }
    Remove-Item -LiteralPath $archive -Force

    $manifest = Join-Path $copy 'Cargo.toml'
    $text = [IO.File]::ReadAllText($manifest)
    $pattern = New-Object Text.RegularExpressions.Regex('(?m)^version = "[^"]*"$')
    if (-not $pattern.IsMatch($text)) { throw 'no [workspace.package] version line in Cargo.toml' }
    [IO.File]::WriteAllText($manifest, $pattern.Replace($text, "version = `"$version`"", 1), $utf8)

    Push-Location -LiteralPath $copy
    & cargo metadata --format-version 1 --offline | Out-Null
    $code = $LASTEXITCODE
    Pop-Location
    if ($code -ne 0) { throw "cargo metadata in the copy failed ($code)" }
    $env:GIT_CEILING_DIRECTORIES = $work
    & $powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $copy 'scripts\release\build-local.ps1') | Out-Null
    $code = $LASTEXITCODE
    Remove-Item Env:\GIT_CEILING_DIRECTORIES
    if ($code -ne 0) { throw "build-local.ps1 of $version failed ($code)" }

    $dest = Join-Path $Out "v$version"
    Remove-Item -LiteralPath $dest -Recurse -Force -ErrorAction SilentlyContinue
    $null = New-Item -ItemType Directory -Force -Path $dest
    Get-ChildItem -LiteralPath (Join-Path $copy 'target\distrib') -File | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $dest }
    $named = Select-String -LiteralPath (Join-Path $dest 'kluris-installer.ps1') -Pattern "^\`$app_version = '$([regex]::Escape($version))'$" -Quiet
    if (-not $named) { throw "the installer of v$version names another version" }
    Write-Output "built v$version into $dest"
}
Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
exit 0
