# Parity allowlist

Every deliberate difference between Kluris v2.28.11 (the oracle) and Kluris 3
that the parity harness tolerates is listed here. A difference that is not
listed is a bug in Kluris 3.

Normalizations every comparison applies (they are not entries): the temp root
becomes `<ROOT>` (raw, JSON-escaped and `C:/` forms), `kluris <version>`
becomes `kluris <VERSION>`, an `updated:` value equal to today at either end of
a run, and human-readable output, whose wording is checked by substring only.

## Format

One entry per difference, under its own heading:

```
### PAR-NNN: <short title>
- Command: the command and flags it applies to (`kluris dream --json`)
- Differs: what v3 does instead of v2, precisely enough to write the assertion
- Decision: the spec decision or interview answer it comes from (link or id)
- Tests: the parity test(s) that encode it
```

## Entries

### PAR-001: JSON/TOML frontmatter ignored
- Command: every command that reads markdown neuron frontmatter (`dream`, `search`, `wake-up`, `mri`, `status`)
- Differs: a markdown file whose (whitespace-stripped) text opens with a line that is exactly `{` is not read as JSON frontmatter: v3 sees no frontmatter (empty metadata, the whole text as body), where v2 parses the `{` ... `}` block with `json.loads` through python-frontmatter's `JSONHandler`. `+++` TOML frontmatter is no frontmatter in both (v2's python-frontmatter has no `toml` package installed), so only JSON differs in practice.
- Decision: accidental v2 behaviour via python-frontmatter, decided 2026-10-25 (T-51 brief, Kluris 3 Phase 1)
- Tests: `crates/kluris-core/tests/frontmatter_read.rs::parse_json_and_toml_frontmatter_are_not_frontmatter` (no parity scenario yet)

### PAR-002: first body line keeps its indentation
- Command: every command that rewrites a markdown neuron's frontmatter (`dream`'s date, parent and synapse fixes)
- Differs: when the body's first non-blank line is indented (`    code first`), v3 writes it with its indentation; v2 writes it without (python-frontmatter strips the whole body on read). Leading whitespace-only lines and all trailing whitespace are still removed, as in v2.
- Decision: Kluris 3 decision (T-52 brief, Phase 1, IMPL-K3-06; research-01 §1 "Body handling")
- Tests: `crates/kluris-core/tests/yaml_emit.rs::markdown_framing_matches_python_frontmatter` (rows "indented first line", "blank lines then indented line"; no parity scenario yet)

### PAR-003: `windsurf` agent is `devin`; `antigravity` joins the default
- Command: every command that reads a brain's `kluris.yml` and every command that rewrites it (`create`, `register`, `companion add`, `companion remove`, `doctor`)
- Differs:
  - An agent name `windsurf` in `agents.commands_for` reads as `devin`, so a `kluris.yml` written by v3 has `  - devin` where v2 has `  - windsurf`.
  - The default `commands_for` (what `create` and `register` write, and what an empty selection means) is v2's `DEFAULT_AGENT_COMMANDS` in v2's order with `devin` in place of `windsurf`, then `antigravity`: `claude, cursor, devin, copilot, codex, gemini, hermes, kilocode, junie, antigravity`. v2 writes its 9 names with `windsurf`.
  - A `commands_for` equal as a set to v2's pre-Hermes 8 names (v2 migrates these too, to its own 9) or to v2.28's 9 names (with `windsurf` or with `devin`) reads as the v3 default, so a brain created by v2 gets `antigravity` without editing its file; v2 keeps the 9 names. Any other list is kept as written (only `windsurf` -> `devin`), unknown names included, as in v2.
  - Every other byte of `kluris.yml` is v2's. v2.28.11 reads a file with `devin` or `antigravity` without error (it skips names outside its registry).
