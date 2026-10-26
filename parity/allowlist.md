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
- Decision: interview-01 #5 ("`|` in a commit subject breaking `status`"; git-layer fixes cite it, SPEC decision 2026-10-27 (tracebacks)); research-03 §5 item 29; T-78
- Tests: `crates/kluris-core/tests/git.rs::{log_keeps_a_subject_with_pipes, file_dates_latest_and_created_per_path}`, `parity/test_list_status_help.py::test_status_subject_with_pipes`

### PAR-013: non-ASCII file names are not C-quoted
- Command: `kluris status` (`uncommitted`, with and without `--json`); the git layer's `status_short` and `file_dates` (dream's dates, once ported)
- Differs: v3 runs git with `-c core.quotePath=false`, so `uncommitted` has `?? projects/é.md` where v2 has `?? "projects/\303\251.md"` (git's octal C-quoting of every byte over 0x7f). A name git quotes for another reason (a space, `"`, `\`, a control character) is quoted in both. The file-dates log reads NUL-separated names (`-z`), never quoted: v2's quoted names never matched a neuron, so their dates were not refreshed.
- Decision: interview-01 #5 ("git dates missing ... non-ASCII names"; the same git-layer fix reaches `status`, and git-layer fixes cite #5 per SPEC decision 2026-10-27 (tracebacks)); research-03 §5 item 26; T-78
- Tests: `crates/kluris-core/tests/git.rs::{status_shows_non_ascii_names_unquoted, file_dates_keep_non_ascii_and_odd_names_as_written}`, `parity/test_list_status_help.py::test_status_non_ascii_names`

### PAR-014: a brain nested in a larger repo sees only itself
- Command: `kluris status` (`recent_commits`, `uncommitted`, the text); the git layer's `file_dates` (dream's dates, once ported)
- Differs: when the brain directory is a subdirectory of the work tree (`git rev-parse --show-prefix` is not empty), v3 runs `git log -10 ... -- .` and `git status --short -- .` in the brain, so `recent_commits` lists only commits that touched the brain and `uncommitted` only its own changes, brain-relative (`M projects/alpha.md`). v2 listed the whole outer repo (`outer: code`, `M ../../README.md`). The file-dates log runs with `--relative`, so its paths are brain-relative (`a.md`, v2: `brain/a.md`, which never matched a neuron). A brain at the repo root runs v2's exact commands (no pathspec, which would drop empty commits).
- Decision: interview-01 #5 ("git dates missing nested-repo brains"; `status` is scoped by the same git-layer fix, and git-layer fixes cite #5 per SPEC decision 2026-10-27 (tracebacks)); research-03 §5 item 26; T-78
- Tests: `crates/kluris-core/tests/git.rs::{log_of_a_nested_brain_lists_only_its_own_commits, status_of_a_nested_brain_is_scoped_and_brain_relative, file_dates_of_a_nested_brain_are_brain_relative, log_keeps_empty_commits_at_the_repo_root}`, `parity/test_list_status_help.py::test_status_nested_brain_is_scoped_to_the_brain`

### PAR-015: `status` of a repo without commits
- Command: `kluris status` (with and without `--json`)
- Differs: in a git repo with no commit yet, v3 reports the brain with `git_enabled: true`, `recent_commits: []` and its `uncommitted` text, and exits 0. v2's `git log` fails there (no `HEAD`): a `CalledProcessError` traceback, exit 1, no JSON.
- Decision: SPEC decision 2026-10-27 (tracebacks), which names this case; found in T-78
- Tests: `crates/kluris-core/tests/git.rs::log_of_a_repo_without_commits_is_empty`, `parity/test_list_status_help.py::test_status_of_a_repo_without_commits`

### PAR-016: user text in human output is never markup
- Command: every command's human (non-JSON) output; today `kluris list`, `kluris status`, `kluris help`, `kluris wake-up`
- Differs: v3 prints names, paths, descriptions, commit subjects and other user text as they are. v2 passed them to Rich as markup: `see [red]docs[/red]` printed `see docs`, and a stray closing tag (`ends [/oops]`) raised `MarkupError` (traceback, exit 1) in `list`.
- Decision: the `MarkupError` crash: SPEC decision 2026-10-27 (tracebacks). Printing markup-like user text literally instead of rendering it: chief's brief for T-78 ("v2's markup-injection bug is not copied", research-03 §5 item 12); no SPEC Decision Log row yet (reported in T-82)
- Tests: `crates/kluris-cli/src/output/human.rs::tests::user_text_is_never_markup`, `parity/test_list_status_help.py::{test_list_and_status_print_user_text_literally, test_list_with_a_closing_tag_that_crashed_v2}`

