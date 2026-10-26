//! The PowerShell a run sends, as text, so the tests can read it. Windows
//! PowerShell 5.1 takes each as `-EncodedCommand` (base64 of UTF-16LE).
//!
//! - The bootstrap ([`bootstrap`]) goes on ssh's command line, which holds
//!   [`ENCODED_LIMIT`] characters of it: it takes the shared lock, unpacks the
//!   run's tarball into the run's folder and dot-sources the run script that
//!   came in it; its `finally` removes the tarball and the lock records it wrote
//!   and closes the lock, however the run ends.
//! - The run script ([`run_script`]) rides in the tarball: the run put in a job
//!   (at most [`PROCESS_LIMIT`] processes, every one ended when the run ends),
//!   old runs swept, then each [`Step`] run with its own bound in minutes.
//! - The stop ([`stop_script`]) ends a run that was stopped on this side, which
//!   Windows' SSH server does not pass on.
//! - The state ([`state_script`]) is what zeewin2 is asked before a run: C:'s
//!   free space and who holds the shared lock, read without opening it.
//!
//! Every script that runs a command ends its output with the line `== exit N`
//! ([`EXIT_MARK`]): ssh to a PowerShell answers 1 for any failure, so the
//! status is read from the line.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

#[cfg(test)]
pub use crate::ssh_ci::outcome::exit_said;
pub use crate::ssh_ci::outcome::{EXIT_MARK, STEP_MARK, TREE_MARK};
use crate::ssh_ci::plan::{
    self, CARGO_TEST, INSTALLER_SMOKE, LIFECYCLE, LIFECYCLE_NEW, LIFECYCLE_OLD, PARITY, PYTHON,
    RELEASE_BUILD, RELEASE_DIST, RELEASE_VERSIONS, SMOKE_PYTHON, TOOLCHAIN, WRAPPER_DIR, fill,
};
pub use crate::ssh_ci::plan::{CI_DIR, TREE_FILE, UV_VERSION, run_id, tarball_name};

/// ConsensFlow's copy folder under the profile, whose lock Kluris shares: one
/// run of either project per machine at a time.
pub const LOCK_DIR: &str = "consensflow-build";
/// The lock, opened by a run for itself alone (`FileShare.None`) and held until it ends.
pub const LOCK: &str = ".windows-run.lock";
/// The holder's process id: what ConsensFlow's scripts before the lock file read.
pub const RUNNING: &str = ".windows-run.pid";
/// The holder's run id and process, two lines.
pub const OWNER: &str = ".windows-run.id";

/// The run script, at the root of the tarball.
pub const RUN_FILE: &str = ".kluris-run.ps1";

/// What cmd.exe reads of a command line (8191), less ssh's `powershell
/// -NoProfile -NonInteractive -EncodedCommand ` and a margin.
pub const ENCODED_LIMIT: usize = 8000;

/// The most processes a run's job holds at once: cargo's jobs, the linker,
/// test binaries, git, Python and the two kluris binaries fit well within it.
pub const PROCESS_LIMIT: u32 = 64;

/// uv's Windows zip ([`UV_VERSION`]), fetched into Kluris's tools folder when
/// the machine has none there.
pub const UV_SHA256: &str = "e07acf3f8a29fe41f9e04b799c3325cb0e0893836bb222bf102829b45c679ad6";
const UV_URL: &str =
    "https://github.com/astral-sh/uv/releases/download/0.12.8/uv-x86_64-pc-windows-msvc.zip";

/// What a run is asked for.
#[derive(Debug, Clone, Copy)]
pub struct Spec<'a> {
    pub id: &'a str,
    /// The tree's `rust-toolchain.toml` channel.
    pub channel: &'a str,
    /// `CARGO_BUILD_JOBS` on this machine.
    pub jobs: u32,
    pub parity: bool,
    /// The `parity/*.py` files `--parity-files` named (already checked); empty:
    /// the whole suite.
    pub parity_files: &'a [String],
    /// Appended to `cargo test --workspace --no-fail-fast`.
    pub cargo_args: &'a [String],
    /// The release installer smoke instead of the tests.
    pub installer_smoke: bool,
    /// The site's install.ps1 is in the tarball, for the smoke to go through.
    pub wrapper: bool,
}

