//! `kluris self-update [--check] [--version X] [--json]` (research-02 §5),
//! with the `self_update` crate:
//!
//! 1. The install receipt says where the installer put kluris; any other
//!    copy (a cargo build, a copy elsewhere) is refused before the network.
//! 2. The GitHub release listing gives the newest version (`--check` stops
//!    here); `--version` names one instead.
//! 3. The archive is the asset named exactly `kluris-<target>.tar.gz` /
//!    `.zip` for the target this binary was built for (`KLURIS_TARGET`), the
//!    binary at `kluris-<target>/kluris(.exe)` in it; its SHA-256 must match
//!    the release's `sha256.sum` and GitHub's digest when there is one, or
//!    nothing is replaced.
//! 4. The binary is replaced in place (`self_replace`, Windows included),
//!    the receipt's `version` updated, then the new binary runs as a fresh
//!    process, `<install path> doctor`, and its exit code is the command's.
//!
//! `KLURIS_GITHUB_TOKEN` is sent when set (never `GITHUB_TOKEN`).
//! `KLURIS_GITHUB_API_URL` replaces `https://api.github.com` for a GitHub
//! API mirror or GitHub Enterprise: https, or plain http only to a loopback
//! host (where the tests' fake runs); anything else is refused before any
//! request. `sha256.sum` is checked whatever the API.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use kluris_core::dist::{
    self, REPO_NAME, REPO_OWNER, Receipt, ReceiptError, SUMS_ASSET, archive_name, binary_in_archive,
};
use self_update::backends::github;
use self_update::update::{ReleaseStatus, UpdateStrategy};

use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::resolve::DistPaths;

/// The target this binary was built for (`build.rs`).
const TARGET: &str = env!("KLURIS_TARGET");
const CURRENT: &str = env!("CARGO_PKG_VERSION");

pub fn run(check: bool, version: Option<&str>, as_json: bool) -> Result<(), CliError> {
    let paths = DistPaths::from_env()?;
    let env = paths.env();
    let receipt_path = env.receipt_path().ok_or_else(|| {
        CliError::new("Cannot find the install receipt folder; set LOCALAPPDATA.")
    })?;
    let exe = std::env::current_exe()
        .map_err(|e| CliError::new(format!("Cannot find this kluris binary: {e}")))?;
    let receipt = Receipt::read(&receipt_path).map_err(|e| refusal(&e, &exe))?;
    let installed = receipt.install_prefix.join(env.binary_name());
    if !same_file(&exe, &installed) {
        return Err(CliError::new(format!(
            "This kluris ({}) is not the installed copy in {} (install receipt {}). \
             Update it the way it was installed, or run `{} self-update`.",
            exe.display(),
            receipt.install_prefix.display(),
            receipt_path.display(),
            installed.display()
        )));
    }
    let api = Api::from_env()?;

    // `--version` names the release; otherwise the listing's newest decides.
    let wanted = match version {
        Some(v) if !check => v.trim_start_matches('v').to_string(),
        _ => {
            let latest = newest_release(&api)?;
            let available = latest.as_deref().is_some_and(|l| {
                self_update::version::bump_is_greater(CURRENT, l).unwrap_or(false)
            });
            match latest {
                Some(latest) if available && !check => latest,
                latest => {
                    let latest = latest.unwrap_or_else(|| CURRENT.to_string());
                    if check {
                        print_check(&latest, available, as_json);
                    } else {
                        print_up_to_date(&latest, as_json);
                    }
                    return Ok(());
                }
            }
        }
    };
    let release = install(&api, &wanted, &exe)?;
    let new_version = release.version().to_string();
    let receipt_note = receipt
        .write_version(&receipt_path, &new_version)
        .err()
        .map(|e| {
            format!(
                "kluris {new_version} is installed, but the receipt {} was not updated: {e}",
                receipt_path.display()
            )
        });
    if let Some(note) = &receipt_note {
        eprintln!("{note}");
    }
    run_doctor(&installed, &new_version, as_json)
}

/// The GitHub API to talk to.
struct Api {
    base: Option<String>,
    token: Option<String>,
}

