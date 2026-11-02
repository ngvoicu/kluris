//! The brain `README.md` earlier releases wrote at `kluris create`, told
//! apart from one a person wrote so housekeeping can bring it to the
//! current render ([`refreshed`]).
//!
//! A release's README is its template with the brain's `name` and
//! `description` filled in, one pass: `# {name}`, `> {description}`, then
//! `{name}` wherever else it is used. [`ERAS`] holds one [`Era`] per
//! template a release shipped from 2.16.1 on (a README before it listed
//! the brain's lobes, so no template recognises it), generated from each
//! release's own template; frozen, do not edit. A release that ships a
//! changed template adds its row last, and its template to the fixtures.

use sha2::{Digest, Sha256};

use crate::agents::render::lf;
use crate::frontmatter::{LineEnding, line_ending};
use crate::scaffold::readme;

/// Where a brain keeps its README.
pub const README: &str = "README.md";

/// The bytes housekeeping writes over `committed`, the README as HEAD
/// holds it, for the brain `name` with `description` (from its
/// `kluris.yml`): the current render, in `committed`'s line endings, when
/// `committed` is exactly what a release rendered for this brain (line
/// endings aside) and is not that render already. `None` for anything
/// else: a README anyone edited, by even a byte, is never touched.
pub fn refreshed(committed: &[u8], name: &str, description: &str) -> Option<Vec<u8>> {
    let committed = std::str::from_utf8(committed).ok()?;
    let text = lf(committed);
    if !ERAS
        .iter()
        .any(|era| era.rendered(&text, name, description))
    {
        return None;
    }
    let current = readme(name, description);
    if current == text {
        return None;
    }
    Some(match line_ending(committed) {
        LineEnding::Lf => current.into_bytes(),
        LineEnding::Crlf => current.replace('\n', "\r\n").into_bytes(),
    })
}

impl Era {
    /// Whether `text` (LF) is this template filled in for `name` and
    /// `description`: each run of text where it falls, the values in the
    /// slots between them, and the template those runs make its own.
    fn rendered(&self, text: &str, name: &str, description: &str) -> bool {
        let mut rest = text.as_bytes();
        let mut template = Vec::with_capacity(text.len());
        for (at, &run) in self.runs.iter().enumerate() {
            let Some((head, tail)) = rest.split_at_checked(run) else {
                return false;
            };
            template.extend_from_slice(head);
            rest = tail;
            if at + 1 == self.runs.len() {
                break;
            }
            let (value, slot) = if at == 1 {
                (description, "{description}")
            } else {
                (name, "{name}")
            };
            let Some(tail) = rest.strip_prefix(value.as_bytes()) else {
                return false;
            };
            template.extend_from_slice(slot.as_bytes());
            rest = tail;
        }
        rest.is_empty() && hex(&Sha256::digest(&template)) == self.sha256
    }
}

/// `bytes` as lowercase hexadecimal.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One template a group of releases shipped: what a render of it is once
/// `name` and `description` are taken back out.
struct Era {
    /// The releases, `first..=last`.
    #[allow(dead_code)]
    releases: &'static str,
    /// The SHA-256 of the template, its slots written `{name}` and
    /// `{description}`: from 3.0.0-rc.1 on, the `sha256sum` of `brain_readme.md`.
    sha256: &'static str,
    /// The byte lengths of the template's runs of text around its slots,
    /// in order: one more than the slots.
    runs: &'static [usize],
}