#[cfg(test)]
pub fn steps(spec: &Spec) -> Vec<plan::Step> {
    if spec.installer_smoke {
        plan::smoke_steps()
    } else {
        plan::steps(spec.parity)
    }
}

/// How long the whole run may take, as this side waits for it.
pub fn bound(spec: &Spec) -> Duration {
    if spec.installer_smoke {
        plan::smoke_bound()
    } else {
        plan::bound(spec.parity)
    }
}

/// The line ssh hands the machine's shell for `script`, or why it does not fit.
pub fn powershell_line(script: &str) -> Result<String, String> {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = STANDARD.encode(bytes);
    if encoded.len() > ENCODED_LIMIT {
        return Err(format!(
            "the script is {} characters encoded, and the machine's shell reads {ENCODED_LIMIT}",
            encoded.len()
        ));
    }
    Ok(format!(
        "powershell -NoProfile -NonInteractive -EncodedCommand {encoded}"
    ))
}

/// `args` as one Windows command line, quoted the way the C runtime (and Rust's
/// `std::env::args`) splits it again.
pub fn command_line(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '"']) {
        return arg.to_owned();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                quoted.push(c);
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

/// `text` as PowerShell takes it between single quotes.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

pub fn bootstrap(id: &str) -> String {
    fill(
        BOOTSTRAP,
        &[
            ("lock_dir", LOCK_DIR),
            ("lock", LOCK),
            ("running", RUNNING),
            ("owner", OWNER),
            ("tarball", &tarball_name(id)),
            ("id", id),
            ("ci", CI_DIR),
            ("tree_mark", TREE_MARK),
            ("tree_file", TREE_FILE),
            ("run_file", RUN_FILE),
            ("exit", EXIT_MARK),
        ],
    )
}

pub fn run_script(spec: &Spec) -> String {
    // Every test binary runs, so one failing binary does not hide the others'.
    let mut test = vec![
        "test".to_owned(),
        "--workspace".to_owned(),
        "--no-fail-fast".to_owned(),
    ];
    test.extend(spec.cargo_args.iter().cloned());
    let toolchain = format!(
        "toolchain install {} --profile minimal --component clippy --component rustfmt --no-self-update",
        spec.channel
    );
    let uv = fill(
        UV_TEMPLATE,
        &[
            ("uv_version", UV_VERSION),
            ("uv_url", UV_URL),
            ("uv_sha256", UV_SHA256),
        ],
    );
    let (test_step, after) = if spec.installer_smoke {
        let wrapper = if spec.wrapper {
            format!(" -Wrapper {WRAPPER_DIR}\\install.ps1")
        } else {
            String::new()
        };
        let smoke = fill(
            SMOKE_TEMPLATE,
            &[
                ("python", SMOKE_PYTHON),
                ("python_minutes", &PYTHON.minutes.to_string()),
                ("dist_minutes", &RELEASE_DIST.minutes.to_string()),
                ("smoke_minutes", &INSTALLER_SMOKE.minutes.to_string()),
                ("wrapper", &wrapper),
                ("versions_minutes", &RELEASE_VERSIONS.minutes.to_string()),
                ("lifecycle_minutes", &LIFECYCLE.minutes.to_string()),
                ("old", LIFECYCLE_OLD),
                ("new", LIFECYCLE_NEW),
            ],
        );
        (String::new(), uv + &smoke)
    } else {
        let test_step = format!(
            "Invoke-Step 'cargo-test' {} $cargo {}\n",
            CARGO_TEST.minutes,
            quoted(&command_line(&test))
        );
        let parity = if spec.parity {
            let pytest = plan::pytest_args(spec.parity_files);
            uv + &fill(
                PARITY_TEMPLATE,
                &[
                    ("release", &RELEASE_BUILD.minutes.to_string()),
                    ("parity", &PARITY.minutes.to_string()),
                    ("pytest", &quoted(&command_line(&pytest))),
                ],
            )
        } else {
            String::new()
        };
        (test_step, parity)
    };
    fill(
        RUN_TEMPLATE,
        &[
            ("job", JOB_SOURCE),
            ("processes", &PROCESS_LIMIT.to_string()),
            ("ci", CI_DIR),
            ("id", spec.id),
            ("jobs", &spec.jobs.to_string()),
            ("toolchain_minutes", &TOOLCHAIN.minutes.to_string()),
            ("toolchain", &quoted(&toolchain)),
            ("test_step", &test_step),
            ("after_steps", &after),
            ("step_mark", STEP_MARK),
        ],
    )
}

pub fn stop_script(id: &str) -> String {
    fill(
        STOP,
        &[
            ("lock_dir", LOCK_DIR),
            ("running", RUNNING),
            ("owner", OWNER),
            ("id", id),
            ("ci", CI_DIR),
            ("tarball", &tarball_name(id)),
        ],
    )
}

/// Removes the run's own tarball after a failed upload, and says `removed`,
/// `absent` or `failed: <why>`. The name is the run's alone, so no lock is taken.
pub fn cleanup_script(id: &str) -> String {
    fill(CLEANUP, &[("tarball", &tarball_name(id))])
}

/// Says `free N` (whole GiB free on C:) and, while a run of either project
/// holds the shared lock, `held <id> <process>` from its record.
pub fn state_script() -> String {
    fill(STATE, &[("lock_dir", LOCK_DIR), ("owner", OWNER)])
}

/// The bootstrap. The lock is ConsensFlow's, taken the way its runner takes it
/// (tools/xtask/src/windows/script.rs there): opened for this run alone before
/// anything else is done; a run holding it, or a ConsensFlow record of an older
/// script's run that is still a live PowerShell, refuses this one with 75 and the
/// holder's name, at once. The records are written as ConsensFlow writes them, so
/// its runs refuse a Kluris run's machine the same way.
const BOOTSTRAP: &str = r##"
$ProgressPreference = 'SilentlyContinue'
$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false) } catch {}
$OutputEncoding = [Text.UTF8Encoding]::new($false)
function Exit-Run([int]$code) { Write-Output "`n«exit»$code"; exit $code }
$lockdir = Join-Path $env:USERPROFILE '«lock_dir»'
$tarball = Join-Path $env:USERPROFILE '«tarball»'
$running = Join-Path $lockdir '«running»'
$owner = Join-Path $lockdir '«owner»'
$held = $false
$lock = $null
try {
New-Item -ItemType Directory -Force $lockdir | Out-Null
try { $lock = [IO.File]::Open((Join-Path $lockdir '«lock»'), 'OpenOrCreate', 'ReadWrite', 'None') } catch [IO.IOException] {} catch { $why = $_.Exception.Message }
$holder = Get-Content $running -ErrorAction SilentlyContinue
$other = if ($holder) { try { Get-Process -Id $holder -ErrorAction Stop | Where-Object { ($_.Name -in @('powershell', 'pwsh')) -and $_.StartTime -le (Get-Item $running).LastWriteTime } } catch {} }
if (-not $lock -or $other) {
  $of = @(Get-Content $owner -ErrorAction SilentlyContinue)
  $who = if ($of[0] -like 'kluris-*') { "a Kluris run ($($of[0]), process $($of[1]))" } elseif ($of[0]) { "a ConsensFlow run ($($of[0]), process $($of[1]))" } elseif ($other) { "process $($other.Id)" } else { 'a run that left no record' }
  Write-Output "The machine's one-run lock is held by $who$(if ($why) { " (the lock could not be opened: $why)" }): one run at a time, not waiting."
  Exit-Run 75
}
Set-Content -Path $running -Value $PID
Set-Content -Path $owner -Value @('«id»', $PID)
$held = $true
$run = Join-Path $env:USERPROFILE '«ci»\runs\«id»'
New-Item -ItemType Directory -Force $run | Out-Null
$tar = Join-Path $env:SystemRoot 'System32\tar.exe'
& $tar -xmzf $tarball -C $run
if ($LASTEXITCODE -ne 0) { Exit-Run $LASTEXITCODE }
Write-Output "«tree_mark»$(Get-Content -LiteralPath (Join-Path $run '«tree_file»') -TotalCount 1)"
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $run '«run_file»'))))
} finally {
  Remove-Item -LiteralPath $tarball -Force -ErrorAction SilentlyContinue
  if ($held) {
    Remove-Item -LiteralPath $owner -Force -ErrorAction SilentlyContinue
    if ((Get-Content $running -ErrorAction SilentlyContinue) -eq $PID) { Remove-Item -LiteralPath $running -Force -ErrorAction SilentlyContinue }
  }
  if ($lock) { $lock.Dispose() }
}
"##;