### PAR-017: `help --json COMMAND` describes that command
- Command: `kluris help COMMAND --json`
- Differs: v3 prints `{"ok": true, "commands": [{"name": ..., "description": ...}]}` with that one command, and for a name not in the table the envelope `{"ok": false, "error": "Unknown command: <name>"}` with exit 1 (as `help <name>` without `--json` fails in both). v2 ignored COMMAND under `--json`: the whole table, exit 0. `help --json` without COMMAND is v2's table (less `pack`, PAR-018).
- Decision: chief's brief for T-78 ("Fix v2's bug that `help --json` ignores COMMAND", research-03 §5 item 20); no SPEC Decision Log row yet (reported in T-82)
- Tests: `parity/test_list_status_help.py::test_help_json_with_a_command`

### PAR-018: `pack` is gone
- Command: `kluris help` (with and without `--json`), `kluris help pack`, `kluris pack`, `kluris --help`
- Differs: the help table has v2's 12 other rows in v2's order, without `pack` (the JSON `commands` array likewise); `help pack` is `Unknown command: pack` (exit 1); `kluris pack` is `No such command 'pack'.` (exit 2, or the envelope with exit 1 under `--json`).
- Decision: SPEC decision 2026-10-24 (`pack` dropped completely); interview-01 product decisions; SPEC acceptance "`kluris pack` exits with the standard 'no such command' error"
- Tests: `parity/test_list_status_help.py::{test_help_text_matches_v2_without_pack, test_help_json_lists_v2s_commands_without_pack, test_help_for_an_unknown_command, test_help_json_with_a_command, test_usage_errors_with_json, test_root_help_flag}`

### PAR-019: `help` with a config that does not read
- Command: `kluris help` (with and without `--json`, with or without COMMAND)
- Differs: both read `~/.kluris/config.yml` first and fail on one that does not parse, with exit 1; v3 prints `Error: Invalid config file <path>: ...` (the envelope under `--json`), as its `list` does, where v2 ends in a traceback (no JSON).
- Decision: SPEC decision 2026-10-27 (tracebacks), which names this case; research-03 §5 item 20
- Tests: `parity/test_list_status_help.py::test_help_with_a_corrupt_config`

