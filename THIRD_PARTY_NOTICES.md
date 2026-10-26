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
