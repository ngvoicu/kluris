//! Per-user sync services (SPEC Phase 7, TEST-K3-51): `kluris sync
//! --install-service --brain X` makes the operating system run `kluris sync
//! --watch --brain X` for the user, from login on; `--uninstall-service`
//! stops and removes it. Both are idempotent.
//!
//! - macOS: a launchd agent, `~/Library/LaunchAgents/<label>.plist` with
//!   the label `dev.ngvoicu.kluris.sync.<brain>`, loaded with `launchctl
//!   bootstrap gui/<uid>` and removed with `bootout`; it logs to
//!   `~/Library/Logs/kluris/sync-<brain>.log`. A clean exit (0, as after
//!   SIGTERM) is not restarted; a failure is, every 30 s at most.
//! - Stopping a launchd or systemd service waits 300 s (`ExitTimeOut`,
//!   `TimeoutStopSec`; their defaults are 20 s and 90 s) before a kill, so
//!   a sync in progress, whose fetch and push may each take a minute,
//!   finishes.
//! - Linux: a systemd user unit, `$XDG_CONFIG_HOME/systemd/user/
//!   kluris-sync-<brain>.service` (`~/.config` without it), enabled and
//!   started with `systemctl --user enable --now`, removed after `disable
//!   --now`; it logs to the user journal and restarts on failure. Without a
//!   systemd user instance nothing is written.
//! - Windows: a Task Scheduler task `\Kluris\sync-<brain>` for the current
//!   user at logon, created from XML (`schtasks /Create /XML`: the task is
//!   hidden, has no time limit and runs once at a time, none of which
//!   `schtasks`'s flags can set), started with `/Run`, ended and deleted
//!   with `/End` and `/Delete`. It runs kluris under `conhost.exe
//!   --headless`, so no console window opens, with `--log-file
//!   %LOCALAPPDATA%\kluris\logs\sync-<brain>.log`. No admin rights.
//!
//! Each runs the absolute path of the kluris that installed it. The
//! environment the watch needs goes with it: `PATH` (launchd and systemd
//! start agents with a minimal one, without Homebrew's git or credential
//! helpers) and `KLURIS_CONFIG` when set, in the definition on macOS and
//! Linux and as `--config` on Windows, where a task has no environment.
//!
//! The commands run through [`Runner`], so tests assert them with a fake.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::atomic;

/// The service manager of the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Launchd,
    Systemd,
    TaskScheduler,
}

impl Manager {
    /// This build's.
    pub fn current() -> Manager {
        if cfg!(windows) {
            Manager::TaskScheduler
        } else if cfg!(target_os = "macos") {
            Manager::Launchd
        } else {
            Manager::Systemd
        }
    }
}

/// The user id launchd's `gui/<uid>` domain names (0 where there is none).
pub fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        rustix::process::getuid().as_raw()
    }
    #[cfg(not(unix))]
    {
        0
    }
}

/// Everything a service depends on, from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceEnv {
    pub manager: Manager,
    pub home: PathBuf,
    /// `XDG_CONFIG_HOME` when set and non-empty (systemd).
    pub xdg_config_home: Option<PathBuf>,
    /// `LOCALAPPDATA` (Task Scheduler).
    pub local_app_data: Option<PathBuf>,
    /// The user id (launchd's `gui/<uid>` domain).
    pub uid: u32,
    /// The account (Task Scheduler's logon trigger and principal): its SID,
    /// or `DOMAIN\user`.
    pub user: Option<String>,
    /// The kluris the service runs, absolute.
    pub kluris: PathBuf,
    /// `PATH`, for launchd and systemd.
    pub path_var: Option<String>,
    /// `KLURIS_CONFIG` when set.
    pub kluris_config: Option<PathBuf>,
    /// Where the task XML is written for `schtasks` to read.
    pub temp_dir: PathBuf,
}

/// One command's end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// `None` when killed by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Ran {
    fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// Runs the service manager's commands.
