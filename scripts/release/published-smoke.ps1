# Install a release PUBLISHED on GitHub through the LIVE site wrapper, on
# Windows, into a sandbox profile, check it and uninstall it:
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release\published-smoke.ps1 `
#       -Version 3.0.0-rc.1 [-Site <install.ps1 URL>] [-Record <file>]
#
# The site URL defaults to https://kluris.ngvoicu.dev/install.ps1. The
# wrapper runs the documented way, `powershell -ExecutionPolicy ByPass -c
# "irm <url> | iex"`, with KLURIS_VERSION=<version>, so it fetches
# github.com/ngvoicu/kluris/releases/download/v<version>/kluris-installer.ps1.
#
# The installer runs as the current user with HOME, USERPROFILE and
# LOCALAPPDATA pointed at a sandbox (and PowerShell's own $HOME set to it),
# so kluris.exe and the receipt land there; the real profile's are compared
# before and after. The one thing it changes outside the sandbox is the user
# PATH (HKCU\Environment\Path): user-path.ps1 saves its exact bytes and
# registry type into -Record first, and they are ALWAYS written back at the
# end, read again and compared byte for byte (`cargo xtask windows` writes a
# record left by an ended smoke back after the step and in its stop). The
# system PATH (HKLM) is only read, before and after, and must not change.
#
# Checks: kluris.exe in the sandbox's .kluris\bin; the receipt naming it and
# <version>; the user PATH starting with that folder as REG_EXPAND_SZ; a
# fresh process (PATH built from the registry) finding it first; `kluris
# --version` = `kluris <version>`; `kluris doctor --json` ok with this
# binary first on PATH; `kluris self-update --check --json` against the real
# GitHub release listing reporting no update (prereleases are skipped, v2
# releases never win); `kluris self-uninstall --yes --json` removing
# kluris.exe, the receipt and its user PATH entry.
#
# Exits 0 when every check passes and the PATH comparison holds, 1 otherwise.
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$Site = 'https://kluris.ngvoicu.dev/install.ps1',
    [string]$Record = ''
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Step([string]$text) { Write-Output "published-smoke: $text" }

$Version = $Version -replace '^v', ''
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$') {
    Write-Output "published-smoke: -Version is a release number such as 3.0.0-rc.1, not '$Version'"
    exit 2
}
if (-not $Record) { $Record = Join-Path ([IO.Path]::GetTempPath()) 'kluris-published-smoke.user-path' }

$powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$userPath = Join-Path $PSScriptRoot 'user-path.ps1'

# The system PATH and the user PATH as Windows keeps them (unexpanded).
function Read-Environment([Microsoft.Win32.RegistryKey]$hive, [string]$sub) {
    $key = $hive.OpenSubKey($sub)
    try {
        if ($null -eq $key.GetValue('Path')) { return @{ Kind = 'absent'; Text = '' } }
        return @{ Kind = $key.GetValueKind('Path').ToString(); Text = $key.GetValue('Path', '', 'DoNotExpandEnvironmentNames') }
    } finally { $key.Close() }
}
$machineKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
$machineBefore = Read-Environment ([Microsoft.Win32.Registry]::LocalMachine) $machineKey

$saved = & $powershell -NoProfile -ExecutionPolicy Bypass -File $userPath -Save $Record
$saveCode = $LASTEXITCODE
$saved | ForEach-Object { Write-Output "published-smoke: $_" }
if ($saveCode -ne 0) {
    Write-Output 'published-smoke: FAIL: the user PATH could not be saved, so nothing was installed'
    exit 1
}

$work = Join-Path ([IO.Path]::GetTempPath()) ("kluris-published-smoke-" + [guid]::NewGuid().ToString('N').Substring(0, 12))
$sandbox = Join-Path $work 'home'
$localAppData = Join-Path $sandbox 'AppData\Local'
$failure = $null
$restored = $false

# A process with the given environment changes, its output and exit code.
function Invoke-Child([string]$exe, [string]$arguments, [hashtable]$environment, [string]$directory) {
    $start = New-Object Diagnostics.ProcessStartInfo($exe, $arguments)
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.WorkingDirectory = $directory
    foreach ($name in $environment.Keys) {
        if ($null -eq $environment[$name]) { $start.EnvironmentVariables.Remove($name) } else { $start.EnvironmentVariables[$name] = $environment[$name] }
    }
    $process = [Diagnostics.Process]::Start($start)
    $errTask = $process.StandardError.ReadToEndAsync()
    $out = $process.StandardOutput.ReadToEnd()
    $process.WaitForExit()
    return @{ Code = $process.ExitCode; Out = $out; Err = $errTask.Result }
}