impl Api {
    /// From the environment; a `KLURIS_GITHUB_API_URL` that is neither
    /// https nor plain http to a loopback host is refused.
    fn from_env() -> Result<Api, CliError> {
        let var = |name| {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let base = var("KLURIS_GITHUB_API_URL").map(|b| b.trim_end_matches('/').to_string());
        if let Some(base) = &base
            && !base.to_ascii_lowercase().starts_with("https://")
            && !is_http_loopback(base)
        {
            return Err(CliError::new(format!(
                "KLURIS_GITHUB_API_URL must be an https URL (plain http only to 127.0.0.1, \
                 localhost or [::1]), not {base}"
            )));
        }
        Ok(Api {
            base,
            token: var("KLURIS_GITHUB_TOKEN"),
        })
    }

    /// The base is plain http to this machine: the token may go there.
    fn http_loopback(&self) -> bool {
        self.base.as_deref().is_some_and(is_http_loopback)
    }
}

/// `url` is `http://` to 127.0.0.1, localhost or [::1] (any port).
fn is_http_loopback(url: &str) -> bool {
    let Some(rest) = url
        .get(..7)
        .filter(|scheme| scheme.eq_ignore_ascii_case("http://"))
        .map(|_| &url[7..])
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return false;
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().map(|h| format!("[{h}]")),
        None => authority.split(':').next().map(str::to_string),
    };
    host.is_some_and(|h| {
        matches!(
            h.to_ascii_lowercase().as_str(),
            "127.0.0.1" | "localhost" | "[::1]"
        )
    })
}

/// The newest version in the release listing; `None` when there is no
/// release.
fn newest_release(api: &Api) -> Result<Option<String>, CliError> {
    let mut builder = github::ReleaseList::configure();
    builder.repo_owner(REPO_OWNER).repo_name(REPO_NAME);
    if let Some(base) = &api.base {
        builder.api_base_url(base.clone());
    }
    if let Some(token) = &api.token {
        builder.auth_token(token.clone());
        if api.http_loopback() {
            builder.dangerously_allow_non_https_auth_forwarding();
        }
    }
    let releases = builder
        .build()
        .and_then(|list| list.fetch())
        .map_err(|e| update_error(&e, None))?;
    let mut newest: Option<String> = None;
    for release in releases.into_vec() {
        let v = release.version();
        let newer = match &newest {
            None => semver_ok(v),
            Some(n) => self_update::version::bump_is_greater(n, v).unwrap_or(false),
        };
        if newer {
            newest = Some(v.to_string());
        }
    }
    Ok(newest)
}

fn semver_ok(v: &str) -> bool {
    self_update::version::bump_is_greater(v, v).is_ok()
}

/// Download, verify and put release `version` in place of `exe`.
fn install(api: &Api, version: &str, exe: &Path) -> Result<self_update::Release, CliError> {
    let want = archive_name(TARGET);
    let matcher_want = want.clone();
    let mut builder = github::Update::configure();
    builder
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(dist::APP_NAME)
        .target(TARGET)
        .current_version(CURRENT)
        .release_tag(format!("v{version}"))
        .bin_install_path(exe)
        .bin_path_in_archive(binary_in_archive(TARGET))
        .asset_matcher(move |assets| assets.iter().find(|a| a.name() == matcher_want).cloned())
        .checksum_from_asset(SUMS_ASSET)
        .update_strategy(UpdateStrategy::Latest)
        .show_download_progress(false)
        .show_output(false)
        .no_confirm(true);
    if let Some(base) = &api.base {
        builder.api_base_url(base.clone());
    }
    if let Some(token) = &api.token {
        builder.auth_token(token.clone());
        if api.http_loopback() {
            builder.dangerously_allow_non_https_auth_forwarding();
        }
    }
    let status = builder
        .build()
        .and_then(|u| u.update_extended())
        .map_err(|e| update_error(&e, Some((version, &want))))?;
    match status {
        ReleaseStatus::Updated(release) => Ok(release),
        _ => Err(CliError::new(format!(
            "Release v{version} was not installed; nothing was replaced."
        ))),
    }
}

/// Why the receipt does not let this copy update.
fn refusal(err: &ReceiptError, exe: &Path) -> CliError {
    match err {
        ReceiptError::Missing(path) => CliError::new(format!(
            "No install receipt at {}: this kluris ({}) was not installed by the Kluris \
             installer. Update it the way it was installed (rebuild a source build), or \
             install with the Kluris installer.",
            path.display(),
            exe.display()
        )),
        other => CliError::new(other.to_string()),
    }
}

