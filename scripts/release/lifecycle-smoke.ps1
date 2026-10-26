# The installed life cycle on Windows, with dist's real output
# (lifecycle-smoke.sh is the macOS/Linux one): install an old release with its
# kluris-installer.ps1, `kluris self-update` to a new one through a fake
# GitHub release API, then `kluris self-uninstall`:
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release\lifecycle-smoke.ps1 `
#       -Releases <dir> -Old 3.0.0 -New 3.0.1 [-Python <python.exe>] [-Record <file>]
#
# <Releases> holds v<Old>\ and v<New>\ (build-versions.ps1), served by
# serve.py with its release API (KLURIS_GITHUB_API_URL points there).
#
# kluris.exe, the receipt and everything kluris writes go to a sandbox
# profile (HOME, USERPROFILE, LOCALAPPDATA, PowerShell's $HOME). The user PATH
# is the one thing outside it that changes: user-path.ps1 saves its exact
# bytes and registry type into -Record first, and they are ALWAYS written back
# at the end, read again and compared byte for byte (a record left by an ended
# smoke makes the next one refuse to start; `cargo xtask windows` writes it
# back after the step and in its stop). The system PATH is only read.
#
# Checks, each command started as a new terminal would start it (PATH built
# from the registry, `cmd /c kluris ...`):
# 1. the old release installs into .kluris\bin; `kluris --version` names it;
# 2. a brain, a companion and the agent skills are made, to be kept;
# 3. `kluris self-update --check --json` reports the new version available;
# 4. `kluris self-update --json` replaces kluris.exe with the new release's
#    (its bytes are the zip's), rewrites only the receipt's `version`, and
#    carries the new binary's `doctor` report; `kluris --version` names it;
# 5. `kluris self-uninstall --yes --json` removes kluris.exe, the receipt and
#    the user PATH entry, and nothing else: every other file is
#    byte-identical to before it; the user PATH is the installed one without
#    exactly that entry. Whether that is the text from before the install is
#    reported: dist's Add-Path writes the value back without its empty
#    entries (`-split ';' -ne ''`) and as REG_EXPAND_SZ, whatever it was.
# 6. KLURIS_NO_MODIFY_PATH=1 (T-111 finding 4): the receipt says
#    modify_path false, the installer leaves the user PATH alone, and
#    self-uninstall removes kluris.exe and the receipt and leaves the user
#    PATH exactly as it was (its text and its type).
# 7. A read-only receipt (the read-only attribute, T-119): the pre-flight
#    blocks the uninstall, exit 1, with every file under the sandbox and the
#    user PATH as they were; once the attribute is cleared a rerun removes
#    kluris.exe, the receipt and the user PATH entry.
#
# Exits 0 when every check passes and the PATH comparison holds, 1 otherwise.
param(
    [Parameter(Mandatory = $true)][string]$Releases,
    [Parameter(Mandatory = $true)][string]$Old,
    [Parameter(Mandatory = $true)][string]$New,
    [string]$Python = 'python',
    [string]$Record = ''
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Step([string]$text) { Write-Output "lifecycle-smoke: $text" }

$Releases = (Resolve-Path -LiteralPath $Releases).Path
foreach ($v in @($Old, $New)) {
    if (-not (Test-Path -LiteralPath (Join-Path $Releases "v$v\kluris-installer.ps1"))) { throw "no v$v\kluris-installer.ps1 in $Releases" }
}
if (-not $Record) { $Record = Join-Path ([IO.Path]::GetTempPath()) 'kluris-lifecycle-smoke.user-path' }
$powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$cmd = Join-Path $env:SystemRoot 'System32\cmd.exe'
$userPath = Join-Path $PSScriptRoot 'user-path.ps1'

function Read-Environment([Microsoft.Win32.RegistryKey]$hive, [string]$sub) {
    $key = $hive.OpenSubKey($sub)
    try {
        if ($null -eq $key.GetValue('Path')) { return @{ Kind = 'absent'; Text = '' } }
        return @{ Kind = $key.GetValueKind('Path').ToString(); Text = $key.GetValue('Path', '', 'DoNotExpandEnvironmentNames') }
    } finally { $key.Close() }
}
$machineKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
$machineBefore = Read-Environment ([Microsoft.Win32.Registry]::LocalMachine) $machineKey
$userBefore = Read-Environment ([Microsoft.Win32.Registry]::CurrentUser) 'Environment'

$saved = & $powershell -NoProfile -ExecutionPolicy Bypass -File $userPath -Save $Record
$saveCode = $LASTEXITCODE
$saved | ForEach-Object { Write-Output $_ }
if ($saveCode -ne 0) {
    Write-Output 'lifecycle-smoke: FAIL: the user PATH could not be saved, so nothing was installed'
    exit 1
}

$work = Join-Path ([IO.Path]::GetTempPath()) ("kluris-lifecycle-smoke-" + [guid]::NewGuid().ToString('N').Substring(0, 12))
$sandbox = Join-Path $work 'home'
$localAppData = Join-Path $sandbox 'AppData\Local'
$served = Join-Path $work 'served'
$server = $null
$failure = $null
$restored = $false

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
    KLURIS_GITHUB_TOKEN = $null; KLURIS_CONFIG = $null; KLURIS_NO_PROMPT = '1'; NO_COLOR = '1'
}
function Invoke-Installer([string]$command, [hashtable]$extra) {
    $quoted = $sandbox.Replace("'", "''")
    $script = "`$ErrorActionPreference = 'Stop'; Set-Variable -Name HOME -Value '$quoted' -Force -Scope Global; if (`$HOME -ne '$quoted') { throw 'HOME is not the sandbox' }; `$ErrorActionPreference = 'Continue'; $command"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script))
    $environment = $sandboxEnv.Clone()
    foreach ($name in $extra.Keys) { $environment[$name] = $extra[$name] }
    return Invoke-Child $powershell "-NoProfile -NonInteractive -ExecutionPolicy ByPass -EncodedCommand $encoded" $environment $work
}
# `kluris <arguments>` as a new terminal starts it: PATH from the registry,
# the command found through it by cmd.
function Invoke-Kluris([string]$arguments) {
    $environment = $sandboxEnv.Clone()
    $environment['Path'] = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
    $environment['KLURIS_GITHUB_API_URL'] = $script:api
    return Invoke-Child $cmd "/d /c kluris $arguments" $environment $work
}
function Last-Json($run, [string]$what) {
    $last = @($run.Out -split "`r?`n" | Where-Object { $_.Trim() })[-1]
    try { return $last | ConvertFrom-Json } catch { throw "$what printed no JSON (exit $($run.Code)): stdout [$($run.Out)] stderr [$($run.Err)]" }
}
# Every file under the sandbox with its SHA-256, but Windows PowerShell's own
# (it keeps a startup cache in LOCALAPPDATA\Microsoft\Windows\PowerShell when
# the installer's powershell.exe runs; that is neither dist's nor kluris's).
function Inventory() {
    $list = @{}
    $own = 'AppData\Local\Microsoft\Windows\PowerShell\'
    Get-ChildItem -LiteralPath $sandbox -Recurse -Force -File | ForEach-Object {
        $relative = $_.FullName.Substring($sandbox.Length + 1)
        if (-not $relative.StartsWith($own, [StringComparison]::OrdinalIgnoreCase)) {
            $list[$relative] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
        }
    }
    return $list
}
# Wait (at most 10 s) until `test` holds: Windows deletes a running exe
# from a helper process after the exe exits.
function Wait-Until([scriptblock]$test) {
    for ($i = 0; $i -lt 50 -and -not (& $test); $i++) { Start-Sleep -Milliseconds 200 }
    return (& $test)
}

