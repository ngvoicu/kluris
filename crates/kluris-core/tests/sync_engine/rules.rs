//! The merge rules: what both sides changed is merged by the rules, the
//! maps are rebuilt from the merged files, and nothing is ever lost or
//! written with a conflict marker.

use kluris_core::sync::ConflictKind;
use kluris_core::sync::merge::merge_markdown;

use crate::common::*;
use crate::criss_cross;

#[test]
fn different_files_merge_into_a_sync_commit_and_push() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let ours_before = w.head(&w.ours);
    let r = w.sync();
    assert!(clean(&r), "{r:?}");
    assert_eq!((r.pulled, r.to_push, r.pushed), (1, 0, true), "{r:?}");
    assert!(
        r.auto_merged.is_empty(),
        "different files are not listed: {r:?}"
    );
    let head = w.head(&w.ours);
    assert_eq!(w.parents(&w.ours, &head), 2);
    assert_eq!(w.run(&w.ours, &["rev-parse", "HEAD^1"]).trim(), ours_before);
    assert_eq!(
        w.run(&w.ours, &["log", "-1", "--format=%B"]),
        "kluris: sync\n\nKluris-Sync: auto\n\n"
    );
    assert_eq!(
        w.run(&w.ours, &["log", "-1", "--format=%an <%ae>"]).trim(),
        "Ana Tester <ana@example.com>"
    );
    assert_eq!(w.remote_head(), head);
    assert!(w.read(&w.ours, "ideas/new.md").contains("# New"));
    assert!(w.read(&w.ours, "notes/second.md").contains("# Second"));
    assert_eq!(w.status(&w.ours), "");
    no_markers_anywhere(&w, &w.ours);
}

#[test]
fn generated_maps_are_rebuilt_from_the_merged_tree() {
    let w = World::new();
    // Both sides add a neuron to the same lobe: both regenerate notes/map.md.
    w.theirs_push(&[("notes/theirs.md", &neuron("", "# Theirs\n"))], &[]);
    w.ours_commit(&[("notes/ours.md", &neuron("", "# Ours\n"))], &[]);
    let r = w.sync();
    assert!(r.pulled > 0 && r.pushed && r.conflicts.is_empty(), "{r:?}");
    let map = w.show(&w.ours, "HEAD", "notes/map.md");
    no_markers(&map);
    assert!(map.contains("- [ours.md](./ours.md) — Ours"), "{map}");
    assert!(map.contains("- [theirs.md](./theirs.md) — Theirs"), "{map}");
    assert_eq!(w.read(&w.ours, "notes/map.md"), map);
    assert!(
        !r.auto_merged.contains(&"notes/map.md".to_string()),
        "{r:?}"
    );
    assert_eq!(w.status(&w.ours), "");
}

#[test]
fn a_new_lobe_on_each_side_rebuilds_brain_md() {
    let w = World::new();
    w.theirs_push(&[("theirs-lobe/t.md", &neuron("", "# T\n"))], &[]);
    w.ours_commit(&[("ours-lobe/o.md", &neuron("", "# O\n"))], &[]);
    let r = w.sync();
    assert!(r.pulled > 0 && r.conflicts.is_empty(), "{r:?}");
    let brain = w.show(&w.ours, "HEAD", "brain.md");
    no_markers(&brain);
    assert!(
        brain.contains("[ours-lobe/]") && brain.contains("[theirs-lobe/]"),
        "{brain}"
    );
}

#[test]
fn non_overlapping_body_edits_merge_and_are_listed() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    w.theirs_push(
        &[("notes/first.md", &base.replace("One.", "One, theirs."))],
        &[],
    );
    w.ours_commit(
        &[("notes/first.md", &base.replace("Five.", "Five, ours."))],
        &[],
    );
    let r = w.sync();
    assert!(r.pulled > 0 && r.pushed && r.conflicts.is_empty(), "{r:?}");
    assert_eq!(r.auto_merged, ["notes/first.md"]);
    let text = w.read(&w.ours, "notes/first.md");
    no_markers(&text);
    assert!(
        text.contains("One, theirs.") && text.contains("Five, ours."),
        "{text}"
    );
}