/// The run script, dot-sourced by the bootstrap with `$run` its folder.
///
/// - The run's process enters a job that ends every process in it when the run's
///   process ends (`KILL_ON_JOB_CLOSE`, 0x2000) and holds at most
///   [`PROCESS_LIMIT`] at once (`ACTIVE_PROCESS`, 0x8): what a step leaves
///   behind (the linker's `mspdbsrv`, a test's child) ends with the run.
/// - Each step is started with the run's standard handles (its output streams
///   back as it comes, never through PowerShell) and waited on for its minutes;
///   one still running then is ended with its tree, and its status is 124.
/// - The tools are the machine's rustup by its full path, with Kluris's own
///   `RUSTUP_HOME`, `CARGO_HOME`, target folder and temp folder; PATH is changed
///   for this process only.
/// - A machine without rustup is told about and not fixed.
const RUN_TEMPLATE: &str = r##"
$ErrorActionPreference = 'Continue'
$state = @{ failed = 0; last = 0; job = [IntPtr]::Zero }
try {
  Add-Type -TypeDefinition @'
«job»
'@
  $state.job = [KlurisJob]::Enter(«processes»)
  if ($state.job -eq [IntPtr]::Zero) { throw [KlurisJob]::Why }
} catch { Write-Output "== no job ($($_.Exception.Message)): no run starts without its bound on processes"; Exit-Run 1 }
Write-Output "== job: at most «processes» processes, all ended with the run"
$ci = Join-Path $env:USERPROFILE '«ci»'
$tools = Join-Path $ci 'tools'
$cache = Join-Path $ci 'cache'
$tmp = Join-Path $run 'tmp'
foreach ($folder in @($tools, $cache, $tmp)) { New-Item -ItemType Directory -Force $folder | Out-Null }
$cutoff = (Get-Date).AddDays(-1)
foreach ($old in @(Get-ChildItem -LiteralPath (Join-Path $ci 'runs') -Directory | Where-Object { $_.Name -ne '«id»' -and $_.LastWriteTime -lt $cutoff })) {
  Remove-Item -LiteralPath $old.FullName -Recurse -Force -ErrorAction SilentlyContinue
  if (Test-Path -LiteralPath $old.FullName) { Write-Output "== sweep: could not remove runs\$($old.Name)" } else { Write-Output "== sweep: removed runs\$($old.Name)" }
}
Get-ChildItem -LiteralPath $env:USERPROFILE -File -Filter 'kluris-*.tgz' | Where-Object { $_.LastWriteTime -lt $cutoff } | Remove-Item -Force -ErrorAction SilentlyContinue
$rustup = Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe'
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not ((Test-Path -LiteralPath $rustup -PathType Leaf) -and (Test-Path -LiteralPath $cargo -PathType Leaf))) {
  Write-Output "Missing on $($env:COMPUTERNAME): rustup for this user ($rustup and $cargo). Kluris installs no Rust there: install rustup at the machine, then run again."
  Exit-Run 1
}
$env:RUSTUP_HOME = Join-Path $tools 'rustup'
$env:CARGO_HOME = Join-Path $tools 'cargo'
$env:CARGO_TARGET_DIR = Join-Path $cache 'target'
$env:CARGO_BUILD_JOBS = '«jobs»'
$env:CARGO_TERM_COLOR = 'never'
$env:TEMP = $tmp
$env:TMP = $tmp
$env:Path = "$(Split-Path $cargo);" + $env:Path
function Invoke-Step([string]$name, [int]$minutes, [string]$exe, [string]$line) {
  Write-Output "«step_mark»${name}: $exe $line (at most $minutes min)"
  $started = Get-Date
  $code = 1
  try {
    $start = [Diagnostics.ProcessStartInfo]::new($exe, $line)
    $start.UseShellExecute = $false
    $start.WorkingDirectory = $run
    $process = [Diagnostics.Process]::Start($start)
    if ($process.WaitForExit($minutes * 60000)) {
      $process.WaitForExit()
      $code = $process.ExitCode
    } else {
      Write-Output "«step_mark»${name}: still running after $minutes min, ended"
      taskkill /PID $process.Id /T /F 2>&1 | Out-Null
      $code = 124
    }
  } catch { Write-Output "«step_mark»${name}: could not start: $($_.Exception.Message)" }
  $state.last = $code
  if ($code -ne 0 -and $state.failed -eq 0) { $state.failed = $code }
  Write-Output "«step_mark»$name exit $code $([int]((Get-Date) - $started).TotalSeconds)s"
}
Invoke-Step 'toolchain' «toolchain_minutes» $rustup «toolchain»
if ($state.failed) { Exit-Run $state.failed }
«test_step»«after_steps»Exit-Run $state.failed
"##;

