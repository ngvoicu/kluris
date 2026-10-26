//! YAML text read the way PyYAML's `SafeLoader` reads it: the ported
//! reader, scanner, parser and composer build the node graph, and the graph
//! is constructed into a [`Value`] in the order
//! `BaseConstructor.construct_document` uses. Everything outside the value
//! model is an error.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use indexmap::IndexMap;

use super::composer::{Body, Composer, Node, NodeTag};
use super::parser::Parser;
use super::reader::{Mark, Reader};
use super::resolve::{self, ScalarTag};
use super::scanner::Scanner;
use super::value::{Map, Value};

/// The deepest `=` chain `construct_scalar` follows.
const MAX_DEPTH: usize = 128;

/// The most nodes one document may hold once aliases are expanded into
/// copies, so an alias bomb is an error instead of an exhausted memory.
const MAX_NODES: usize = 1_000_000;

/// The deepest value aliases may splice together.
const MAX_VALUE_DEPTH: usize = 1024;

/// Why a YAML text could not be read into a [`Value`]: invalid YAML, or a
/// construct outside the value model. Lines and columns are 1-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl YamlError {
    /// An error at PyYAML's `mark` (0-based; stored 1-based).
    pub(super) fn new(message: String, mark: Mark) -> YamlError {
        YamlError {
            message,
            line: mark.line + 1,
            column: mark.column + 1,
        }
    }

    /// A `ConstructorError` at a node's start.
    fn at(mark: Mark, message: impl Into<String>) -> YamlError {
        YamlError::new(format!("ConstructorError: {}", message.into()), mark)
    }
}

/// A `MarkedYAMLError` of PyYAML: the class, the context and the problem in
/// the message, at the problem's mark. The context's mark is named when it is
/// elsewhere.
pub(super) fn marked(
    class: &str,
    context: Option<&str>,
    context_mark: Option<Mark>,
    problem: &str,
    mark: Mark,
) -> YamlError {
    let mut message = format!("{class}: ");
    if let Some(context) = context {
        message.push_str(context);
        if let Some(at) = context_mark
            && at != mark
        {
            message.push_str(&format!(
                " (line {}, column {})",
                at.line + 1,
                at.column + 1
            ));
        }
        message.push_str(": ");
    }
    message.push_str(problem);
    YamlError::new(message, mark)
}