#[test]
fn dates_take_the_later_updated_and_the_earlier_created() {
    let w = World::new();
    let theirs = "---\ncreated: '2025-12-01'\nparent: ./map.md\nupdated: '2026-03-01'\n---\n\n# First\n\nOne, theirs.\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n";
    let ours = "---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-02-01'\n---\n\n# First\n\nOne.\n\nTwo.\n\nThree.\n\nFour.\n\nFive, ours.\n";
    w.theirs_push(&[("notes/first.md", theirs)], &[]);
    w.ours_commit(&[("notes/first.md", ours)], &[]);
    let r = w.sync();
    assert!(r.conflicts.is_empty(), "{r:?}");
    let text = w.read(&w.ours, "notes/first.md");
    assert!(text.contains("created: '2025-12-01'\n"), "{text}");
    assert!(text.contains("updated: '2026-03-01'\n"), "{text}");
    assert!(
        text.contains("One, theirs.") && text.contains("Five, ours."),
        "{text}"
    );
}

#[test]
fn related_and_tags_are_the_union_ours_first() {
    let w = World::new();
    let with = |tags: &str, related: &str| {
        format!(
            "---\ncreated: '2026-01-01'\nparent: ./map.md\nrelated:\n{related}tags:\n{tags}updated: '2026-01-01'\n---\n\n# Keep\n"
        )
    };
    let base = with("- api\n", "- ../ideas/idea.md\n");
    w.ours_commit(&[("notes/keep.md", &base)], &[]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    w.theirs_push(
        &[(
            "notes/keep.md",
            &with(
                "- api\n- zeta\n- auth\n",
                "- ../ideas/idea.md\n- first.md\n",
            ),
        )],
        &[],
    );
    w.ours_commit(
        &[(
            "notes/keep.md",
            &with("- api\n- beta\n", "- ../ideas/idea.md\n"),
        )],
        &[],
    );
    let r = w.sync();
    assert!(r.conflicts.is_empty(), "{r:?}");
    assert_eq!(r.auto_merged, ["notes/keep.md"]);
    let text = w.read(&w.ours, "notes/keep.md");
    assert!(
        text.contains("tags:\n- api\n- beta\n- zeta\n- auth\n"),
        "{text}"
    );
    assert!(
        text.contains("related:\n- ../ideas/idea.md\n- first.md\n"),
        "{text}"
    );
}

#[test]
fn crlf_neurons_merge_and_keep_their_line_endings() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md").replace('\n', "\r\n");
    w.ours_commit(&[("notes/first.md", &base)], &[]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    w.theirs_push(
        &[("notes/first.md", &base.replace("One.", "One, theirs."))],
        &[],
    );
    w.ours_commit(
        &[("notes/first.md", &base.replace("Five.", "Five, ours."))],
        &[],
    );
    let r = w.sync();
    assert!(r.conflicts.is_empty() && r.pulled > 0, "{r:?}");
    let text = w.read(&w.ours, "notes/first.md");
    assert!(
        text.contains("One, theirs.") && text.contains("Five, ours."),
        "{text:?}"
    );
    assert_eq!(
        text.matches('\n').count(),
        text.matches("\r\n").count(),
        "{text:?}"
    );
}

#[test]
fn non_ascii_names_sync() {
    let w = World::new();
    w.theirs_push(&[("été/naïve.md", &neuron("", "# Naïve\n"))], &[]);
    w.ours_commit(&[("notes/café ünï.md", &neuron("", "# Café\n"))], &[]);
    let r = w.sync();
    assert!(r.pulled > 0 && r.pushed && r.conflicts.is_empty(), "{r:?}");
    assert!(w.read(&w.ours, "été/naïve.md").contains("# Naïve"));
    assert!(w.read(&w.ours, "brain.md").contains("été"));
    assert_eq!(w.status(&w.ours), "");
}

#[test]
fn the_same_change_on_both_sides_is_no_conflict() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    let same = base.replace("Three.", "Three, both.");
    w.theirs_push(&[("notes/first.md", &same)], &[]);
    w.ours_commit(&[("notes/first.md", &same)], &[]);
    let r = w.sync();
    assert!(r.conflicts.is_empty() && r.pulled > 0, "{r:?}");
    assert!(w.read(&w.ours, "notes/first.md").contains("Three, both."));
}