$sandboxEnv = @{
    HOME = $sandbox; USERPROFILE = $sandbox; LOCALAPPDATA = $localAppData
    XDG_CONFIG_HOME = $null; GITHUB_PATH = $null; KLURIS_INSTALL_DIR = $null; KLURIS_NO_MODIFY_PATH = $null
    KLURIS_GITHUB_API_URL = $null; KLURIS_GITHUB_TOKEN = $null; KLURIS_RELEASES_URL = $null; KLURIS_DOWNLOAD_URL = $null
    KLURIS_NO_PROMPT = '1'; NO_COLOR = '1'
}
# `powershell -ExecutionPolicy ByPass -Command <command>` with the sandbox's
# environment, PowerShell's own $HOME set to the sandbox first.
function Invoke-Installer([string]$command, [hashtable]$extra) {
    $quoted = $sandbox.Replace("'", "''")
    $script = "`$ErrorActionPreference = 'Stop'; Set-Variable -Name HOME -Value '$quoted' -Force -Scope Global; if (`$HOME -ne '$quoted') { throw 'HOME is not the sandbox' }; `$ErrorActionPreference = 'Continue'; $command"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script))
    $environment = $sandboxEnv.Clone()
    foreach ($name in $extra.Keys) { $environment[$name] = $extra[$name] }
    return Invoke-Child $powershell "-NoProfile -NonInteractive -ExecutionPolicy ByPass -EncodedCommand $encoded" $environment $work
}

function Hash-Of([string]$path) { if (Test-Path -LiteralPath $path) { (Get-FileHash -LiteralPath $path).Hash } else { 'absent' } }

# The last non-empty line of a command's output, as JSON.
function Json-Of($result, [string]$what) {
    $last = @($result.Out -split "`r?`n" | Where-Object { $_.Trim() })[-1]
    try { return $last | ConvertFrom-Json } catch { throw "$what printed no JSON ($($_.Exception.Message)): stdout [$($result.Out)] stderr [$($result.Err)]" }
}