/// uv fetched (and its hash checked) into the tools folder when it is not
/// there: the parity steps and the installer smoke run on it.
const UV_TEMPLATE: &str = r##"$uvdir = Join-Path $tools 'uv-«uv_version»'
$uv = Join-Path $uvdir 'uv.exe'
if (-not (Test-Path -LiteralPath $uv -PathType Leaf)) {
  $started = Get-Date
  $zip = Join-Path $tools 'uv-«uv_version».zip'
  try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -UseBasicParsing -TimeoutSec 300 -Uri '«uv_url»' -OutFile $zip
    $sha = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
    if ($sha -ne '«uv_sha256»') { throw "the zip hashes to $sha, not «uv_sha256»" }
    Expand-Archive -LiteralPath $zip -DestinationPath $uvdir -Force
  } catch { Write-Output "== step uv: could not fetch uv «uv_version»: $($_.Exception.Message)" } finally { Remove-Item -LiteralPath $zip -Force -ErrorAction SilentlyContinue }
  $code = if (Test-Path -LiteralPath $uv -PathType Leaf) { 0 } else { 1 }
  if ($code -and -not $state.failed) { $state.failed = $code }
  Write-Output "== step uv exit $code $([int]((Get-Date) - $started).TotalSeconds)s"
}
"##;