### PAR-020: `search` ranks with BM25F over heading chunks
- Command: `kluris search` (with and without `--json`)
- Differs: v3 ranks with field-weighted BM25 over heading chunks (Snowball stemming in the brain's language, diacritic folding, prefix fallback, phrase guard, deprecated ×0.5), so against v2's literal-substring search:
  - result order, `score` values (`round(bm25 × 100)`, at least 1; still an integer) and `snippet` text differ, and v3 returns more results (any item BM25 matches; `total` is still the count after `--limit`);
  - `matched_fields` lists the fields where a query word matched or v2's substring matched, in the order `title, tag, path, heading, body` (`heading` is new);
  - each result has, after v2's seven keys (`file, file_type, title, matched_fields, snippet, score, deprecated`), the keys `heading` (string or null), `line` (1-based line of the best chunk, or null for glossary entries), `matched_terms`, `coverage` (float 0-1), `related` (up to 3 brain-relative neuron paths), `updated` (string or null), and on deprecated results only `replaced_by` (string or null); the top level ends with `"engine": "bm25"` after `results`.
  - Kept as v2: the error envelope and messages, exit codes, `--lobe`/`--tag` filters (nothing v3 returns is an item v2's filters drop), glossary entries as their own results (`file: "glossary.md"`, term as `title`), the `brain.md` item, `file_type`, `title` and `deprecated` of each item, `limit <= 0` giving no results, and the text output's wording. Invariant: with `--limit` high enough every item v2 returns (file and title) is also in v3's results, with v2's `matched_fields` among v3's. The reader differences of PAR-001, PAR-006, PAR-009 and PAR-010 carry over (a neuron v3 reads otherwise is searched as v3 reads it).
- Decision: research-05 §6 (rung 1 for v3.0: "Don't keep v2's ordering as the default"; the JSON contract and the parity invariant), research-06 §6; chief's brief for T-77 (Kluris 3 Phase 2, TEST-K3-15 / IMPL-K3-16), 2026-10-26
- Tests: `crates/kluris-core/tests/search.rs`, `crates/kluris-cli/tests/search.rs`, `parity/test_search.py` (fixture brains, the search-eval brains, the registered brains with the 74 tuning queries), `cargo xtask search-eval`

### PAR-021: `search.language` in `kluris.yml` is kept on rewrite
- Command: every command that rewrites a brain's `kluris.yml` (`create`, `register`, `companion add`, `companion remove`, `doctor`); read by `kluris search`
- Differs: a `search:` mapping whose `language` is a string is read (`en` or `ro`, any case, picks the search stemmer and stop words; another value means autodetect) and written back after `companions` as `search:\n  language: <value>\n`; v2's pydantic model ignores the key and drops it on rewrite. Any other `search` value (not a mapping, no string `language`) is ignored and dropped, as in v2, and never makes the file invalid. Without the key every byte is v2's.
- Decision: chief's brief for T-77 ("a local override `search: {language: en|ro}` in the brain's `kluris.yml` ... check the config module keeps that key on rewrite"), research-05 question 2, 2026-10-26
- Tests: `crates/kluris-core/tests/config.rs::the_search_language_key_is_read_and_kept_on_rewrite`, `parity/test_search.py::test_language_override_keeps_the_invariant`

### PAR-022: a `NUL` stdin on Windows is not a terminal
- Command: every command that resolves brains (`wake-up`, `search`, `status`; later `dream`, `mri`, `companion add/remove`) with two or more brains, no `--brain`, no `--json`, `KLURIS_NO_PROMPT` not `1`, and stdin the null device (`< NUL`, `subprocess.DEVNULL`) on Windows
- Differs: v3 asks whether stdin is a console (Rust's `IsTerminal`, `GetConsoleMode`), which `NUL` is not, so it prints the non-interactive error `Multiple brains registered. Pass --brain NAME[ or 'all']. Available: ...` on stderr. v2's `sys.stdin.isatty()` is true for any character device, `NUL` included: it prints the picker menu and `Choice: ` on stdout, reads end of input and prints `Aborted!`. Both exit 1. On macOS and Linux `/dev/null` is no terminal for either, and both print the error.
- Decision: SPEC decision 2026-10-26 (resolver hook and `< NUL`; "NUL is not a person typing"), kept by its 2026-10-27 update (T-80)
- Tests: `crates/kluris-cli/tests/resolver.rs::a_null_stdin_is_not_a_terminal` (every OS), `parity/test_resolver.py::test_null_stdin` (the difference on Windows, equality elsewhere)

### PAR-023: a failing `git log` is an error, not a traceback
- Command: `kluris status` (with and without `--json`)
- Differs: when `git log` fails anywhere but in a fresh repo, v3 stops with ``Error: `git log -10 --format=%H%x00%s%x00%aI%x00` failed (exit 128): <git's message>`` on stderr (the JSON envelope under `--json`), exit 1; v2 ends in a `CalledProcessError` traceback, exit 1, no JSON. Cases tested: HEAD's commit object missing (`fatal: bad object HEAD`), the branch ref replaced by `invalid-oid` (`fatal: your current branch appears to be broken`), an orphan branch beside other history (`fatal: your current branch 'fresh' does not have any commits yet`). Only a fresh repo (HEAD a symbolic ref, no refs, no commit objects) logs as no commits (PAR-015).
- Decision: SPEC decision 2026-10-27 (tracebacks), which names this case; review T-79 finding 1 (T-82)
- Tests: `crates/kluris-core/tests/git.rs::{log_of_a_corrupt_repo_is_an_error_naming_the_command, log_with_a_broken_branch_ref_is_an_error, log_on_an_orphan_branch_of_a_repo_with_history_is_an_error, log_of_a_fresh_repo_with_staged_files_is_empty}`, `parity/test_list_status_help.py::{test_status_of_a_corrupt_repo, test_status_when_head_does_not_verify}`

### PAR-024: `search` reports the files it skips
- Command: `kluris search` (with and without `--json`)
- Differs: a neuron whose frontmatter cannot be read (it does not parse, is not UTF-8, cannot be opened) and a `glossary.md` that does not read are left out of the results, as in v2, and reported, where v2 drops them silently. With `--json` the top level gets `"skipped": [{"file": <path as v2 prints it>, "error": <reason, no absolute path>}]` after `engine`, only when at least one file was skipped (otherwise the output is PAR-020's); in text mode stderr gets one line per file, `Skipped <file>: <reason>`, and stdout is unchanged. The exit code stays 0.
- Decision: SPEC acceptance criterion "One malformed neuron (invalid YAML, non-string key, BOM, `related: [1]`, a link outside the brain) never crashes `dream`, `mri`, `wake-up` or `search`: the file is skipped and reported, `--json` output stays valid JSON, other files are processed"; review T-79 finding 2, chief's brief for T-83, 2026-10-26
- Tests: `crates/kluris-core/tests/search.rs::malformed_neurons_are_skipped_and_reported`, `crates/kluris-cli/tests/search.rs::skipped_files_are_reported_in_json_and_on_stderr`, `parity/test_search.py::test_malformed_neuron_is_skipped_and_reported`

### PAR-025: root `kluris --help` lists only the ported commands (temporary, until Phase 4 ends)
- Command: `kluris --help` (and `kluris` with no arguments, which prints the same list)
- Differs: the Commands section lists only the commands Kluris 3 has ported so far (today `list`, `wake-up`, `search`, `status`, `help`), with v2's one-line descriptions; v2 lists all 13 (with `pack`). `kluris help` (with and without `--json`) lists every command but `pack`, as decided (PAR-018). Temporary: by the end of Phase 4 every command exists and `kluris --help` lists exactly the SPEC acceptance list.
- Decision: SPEC decision 2026-10-27 (tracebacks) row ("a temporary surface difference (root `kluris --help` listing only ported commands until Phase 4 ends) gets its own allowlist entry marked temporary"); SPEC Deviations [IMPL-K3-18]
- Tests: `parity/test_list_status_help.py::test_root_help_flag` (each ported command's v2 line; no `pack`)