pub trait Runner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<Ran>;

    /// Wait before a retry.
    fn pause(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    /// Not a brain name a service can carry.
    BadName(String),
    /// A path that cannot be passed in a quoted Windows argument.
    Unquotable(PathBuf),
    /// No systemd user instance to talk to (its words).
    NoUserSystemd(String),
    /// What the environment lacks (`LOCALAPPDATA`, the user name).
    Missing(&'static str),
    /// A command that failed: the command line and its words.
    Command {
        command: String,
        detail: String,
    },
    Io {
        path: PathBuf,
        detail: String,
    },
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceError::BadName(name) => write!(
                f,
                "'{name}' is not a brain name a sync service can use (lowercase letters, digits and hyphens, starting with a letter)"
            ),
            ServiceError::Unquotable(path) => write!(
                f,
                "{} has a double quote in it, which a Windows task cannot pass",
                path.display()
            ),
            ServiceError::NoUserSystemd(detail) => write!(
                f,
                "no systemd user instance answers here, so a sync service cannot run: {detail}. \
                 Run `kluris sync --watch` in a terminal instead, or enable systemd for your user \
                 (on WSL: systemd=true under [boot] in /etc/wsl.conf)"
            ),
            ServiceError::Missing(what) => write!(f, "{what} is not set"),
            ServiceError::Command { command, detail } => {
                write!(f, "`{command}` failed: {detail}")
            }
            ServiceError::Io { path, detail } => write!(f, "{}: {detail}", path.display()),
        }
    }
}

impl std::error::Error for ServiceError {}

/// Whether a service runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceState {
    Running,
    /// Installed but not running: the manager's word for its state.
    Stopped(String),
}

/// An installed sync service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub brain: String,
    /// The label, unit or task name.
    pub name: String,
    /// The file that defines it (none for a task).
    pub definition: Option<PathBuf>,
    pub state: ServiceState,
}

/// What [`install`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    pub name: String,
    pub definition: Option<PathBuf>,
    /// The log file (none: the journal).
    pub log: Option<PathBuf>,
    /// Something was written, loaded or started; false when it was already
    /// installed as asked and running.
    pub changed: bool,
}

/// What [`uninstall`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallOutcome {
    pub name: String,
    /// It was there, and is gone.
    pub removed: bool,
}

const LABEL_PREFIX: &str = "dev.ngvoicu.kluris.sync.";
const UNIT_PREFIX: &str = "kluris-sync-";
const TASK_FOLDER: &str = r"\Kluris\";
const BOOTSTRAP_ATTEMPTS: usize = 3;

/// A name `kluris create` accepts; nothing else reaches a file name, a
/// label or a task name.
fn check_name(brain: &str) -> Result<(), ServiceError> {
    let mut chars = brain.chars();
    let ok = brain.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(ServiceError::BadName(brain.to_string()))
    }
}

/// Element text: `&`, `<` and `>` as XML entities.
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// One word of a systemd command line or assignment, quoted: `\` and `"`
/// escaped, `%` (specifiers) doubled; `$` doubled when `dollar` (command
/// lines expand variables).
fn systemd_quoted(text: &str, dollar: bool) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '$' if dollar => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A path in double quotes for a Windows command line.
fn windows_quoted(path: &Path) -> Result<String, ServiceError> {
    let text = path.display().to_string();
    if text.contains('"') {
        return Err(ServiceError::Unquotable(path.to_path_buf()));
    }
    Ok(format!("\"{text}\""))
}

/// `base` then `parts`, joined with `separator`: a definition names the
/// paths of the OS it is for (`/` for launchd and systemd, `\` for Task
/// Scheduler), the same whatever OS renders it.
fn joined(base: &Path, parts: &[&str], separator: char) -> PathBuf {
    let mut text = base.display().to_string();
    for part in parts {
        if !text.ends_with(separator) {
            text.push(separator);
        }
        text.push_str(part);
    }
    PathBuf::from(text)
}

fn unix_join(base: &Path, parts: &[&str]) -> PathBuf {
    joined(base, parts, '/')
}

fn windows_join(base: &Path, parts: &[&str]) -> PathBuf {
    joined(base, parts, '\\')
}

/// The unit's first line.
fn comment(brain: &str) -> String {
    format!(
        "Written by `kluris sync --install-service --brain {brain}`; `kluris sync --uninstall-service --brain {brain}` removes it."
    )
}

/// The XML definitions' comment: no `--` (and so no brain name, which may
/// hold one), as XML comments cannot.
const XML_COMMENT: &str = "<!-- A Kluris sync service: kluris sync writes it with its install-service option and removes it with uninstall-service. -->";

