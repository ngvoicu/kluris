//! YAML reading with PyYAML's YAML 1.1 semantics (`SafeLoader`), the way v2
//! reads frontmatter, YAML neurons and configs.

mod parse;
mod resolve;
mod value;

pub use parse::{YamlError, load};
pub use value::{Date, DateTime, Map, Value};
