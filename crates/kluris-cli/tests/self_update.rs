//! `kluris self-update` against a fake GitHub release API (`wiremock`)
//! serving real `.tar.gz` / `.zip` archives built here, with a temp install
//! receipt and install prefix (never the real ones).
//!
//! Each test copies the built `kluris` into `<home>/.kluris/bin` (the
//! installer's flat layout: the receipt's `install_prefix` is that folder)
//! and runs the copy, so the running binary is the installed one. The
//! release's new binary is `common::Fake`: it answers `--version` as that
//! release would and records how `doctor` was started (a shell script on
//! Unix, the `fake_kluris` example on Windows: build the examples there).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::runtime::Runtime;
use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;

/// The target the binary under test was built for (`build.rs`).
const TARGET: &str = env!("KLURIS_TARGET");
const CURRENT: &str = env!("CARGO_PKG_VERSION");
const RELEASES: &str = "/repos/ngvoicu/kluris/releases";

fn windows() -> bool {
    TARGET.contains("windows")
}

fn exe_name() -> &'static str {
    if windows() { "kluris.exe" } else { "kluris" }
}

fn archive_name(target: &str) -> String {
    if target.contains("windows") {
        format!("kluris-{target}.zip")
    } else {
        format!("kluris-{target}.tar.gz")
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A `.tar.gz` holding `kluris-<target>/<exe>` (mode 755) and a README, as
/// dist builds it.
fn tar_gz(target: &str, exe: &[u8]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    let mut add = |name: String, bytes: &[u8], mode: u32| {
        let mut h = tar::Header::new_gnu();
        h.set_size(bytes.len() as u64);
        h.set_mode(mode);
        h.set_cksum();
        tar.append_data(&mut h, name, bytes).unwrap();
    };
    add(format!("kluris-{target}/kluris"), exe, 0o755);
    add(format!("kluris-{target}/README.md"), b"# kluris\n", 0o644);
    tar.into_inner().unwrap().finish().unwrap()
}

/// A `.zip` as dist 0.33.1 builds it for Windows: flat, `kluris.exe` and the
/// other files at its root, no `kluris-<target>/` folder (that is the tar.gz
/// layout only).
fn zip(_target: &str, exe: &[u8]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    w.start_file("kluris.exe", opts).unwrap();
    w.write_all(exe).unwrap();
    w.start_file("README.md", opts).unwrap();
    w.write_all(b"# kluris\n").unwrap();
    w.finish().unwrap().into_inner()
}

fn archive(target: &str, exe: &[u8]) -> Vec<u8> {
    if target.contains("windows") {
        zip(target, exe)
    } else {
        tar_gz(target, exe)
    }
}

/// One asset of a fake release.
struct Asset {
    name: String,
    bytes: Vec<u8>,
    /// GitHub's `digest` field; `None` leaves it out.
    digest: Option<String>,
}

/// A fake release: its tag and assets, in the API's order.
struct Release {
    tag: String,
    assets: Vec<Asset>,
}

/// What a test run looks like: a temp HOME with the installed copy and its
/// receipt, and the fake API.
struct Fixture {
    rt: Runtime,
    server: MockServer,
    _root: TempDir,
    home: PathBuf,
    config_home: PathBuf,
    prefix: PathBuf,
    exe: PathBuf,
    receipt: PathBuf,
    receipt_text: String,
    /// The bytes of the installed copy before the run.
    old_exe: Vec<u8>,
    /// Where the fake new binary records its arguments (Unix).
    args_file: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let server = rt.block_on(MockServer::start());
        let root = TempDir::new().unwrap();
        let root_path = fs::canonicalize(root.path()).unwrap();
        let home = root_path.join("home");
        let config_home = root_path.join("config");
        let prefix = home.join(".kluris").join("bin");
        fs::create_dir_all(&prefix).unwrap();
        fs::create_dir_all(config_home.join("kluris")).unwrap();
        let exe = prefix.join(exe_name());
        fs::copy(assert_cmd::cargo::cargo_bin!("kluris"), &exe).unwrap();
        let old_exe = fs::read(&exe).unwrap();
        let receipt = config_home.join("kluris").join("kluris-receipt.json");
        // dist 0.33.1's receipt, as its installer writes it (one line).
        let receipt_text = format!(
            "{{\"binaries\":[\"{bin}\"],\"binary_aliases\":{{}},\"cdylibs\":[],\"cstaticlibs\":[],\
             \"install_layout\":\"flat\",\"install_prefix\":{prefix},\"modify_path\":true,\
             \"provider\":{{\"source\":\"cargo-dist\",\"version\":\"0.33.1\"}},\
             \"source\":{{\"app_name\":\"kluris\",\"name\":\"kluris\",\"owner\":\"ngvoicu\",\
             \"release_type\":\"github\"}},\"version\":\"{CURRENT}\"}}\n",
            bin = exe_name(),
            prefix = serde_json::to_string(&prefix.display().to_string()).unwrap(),
        );
        fs::write(&receipt, &receipt_text).unwrap();
        let args_file = root_path.join("doctor-args.txt");
        Fixture {
            rt,
            server,
            _root: root,
            home,
            config_home,
            prefix,
            exe,
            receipt,
            receipt_text,
            old_exe,
            args_file,
        }
    }

    /// Release `version`'s new binary: `--version` prints `kluris
    /// <version>`; `doctor` records how it was started, prints `doctor_out`
    /// and exits with `code`.
    fn new_binary(&self, version: &str, code: i32, doctor_out: &str) -> Vec<u8> {
        common::fake_for(version, code, doctor_out, &self.args_file)
    }

    /// A release `tag` whose archive for this target holds `exe`, with a
    /// correct `sha256.sum`, GitHub digests, and decoys a "contains the
    /// target" match would pick first.
    fn release(&self, tag: &str, exe: &[u8]) -> Release {
        let want = archive_name(TARGET);
        let real = archive(TARGET, exe);
        let other_target = if TARGET.contains("x86_64") {
            TARGET.replace("x86_64", "aarch64")
        } else {
            TARGET.replace("aarch64", "x86_64")
        };
        let other = archive(&other_target, b"wrong target");
        let sums = format!(
            "{} *{}\n{} *{}\n\n",
            sha256_hex(&other),
            archive_name(&other_target),
            sha256_hex(&real),
            want
        );
        let decoy = |name: String| Asset {
            digest: None,
            bytes: b"not an archive".to_vec(),
            name,
        };
        Release {
            tag: tag.into(),
            assets: vec![
                decoy(format!("{want}.sha256")),
                decoy(format!("kluris-{TARGET}-debug.tar.gz")),
                Asset {
                    digest: Some(format!("sha256:{}", sha256_hex(&other))),
                    name: archive_name(&other_target),
                    bytes: other,
                },
                Asset {
                    digest: Some(format!("sha256:{}", sha256_hex(&real))),
                    name: want,
                    bytes: real,
                },
                Asset {
                    digest: None,
                    name: "sha256.sum".into(),
                    bytes: sums.into_bytes(),
                },
            ],
        }
    }

    fn release_json(&self, index: usize, release: &Release) -> serde_json::Value {
        let assets: Vec<serde_json::Value> = release
            .assets
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let mut v = serde_json::json!({
                    "name": a.name,
                    "url": format!("{}{RELEASES}/assets/{index}{i}", self.server.uri()),
                    "browser_download_url": format!("https://github.invalid/{}", a.name),
                });
                if let Some(d) = &a.digest {
                    v["digest"] = serde_json::Value::String(d.clone());
                }
                v
            })
            .collect();
        serde_json::json!({
            "tag_name": release.tag,
            "name": release.tag,
            "created_at": "2026-10-26T00:00:00Z",
            "body": "",
            "html_url": format!("https://github.invalid/{}", release.tag),
            "assets": assets,
        })
    }

    /// Serve `releases` (newest first) on the listing, each on its tag URL,
    /// and every asset at its `url`.
    fn serve(&self, releases: &[Release]) {
        let listing: Vec<serde_json::Value> = releases
            .iter()
            .enumerate()
            .map(|(i, r)| self.release_json(i, r))
            .collect();
        self.mount(
            Mock::given(method("GET"))
                .and(path(RELEASES))
                .respond_with(ResponseTemplate::new(200).set_body_json(&listing)),
        );
        for (i, r) in releases.iter().enumerate() {
            self.mount(
                Mock::given(method("GET"))
                    .and(path(format!("{RELEASES}/tags/{}", r.tag)))
                    .respond_with(
                        ResponseTemplate::new(200).set_body_json(self.release_json(i, r)),
                    ),
            );
            for (j, a) in r.assets.iter().enumerate() {
                self.mount(
                    Mock::given(method("GET"))
                        .and(path(format!("{RELEASES}/assets/{i}{j}")))
                        .respond_with(ResponseTemplate::new(200).set_body_bytes(a.bytes.clone())),
                );
            }
        }
    }

    fn mount(&self, mock: Mock) {
        self.rt.block_on(mock.mount(&self.server));
    }

    /// The requests the fake API received, as `METHOD path`.
    fn requests(&self) -> Vec<String> {
        self.rt
            .block_on(self.server.received_requests())
            .unwrap_or_default()
            .iter()
            .map(|r| format!("{} {}", r.method, r.url.path()))
            .collect()
    }

    /// The installed copy, with the API pointed at the fake.
    fn cmd(&self, exe: &Path) -> Command {
        let mut cmd = Command::new(exe);
        cmd.env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("LOCALAPPDATA", self.home.join("AppData").join("Local"))
            .env(
                "KLURIS_CONFIG",
                self.home.join(".kluris").join("config.yml"),
            )
            .env("KLURIS_GITHUB_API_URL", self.server.uri())
            .env("KLURIS_NO_PROMPT", "1")
            .env("NO_COLOR", "1")
            .env("GIT_CEILING_DIRECTORIES", &self.home)
            .env_remove("KLURIS_GITHUB_TOKEN")
            .env_remove("ZDOTDIR")
            .stdin(Stdio::null());
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(&self.exe).args(args).output().unwrap()
    }

    fn assert_untouched(&self) {
        assert!(
            fs::read(&self.exe).unwrap() == self.old_exe,
            "the installed binary changed"
        );
        assert_eq!(
            fs::read_to_string(&self.receipt).unwrap(),
            self.receipt_text
        );
        assert!(!self.args_file.exists(), "doctor ran");
    }

    /// The names in the install folder, sorted.
    fn prefix_names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.prefix)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn asset_requests(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .filter(|r| r.contains("/assets/"))
            .collect()
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn check_json_prints_current_and_latest_and_changes_nothing() {
    let f = Fixture::new();
    let new = f.new_binary("3.99.0", 0, "");
    f.serve(&[f.release("v3.99.0", &new), f.release("v3.98.0", &new)]);
    let out = f.run(&["self-update", "--check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        format!(
            "{{\"ok\": true, \"current\": \"{CURRENT}\", \"latest\": \"3.99.0\", \"update_available\": true}}\n"
        )
    );
    f.assert_untouched();
    assert_eq!(f.asset_requests(), Vec::<String>::new());
}

#[test]
fn check_human_names_both_versions() {
    let f = Fixture::new();
    let new = f.new_binary("3.99.0", 0, "");
    f.serve(&[f.release("v3.99.0", &new)]);
    let out = f.run(&["self-update", "--check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains(CURRENT) && text.contains("3.99.0"), "{text}");
    f.assert_untouched();
}

#[test]
fn check_when_up_to_date() {
    let f = Fixture::new();
    let new = f.new_binary("3.99.0", 0, "");
    f.serve(&[f.release(&format!("v{CURRENT}"), &new)]);
    let out = f.run(&["self-update", "--check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        format!(
            "{{\"ok\": true, \"current\": \"{CURRENT}\", \"latest\": \"{CURRENT}\", \"update_available\": false}}\n"
        )
    );
    f.assert_untouched();
}

#[test]
fn up_to_date_installs_nothing_and_runs_no_doctor() {
    let f = Fixture::new();
    let new = f.new_binary("3.99.0", 0, "");
    f.serve(&[f.release(&format!("v{CURRENT}"), &new)]);
    let out = f.run(&["self-update", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        format!(
            "{{\"ok\": true, \"current\": \"{CURRENT}\", \"latest\": \"{CURRENT}\", \"updated\": false}}\n"
        )
    );
    f.assert_untouched();
    let human = f.run(&["self-update"]);
    assert_eq!(human.status.code(), Some(0));
    assert!(stdout(&human).contains("up to date"), "{}", stdout(&human));
    f.assert_untouched();
}

#[test]
fn update_installs_the_exact_asset_updates_the_receipt_and_runs_doctor() {
    let f = Fixture::new();
    let new = f.new_binary("3.99.0", 7, "doctor ran");
    f.serve(&[f.release("v3.99.0", &new), f.release("v3.98.0", b"older")]);
    let expected_code = 7;
    let out = f.run(&["self-update"]);
    assert_eq!(out.status.code(), Some(expected_code), "{}", stderr(&out));
    assert!(
        fs::read(&f.exe).unwrap() == new,
        "the new binary is in place"
    );
    assert_eq!(
        fs::read_to_string(&f.receipt).unwrap(),
        f.receipt_text.replace(
            &format!("\"version\":\"{CURRENT}\""),
            "\"version\":\"3.99.0\""
        )
    );
    let text = stdout(&out);
    assert!(text.contains(CURRENT) && text.contains("3.99.0"), "{text}");
    assert!(text.contains("doctor ran"), "{text}");
    // A fresh process of the new binary, at the install path.
    assert_eq!(
        fs::read_to_string(&f.args_file).unwrap(),
        format!("{}\ndoctor\n", f.exe.display())
    );
    // Only this target's archive and the sums were downloaded.
    let mut assets = f.asset_requests();
    assets.sort();
    assert_eq!(
        assets,
        vec![
            format!("GET {RELEASES}/assets/03"),
            format!("GET {RELEASES}/assets/04"),
        ]
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&f.exe).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "executable");
    }
    // No leftovers beside the binary: no staged copy, and on Windows the
    // old exe has been moved out to be deleted once the update exits.
    assert_eq!(f.prefix_names(), vec![exe_name().to_string()]);
}

#[test]
fn update_json_carries_doctors_json_and_exit_code() {
    let f = Fixture::new();
    let new = f.new_binary("3.99.0", 1, r#"{"ok": false, "checks": []}"#);
    f.serve(&[f.release("v3.99.0", &new)]);
    let out = f.run(&["self-update", "--json"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        format!(
            "{{\"ok\": false, \"current\": \"{CURRENT}\", \"latest\": \"3.99.0\", \"updated\": true, \
             \"doctor\": {{\"ok\": false, \"checks\": []}}}}\n"
        )
    );
    assert_eq!(
        fs::read_to_string(&f.args_file).unwrap(),
        format!("{}\ndoctor\n--json\n", f.exe.display())
    );
}

#[test]
fn version_flag_installs_that_release() {
    let f = Fixture::new();
    let newest = f.new_binary("3.99.0", 0, "newest");
    let wanted = f.new_binary("3.98.0", 0, "wanted");
    f.serve(&[f.release("v3.99.0", &newest), f.release("v3.98.0", &wanted)]);
    for given in ["3.98.0", "v3.98.0"] {
        fs::write(&f.exe, &f.old_exe).unwrap();
        fs::write(&f.receipt, &f.receipt_text).unwrap();
        let expected_code = 0;
        let out = f.run(&["self-update", "--version", given]);
        assert_eq!(out.status.code(), Some(expected_code), "{}", stderr(&out));
        assert!(fs::read(&f.exe).unwrap() == wanted, "{given}");
        assert!(
            fs::read_to_string(&f.receipt)
                .unwrap()
                .contains("\"version\":\"3.98.0\"")
        );
    }
    assert!(
        f.requests()
            .contains(&format!("GET {RELEASES}/tags/v3.98.0"))
    );
}

#[test]
fn a_sha256_sum_mismatch_replaces_nothing() {
    let f = Fixture::new();
    let new = f.new_binary("3.99.0", 0, "");
    let mut release = f.release("v3.99.0", &new);
    let want = archive_name(TARGET);
    let sums = release
        .assets
        .iter_mut()
        .find(|a| a.name == "sha256.sum")
        .unwrap();
    sums.bytes = format!("{} *{want}\n", "0".repeat(64)).into_bytes();
    f.serve(&[release]);
    let out = f.run(&["self-update"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("SHA-256") && err.contains("nothing was replaced"),
        "{err}"
    );
    f.assert_untouched();
    let json = f.run(&["self-update", "--json"]);
    assert_eq!(json.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("nothing was replaced")
    );
    f.assert_untouched();
}

#[test]
fn a_github_digest_mismatch_replaces_nothing() {
    let f = Fixture::new();
    let new = f.new_binary("3.99.0", 0, "");
    let mut release = f.release("v3.99.0", &new);
    let want = archive_name(TARGET);
    let asset = release.assets.iter_mut().find(|a| a.name == want).unwrap();
    asset.digest = Some(format!("sha256:{}", "1".repeat(64)));
    f.serve(&[release]);
    let out = f.run(&["self-update"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("nothing was replaced"),
        "{}",
        stderr(&out)
    );
    f.assert_untouched();
}

#[test]
fn a_release_without_sha256_sum_is_refused() {
    let f = Fixture::new();
    let new = f.new_binary("3.99.0", 0, "");
    let mut release = f.release("v3.99.0", &new);
    release.assets.retain(|a| a.name != "sha256.sum");
    f.serve(&[release]);
    let out = f.run(&["self-update"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("sha256.sum"), "{}", stderr(&out));
    f.assert_untouched();
}

#[test]
fn a_release_without_this_targets_archive_is_refused() {
    // Named with --version: the listing's candidates never include it.
    let f = Fixture::new();
    let new = f.new_binary("3.99.0", 0, "");
    let mut release = f.release("v3.99.0", &new);
    let want = archive_name(TARGET);
    release.assets.retain(|a| a.name != want);
    f.serve(&[release]);
    let out = f.run(&["self-update", "--version", "3.99.0"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains(&want), "{}", stderr(&out));
    f.assert_untouched();
}

#[test]
fn the_only_release_without_this_targets_archive_means_up_to_date() {
    let f = Fixture::new();
    let new = f.new_binary("3.99.0", 0, "");
    let mut release = f.release("v3.99.0", &new);
    let want = archive_name(TARGET);
    release.assets.retain(|a| a.name != want);
    f.serve(&[release]);
    let out = f.run(&["self-update", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("\"updated\": false"),
        "{}",
        stdout(&out)
    );
    f.assert_untouched();
}

#[test]
fn kluris_github_token_is_sent_when_set_and_only_then() {
    let f = Fixture::new();
    let new = f.new_binary("3.99.0", 0, "");
    // With the token: only a request carrying it is answered.
    f.mount(
        Mock::given(method("GET"))
            .and(path(RELEASES))
            .and(header("authorization", "token kt-123"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(vec![f.release_json(0, &f.release("v3.99.0", &new))]),
            ),
    );
    let out = f
        .cmd(&f.exe)
        .args(["self-update", "--check", "--json"])
        .env("KLURIS_GITHUB_TOKEN", "kt-123")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("\"latest\": \"3.99.0\""));
    // Without it (GITHUB_TOKEN is not Kluris's): no authorization header.
    let out = f
        .cmd(&f.exe)
        .args(["self-update", "--check", "--json"])
        .env("GITHUB_TOKEN", "gh-456")
        .env("GH_TOKEN", "gh-789")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "unanswered without the token");
    let received = f.rt.block_on(f.server.received_requests()).unwrap();
    assert_eq!(received.len(), 2);
    assert_eq!(
        received[0]
            .headers
            .get("authorization")
            .map(|v| v.to_str().unwrap()),
        Some("token kt-123")
    );
    assert!(received[1].headers.get("authorization").is_none());
}

#[test]
fn a_rate_limit_says_to_set_kluris_github_token() {
    for (status, headers) in [
        (
            403,
            vec![("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "1")],
        ),
        (429, vec![]),
    ] {
        let f = Fixture::new();
        let mut resp = ResponseTemplate::new(status)
            .set_body_string(r#"{"message":"API rate limit exceeded"}"#);
        for (k, v) in headers {
            resp = resp.insert_header(k, v);
        }
        f.mount(
            Mock::given(method("GET"))
                .and(path(RELEASES))
                .respond_with(resp),
        );
        for args in [vec!["self-update"], vec!["self-update", "--check"]] {
            let out = f.run(&args);
            assert_eq!(out.status.code(), Some(1));
            let err = stderr(&out);
            assert!(
                err.contains("rate limit") && err.contains("KLURIS_GITHUB_TOKEN"),
                "{status}: {err}"
            );
        }
        let out = f.run(&["self-update", "--json"]);
        assert_eq!(out.status.code(), Some(1));
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(v["error"].as_str().unwrap().contains("KLURIS_GITHUB_TOKEN"));
        f.assert_untouched();
    }
}

#[test]
fn refuses_without_a_receipt() {
    let f = Fixture::new();
    fs::remove_file(&f.receipt).unwrap();
    let out = f.run(&["self-update"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("receipt") && err.contains(&f.receipt.display().to_string()),
        "{err}"
    );
    assert_eq!(f.requests(), Vec::<String>::new());
    assert!(fs::read(&f.exe).unwrap() == f.old_exe);
    let out = f.run(&["self-update", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false);
}

#[test]
fn refuses_when_the_running_binary_is_outside_the_install_prefix() {
    let f = Fixture::new();
    let new = f.new_binary("3.99.0", 0, "");
    f.serve(&[f.release("v3.99.0", &new)]);
    // The cargo build, not the installed copy.
    let out = f
        .cmd(Path::new(assert_cmd::cargo::cargo_bin!("kluris")))
        .arg("self-update")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains(&f.prefix.display().to_string()), "{err}");
    assert!(err.contains("not the installed"), "{err}");
    f.assert_untouched();
    assert_eq!(f.requests(), Vec::<String>::new());
}

#[test]
fn self_update_is_in_help() {
    let f = Fixture::new();
    let out = f.run(&["self-update", "--help"]);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    for flag in [
        "--check",
        "--version",
        "--json",
        "KLURIS_GITHUB_TOKEN",
        "KLURIS_GITHUB_API_URL",
    ] {
        assert!(text.contains(flag), "{text}");
    }
}

#[test]
fn a_plain_http_api_url_is_refused_unless_loopback() {
    // KLURIS_GITHUB_API_URL is for a GitHub API mirror or GitHub Enterprise:
    // https, or plain http only on this machine (the fake above is
    // http://127.0.0.1), with or without a token.
    let f = Fixture::new();
    for token in [None, Some("kt-123")] {
        for json in [false, true] {
            let mut cmd = f.cmd(&f.exe);
            cmd.args(["self-update", "--check"]).env(
                "KLURIS_GITHUB_API_URL",
                "http://github.example.invalid/api/v3",
            );
            if json {
                cmd.arg("--json");
            }
            if let Some(token) = token {
                cmd.env("KLURIS_GITHUB_TOKEN", token);
            }
            let out = cmd.output().unwrap();
            assert_eq!(out.status.code(), Some(1));
            let message = if json {
                let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
                v["error"].as_str().unwrap().to_string()
            } else {
                stderr(&out)
            };
            assert!(
                message.contains("KLURIS_GITHUB_API_URL") && message.contains("https"),
                "{message}"
            );
        }
    }
    let out = f
        .cmd(&f.exe)
        .args(["self-update"])
        .env("KLURIS_GITHUB_API_URL", "http://github.example.invalid")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("KLURIS_GITHUB_API_URL"),
        "{}",
        stderr(&out)
    );
    f.assert_untouched();
    assert_eq!(f.requests(), Vec::<String>::new());
}

/// Install the newest release and check what landed: the binary of
/// `bytes` (that release's binary) and `version` in the receipt.
fn assert_installed(f: &Fixture, args: &[&str], bytes: &[u8], version: &str) {
    let expected_code = 0;
    let out = f.run(args);
    assert_eq!(out.status.code(), Some(expected_code), "{}", stderr(&out));
    assert!(fs::read(&f.exe).unwrap() == bytes, "{version} is in place");
    assert!(
        fs::read_to_string(&f.receipt)
            .unwrap()
            .contains(&format!("\"version\":\"{version}\"")),
        "{version} in the receipt"
    );
}

/// Release `version`'s binary, told apart from the others by `mark`.
fn marked_binary(f: &Fixture, version: &str, mark: &str) -> Vec<u8> {
    f.new_binary(version, 0, mark)
}

#[test]
fn the_newest_release_is_never_a_prerelease() {
    let f = Fixture::new();
    let rc = marked_binary(&f, "3.1.0-rc.1", "rc");
    let stable = marked_binary(&f, "3.0.1", "stable");
    // The candidate is the semver maximum and listed first.
    f.serve(&[f.release("v3.1.0-rc.1", &rc), f.release("v3.0.1", &stable)]);
    let out = f.run(&["self-update", "--check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        format!(
            "{{\"ok\": true, \"current\": \"{CURRENT}\", \"latest\": \"3.0.1\", \"update_available\": true}}\n"
        )
    );
    assert_installed(&f, &["self-update"], &stable, "3.0.1");
    assert!(
        !f.requests()
            .contains(&format!("GET {RELEASES}/tags/v3.1.0-rc.1"))
    );
}

#[test]
fn only_prereleases_newer_means_up_to_date() {
    let f = Fixture::new();
    let rc = marked_binary(&f, "3.1.0-rc.1", "rc");
    f.serve(&[
        f.release("v3.1.0-rc.1", &rc),
        f.release(&format!("v{CURRENT}"), &rc),
    ]);
    let out = f.run(&["self-update", "--check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("\"update_available\": false"),
        "{}",
        stdout(&out)
    );
    let out = f.run(&["self-update", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("\"updated\": false"),
        "{}",
        stdout(&out)
    );
    f.assert_untouched();
}

#[test]
fn a_prerelease_installs_when_named() {
    let f = Fixture::new();
    let rc = marked_binary(&f, "3.1.0-rc.1", "rc");
    let stable = marked_binary(&f, "3.0.1", "stable");
    f.serve(&[f.release("v3.1.0-rc.1", &rc), f.release("v3.0.1", &stable)]);
    assert_installed(
        &f,
        &["self-update", "--version", "3.1.0-rc.1"],
        &rc,
        "3.1.0-rc.1",
    );
}

#[test]
fn a_v2_release_published_after_v3_never_wins() {
    let f = Fixture::new();
    // A v3 release newer than this build (3.0.0 itself would be no update).
    let v3 = marked_binary(&f, "3.0.1", "v3");
    // Newest first, as GitHub lists them: v2.29.0 (no binaries) came later.
    let v2 = Release {
        tag: "v2.29.0".into(),
        assets: Vec::new(),
    };
    f.serve(&[v2, f.release("v3.0.1", &v3)]);
    let out = f.run(&["self-update", "--check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("\"latest\": \"3.0.1\""),
        "{}",
        stdout(&out)
    );
    assert_installed(&f, &["self-update"], &v3, "3.0.1");
}

/// A v2 release as GitHub lists them: Python wheels, no Kluris 3 archive.
fn v2_release(tag: &str) -> Release {
    let version = tag.trim_start_matches('v');
    Release {
        tag: tag.into(),
        assets: vec![
            Asset {
                name: format!("kluris-{version}-py3-none-any.whl"),
                bytes: b"a wheel".to_vec(),
                digest: None,
            },
            Asset {
                name: format!("kluris-{version}.tar.gz"),
                bytes: b"an sdist".to_vec(),
                digest: None,
            },
        ],
    }
}

#[test]
fn only_v2_releases_listed_means_up_to_date_and_latest_is_current() {
    // An rc user while only v2 releases are published: none carries this
    // target's archive, so none is a candidate.
    let f = Fixture::new();
    f.serve(&[v2_release("v2.28.11"), v2_release("v2.28.10")]);
    let out = f.run(&["self-update", "--check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        format!(
            "{{\"ok\": true, \"current\": \"{CURRENT}\", \"latest\": \"{CURRENT}\", \"update_available\": false}}\n"
        )
    );
    let out = f.run(&["self-update", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        format!(
            "{{\"ok\": true, \"current\": \"{CURRENT}\", \"latest\": \"{CURRENT}\", \"updated\": false}}\n"
        )
    );
    f.assert_untouched();
}

#[test]
fn an_older_candidate_than_current_is_not_latest() {
    // The only candidate is older than this binary: latest is current.
    let f = Fixture::new();
    let old = marked_binary(&f, "2.99.0", "old");
    f.serve(&[f.release("v2.99.0", &old)]);
    let out = f.run(&["self-update", "--check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        format!(
            "{{\"ok\": true, \"current\": \"{CURRENT}\", \"latest\": \"{CURRENT}\", \"update_available\": false}}\n"
        )
    );
    f.assert_untouched();
}

#[test]
fn a_newer_release_without_this_targets_archive_is_skipped() {
    // v3.99.1 shipped without this platform: v3.99.0, which has it, wins.
    let f = Fixture::new();
    let newer = marked_binary(&f, "3.99.1", "newer");
    let older = marked_binary(&f, "3.99.0", "older");
    let want = archive_name(TARGET);
    let mut missing = f.release("v3.99.1", &newer);
    missing.assets.retain(|a| a.name != want);
    f.serve(&[missing, f.release("v3.99.0", &older)]);
    let out = f.run(&["self-update", "--check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        format!(
            "{{\"ok\": true, \"current\": \"{CURRENT}\", \"latest\": \"3.99.0\", \"update_available\": true}}\n"
        )
    );
    assert_installed(&f, &["self-update"], &older, "3.99.0");
    assert!(
        !f.requests()
            .contains(&format!("GET {RELEASES}/tags/v3.99.1")),
        "the release without this target's archive is never fetched"
    );
}

#[test]
fn every_page_of_the_listing_is_read() {
    // GitHub pages the listing (100 a page here; the repo has many v2
    // releases): the v3 release on page two must be found.
    let f = Fixture::new();
    let new = marked_binary(&f, "3.99.0", "paged");
    let v2: Vec<Release> = (0..3)
        .map(|i| Release {
            tag: format!("v2.28.{i}"),
            assets: Vec::new(),
        })
        .collect();
    let page1: Vec<serde_json::Value> = v2
        .iter()
        .enumerate()
        .map(|(i, r)| f.release_json(i, r))
        .collect();
    let v3 = f.release("v3.99.0", &new);
    let page2 = vec![f.release_json(5, &v3)];
    let next = format!(
        "<{}{RELEASES}?per_page=100&page=2>; rel=\"next\"",
        f.server.uri()
    );
    f.mount(
        Mock::given(method("GET"))
            .and(path(RELEASES))
            .and(query_param_is_missing("page"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("link", next.as_str())
                    .set_body_json(&page1),
            ),
    );
    f.mount(
        Mock::given(method("GET"))
            .and(path(RELEASES))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&page2)),
    );
    let out = f.run(&["self-update", "--check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("\"latest\": \"3.99.0\""),
        "{}",
        stdout(&out)
    );
    let pages =
        f.rt.block_on(f.server.received_requests())
            .unwrap()
            .iter()
            .filter(|r| r.url.path() == RELEASES)
            .count();
    assert_eq!(pages, 2);
}

/// A release that passes the checksums but whose binary does not run here,
/// exits non-zero for `--version`, or names another version: the installed
/// binary, the receipt and the install folder stay as they were, and no
/// doctor runs.
#[test]
fn an_unusable_release_replaces_nothing() {
    type MakeBinary = Box<dyn Fn(&Fixture) -> Vec<u8>>;
    let cases: Vec<(&str, MakeBinary)> = vec![
        ("wrong format", Box::new(|_| common::wrong_format())),
        (
            "--version exits 1",
            Box::new(|f| {
                common::Fake {
                    version_line: "kluris 3.99.0",
                    version_code: 1,
                    code: 0,
                    out: "",
                    args_file: &f.args_file,
                }
                .bytes()
            }),
        ),
        (
            "another version",
            Box::new(|f| {
                common::Fake {
                    version_line: "kluris 9.9.9",
                    version_code: 0,
                    code: 0,
                    out: "",
                    args_file: &f.args_file,
                }
                .bytes()
            }),
        ),
    ];
    for (case, bytes) in cases {
        let f = Fixture::new();
        f.serve(&[f.release("v3.99.0", &bytes(&f))]);
        for json in [false, true] {
            let mut args = vec!["self-update"];
            if json {
                args.push("--json");
            }
            let out = f.run(&args);
            assert_eq!(out.status.code(), Some(1), "{case}: {}", stdout(&out));
            let message = if json {
                let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
                assert_eq!(v["ok"], false, "{case}");
                v["error"].as_str().unwrap().to_string()
            } else {
                stderr(&out)
            };
            assert!(
                message.contains("3.99.0") && message.contains("nothing was replaced"),
                "{case}: {message}"
            );
            f.assert_untouched();
            assert_eq!(f.prefix_names(), vec![exe_name().to_string()], "{case}");
        }
    }
}

/// The old binary a Windows swap leaves (`.kluris.exe.old-<pid>-<n>`) goes
/// at the start of the next `self-update` and `doctor`; nothing else does.
#[test]
fn stale_old_binaries_are_swept_by_self_update_and_doctor() {
    let stale = [
        format!(".{}.old-123-0", exe_name()),
        format!(".{}.old-9-12", exe_name()),
    ];
    let kept = [
        format!(".{}.old-x-1", exe_name()),
        format!("{}.old-1-2", exe_name()),
        format!(".{}.old-1-2.bak", exe_name()),
        ".notes.old-1-2".to_string(),
    ];
    for args in [
        vec!["self-update", "--check", "--json"],
        vec!["doctor", "--no-refresh", "--json"],
    ] {
        let f = Fixture::new();
        let new = f.new_binary("3.99.0", 0, "");
        f.serve(&[f.release("v3.99.0", &new)]);
        for name in stale.iter().chain(&kept) {
            fs::write(f.prefix.join(name), b"old").unwrap();
        }
        let out = f.run(&args);
        assert!(out.status.code().is_some(), "{args:?}");
        let mut expected: Vec<String> = kept.to_vec();
        expected.push(exe_name().to_string());
        expected.sort();
        assert_eq!(f.prefix_names(), expected, "{args:?}: {}", stderr(&out));
    }
}

/// An interrupted update's staged copy (`.kluris(.exe).new-<pid>-<n>`) goes
/// at the start of `self-update` and `doctor` once it is over an hour old:
/// a fresh one may be another update's, running now. Look-alike names and
/// symlinks are never touched.
#[test]
fn interrupted_staging_files_are_swept_once_an_hour_old() {
    let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 3600);
    let old_staged = format!(".{}.new-123-0", exe_name());
    let fresh_staged = format!(".{}.new-456-1", exe_name());
    let look_alikes = [
        format!(".{}.new-x-1", exe_name()),
        format!("{}.new-1-2", exe_name()),
        format!(".{}.new-1-2.part", exe_name()),
        ".notes.new-1-2".to_string(),
    ];
    for args in [
        vec!["self-update", "--check", "--json"],
        vec!["doctor", "--no-refresh", "--json"],
    ] {
        let f = Fixture::new();
        let new = f.new_binary("3.99.0", 0, "");
        f.serve(&[f.release("v3.99.0", &new)]);
        let write = |name: &str, old: bool| {
            let path = f.prefix.join(name);
            fs::write(&path, b"half a binary").unwrap();
            if old {
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(two_hours_ago)
                    .unwrap();
            }
        };
        write(&old_staged, true);
        write(&fresh_staged, false);
        for name in &look_alikes {
            write(name, true);
        }
        let mut kept: Vec<String> = look_alikes.to_vec();
        kept.push(fresh_staged.clone());
        // A symlink with the staged name, to an hour-old file outside.
        #[cfg(unix)]
        {
            let target = f.home.join("target-of-link");
            fs::write(&target, b"not ours").unwrap();
            fs::File::options()
                .write(true)
                .open(&target)
                .unwrap()
                .set_modified(two_hours_ago)
                .unwrap();
            let link = format!(".{}.new-789-2", exe_name());
            std::os::unix::fs::symlink(&target, f.prefix.join(&link)).unwrap();
            kept.push(link);
        }
        let out = f.run(&args);
        assert!(out.status.code().is_some(), "{args:?}");
        kept.push(exe_name().to_string());
        kept.sort();
        assert_eq!(f.prefix_names(), kept, "{args:?}: {}", stderr(&out));
        #[cfg(unix)]
        assert_eq!(
            fs::read(f.home.join("target-of-link")).unwrap(),
            b"not ours"
        );
    }
}
