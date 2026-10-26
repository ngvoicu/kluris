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

/// What the unpack, the sweep and uv's download may take besides the steps.
const MARGIN_MINUTES: u32 = 20;

pub fn steps(parity: bool) -> Vec<Step> {
    let mut steps = vec![TOOLCHAIN, CARGO_TEST];
    if parity {
        steps.extend([RELEASE_BUILD, PARITY]);
    }
    steps
}

/// How long the whole run may take, as this side waits for it.
pub fn bound(parity: bool) -> Duration {
    let minutes: u32 = steps(parity).iter().map(|step| step.minutes).sum::<u32>() + MARGIN_MINUTES;
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
