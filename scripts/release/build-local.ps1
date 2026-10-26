# Build a local release for this Windows machine the way release.yml does on
# GitHub (build-local.sh is the macOS/Linux one): dist's local artifacts for
# one target (the zip and its .sha256), its manifest beside them, then the
# global ones (kluris-installer.ps1, kluris-installer.sh, sha256.sum, the
# source tarball), into target\distrib.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release\build-local.ps1 [-Target <triple>]
#
# dist 0.33.1 (x86_64; it runs under emulation on arm64) is fetched once into
# KLURIS_TOOLS_DIR (else target\tools), its zip checked against the release
# asset's SHA-256. dist makes the source tarball with `git archive HEAD`, so
# a tree that is no git checkout (the unpacked tarball of `cargo xtask
# windows`) is made a one-commit repository of its own first.
param([string]$Target = '')

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$distVersion = '0.33.1'
$distSha256 = '79dfcce84e6a8e69210f11fc23d445c3aac7af6a29e0e981867f053c0a869316'
$distUrl = "https://github.com/axodotdev/cargo-dist/releases/download/v$distVersion/cargo-dist-x86_64-pc-windows-msvc.zip"

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$tools = if ($env:KLURIS_TOOLS_DIR) { $env:KLURIS_TOOLS_DIR } else { Join-Path $root 'target\tools' }
$distDir = Join-Path $tools "dist-$distVersion-windows-x86_64"
$dist = Join-Path $distDir 'dist.exe'
if (-not (Test-Path -LiteralPath $dist -PathType Leaf)) {
    $null = New-Item -ItemType Directory -Force -Path $tools
    $zip = Join-Path $tools "dist-$distVersion.zip"
    $part = "$distDir.part"
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -UseBasicParsing -TimeoutSec 300 -Uri $distUrl -OutFile $zip
        $sha = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
        if ($sha -ne $distSha256) { throw "the dist zip hashes to $sha, not $distSha256" }
        Remove-Item -LiteralPath $part -Recurse -Force -ErrorAction SilentlyContinue
        Expand-Archive -LiteralPath $zip -DestinationPath $part -Force
        Move-Item -LiteralPath $part -Destination $distDir
    } finally {
        Remove-Item -LiteralPath $zip -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $part -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# Native commands from here on, their status checked by hand: Windows
# PowerShell 5.1 turns a native command's redirected stderr into an error,
# which 'Stop' would throw.
$ErrorActionPreference = 'Continue'
if (-not $Target) {
    $hostLine = & rustc -vV | Where-Object { $_ -like 'host: *' }
    if ($LASTEXITCODE -ne 0 -or -not $hostLine) { throw 'rustc -vV names no host target' }
    $Target = "$hostLine".Substring(6).Trim()
}

Set-Location -LiteralPath $root
& git rev-parse --is-inside-work-tree 2>$null | Out-Null
if ($LASTEXITCODE -ne 0) {
    & git init -q
    & git add -A
    & git -c user.name=kluris-ci -c user.email=kluris-ci@localhost -c core.autocrlf=false commit -q -m 'release tree'
    if ($LASTEXITCODE -ne 0) { throw 'could not make the tree a git repository for the source tarball' }
}

$out = Join-Path $root 'target\distrib'
$manifests = Join-Path $root 'target\dist-manifests'
Remove-Item -LiteralPath $out, $manifests -Recurse -Force -ErrorAction SilentlyContinue
$null = New-Item -ItemType Directory -Force -Path $manifests
$utf8 = New-Object Text.UTF8Encoding($false)

$local = & $dist build --output-format=json --artifacts=local "--target=$Target"
if ($LASTEXITCODE -ne 0) { throw "dist build --artifacts=local failed ($LASTEXITCODE)" }
[IO.File]::WriteAllText((Join-Path $manifests 'local.json'), ($local -join "`n"), $utf8)
[IO.File]::WriteAllText((Join-Path $out "$Target-dist-manifest.json"), ($local -join "`n"), $utf8)
$global = & $dist build --output-format=json --artifacts=global
if ($LASTEXITCODE -ne 0) { throw "dist build --artifacts=global failed ($LASTEXITCODE)" }
[IO.File]::WriteAllText((Join-Path $manifests 'global.json'), ($global -join "`n"), $utf8)
Get-ChildItem -LiteralPath $out -Name | ForEach-Object { Write-Output "  $_" }
Write-Output $out