impl ServiceEnv {
    /// The label, unit or task name of the brain's service.
    pub fn name(&self, brain: &str) -> String {
        match self.manager {
            Manager::Launchd => format!("{LABEL_PREFIX}{brain}"),
            Manager::Systemd => format!("{UNIT_PREFIX}{brain}.service"),
            Manager::TaskScheduler => format!("{TASK_FOLDER}sync-{brain}"),
        }
    }

    /// Where the definitions are: the LaunchAgents or unit folder, the task
    /// folder.
    pub fn location(&self) -> PathBuf {
        match self.manager {
            Manager::Launchd => unix_join(&self.home, &["Library", "LaunchAgents"]),
            Manager::Systemd => match &self.xdg_config_home {
                Some(config) => unix_join(config, &["systemd", "user"]),
                None => unix_join(&self.home, &[".config", "systemd", "user"]),
            },
            Manager::TaskScheduler => PathBuf::from(TASK_FOLDER),
        }
    }

    /// The file that defines the brain's service (none for a task).
    pub fn definition(&self, brain: &str) -> Option<PathBuf> {
        match self.manager {
            Manager::Launchd => Some(unix_join(
                &self.location(),
                &[&format!("{}.plist", self.name(brain))],
            )),
            Manager::Systemd => Some(unix_join(&self.location(), &[&self.name(brain)])),
            Manager::TaskScheduler => None,
        }
    }

    /// The service's log file (none: the user journal).
    pub fn log(&self, brain: &str) -> Option<PathBuf> {
        let file = format!("sync-{brain}.log");
        match self.manager {
            Manager::Launchd => Some(unix_join(&self.home, &["Library", "Logs", "kluris", &file])),
            Manager::Systemd => None,
            Manager::TaskScheduler => Some(windows_join(
                self.local_app_data.as_deref()?,
                &["kluris", "logs", &file],
            )),
        }
    }

    /// The variables the service is given: `PATH`, then `KLURIS_CONFIG`.
    fn variables(&self) -> Vec<(&'static str, String)> {
        let mut vars = Vec::new();
        if let Some(path) = &self.path_var {
            vars.push(("PATH", path.clone()));
        }
        if let Some(config) = &self.kluris_config {
            vars.push(("KLURIS_CONFIG", config.display().to_string()));
        }
        vars
    }

    /// The definition: the plist, the unit, or the task XML.
    pub fn render(&self, brain: &str) -> Result<String, ServiceError> {
        check_name(brain)?;
        match self.manager {
            Manager::Launchd => Ok(self.plist(brain)),
            Manager::Systemd => Ok(self.unit(brain)),
            Manager::TaskScheduler => self.task(brain),
        }
    }