/// A file changed on one side only still merges as git's merge-tree says.
#[test]
fn a_criss_cross_history_merges_one_sided_changes() {
    let w = World::new();
    criss_cross(&w);
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let r = w.sync();
    assert!(r.pulled > 0 && r.pushed && r.conflicts.is_empty(), "{r:?}");
    assert!(w.read(&w.ours, "ideas/new.md").contains("# New"));
    assert!(w.read(&w.ours, "notes/second.md").contains("# Second"));
}

#[test]
fn a_rename_on_one_side_and_an_edit_on_the_other_merge_as_git_does() {
    let w = World::new();
    w.run(&w.ours, &["mv", "notes/keep.md", "notes/renamed.md"]);
    w.commit(&w.ours, &["notes/keep.md", "notes/renamed.md"]);
    let edited = w
        .read(&w.theirs, "notes/keep.md")
        .replace("# Keep", "# Keep\n\nTheir approved line.");
    w.theirs_push(&[("notes/keep.md", &edited)], &[]);
    let r = w.sync();
    assert!(r.pulled > 0 && r.pushed && r.conflicts.is_empty(), "{r:?}");
    assert!(
        w.read(&w.ours, "notes/renamed.md")
            .contains("Their approved line.")
    );
    assert!(!w.ours.join("notes").join("keep.md").exists());
    assert_eq!(w.status(&w.ours), "");
}

/// Both clones hold `old/a.md` and `old/b.md`; the renamer moves `old/` to
/// `new/`, the adder adds `old/c.md`. Returns the added blob.
fn folder_rename_and_add(w: &World, ours_renames: bool) -> String {
    for name in ["a", "b"] {
        w.write(
            &w.ours,
            &format!("old/{name}.md"),
            &neuron("", &format!("# Old {name}\n")),
        );
    }
    w.commit(&w.ours, &["old/a.md", "old/b.md"]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q", "--ff-only"]);
    let (renamer, adder) = if ours_renames {
        (&w.ours, &w.theirs)
    } else {
        (&w.theirs, &w.ours)
    };
    w.run(renamer, &["mv", "old", "new"]);
    w.run(renamer, &["commit", "-q", "-m", "rename the lobe"]);
    w.write(
        adder,
        "old/c.md",
        &neuron("", "# Team approved new neuron\n"),
    );
    w.run(adder, &["add", "old/c.md"]);
    w.run(adder, &["commit", "-q", "-m", "a neuron in the old lobe"]);
    w.run(&w.theirs, &["push", "-q"]);
    w.run(adder, &["rev-parse", "HEAD:old/c.md"])
        .trim()
        .to_string()
}

#[test]
fn a_file_added_in_a_folder_the_other_side_renamed_follows_the_rename() {
    for ours_renames in [true, false] {
        let w = World::new();
        let blob = folder_rename_and_add(&w, ours_renames);
        let r = w.sync();
        assert!(
            r.pulled > 0 && r.pushed && r.conflicts.is_empty(),
            "{ours_renames}: {r:?}"
        );
        let tree = w.run(&w.ours, &["ls-tree", "-r", "HEAD"]);
        assert!(
            tree.contains(&format!("{blob}\tnew/c.md")),
            "{ours_renames}: {tree}"
        );
        assert!(!tree.contains("old/"), "{ours_renames}: {tree}");
        assert_eq!(w.remote_head(), w.head(&w.ours));
        assert!(w.read(&w.ours, "new/map.md").contains("c.md"));
        assert_eq!(w.status(&w.ours), "");
    }
}

/// A small deterministic generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn file(created: &str, updated: &str, tags: &[String], body: &[String]) -> String {
    let mut text = format!("---\ncreated: '{created}'\nparent: ./map.md\n");
    if !tags.is_empty() {
        text.push_str("tags:\n");
        for t in tags {
            text.push_str(&format!("- {t}\n"));
        }
    }
    text.push_str(&format!("updated: '{updated}'\n---\n\n"));
    text.push_str(&body.join("\n"));
    text.push('\n');
    text
}

