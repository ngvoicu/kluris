//! The commands Kluris prints for settling a waiting merge, one per line,
//! each complete and to be run as it is printed (POSIX shell quoting):
//! `kluris commit`'s refusals ([`MergeNeeds`]), `kluris resolve`'s and
//! `kluris sync`'s advice use these lines alike.
//!
//! A conflicted path without a recorded choice is settled one of three
//! ways ([`Settle`]):
//! - a structure path (a file against a directory, or a rename) takes a
//!   side with `kluris resolve --ours` or `--theirs`: a file says nothing
//!   about the directory, so `--file` is never offered there;
//! - a binary path, or a text path `--paths` cannot name exactly
//!   ([`fits_paths`]), takes a side or a version the person wrote to a file
//!   outside the brain, with `kluris resolve --file`;
//! - any other text path is written in the work tree and named in the one
//!   `kluris commit --merge --paths` line.
//!
//! With no path left for `--paths` and none left for `kluris resolve`
//! (every choice recorded, the merge not landed), the commit line is
//! `kluris commit --brain B --merge ID` alone. With paths left only for
//! `kluris resolve`, there is no commit line: the last choice finishes the
//! merge.

use std::fmt;
use std::path::Path;

/// The width of a choice's label, `keep theirs:` and the spaces after it.
const LABEL: usize = 14;

/// How an open conflicted path is settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// A side, with `kluris resolve --ours|--theirs`.
    Side,
    /// A side, or a file, with `kluris resolve`.
    SideOrFile,
    /// The file in the work tree, named in `--paths`.
    Paths,
}

impl Settle {
    /// How a path is settled: `structure` for a file against a directory
    /// or a rename, `binary` for a side that is not text.
    pub fn of(rel: &str, structure: bool, binary: bool) -> Settle {
        if structure {
            Settle::Side
        } else if binary || !fits_paths(rel) {
            Settle::SideOrFile
        } else {
            Settle::Paths
        }
    }
}

/// Whether `--paths` names `rel` exactly: its value is split at commas and
/// each name trimmed, so a name with a comma, or with spaces at either
/// end, cannot be given there.
pub fn fits_paths(rel: &str) -> bool {
    !rel.is_empty() && !rel.contains(',') && rel.trim() == rel
}

/// `word` for a POSIX shell: as it is when it holds only letters, digits
/// and `._/-`, else in single quotes.
pub fn shell_word(word: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "._/-".contains(c);
    if !word.is_empty() && word.chars().all(plain) {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// The name of the file a person writes the version to keep of `rel` to:
/// `merged`, with `rel`'s extension (spaces around the name aside) when it
/// is letters and digits.
pub fn merged_name(rel: &str) -> String {
    match Path::new(rel.trim()).extension().and_then(|e| e.to_str()) {
        Some(ext) if !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()) => {
            format!("merged.{ext}")
        }
        _ => "merged".to_string(),
    }
}

/// The lines that settle `rel` of waiting merge `merge` in `brain` with
/// `kluris resolve`: a labeled command per side, and with `file` the
/// version the person writes to [`merged_name`].
pub fn resolve_lines(brain: &str, merge: &str, rel: &str, file: bool) -> Vec<String> {
    let command = |choice: &str| {
        format!(
            "kluris resolve --brain {} {} {choice} --merge {merge}",
            shell_word(brain),
            shell_word(rel)
        )
    };
    let mut lines = vec![
        format!("{:LABEL$}{}", "keep yours:", command("--ours")),
        format!("{:LABEL$}{}", "keep theirs:", command("--theirs")),
    ];
    if file {
        let name = merged_name(rel);
        lines.push(format!(
            "{:LABEL$}write the version to keep to {name}, in a folder outside the brain, and run there:",
            "keep a merge:"
        ));
        lines.push(format!(
            "{:LABEL$}{}",
            "",
            command(&format!("--file {}", shell_word(&name)))
        ));
    }
    lines
}

/// The lines that finish waiting merge `merge` in `brain` with `kluris
/// commit`: the text `paths` written in the work tree, then named; with
/// none, the merge alone when no path is `open` for `kluris resolve`;
/// otherwise none.
pub fn commit_lines(brain: &str, merge: &str, paths: &[String], open: bool) -> Vec<String> {
    let command = format!(
        "kluris commit --brain {} --merge {merge}",
        shell_word(brain)
    );
    if !paths.is_empty() {
        let names: Vec<String> = paths.iter().map(|rel| shell_word(rel)).collect();
        return vec![
            format!(
                "write the version you want to {} (a deleted file is a deletion), then:",
                paths.join(", ")
            ),
            format!("{command} --paths {}", names.join(",")),
        ];
    }
    if open {
        return Vec::new();
    }
    vec![
        "every path has its choice; finish the merge:".to_string(),
        command,
    ]
}

/// What finishing a waiting merge with `kluris commit --merge` still
/// needs: each open path's [`Settle`]. Paths with a recorded choice need
/// nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeNeeds {
    /// The brain, as `--brain` takes it.
    pub brain: String,
    /// The waiting merge's id.
    pub merge: String,
    /// The conflicted paths without a recorded choice, in the merge's
    /// order, each with how it is settled.
    pub open: Vec<(String, Settle)>,
}

