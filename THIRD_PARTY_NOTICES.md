# Third-party notices

Kluris 3 contains code ported from the projects below. Their licenses are
kept next to the ported code.

## PyYAML 6.0.3

- Source: https://github.com/yaml/pyyaml (the 6.0.3 release, as installed in
  the parity oracle).
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

- Source: https://github.com/python/cpython/tree/3.11 (the Python 3.11 the
  parity oracle runs).
- License: Python Software Foundation License Version 2. Copyright (c)
  2001-2023 Python Software Foundation. Full text:
  `crates/kluris-core/src/LICENSE-PSF` and
  `xtask/src/sync_specmint/LICENSE-PSF` (identical copies, `LICENSE.txt`
  from the oracle's Python 3.11 install).
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
  what v2's `scripts/sync-specmint.py` prints.

## Unicode Character Database 14.0

- Source: https://www.unicode.org/Public/14.0.0/ucd/, as Python 3.11's
  `unicodedata`, `str.title`, `str.lower` and `int()`/`float()` expose it
  (generators in `parity/tools/`).
- License: Unicode License v3. Copyright (c) 1991-2026 Unicode, Inc. Full
  text: `crates/kluris-core/src/LICENSE-Unicode`,
  `crates/kluris-core/src/yaml/LICENSE-Unicode` and
  `crates/kluris-core/tests/fixtures/LICENSE-Unicode` (identical copies).
- Generated tables in `crates/kluris-core/src/pycase_tables.rs` (full
  titlecase and lowercase mappings and the `Cased`/`Case_Ignorable`
  ranges, from `parity/tools/gen_pycase.py`),
  `crates/kluris-core/src/yaml/resolve.rs` (`DIGIT_ZEROS`, the zero of
  every Unicode decimal-digit run, from `parity/tools/unicode_digits.py`)
  and `crates/kluris-core/tests/fixtures/py_title.tsv` (`str.title()`
  expectations, from `parity/tools/gen_pycase.py`).
