# Smoke-test the release installer on Windows, against a local release folder
# served the way GitHub Releases serves it (scripts/release/serve.py):
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release\installer-smoke.ps1 `
#       -Release target\distrib [-Wrapper <site install.ps1>] [-Python <python.exe>] [-Record <file>]
#
# The release folder is what scripts\release\build-local.ps1 writes: this
# machine's zip, kluris-installer.ps1 and sha256.sum.
#
# The installer runs as the current user, with HOME, USERPROFILE and
# LOCALAPPDATA pointed at a sandbox (and PowerShell's own $HOME set to it), so
# kluris.exe and the receipt land there; the real profile's are compared
# before and after. The one thing it changes outside the sandbox is the user
# PATH (HKCU\Environment\Path), which is what is tested: user-path.ps1 saves
# its exact bytes and registry type into -Record first, and they are ALWAYS
# written back at the end (or the value removed, when there was none), read
# again and compared byte for byte. A record left by a smoke that was ended
# first makes the next smoke refuse to start; `cargo xtask windows` writes it
# back after the step and in its stop. The system PATH (HKLM) is only read,
# before and after, and must not change.
#
# Checks: kluris.exe in the sandbox's default .kluris\bin (KLURIS_INSTALL_DIR
# unset), the receipt naming it, the user PATH starting with that folder as
# REG_EXPAND_SZ, a fresh process (PATH built from the registry, as a new
# terminal gets it) finding kluris.exe there first, `kluris --version`
# naming the release and `kluris doctor --json` ok with this binary first on
# PATH. The installer runs the documented way:
# `powershell -ExecutionPolicy ByPass -c "irm <url> | iex"`.
#
# With -Wrapper, the site's install.ps1 is the entry point, served beside the
# release and run as a bare `irm <url> | iex` in a PowerShell whose policy is
# Restricted (the wrapper's own process-scope Bypass must make it work), with
# KLURIS_VERSION=<v> and v<v> fetching releases/download/v<v>/, without it
# releases/latest/download/, and a malformed KLURIS_VERSION fetching nothing.
#
# Exits 0 when every check passes and the PATH comparison holds, 1 otherwise.
param(
    [Parameter(Mandatory = $true)][string]$Release,
    [string]$Wrapper = '',
    [string]$Python = 'python',
    [string]$Record = ''
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Step([string]$text) { Write-Output "installer-smoke: $text" }

$Release = (Resolve-Path -LiteralPath $Release).Path
$installerPs1 = Join-Path $Release 'kluris-installer.ps1'
if (-not (Test-Path -LiteralPath $installerPs1)) { throw "no kluris-installer.ps1 in $Release" }
$version = $null
foreach ($line in [IO.File]::ReadAllLines($installerPs1)) {
    if ($line -match "^\`$app_version = '([^']+)'$") { $version = $Matches[1]; break }
}
if (-not $version) { throw 'kluris-installer.ps1 names no $app_version' }
if ($Wrapper) { $Wrapper = (Resolve-Path -LiteralPath $Wrapper).Path }
if (-not $Record) { $Record = Join-Path ([IO.Path]::GetTempPath()) 'kluris-installer-smoke.user-path' }

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
$saved | ForEach-Object { Write-Output $_ }
if ($saveCode -ne 0) {
    Write-Output 'installer-smoke: FAIL: the user PATH could not be saved, so nothing was installed'
    exit 1
}

$work = Join-Path ([IO.Path]::GetTempPath()) ("kluris-installer-smoke-" + [guid]::NewGuid().ToString('N').Substring(0, 12))
$sandbox = Join-Path $work 'home'
$localAppData = Join-Path $sandbox 'AppData\Local'
$served = Join-Path $work 'served'
$server = $null
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
}
# `powershell -ExecutionPolicy <policy> -Command <command>` with the sandbox's
# environment, PowerShell's own $HOME set to the sandbox first.
function Invoke-Installer([string]$policy, [string]$command, [hashtable]$extra) {
    $quoted = $sandbox.Replace("'", "''")
    $script = "`$ErrorActionPreference = 'Stop'; Set-Variable -Name HOME -Value '$quoted' -Force -Scope Global; if (`$HOME -ne '$quoted') { throw 'HOME is not the sandbox' }; `$ErrorActionPreference = 'Continue'; $command"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script))
    $environment = $sandboxEnv.Clone()
    foreach ($name in $extra.Keys) { $environment[$name] = $extra[$name] }
    return Invoke-Child $powershell "-NoProfile -NonInteractive -ExecutionPolicy $policy -EncodedCommand $encoded" $environment $work
}

function Hash-Of([string]$path) { if (Test-Path -LiteralPath $path) { (Get-FileHash -LiteralPath $path).Hash } else { 'absent' } }

