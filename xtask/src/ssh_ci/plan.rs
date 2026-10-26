//! What a run is, whichever machine it goes to: its id, its steps and their
//! bounds, the files beside the tree in the tarball, and the templates the
//! scripts are filled from.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

/// Kluris's own folder under the home there: `runs/<id>`, `cache/`, `tools/`.
pub const CI_DIR: &str = ".kluris-ci";
/// The commit the tarball was made from, at the root of the tarball.
pub const TREE_FILE: &str = ".kluris-tree";

/// uv, fetched into Kluris's tools folder when the machine has none there.
pub const UV_VERSION: &str = "0.12.8";
/// Where the site wrapper `--wrapper` names goes in the tarball.
pub const WRAPPER_DIR: &str = ".kluris-site";
/// The Python the installer smoke's local release server runs on (uv's).
pub const SMOKE_PYTHON: &str = "3.11";

/// One program a run starts, with its bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub name: &'static str,
    pub minutes: u32,
}

pub const TOOLCHAIN: Step = Step {
    name: "toolchain",
    minutes: 30,
};
pub const CARGO_TEST: Step = Step {
    name: "cargo-test",
    minutes: 90,
};
pub const RELEASE_BUILD: Step = Step {
    name: "release-build",
    minutes: 60,
};
pub const PARITY: Step = Step {
    name: "parity",
    minutes: 90,
};
/// uv's Python for the installer smoke's local server.
pub const PYTHON: Step = Step {
    name: "python",
    minutes: 10,
};
/// dist's release of this machine's target (scripts/release/build-local).
pub const RELEASE_DIST: Step = Step {
    name: "release-dist",
    minutes: 60,
};
/// scripts/release/installer-smoke against that release.
pub const INSTALLER_SMOKE: Step = Step {
    name: "installer-smoke",
    minutes: 20,
};
/// `cargo clippy --workspace --all-targets -- -D warnings`, as ci.yml runs it.
pub const CLIPPY: Step = Step {
    name: "clippy",
    minutes: 60,
};
/// The tree built as the two life-cycle releases (scripts/release/build-versions).
pub const RELEASE_VERSIONS: Step = Step {
    name: "release-versions",
    minutes: 60,
};
/// scripts/release/lifecycle-smoke: install the old release, self-update to
/// the new one, self-uninstall.
pub const LIFECYCLE: Step = Step {
    name: "lifecycle",
    minutes: 20,
};
/// scripts/release/published-smoke: a published GitHub release installed
/// through the live site wrapper, checked, and uninstalled.
pub const PUBLISHED_SMOKE: Step = Step {
    name: "published-smoke",
    minutes: 20,
};
/// The two releases the life cycle installs and updates between: any two
/// versions, the second the higher; only the version differs between them.
pub const LIFECYCLE_OLD: &str = "3.0.0";
pub const LIFECYCLE_NEW: &str = "3.0.1";

/// What the unpack, the sweep and uv's download may take besides the steps.
const MARGIN_MINUTES: u32 = 20;

pub fn steps(parity: bool) -> Vec<Step> {
    let mut steps = vec![TOOLCHAIN, CARGO_TEST];
    if parity {
        steps.extend([RELEASE_BUILD, PARITY]);
    }
    steps
}

/// The steps of an installer smoke, which runs no tests.
pub fn smoke_steps() -> Vec<Step> {
    vec![
        TOOLCHAIN,
        PYTHON,
        RELEASE_DIST,
        INSTALLER_SMOKE,
        RELEASE_VERSIONS,
        LIFECYCLE,
    ]
}

/// The steps of a published-release smoke, which builds nothing: with
/// `python`, uv's Python first, for the script's JSON checks.
pub fn published_steps(python: bool) -> Vec<Step> {
    if python {
        vec![TOOLCHAIN, PYTHON, PUBLISHED_SMOKE]
    } else {
        vec![TOOLCHAIN, PUBLISHED_SMOKE]
    }
}

/// How long a published-release smoke may take.
pub fn published_bound(python: bool) -> Duration {
    bound_of(&published_steps(python))
}

/// The steps of a clippy run, which runs no tests.
pub fn clippy_steps() -> Vec<Step> {
    vec![TOOLCHAIN, CLIPPY]
}

/// How long a clippy run may take.
pub fn clippy_bound() -> Duration {
    bound_of(&clippy_steps())
}

/// `cargo clippy`'s arguments, ci.yml's gate.
pub fn clippy_args() -> Vec<String> {
    [
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// How long the whole run may take, as this side waits for it.
pub fn bound(parity: bool) -> Duration {
    bound_of(&steps(parity))
}

/// How long an installer smoke may take.
pub fn smoke_bound() -> Duration {
    bound_of(&smoke_steps())
}

fn bound_of(steps: &[Step]) -> Duration {
    let minutes: u32 = steps.iter().map(|step| step.minutes).sum::<u32>() + MARGIN_MINUTES;
    Duration::from_secs(u64::from(minutes) * 60)
}

/// A run's name: `kluris-`, this process's id and a random part, each as eight
/// hex digits. ConsensFlow's ids are the same without the prefix, so a lock
/// record says whose run holds it.
pub fn run_id() -> String {
    let random = RandomState::new().build_hasher().finish();
    format!(
        "kluris-{:08x}-{:08x}",
        std::process::id(),
        random & 0xffff_ffff
    )
}

/// The tarball's name there.
pub fn tarball_name(id: &str) -> String {
    format!("{id}.tgz")
}

/// The `uv` arguments of the parity step: pytest on the whole suite, or on
/// the files `--parity-files` named (already checked).
pub fn pytest_args(parity_files: &[String]) -> Vec<String> {
    let mut args: Vec<String> = ["run", "--project", "parity", "--locked", "pytest"]
        .map(str::to_owned)
        .to_vec();
    if parity_files.is_empty() {
        args.push("parity".to_owned());
    } else {
        args.extend(parity_files.iter().cloned());
    }
    args.push("-q".to_owned());
    args
}

/// `template` with each `«name»` replaced by its part, in one pass, so that a
/// part holding a mark is only text.
pub fn fill(template: &str, parts: &[(&str, &str)]) -> String {
    let mut text = String::with_capacity(template.len() + 256);
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('«') {
        text.push_str(before);
        let Some((name, tail)) = after.split_once('»') else {
            text.push('«');
            rest = after;
            continue;
        };
        match parts.iter().find(|(part, _)| *part == name) {
            Some((_, value)) => text.push_str(value),
            None => {
                text.push('«');
                text.push_str(name);
                text.push('»');
            }
        }
        rest = tail;
    }
    text.push_str(rest);
    text
}
