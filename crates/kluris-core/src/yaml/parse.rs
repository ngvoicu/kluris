//! YAML text read the way PyYAML's `SafeLoader` reads it, in its two steps:
//! `saphyr-parser` events are composed into a node graph (anchors shared,
//! tags resolved), then the graph is constructed into a [`Value`] in the
//! order `BaseConstructor.construct_document` uses. Everything outside the
//! value model is an error.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use indexmap::IndexMap;

use saphyr_parser::{Event, Marker, Parser, ScalarStyle, Span, Tag};

use super::resolve::{self, ImplicitTag, ScalarTag};
use super::value::{Map, Value};

/// Deeper nesting is refused (PyYAML's recursive composer fails well before
/// Python's recursion limit anyway).
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
pub fn load(source: &str) -> Result<Value, YamlError> {
    match compose(source)? {
        None => Ok(Value::Null),
        Some(root) => Constructor::default().document(&root),
    }
}

// --- composing ---------------------------------------------------------------

/// A node's tag after resolution (`Composer` + `Resolver`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum NodeTag {
    Scalar(ScalarTag),
    Seq,
    Map,
    /// `tag:yaml.org,2002:value`, what `=` resolves to.
    Value,
    /// `tag:yaml.org,2002:merge`, what `<<` resolves to.
    Merge,
    /// Core tags PyYAML constructs into values outside the value model
    /// (bytes, a set, lists of pairs).
    Binary,
    Set,
    Omap,
    Pairs,
    /// Any other tag; no constructor in the value model.
    Other(String),
}

enum Body {
    Scalar(String),
    Seq(Vec<Rc<Node>>),
    Map(Vec<(Rc<Node>, Rc<Node>)>),
    /// An alias to a collection still being composed (the node with this
    /// id). PyYAML builds a cycle; the value model cannot hold one, so it is
    /// an error if it is still there once the value is built.
    Recursive(usize),
}

struct Node {
    id: usize,
    tag: NodeTag,
    mark: Marker,
    body: Body,
}

const CORE: &str = "tag:yaml.org,2002:";

/// The tag PyYAML gives a node: an explicit tag (`!!` expanded, verbatim as
/// written), the implicit resolvers for a plain scalar or one tagged `!`,
/// `!!str` for any other untagged scalar, `!!seq`/`!!map` for collections.
fn resolve_tag(
    tag: Option<&Tag>,
    collection: Option<NodeTag>,
    scalar: Option<(&str, ScalarStyle)>,
) -> NodeTag {
    let full = tag.map(|t| format!("{}{}", t.handle, t.suffix));
    match (full.as_deref(), collection, scalar) {
        (None | Some("!"), Some(default), _) => default,
        (Some("!"), None, Some((text, _))) => implicit(text),
        (None, None, Some((text, ScalarStyle::Plain))) => implicit(text),
        (None, _, _) => NodeTag::Scalar(ScalarTag::Str),
        (Some(full), _, _) => match full.strip_prefix(CORE) {
            Some("str") => NodeTag::Scalar(ScalarTag::Str),
            Some("int") => NodeTag::Scalar(ScalarTag::Int),
            Some("float") => NodeTag::Scalar(ScalarTag::Float),
            Some("bool") => NodeTag::Scalar(ScalarTag::Bool),
            Some("null") => NodeTag::Scalar(ScalarTag::Null),
            Some("timestamp") => NodeTag::Scalar(ScalarTag::Timestamp),
            Some("seq") => NodeTag::Seq,
            Some("map") => NodeTag::Map,
            Some("value") => NodeTag::Value,
            Some("merge") => NodeTag::Merge,
            Some("binary") => NodeTag::Binary,
            Some("set") => NodeTag::Set,
            Some("omap") => NodeTag::Omap,
            Some("pairs") => NodeTag::Pairs,
            Some(other) => NodeTag::Other(format!("!!{other}")),
            None => NodeTag::Other(full.to_string()),
        },
    }
}

fn implicit(text: &str) -> NodeTag {
    match resolve::implicit_tag(text) {
        ImplicitTag::Bool => NodeTag::Scalar(ScalarTag::Bool),
        ImplicitTag::Float => NodeTag::Scalar(ScalarTag::Float),
        ImplicitTag::Int => NodeTag::Scalar(ScalarTag::Int),
        ImplicitTag::Null => NodeTag::Scalar(ScalarTag::Null),
        ImplicitTag::Timestamp => NodeTag::Scalar(ScalarTag::Timestamp),
        ImplicitTag::Str => NodeTag::Scalar(ScalarTag::Str),
        ImplicitTag::Merge => NodeTag::Merge,
        ImplicitTag::Value => NodeTag::Value,
    }
}