impl MergeNeeds {
    /// The open paths settled as `how`.
    pub fn paths(&self, how: Settle) -> Vec<String> {
        self.open
            .iter()
            .filter(|(_, settle)| *settle == how)
            .map(|(rel, _)| rel.clone())
            .collect()
    }
}

/// The steps, a line each: each path `kluris resolve` settles under its
/// name, then the commit lines.
impl fmt::Display for MergeNeeds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut lines = Vec::new();
        for (rel, how) in &self.open {
            if *how == Settle::Paths {
                continue;
            }
            lines.push(format!("  {rel}:"));
            for line in resolve_lines(&self.brain, &self.merge, rel, *how == Settle::SideOrFile) {
                lines.push(format!("    {line}"));
            }
        }
        let paths = self.paths(Settle::Paths);
        let open = paths.len() < self.open.len();
        let mut commit = commit_lines(&self.brain, &self.merge, &paths, open).into_iter();
        if let Some(step) = commit.next() {
            lines.push(format!("  {step}"));
        }
        lines.extend(commit.map(|line| format!("    {line}")));
        f.write_str(&lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_names_what_its_parser_gives_back() {
        for rel in ["notes/a.md", "notes/with space.md", "notes/it's.md"] {
            assert!(fits_paths(rel), "{rel}");
        }
        for rel in ["notes/a,b.md", "notes/trailing.md ", " notes/a.md", ""] {
            assert!(!fits_paths(rel), "{rel:?}");
        }
    }

    #[test]
    fn the_merged_file_keeps_a_plain_extension() {
        assert_eq!(merged_name("notes/a.md"), "merged.md");
        assert_eq!(merged_name("notes/data.bin"), "merged.bin");
        assert_eq!(merged_name("notes/trailing.md "), "merged.md");
        assert_eq!(merged_name("notes/a.m d"), "merged");
        assert_eq!(merged_name("notes/README"), "merged");
    }

    #[test]
    fn a_word_is_quoted_only_when_a_shell_would_change_it() {
        assert_eq!(shell_word("notes/a.md"), "notes/a.md");
        assert_eq!(shell_word("notes/a,b.md"), "'notes/a,b.md'");
        assert_eq!(shell_word("it's"), "'it'\\''s'");
        assert_eq!(shell_word(""), "''");
    }

    #[test]
    fn every_choice_is_its_own_command() {
        assert_eq!(
            resolve_lines("alpha", "abc", "notes/topic.md", false),
            [
                "keep yours:   kluris resolve --brain alpha notes/topic.md --ours --merge abc",
                "keep theirs:  kluris resolve --brain alpha notes/topic.md --theirs --merge abc",
            ]
        );
        assert_eq!(
            resolve_lines("alpha", "abc", "notes/a,b.md", true)[2..],
            [
                "keep a merge: write the version to keep to merged.md, in a folder outside the brain, and run there:",
                "              kluris resolve --brain alpha 'notes/a,b.md' --file merged.md --merge abc",
            ]
        );
    }

    #[test]
    fn the_commit_line_names_what_is_left_or_nothing() {
        let paths = vec!["notes/a.md".to_string()];
        assert_eq!(
            commit_lines("alpha", "abc", &paths, true)[1],
            "kluris commit --brain alpha --merge abc --paths notes/a.md"
        );
        assert_eq!(
            commit_lines("alpha", "abc", &[], false)[1],
            "kluris commit --brain alpha --merge abc"
        );
        assert!(commit_lines("alpha", "abc", &[], true).is_empty());
    }
}