    fn plist(&self, brain: &str) -> String {
        let log = self.log(brain).unwrap_or_default().display().to_string();
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        );
        out.push_str(XML_COMMENT);
        out.push('\n');
        out.push_str("<plist version=\"1.0\">\n<dict>\n");
        out.push_str(&format!(
            "\t<key>Label</key>\n\t<string>{}</string>\n",
            xml(&self.name(brain))
        ));
        out.push_str("\t<key>ProgramArguments</key>\n\t<array>\n");
        let kluris = self.kluris.display().to_string();
        for arg in [kluris.as_str(), "sync", "--watch", "--brain", brain] {
            out.push_str(&format!("\t\t<string>{}</string>\n", xml(arg)));
        }
        out.push_str("\t</array>\n");
        let vars = self.variables();
        if !vars.is_empty() {
            out.push_str("\t<key>EnvironmentVariables</key>\n\t<dict>\n");
            for (key, value) in vars {
                out.push_str(&format!(
                    "\t\t<key>{key}</key>\n\t\t<string>{}</string>\n",
                    xml(&value)
                ));
            }
            out.push_str("\t</dict>\n");
        }
        out.push_str(
            "\t<key>RunAtLoad</key>\n\t<true/>\n\
             \t<key>KeepAlive</key>\n\t<dict>\n\t\t<key>SuccessfulExit</key>\n\t\t<false/>\n\t</dict>\n\
             \t<key>ThrottleInterval</key>\n\t<integer>30</integer>\n\
             \t<key>ExitTimeOut</key>\n\t<integer>300</integer>\n\
             \t<key>ProcessType</key>\n\t<string>Background</string>\n",
        );
        for key in ["StandardOutPath", "StandardErrorPath"] {
            out.push_str(&format!(
                "\t<key>{key}</key>\n\t<string>{}</string>\n",
                xml(&log)
            ));
        }
        out.push_str("</dict>\n</plist>\n");
        out
    }

    fn unit(&self, brain: &str) -> String {
        let mut out = format!(
            "# {}\n[Unit]\nDescription=Kluris sync for the brain {brain}\n\n[Service]\nType=simple\n",
            comment(brain)
        );
        out.push_str(&format!(
            "ExecStart={} sync --watch --brain {brain}\n",
            systemd_quoted(&self.kluris.display().to_string(), true)
        ));
        for (key, value) in self.variables() {
            out.push_str(&format!(
                "Environment={}\n",
                systemd_quoted(&format!("{key}={value}"), false)
            ));
        }
        out.push_str(
            "Restart=on-failure\nRestartSec=30\nTimeoutStopSec=300\n\n[Install]\nWantedBy=default.target\n",
        );
        out
    }

    fn task(&self, brain: &str) -> Result<String, ServiceError> {
        let user = self
            .user
            .as_deref()
            .ok_or(ServiceError::Missing("USERNAME"))?;
        let log = self
            .log(brain)
            .ok_or(ServiceError::Missing("LOCALAPPDATA"))?;
        let mut arguments = format!(
            "--headless {} sync --watch --brain {brain} --log-file {}",
            windows_quoted(&self.kluris)?,
            windows_quoted(&log)?
        );
        if let Some(config) = &self.kluris_config {
            arguments.push_str(&format!(" --config {}", windows_quoted(config)?));
        }
        let user = xml(user);
        Ok(format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
{comment}
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Kluris sync for the brain {brain}</Description>
    <URI>{uri}</URI>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>true</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>3</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>%SystemRoot%\System32\conhost.exe</Command>
      <Arguments>{arguments}</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
            comment = XML_COMMENT,
            uri = xml(&self.name(brain)),
            arguments = xml(&arguments),
        ))
    }

    fn gui_domain(&self) -> String {
        format!("gui/{}", self.uid)
    }

    fn launchd_target(&self, brain: &str) -> String {
        format!("{}/{}", self.gui_domain(), self.name(brain))
    }
}

fn line(program: &str, args: &[&str]) -> String {
    std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run a command; a failure to start it is an error, its exit code is not.
fn run(runner: &dyn Runner, program: &str, args: &[&str]) -> Result<Ran, ServiceError> {
    runner
        .run(program, args)
        .map_err(|e| ServiceError::Command {
            command: line(program, args),
            detail: e.to_string(),
        })
}

/// Run a command that must succeed.
fn must(runner: &dyn Runner, program: &str, args: &[&str]) -> Result<Ran, ServiceError> {
    let ran = run(runner, program, args)?;
    if ran.ok() {
        Ok(ran)
    } else {
        Err(failed(program, args, &ran))
    }
}

fn failed(program: &str, args: &[&str], ran: &Ran) -> ServiceError {
    let words = [ran.stderr.trim(), ran.stdout.trim()]
        .iter()
        .find(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| match ran.code {
            Some(code) => format!("exit {code}"),
            None => "killed".to_string(),
        });
    ServiceError::Command {
        command: line(program, args),
        detail: words,
    }
}

fn io_error(path: &Path, e: &io::Error) -> ServiceError {
    ServiceError::Io {
        path: path.to_path_buf(),
        detail: e.to_string(),
    }
}

/// Write `text` to `path` unless it already holds it; whether it wrote.
fn write_if_changed(path: &Path, text: &str) -> Result<bool, ServiceError> {
    match fs::read(path) {
        Ok(bytes) if bytes == text.as_bytes() => return Ok(false),
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_error(path, &e)),
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| io_error(dir, &e))?;
    }
    atomic::write_atomic(path, text.as_bytes()).map_err(|e| io_error(path, &e))?;
    Ok(true)
}

fn remove_if_there(path: &Path) -> Result<bool, ServiceError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io_error(path, &e)),
    }
}

/// Install the brain's service and start it (see the module docs).
pub fn install(
    env: &ServiceEnv,
    brain: &str,
    runner: &dyn Runner,
) -> Result<InstallOutcome, ServiceError> {
    let text = env.render(brain)?;
    let changed = match env.manager {
        Manager::Launchd => install_launchd(env, brain, &text, runner)?,
        Manager::Systemd => install_systemd(env, brain, &text, runner)?,
        Manager::TaskScheduler => install_task(env, brain, &text, runner)?,
    };
    Ok(InstallOutcome {
        name: env.name(brain),
        definition: env.definition(brain),
        log: env.log(brain),
        changed,
    })
}

