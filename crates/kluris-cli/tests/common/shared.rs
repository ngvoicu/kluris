//! Files a golden snapshot shows by reference instead of by their bytes, so
//! that a file many scenarios write is held once:
//! - a runtime companion file that holds its bytes from `vendored/` reads
//!   `<vendored/NAME/PATH>`;
//! - a brain's SKILL.md that Kluris generated (it holds the
//!   `<!-- kluris:generated` line) reads `<snapshot LABEL>`, its text
//!   checked against the shared snapshot `LABEL` (one file for every golden
//!   test, named without the test's module), where `LABEL` is
//!   `skill_<brain>_<companions>` and the test's variant, if any, after it.
//!   That snapshot ends with an `<end of file>` line, since a snapshot's
//!   trailing whitespace is not compared and a skill ends in a blank line.

use std::fs;
use std::path::{Path, PathBuf};

use super::golden::{file_body, normalize};

/// The companions Kluris ships, in its own order.
pub const COMPANIONS: [&str; 4] = [
    "specmint-core",
    "specmint-tdd",
    "specmint-core-html",
    "specmint-tdd-html",
];

/// The workspace's `vendored/` folder, where the companions come from.
pub fn vendored() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendored")
}

/// `<vendored/NAME/PATH>` for a file at `home/.kluris/companions/NAME/PATH`
/// (relative to `work/`) whose bytes are `vendored/NAME/PATH`'s.
pub fn vendored_marker(path: &str, bytes: &[u8]) -> Option<String> {
    let rel = path.strip_prefix("home/.kluris/companions/")?;
    let vendored = fs::read(vendored().join(rel)).ok()?;
    (vendored == bytes).then(|| format!("<vendored/{rel}>\n"))
}

/// The brain of `home/<folder>/kluris-<brain>/SKILL.md`.
pub fn skill_brain(path: &str) -> Option<&str> {
    let dir = path.strip_prefix("home/")?.strip_suffix("/SKILL.md")?;
    dir.rsplit('/').next()?.strip_prefix("kluris-")
}

/// `<snapshot LABEL>` for the generated brain SKILL.md at `path` (relative
/// to `work/`), after asserting that the shared snapshot `LABEL` holds its
/// text, normalized against `root`; `None` for any other file. `variant`
/// gets that text and names what tells it from the other texts of the same
/// brain and companions (empty when nothing does). On Windows a text that
/// holds a path under the root (`<ROOT>\`) is the `windows` variant.
pub fn skill_reference(
    path: &str,
    bytes: &[u8],
    root: &Path,
    variant: impl FnOnce(&str) -> String,
) -> Option<String> {
    let brain = skill_brain(path)?;
    let text = normalize(&file_body(bytes), root);
    if !text.contains("\n<!-- kluris:generated") {
        return None;
    }
    let companions: Vec<&str> = COMPANIONS
        .into_iter()
        .filter(|c| text.contains(&format!("/{c}/SKILL.md")))
        .collect();
    let companions = if companions.is_empty() {
        "none".to_string()
    } else {
        companions.join("+")
    };
    let mut label = format!("skill_{brain}_{companions}");
    let variant = variant(&text);
    if !variant.is_empty() {
        label = format!("{label}_{variant}");
    }
    let mut settings = insta::Settings::clone_current();
    settings.set_prepend_module_to_snapshot(false);
    settings.set_snapshot_path(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots"));
    if cfg!(windows) && text.contains("<ROOT>\\") {
        settings.set_snapshot_suffix("windows");
    }
    let held = format!("{text}<end of file>\n");
    settings.bind(|| insta::assert_snapshot!(label.as_str(), held.as_str()));
    Some(format!("<snapshot {label}>\n"))
}
