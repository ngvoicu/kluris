# Third-party notices

Kluris contains code ported from the projects below. Their licenses are
kept next to the ported code.

## PyYAML 6.0.3

- Source: https://github.com/yaml/pyyaml (the 6.0.3 release).
- License: MIT. Copyright (c) 2017-2021 Ingy döt Net; Copyright (c) 2006-2016
  Kirill Simonov. Full text: `crates/kluris-core/src/yaml/LICENSE-PyYAML`.
- Ported in `crates/kluris-core/src/yaml/`: `reader.rs` (`reader.py`),
  `scanner.rs` (`scanner.py`, `tokens.py`), `parser.rs` (`parser.py`,
  `events.py`), `composer.rs` (`composer.py`, `nodes.py`, `resolver.py`),
  `resolve.rs` (`resolver.py`, `constructor.py`), `parse.rs`
  (`constructor.py`) and `emit.rs` (`representer.py`, `serializer.py`,
  `emitter.py`).

## libyaml 0.2.5

- Source: https://github.com/yaml/libyaml (the 0.2.5 release, the version
  PyYAML 6.0.3's `CSafeDumper` uses).
- License: MIT. Copyright (c) 2017-2020 Ingy döt Net; Copyright (c) 2006-2016
  Kirill Simonov. Full text: `crates/kluris-core/src/yaml/LICENSE-libyaml`.
- Ported in `crates/kluris-core/src/yaml/emit.rs`: the scalar analysis,
  style selection and line folding of `emitter.c`.

## CPython 3.11

- Source: https://github.com/python/cpython/tree/3.11.
- License: Python Software Foundation License Version 2. Copyright (c)
  2001-2023 Python Software Foundation. Full text:
  `crates/kluris-core/src/LICENSE-PSF` and
  `xtask/src/sync_specmint/LICENSE-PSF` (identical copies, `LICENSE.txt`
  from a Python 3.11 install).
- Translated in `crates/kluris-core/src/`: `pycase.rs`
  (`Objects/unicodeobject.c`'s `do_title`, `do_lower`, `lower_ucs4` and
  `handle_capital_sigma`), `wake_up.rs` (`Modules/_datetimemodule.c`'s
  `parse_isoformat_date` and the week-date checks after it, as
  `iso_date`), `brain/sandbox.rs` (`Lib/posixpath.py`'s `realpath` and
  `_joinrealpath`, `Lib/ntpath.py`'s `realpath`, `normpath` and
  `_readlink_deep`) and `agents/render.rs` (`Lib/pathlib.py`'s
  `_WindowsFlavour.splitroot`, as `windows_splitroot`).
- Translated in `xtask/src/sync_specmint/difflib.rs` (a development task,
  not shipped in the `kluris` binary): `Lib/difflib.py`'s `SequenceMatcher`
  matching blocks, opcodes and grouped opcodes, `unified_diff` and
  `_format_range_unified`, so `cargo xtask sync-specmint --diff` prints
  its diffs as Python's `difflib` prints them.

## Unicode Character Database 14.0

- Source: https://www.unicode.org/Public/14.0.0/ucd/, as Python 3.11's
  `unicodedata`, `str.title`, `str.lower` and `int()`/`float()` expose it.
- License: Unicode License v3. Copyright (c) 1991-2026 Unicode, Inc. Full
  text: `crates/kluris-core/src/LICENSE-Unicode`,
  `crates/kluris-core/src/yaml/LICENSE-Unicode` and
  `crates/kluris-core/tests/fixtures/LICENSE-Unicode` (identical copies).
- Generated tables in `crates/kluris-core/src/pycase_tables.rs` (full
  titlecase and lowercase mappings and the `Cased`/`Case_Ignorable`
  ranges, from Python 3.11's Unicode database),
  `crates/kluris-core/src/yaml/resolve.rs` (`DIGIT_ZEROS`, the zero of
  every Unicode decimal-digit run, from Python 3.11's `unicodedata`)
  and `crates/kluris-core/tests/fixtures/py_title.tsv` (`str.title()`
  expectations, recorded from Python 3.11).
