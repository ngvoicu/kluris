//! YAML reading with PyYAML's YAML 1.1 semantics (`SafeLoader`) and writing
//! with PyYAML's exact output, the way v2 reads and writes frontmatter, YAML
//! neurons and configs.

mod emit;
mod parse;
mod resolve;
mod value;

pub use emit::{Profile, emit};
pub use parse::{YamlError, load};
pub(crate) use resolve::is_py_space;
pub use value::{Date, DateTime, Map, Value};
