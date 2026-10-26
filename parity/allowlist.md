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
- Tests: `crates/kluris-core/tests/yaml_emit.rs::markdown_framing_matches_python_frontmatter` (rows "indented first line", "blank lines then indented line"); dream: `crates/kluris-cli/tests/dream_resilience.rs::the_first_body_line_keeps_its_indentation`, `parity/test_dream.py::test_first_body_line_keeps_its_indentation` (v2 writes `let x = 1;` without its indentation)

### PAR-003: `windsurf` agent is `devin`; `antigravity` joins the default
- Command: every command that reads a brain's `kluris.yml` and every command that rewrites it (`create`, `register`, `companion add`, `companion remove`, `doctor`)
- Differs:
  - An agent name `windsurf` in `agents.commands_for` reads as `devin`, so a `kluris.yml` written by v3 has `  - devin` where v2 has `  - windsurf`.
  - The default `commands_for` (what `create` and `register` write, and what an empty selection means) is v2's `DEFAULT_AGENT_COMMANDS` in v2's order with `devin` in place of `windsurf`, then `antigravity`: `claude, cursor, devin, copilot, codex, gemini, hermes, kilocode, junie, antigravity`. v2 writes its 9 names with `windsurf`.
  - A `commands_for` equal as a set to v2's pre-Hermes 8 names (v2 migrates these too, to its own 9) or to v2.28's 9 names (with `windsurf` or with `devin`) reads as the v3 default, so a brain created by v2 gets `antigravity` without editing its file; v2 keeps the 9 names. Any other list is kept as written (only `windsurf` -> `devin`), unknown names included, as in v2.
  - Every other byte of `kluris.yml` is v2's. v2.28.11 reads a file with `devin` or `antigravity` without error (it skips names outside its registry).