try {
    $null = New-Item -ItemType Directory -Force -Path $sandbox, $localAppData, $served
    $start = Inventory
    foreach ($v in @($Old, $New)) { Copy-Item -LiteralPath (Join-Path $Releases "v$v") -Destination $served -Recurse }
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
    $script:api = "http://127.0.0.1:$(([IO.File]::ReadAllText($portFile)).Trim())"
    Step "releases v$Old and v$New and their release API served at $api"

    # 1. The old release, with its own installer.
    $download = "$api/ngvoicu/kluris/releases/download/v$Old"
    $install = Invoke-Installer "irm '$download/kluris-installer.ps1' | iex" @{ KLURIS_DOWNLOAD_URL = $download }
    if ($install.Code -ne 0) { throw "kluris-installer.ps1 of v$Old exited with $($install.Code): $($install.Out) $($install.Err)" }
    $bin = Join-Path $sandbox '.kluris\bin'
    $exe = Join-Path $bin 'kluris.exe'
    $receipt = Join-Path $localAppData 'kluris\kluris-receipt.json'
    if (-not (Test-Path -LiteralPath $exe)) { throw "no $exe" }
    $ver = Invoke-Kluris '--version'
    if ($ver.Out.Trim() -ne "kluris $Old") { throw "after the install kluris --version printed '$($ver.Out.Trim())' $($ver.Err)" }
    Step "installed v${Old}: $($ver.Out.Trim())"
    $receiptBytes = [IO.File]::ReadAllBytes($receipt)
    $ending = if ($receiptBytes.Length -ge 2 -and $receiptBytes[-2] -eq 13 -and $receiptBytes[-1] -eq 10) { 'CRLF' } elseif ($receiptBytes[-1] -eq 10) { 'LF' } else { 'no line end' }
    $bom = if ($receiptBytes.Length -ge 3 -and $receiptBytes[0] -eq 0xEF -and $receiptBytes[1] -eq 0xBB) { 'a BOM' } else { 'no BOM' }
    Step "the receipt as dist writes it ($ending, $bom): $([IO.File]::ReadAllText($receipt).Trim())"
    $userNow = Read-Environment ([Microsoft.Win32.Registry]::CurrentUser) 'Environment'
    Step "the user PATH as dist writes it ($($userNow.Kind)), first entry: $(($userNow.Text -split ';')[0])"
    $installed = Inventory
    $userInstalled = $userNow

    # 2. What the uninstall must keep.
    $null = New-Item -ItemType Directory -Force -Path (Join-Path $sandbox 'brains')
    $create = Invoke-Kluris "create lc-brain --type personal --path `"$(Join-Path $sandbox 'brains')`" --description `"Life-cycle smoke`" --json"
    if ($create.Code -ne 0) { throw "kluris create failed: $($create.Out) $($create.Err)" }
    $companion = Invoke-Kluris 'companion add specmint-core --brain lc-brain --json'
    if ($companion.Code -ne 0) { throw "kluris companion add failed: $($companion.Out) $($companion.Err)" }
    foreach ($kept in @('.kluris\config.yml', '.agents\skills\kluris-lc-brain\SKILL.md', '.kluris\companions\specmint-core\SKILL.md', 'brains\lc-brain\brain.md')) {
        if (-not (Test-Path -LiteralPath (Join-Path $sandbox $kept))) { throw "no $kept after kluris create and companion add" }
    }
    Step 'made a brain, a companion and the agent skills'

    # 3. The check.
    $check = Invoke-Kluris 'self-update --check --json'
    $report = Last-Json $check 'self-update --check --json'
    if ($check.Code -ne 0 -or -not $report.ok -or $report.current -ne $Old -or $report.latest -ne $New -or -not $report.update_available) { throw "self-update --check --json: $($check.Out) $($check.Err)" }
    Step "self-update --check --json: $(($check.Out).Trim())"

    # 4. The update.
    $receiptBefore = [IO.File]::ReadAllText($receipt)
    $update = Invoke-Kluris 'self-update --json'
    $report = Last-Json $update 'self-update --json'
    if ($update.Code -ne 0 -or -not $report.ok) { throw "self-update --json exited $($update.Code): $($update.Out) $($update.Err)" }
    Step "self-update --json: $((($update.Out).Trim()).Substring(0, [Math]::Min(200, ($update.Out).Trim().Length)))"
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zipPath = @(Get-ChildItem -LiteralPath (Join-Path $Releases "v$New") -Filter 'kluris-*.zip')[0].FullName
    $zip = [IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
        Step "the v$New zip as dist builds it: $(($zip.Entries | ForEach-Object { $_.FullName }) -join ', ')"
        $entry = @($zip.Entries | Where-Object { $_.Name -eq 'kluris.exe' })[0]
        $stream = $entry.Open(); $memory = New-Object IO.MemoryStream; $stream.CopyTo($memory); $stream.Close()
        $want = $memory.ToArray()
    } finally { $zip.Dispose() }
    $null = Wait-Until { @(Get-ChildItem -LiteralPath $bin -Force).Count -eq 1 }
    Step ".kluris\bin after the update: $((Get-ChildItem -LiteralPath $bin -Force | ForEach-Object { $_.Name }) -join ', ')"
    $got = [IO.File]::ReadAllBytes($exe)
    if ($got.Length -ne $want.Length -or [Convert]::ToBase64String($got) -ne [Convert]::ToBase64String($want)) { throw "kluris.exe is not v$New's" }
    $expected = $receiptBefore.Replace("`"version`":`"$Old`"", "`"version`":`"$New`"")
    if ([IO.File]::ReadAllText($receipt) -cne $expected) { throw "the receipt changed other than its version: $([IO.File]::ReadAllText($receipt))" }
    $ver = Invoke-Kluris '--version'
    if ($ver.Out.Trim() -ne "kluris $New") { throw "after self-update kluris --version printed '$($ver.Out.Trim())'" }
    Step "updated: $($ver.Out.Trim()); kluris.exe is v$New's zip's; the receipt's version alone changed"

    # 5. The uninstall.
    $before = Inventory
    $uninstall = Invoke-Kluris 'self-uninstall --yes --json'
    $report = Last-Json $uninstall 'self-uninstall --yes --json'
    if ($uninstall.Code -ne 0 -or -not $report.ok) { throw "self-uninstall --yes --json exited $($uninstall.Code): $($uninstall.Out) $($uninstall.Err)" }
    Step "self-uninstall --yes --json: $((($uninstall.Out).Trim()).Substring(0, [Math]::Min(300, ($uninstall.Out).Trim().Length)))"
    $null = Wait-Until { -not (Test-Path -LiteralPath $exe) }
    $after = Inventory
    $ours = @($installed.Keys | Where-Object { -not $start.ContainsKey($_) })
    $problems = @()
    foreach ($path in @('.kluris\bin\kluris.exe', 'AppData\Local\kluris\kluris-receipt.json')) { if ($ours -notcontains $path) { $problems += "the installer did not write $path" } }
    foreach ($path in $ours) { if ($after.ContainsKey($path)) { $problems += "still there: $path" } }
    foreach ($path in $before.Keys) {
        if ($ours -contains $path) { continue }
        if (-not $after.ContainsKey($path)) { $problems += "removed but not the installer's: $path" }
        elseif ($after[$path] -ne $before[$path]) { $problems += "changed: $path" }
    }
    foreach ($path in $after.Keys) { if (-not $before.ContainsKey($path)) { $problems += "new after the uninstall: $path" } }
    $userAfter = Read-Environment ([Microsoft.Win32.Registry]::CurrentUser) 'Environment'
    $installedEntries = @($userInstalled.Text -split ';')
    $owed = ($installedEntries | Select-Object -Skip 1) -join ';'
    if ($installedEntries[0] -ne $bin) { $problems += "the installed user PATH did not start with $bin" }
    if ($userAfter.Text -cne $owed) { $problems += 'the user PATH after the uninstall is not the installed one without its first entry' }
    $machineNow = Read-Environment ([Microsoft.Win32.Registry]::LocalMachine) $machineKey
    if ($machineNow.Kind -ne $machineBefore.Kind -or $machineNow.Text -cne $machineBefore.Text) { $problems += 'the system PATH changed' }
    if ($problems.Count) { throw ($problems -join '; ') }
    Step "removed exactly $($ours -join ', ') and the user PATH entry; kept $($after.Count) files byte-identical (config, companion, skills, brain)"
    Step "the user PATH is the installed one without exactly $bin; its type went $($userBefore.Kind) -> $($userAfter.Kind) (dist's installer writes REG_EXPAND_SZ)"
    $empties = @($userBefore.Text -split ';' | Where-Object { $_ -eq '' }).Count
    if ($userAfter.Text -ceq $userBefore.Text) {
        Step 'the user PATH text is the one from before the install'
    } elseif ((@($userBefore.Text -split ';' | Where-Object { $_ -ne '' }) -join ';') -ceq $userAfter.Text) {
        Step "the user PATH text differs from before the install by its $empties empty entries alone (trailing ';' included), which dist's installer dropped"
    } else {
        $problems += 'the user PATH after the uninstall differs from before the install by more than the empty entries dist drops'
        throw ($problems -join '; ')
    }
    foreach ($d in @('.kluris\bin', 'AppData\Local\kluris')) { if (Test-Path -LiteralPath (Join-Path $sandbox $d)) { Step "left folder: $d" } }

    # 6. No PATH setup by the installer, and none undone by the uninstall.
    $sandbox = Join-Path $work 'home-no-modify-path'
    $localAppData = Join-Path $sandbox 'AppData\Local'
    $null = New-Item -ItemType Directory -Force -Path $sandbox, $localAppData
    $sandboxEnv['HOME'] = $sandbox; $sandboxEnv['USERPROFILE'] = $sandbox; $sandboxEnv['LOCALAPPDATA'] = $localAppData
    $userStart = Read-Environment ([Microsoft.Win32.Registry]::CurrentUser) 'Environment'
    $install = Invoke-Installer "irm '$download/kluris-installer.ps1' | iex" @{ KLURIS_DOWNLOAD_URL = $download; KLURIS_NO_MODIFY_PATH = '1' }
    if ($install.Code -ne 0) { throw "the KLURIS_NO_MODIFY_PATH=1 install exited with $($install.Code): $($install.Out) $($install.Err)" }
    $exe2 = Join-Path $sandbox '.kluris\bin\kluris.exe'
    $receipt2 = Join-Path $localAppData 'kluris\kluris-receipt.json'
    if (-not (Test-Path -LiteralPath $exe2)) { throw "no $exe2 after the KLURIS_NO_MODIFY_PATH=1 install" }
    if ([IO.File]::ReadAllText($receipt2) -notmatch '"modify_path":false') { throw 'the KLURIS_NO_MODIFY_PATH=1 receipt does not say modify_path false' }
    $userInstalled2 = Read-Environment ([Microsoft.Win32.Registry]::CurrentUser) 'Environment'
    if ($userInstalled2.Text -cne $userStart.Text -or $userInstalled2.Kind -ne $userStart.Kind) { throw 'the KLURIS_NO_MODIFY_PATH=1 install changed the user PATH' }
    $environment = $sandboxEnv.Clone()
    $uninstall2 = Invoke-Child $exe2 'self-uninstall --yes --json' $environment $work
    if ($uninstall2.Code -ne 0) { throw "self-uninstall after a KLURIS_NO_MODIFY_PATH=1 install exited $($uninstall2.Code): $($uninstall2.Out) $($uninstall2.Err)" }
    $null = Wait-Until { -not (Test-Path -LiteralPath $exe2) }
    if ((Test-Path -LiteralPath $exe2) -or (Test-Path -LiteralPath $receipt2)) { throw 'self-uninstall left kluris.exe or the receipt of a KLURIS_NO_MODIFY_PATH=1 install' }
    $userAfter2 = Read-Environment ([Microsoft.Win32.Registry]::CurrentUser) 'Environment'
    if ($userAfter2.Text -cne $userStart.Text -or $userAfter2.Kind -ne $userStart.Kind) { throw 'self-uninstall changed the user PATH of a KLURIS_NO_MODIFY_PATH=1 install' }
    Step "KLURIS_NO_MODIFY_PATH=1: modify_path false; the user PATH untouched by the install and the uninstall ($($userStart.Kind)); kluris.exe and the receipt removed"

    # 7. A read-only receipt: blocked with nothing changed, then a rerun.
    $sandbox = Join-Path $work 'home-read-only-receipt'
    $localAppData = Join-Path $sandbox 'AppData\Local'
    $null = New-Item -ItemType Directory -Force -Path $sandbox, $localAppData
    $sandboxEnv['HOME'] = $sandbox; $sandboxEnv['USERPROFILE'] = $sandbox; $sandboxEnv['LOCALAPPDATA'] = $localAppData
    $install = Invoke-Installer "irm '$download/kluris-installer.ps1' | iex" @{ KLURIS_DOWNLOAD_URL = $download }
    if ($install.Code -ne 0) { throw "the install for the read-only receipt case exited with $($install.Code): $($install.Out) $($install.Err)" }
    $exe3 = Join-Path $sandbox '.kluris\bin\kluris.exe'
    $bin3 = Join-Path $sandbox '.kluris\bin'
    $receipt3 = Join-Path $localAppData 'kluris\kluris-receipt.json'
    $userInstalled3 = Read-Environment ([Microsoft.Win32.Registry]::CurrentUser) 'Environment'
    if (@($userInstalled3.Text -split ';') -notcontains $bin3) { throw "the install for the read-only receipt case did not add $bin3 to the user PATH" }
    $before3 = Inventory
    Set-ItemProperty -LiteralPath $receipt3 -Name IsReadOnly -Value $true
    try {
        $blocked3 = Invoke-Child $exe3 'self-uninstall --yes --json' $sandboxEnv.Clone() $work
    } finally {
        Set-ItemProperty -LiteralPath $receipt3 -Name IsReadOnly -Value $false
    }
    if ($blocked3.Code -ne 1) { throw "self-uninstall with a read-only receipt exited $($blocked3.Code), not 1: $($blocked3.Out) $($blocked3.Err)" }
    $json3 = Last-Json $blocked3 'self-uninstall with a read-only receipt'
    if ($json3.ok -or @($json3.removed).Count -ne 0 -or @($json3.blocked).Count -eq 0) { throw "self-uninstall with a read-only receipt was not blocked: $($blocked3.Out)" }
    $after3 = Inventory
    $changed3 = @($before3.Keys + $after3.Keys | Sort-Object -Unique | Where-Object { $before3[$_] -ne $after3[$_] })
    if ($changed3.Count -ne 0) { throw "the blocked self-uninstall changed $($changed3 -join ', ')" }
    $userBlocked3 = Read-Environment ([Microsoft.Win32.Registry]::CurrentUser) 'Environment'
    if ($userBlocked3.Text -cne $userInstalled3.Text -or $userBlocked3.Kind -ne $userInstalled3.Kind) { throw 'the blocked self-uninstall changed the user PATH' }
    Step "a read-only receipt: exit 1, nothing changed (every file and the user PATH): $((@($json3.blocked) | ForEach-Object { "$($_.path): $($_.problem); fix: $($_.fix)" }) -join ' | ')"
    $rerun3 = Invoke-Child $exe3 'self-uninstall --yes --json' $sandboxEnv.Clone() $work
    if ($rerun3.Code -ne 0) { throw "the rerun after clearing the read-only attribute exited $($rerun3.Code): $($rerun3.Out) $($rerun3.Err)" }
    $null = Wait-Until { -not (Test-Path -LiteralPath $exe3) }
    if ((Test-Path -LiteralPath $exe3) -or (Test-Path -LiteralPath $receipt3)) { throw 'the rerun left kluris.exe or the receipt' }
    $userAfter3 = Read-Environment ([Microsoft.Win32.Registry]::CurrentUser) 'Environment'
    if (@($userAfter3.Text -split ';') -contains $bin3) { throw "the rerun left $bin3 in the user PATH" }
    Step 'the rerun finished: kluris.exe, the receipt and the user PATH entry gone'
} catch {
    $failure = $_
} finally {
    $back = & $powershell -NoProfile -ExecutionPolicy Bypass -File $userPath -Restore $Record
    $restored = ($LASTEXITCODE -eq 0)
    $back | ForEach-Object { Write-Output "lifecycle-smoke: $_" }
    if ($server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 500
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
if ($failure) {
    Write-Output "lifecycle-smoke: FAIL: $($failure.Exception.Message)"
    exit 1
}
if (-not $restored) {
    Write-Output 'lifecycle-smoke: FAIL: the user PATH was not written back identically'
    exit 1
}
Step "ok (Windows $env:PROCESSOR_ARCHITECTURE, v$Old -> v$New -> uninstalled; no-modify-path, read-only-receipt)"
exit 0