/// The releases' templates, oldest first.
const ERAS: &[Era] = &[
    Era {
        releases: "2.16.1..=2.19.0",
        sha256: "25d42fa95720945c6e34a0d88575e20fb515d35509f8810473b5d33932e7e307",
        runs: &[
            2, 4, 698, 47, 520, 69, 57, 463, 11, 506, 346, 63, 244, 21, 31, 105, 52, 31, 29, 47,
            41, 55, 106, 224, 61, 78, 237, 53, 59, 59, 325, 51, 35, 53, 611, 60, 62, 66, 189, 62,
            58, 54, 207, 77, 80, 94, 99, 319, 55, 258, 97, 43, 47, 2414,
        ],
    },
    Era {
        releases: "2.20.0",
        sha256: "4e7b0e110216f8ecd637652b008d02314663400aec6b0a8d91346cf44b757a7f",
        runs: &[
            2, 4, 698, 47, 520, 69, 57, 463, 11, 687, 63, 244, 21, 31, 105, 52, 31, 29, 47, 41, 55,
            106, 224, 61, 78, 237, 53, 59, 59, 325, 51, 35, 53, 611, 60, 62, 66, 189, 62, 58, 54,
            185, 385, 351, 75, 183, 41, 153, 77, 80, 94, 99, 319, 55, 258, 97, 43, 47, 2414,
        ],
    },
    Era {
        releases: "2.20.1..=2.20.2",
        sha256: "58cbb90cb6517ce6a96653a60cccf16d2e8f8137a25f83a46f7ac14c3cf2e452",
        runs: &[
            2, 4, 698, 47, 520, 69, 57, 463, 11, 687, 63, 244, 21, 31, 105, 52, 31, 29, 47, 41, 55,
            106, 224, 61, 78, 237, 53, 59, 59, 325, 51, 35, 53, 611, 60, 62, 66, 189, 62, 58, 54,
            185, 385, 488, 75, 156, 77, 80, 94, 99, 319, 55, 258, 97, 43, 47, 2414,
        ],
    },
    Era {
        releases: "2.21.0..=2.28.9",
        sha256: "56e515ed6e9b223a282e4d859bf5c76f6215fdda20be615fb641a616d77c221a",
        runs: &[
            2, 4, 695, 46, 60, 48, 23, 359, 69, 57, 462, 10, 187, 42, 40, 571, 63, 244, 21, 31,
            105, 52, 31, 29, 47, 41, 55, 106, 224, 61, 78, 237, 53, 59, 59, 325, 51, 35, 53, 611,
            60, 62, 66, 189, 62, 58, 54, 185, 385, 488, 75, 156, 77, 80, 94, 99, 319, 55, 258, 97,
            43, 47, 2414,
        ],
    },
    Era {
        releases: "2.28.10..=3.0.0-rc.1",
        sha256: "fa23739cd03db58c50152eefaa665882b45cbb33a36d0aaa39adaa6c18158aa1",
        runs: &[
            2, 4, 713, 46, 60, 48, 23, 359, 69, 57, 462, 10, 187, 42, 40, 571, 63, 244, 21, 31,
            105, 52, 31, 29, 47, 41, 55, 106, 224, 61, 78, 237, 53, 59, 59, 325, 51, 35, 53, 611,
            60, 62, 66, 189, 62, 58, 54, 185, 385, 488, 75, 156, 77, 80, 94, 99, 319, 55, 258, 97,
            43, 47, 2414,
        ],
    },
    Era {
        releases: "3.0.0",
        sha256: "07bcbe292c3639ee7b36896e8b22b71a2ba5d19078a0c844d3e65bd7a652669c",
        runs: &[
            2, 4, 998, 46, 60, 48, 23, 359, 69, 57, 462, 10, 194, 42, 40, 571, 63, 244, 21, 31,
            105, 52, 31, 29, 47, 41, 55, 106, 224, 61, 78, 237, 53, 59, 59, 325, 51, 35, 53, 611,
            60, 62, 66, 189, 62, 58, 54, 185, 385, 488, 75, 156, 77, 80, 94, 99, 319, 55, 263, 97,
            43, 47, 2414,
        ],
    },
    Era {
        releases: "3.1.0..=3.1.1",
        sha256: "0f3ca6d10ce3f90009e3f2263e819654cbc4861e44f6a04b896bce5aee9f63f8",
        runs: &[
            2, 4, 998, 46, 60, 48, 23, 359, 69, 57, 462, 10, 199, 42, 40, 571, 63, 244, 21, 31,
            105, 52, 31, 29, 47, 41, 55, 106, 224, 61, 78, 237, 53, 59, 59, 325, 51, 35, 53, 611,
            60, 62, 66, 189, 62, 58, 54, 185, 385, 488, 75, 156, 77, 80, 94, 99, 319, 55, 263, 97,
            43, 47, 2414,
        ],
    },
    Era {
        releases: "3.2.0",
        sha256: "9f8694ad57d5bf53e9f06bb9be649d736705a297e5f79f4bc11b1607291c9def",
        runs: &[
            2, 4, 1115, 46, 60, 48, 23, 359, 69, 57, 462, 10, 199, 42, 40, 628, 63, 244, 21, 31,
            153, 52, 31, 29, 47, 41, 55, 106, 224, 61, 78, 237, 53, 59, 59, 325, 51, 35, 53, 611,
            60, 62, 66, 189, 62, 58, 54, 806, 525, 186, 77, 385, 488, 75, 156, 77, 80, 94, 99, 221,
            103, 99, 162, 55, 263, 97, 43, 47, 2417,
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scaffold::fill as render;
    use std::path::PathBuf;

    /// A frozen template, `tests/fixtures/brain-readme/<release>.md`.
    fn template(release: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/brain-readme")
            .join(format!("{release}.md"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// The last release of each era, the name of its fixture.
    fn last(era: &Era) -> &'static str {
        era.releases.rsplit("..=").next().unwrap_or(era.releases)
    }

    /// Names and descriptions a render must be told by: one the template's
    /// own text holds, one with a slot's spelling, one not ASCII.
    const BRAINS: [(&str, &str); 4] = [
        ("alpha", "d"),
        ("kluris", "The kluris brain"),
        ("brain", "Holds {name} and {description}"),
        ("acme", "Acme \"platform\" — ünïcode"),
    ];

    #[test]
    fn every_eras_render_becomes_the_current_one() {
        for era in ERAS {
            let template = template(last(era));
            for (name, description) in BRAINS {
                let old = render(&template, name, description);
                let new = readme(name, description);
                let got = refreshed(old.as_bytes(), name, description);
                if old == new {
                    assert_eq!(got, None, "{}: already current", era.releases);
                } else {
                    assert_eq!(got, Some(new.into_bytes()), "{} {name}", era.releases);
                }
            }
        }
    }

    #[test]
    fn a_crlf_render_is_rewritten_crlf() {
        let old = render(&template("3.1.1"), "alpha", "d").replace('\n', "\r\n");
        let new = readme("alpha", "d").replace('\n', "\r\n");
        assert_eq!(
            refreshed(old.as_bytes(), "alpha", "d"),
            Some(new.clone().into_bytes())
        );
        assert_eq!(refreshed(new.as_bytes(), "alpha", "d"), None);
    }

    #[test]
    fn the_current_render_is_left_as_it_is() {
        for (name, description) in BRAINS {
            assert_eq!(
                refreshed(readme(name, description).as_bytes(), name, description),
                None
            );
        }
    }

    #[test]
    fn a_readme_anyone_edited_is_never_touched() {
        let old = render(&template("2.28.9"), "alpha", "d");
        // A byte added, removed or changed anywhere, at either end too.
        let mut edits = vec![format!("{old}x"), format!("x{old}"), old[1..].to_string()];
        edits.push(old[..old.len() - 1].to_string());
        edits.push(old.replacen("Quick start", "Quick Start", 1));
        edits.push(format!("\u{feff}{old}"));
        for text in edits {
            assert_eq!(refreshed(text.as_bytes(), "alpha", "d"), None);
        }
        // Another brain's render, and bytes that are not UTF-8.
        assert_eq!(refreshed(old.as_bytes(), "beta", "d"), None);
        assert_eq!(refreshed(old.as_bytes(), "alpha", "e"), None);
        let mut bytes = old.into_bytes();
        bytes.push(0xff);
        assert_eq!(refreshed(&bytes, "alpha", "d"), None);
    }

    /// An era's `sha256` and `runs` for `template`.
    fn row(template: &str) -> (String, Vec<usize>) {
        let sha256 = hex(&Sha256::digest(template.as_bytes()));
        let mut runs = vec![0];
        let mut rest = template;
        while let Some(at) = rest.find('{') {
            let tail = &rest[at..];
            let slot = ["{name}", "{description}"]
                .into_iter()
                .find(|slot| tail.starts_with(slot));
            let last = runs.len() - 1;
            match slot {
                Some(slot) => {
                    runs[last] += at;
                    runs.push(0);
                    rest = &tail[slot.len()..];
                }
                None => {
                    runs[last] += at + 1;
                    rest = &tail[1..];
                }
            }
        }
        let last = runs.len() - 1;
        runs[last] += rest.len();
        (sha256, runs)
    }

    #[test]
    fn every_era_is_its_fixtures_template() {
        for era in ERAS {
            let (sha256, runs) = row(&template(last(era)));
            assert_eq!((era.sha256, era.runs), (sha256.as_str(), &runs[..]));
        }
    }

    /// The current template is the last era's, so a release that changes
    /// it cannot ship without its row (the failure prints it) and fixture.
    #[test]
    fn the_current_template_is_the_last_era() {
        let (sha256, runs) = row(include_str!("../../templates/brain_readme.md"));
        let last = ERAS.last().map(|era| (era.sha256, era.runs));
        assert_eq!(
            last,
            Some((sha256.as_str(), &runs[..])),
            "add the row sha256: \"{sha256}\", runs: &{runs:?}"
        );
    }
}