try {
    $null = New-Item -ItemType Directory -Force -Path $sandbox, $localAppData, $served
    $realBin = Join-Path $env:USERPROFILE '.kluris\bin\kluris.exe'
    $realReceipt = Join-Path $env:LOCALAPPDATA 'kluris\kluris-receipt.json'
    $realBefore = "$(Hash-Of $realBin) $(Hash-Of $realReceipt)"

    Get-ChildItem -LiteralPath $Release -File | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $served }
    if ($Wrapper) { Copy-Item -LiteralPath $Wrapper -Destination (Join-Path $served 'install.ps1') }
    $portFile = Join-Path $work 'port'
    $log = Join-Path $work 'requests.log'
    $serve = Join-Path $PSScriptRoot 'serve.py'
    $server = Start-Process -FilePath $Python -ArgumentList @('-I', "`"$serve`"", "`"$served`"", "`"$portFile`"", "`"$log`"") `
        -WorkingDirectory $work -NoNewWindow -PassThru
    for ($i = 0; -not (Test-Path -LiteralPath $portFile); $i++) {
        if ($server.HasExited) { throw 'the local server exited' }
        if ($i -ge 600) { throw 'the local server did not start in 60 s' }
        Start-Sleep -Milliseconds 100
    }
    $port = ([IO.File]::ReadAllText($portFile)).Trim()
    $releases = "http://127.0.0.1:$port/ngvoicu/kluris/releases"
    $download = "$releases/download/v$version"
    $site = "http://127.0.0.1:$port/install.ps1"
    Step "release $version served at $releases"
    # The requests after the wrapper's own download.
    function Requests() { if (Test-Path -LiteralPath $log) { @([IO.File]::ReadAllLines($log) | Where-Object { $_ -ne '/install.ps1' }) } else { @() } }
    function Clear-Requests() { [IO.File]::WriteAllText($log, '') }

    if ($Wrapper) {
        Clear-Requests
        $bad = Invoke-Installer 'Restricted' "irm '$site' | iex" @{ KLURIS_RELEASES_URL = $releases; KLURIS_VERSION = '3.0.0/../../evil' }
        if ($bad.Code -eq 0) { throw 'the wrapper ran with a malformed KLURIS_VERSION' }
        if ((Requests).Count) { throw "the wrapper fetched $((Requests) -join ', ') for a malformed KLURIS_VERSION" }
        if (Test-Path -LiteralPath (Join-Path $sandbox '.kluris')) { throw 'the wrapper wrote .kluris for a malformed KLURIS_VERSION' }
        Step "wrapper refuses a malformed KLURIS_VERSION (exit $($bad.Code))"
        foreach ($pin in @($version, "v$version")) {
            Clear-Requests
            $pinned = Join-Path $work 'pinned'
            $run = Invoke-Installer 'Restricted' "irm '$site' | iex" @{ KLURIS_RELEASES_URL = $releases; KLURIS_VERSION = $pin; KLURIS_DOWNLOAD_URL = $download; KLURIS_NO_MODIFY_PATH = '1'; KLURIS_INSTALL_DIR = $pinned }
            if ($run.Code -ne 0) { throw "the wrapper failed with KLURIS_VERSION=${pin}: $($run.Out) $($run.Err)" }
            if (-not (Test-Path -LiteralPath (Join-Path $pinned 'kluris.exe'))) { throw "KLURIS_VERSION=$pin installed nothing: $($run.Out) $($run.Err)" }
            $first = @(Requests)[0]
            if ($first -ne "/ngvoicu/kluris/releases/download/v$version/kluris-installer.ps1") { throw "KLURIS_VERSION=$pin fetched $first" }
            Step "wrapper with KLURIS_VERSION=$pin fetched $first"
            Remove-Item -LiteralPath $pinned -Recurse -Force
        }
        Remove-Item -LiteralPath (Join-Path $localAppData 'kluris') -Recurse -Force -ErrorAction SilentlyContinue
        Clear-Requests
        $install = Invoke-Installer 'Restricted' "irm '$site' | iex" @{ KLURIS_RELEASES_URL = $releases; KLURIS_DOWNLOAD_URL = $download }
        $first = @(Requests)[0]
        if ($first -ne '/ngvoicu/kluris/releases/latest/download/kluris-installer.ps1') { throw "the wrapper without KLURIS_VERSION fetched $first" }
        Step "wrapper without KLURIS_VERSION, under a Restricted policy, fetched $first"
    } else {
        $install = Invoke-Installer 'ByPass' "irm '$download/kluris-installer.ps1' | iex" @{ KLURIS_DOWNLOAD_URL = $download }
    }
    Write-Output $install.Out
    # Windows PowerShell writes its progress and information records to a
    # redirected stderr as CLIXML: shown only when the install failed.
    if ($install.Code -ne 0) { throw "the installer exited with $($install.Code): $($install.Err)" }
    if ($install.Out -notmatch 'kluris installed\. Open a new terminal, then run: kluris doctor') { throw 'the installer did not print the success message' }

    $bin = Join-Path $sandbox '.kluris\bin'
    $exe = Join-Path $bin 'kluris.exe'
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "no $exe" }
    $receiptFile = Join-Path $localAppData 'kluris\kluris-receipt.json'
    if (-not (Test-Path -LiteralPath $receiptFile)) { throw "no receipt at $receiptFile" }
    $receipt = [IO.File]::ReadAllText($receiptFile) | ConvertFrom-Json
    if ($receipt.install_prefix -ne $bin) { throw "the receipt's install_prefix is $($receipt.install_prefix)" }
    if ($receipt.version -ne $version) { throw "the receipt's version is $($receipt.version)" }
    Step "installed: $exe, receipt $receiptFile"

    $user = Read-Environment ([Microsoft.Win32.Registry]::CurrentUser) 'Environment'
    if ($user.Kind -ne 'ExpandString') { throw "the user PATH is $($user.Kind), not REG_EXPAND_SZ" }
    $firstEntry = ($user.Text -split ';')[0]
    if ($firstEntry -ne $bin) { throw "the user PATH starts with '$firstEntry', not $bin" }
    $machineNow = Read-Environment ([Microsoft.Win32.Registry]::LocalMachine) $machineKey
    if ($machineNow.Kind -ne $machineBefore.Kind -or $machineNow.Text -cne $machineBefore.Text) { throw 'the system PATH changed' }
    Step "user PATH starts with $bin (REG_EXPAND_SZ); system PATH unchanged"
    $realAfter = "$(Hash-Of $realBin) $(Hash-Of $realReceipt)"
    if ($realAfter -ne $realBefore) { throw "the real profile's kluris.exe or receipt changed: $realBefore -> $realAfter" }

    # A new terminal: PATH as Windows builds it, the system's then the user's.
    $fresh = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
    $freshEnv = $sandboxEnv.Clone()
    $freshEnv['Path'] = $fresh
    $where = Invoke-Child (Join-Path $env:SystemRoot 'System32\where.exe') 'kluris' $freshEnv $work
    $found = @($where.Out -split "`r?`n" | Where-Object { $_ })[0]
    if ($found -ne $exe) { throw "a fresh process finds kluris at '$found'" }
    Step "a fresh process finds $found"
    $command = Invoke-Child $powershell '-NoProfile -NonInteractive -Command "(Get-Command kluris).Source"' $freshEnv $work
    if ($command.Out.Trim() -ne $exe) { throw "a fresh PowerShell finds kluris at '$($command.Out.Trim())'" }
    Step "a fresh PowerShell finds $($command.Out.Trim())"
    $ver = Invoke-Child $found '--version' $freshEnv $work
    if ($ver.Out.Trim() -ne "kluris $version") { throw "kluris --version printed '$($ver.Out.Trim())'" }
    Step "kluris --version: $($ver.Out.Trim())"
    $doctor = Invoke-Child $found 'doctor --json' $freshEnv $work
    if ($doctor.Code -ne 0) { throw "kluris doctor --json exited $($doctor.Code): $($doctor.Out) $($doctor.Err)" }
    $last = @($doctor.Out -split "`r?`n" | Where-Object { $_.Trim() })[-1]
    try { $report = $last | ConvertFrom-Json } catch { throw "kluris doctor --json printed no JSON ($($_.Exception.Message)): stdout [$($doctor.Out)] stderr [$($doctor.Err)]" }
    $pathCheck = $report.checks | Where-Object { $_.name -eq 'path' }
    if (-not $report.ok -or -not $pathCheck.passed -or $pathCheck.warning -or $pathCheck.detail -ne $exe) { throw "kluris doctor --json is not ok with this binary first on PATH: $($doctor.Out)" }
    Step "kluris doctor --json: ok; $(($report.checks | ForEach-Object { "$($_.name) $(if ($_.passed) { 'passed' } else { 'FAILED' })" }) -join ', ')"
} catch {
    $failure = $_
} finally {
    $back = & $powershell -NoProfile -ExecutionPolicy Bypass -File $userPath -Restore $Record
    $restored = ($LASTEXITCODE -eq 0)
    $back | ForEach-Object { Write-Output "installer-smoke: $_" }
    if ($server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 300
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
if ($failure) {
    Write-Output "installer-smoke: FAIL: $($failure.Exception.Message)"
    exit 1
}
if (-not $restored) {
    Write-Output 'installer-smoke: FAIL: the user PATH was not written back identically'
    exit 1
}
Step "ok (Windows $env:PROCESSOR_ARCHITECTURE, kluris $version$(if ($Wrapper) { ', through the site wrapper' }))"
exit 0