/// Python's `repr()` of one character, as PyYAML's messages quote them.
pub(super) fn py_repr(c: char) -> String {
    match c {
        '\\' => "'\\\\'".to_string(),
        '\'' => "\"'\"".to_string(),
        '\t' => "'\\t'".to_string(),
        '\n' => "'\\n'".to_string(),
        '\r' => "'\\r'".to_string(),
        c if (c as u32) < 0x20 || (0x7F..0xA0).contains(&(c as u32)) => {
            format!("'\\x{:02x}'", c as u32)
        }
        c => format!("'{c}'"),
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
pub fn load(source: &str) -> Result<Value, YamlError> {
    let reader = Reader::new(source)?;
    let mut composer = Composer::new(Parser::new(Scanner::new(reader)));
    match composer.get_single_node()? {
        None => Ok(Value::Null),
        Some(root) => Constructor::default().document(&root),
    }
}

// --- constructing --------------------------------------------------------------

/// What `construct_object` returns: a scalar's value, a collection that is
/// filled later (PyYAML's generator constructors), or something PyYAML
/// builds that the value model cannot hold. That last one is an error only if
/// it ends up in the document's value; a repeated key can drop it.
#[derive(Clone)]
enum Item {
    Value(Value),
    Slot(usize),
    Outside { error: YamlError, hashable: bool },
}

enum Slot {
    Pending,
    Seq(Vec<Item>),
    /// Like a Python dict: a repeated key keeps its first position and takes
    /// the last value.
    Map(IndexMap<String, Item>),
    /// A mapping PyYAML builds with a key that is not a string.
    Outside(YamlError),
}

/// Which generator fills a slot.
#[derive(Clone, Copy)]
enum Fill {
    Seq,
    Map,
    /// `construct_yaml_set`: a mapping's keys and values are constructed;
    /// the set itself is outside the value model.
    Set,
    /// `construct_yaml_omap` / `construct_yaml_pairs`: a sequence of
    /// one-pair mappings; the list of pairs is outside the value model.
    Pairs,
}

fn outside(node: &Node, what: &str, hashable: bool) -> Item {
    Item::Outside {
        error: YamlError::at(node.mark, format!("{what} is outside the value model")),
        hashable,
    }
}

/// `SafeConstructor` over the node graph. Collections get a slot when first
/// constructed and are filled in rounds, as `construct_document` runs its
/// state generators; scalars are constructed on the spot. The order matters:
/// filling a mapping retags its `=` keys as strings (`flatten_mapping`), and
/// a scalar constructed before that still sees `!!value`.
#[derive(Default)]
struct Constructor {
    constructed: HashMap<usize, Item>,
    retagged: HashSet<usize>,
    slots: Vec<Slot>,
    queue: Vec<(Rc<Node>, usize, Fill)>,
    nodes: usize,
}

impl Constructor {
    fn document(mut self, root: &Rc<Node>) -> Result<Value, YamlError> {
        let item = self.object(root)?;
        while !self.queue.is_empty() {
            for (node, slot, fill) in std::mem::take(&mut self.queue) {
                self.fill(&node, slot, fill)?;
            }
        }
        self.assemble(&item, root.mark)
    }

    fn tag<'a>(&self, node: &'a Node) -> &'a NodeTag {
        static STR: NodeTag = NodeTag::Scalar(ScalarTag::Str);
        if self.retagged.contains(&node.id) {
            &STR
        } else {
            &node.tag
        }
    }

    fn queue(&mut self, node: &Rc<Node>, fill: Fill) -> usize {
        self.slots.push(Slot::Pending);
        let slot = self.slots.len() - 1;
        self.queue.push((Rc::clone(node), slot, fill));
        slot
    }

    /// `construct_object` (not deep).
    fn object(&mut self, node: &Rc<Node>) -> Result<Item, YamlError> {
        if let Some(item) = self.constructed.get(&node.id) {
            return Ok(item.clone());
        }
        let fail = |message: String| Err(YamlError::at(node.mark, message));
        let item = match (self.tag(node).clone(), &node.body) {
            // The collection is an ancestor, so it has its slot already.
            (_, Body::Recursive(target)) => match self.constructed.get(target) {
                Some(item) => item.clone(),
                None => return fail("recursive aliases are not supported".into()),
            },
            (NodeTag::Seq, Body::Seq(_)) => Item::Slot(self.queue(node, Fill::Seq)),
            (NodeTag::Map, Body::Map(_)) => Item::Slot(self.queue(node, Fill::Map)),
            (NodeTag::Seq, _) => {
                return fail(format!(
                    "expected a sequence node, but found {}",
                    kind(node)
                ));
            }
            (NodeTag::Map | NodeTag::Set, Body::Scalar { .. } | Body::Seq(_)) => {
                return fail(format!("expected a mapping node, but found {}", kind(node)));
            }
            (NodeTag::Set, _) => {
                self.queue(node, Fill::Set);
                outside(node, "a set", false)
            }
            // The shape of an ordered map is checked when it is filled.
            (NodeTag::Omap | NodeTag::Pairs, _) => {
                self.queue(node, Fill::Pairs);
                outside(node, "a list of pairs", false)
            }
            // `construct_yaml_timestamp` matches its pattern on `node.value`,
            // a list for a mapping.
            (NodeTag::Scalar(ScalarTag::Timestamp), Body::Map(_)) => {
                return fail("a timestamp cannot be built from a mapping".into());
            }
            (NodeTag::Scalar(tag), _) => {
                let (text, surrogate) = self.scalar(node)?;
                match tag {
                    // Python builds the string; a Rust one cannot hold it.
                    ScalarTag::Str if surrogate => outside(node, "a lone surrogate", true),
                    // U+FFFD in its place fails `int()`, `float()`, the
                    // booleans and the timestamp pattern as the surrogate does.
                    _ => Item::Value(
                        resolve::tagged_scalar(tag, &text)
                            .map_err(|e| YamlError::at(node.mark, e))?,
                    ),
                }
            }
            (NodeTag::Binary, _) => {
                let (text, _) = self.scalar(node)?;
                check_base64(&text).map_err(|e| YamlError::at(node.mark, e))?;
                outside(node, "binary data", true)
            }
            (NodeTag::Value, _) => {
                return fail(
                    "could not determine a constructor for the tag 'tag:yaml.org,2002:value'"
                        .into(),
                );
            }
            (NodeTag::Merge, _) => return fail("merge keys (<<) are not supported".into()),
            (NodeTag::Other(name), _) => return fail(format!("unsupported tag {name}")),
        };
        self.constructed.insert(node.id, item.clone());
        Ok(item)
    }

    /// `SafeConstructor.construct_scalar`: a scalar's text (and whether it
    /// holds a lone surrogate), or for a mapping the text of the value of its
    /// first `=` key.
    fn scalar(&self, node: &Node) -> Result<(String, bool), YamlError> {
        let mut node = node;
        for _ in 0..=MAX_DEPTH {
            match &node.body {
                Body::Scalar { text, surrogate } => return Ok((text.clone(), *surrogate)),
                Body::Map(pairs) => {
                    match pairs
                        .iter()
                        .find(|(key, _)| *self.tag(key) == NodeTag::Value)
                    {
                        Some((_, value)) => node = value,
                        None => {
                            return Err(YamlError::at(
                                node.mark,
                                "expected a scalar node, but found mapping",
                            ));
                        }
                    }
                }
                _ => {
                    return Err(YamlError::at(
                        node.mark,
                        format!("expected a scalar node, but found {}", kind(node)),
                    ));
                }
            }
        }
        Err(YamlError::at(
            node.mark,
            format!("`=` values nested more than {MAX_DEPTH} deep"),
        ))
    }

    /// A mapping key as `construct_mapping` takes it: `Ok(Ok(name))` for a
    /// string, `Ok(Err(error))` for a hashable key outside the value model (an
    /// error only if the mapping is used), `Err` for an unhashable key.
    fn key(&mut self, key: &Rc<Node>) -> Result<Result<String, YamlError>, YamlError> {
        match self.object(key)? {
            Item::Value(mut value) => Ok(match &mut value {
                Value::Str(name) => Ok(std::mem::take(name)),
                other => Err(YamlError::at(
                    key.mark,
                    format!("mapping keys must be strings, found {}", other.type_name()),
                )),
            }),
            Item::Outside {
                error,
                hashable: true,
            } => Ok(Err(error)),
            Item::Slot(_) | Item::Outside { .. } => Err(YamlError::at(
                key.mark,
                "while constructing a mapping, found unhashable key",
            )),
        }
    }

    /// `flatten_mapping` without merges: `=` keys become strings.
    fn flatten(&mut self, pairs: &[(Rc<Node>, Rc<Node>)]) -> Result<(), YamlError> {
        for (key, _) in pairs {
            match self.tag(key) {
                NodeTag::Merge => {
                    return Err(YamlError::at(key.mark, "merge keys (<<) are not supported"));
                }
                NodeTag::Value => {
                    self.retagged.insert(key.id);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Run a collection's generator.
    fn fill(&mut self, node: &Rc<Node>, slot: usize, fill: Fill) -> Result<(), YamlError> {
        let unexpected = || YamlError::at(node.mark, "not a collection");
        let filled = match (fill, &node.body) {
            (Fill::Seq, Body::Seq(items)) => Slot::Seq(
                items
                    .iter()
                    .map(|item| self.object(item))
                    .collect::<Result<_, _>>()?,
            ),
            (Fill::Map | Fill::Set, Body::Map(pairs)) => {
                self.flatten(pairs)?;
                let mut entries = IndexMap::with_capacity(pairs.len());
                let mut bad_key = None;
                for (key, value) in pairs {
                    let name = self.key(key)?;
                    let item = self.object(value)?;
                    match name {
                        Ok(name) => {
                            entries.insert(name, item);
                        }
                        Err(error) => {
                            bad_key.get_or_insert(error);
                        }
                    }
                }
                match bad_key {
                    Some(error) => Slot::Outside(error),
                    None => Slot::Map(entries),
                }
            }
            (Fill::Pairs, Body::Seq(items)) => {
                for item in items {
                    match &item.body {
                        Body::Map(pairs) if pairs.len() == 1 => {
                            self.object(&pairs[0].0)?;
                            self.object(&pairs[0].1)?;
                        }
                        _ => {
                            return Err(YamlError::at(
                                item.mark,
                                "while constructing an ordered map, expected a mapping of length 1",
                            ));
                        }
                    }
                }
                Slot::Pending
            }
            (Fill::Pairs, _) => {
                return Err(YamlError::at(
                    node.mark,
                    "while constructing an ordered map, expected a sequence",
                ));
            }
            _ => return Err(unexpected()),
        };
        self.slots[slot] = filled;
        Ok(())
    }

    /// One more node of the value, within the node budget and the depth.
    fn count(&mut self, mark: Mark, depth: usize) -> Result<(), YamlError> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(YamlError::at(
                mark,
                format!("the document expands to more than {MAX_NODES} nodes"),
            ));
        }
        if depth > MAX_VALUE_DEPTH {
            return Err(YamlError::at(
                mark,
                format!("the value is nested more than {MAX_VALUE_DEPTH} deep"),
            ));
        }
        Ok(())
    }

    /// An item as a finished value, or the slot to expand. A slot is taken
    /// out while it is expanded, so the items it holds are assembled without
    /// cloning them, and an alias back into it is a cycle.
    fn open(&mut self, item: &Item, mark: Mark) -> Result<Result<Value, (usize, Slot)>, YamlError> {
        let slot = match item {
            Item::Value(value) => return Ok(Ok(value.clone())),
            Item::Outside { error, .. } => return Err(error.clone()),
            Item::Slot(slot) => *slot,
        };
        match std::mem::replace(&mut self.slots[slot], Slot::Pending) {
            Slot::Pending => Err(YamlError::at(mark, "recursive aliases are not supported")),
            Slot::Outside(error) => {
                self.slots[slot] = Slot::Outside(error.clone());
                Err(error)
            }
            filled => Ok(Err((slot, filled))),
        }
    }

    /// The owned value of an item, expanding shared collections into copies
    /// within the node budget. Iterative: a document nested as deep as the
    /// composer allows needs no deep stack.
    fn assemble(&mut self, item: &Item, mark: Mark) -> Result<Value, YamlError> {
        /// A collection being assembled: its slot (taken out), the next item
        /// and the value so far.
        struct Frame {
            slot: usize,
            filled: Slot,
            next: usize,
            value: Value,
        }
        let start = |slot: usize, filled: Slot| {
            let value = match filled {
                Slot::Map(_) => Value::Map(Map::new()),
                _ => Value::Seq(Vec::new()),
            };
            Frame {
                slot,
                filled,
                next: 0,
                value,
            }
        };
        self.count(mark, 0)?;
        let mut stack = match self.open(item, mark)? {
            Ok(value) => return Ok(value),
            Err((slot, filled)) => vec![start(slot, filled)],
        };
        loop {
            let depth = stack.len();
            let Some(frame) = stack.last_mut() else {
                unreachable!("the loop returns when the stack empties")
            };
            let next = match &frame.filled {
                Slot::Seq(items) => items.get(frame.next).cloned().map(|item| (None, item)),
                Slot::Map(entries) => entries
                    .get_index(frame.next)
                    .map(|(key, item)| (Some(key.clone()), item.clone())),
                Slot::Pending | Slot::Outside(_) => None,
            };
            frame.next += 1;
            let (key, value) = match next {
                Some((key, item)) => {
                    self.count(mark, depth)?;
                    match self.open(&item, mark)? {
                        Ok(value) => (key, value),
                        Err((slot, filled)) => {
                            stack.push(start(slot, filled));
                            continue;
                        }
                    }
                }
                None => {
                    let Some(done) = stack.pop() else {
                        unreachable!("checked above")
                    };
                    self.slots[done.slot] = done.filled;
                    let Some(parent) = stack.last() else {
                        return Ok(done.value);
                    };
                    let key = match &parent.filled {
                        Slot::Map(entries) => {
                            entries.get_index(parent.next - 1).map(|(k, _)| k.clone())
                        }
                        _ => None,
                    };
                    (key, done.value)
                }
            };
            let Some(parent) = stack.last_mut() else {
                unreachable!("a child has a parent")
            };
            match (&mut parent.value, key) {
                (Value::Map(map), Some(key)) => {
                    map.insert(key, value);
                }
                (Value::Seq(items), _) => items.push(value),
                _ => unreachable!("a map entry has a key"),
            }
        }
    }
}

/// `base64.decodebytes` on the ASCII encoding of `text`, as
/// `construct_yaml_binary` calls it: CPython's non-strict
/// `binascii.a2b_base64` skips characters outside the alphabet and stops at
/// complete padding; the data characters must not end one past a multiple of
/// four, nor short of one without padding.
fn check_base64(text: &str) -> Result<(), String> {
    if !text.is_ascii() {
        return Err("failed to convert base64 data into ascii".into());
    }
    let (mut quad, mut pads, mut data) = (0usize, 0usize, 0usize);
    for c in text.chars() {
        if c == '=' {
            if quad >= 2 {
                pads += 1;
                if quad + pads >= 4 {
                    return Ok(());
                }
            }
            continue;
        }
        if c.is_ascii_alphanumeric() || c == '+' || c == '/' {
            pads = 0;
            data += 1;
            quad = (quad + 1) % 4;
        }
    }
    match quad {
        0 => Ok(()),
        1 => Err(format!(
            "failed to decode base64 data: number of data characters ({data}) cannot be 1 more than a multiple of 4"
        )),
        _ => Err("failed to decode base64 data: Incorrect padding".into()),
    }
}

fn kind(node: &Node) -> &'static str {
    match node.body {
        Body::Scalar { .. } => "scalar",
        Body::Seq(_) | Body::Recursive(_) => "sequence",
        Body::Map(_) => "mapping",
    }
}
