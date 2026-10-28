//! The URL git writes in `FETCH_HEAD`, against real git: for each form of a
//! remote's URL, a plain `git fetch` writes the line, and Kluris's
//! [`fetch_head_url`] of the URL `git remote get-url` prints is what that
//! line names; Kluris's own fetch finds its line by it.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use kluris_core::git::Git;
use kluris_core::sync::plumbing::{Fetch, SyncGit, Tracking, fetch_head_url};

mod common;

use common::*;

/// What `git fetch --refmap= origin refs/heads/main` wrote in `FETCH_HEAD`
/// after `branch 'main' of `, and what Kluris computes from `git remote
/// get-url origin`.
fn written_and_computed(w: &World) -> (String, String) {
    let _ = fs::remove_file(w.ours.join(".git").join("FETCH_HEAD"));
    w.run(
        &w.ours,
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            "--write-fetch-head",
            "--refmap=",
            "origin",
            "refs/heads/main",
        ],
    );
    let text = fs::read_to_string(w.ours.join(".git").join("FETCH_HEAD")).unwrap();
    let line = text.lines().next().unwrap_or_default();
    let written = line
        .splitn(3, '\t')
        .nth(2)
        .and_then(|what| what.strip_prefix("branch 'main' of "))
        .unwrap_or_else(|| panic!("{text}"))
        .to_string();
    let url = w.run(&w.ours, &["remote", "get-url", "origin"]);
    (written, fetch_head_url(url.trim_end_matches('\n')))
}

/// Kluris's own fetch of origin's `main`.
fn kluris_fetch(w: &World) -> Fetch {
    let git = w.git();
    let repo = git.repo(&w.ours).unwrap();
    let tracking = Tracking {
        remote: "origin".into(),
        refname: "refs/remotes/origin/main".into(),
        remote_ref: "refs/heads/main".into(),
    };
    git.fetch(&repo, &tracking, Instant::now() + Duration::from_secs(60))
        .unwrap_or_else(|e| panic!("{e:?}"))
}

/// Origin set to `url`: git's line names exactly what Kluris computes, and
/// Kluris's fetch finds the remote's commit by it.
fn assert_url(w: &World, url: &str) {
    w.run(&w.ours, &["remote", "set-url", "origin", url]);
    let (written, computed) = written_and_computed(w);
    assert_eq!(computed, written, "{url}");
    assert_eq!(kluris_fetch(w), Fetch::Commit(w.remote_head()), "{url}");
}

fn slashed(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

#[test]
fn a_plain_path_with_and_without_git_and_a_trailing_slash() {
    let w = World::new();
    let path = w.remote.to_string_lossy().into_owned();
    assert_url(&w, &path);
    assert_url(&w, &format!("{path}/"));
    assert_url(&w, &format!("{path}//"));
    let plain = w.dir.path().join("plain");
    w.run(
        w.dir.path(),
        &["clone", "-q", "--bare", &path, &plain.to_string_lossy()],
    );
    let plain = plain.to_string_lossy().into_owned();
    assert_url(&w, &plain);
    assert_url(&w, &format!("{plain}/"));
}

#[test]
fn a_relative_path() {
    let w = World::new();
    assert_url(&w, "../remote.git");
    assert_url(&w, "../remote.git/");
}

#[test]
fn a_file_url() {
    let w = World::new();
    let path = slashed(&w.remote);
    let url = if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    };
    assert_url(&w, &url);
    assert_url(&w, &format!("{url}/"));
}

/// An ssh "server": `core.sshCommand` runs this script, which serves the
/// command git asks for (`git-upload-pack '<path>'`) from the test's folder.
fn fake_ssh(w: &World) {
    let script = w.dir.path().join("fake-ssh.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nshift\ncd '{}' || exit 1\neval \"set -- ${{1#git-upload-pack }}\"\ncase $1 in /*) set -- \".$1\" ;; esac\nexec git upload-pack \"$1\"\n",
            slashed(w.dir.path())
        ),
    )
    .unwrap();
    w.run(
        &w.ours,
        &[
            "config",
            "core.sshCommand",
            &format!("sh '{}'", slashed(&script)),
        ],
    );
    w.run(&w.ours, &["config", "ssh.variant", "simple"]);
}

#[test]
fn ssh_and_scp_like_urls_lose_their_user() {
    let w = World::new();
    fake_ssh(&w);
    assert_url(&w, "ssh://git@example.invalid/remote.git");
    assert_url(&w, "ssh://git@example.invalid/remote.git/");
    assert_url(&w, "git@example.invalid:remote.git");
}

/// A smart-http server for `root`'s repositories on a loopback port, with
/// `git upload-pack --stateless-rpc`, for this test's life.
fn serve_http(w: &World) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let root = w.dir.path().to_path_buf();
    // The hermetic environment `World` gives its git.
    let envs: Vec<_> = w
        .command(&root, &[])
        .get_envs()
        .map(|(k, v)| (k.to_os_string(), v.map(|v| v.to_os_string())))
        .collect();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = answer(stream, &root, &envs);
        }
    });
    port
}

fn pkt(text: &str) -> Vec<u8> {
    let mut out = format!("{:04x}", text.len() + 4).into_bytes();
    out.extend_from_slice(text.as_bytes());
    out
}

type Envs = [(std::ffi::OsString, Option<std::ffi::OsString>)];

fn answer(stream: TcpStream, root: &Path, envs: &Envs) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let mut parts = request.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let path = target.split('?').next().unwrap_or("");
    let upload_pack = |args: &[&str], input: &[u8]| -> Vec<u8> {
        let mut cmd = std::process::Command::new("git");
        for (key, value) in envs {
            match value {
                Some(value) => cmd.env(key, value),
                None => cmd.env_remove(key),
            };
        }
        let mut child = cmd
            .arg("upload-pack")
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap().stdout
    };
    let (kind, out) = if method == "GET" {
        let repo = path.trim_start_matches('/').trim_end_matches("/info/refs");
        let mut out = pkt("# service=git-upload-pack\n");
        out.extend_from_slice(b"0000");
        out.extend(upload_pack(
            &["--stateless-rpc", "--advertise-refs", repo],
            b"",
        ));
        ("advertisement", out)
    } else {
        let repo = path
            .trim_start_matches('/')
            .trim_end_matches("/git-upload-pack");
        ("result", upload_pack(&["--stateless-rpc", repo], &body))
    };
    let mut stream = stream;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-git-upload-pack-{kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        out.len()
    )?;
    stream.write_all(&out)?;
    stream.flush()
}

#[test]
fn an_http_url_loses_its_credentials() {
    let w = World::new();
    let port = serve_http(&w);
    assert_url(
        &w,
        &format!("http://user:token@127.0.0.1:{port}/remote.git/"),
    );
    assert_url(&w, &format!("http://127.0.0.1:{port}/remote.git"));
}

#[test]
fn an_insteadof_rewrite() {
    let w = World::new();
    let base = format!("{}/", slashed(w.dir.path()));
    w.run(
        &w.ours,
        &["config", &format!("url.{base}.insteadOf"), "brains:"],
    );
    assert_url(&w, "brains:remote.git");
}