/// A collection still being composed.
struct Open {
    id: usize,
    tag: NodeTag,
    mark: Marker,
    anchor: usize,
    map: bool,
    items: Vec<Rc<Node>>,
}

/// Compose the single document of `source` into a node graph (`None` for an
/// empty stream).
fn compose(source: &str) -> Result<Option<Rc<Node>>, YamlError> {
    let mut documents = 0;
    // saphyr-parser's marker indexes count characters, not bytes.
    let source_chars = source.chars().count();
    let mut anchor_names = AnchorNames::new(source);
    let mut anchors: HashMap<usize, Rc<Node>> = HashMap::new();
    let mut stack: Vec<Open> = Vec::new();
    let mut root = None;
    let mut next_id = 0;
    for item in Parser::new_from_str(source) {
        let (event, span) = item.map_err(|e| YamlError::at(*e.marker(), e.info().to_string()))?;
        let mark = span.start;
        // A block scalar's header, and the indentation of the block
        // collection around it (PyYAML's `Scanner.indent`): a mapping's
        // column, or the column of the item's `-` (an indentless sequence
        // keeps its mapping's indentation, which is that column too).
        let header = match &event {
            Event::Scalar(_, ScalarStyle::Literal | ScalarStyle::Folded, ..) => {
                anchor_names.block_header(span.start.index()).map(|header| {
                    let parent = match stack.last() {
                        None => -1,
                        Some(open) if open.map => open.mark.col() as isize,
                        Some(open) => {
                            anchor_names.entry_dash(header).unwrap_or(open.mark.col()) as isize
                        }
                    };
                    (header, parent)
                })
            }
            _ => None,
        };
        anchor_names.event(&event, span)?;
        let opens_map = matches!(event, Event::MappingStart(..));
        let (node, anchor) = match event {
            Event::DocumentStart(_) => {
                documents += 1;
                if documents > 1 {
                    return Err(YamlError::at(
                        mark,
                        "expected a single document in the stream, but found another document",
                    ));
                }
                continue;
            }
            Event::Scalar(mut text, style, anchor, tag) => {
                // saphyr-parser chomps a block scalar that runs to the end of
                // the input differently from PyYAML; such a scalar is read
                // again the way PyYAML reads it.
                if let Some((header, parent)) = header
                    && span.end.index() == source_chars
                {
                    let folded = style == ScalarStyle::Folded;
                    if let Some(read) =
                        block_scalar_to_end(&anchor_names.chars, header, parent, folded)
                    {
                        text = read.into();
                    }
                }
                let tag = resolve_tag(tag.as_deref(), None, Some((&text, style)));
                let body = Body::Scalar(text.into_owned());
                next_id += 1;
                (
                    Node {
                        id: next_id,
                        tag,
                        mark,
                        body,
                    },
                    anchor,
                )
            }
            Event::SequenceStart(anchor, tag) | Event::MappingStart(anchor, tag) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(YamlError::at(
                        mark,
                        format!("collections nested more than {MAX_DEPTH} deep"),
                    ));
                }
                let map = opens_map;
                let default = if map { NodeTag::Map } else { NodeTag::Seq };
                let tag = resolve_tag(tag.as_deref(), Some(default), None);
                next_id += 1;
                stack.push(Open {
                    id: next_id,
                    tag,
                    mark,
                    anchor,
                    map,
                    items: Vec::new(),
                });
                continue;
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let Some(open) = stack.pop() else {
                    return Err(YamlError::at(mark, "unexpected end of a collection"));
                };
                let body = if open.map {
                    if open.items.len() % 2 != 0 {
                        return Err(YamlError::at(mark, "mapping key without a value"));
                    }
                    let mut items = open.items.into_iter();
                    let mut pairs = Vec::new();
                    while let (Some(key), Some(value)) = (items.next(), items.next()) {
                        pairs.push((key, value));
                    }
                    Body::Map(pairs)
                } else {
                    Body::Seq(open.items)
                };
                (
                    Node {
                        id: open.id,
                        tag: open.tag,
                        mark: open.mark,
                        body,
                    },
                    open.anchor,
                )
            }
            Event::Alias(id) => {
                let node = match anchors.get(&id) {
                    Some(node) => Rc::clone(node),
                    None => match stack.iter().find(|open| open.anchor == id) {
                        Some(open) => {
                            next_id += 1;
                            Rc::new(Node {
                                id: next_id,
                                tag: open.tag.clone(),
                                mark,
                                body: Body::Recursive(open.id),
                            })
                        }
                        None => return Err(YamlError::at(mark, "found undefined alias")),
                    },
                };
                attach(&mut stack, &mut root, node);
                continue;
            }
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentEnd => {
                continue;
            }
        };
        let node = Rc::new(node);
        if next_id > MAX_NODES {
            return Err(YamlError::at(
                mark,
                format!("the document has more than {MAX_NODES} nodes"),
            ));
        }
        if anchor != 0 {
            anchors.insert(anchor, Rc::clone(&node));
        }
        attach(&mut stack, &mut root, node);
    }
    Ok(root)
}