/// Stop and remove the brain's service (see the module docs).
pub fn uninstall(
    env: &ServiceEnv,
    brain: &str,
    runner: &dyn Runner,
) -> Result<UninstallOutcome, ServiceError> {
    check_name(brain)?;
    let removed = match env.manager {
        Manager::Launchd => uninstall_launchd(env, brain, runner)?,
        Manager::Systemd => uninstall_systemd(env, brain, runner)?,
        Manager::TaskScheduler => uninstall_task(env, brain, runner)?,
    };
    Ok(UninstallOutcome {
        name: env.name(brain),
        removed,
    })
}

/// The launchd job's state: `None` when it is not loaded.
fn launchd_state(
    env: &ServiceEnv,
    brain: &str,
    runner: &dyn Runner,
) -> Result<Option<ServiceState>, ServiceError> {
    let target = env.launchd_target(brain);
    let ran = run(runner, "launchctl", &["print", &target])?;
    if !ran.ok() {
        return Ok(None);
    }
    let state = ran
        .stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("state = "))
        .unwrap_or("unknown")
        .trim()
        .to_string();
    Ok(Some(if state == "running" {
        ServiceState::Running
    } else {
        ServiceState::Stopped(state)
    }))
}

fn install_launchd(
    env: &ServiceEnv,
    brain: &str,
    text: &str,
    runner: &dyn Runner,
) -> Result<bool, ServiceError> {
    let plist = env.definition(brain).unwrap_or_default();
    let state = launchd_state(env, brain, runner)?;
    if let Some(dir) = env.log(brain).as_deref().and_then(Path::parent) {
        fs::create_dir_all(dir).map_err(|e| io_error(dir, &e))?;
    }
    let written = write_if_changed(&plist, text)?;
    let target = env.launchd_target(brain);
    match (state, written) {
        (Some(ServiceState::Running), false) => return Ok(false),
        (Some(_), false) => {
            must(runner, "launchctl", &["kickstart", &target])?;
            return Ok(true);
        }
        (Some(_), true) => {
            must(runner, "launchctl", &["bootout", &target])?;
        }
        (None, _) => {}
    }
    // A bootout finishes on its own time: a bootstrap right after it can
    // fail with an I/O error, so it is tried again.
    let domain = env.gui_domain();
    let plist_text = plist.display().to_string();
    let args = ["bootstrap", domain.as_str(), plist_text.as_str()];
    let mut last = None;
    for attempt in 0..BOOTSTRAP_ATTEMPTS {
        if attempt > 0 {
            runner.pause(Duration::from_secs(1));
        }
        let ran = run(runner, "launchctl", &args)?;
        if ran.ok() {
            return Ok(true);
        }
        last = Some(ran);
    }
    let ran = last.unwrap_or(Ran {
        code: None,
        stdout: String::new(),
        stderr: String::new(),
    });
    Err(failed("launchctl", &args, &ran))
}

fn uninstall_launchd(
    env: &ServiceEnv,
    brain: &str,
    runner: &dyn Runner,
) -> Result<bool, ServiceError> {
    let loaded = launchd_state(env, brain, runner)?.is_some();
    if loaded {
        must(
            runner,
            "launchctl",
            &["bootout", &env.launchd_target(brain)],
        )?;
    }
    let removed = match env.definition(brain) {
        Some(plist) => remove_if_there(&plist)?,
        None => false,
    };
    Ok(loaded || removed)
}