#[test]
fn auto_merges_never_write_markers_and_keep_every_change() {
    let w = World::new();
    let git = w.git();
    let cases: usize = std::env::var("KLURIS_SYNC_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for case in 0..cases {
        let n = 8 + rng.below(20);
        let base_body: Vec<String> = (0..n).map(|i| format!("line {i}")).collect();
        let base_tags: Vec<String> = (0..rng.below(3)).map(|i| format!("t{i}")).collect();
        // Ours edits in the first half, theirs in the second, at least two
        // unchanged lines apart: never overlapping.
        let i = rng.below(n / 2 - 1);
        let j = n / 2 + 1 + rng.below(n - n / 2 - 1);
        let mut ours_body = base_body.clone();
        ours_body[i] = format!("ours {case}");
        if rng.below(2) == 0 {
            ours_body.insert(i + 1, format!("ours added {case}"));
        }
        let mut theirs_body = base_body.clone();
        theirs_body[j] = format!("theirs {case}");
        let mut ours_tags = base_tags.clone();
        ours_tags.push(format!("o{}", rng.below(5)));
        let mut theirs_tags = base_tags.clone();
        theirs_tags.push(format!("h{}", rng.below(5)));
        let day = |r: &mut Rng| format!("2026-0{}-1{}", 1 + r.below(9), r.below(10));
        let (oc, ou, tc, tu) = (day(&mut rng), day(&mut rng), day(&mut rng), day(&mut rng));
        let base = file("2026-01-01", "2026-01-01", &base_tags, &base_body);
        let ours = file(&oc, &ou, &ours_tags, &ours_body);
        let theirs = file(&tc, &tu, &theirs_tags, &theirs_body);
        let merged = merge_markdown(
            Some(base.as_bytes()),
            ours.as_bytes(),
            theirs.as_bytes(),
            &git,
        )
        .unwrap_or_else(|kind| panic!("case {case}: {kind:?}\n{ours}\n{theirs}"));
        let text = String::from_utf8(merged.bytes).unwrap();
        no_markers(&text);
        for line in ours_body
            .iter()
            .chain(&theirs_body)
            .filter(|l| !l.starts_with("line "))
        {
            assert!(
                text.contains(line.as_str()),
                "case {case} lost {line:?}:\n{text}"
            );
        }
        for tag in ours_tags.iter().chain(&theirs_tags) {
            assert!(
                text.contains(&format!("- {tag}\n")),
                "case {case} lost tag {tag}:\n{text}"
            );
        }
        let (created, updated) = (oc.clone().min(tc.clone()), ou.clone().max(tu.clone()));
        assert!(
            text.contains(&format!("created: '{created}'\n")),
            "case {case}:\n{text}"
        );
        assert!(
            text.contains(&format!("updated: '{updated}'\n")),
            "case {case}:\n{text}"
        );
    }
}

/// Every conflict kind is spelled one way: lowercase words joined by `_`.
#[test]
fn conflict_kinds_are_lowercase_with_underscores() {
    let kinds = [
        ConflictKind::Content,
        ConflictKind::DeleteEdit,
        ConflictKind::Parent,
        ConflictKind::Frontmatter,
        ConflictKind::Structure,
        ConflictKind::AmbiguousBase,
    ];
    let names: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
    assert_eq!(
        names,
        [
            "content",
            "delete_edit",
            "parent",
            "frontmatter",
            "structure",
            "ambiguous_base"
        ]
    );
    for name in names {
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{name}"
        );
    }
}
