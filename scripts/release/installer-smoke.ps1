# Smoke-test the release installer on Windows, against a local release folder
# served the way GitHub Releases serves it (scripts/release/serve.py):
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release\installer-smoke.ps1 `
#       -Release target\distrib [-Wrapper <site install.ps1>] [-Python <python.exe>]
#
# The release folder is this machine's zip, kluris-installer.ps1 and
# sha256.sum (what dist builds for x86_64-pc-windows-msvc or
# aarch64-pc-windows-msvc).
#
# The installer runs as the current user, with HOME, USERPROFILE and
# LOCALAPPDATA pointed at a sandbox (and PowerShell's own $HOME set to it), so
# kluris.exe and the receipt land there. The one thing it changes outside the
# sandbox is the user PATH (HKCU\Environment\Path), which is what is tested:
# its exact bytes and registry type are saved first and ALWAYS written back
# at the end (or the value removed, when there was none), then read again and
# compared byte for byte. The system PATH (HKLM) is only read, before and
# after, and must not change.
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
    [string]$Python = 'python'
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class KlurisRegistry {
  static readonly UIntPtr CurrentUser = new UIntPtr(0x80000001u);
  static readonly UIntPtr LocalMachine = new UIntPtr(0x80000002u);
  [DllImport("advapi32.dll", CharSet = CharSet.Unicode)] static extern int RegOpenKeyExW(UIntPtr key, string sub, uint options, int sam, out UIntPtr result);
  [DllImport("advapi32.dll", CharSet = CharSet.Unicode)] static extern int RegQueryValueExW(UIntPtr key, string name, IntPtr reserved, out uint type, byte[] data, ref uint size);
  [DllImport("advapi32.dll", CharSet = CharSet.Unicode)] static extern int RegSetValueExW(UIntPtr key, string name, int reserved, uint type, byte[] data, uint size);
  [DllImport("advapi32.dll", CharSet = CharSet.Unicode)] static extern int RegDeleteValueW(UIntPtr key, string name);
  [DllImport("advapi32.dll")] static extern int RegCloseKey(UIntPtr key);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr window, uint message, UIntPtr wParam, string lParam, uint flags, uint timeout, out UIntPtr result);
  const int KeyRead = 0x20019, KeySetValue = 0x0002;
  static UIntPtr Open(bool machine, string sub, int sam) {
    UIntPtr key;
    int status = RegOpenKeyExW(machine ? LocalMachine : CurrentUser, sub, 0, sam, out key);
    if (status != 0) throw new Exception("RegOpenKeyEx " + sub + ": " + status);
    return key;
  }
  // The value's raw bytes and type, or null when there is no such value.
  public static byte[] Read(bool machine, string sub, string name, out uint type) {
    UIntPtr key = Open(machine, sub, KeyRead);
    try {
      uint size = 0;
      int status = RegQueryValueExW(key, name, IntPtr.Zero, out type, null, ref size);
      if (status == 2) { type = 0; return null; }
      if (status != 0 && status != 234) throw new Exception("RegQueryValueEx " + name + ": " + status);
      byte[] data = new byte[size];
      status = RegQueryValueExW(key, name, IntPtr.Zero, out type, data, ref size);
      if (status != 0) throw new Exception("RegQueryValueEx " + name + ": " + status);
      if (size != data.Length) Array.Resize(ref data, (int)size);
      return data;
    } finally { RegCloseKey(key); }
  }
  // Writes the raw bytes back with their type, or removes the value (data null).
  public static void Write(string sub, string name, uint type, byte[] data) {
    UIntPtr key = Open(false, sub, KeySetValue);
    try {
      int status = data == null ? RegDeleteValueW(key, name) : RegSetValueExW(key, name, 0, type, data, (uint)data.Length);
      if (status != 0 && !(data == null && status == 2)) throw new Exception("writing " + name + ": " + status);
    } finally { RegCloseKey(key); }
  }
  // WM_SETTINGCHANGE "Environment", so Explorer rereads the environment.
  public static void Broadcast() {
    UIntPtr result;
    SendMessageTimeoutW(new IntPtr(0xffff), 0x001A, UIntPtr.Zero, "Environment", 0x0002, 5000, out result);
  }
}
'@

$typeNames = @{ 0 = 'none'; 1 = 'REG_SZ'; 2 = 'REG_EXPAND_SZ'; 7 = 'REG_MULTI_SZ' }
function Show-Type([uint32]$type) { if ($typeNames.ContainsKey([int]$type)) { $typeNames[[int]$type] } else { "type $type" } }
function Same-Bytes($a, $b) {
    if ($null -eq $a -or $null -eq $b) { return ($null -eq $a) -and ($null -eq $b) }
    if ($a.Length -ne $b.Length) { return $false }
    for ($i = 0; $i -lt $a.Length; $i++) { if ($a[$i] -ne $b[$i]) { return $false } }
    return $true
}
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

$userKey = 'Environment'
$machineKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
[uint32]$savedType = 0
$savedPath = [KlurisRegistry]::Read($false, $userKey, 'Path', [ref]$savedType)
[uint32]$machineType = 0
$machineBefore = [KlurisRegistry]::Read($true, $machineKey, 'Path', [ref]$machineType)
$savedLength = if ($null -eq $savedPath) { 'absent' } else { "$($savedPath.Length) bytes" }
Step "user PATH saved: $savedLength, $(Show-Type $savedType)"

$work = Join-Path ([IO.Path]::GetTempPath()) ("kluris-installer-smoke-" + [guid]::NewGuid().ToString('N').Substring(0, 12))
$sandbox = Join-Path $work 'home'
$localAppData = Join-Path $sandbox 'AppData\Local'
$served = Join-Path $work 'served'
$null = New-Item -ItemType Directory -Force -Path $sandbox, $localAppData, $served
$realBin = Join-Path $env:USERPROFILE '.kluris\bin\kluris.exe'
$realReceipt = Join-Path $env:LOCALAPPDATA 'kluris\kluris-receipt.json'
$realBefore = @($realBin, $realReceipt | ForEach-Object { if (Test-Path -LiteralPath $_) { (Get-FileHash -LiteralPath $_).Hash } else { 'absent' } })
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

$powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$sandboxEnv = @{
    HOME = $sandbox; USERPROFILE = $sandbox; LOCALAPPDATA = $localAppData
    XDG_CONFIG_HOME = $null; GITHUB_PATH = $null; KLURIS_INSTALL_DIR = $null; KLURIS_NO_MODIFY_PATH = $null
}
# `powershell <policy> -Command <command>` as the sandbox user, PowerShell's
# own $HOME set to the sandbox first.
function Invoke-Installer([string]$policy, [string]$command, [hashtable]$extra) {
    $quoted = $sandbox.Replace("'", "''")
    $script = "`$ErrorActionPreference = 'Stop'; Set-Variable -Name HOME -Value '$quoted' -Force -Scope Global; if (`$HOME -ne '$quoted') { throw 'HOME is not the sandbox' }; `$ErrorActionPreference = 'Continue'; $command"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script))
    $environment = $sandboxEnv.Clone()
    foreach ($name in $extra.Keys) { $environment[$name] = $extra[$name] }
    return Invoke-Child $powershell "-NoProfile -NonInteractive -ExecutionPolicy $policy -EncodedCommand $encoded" $environment $work
}