/// A systemd user instance answers.
fn systemd_reachable(runner: &dyn Runner) -> Result<(), ServiceError> {
    let args = ["--user", "show-environment"];
    match runner.run("systemctl", &args) {
        Ok(ran) if ran.ok() => Ok(()),
        Ok(ran) => Err(ServiceError::NoUserSystemd(
            [ran.stderr.trim(), ran.stdout.trim()]
                .iter()
                .find(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{} failed", line("systemctl", &args))),
        )),
        Err(e) => Err(ServiceError::NoUserSystemd(format!("systemctl: {e}"))),
    }
}

/// `systemctl --user is-active`'s word for the unit.
fn systemd_state(unit: &str, runner: &dyn Runner) -> Result<ServiceState, ServiceError> {
    let ran = run(runner, "systemctl", &["--user", "is-active", unit])?;
    let word = ran.stdout.trim();
    Ok(if ran.ok() && word == "active" {
        ServiceState::Running
    } else if word.is_empty() {
        ServiceState::Stopped("unknown".into())
    } else {
        ServiceState::Stopped(word.to_string())
    })
}

fn install_systemd(
    env: &ServiceEnv,
    brain: &str,
    text: &str,
    runner: &dyn Runner,
) -> Result<bool, ServiceError> {
    systemd_reachable(runner)?;
    let unit = env.name(brain);
    let running = systemd_state(&unit, runner)? == ServiceState::Running;
    let path = env.definition(brain).unwrap_or_default();
    let written = write_if_changed(&path, text)?;
    if running && !written {
        return Ok(false);
    }
    if written {
        must(runner, "systemctl", &["--user", "daemon-reload"])?;
    }
    must(runner, "systemctl", &["--user", "enable", "--now", &unit])?;
    if running && written {
        must(runner, "systemctl", &["--user", "restart", &unit])?;
    }
    Ok(true)
}

fn uninstall_systemd(
    env: &ServiceEnv,
    brain: &str,
    runner: &dyn Runner,
) -> Result<bool, ServiceError> {
    let path = env.definition(brain).unwrap_or_default();
    match fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(io_error(&path, &e)),
    }
    systemd_reachable(runner)?;
    let unit = env.name(brain);
    must(runner, "systemctl", &["--user", "disable", "--now", &unit])?;
    remove_if_there(&path)?;
    must(runner, "systemctl", &["--user", "daemon-reload"])?;
    Ok(true)
}

/// Whether the task exists.
fn task_exists(task: &str, runner: &dyn Runner) -> Result<bool, ServiceError> {
    Ok(run(runner, "schtasks", &["/Query", "/TN", task])?.ok())
}

/// The task's XML as schtasks reads it: UTF-16LE with a byte order mark.
fn utf16le(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

fn install_task(
    env: &ServiceEnv,
    brain: &str,
    text: &str,
    runner: &dyn Runner,
) -> Result<bool, ServiceError> {
    let task = env.name(brain);
    if task_exists(&task, runner)? {
        // Ended so the new definition is what runs; not running is fine.
        run(runner, "schtasks", &["/End", "/TN", &task])?;
    }
    let file = env
        .temp_dir
        .join(format!("kluris-sync-{brain}-{}.xml", std::process::id()));
    fs::write(&file, utf16le(text)).map_err(|e| io_error(&file, &e))?;
    let file_text = file.display().to_string();
    let created = must(
        runner,
        "schtasks",
        &["/Create", "/TN", &task, "/XML", &file_text, "/F"],
    );
    let _ = fs::remove_file(&file);
    created?;
    must(runner, "schtasks", &["/Run", "/TN", &task])?;
    Ok(true)
}

fn uninstall_task(
    env: &ServiceEnv,
    brain: &str,
    runner: &dyn Runner,
) -> Result<bool, ServiceError> {
    let task = env.name(brain);
    if !task_exists(&task, runner)? {
        return Ok(false);
    }
    run(runner, "schtasks", &["/End", "/TN", &task])?;
    must(runner, "schtasks", &["/Delete", "/TN", &task, "/F"])?;
    Ok(true)
}

/// The brain a definition file or task name belongs to, if it is one of
/// Kluris's.
fn brain_of<'a>(name: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let brain = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    check_name(brain).ok().map(|()| brain)
}

/// The files in `dir` that are Kluris's definitions, sorted by brain.
fn definitions(
    dir: &Path,
    prefix: &str,
    suffix: &str,
) -> Result<Vec<(String, PathBuf)>, ServiceError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_error(dir, &e)),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| io_error(dir, &e))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some(brain) = brain_of(name, prefix, suffix) {
            found.push((brain.to_string(), entry.path()));
        }
    }
    found.sort();
    Ok(found)
}

