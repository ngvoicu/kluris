//! YAML reading with PyYAML's YAML 1.1 semantics (`SafeLoader`, its reader,
//! scanner, parser and composer ported from PyYAML 6.0.3) and writing with
//! PyYAML's exact output, the way v2 reads and writes frontmatter, YAML
//! neurons and configs.

mod composer;
mod emit;
mod parse;
mod parser;
mod reader;
mod resolve;
mod scanner;
mod value;

pub use emit::{Profile, emit};
pub(crate) use emit::{date_iso, datetime_iso};
pub use parse::{YamlError, load};
pub(crate) use resolve::is_py_space;
pub use value::{Date, DateTime, Map, Value};