- Decision: Windsurf became Devin (SPEC decision log 2026-10-24, research-04, interview-01 #15-16; SPEC acceptance "a `windsurf` entry in an existing `kluris.yml` is treated as `devin`"; T-53 brief); default list with `antigravity` and the v2-default migration decided by the chief on T-53 (m-4244), 2026-10-25
- Tests: `crates/kluris-core/tests/config.rs::{defaults_are_v2_defaults_with_devin_then_antigravity, v2_default_lists_migrate_to_the_v3_default, v2_brain_config_round_trips_with_the_v3_default_agents, windsurf_is_read_as_devin_and_unknown_agents_are_kept}`, `parity/test_config_roundtrip.py::test_v2_reads_v3_agent_names`, `parity/test_companions.py` (`companion add`/`remove` rewriting a `kluris.yml` written by v2: `expected_v3`)

### PAR-004: CRLF in stdout and stderr reads as LF
- Command: every command the harness runs, on Windows
- Differs: on Windows v2 (Python text streams) ends each line of stdout and stderr with CRLF where v3 (Rust `println!`) writes LF. The harness decodes both streams of both binaries with every `\r\n` read as `\n` before comparing; a lone `\r` is kept. Files written into brains and configs are compared as bytes, without this.
- Decision: Kluris 3 decision, chief's brief for T-56 (Phase 1, TEST-K3-09 / IMPL-K3-10), 2026-10-25
- Tests: `parity/test_harness.py::test_run_turns_crlf_into_lf`; every parity test on Windows (`cargo xtask windows --parity`)

### PAR-005: config files keep their line endings
- Command: every command that rewrites `~/.kluris/config.yml` or a brain's `kluris.yml` (`create`, `register`, `remove`, `companion add`, `companion remove`, `doctor`)
- Differs: v3 writes an existing config file with the line endings it had (CRLF when its first line break is CRLF, else LF) and a new one with LF on every OS. v2 writes in text mode: LF on macOS/Linux (an existing CRLF file becomes LF) and CRLF on Windows (a new file is CRLF). So on macOS/Linux a CRLF config stays CRLF in v3, and on Windows a config v3 creates is LF.
- Decision: interview-01 #3 (keep each file's line endings; new files LF), SPEC decision log 2026-10-25 (Windows text mode) and 2026-10-26 (T-54 finding 1, fixed in T-59)
- Tests: `crates/kluris-core/tests/config.rs::{global_config_rewrite_keeps_the_existing_line_endings, brain_config_rewrite_keeps_the_existing_line_endings, the_first_line_break_decides_the_endings, write_through_a_symlink_keeps_the_target_endings}`, `parity/test_config_roundtrip.py::test_v2_reads_a_config_rewritten_by_v3[crlf]`, `crates/kluris-core/tests/companions.rs::add_rewrites_a_v2_kluris_yml_keeping_crlf`

### PAR-006: frontmatter YAML read the way PyYAML's pure-Python SafeLoader reads it
- Command: every command that reads markdown neuron frontmatter (`dream`, `search`, `wake-up`, `mri`, `status`); YAML neurons and config files read the same way in both versions
- Differs: v3 reads every YAML text with a port of PyYAML 6.0.3's pure-Python `SafeLoader`. v2 read markdown frontmatter with libyaml's `CSafeLoader` (python-frontmatter picks it when libyaml is installed), which reads some unusual texts otherwise. Measured over 20,000 generated texts (67 differ):
  - libyaml reads a value where `SafeLoader` (and v3) fail: tabs in places the pure scanner refuses (`a: b\tc` is an error in v3, `{'a': 'b\tc'}` in v2), `a?b` inside a flow collection (`[a?b]`), a BOM after the start of the stream, and an empty `!`-tagged node before `:` (`! &a: []`, a `''` key in v2, a null key and so an error in v3).
  - libyaml fails where `SafeLoader` (and v3) read a value: glued flow forms such as `v: {a:[]}`, `{a:{}}`, `{a:,b}`, `{?x:[?x]}`. (It also fails on `[?]` and `"\ud800"`, which `SafeLoader` reads to values outside the v3 model, so both versions fail there.)
  None of the 2,345 markdown files of the seven local brains is in these classes (v3 reads all of them as v2 does).
- Decision: one YAML reading rule for v3, the pure-Python `SafeLoader` (chief's decision on T-67, 2026-10-26, replacing saphyr-parser with a port of PyYAML's front end)
- Tests: `parity/test_yaml_load_differential.py` (counts the texts where `CSafeLoader` reads otherwise), the `// CSafeLoader:` notes on the rows of `crates/kluris-core/tests/yaml_read.rs::documents`

### PAR-007: deprecation check skips a neuron it cannot read
- Command: the deprecation check behind `kluris wake-up --json` (`deprecation`, `deprecation_count`) and `kluris dream` (deprecation warnings, with and without `--json`); `kluris_core::brain::deprecation::detect_deprecation_issues`
- Differs: a markdown neuron whose frontmatter cannot be read (it does not parse, is not UTF-8, cannot be opened, or is a broken symlink that points inside the brain) is left out: v3 reports the issues of the other neurons, as v2's own rules give them over those neurons, and lists the left-out files (`DeprecationReport::skipped`). v2's `detect_deprecation_issues` raises on the first such file, so v2's wake-up reports `deprecation: []` and `deprecation_count: 0` for the whole brain and v2's dream ends with a traceback (research-03 §5 item 15). A YAML neuron whose `#---` block does not parse is no difference: both read it as no metadata (active). How wake-up and dream present the skipped files is decided when they are ported.
- Decision: "a malformed neuron never stops the walk: it is returned as a per-file error value so callers can skip and report it" (chief's brief for T-75, Kluris 3 Phase 2, TEST-K3-11 / IMPL-K3-12), 2026-10-26
- Tests: `crates/kluris-core/tests/brain_walk.rs::{deprecation_skips_malformed_neurons_and_reports_them, deprecation_with_symlinks_matches_v2}`, `parity/test_brain_model.py::test_fixture_brain_model_matches_v2[malformed]` and `[links]` (v2's rules run with `preparsed=` over the neurons that read); wake-up: `crates/kluris-core/tests/wake_up.rs::malformed_neurons_are_skipped_and_counted`, `parity/test_wake_up.py::{test_malformed_fixture_brain, test_fixture_brain[links]}` (v2's `deprecation: []` against v3's issues of the neurons that read)

### PAR-008: neurons in path order, not directory-read order
- Command: every output whose order follows v2's neuron walk: `kluris wake-up --json` `deprecation[]`, `kluris dream` (`deprecation[]`, `broken_synapses_detail[]`, `--broken-only`'s `broken_synapses[]`, the text's deprecation lines, and the `related` lists its reverse-link fix writes into neurons), `kluris mri` (the graph's node ids and the order of its `nodes` and `edges`); `kluris_core::brain::walk::walk` and `neuron_index::neuron_files`
- Differs: v3 lists files sorted by the bytes of their `/`-separated brain-relative path, and neurons as markdown then YAML, each in that order. v2 keeps `os.walk`'s directory-read order (markdown then YAML), which depends on the filesystem and differs between machines. So the same set of entries can come out in another order; parity compares these lists as sets or multisets (research-03 §3.2), and an order that v2 itself sorts (search results, `recent`) is not affected. In brain files: when one dream adds two or more reverse `related` links to the same neuron, v3 appends them in its path order and v2 in readdir order (on APFS, sources `zeta, alpha, mid, Beta, _u, a-b, a.b` gave v2 `alpha, mid, _u, a-b, a.b, zeta, Beta`, v3 `Beta, _u, a-b, a.b, alpha, mid, zeta`); same entries, other bytes.
- Decision: one deterministic walker for every command (chief's brief for T-75, Kluris 3 Phase 2), 2026-10-26; the reverse `related` order in brain files: chief's answer in T-89 (m-4496), citing the same decision
- Tests: `crates/kluris-core/tests/brain_walk.rs::{walker_order_is_by_path_bytes, neuron_index_matches_v2_on_the_full_brain}`, `parity/test_brain_model.py` (neuron sets; deprecation in v3's order against v2's rules in that order), `parity/test_wake_up.py` (`deprecation[]` as a multiset, `parity/runs.py::assert_wake_up_equal`); dream: `crates/kluris-core/tests/dream.rs::reverse_links_are_added_in_path_order`, `parity/test_dream.py` (the JSON lists and text lines as multisets; a `related` list holding v2's entries in another order is taken in v3's order by `parity/tools/dream_oracle.py`; the `knowledge/src/*` sources of `test_three_runs_converge_like_v2`)

### PAR-009: `wake-up` counts the neurons it cannot read in `parse_errors`
- Command: `kluris wake-up --json`
- Differs: `parse_errors` is the number of markdown neurons whose frontmatter v3 cannot read (it does not parse, is not UTF-8, cannot be opened, or is a broken symlink that points inside the brain); those neurons are left out of `recent` and `deprecation` (PAR-007). v2's CLI always prints `parse_errors: 0` (only its pack's boot snapshot counts). YAML neurons are never counted: both versions read a `#---` block that does not parse as no metadata. Where both readers fail (a timestamp PyYAML cannot construct such as `2026-02-30`, a non-string key) the only difference is the count. Frontmatter that v2 reads but v3's reader refuses as an error value (a `<<` merge key, `!!binary`, `!!set`) also counts, and those neurons drop out of `recent` and `deprecation` in v3. Text output does not show the count, as in v2.
- Decision: skip and report malformed neurons (chief's brief for T-76, Kluris 3 Phase 2, TEST-K3-13 / IMPL-K3-14, 2026-10-26; SPEC acceptance "one malformed neuron never crashes ... `wake-up`"); the refused YAML forms: TEST-K3-03
- Tests: `crates/kluris-core/tests/wake_up.rs::malformed_neurons_are_skipped_and_counted`, `parity/test_wake_up.py::{test_malformed_fixture_brain, test_fixture_brain[links], test_odd_updated_values, test_frontmatter_v3_refuses}`

### PAR-010: markdown frontmatter behind a UTF-8 BOM is read
- Command: every command that reads markdown frontmatter; today `kluris wake-up --json` (`recent`, `brain_md`, lobe descriptions, deprecation) and `kluris mri` (a node's frontmatter fields, body, title, excerpt and edges)
- Differs: a markdown file that starts with a UTF-8 BOM and then a `---` block is read with its frontmatter, the BOM dropped. v2 (python-frontmatter) sees no frontmatter there: empty metadata, and the whole text, BOM and `---` lines included, as the body. So in wake-up such a neuron's `updated` puts it in v3's `recent` and not in v2's, brain.md's `brain_md` is its body without the BOM and frontmatter in v3 (v2: `"\ufeff---\n..."`), and a map.md `description` in that frontmatter is used by v3 (v2 falls back to the body).
- Decision: SPEC TEST-K3-03 ("UTF-8 BOM (read correctly)"), research-01 §1 (the BOM bug); the dream side (one block, BOM kept) is PAR-038
- Tests: `crates/kluris-core/tests/frontmatter_read.rs::{parse_bom_is_detected_and_frontmatter_still_read, read_bom_file}`, `parity/test_wake_up.py::test_bom_frontmatter_is_read`; dream: `crates/kluris-cli/tests/dream_resilience.rs::a_bom_file_keeps_its_bom_and_gets_one_block`, `parity/test_dream.py::test_bom_file_keeps_one_block`

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
- Command: `kluris status` (`uncommitted`, with and without `--json`); the git layer's `status_short` and `file_dates` (the date sync of `kluris dream` and of `kluris mri`'s preflight)
- Differs: v3 runs git with `-c core.quotePath=false`, so `uncommitted` has `?? projects/é.md` where v2 has `?? "projects/\303\251.md"` (git's octal C-quoting of every byte over 0x7f). A name git quotes for another reason (a space, `"`, `\`, a control character) is quoted in both. The file-dates log reads NUL-separated names (`-z`), never quoted: v2's quoted names never matched a neuron, so their dates were not refreshed.
- Decision: interview-01 #5 ("git dates missing ... non-ASCII names"; the same git-layer fix reaches `status`, and git-layer fixes cite #5 per SPEC decision 2026-10-27 (tracebacks)); research-03 §5 item 26; T-78
- Tests: `crates/kluris-core/tests/git.rs::{status_shows_non_ascii_names_unquoted, file_dates_keep_non_ascii_and_odd_names_as_written}`, `parity/test_list_status_help.py::test_status_non_ascii_names`, `crates/kluris-core/tests/dream.rs::date_sync_uses_brain_relative_paths_in_a_nested_repo_with_non_ascii_names`

### PAR-014: a brain nested in a larger repo sees only itself
- Command: `kluris status` (`recent_commits`, `uncommitted`, the text); the git layer's `file_dates` (the date sync of `kluris dream` and of `kluris mri`'s preflight)
- Differs: when the brain directory is a subdirectory of the work tree (`git rev-parse --show-prefix` is not empty), v3 runs `git log -10 ... -- .` and `git status --short -- .` in the brain, so `recent_commits` lists only commits that touched the brain and `uncommitted` only its own changes, brain-relative (`M projects/alpha.md`). v2 listed the whole outer repo (`outer: code`, `M ../../README.md`). The file-dates log runs with `--relative`, so its paths are brain-relative (`a.md`, v2: `brain/a.md`, which never matched a neuron). A brain at the repo root runs v2's exact commands (no pathspec, which would drop empty commits).
- Decision: interview-01 #5 ("git dates missing nested-repo brains"; `status` is scoped by the same git-layer fix, and git-layer fixes cite #5 per SPEC decision 2026-10-27 (tracebacks)); research-03 §5 item 26; T-78
- Tests: `crates/kluris-core/tests/git.rs::{log_of_a_nested_brain_lists_only_its_own_commits, status_of_a_nested_brain_is_scoped_and_brain_relative, file_dates_of_a_nested_brain_are_brain_relative, log_keeps_empty_commits_at_the_repo_root}`, `parity/test_list_status_help.py::test_status_nested_brain_is_scoped_to_the_brain`, `crates/kluris-core/tests/dream.rs::date_sync_uses_brain_relative_paths_in_a_nested_repo_with_non_ascii_names`

### PAR-015: `status` of a repo without commits
- Command: `kluris status` (with and without `--json`)
- Differs: in a git repo with no commit yet, v3 reports the brain with `git_enabled: true`, `recent_commits: []` and its `uncommitted` text, and exits 0. v2's `git log` fails there (no `HEAD`): a `CalledProcessError` traceback, exit 1, no JSON.
- Decision: SPEC decision 2026-10-27 (tracebacks), which names this case; found in T-78
- Tests: `crates/kluris-core/tests/git.rs::log_of_a_repo_without_commits_is_empty`, `parity/test_list_status_help.py::test_status_of_a_repo_without_commits`

### PAR-016: user text in human output is never markup
- Command: every command's human (non-JSON) output; today `kluris list`, `kluris status`, `kluris help`, `kluris wake-up`
- Differs: v3 prints names, paths, descriptions, commit subjects and other user text as they are. v2 passed them to Rich as markup: `see [red]docs[/red]` printed `see docs`, and a stray closing tag (`ends [/oops]`) raised `MarkupError` (traceback, exit 1) in `list`.
- Decision: SPEC decision 2026-10-27 (no markup in user text); the `MarkupError` crash also falls under SPEC decision 2026-10-27 (tracebacks); research-03 §5 item 12
- Tests: `crates/kluris-cli/src/output/human.rs::tests::user_text_is_never_markup`, `parity/test_list_status_help.py::{test_list_and_status_print_user_text_literally, test_list_with_a_closing_tag_that_crashed_v2}`

### PAR-017: `help --json COMMAND` describes that command
- Command: `kluris help COMMAND --json`
- Differs: v3 prints `{"ok": true, "commands": [{"name": ..., "description": ...}]}` with that one command, and for a name not in the table the envelope `{"ok": false, "error": "Unknown command: <name>"}` with exit 1 (as `help <name>` without `--json` fails in both). v2 ignored COMMAND under `--json`: the whole table, exit 0. `help --json` without COMMAND is v2's table (less `pack`, PAR-018).
- Decision: SPEC decision 2026-10-27 (help --json COMMAND); research-03 §5 item 20
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
- Differs: the Commands section lists only the commands Kluris 3 has ported so far (today `list`, `wake-up`, `search`, `status`, `companion`, `dream`, `mri`, `help`), with v2's one-line descriptions; v2 lists all 13 (with `pack`). `kluris help` (with and without `--json`) lists every command but `pack`, as decided (PAR-018). Temporary: by the end of Phase 4 every command exists and `kluris --help` lists exactly the SPEC acceptance list.
- Decision: SPEC decision 2026-10-27 (tracebacks) row ("a temporary surface difference (root `kluris --help` listing only ported commands until Phase 4 ends) gets its own allowlist entry marked temporary"); SPEC Deviations [IMPL-K3-18]
- Tests: `parity/test_list_status_help.py::test_root_help_flag` (each ported command's v2 line; no `pack`), `parity/test_mri.py::test_mri_is_in_the_root_help`

### PAR-026: SKILL.md placeholder fixes
- Command: every command that installs agent skills (`create`, `register`, `remove`, `companion add`, `companion remove`, `doctor`); today `companion add`, `companion remove` and `cargo xtask skills-install`
- Differs: (1) the "Fix broken synapses" step reads ``a list of `{brain, file, target}` entries.``, where v2 leaks the literal `{{brain, file, target}}` (its template is filled with `str.replace`, not `.format`); (2) placeholder values are substituted in one pass, so a description, brain path or companion path containing `{brain_path}`, `{brain_flag_hint}`, `{specmint_block}`, `{git_label}` or any other placeholder name is written as it is, where v2 replaces the placeholders one after another and expands those found inside an earlier value. Without such a value, (2) changes no byte.
- Decision: interview-01 #4 ("the literal `{{brain, file, target}}` in every SKILL.md"), SPEC decision 2026-10-24 (fix the v2 bugs of interview-01 #4-#6); research-03 §5 #13
- Tests: `crates/kluris-core/tests/agents_render.rs::{every_case_is_v2_with_exactly_the_decided_changes, rendered_skill_has_no_leaked_double_braces, placeholders_inside_values_are_not_expanded}`, `parity/test_skill_install.py::test_v2_home_is_migrated`, `parity/test_companions.py` (the skills `companion add`/`remove` re-render, via `test_skill_install._v2_to_v3`)

### PAR-027: SKILL.md carries a Kluris marker line
- Command: as PAR-026
- Differs: after the frontmatter's closing `---` and its blank line, v3 writes ``<!-- kluris:generated -- Kluris writes this skill and `kluris doctor` rewrites it; edits here are lost. -->`` and a blank line, then v2's `# Brain: <name>`. The frontmatter (`name: kluris-<brain>`, the quoted description) is v2's byte for byte, so every agent's rules (research-04: `name` equals the folder, lowercase/digits/hyphens, at most 64 characters, description at most 1,024) hold as in v2. Installs recognise Kluris-written skills by this line (a prefix match on `<!-- kluris:generated`) or by v1/v2's frontmatter shape (PAR-031).
- Decision: interview-01 #6 ("the sweep deletes only Kluris-generated skills (marked in SKILL.md)"); SPEC acceptance criterion "Generated SKILL.md files ... carry a Kluris marker comment"
- Tests: `crates/kluris-core/tests/agents_render.rs::{marker_follows_the_frontmatter, frontmatter_fits_every_agent, v3_and_v2_skills_are_recognised, foreign_skills_are_not_recognised}`, `parity/test_companions.py` (the skills `companion add`/`remove` re-render, via `test_skill_install._v2_to_v3`)

### PAR-028: SKILL.md's search advice is the "Search" paragraph
- Command: as PAR-026
- Differs: in "## Query first -- never guess", v2's two paragraphs from ``Start with the cached wake-up snapshot. For targeted lookup, prefer`` through ``knowledge was superseded.`` (v2 `agents.py:SKILL_BODY` lines 38-46) are replaced by: the sentence ``Start with the cached wake-up snapshot. For targeted lookup, search the brain.``, a `### Search` heading and three paragraphs: run ``kluris search "<2-5 distinctive words>" --brain <name> --json`` with keywords, not a sentence, 2-3 phrasings in parallel (synonyms, glossary terms, the brain's language), v2's `--lobe`/`--tag`/`--limit 20` advice; read `title`, `heading` and `snippet` first and open files at `line`, a top hit with `coverage` under about 0.5 and no `title`/`heading` match is weak, follow `related`, read `replaced_by` for a `deprecated` hit; if every search is weak navigate `brain_md` + `lobes[]` -> `map.md` -> neurons (v2's fallback), say "Nothing documented about X yet." and do not stretch a loose match. research-05's en dash is written `2-3` to keep the file ASCII outside user values, like v2's. Every other line, including the `**Search**` intent, the CLI command list and v2's write flow (`git add -A && git commit`), is v2's.
- Decision: SPEC decision 2026-10-24 (search: "agents send 2-3 phrasings in parallel (rung 2, skill text only)"), research-05 §6 "New skill 'Search' paragraph"; SPEC decision 2026-10-27 (v3.0's skill write flow stays v2's)
- Tests: `crates/kluris-core/tests/agents_render.rs::{every_case_is_v2_with_exactly_the_decided_changes, search_paragraph_names_the_search_json_keys}`, snapshots `crates/kluris-core/tests/snapshots/agents_render__*.snap`, `parity/test_skill_install.py::test_v2_home_is_migrated`, `parity/test_companions.py` (the skills `companion add`/`remove` re-render, via `test_skill_install._v2_to_v3`)

### PAR-029: SKILL.md is LF on every OS
- Command: as PAR-026, on Windows
- Differs: v3 writes every SKILL.md with LF line endings; v2 writes it in Python text mode, CRLF on Windows. Every copy of a skill is byte-identical on every OS.
- Decision: interview-01 #3 and SPEC decision 2026-10-25 (Windows text mode: "new files LF on every OS"); a SKILL.md is regenerated whole on every install
- Tests: `parity/test_skill_install.py::test_v2_home_is_migrated` (reads v2's copy with CRLF as LF), `crates/kluris-core/tests/agents_render.rs::crlf_copies_are_recognised`, `parity/test_companions.py` (the skills `companion add`/`remove` re-render, via `test_skill_install._v2_to_v3`)

### PAR-030: the skill install matrix
- Command: as PAR-026
- Differs: v3 writes `kluris-<brain>/SKILL.md` for every registered brain to `~/.agents/skills` always, plus `~/.claude/skills` when `claude` is selected, `~/.hermes/skills` and every `~/.hermes/profiles/*/skills` when `hermes` is, `~/.gemini/config/skills` when `antigravity` is, and `$COPILOT_HOME/skills` when `copilot` is selected and `COPILOT_HOME` is set. v2 writes one folder per agent (`~/.cursor/skills`, `~/.codex/skills`, `~/.codeium/windsurf/skills`, `~/.copilot/skills`, `~/.gemini/skills`, `~/.kilo/skills`, `~/.junie/skills` too) and Windsurf workflows (`~/.codeium/windsurf/global_workflows/kluris-<brain>.md`) whether or not Windsurf is selected; v3 writes none of these (cursor, codex, gemini, devin/windsurf, kilocode and junie are served by `~/.agents/skills`). The selection is v2's (the union of `commands_for` over brains with a `kluris.yml`; empty means every registry agent; unknown names are skipped), with v3's default list (antigravity added, config PAR entries). v3 creates only the folders it writes (v2 runs `mkdir -p` for every agent). Each folder is staged then renamed skill by skill; an existing Kluris-written copy is moved aside and removed once the new one is in place; a same-named folder Kluris did not write is left as it is and reported (v2 replaces it).
- Decision: SPEC decisions 2026-10-24 (skills `~/.agents/skills`-first; devin replaces windsurf, no workflows) and 2026-10-25 (install matrix confirmed by research-07; Antigravity gets one folder); interview-01 #6, #15, #16; research-03 §5 #22
- Tests: `crates/kluris-core/tests/skill_install.rs` (every matrix row, `COPILOT_HOME`, Hermes profiles, union, byte-identical copies, foreign same-named folder, failed destination), `parity/test_skill_install.py`, `parity/test_companions.py` (the skills `companion add`/`remove` re-render, via `test_skill_install._v2_to_v3`)

### PAR-031: the install sweep removes only Kluris-written skills
- Command: as PAR-026
- Differs: v2 deletes every `kluris*` entry in each agent's skills folder and legacy command folders, a user's own `kluris-notes` included, and never touches agents left out of `commands_for`. v3 removes a `kluris*` entry only when Kluris wrote it: a real folder whose `SKILL.md` frontmatter opens with `name: <folder name>` and either has the PAR-027 marker line or Kluris's description (`You are the team's subject matter expert`, quoted or not: every v1/v2 rendering), or, in Windsurf `global_workflows`, a `kluris*.md` file opening with that description. Removed: in written folders (`~/.agents/skills` included), the skills of brains no longer registered; in the folders of deselected registry agents, every Kluris-written skill; and, after `~/.agents/skills` is written (skipped if that fails), v2's copies in `~/.codeium/windsurf*/skills` and `global_workflows`, `~/.cursor/skills`, `~/.codex/skills`, `~/.copilot/skills` (unless `COPILOT_HOME` is set), `~/.gemini/skills`, `~/.kilo/skills`, `~/.kilocode/skills`, `~/.junie/skills`. Folders are never removed, and v2's legacy command folders (`~/.claude/commands`, ...) are not looked at.
- Decision: interview-01 #6 (sweep only Kluris-generated skills; skills for deselected agents removed); SPEC acceptance criteria (only marked skills swept; `~/.agents/skills` never swept of anything else; the v2 migration list) and decision 2026-10-25 (v2's per-agent copies cause real duplicates, research-07); research-03 §5 #22, #23
- Tests: `crates/kluris-core/tests/skill_install.rs::{deselected_agents_lose_their_copies_and_keep_their_folders, removed_brains_lose_their_skills_everywhere, a_users_own_kluris_skill_is_never_touched, v2_install_is_migrated_and_nothing_else_is_touched, copilot_folder_is_kept_when_copilot_home_is_set, migration_waits_for_the_universal_copy}`, `parity/test_skill_install.py::{test_v2_home_is_migrated, test_copilot_home_keeps_the_copilot_folder}`, `parity/test_companions.py` (the skills `companion add`/`remove` re-render, via `test_skill_install._v2_to_v3`)

### PAR-032: a brain whose `kluris.yml` does not read is reported by the install
- Command: as PAR-026
- Differs: a registered brain whose `kluris.yml` exists but does not read or validate contributes no agents to the selection and renders without companions; the install goes on and reports `(brain, error)`. v2's `_do_install` raises (`read_brain_config` is not guarded there), a traceback.
- Decision: SPEC decision 2026-10-27 (every v2 traceback becomes a correct result or a clean error, each allowlisted)
- Tests: `crates/kluris-core/tests/skill_install.rs::a_brain_with_an_invalid_kluris_yml_is_reported_and_still_rendered`

### PAR-033: dates in rewritten frontmatter are always quoted
- Command: `kluris dream` (with and without `--json`, `--broken-only`, `--brain all`) and `kluris mri`'s preflight: every neuron they rewrite
- Differs: v3 writes every top-level date and datetime of a neuron it rewrites as its quoted ISO text: `created: '2026-01-01'`, `when: '2026-01-02T10:00:00+00:00'`. v2 does so only on its date path (`_sync_brain_state`'s `preloaded=` write). Its parent and reverse-link fixes (`fix_missing_frontmatter`, `fix_bidirectional_synapses`) reload a markdown neuron raw and write `created: 2026-01-01` and `when: 2026-01-02 10:00:00+00:00` unquoted. The file stays that way until a later date pass rewrites it, which never happens while `updated` already equals the git date. YAML neurons and map/brain files are no difference: v2 writes them quoted too. Dates nested in a list or mapping (`nested: [2026-01-03]`) are written unquoted by both. The values read back the same in both.
- Decision: interview-01 #2 (always write the converged, quoted form); SPEC decision log 2026-10-24 ("Always write quoted dates")
- Tests: `crates/kluris-core/tests/dream.rs::missing_parents_are_inferred_below_the_root_with_quoted_dates`, `parity/test_dream.py` (every file whose bytes differ must equal `parity/tools/dream_oracle.py`'s rewrite of v2's file through v2's own reader and dream-path writer; `projects/no-parent.md` in `test_three_runs_converge_like_v2`)

### PAR-034: YAML anchors and aliases are written as plain copies
- Command: `kluris dream` and `kluris mri`'s preflight: every neuron they rewrite
- Differs: a frontmatter value written with an anchor and alias (`base: &x [a, b]` / `copy: *x`) is written by v3 as two plain copies (`base:\n- a\n- b\ncopy:\n- a\n- b`). v2 writes `base: &id001` ... `copy: *id001` on both of its write paths. The SPEC row says v2's date path writes plain copies, but running the oracle in T-89 showed that path writing `&id001` too. The values read back the same.
- Decision: SPEC decision log 2026-10-25 (YAML anchors/aliases in user frontmatter are written as plain copies; v3's value model holds no shared references)
- Tests: `crates/kluris-core/tests/dream.rs::missing_parents_are_inferred_below_the_root_with_quoted_dates`, `parity/test_dream.py::test_three_runs_converge_like_v2` (`projects/agent.md`; the expected bytes come from `parity/tools/dream_oracle.py`, which un-shares v2's values before writing)

### PAR-035: brain files keep their line endings; new files are LF
- Command: `kluris dream` and `kluris mri`'s preflight: every neuron, `map.md` and `brain.md` they write
- Differs: v3 writes an existing file with the line endings it had (CRLF when its first line break is CRLF, else LF), and a file it creates (a new `map.md`, `brain.md`) with LF, on every OS.
  - macOS/Linux: v2 writes markdown LF, so a CRLF neuron or map becomes LF. A CRLF YAML neuron gets v2's LF `#---` block before its CRLF body; v3 writes the block CRLF.
  - Windows: v2 writes markdown in text mode, so every markdown file it writes is CRLF, new files and LF files included. v3 keeps LF files LF and creates LF files.
- Decision: interview-01 #3 (keep each file's line endings; new files LF); SPEC decision log 2026-10-24 and 2026-10-25 (Windows text mode)
- Tests: `crates/kluris-core/tests/dream.rs::files_keep_their_line_endings_and_new_files_are_lf`, `crates/kluris-cli/tests/dream_resilience.rs::crlf_files_stay_crlf_and_new_files_are_lf`, `parity/test_dream.py::test_crlf_file_stays_crlf` and `knowledge/crlf.md` in `test_three_runs_converge_like_v2` ( the expected bytes use the file's endings before the run, or LF for a new file, so Windows runs compare against the same rule)

### PAR-036: no `map.md` in pruned directories, and none listed
- Command: `kluris dream` and `kluris mri`'s preflight
- Differs: v3 writes `map.md` only into the directories its one walker descends into, so never into a dot directory (`.github/`, `.obsidian/`, `.archive/`), `node_modules/`, `__pycache__/` or any other `SKIP_DIRS` name, at any depth. A map never lists such a directory as a sub-lobe (`- [.archive/](./.archive/map.md)`) or sibling (`sideways [.archive](../.archive/map.md) | ...`, `siblings: [../.archive/map.md, ...]`), even when a `map.md` an earlier v2 dream wrote is still there. v3 leaves those old files alone. v2 walks every directory but `.git`, writes a `map.md` into each one, and lists any child directory with a `map.md` except the `SKIP_DIRS` names. On real data: consensflow-sme has a top-level `.archive/` with v2-written maps, so v3's first dream rewrites its 8 top-level lobe maps (the `.archive` sibling taken out, `updated` today). A later v2 dream would add it back while both versions are in use.
- Decision: interview-01 #4 (`map.md` written into hidden and tooling folders; one shared walker); SPEC decision log 2026-10-27 (pruned dirs; chief's answer in T-89, m-4496)
- Tests: `crates/kluris-core/tests/dream.rs::{pruned_directories_get_no_map_and_are_never_listed, maps_and_brain_md_are_generated_like_v2}`, `parity/test_dream.py::{test_pruned_directories, test_registered_brains}` (v3's map equals v2's with the pruned directories taken out of `siblings`, `sideways` and Contents, and `updated` today when that changed the body)

### PAR-037: report paths are `/`-joined on Windows too
- Command: `kluris dream --json` (`deprecation[]` `file`/`source`/`target`, `broken_synapses_detail[]` and `--broken-only`'s `broken_synapses[]` `file`) and `kluris wake-up --json` (`deprecation[]`); Windows only
- Differs: v3 writes these brain-relative paths with `/` on every OS (`knowledge/old.md`); v2 writes `str(Path)`, native on Windows (`knowledge\\old.md` in the JSON). A `replaced_by` target and a broken link's `target` are as written in both. The text output keeps v2's native paths.
- Decision: interview-01 #7 (JSON paths always use forward slashes, Windows included); chief's answer in T-89 (m-4496: apply #7 at the source, one Windows-only entry for dream and wake-up)
- Tests: `parity/test_dream.py::test_windows_paths_in_json_use_forward_slashes`, `parity/runs.py::slash_paths` (v3 has no backslash on any OS; on Windows v2's paths are compared in `/` form), used by `parity/test_dream.py` and `parity/test_wake_up.py`; `crates/kluris-core/tests/{brain_walk,wake_up,dream}.rs` (the report types hold `/`-joined strings)

### PAR-038: dream skips and reports a file it cannot read
- Command: `kluris dream` (with and without `--json`, `--broken-only`, `--brain all`)
- Differs:
  - Dream does not rewrite a neuron whose frontmatter does not parse: invalid YAML, a non-string key, a YAML neuron's `#---` block. It also leaves one that is not UTF-8, cannot be read or cannot be written. The other files are processed.
  - The validators and the deprecation check skip that file too, so it gets no other report (PAR-007).
  - A skipped file makes the brain unhealthy: `healthy: false` and exit 1, like v2's exit 1.
  - With `--json`, both the full and the `--broken-only` output end, after v2's keys, with `"skipped": [{"brain", "file", "error"}]`. The key is there only when a file was skipped. `file` is `/`-joined; `error` is the reason with no absolute path (`invalid frontmatter: ...`, `the file is not valid UTF-8`).
  - Without `--json`, each brain's report is followed by one line on stderr per skipped file: `Skipped <file>: <reason>`, or under `--brain all` `Skipped <brain>: <file>: <reason>`.
  - v2's date pass swallows the error, but its parent fix raises: a traceback, exit 1, no JSON, the date writes already on disk. Invalid YAML is a `yaml` error, a non-string key `TypeError: keywords must be strings`, invalid UTF-8 `UnicodeDecodeError`.
  - For a YAML neuron whose block does not parse, v2 exits 0 and writes a second `#---` block above the old one.
  - A markdown neuron behind a UTF-8 BOM is read with its frontmatter (PAR-010) and rewritten with its BOM and one frontmatter block. v2 sees no frontmatter there and prepends a second block.
- Decision: SPEC decision log 2026-10-25 (a neuron whose frontmatter does not parse is skipped and reported by `dream`) and 2026-10-26 (dream uses a parse-failed flag for YAML neurons); interview-01 #4 (BOM duplicate frontmatter; one bad neuron crashing dream); SPEC decision 2026-10-27 (tracebacks); the chief's answer in T-94 (m-4524: `{brain, file, error}`, the stderr lines, unhealthy)
- Tests: `crates/kluris-cli/tests/dream_resilience.rs::{malformed_files_are_skipped_reported_and_left_alone, broken_only_lists_the_skipped_files_too, text_mode_prints_one_skipped_line_per_file_on_stderr, skipped_lines_name_the_brain_under_brain_all, no_skipped_key_when_nothing_was_skipped, a_bom_file_keeps_its_bom_and_gets_one_block}`, `crates/kluris-core/tests/dream.rs::every_write_is_listed_once_and_a_neuron_that_does_not_parse_is_skipped`, `parity/test_dream.py::{test_malformed_neuron_v2_crashes_v3_skips_and_reports, test_yaml_neuron_whose_block_does_not_parse, test_bom_file_keeps_one_block}`

### PAR-039: `companion remove` rewrites only the brains that had the companion
- Command: `kluris companion remove [NAME] [--brain NAME|all] [--json]`
- Differs: v3 rewrites the `kluris.yml` of a resolved brain only when its `companions` list holds one of the removed names, and reports only those brains (`"brains"` in the JSON, the brain list in `Removed companion(s) <names> from: <brains>`); a brain without them keeps its file byte for byte (comments, unknown keys and v2's agent names included). v2 rewrites every resolved brain (dropping comments and unknown keys, normalizing its list) and reports all of them. When no resolved brain had any of them, the JSON has `"brains": []` and the text reads `Removed companion(s) <names> from: (none)`; the skill install still runs, as in v2. `companion add` is unchanged (every resolved brain is rewritten and reported, as in v2).
- Decision: research-03 §5 #25 ("rewrites kluris.yml of brains that never had the companion and reports them as changed"), fixed as a wrong result under interview-01 #5; chief's brief for T-91, 2026-10-26
- Tests: `crates/kluris-core/tests/companions.rs::remove_touches_only_brains_that_had_the_companion`, `parity/test_companions.py::{test_remove_all_touches_only_brains_that_had_it, test_remove_picker_with_several_enabled}`

### PAR-040: a brain whose `kluris.yml` does not read stops `companion add|remove` before any write
- Command: `kluris companion add|remove` with a companion selected (by NAME or the picker)
- Differs: v3 reads every resolved brain's `kluris.yml` before writing anything; when one is missing or does not read or validate, it stops with `Error: Cannot access config file <path>: <reason>` or `Error: Invalid config file <path>: <reason>` on stderr (the JSON envelope under `--json`), exit 1, and nothing changes: no companion folder installed, no `kluris.yml` written, no skill install. v2 installs the companion folders, rewrites the brains before the broken one, then ends in a `FileNotFoundError`/YAML traceback (exit 1, no JSON) and skips its skill install. Where v2 reads no config (an empty selection, the "name required" error, the remove picker's list of enabled companions, which skips such a brain as v2 does) nothing differs.
- Decision: SPEC decision 2026-10-27 (every v2 traceback becomes a correct result or a clean Kluris error, allowlisted); v3 rule "every write is one change set"
- Tests: `crates/kluris-core/tests/companions.rs::load_brains_reads_every_kluris_yml_first`, `parity/test_companions.py::test_a_brain_whose_kluris_yml_does_not_read`

### PAR-041: installed companion files are written, not copied with their metadata
- Command: `kluris companion add` (and every later companion refresh)
- Differs: the files under `~/.kluris/companions/<name>/` are byte for byte v2's (the embedded `vendored/` tree), written with the install time as modification time and the default file mode; v2 copies them with `shutil.copy2`, keeping the installed package's modification times and mode bits. Folder layout, allowlist and the stage-then-rename are v2's.
- Decision: SPEC architecture ("companions (embedded)", technology table: `include_dir` for the companion folders) and research-03 §2.10 ("Rust must embed these four folders"): an embedded tree has no file metadata to copy
- Tests: `crates/kluris-core/tests/companions.rs::{every_embedded_companion_installs_as_vendored, embedded_trees_equal_vendored_byte_for_byte}`, `parity/test_companions.py` (the installed tree compared by bytes)

### PAR-042: the MRI graphs only what the shared walker sees
- Command: `kluris mri` (the graph in the page, its header counts, and `nodes`/`edges` in the JSON)
- Differs: v3 builds the graph over the one walker every command uses, so no file under a dot directory (`.archive/`, `.obsidian/`, `.github/`), `node_modules/`, `__pycache__/` or another `SKIP_DIRS` name, and no symlink that resolves outside the brain (or loops), is a node, and links to them make no edges. v2's MRI walk prunes only `.git` and graphs all of those, so its graph has more nodes and edges, and the nodes it shares with v3 can have a higher `degree` and more edges. Every other node and edge is v2's. On real data: consensflow-sme, whose `.archive/` holds most of its files, is 615 nodes and 2,629 edges in v3 and 1,534 nodes and 3,582 edges in v2; the six other local brains are identical.
- Decision: interview-01 #4 (one shared walker; research-03 §5 items 1-2: the MRI walker graphs `node_modules`, hidden directories and symlinks that escape the brain); chief's brief for T-95 ("the MRI no longer graphs dot-dirs, node_modules or symlinks escaping the brain"); SPEC decision log 2026-10-26 (one deterministic walker) and 2026-10-27 (pruned dirs)
- Tests: `crates/kluris-core/tests/mri.rs::pruned_directories_and_escaping_symlinks_are_not_graphed`, `parity/test_mri.py::{test_pruned_directories_and_escaping_symlinks, test_registered_brains}` (v2's graph without the pruned files, their edges, and the degrees and edge counts recomputed, equals v3's)

### PAR-043: neuron text cannot end the page's script
- Command: `kluris mri` (the page)
- Differs: in the graph JSON injected into the page's `<script>`, v3 writes every `</` as `<\/` and every `<!--` as `\u003c!--`; both are escapes inside JSON strings, so the graph reads back the same. v2 injects `json.dumps(graph, indent=2)` as is, so a neuron containing `</script>` ends the script early and the page breaks (research-03 found 4 such occurrences in one generated page). `<!--` is escaped too because, followed by `<script>`, it puts the HTML parser in the state where the page's own `</script>` no longer closes the element.
- Decision: interview-01 #4 (v2 crash/damage bugs are fixed); research-03 §5 item 16; chief's brief for T-95 ("escape `</` as `<\/` in the injected JSON")
- Tests: `crates/kluris-core/tests/mri.rs::neuron_text_cannot_end_the_script`, `parity/test_mri.py::test_mri_rich_brain` (v3's page has one `</script>`, v2's more; every page's JSON must equal `json.dumps(indent=2)` of its parse with these two escapes)

### PAR-044: the page is named after the registered brain
- Command: `kluris mri` (the page's `<title>`, its header and breadcrumb, and the script's `BRAIN_NAME`)
- Differs: v3 uses the brain's registered name (the `--brain` name) in all four places; v2 uses the brain directory's name (`brain_path.name`). They differ when the directory is named otherwise (`<title>calm-brain Brain MRI</title>` where v2 has `<title>Some Folder Brain MRI</title>`). A registered name is always a valid brain name, so it needs no HTML escaping, which v2's directory name never got.
- Decision: research-03 §5 item 17; chief's brief for T-95 ("the page title is the registered brain name, not the directory name")
- Tests: `parity/test_mri.py::test_page_is_named_after_the_registered_brain`, `crates/kluris-core/tests/mri.rs::the_page_is_v2s_shell_css_and_js_with_the_data_injected`

### PAR-045: the page is LF on Windows
- Command: `kluris mri`, on Windows
- Differs: v3 writes `brain-mri.html` (or the `--output` file) with LF line endings on every OS. v2 writes it in text mode, so on Windows every line ends in CRLF. The parity tests read v2's page with CRLF as LF on Windows before comparing.
- Decision: interview-01 #3 (new files LF); SPEC decision log 2026-10-25 (on Windows v2 writes in text mode; v3 keeps LF)
- Tests: `crates/kluris-core/tests/mri.rs::generate_writes_the_page_and_creates_parent_directories`, `parity/test_mri.py` (`v3_page` refuses a `\r`; `v2_page` reads CRLF as LF on Windows)

### PAR-046: what makes v2's MRI raise is a result
- Command: `kluris mri`
- Differs: where v2's `build_graph` or `json.dumps` raises (traceback, exit 1, no page, no JSON), v3 writes the page:
  - a `parent`, `replaced_by` or `related` entry that is not a string (`parent: 7`, `related: [5]`, a list, a date in `related`) makes no edge; its node field is Python's `str()` of the value, as in v2 (v2's `pathlib` raises `TypeError`);
  - a date or datetime nested in `tags` or `related` (`tags: [2026-03-04]`) is written as its ISO text (`"2026-03-04"`); v2's `json.dumps` raises `TypeError` (top-level dates are ISO text in both);
  - values nested deeper than Python's recursion limit are written (v3's writer keeps its own stack).
  The preflight skips a neuron it cannot read, as dream does (PAR-038), where v2's preflight raises; the graph then shows that file as v2's `build_graph` would: a node with its stem title and no fields or edges.
- Decision: SPEC decision log 2026-10-27 (every v2 traceback becomes a correct result or a clean error, each allowlisted); interview-01 #4
- Tests: `crates/kluris-core/tests/mri.rs::{edges_follow_v2, neuron_fields_follow_v2, missing_fields_are_empty_and_non_strings_are_python_str, a_file_that_does_not_read_is_a_node_without_fields_or_edges, indented_json_matches_python}` (no parity scenario: v2 raises)

### PAR-047: odd `related` values never crash dream
- Command: `kluris dream` (with and without `--json`)
- Differs:
  - A `related` list with an entry that is not a string (`related: [1]`) is v2's existing type issue `{file, field: "related", kind: "type"}`. It is counted in `frontmatter_issues`, so the brain is unhealthy. The one-way check ignores that entry; the reverse-link fix and the broken-link check already did, as in v2.
  - A `related` target outside the brain (`../../outside.md`, `/etc/hosts.md`) counts as a broken synapse, as in v2, and never as a one-way synapse.
  - v2's `validate_bidirectional` raises on both: `TypeError: unsupported operand type(s) for /`, and `ValueError ... is not in the subpath` for an outside `.md` file that exists. A traceback, exit 1, no JSON.
- Decision: interview-01 #4 (one bad neuron crashing dream); SPEC decision 2026-10-27 (tracebacks); the chief's answer in T-94 (m-4524 #2: v2's type issue)
- Tests: `crates/kluris-cli/tests/dream_resilience.rs::{a_non_string_related_entry_is_a_type_issue, related_paths_outside_the_brain_are_broken_not_one_way}`, `parity/test_dream.py::{test_non_string_related_entry, test_related_outside_the_brain}`

### PAR-048: a missing `kluris.yml` is a clean error
- Command: `kluris dream` (with and without `--json`, `--brain all`)
- Differs: v3 reads every resolved brain's `kluris.yml` before dreaming any of them. When one does not exist, it stops with `Error: Brain <name> has no kluris.yml: <path>` on stderr, or `{"ok": false, "error": "Brain <name> has no kluris.yml: <path>"}` under `--json`, exit 1, and writes nothing in any brain. A `kluris.yml` that does not read or validate stops it the same way, with the config error. v2 raises `FileNotFoundError` (a traceback, exit 1, no JSON) when it reaches that brain, after dreaming the brains before it. v2's dream has no path that runs without the file (it needs `name` and `description` for brain.md).
- Decision: interview-01 #4 ("missing `kluris.yml` crashes"); SPEC decision 2026-10-27 (tracebacks)
- Tests: `crates/kluris-cli/tests/dream_resilience.rs::a_missing_kluris_yml_is_a_clean_error_and_nothing_is_written`, `parity/test_dream.py::test_missing_kluris_yml`

### PAR-049: link checking skips code, other schemes and titles, and reads URL-encoded paths
- Command: `kluris dream` (`broken_synapses`, `broken_synapses_detail`, `--broken-only`, `healthy`; and the orphan check, which reads map.md/brain.md links the same way)
- Differs: v3 does not report these as broken links, where v2 does:
  - links inside fenced code blocks (```` ``` ```` or `~~~`, up to 3 spaces of indentation) and inside inline code spans;
  - targets with a URI scheme of two or more characters (`mailto:`, `ftp:`, `tel:`, `file:`; v2 skipped only `http://` and `https://`). A one-letter "scheme" (`C:/x.md`) stays a path.
  - A link title is not part of the path (`[a](b.md "t")`, `'t'`, `(t)`), and a broken titled link reports the path alone.
  - A body link is fine when its target names a file either as written or URL-decoded (`my%20note.md` is `my note.md`); `related` entries are not decoded.
  - Image links are checked as v2 checks them: `![x](y.png)` is broken when `y.png` is missing.
  - On the seven local brains this changes no count (`test_registered_brains`).
- Decision: interview-01 #5 ("link checking flagging code blocks, `mailto:` and titled links"); research-03 §5 #32; the chief's brief for T-94 (URL-decoded paths; image links keep v2's behaviour, m-4524)
- Tests: `crates/kluris-core/src/dream/links.rs` (unit tests), `crates/kluris-cli/tests/dream_resilience.rs::link_checking_ignores_code_other_schemes_and_titles_and_decodes_paths`, `parity/test_dream.py::test_link_checking`