try {
    Get-ChildItem -LiteralPath $Release -File | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $served }
    if ($Wrapper) { Copy-Item -LiteralPath $Wrapper -Destination (Join-Path $served 'install.ps1') }
    $portFile = Join-Path $work 'port'
    $log = Join-Path $work 'requests.log'
    $serve = Join-Path $PSScriptRoot 'serve.py'
    $server = Start-Process -FilePath $Python -ArgumentList @('-I', "`"$serve`"", "`"$served`"", "`"$portFile`"", "`"$log`"") `
        -WorkingDirectory $work -NoNewWindow -PassThru
    for ($i = 0; -not (Test-Path -LiteralPath $portFile); $i++) {
        if ($i -ge 100 -or $server.HasExited) { throw 'the local server did not start' }
        Start-Sleep -Milliseconds 100
    }
    $port = ([IO.File]::ReadAllText($portFile)).Trim()
    $releases = "http://127.0.0.1:$port/ngvoicu/kluris/releases"
    $download = "$releases/download/v$version"
    $site = "http://127.0.0.1:$port/install.ps1"
    Step "release $version served at $releases"
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
    if ($install.Err) { Write-Output $install.Err }
    if ($install.Code -ne 0) { throw "the installer exited with $($install.Code)" }
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

    [uint32]$nowType = 0
    $nowBytes = [KlurisRegistry]::Read($false, $userKey, 'Path', [ref]$nowType)
    if ($nowType -ne 2) { throw "the user PATH is $(Show-Type $nowType), not REG_EXPAND_SZ" }
    $nowPath = [Text.Encoding]::Unicode.GetString($nowBytes).TrimEnd([char]0)
    $firstEntry = ($nowPath -split ';')[0]
    if ($firstEntry -ne $bin) { throw "the user PATH starts with '$firstEntry', not $bin" }
    [uint32]$type = 0
    $machineNow = [KlurisRegistry]::Read($true, $machineKey, 'Path', [ref]$type)
    if (-not (Same-Bytes $machineBefore $machineNow)) { throw 'the system PATH changed' }
    Step "user PATH starts with $bin (REG_EXPAND_SZ); system PATH unchanged"
    $after = @($realBin, $realReceipt | ForEach-Object { if (Test-Path -LiteralPath $_) { (Get-FileHash -LiteralPath $_).Hash } else { 'absent' } })
    if (($after -join ' ') -ne ($realBefore -join ' ')) { throw "the real profile's kluris.exe or receipt changed: $($realBefore -join ' ') -> $($after -join ' ')" }

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
    $report = ($doctor.Out -split "`r?`n" | Where-Object { $_.Trim() })[-1] | ConvertFrom-Json
    $pathCheck = $report.checks | Where-Object { $_.name -eq 'path' }
    if (-not $report.ok -or -not $pathCheck.passed -or $pathCheck.warning -or $pathCheck.detail -ne $exe) { throw "kluris doctor --json is not ok with this binary first on PATH: $($doctor.Out)" }
    Step "kluris doctor --json: ok; $(($report.checks | ForEach-Object { "$($_.name) $(if ($_.passed) { 'passed' } else { 'FAILED' })" }) -join ', ')"
} catch {
    $failure = $_
} finally {
    try {
        [KlurisRegistry]::Write($userKey, 'Path', $savedType, $savedPath)
        [KlurisRegistry]::Broadcast()
        [uint32]$backType = 0
        $back = [KlurisRegistry]::Read($false, $userKey, 'Path', [ref]$backType)
        $restored = (Same-Bytes $savedPath $back) -and ($backType -eq $savedType)
        $backLength = if ($null -eq $back) { 'absent' } else { "$($back.Length) bytes" }
        if ($restored) {
            Step "user PATH restored: $backLength, $(Show-Type $backType), byte for byte identical to the saved value"
        } else {
            Write-Output "installer-smoke: FAIL: the user PATH read back is $backLength, $(Show-Type $backType): NOT identical to the saved $savedLength, $(Show-Type $savedType)"
        }
    } catch {
        Write-Output "installer-smoke: FAIL: restoring the user PATH: $($_.Exception.Message)"
    }
    if ($server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 300
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
if ($failure) {
    Write-Output "installer-smoke: FAIL: $($failure.Exception.Message)"
    exit 1
}
if (-not $restored) { exit 1 }
Step "ok (Windows $env:PROCESSOR_ARCHITECTURE, kluris $version$(if ($Wrapper) { ', through the site wrapper' }))"
exit 0