- Decision: Windsurf became Devin (SPEC decision log 2026-10-24, research-04, interview-01 #15-16; SPEC acceptance "a `windsurf` entry in an existing `kluris.yml` is treated as `devin`"; T-53 brief); default list with `antigravity` and the v2-default migration decided by the chief on T-53 (m-4244), 2026-10-25
- Tests: `crates/kluris-core/tests/config.rs::{defaults_are_v2_defaults_with_devin_then_antigravity, v2_default_lists_migrate_to_the_v3_default, v2_brain_config_round_trips_with_the_v3_default_agents, windsurf_is_read_as_devin_and_unknown_agents_are_kept}`, `parity/test_config_roundtrip.py::test_v2_reads_v3_agent_names` (no v3 command writes `kluris.yml` yet)

### PAR-004: CRLF in stdout and stderr reads as LF
- Command: every command the harness runs, on Windows
- Differs: on Windows v2 (Python text streams) ends each line of stdout and stderr with CRLF where v3 (Rust `println!`) writes LF. The harness decodes both streams of both binaries with every `\r\n` read as `\n` before comparing; a lone `\r` is kept. Files written into brains and configs are compared as bytes, without this.
- Decision: Kluris 3 decision, chief's brief for T-56 (Phase 1, TEST-K3-09 / IMPL-K3-10), 2026-10-25
- Tests: `parity/test_harness.py::test_run_turns_crlf_into_lf`; every parity test on Windows (`cargo xtask windows --parity`)

### PAR-005: config files keep their line endings
- Command: every command that rewrites `~/.kluris/config.yml` or a brain's `kluris.yml` (`create`, `register`, `remove`, `companion add`, `companion remove`, `doctor`)
- Differs: v3 writes an existing config file with the line endings it had (CRLF when its first line break is CRLF, else LF) and a new one with LF on every OS. v2 writes in text mode: LF on macOS/Linux (an existing CRLF file becomes LF) and CRLF on Windows (a new file is CRLF). So on macOS/Linux a CRLF config stays CRLF in v3, and on Windows a config v3 creates is LF.
- Decision: interview-01 #3 (keep each file's line endings; new files LF), SPEC decision log 2026-10-25 (Windows text mode) and 2026-10-26 (T-54 finding 1, fixed in T-59)
- Tests: `crates/kluris-core/tests/config.rs::{global_config_rewrite_keeps_the_existing_line_endings, brain_config_rewrite_keeps_the_existing_line_endings, the_first_line_break_decides_the_endings, write_through_a_symlink_keeps_the_target_endings}`, `parity/test_config_roundtrip.py::test_v2_reads_a_config_rewritten_by_v3[crlf]`

### PAR-006: frontmatter YAML read the way PyYAML's pure-Python SafeLoader reads it
- Command: every command that reads markdown neuron frontmatter (`dream`, `search`, `wake-up`, `mri`, `status`); YAML neurons and config files read the same way in both versions
- Differs: v3 reads every YAML text with a port of PyYAML 6.0.3's pure-Python `SafeLoader`. v2 read markdown frontmatter with libyaml's `CSafeLoader` (python-frontmatter picks it when libyaml is installed), which reads some unusual texts otherwise. Measured over 20,000 generated texts (67 differ):
  - libyaml reads a value where `SafeLoader` (and v3) fail: tabs in places the pure scanner refuses (`a: b\tc` is an error in v3, `{'a': 'b\tc'}` in v2), `a?b` inside a flow collection (`[a?b]`), a BOM after the start of the stream, and an empty `!`-tagged node before `:` (`! &a: []`, a `''` key in v2, a null key and so an error in v3).
  - libyaml fails where `SafeLoader` (and v3) read a value: glued flow forms such as `v: {a:[]}`, `{a:{}}`, `{a:,b}`, `{?x:[?x]}`. (It also fails on `[?]` and `"\ud800"`, which `SafeLoader` reads to values outside the v3 model, so both versions fail there.)
  None of the 2,345 markdown files of the seven local brains is in these classes (v3 reads all of them as v2 does).
- Decision: one YAML reading rule for v3, the pure-Python `SafeLoader` (chief's decision on T-67, 2026-10-26, replacing saphyr-parser with a port of PyYAML's front end)
- Tests: `parity/test_yaml_load_differential.py` (counts the texts where `CSafeLoader` reads otherwise), the `// CSafeLoader:` notes on the rows of `crates/kluris-core/tests/yaml_read.rs::documents`

### PAR-007: deprecation check skips a neuron it cannot read
- Command: the deprecation check behind `kluris wake-up --json` (`deprecation`, `deprecation_count`) and `kluris dream` (deprecation warnings), once those are ported; today `kluris_core::brain::deprecation::detect_deprecation_issues`
- Differs: a markdown neuron whose frontmatter cannot be read (it does not parse, is not UTF-8, cannot be opened, or is a broken symlink that points inside the brain) is left out: v3 reports the issues of the other neurons, as v2's own rules give them over those neurons, and lists the left-out files (`DeprecationReport::skipped`). v2's `detect_deprecation_issues` raises on the first such file, so v2's wake-up reports `deprecation: []` and `deprecation_count: 0` for the whole brain and v2's dream ends with a traceback (research-03 §5 item 15). A YAML neuron whose `#---` block does not parse is no difference: both read it as no metadata (active). How wake-up and dream present the skipped files is decided when they are ported.
- Decision: "a malformed neuron never stops the walk: it is returned as a per-file error value so callers can skip and report it" (chief's brief for T-75, Kluris 3 Phase 2, TEST-K3-11 / IMPL-K3-12), 2026-10-26
- Tests: `crates/kluris-core/tests/brain_walk.rs::{deprecation_skips_malformed_neurons_and_reports_them, deprecation_with_symlinks_matches_v2}`, `parity/test_brain_model.py::test_fixture_brain_model_matches_v2[malformed]` and `[links]` (v2's rules run with `preparsed=` over the neurons that read); wake-up: `crates/kluris-core/tests/wake_up.rs::malformed_neurons_are_skipped_and_counted`, `parity/test_wake_up.py::{test_malformed_fixture_brain, test_fixture_brain[links]}` (v2's `deprecation: []` against v3's issues of the neurons that read)

### PAR-008: neurons in path order, not directory-read order
- Command: every output whose order follows v2's neuron walk, once ported: `kluris wake-up --json` `deprecation[]`, `kluris dream --json` `broken_synapses_detail[]`, MRI node ids; today `kluris_core::brain::walk::walk` and `neuron_index::neuron_files`
- Differs: v3 lists files sorted by the bytes of their `/`-separated brain-relative path, and neurons as markdown then YAML, each in that order. v2 keeps `os.walk`'s directory-read order (markdown then YAML), which depends on the filesystem and differs between machines. So the same set of entries can come out in another order; parity compares these lists as sets or multisets (research-03 §3.2), and an order that v2 itself sorts (search results, `recent`) is not affected.
- Decision: one deterministic walker for every command (chief's brief for T-75, Kluris 3 Phase 2), 2026-10-26
- Tests: `crates/kluris-core/tests/brain_walk.rs::{walker_order_is_by_path_bytes, neuron_index_matches_v2_on_the_full_brain}`, `parity/test_brain_model.py` (neuron sets; deprecation in v3's order against v2's rules in that order), `parity/test_wake_up.py` (`deprecation[]` as a multiset, `parity/runs.py::assert_wake_up_equal`)

### PAR-009: `wake-up` counts the neurons it cannot read in `parse_errors`
- Command: `kluris wake-up --json`
- Differs: `parse_errors` is the number of markdown neurons whose frontmatter v3 cannot read (it does not parse, is not UTF-8, cannot be opened, or is a broken symlink that points inside the brain); those neurons are left out of `recent` and `deprecation` (PAR-007). v2's CLI always prints `parse_errors: 0` (only its pack's boot snapshot counts). YAML neurons are never counted: both versions read a `#---` block that does not parse as no metadata. Where both readers fail (a timestamp PyYAML cannot construct such as `2026-02-30`, a non-string key) the only difference is the count. Frontmatter that v2 reads but v3's reader refuses as an error value (a `<<` merge key, `!!binary`, `!!set`) also counts, and those neurons drop out of `recent` and `deprecation` in v3. Text output does not show the count, as in v2.
- Decision: skip and report malformed neurons (chief's brief for T-76, Kluris 3 Phase 2, TEST-K3-13 / IMPL-K3-14, 2026-10-26; SPEC acceptance "one malformed neuron never crashes ... `wake-up`"); the refused YAML forms: TEST-K3-03
- Tests: `crates/kluris-core/tests/wake_up.rs::malformed_neurons_are_skipped_and_counted`, `parity/test_wake_up.py::{test_malformed_fixture_brain, test_fixture_brain[links], test_odd_updated_values, test_frontmatter_v3_refuses}`

### PAR-010: markdown frontmatter behind a UTF-8 BOM is read
- Command: every command that reads markdown frontmatter; today `kluris wake-up --json` (`recent`, `brain_md`, lobe descriptions, deprecation)
- Differs: a markdown file that starts with a UTF-8 BOM and then a `---` block is read with its frontmatter, the BOM dropped. v2 (python-frontmatter) sees no frontmatter there: empty metadata, and the whole text, BOM and `---` lines included, as the body. So in wake-up such a neuron's `updated` puts it in v3's `recent` and not in v2's, brain.md's `brain_md` is its body without the BOM and frontmatter in v3 (v2: `"\ufeff---\n..."`), and a map.md `description` in that frontmatter is used by v3 (v2 falls back to the body).
- Decision: SPEC TEST-K3-03 ("UTF-8 BOM (read correctly)"), research-01 §1 (the BOM bug); the dream side is TEST-K3-21
- Tests: `crates/kluris-core/tests/frontmatter_read.rs::{parse_bom_is_detected_and_frontmatter_still_read, read_bom_file}`, `parity/test_wake_up.py::test_bom_frontmatter_is_read`

### PAR-011: a lobe `map.md` that is not UTF-8 gives an empty description
- Command: `kluris wake-up` (with and without `--json`)
- Differs: when a lobe's `map.md` is not valid UTF-8, v3 gives that lobe `description: ""` and prints the payload. v2 falls back to `map_file.read_text()`, whose `UnicodeDecodeError` its `except OSError` does not catch: a traceback on stderr, exit 1, no output.
- Decision: a malformed file never crashes `wake-up` (SPEC acceptance; chief's brief for T-76, 2026-10-26)
- Tests: `crates/kluris-core/tests/wake_up.rs::lobe_description_rules`, `parity/test_wake_up.py::test_map_md_that_is_not_utf8`

### PAR-012: a commit subject containing `|` is kept whole
- Command: `kluris status` (with and without `--json`, `recent_commits` and the dated lines); the git layer's `log`
- Differs: v3 reads `git log` with NUL-separated fields (`--format=%H%x00%s%x00%aI%x00`), so the subject `has | pipe | char` is listed as it is. v2 split `%H|%s|%aI` on `|`: its `recent_commits` has `"has "`, and its text line is `   pipe | ch has ` (the subject's tail standing in for `date[:10]`).
- Decision: fix v2's `|` split (research-03 §5 item 29; chief's brief for T-78, Kluris 3 Phase 2, TEST-K3-17 / IMPL-K3-18), 2026-10-26
- Tests: `crates/kluris-core/tests/git.rs::{log_keeps_a_subject_with_pipes, file_dates_latest_and_created_per_path}`, `parity/test_list_status_help.py::test_status_subject_with_pipes`

### PAR-013: non-ASCII file names are not C-quoted
- Command: `kluris status` (`uncommitted`, with and without `--json`); the git layer's `status_short` and `file_dates` (dream's dates, once ported)
- Differs: v3 runs git with `-c core.quotePath=false`, so `uncommitted` has `?? projects/é.md` where v2 has `?? "projects/\303\251.md"` (git's octal C-quoting of every byte over 0x7f). A name git quotes for another reason (a space, `"`, `\`, a control character) is quoted in both. The file-dates log reads NUL-separated names (`-z`), never quoted: v2's quoted names never matched a neuron, so their dates were not refreshed.
- Decision: fix v2's quoted non-ASCII names (research-03 §5 item 26; chief's brief for T-78), 2026-10-26
- Tests: `crates/kluris-core/tests/git.rs::{status_shows_non_ascii_names_unquoted, file_dates_keep_non_ascii_and_odd_names_as_written}`, `parity/test_list_status_help.py::test_status_non_ascii_names`

### PAR-014: a brain nested in a larger repo sees only itself
- Command: `kluris status` (`recent_commits`, `uncommitted`, the text); the git layer's `file_dates` (dream's dates, once ported)
- Differs: when the brain directory is a subdirectory of the work tree (`git rev-parse --show-prefix` is not empty), v3 runs `git log -10 ... -- .` and `git status --short -- .` in the brain, so `recent_commits` lists only commits that touched the brain and `uncommitted` only its own changes, brain-relative (`M projects/alpha.md`). v2 listed the whole outer repo (`outer: code`, `M ../../README.md`). The file-dates log runs with `--relative`, so its paths are brain-relative (`a.md`, v2: `brain/a.md`, which never matched a neuron). A brain at the repo root runs v2's exact commands (no pathspec, which would drop empty commits).
- Decision: scope git to the brain when nested (research-03 §5 item 26; chief's brief for T-78), 2026-10-26
- Tests: `crates/kluris-core/tests/git.rs::{log_of_a_nested_brain_lists_only_its_own_commits, status_of_a_nested_brain_is_scoped_and_brain_relative, file_dates_of_a_nested_brain_are_brain_relative, log_keeps_empty_commits_at_the_repo_root}`, `parity/test_list_status_help.py::test_status_nested_brain_is_scoped_to_the_brain`

### PAR-015: `status` of a repo without commits
- Command: `kluris status` (with and without `--json`)
- Differs: in a git repo with no commit yet, v3 reports the brain with `git_enabled: true`, `recent_commits: []` and its `uncommitted` text, and exits 0. v2's `git log` fails there (no `HEAD`): a `CalledProcessError` traceback, exit 1, no JSON.
- Decision: a v2 crash the brief did not name; fixed as an empty log so `status` never crashes on a fresh repo (T-78, reported to the chief), 2026-10-26
- Tests: `crates/kluris-core/tests/git.rs::log_of_a_repo_without_commits_is_empty`, `parity/test_list_status_help.py::test_status_of_a_repo_without_commits`

### PAR-016: user text in human output is never markup
- Command: every command's human (non-JSON) output; today `kluris list`, `kluris status`, `kluris help`, `kluris wake-up`
- Differs: v3 prints names, paths, descriptions, commit subjects and other user text as they are. v2 passed them to Rich as markup: `see [red]docs[/red]` printed `see docs`, and a stray closing tag (`ends [/oops]`) raised `MarkupError` (traceback, exit 1) in `list`.
- Decision: v2's markup-injection bug is not copied (research-03 §5 item 12; decided, chief's brief for T-78), 2026-10-26
- Tests: `crates/kluris-cli/src/output/human.rs::tests::user_text_is_never_markup`, `parity/test_list_status_help.py::{test_list_and_status_print_user_text_literally, test_list_with_a_closing_tag_that_crashed_v2}`

### PAR-017: `help --json COMMAND` describes that command
- Command: `kluris help COMMAND --json`
- Differs: v3 prints `{"ok": true, "commands": [{"name": ..., "description": ...}]}` with that one command, and for a name not in the table the envelope `{"ok": false, "error": "Unknown command: <name>"}` with exit 1 (as `help <name>` without `--json` fails in both). v2 ignored COMMAND under `--json`: the whole table, exit 0. `help --json` without COMMAND is v2's table (less `pack`, PAR-018).
- Decision: fix v2's ignored argument (research-03 §5 item 20; chief's brief for T-78), 2026-10-26
- Tests: `parity/test_list_status_help.py::test_help_json_with_a_command`

### PAR-018: `pack` is gone
- Command: `kluris help` (with and without `--json`), `kluris help pack`, `kluris pack`, `kluris --help`
- Differs: the help table has v2's 12 other rows in v2's order, without `pack` (the JSON `commands` array likewise); `help pack` is `Unknown command: pack` (exit 1); `kluris pack` is `No such command 'pack'.` (exit 2, or the envelope with exit 1 under `--json`).
- Decision: Kluris 3 drops `pack` (SPEC; research-03 §1.14 "Rust drops `pack`")
- Tests: `parity/test_list_status_help.py::{test_help_text_matches_v2_without_pack, test_help_json_lists_v2s_commands_without_pack, test_help_for_an_unknown_command, test_help_json_with_a_command, test_usage_errors_with_json, test_root_help_flag}`

### PAR-019: `help` with a config that does not read
- Command: `kluris help` (with and without `--json`, with or without COMMAND)
- Differs: both read `~/.kluris/config.yml` first and fail on one that does not parse, with exit 1; v3 prints `Error: Invalid config file <path>: ...` (the envelope under `--json`), as its `list` does, where v2 ends in a traceback (no JSON).
- Decision: no traceback for a bad config (research-03 §5 item 20; chief's brief for T-78), 2026-10-26
- Tests: `parity/test_list_status_help.py::test_help_with_a_corrupt_config`