/// `Scanner.scan_block_scalar` of PyYAML for a block scalar whose header
/// (`|` or `>`) is at character `start` and which runs to the end of `chars`;
/// `parent` is the indentation of the enclosing block collection (-1 at the
/// top). `None` when the header is not followed by a line break.
///
/// Only `\n`, `\r\n` and `\r` break lines here, as in saphyr-parser; PyYAML
/// also breaks at U+0085, U+2028 and U+2029 (the known gap, see
/// `frontmatter::universal_newlines`).
fn block_scalar_to_end(
    chars: &[char],
    start: usize,
    parent: isize,
    folded: bool,
) -> Option<String> {
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    let mut i = start + 1;
    let (mut chomping, mut increment) = (None, None);
    for _ in 0..2 {
        match at(i) {
            '+' if chomping.is_none() => chomping = Some(true),
            '-' if chomping.is_none() => chomping = Some(false),
            c @ '1'..='9' if increment.is_none() => increment = c.to_digit(10),
            _ => break,
        }
        i += 1;
    }
    // The rest of the header line (blanks, a comment) and its line break.
    while !matches!(at(i), '\n' | '\r' | '\0') {
        i += 1;
    }
    if at(i) == '\0' {
        return None;
    }
    let line_break = |i: &mut usize| -> bool {
        match at(*i) {
            '\r' if at(*i + 1) == '\n' => *i += 2,
            '\n' | '\r' => *i += 1,
            _ => return false,
        }
        true
    };
    line_break(&mut i);
    let mut column = 0usize;
    let min_indent = usize::try_from(parent + 1).unwrap_or(0).max(1);
    let mut breaks = 0usize;
    // `scan_block_scalar_breaks`: the line breaks of the lines that end
    // before the indentation.
    let scan_breaks = |i: &mut usize, column: &mut usize, indent: usize| -> usize {
        let mut breaks = 0;
        loop {
            while *column < indent && at(*i) == ' ' {
                *i += 1;
                *column += 1;
            }
            if !line_break(i) {
                return breaks;
            }
            breaks += 1;
            *column = 0;
        }
    };
    let indent = match increment {
        Some(increment) => {
            let indent = min_indent + increment as usize - 1;
            breaks = scan_breaks(&mut i, &mut column, indent);
            indent
        }
        // `scan_block_scalar_indentation`.
        None => {
            let mut max_indent = 0;
            loop {
                if at(i) == ' ' {
                    i += 1;
                    column += 1;
                    max_indent = max_indent.max(column);
                } else if line_break(&mut i) {
                    breaks += 1;
                    column = 0;
                } else {
                    break;
                }
            }
            min_indent.max(max_indent)
        }
    };
    let mut chunks = String::new();
    let mut last_break = false;
    while column == indent && at(i) != '\0' {
        chunks.extend(std::iter::repeat_n('\n', breaks));
        let leading_non_space = !matches!(at(i), ' ' | '\t');
        while !matches!(at(i), '\0' | '\r' | '\n') {
            chunks.push(at(i));
            i += 1;
        }
        last_break = line_break(&mut i);
        column = 0;
        breaks = scan_breaks(&mut i, &mut column, indent);
        if column == indent && at(i) != '\0' {
            if folded && last_break && leading_non_space && !matches!(at(i), ' ' | '\t') {
                if breaks == 0 {
                    chunks.push(' ');
                }
            } else if last_break {
                chunks.push('\n');
            }
        } else {
            break;
        }
    }
    if chomping != Some(false) && last_break {
        chunks.push('\n');
    }
    if chomping == Some(true) {
        chunks.extend(std::iter::repeat_n('\n', breaks));
    }
    Some(chunks)
}