try {
    $null = New-Item -ItemType Directory -Force -Path $sandbox, $localAppData
    $realBin = Join-Path $env:USERPROFILE '.kluris\bin\kluris.exe'
    $realReceipt = Join-Path $env:LOCALAPPDATA 'kluris\kluris-receipt.json'
    $realBefore = "$(Hash-Of $realBin) $(Hash-Of $realReceipt)"

    Step "installing kluris $Version through $Site"
    $install = Invoke-Installer "irm '$Site' | iex" @{ KLURIS_VERSION = $Version }
    Write-Output $install.Out
    # Windows PowerShell writes its progress and information records to a
    # redirected stderr as CLIXML: shown only when the install failed.
    if ($install.Code -ne 0) { throw "the install exited with $($install.Code): $($install.Err)" }
    if ($install.Out -notmatch 'kluris installed\. Open a new terminal, then run: kluris doctor') { throw "the installer did not print the success message: $($install.Err)" }

    $bin = Join-Path $sandbox '.kluris\bin'
    $exe = Join-Path $bin 'kluris.exe'
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "no $exe" }
    $receiptFile = Join-Path $localAppData 'kluris\kluris-receipt.json'
    if (-not (Test-Path -LiteralPath $receiptFile)) { throw "no receipt at $receiptFile" }
    $receiptText = [IO.File]::ReadAllText($receiptFile)
    $receipt = $receiptText | ConvertFrom-Json
    if ($receipt.install_prefix -ne $bin) { throw "the receipt's install_prefix is $($receipt.install_prefix)" }
    if ($receipt.version -ne $Version) { throw "the receipt's version is $($receipt.version)" }
    Step "installed: $exe; the receipt: $($receiptText.Trim())"

    $user = Read-Environment ([Microsoft.Win32.Registry]::CurrentUser) 'Environment'
    if ($user.Kind -ne 'ExpandString') { throw "the user PATH is $($user.Kind), not REG_EXPAND_SZ" }
    $firstEntry = ($user.Text -split ';')[0]
    if ($firstEntry -ne $bin) { throw "the user PATH starts with '$firstEntry', not $bin" }
    Step "user PATH starts with $bin (REG_EXPAND_SZ)"

    # A new terminal: PATH as Windows builds it, the system's then the user's.
    $fresh = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
    $freshEnv = $sandboxEnv.Clone()
    $freshEnv['Path'] = $fresh
    $where = Invoke-Child (Join-Path $env:SystemRoot 'System32\where.exe') 'kluris' $freshEnv $work
    $found = @($where.Out -split "`r?`n" | Where-Object { $_ })[0]
    if ($found -ne $exe) { throw "a fresh process finds kluris at '$found'" }
    Step "a fresh process finds $found"
    $ver = Invoke-Child $found '--version' $freshEnv $work
    if ($ver.Out.Trim() -ne "kluris $Version") { throw "kluris --version printed '$($ver.Out.Trim())'" }
    Step "kluris --version: $($ver.Out.Trim())"

    $doctor = Invoke-Child $found 'doctor --json' $freshEnv $work
    if ($doctor.Code -ne 0) { throw "kluris doctor --json exited $($doctor.Code): $($doctor.Out) $($doctor.Err)" }
    $report = Json-Of $doctor 'kluris doctor --json'
    $pathCheck = $report.checks | Where-Object { $_.name -eq 'path' }
    if (-not $report.ok -or -not $pathCheck.passed -or $pathCheck.warning -or $pathCheck.detail -ne $exe) { throw "kluris doctor --json is not ok with this binary first on PATH: $($doctor.Out)" }
    Step "kluris doctor --json: ok; $(($report.checks | ForEach-Object { "$($_.name) $(if ($_.passed) { 'passed' } else { 'FAILED' })" }) -join ', ')"

    $check = Invoke-Child $found 'self-update --check --json' $freshEnv $work
    if ($check.Code -ne 0) { throw "kluris self-update --check --json exited $($check.Code): $($check.Out) $($check.Err)" }
    $update = Json-Of $check 'kluris self-update --check --json'
    if ($update.ok -ne $true -or $update.current -ne $Version -or $update.update_available -ne $false) { throw "kluris self-update --check --json: $($check.Out)" }
    Step "self-update --check --json: $(@($check.Out -split "`r?`n" | Where-Object { $_.Trim() })[-1])"

    $uninstall = Invoke-Child $found 'self-uninstall --yes --json' $freshEnv $work
    if ($uninstall.Code -ne 0) { throw "kluris self-uninstall --yes --json exited $($uninstall.Code): $($uninstall.Out) $($uninstall.Err)" }
    $line = @($uninstall.Out -split "`r?`n" | Where-Object { $_.Trim() })[-1]
    Step "self-uninstall --yes --json: $($line.Substring(0, [Math]::Min(400, $line.Length)))"
    # The running kluris.exe goes once it has exited.
    for ($i = 0; (Test-Path -LiteralPath $exe) -and $i -lt 50; $i++) { Start-Sleep -Milliseconds 200 }
    if (Test-Path -LiteralPath $exe) { throw "self-uninstall left $exe" }
    if (Test-Path -LiteralPath $receiptFile) { throw "self-uninstall left $receiptFile" }
    $userAfter = Read-Environment ([Microsoft.Win32.Registry]::CurrentUser) 'Environment'
    if (@($userAfter.Text -split ';') -contains $bin) { throw "self-uninstall left $bin in the user PATH" }
    Step "uninstalled: kluris.exe, the receipt and the user PATH entry are gone"

    $machineNow = Read-Environment ([Microsoft.Win32.Registry]::LocalMachine) $machineKey
    if ($machineNow.Kind -ne $machineBefore.Kind -or $machineNow.Text -cne $machineBefore.Text) { throw 'the system PATH changed' }
    $realAfter = "$(Hash-Of $realBin) $(Hash-Of $realReceipt)"
    if ($realAfter -ne $realBefore) { throw "the real profile's kluris.exe or receipt changed: $realBefore -> $realAfter" }
    Step "system PATH unchanged; the real profile's kluris.exe and receipt unchanged"
} catch {
    $failure = $_
} finally {
    $back = & $powershell -NoProfile -ExecutionPolicy Bypass -File $userPath -Restore $Record
    $restored = ($LASTEXITCODE -eq 0)
    $back | ForEach-Object { Write-Output "published-smoke: $_" }
    Start-Sleep -Milliseconds 300
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
if ($failure) {
    Write-Output "published-smoke: FAIL: $($failure.Exception.Message)"
    exit 1
}
if (-not $restored) {
    Write-Output 'published-smoke: FAIL: the user PATH was not written back identically'
    exit 1
}
Step "ok (Windows $env:PROCESSOR_ARCHITECTURE, kluris $Version from GitHub through $Site)"
exit 0