/// Every installed Kluris sync service with whether it runs. Only reads.
/// A systemd unit with no user instance to stop it is an error.
pub fn installed(env: &ServiceEnv, runner: &dyn Runner) -> Result<Vec<Installed>, ServiceError> {
    match env.manager {
        Manager::Launchd => {
            let found = definitions(&env.location(), LABEL_PREFIX, ".plist")?;
            let mut list = Vec::new();
            for (brain, path) in found {
                let state = launchd_state(env, &brain, runner)?
                    .unwrap_or_else(|| ServiceState::Stopped("not loaded".into()));
                list.push(Installed {
                    name: env.name(&brain),
                    brain,
                    definition: Some(path),
                    state,
                });
            }
            Ok(list)
        }
        Manager::Systemd => {
            let found = definitions(&env.location(), UNIT_PREFIX, ".service")?;
            if found.is_empty() {
                return Ok(Vec::new());
            }
            systemd_reachable(runner)?;
            let mut list = Vec::new();
            for (brain, path) in found {
                let name = env.name(&brain);
                let state = systemd_state(&name, runner)?;
                list.push(Installed {
                    brain,
                    name,
                    definition: Some(path),
                    state,
                });
            }
            Ok(list)
        }
        Manager::TaskScheduler => {
            let ran = must(runner, "powershell.exe", &task_states_args())?;
            let mut list = Vec::new();
            for line in ran.stdout.lines() {
                let Some((task, state)) = line.trim().split_once('\t') else {
                    continue;
                };
                let Some(brain) = brain_of(task, "sync-", "") else {
                    continue;
                };
                let state = state.trim();
                let state = if state == "Running" {
                    ServiceState::Running
                } else {
                    ServiceState::Stopped(state.to_string())
                };
                list.push(Installed {
                    brain: brain.to_string(),
                    name: format!("{TASK_FOLDER}{task}"),
                    definition: None,
                    state,
                });
            }
            list.sort_by(|a, b| a.brain.cmp(&b.brain));
            list.dedup_by(|a, b| a.name == b.name);
            Ok(list)
        }
    }
}

/// The tasks in `\Kluris\` and their state, one `<name>\t<state>` line each:
/// PowerShell's ScheduledTask `State` is an enum (`Ready`, `Running`,
/// `Disabled`, `Queued`, `Unknown`) whose name is the same in every display
/// language, where `schtasks /Query`'s status column is translated. No
/// `\Kluris\` folder prints nothing.
const TASK_STATES_SCRIPT: &str = "Get-ScheduledTask -TaskPath '\\Kluris\\' -ErrorAction SilentlyContinue | ForEach-Object { $_.TaskName + [char]9 + [string]$_.State }";

fn task_states_args() -> [&'static str; 4] {
    [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        TASK_STATES_SCRIPT,
    ]
}

/// For `kluris remove`: stop and remove the brain's service when it has one
/// ([`uninstall`]); its name, or `None` when there is none (with launchd and
/// systemd, no definition file means no command is run).
pub fn remove_brain_service(
    env: &ServiceEnv,
    brain: &str,
    runner: &dyn Runner,
) -> Result<Option<String>, ServiceError> {
    if check_name(brain).is_err() {
        return Ok(None);
    }
    if let Some(definition) = env.definition(brain) {
        match fs::symlink_metadata(&definition) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_error(&definition, &e)),
        }
    }
    let done = uninstall(env, brain, runner)?;
    Ok(done.removed.then_some(done.name))
}

/// The installed services, for `kluris self-uninstall`
/// ([`crate::dist::SyncServices`]).
pub struct Services<'a> {
    pub env: &'a ServiceEnv,
    pub runner: &'a dyn Runner,
}

impl crate::dist::SyncServices for Services<'_> {
    fn location(&self) -> PathBuf {
        self.env.location()
    }

    fn installed(&self) -> Result<Vec<Installed>, String> {
        installed(self.env, self.runner).map_err(|e| e.to_string())
    }

    fn remove(&self, service: &Installed) -> Result<(), String> {
        uninstall(self.env, &service.brain, self.runner)
            .map(drop)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_kluris_shaped_names_are_brains() {
        assert_eq!(
            brain_of("kluris-sync-acme.service", UNIT_PREFIX, ".service"),
            Some("acme")
        );
        assert_eq!(
            brain_of("kluris-sync-.service", UNIT_PREFIX, ".service"),
            None
        );
        assert_eq!(
            brain_of("kluris-sync-A.service", UNIT_PREFIX, ".service"),
            None
        );
        assert_eq!(brain_of("other.service", UNIT_PREFIX, ".service"), None);
    }
}