/// What a failed GitHub call or update says. `release` is `(version,
/// archive name)` once an update has started.
fn update_error(err: &self_update::Error, release: Option<(&str, &str)>) -> CliError {
    use self_update::Error as E;
    let message = match err {
        E::RateLimited { status, .. } => format!(
            "GitHub's API rate limit was reached (HTTP {status}). Set KLURIS_GITHUB_TOKEN to a \
             GitHub token (no scopes needed) and run again, or try again later."
        ),
        E::Unauthorized { status, .. } => {
            format!("GitHub refused the request (HTTP {status}); check KLURIS_GITHUB_TOKEN.")
        }
        E::ChecksumMismatch { .. } => format!(
            "The download's SHA-256 does not match the release's {SUMS_ASSET} or GitHub's \
             digest ({err}); nothing was replaced."
        ),
        E::ChecksumSourceInvalid { .. } => format!(
            "The download cannot be checked against the release's {SUMS_ASSET} ({err}); \
             nothing was replaced."
        ),
        E::NoReleaseFound { .. } => match release {
            Some((version, want)) => {
                format!("Release v{version} has no {want} for this platform; nothing was replaced.")
            }
            None => "No kluris release was found.".to_string(),
        },
        E::NotFound { .. } => match release {
            Some((version, _)) => format!("There is no release v{version}; nothing was replaced."),
            None => format!("GitHub has no releases for {REPO_OWNER}/{REPO_NAME} ({err})."),
        },
        other => match release {
            Some(_) => format!("Self-update failed: {other}"),
            None => format!("Cannot check for a new release: {other}"),
        },
    };
    CliError::new(message)
}

/// Both paths name the same file.
fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn print_check(latest: &str, available: bool, as_json: bool) {
    if as_json {
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("current".into(), Json::Str(CURRENT.into())),
            ("latest".into(), Json::Str(latest.into())),
            ("update_available".into(), Json::Bool(available)),
        ])));
    } else if available {
        human::print(&format!(
            "kluris {CURRENT} is installed; the newest release is {latest}. \
             Run `kluris self-update` to install it.\n"
        ));
    } else {
        human::print(&format!(
            "kluris {CURRENT} is up to date (newest release: {latest}).\n"
        ));
    }
}

fn print_up_to_date(latest: &str, as_json: bool) {
    if as_json {
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("current".into(), Json::Str(CURRENT.into())),
            ("latest".into(), Json::Str(latest.into())),
            ("updated".into(), Json::Bool(false)),
        ])));
    } else {
        human::print(&format!(
            "kluris {CURRENT} is up to date (newest release: {latest}).\n"
        ));
    }
}

/// Run the new binary's `doctor` as a fresh process and pass on its exit
/// code. Under `--json` its JSON line is carried in this command's.
fn run_doctor(installed: &Path, new_version: &str, as_json: bool) -> Result<(), CliError> {
    let mut cmd = Command::new(installed);
    cmd.arg("doctor");
    let started = if as_json {
        cmd.arg("--json")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
    } else {
        human::print(&format!(
            "Updated kluris {CURRENT} -> {new_version}. Running `kluris doctor`:\n"
        ));
        cmd.status().map(|status| std::process::Output {
            status,
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    };
    let output = started.map_err(|e| {
        CliError::new(format!(
            "kluris {new_version} is installed, but `{} doctor` did not start: {e}",
            installed.display()
        ))
    })?;
    let code = output
        .status
        .code()
        .and_then(|c| u8::try_from(c).ok())
        .unwrap_or(1);
    if as_json {
        let text = String::from_utf8_lossy(&output.stdout);
        let doctor = serde_json::from_str::<serde_json::Value>(text.trim())
            .map(|v| from_serde(&v))
            .unwrap_or_else(|_| Json::Str(text.into_owned()));
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(code == 0)),
            ("current".into(), Json::Str(CURRENT.into())),
            ("latest".into(), Json::Str(new_version.into())),
            ("updated".into(), Json::Bool(true)),
            ("doctor".into(), doctor),
        ])));
    }
    if code == 0 {
        Ok(())
    } else {
        Err(CliError::exit_code(code))
    }
}

/// A `serde_json` value as Kluris's JSON writer's.
fn from_serde(value: &serde_json::Value) -> Json {
    use serde_json::Value as V;
    match value {
        V::Null => Json::Null,
        V::Bool(b) => Json::Bool(*b),
        V::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Json::Int(i),
            (None, _) if n.is_u64() => Json::BigInt(n.to_string()),
            (None, Some(f)) => Json::Float(f),
            (None, None) => Json::BigInt(n.to_string()),
        },
        V::String(s) => Json::Str(s.clone()),
        V::Array(items) => Json::Array(items.iter().map(from_serde).collect()),
        V::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), from_serde(v)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::is_http_loopback;

    #[test]
    fn loopback_is_the_host_not_a_prefix() {
        for yes in [
            "http://127.0.0.1",
            "http://127.0.0.1:8080/api",
            "HTTP://LOCALHOST:1",
            "http://[::1]:9/x",
        ] {
            assert!(is_http_loopback(yes), "{yes}");
        }
        for no in [
            "http://localhost.evil.example",
            "http://127.0.0.1.evil.example/",
            "http://user@127.0.0.1",
            "https://127.0.0.1",
            "http://example.com",
            "http:/127.0.0.1",
        ] {
            assert!(!is_http_loopback(no), "{no}");
        }
    }
}