/// The parity steps: a release `kluris.exe`, then the harness, with the
/// oracle's venv, uv's cache and Python kept in Kluris's folder.
const PARITY_TEMPLATE: &str = r##"if (Test-Path -LiteralPath $uv -PathType Leaf) {
  $env:UV_CACHE_DIR = Join-Path $cache 'uv'
  $env:UV_PYTHON_INSTALL_DIR = Join-Path $tools 'python'
  $env:UV_PYTHON_PREFERENCE = 'only-managed'
  $env:UV_PROJECT_ENVIRONMENT = Join-Path $cache 'parity-venv'
  $env:KLURIS_PARITY_ORACLE_DIR = Join-Path $cache 'oracle'
  $env:KLURIS_V3_BIN = Join-Path $env:CARGO_TARGET_DIR 'release\kluris.exe'
  $env:Path = "$uvdir;" + $env:Path
  Invoke-Step 'release-build' «release» $cargo 'build --release --package kluris'
  if ($state.last -eq 0) { Invoke-Step 'parity' «parity» $uv «pytest» }
}
"##;

/// The installer smoke: uv's Python for the local release server, the tree's
/// release built by dist (scripts\release\build-local.ps1, dist fetched into
/// the tools folder), then scripts\release\installer-smoke.ps1 against it;
/// then the tree built as two releases (build-versions.ps1) and the installed
/// life cycle across them (lifecycle-smoke.ps1), whatever the first smoke's
/// end. Each smoke saves the user PATH in `$record` before it installs and
/// removes the record once it has written the PATH back; a record still
/// there after a step (a smoke ended past its bound never reaches its
/// `finally`) is written back here, and by the stop.
const SMOKE_TEMPLATE: &str = r##"if (Test-Path -LiteralPath $uv -PathType Leaf) {
  $env:UV_CACHE_DIR = Join-Path $cache 'uv'
  $env:UV_PYTHON_INSTALL_DIR = Join-Path $tools 'python'
  $env:UV_PYTHON_PREFERENCE = 'only-managed'
  $env:KLURIS_TOOLS_DIR = $tools
  $env:Path = "$uvdir;" + $env:Path
  $powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  $record = Join-Path $run 'user-path.saved'
  Invoke-Step 'python' «python_minutes» $uv 'python install «python»'
  $python = if ($state.last -eq 0) { @(& $uv python find «python» 2>$null)[-1] } else { '' }
  if ($python) { Invoke-Step 'release-dist' «dist_minutes» $powershell '-NoProfile -ExecutionPolicy Bypass -File scripts\release\build-local.ps1' }
  if ($python -and $state.last -eq 0) {
    Invoke-Step 'installer-smoke' «smoke_minutes» $powershell "-NoProfile -ExecutionPolicy Bypass -File scripts\release\installer-smoke.ps1 -Release target\distrib -Python `"$python`" -Record `"$record`"«wrapper»"
  }
  if (Test-Path -LiteralPath $record) {
    Write-Output '== the installer smoke left the user PATH it saved: writing it back'
    Invoke-Step 'user-path' 2 $powershell "-NoProfile -ExecutionPolicy Bypass -File scripts\release\user-path.ps1 -Restore `"$record`""
  }
  if ($python) { Invoke-Step 'release-versions' «versions_minutes» $powershell '-NoProfile -ExecutionPolicy Bypass -File scripts\release\build-versions.ps1 -Out target\releases -Versions «old»,«new»' }
  if ($python -and $state.last -eq 0 -and -not (Test-Path -LiteralPath $record)) {
    Invoke-Step 'lifecycle' «lifecycle_minutes» $powershell "-NoProfile -ExecutionPolicy Bypass -File scripts\release\lifecycle-smoke.ps1 -Releases target\releases -Old «old» -New «new» -Python `"$python`" -Record `"$record`""
  }
  if (Test-Path -LiteralPath $record) {
    Write-Output '== the life-cycle smoke left the user PATH it saved: writing it back'
    Invoke-Step 'user-path' 2 $powershell "-NoProfile -ExecutionPolicy Bypass -File scripts\release\user-path.ps1 -Restore `"$record`""
  }
}
"##;

/// The C# of the job: `Enter` makes it (end-on-close and the process limit),
/// puts this process in it (and with it every process it starts) and answers
/// it, or zero with `Why` said. The structures are the system's
/// (`JOBOBJECT_EXTENDED_LIMIT_INFORMATION`; 9 is its information class).
const JOB_SOURCE: &str = r#"using System;
using System.Runtime.InteropServices;
public static class KlurisJob {
  [StructLayout(LayoutKind.Sequential)] struct Basic { public long UserTime, JobUserTime; public uint Flags; public UIntPtr MinWs, MaxWs; public uint Active; public UIntPtr Affinity; public uint Priority, Scheduling; }
  [StructLayout(LayoutKind.Sequential)] struct Io { public ulong A, B, C, D, E, F; }
  [StructLayout(LayoutKind.Sequential)] struct Extended { public Basic Basic; public Io Io; public UIntPtr ProcessMem, JobMem, PeakProcess, PeakJob; }
  [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr CreateJobObject(IntPtr attributes, string name);
  [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetInformationJobObject(IntPtr job, int kind, ref Extended info, int size);
  [DllImport("kernel32.dll", SetLastError = true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
  [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
  public static string Why = "";
  public static IntPtr Enter(uint processes) {
    IntPtr job = CreateJobObject(IntPtr.Zero, null);
    if (job == IntPtr.Zero) { Why = "CreateJobObject failed: " + Marshal.GetLastWin32Error(); return IntPtr.Zero; }
    var limits = new Extended();
    limits.Basic.Flags = 0x2000 | 0x8;
    limits.Basic.Active = processes;
    if (!SetInformationJobObject(job, 9, ref limits, Marshal.SizeOf(typeof(Extended)))) { Why = "SetInformationJobObject failed: " + Marshal.GetLastWin32Error(); return IntPtr.Zero; }
    if (!AssignProcessToJobObject(job, GetCurrentProcess())) { Why = "AssignProcessToJobObject failed: " + Marshal.GetLastWin32Error(); return IntPtr.Zero; }
    return job;
  }
}"#;

/// The stop: the run's process tree ended (its job ends the rest) only if the
/// lock's record names this run, its records and tarball removed. It ends with 0
/// whether or not there was anything left to end.
const STOP: &str = r##"
$ProgressPreference = 'SilentlyContinue'
$lockdir = Join-Path $env:USERPROFILE '«lock_dir»'
$running = Join-Path $lockdir '«running»'
$owner = Join-Path $lockdir '«owner»'
$of = @(Get-Content $owner -ErrorAction SilentlyContinue)
if ($of[0] -eq '«id»') { taskkill /PID ($of[1]) /T /F 2>&1 | Out-Null; Remove-Item -LiteralPath $owner -Force -ErrorAction SilentlyContinue; if ((Get-Content $running -ErrorAction SilentlyContinue) -eq $of[1]) { Remove-Item -LiteralPath $running -Force -ErrorAction SilentlyContinue } }
$record = Join-Path $env:USERPROFILE '«ci»\runs\«id»\user-path.saved'
if (Test-Path -LiteralPath $record) { & (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') -NoProfile -ExecutionPolicy Bypass -File (Join-Path $env:USERPROFILE '«ci»\runs\«id»\scripts\release\user-path.ps1') -Restore $record }
Remove-Item -LiteralPath (Join-Path $env:USERPROFILE '«tarball»') -Force -ErrorAction SilentlyContinue
exit 0
"##;

/// The cleanup of a failed upload: the run's tarball and nothing else.
const CLEANUP: &str = r##"
$ProgressPreference = 'SilentlyContinue'
$tarball = Join-Path $env:USERPROFILE '«tarball»'
if (-not (Test-Path -LiteralPath $tarball)) { 'absent'; exit 0 }
try { Remove-Item -LiteralPath $tarball -Force -ErrorAction Stop; 'removed' } catch { "failed: $($_.Exception.Message)" }
exit 0
"##;

const STATE: &str = r##"
$ProgressPreference = 'SilentlyContinue'
"free $([math]::Floor((Get-PSDrive -Name C).Free / 1GB))"
$of = @(Get-Content -LiteralPath (Join-Path $env:USERPROFILE '«lock_dir»\«owner»') -ErrorAction SilentlyContinue)
if ($of.Count) { "held $($of -join ' ')" }
"##;
