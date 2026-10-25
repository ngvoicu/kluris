//! Events from `saphyr-parser` built into a [`Value`], with PyYAML's
//! `SafeLoader` semantics for everything inside the value model and an error
//! for everything outside it.

use std::collections::HashMap;
use std::fmt;

use saphyr_parser::{Event, Marker, Parser, ScalarStyle, Tag};

use super::resolve;
use super::value::{Map, Value};

/// Deeper nesting is refused (PyYAML's recursive composer fails well before
/// Python's recursion limit anyway).
const MAX_DEPTH: usize = 128;

/// The most nodes one document may hold once aliases are expanded into
/// copies, so an alias bomb is an error instead of an exhausted memory.
const MAX_NODES: usize = 1_000_000;

/// Why a YAML text could not be read into a [`Value`]: invalid YAML, or a
/// construct outside the value model. Lines and columns are 1-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl YamlError {
    fn at(mark: Marker, message: impl Into<String>) -> YamlError {
        YamlError {
            message: message.into(),
            line: mark.line(),
            column: mark.col() + 1,
        }
    }
}

impl fmt::Display for YamlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (line {}, column {})",
            self.message, self.line, self.column
        )
    }
}

impl std::error::Error for YamlError {}

/// Load a single YAML document like `yaml.safe_load`: an empty stream is
/// `Null`; more than one document is an error.
pub fn load(text: &str) -> Result<Value, YamlError> {
    let mut builder = Builder::default();
    let mut documents = 0;
    for item in Parser::new_from_str(text) {
        let (event, span) = item.map_err(|e| YamlError::at(*e.marker(), e.info().to_string()))?;
        let mark = span.start;
        match event {
            Event::DocumentStart(_) => {
                documents += 1;
                if documents > 1 {
                    return Err(YamlError::at(
                        mark,
                        "expected a single document in the stream, but found another document",
                    ));
                }
            }
            Event::Scalar(text, style, anchor, tag) => {
                refuse_tag(tag.as_deref(), mark)?;
                let value = if style == ScalarStyle::Plain {
                    resolve::plain_scalar(&text).map_err(|e| YamlError::at(mark, e))?
                } else {
                    Value::Str(text.into_owned())
                };
                builder.complete(value, 1, anchor, mark)?;
            }
            Event::SequenceStart(anchor, tag) => {
                refuse_tag(tag.as_deref(), mark)?;
                builder.open(Node::Seq(Vec::new()), anchor, mark)?;
            }
            Event::MappingStart(anchor, tag) => {
                refuse_tag(tag.as_deref(), mark)?;
                builder.open(Node::Map(Map::new(), None), anchor, mark)?;
            }
            Event::SequenceEnd | Event::MappingEnd => builder.close(mark)?,
            Event::Alias(id) => builder.alias(id, mark)?,
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentEnd => {}
        }
    }
    Ok(builder.root.unwrap_or(Value::Null))
}

/// Every explicit tag is outside the value model, the core ones included.
fn refuse_tag(tag: Option<&Tag>, mark: Marker) -> Result<(), YamlError> {
    match tag {
        None => Ok(()),
        Some(tag) => {
            let name = if tag.is_yaml_core_schema() {
                format!("!!{}", tag.suffix)
            } else {
                format!("{}{}", tag.handle, tag.suffix)
            };
            Err(YamlError::at(
                mark,
                format!("explicit tags are not supported ({name})"),
            ))
        }
    }
}

/// A collection still being built; a mapping holds the key waiting for its
/// value.
enum Node {
    Seq(Vec<Value>),
    Map(Map, Option<String>),
}

struct Frame {
    node: Node,
    anchor: usize,
    /// The node count before this collection opened.
    nodes_before: usize,
}

#[derive(Default)]
struct Builder {
    stack: Vec<Frame>,
    /// Completed anchored nodes and their size in nodes.
    anchors: HashMap<usize, (Value, usize)>,
    nodes: usize,
    root: Option<Value>,
}

impl Builder {
    fn count(&mut self, nodes: usize, mark: Marker) -> Result<(), YamlError> {
        self.nodes = self.nodes.saturating_add(nodes);
        if self.nodes > MAX_NODES {
            return Err(YamlError::at(
                mark,
                format!("the document expands to more than {MAX_NODES} nodes"),
            ));
        }
        Ok(())
    }

    fn open(&mut self, node: Node, anchor: usize, mark: Marker) -> Result<(), YamlError> {
        if self.stack.len() >= MAX_DEPTH {
            return Err(YamlError::at(
                mark,
                format!("collections nested more than {MAX_DEPTH} deep"),
            ));
        }
        let nodes_before = self.nodes;
        self.count(1, mark)?;
        self.stack.push(Frame {
            node,
            anchor,
            nodes_before,
        });
        Ok(())
    }

    fn close(&mut self, mark: Marker) -> Result<(), YamlError> {
        let Some(frame) = self.stack.pop() else {
            return Err(YamlError::at(mark, "unexpected end of a collection"));
        };
        let value = match frame.node {
            Node::Seq(items) => Value::Seq(items),
            Node::Map(map, None) => Value::Map(map),
            Node::Map(_, Some(_)) => {
                return Err(YamlError::at(mark, "mapping key without a value"));
            }
        };
        let size = self.nodes - frame.nodes_before;
        self.attach(value, size, frame.anchor, mark)
    }

    fn complete(
        &mut self,
        value: Value,
        size: usize,
        anchor: usize,
        mark: Marker,
    ) -> Result<(), YamlError> {
        self.count(size, mark)?;
        self.attach(value, size, anchor, mark)
    }

    fn alias(&mut self, id: usize, mark: Marker) -> Result<(), YamlError> {
        let Some((value, size)) = self.anchors.get(&id).cloned() else {
            let message = if self.stack.iter().any(|f| f.anchor == id) {
                "recursive aliases are not supported"
            } else {
                "found undefined alias"
            };
            return Err(YamlError::at(mark, message));
        };
        self.complete(value, size, 0, mark)
    }

    /// Hand a finished node to its parent (or make it the document).
    fn attach(
        &mut self,
        value: Value,
        size: usize,
        anchor: usize,
        mark: Marker,
    ) -> Result<(), YamlError> {
        if anchor != 0 {
            self.anchors.insert(anchor, (value.clone(), size));
        }
        let Some(parent) = self.stack.last_mut() else {
            self.root = Some(value);
            return Ok(());
        };
        match &mut parent.node {
            Node::Seq(items) => items.push(value),
            Node::Map(map, pending) => match pending.take() {
                Some(key) => {
                    // Like a Python dict: a repeated key keeps its first
                    // position and takes the last value.
                    map.insert(key, value);
                }
                None => match value {
                    Value::Str(key) => *pending = Some(key),
                    other => {
                        return Err(YamlError::at(
                            mark,
                            format!("mapping keys must be strings, found {}", other.type_name()),
                        ));
                    }
                },
            },
        }
        Ok(())
    }
}