fn attach(stack: &mut [Open], root: &mut Option<Rc<Node>>, node: Rc<Node>) {
    match stack.last_mut() {
        Some(open) => open.items.push(node),
        None => *root = Some(node),
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
        self.assemble(&item, root.mark, 0)
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
            (NodeTag::Map | NodeTag::Set, Body::Scalar(_) | Body::Seq(_)) => {
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
                let text = self.scalar(node, 0)?;
                Item::Value(
                    resolve::tagged_scalar(tag, &text).map_err(|e| YamlError::at(node.mark, e))?,
                )
            }
            (NodeTag::Binary, _) => {
                let text = self.scalar(node, 0)?;
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

    /// `SafeConstructor.construct_scalar`: a scalar's text, or for a mapping
    /// the text of the value of its first `=` key.
    fn scalar(&self, node: &Node, depth: usize) -> Result<String, YamlError> {
        match &node.body {
            Body::Scalar(text) => Ok(text.clone()),
            Body::Map(pairs) if depth < MAX_DEPTH => {
                match pairs
                    .iter()
                    .find(|(key, _)| *self.tag(key) == NodeTag::Value)
                {
                    Some((_, value)) => self.scalar(value, depth + 1),
                    None => Err(YamlError::at(
                        node.mark,
                        "expected a scalar node, but found mapping",
                    )),
                }
            }
            _ => Err(YamlError::at(
                node.mark,
                format!("expected a scalar node, but found {}", kind(node)),
            )),
        }
    }

    /// A mapping key as `construct_mapping` takes it: `Ok(Ok(name))` for a
    /// string, `Ok(Err(error))` for a hashable key outside the value model (an
    /// error only if the mapping is used), `Err` for an unhashable key.
    fn key(&mut self, key: &Rc<Node>) -> Result<Result<String, YamlError>, YamlError> {
        match self.object(key)? {
            Item::Value(Value::Str(name)) => Ok(Ok(name)),
            Item::Value(other) => Ok(Err(YamlError::at(
                key.mark,
                format!("mapping keys must be strings, found {}", other.type_name()),
            ))),
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

    /// The owned value of an item, expanding shared collections into copies
    /// within the node budget.
    fn assemble(&mut self, item: &Item, mark: Marker, depth: usize) -> Result<Value, YamlError> {
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
        let slot = match item {
            Item::Value(value) => return Ok(value.clone()),
            Item::Outside { error, .. } => return Err(error.clone()),
            Item::Slot(slot) => *slot,
        };
        // Take the slot out while it is expanded, so the items it holds can
        // be assembled without cloning them.
        let filled = std::mem::replace(&mut self.slots[slot], Slot::Pending);
        let value = match &filled {
            Slot::Pending => Err(YamlError::at(mark, "recursive aliases are not supported")),
            Slot::Outside(error) => Err(error.clone()),
            Slot::Seq(items) => items
                .iter()
                .map(|item| self.assemble(item, mark, depth + 1))
                .collect::<Result<_, _>>()
                .map(Value::Seq),
            Slot::Map(entries) => {
                let mut map = Map::new();
                let mut result = Ok(());
                for (key, item) in entries {
                    match self.assemble(item, mark, depth + 1) {
                        Ok(value) => {
                            map.insert(key.clone(), value);
                        }
                        Err(e) => {
                            result = Err(e);
                            break;
                        }
                    }
                }
                result.map(|()| Value::Map(map))
            }
        };
        self.slots[slot] = filled;
        value
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
        Body::Scalar(_) => "scalar",
        Body::Seq(_) | Body::Recursive(_) => "sequence",
        Body::Map(_) => "mapping",
    }
}

// --- anchor names --------------------------------------------------------------

/// The anchor names of a document, checked the way PyYAML checks them:
/// `[0-9A-Za-z_-]` only, and each defined once.
///
/// saphyr-parser reports an anchor only as a number, so the name is read from
/// the source between the previous event and the anchored one, where only
/// indicators, whitespace, comments, directives, the tag and the anchor can
/// be. Marker indexes count characters.
struct AnchorNames {
    chars: Vec<char>,
    /// Where the text not yet covered by an event starts.
    gap: Marker,
    seen: HashMap<String, Marker>,
}

impl AnchorNames {
    fn new(source: &str) -> AnchorNames {
        AnchorNames {
            chars: source.chars().collect(),
            gap: Marker::new(0, 1, 0),
            seen: HashMap::new(),
        }
    }

    fn event(&mut self, event: &Event, span: Span) -> Result<(), YamlError> {
        let anchor = match event {
            // An implicit document start spans the document's first token,
            // which can be the anchor itself.
            Event::DocumentStart(false) => return Ok(()),
            Event::Scalar(_, _, anchor, _)
            | Event::SequenceStart(anchor, _)
            | Event::MappingStart(anchor, _) => *anchor,
            _ => 0,
        };
        if anchor != 0
            && let Some((name, at)) = self.find(span.start.index())
        {
            check_anchor_name(&name, self.chars.get(at.index() + name.chars().count() + 1))
                .map_err(|e| YamlError::at(at, e))?;
            if let Some(first) = self.seen.get(&name) {
                return Err(YamlError::at(
                    at,
                    format!(
                        "found duplicate anchor '{name}'; first occurrence at line {}, column {}",
                        first.line(),
                        first.col() + 1
                    ),
                ));
            }
            self.seen.insert(name, at);
        }
        if span.end.index() > self.gap.index() {
            self.gap = span.end;
        }
        Ok(())
    }

    /// The anchor token (`&name`) in the gap before `end`, and where it is.
    fn find(&self, end: usize) -> Option<(String, Marker)> {
        let at = self.token(end, |c| c == '&')?;
        let name = self.chars[at.index() + 1..end]
            .iter()
            .take_while(|&&c| saphyr_anchor_char(c))
            .collect();
        Some((name, at))
    }

    /// Where the header (`|` or `>`) of the block scalar whose span starts at
    /// `start` is: saphyr-parser's span of a block scalar with content starts
    /// at the content.
    fn block_header(&self, start: usize) -> Option<usize> {
        let header = |c: char| c == '|' || c == '>';
        if self.chars.get(start).copied().is_some_and(header) {
            // An empty block scalar's span starts at its header.
            return Some(start);
        }
        self.token(start, header).map(|at| at.index())
    }

    /// The column of the last `-` entry indicator on the line of the block
    /// scalar header at `header`, before it (props and blanks between).
    fn entry_dash(&self, header: usize) -> Option<usize> {
        let line_start = self.chars[..header]
            .iter()
            .rposition(|&c| c == '\n' || c == '\r')
            .map_or(0, |i| i + 1);
        let at = |i: usize| self.chars.get(i).copied().filter(|_| i < header);
        let (mut i, mut dash) = (line_start, None);
        while let Some(c) = at(i) {
            match c {
                ' ' | '\t' => i += 1,
                '-' if at(i + 1).is_none_or(is_break_or_blank) => {
                    dash = Some(i - line_start);
                    i += 1;
                }
                '!' | '&' => {
                    while at(i).is_some_and(|c| !is_break_or_blank(c)) {
                        i += 1;
                    }
                }
                _ => break,
            }
        }
        dash
    }

    /// The first token in the gap before `end` that starts with a character
    /// `wanted` accepts, skipping blanks, comments, directives, tags,
    /// anchors and indicators.
    fn token(&self, end: usize, wanted: impl Fn(char) -> bool) -> Option<Marker> {
        let (mut i, mut line, mut col) = (self.gap.index(), self.gap.line(), self.gap.col());
        let at = |i: usize| self.chars.get(i).copied().filter(|_| i < end);
        while let Some(c) = at(i) {
            if wanted(c) {
                return Some(Marker::new(i, line, col));
            }
            let start = i;
            i += 1;
            match c {
                // A comment or a directive runs to the end of the line.
                '#' | '%' => {
                    while at(i).is_some_and(|c| c != '\n' && c != '\r') {
                        i += 1;
                    }
                }
                // A verbatim tag runs to its closing `>`.
                '!' if at(i) == Some('<') => {
                    while at(i).is_some_and(|c| c != '>') {
                        i += 1;
                    }
                    i += 1;
                }
                '!' => {
                    while at(i).is_some_and(|c| !is_break_or_blank(c)) {
                        i += 1;
                    }
                }
                '&' => {
                    while at(i).is_some_and(saphyr_anchor_char) {
                        i += 1;
                    }
                }
                _ => {}
            }
            for c in &self.chars[start..i] {
                if *c == '\n' {
                    line += 1;
                    col = 0;
                } else {
                    col += 1;
                }
            }
        }
        None
    }
}

fn is_break_or_blank(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// saphyr-parser's `is_anchor_char`: the characters it reads into a name.
fn saphyr_anchor_char(c: char) -> bool {
    !is_break_or_blank(c) && !matches!(c, ',' | '[' | ']' | '{' | '}' | '\u{feff}' | '\0')
}

/// PyYAML's `scan_anchor`: a name of `[0-9A-Za-z_-]`, followed by a blank, a
/// line break, the end, or one of `?:,]}%@` and the backquote.
fn check_anchor_name(name: &str, next: Option<&char>) -> Result<(), String> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && next.is_none_or(|&c| {
            is_break_or_blank(c) || matches!(c, '?' | ':' | ',' | ']' | '}' | '%' | '@' | '`')
        });
    if valid {
        Ok(())
    } else {
        Err("while scanning an anchor, expected alphabetic or numeric character".into())
    }
}
