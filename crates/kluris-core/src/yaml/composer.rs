//! Port of PyYAML 6.0.3's `composer.py` (with `nodes.py` and the tag
//! resolution of `resolver.py`): events become a graph of nodes, anchors
//! shared, aliases resolved, tags resolved.

use std::collections::HashMap;
use std::rc::Rc;

use super::parse::{YamlError, marked};
use super::parser::{Event, EventKind, Parser};
use super::reader::Mark;
use super::resolve::{self, ImplicitTag, ScalarTag};

/// The deepest nesting composed. PyYAML's recursive composer runs out of
/// Python's 1000 frames at about 493 to 495 levels (fewer the deeper its
/// caller already is), which is an error there too.
const MAX_DEPTH: usize = 490;

/// A node's tag after resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeTag {
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

pub enum Body {
    /// A scalar's value; `surrogate` when an escape made a lone surrogate
    /// (U+FFFD stands in for it).
    Scalar {
        text: String,
        surrogate: bool,
    },
    Seq(Vec<Rc<Node>>),
    Map(Vec<(Rc<Node>, Rc<Node>)>),
    /// An alias to a collection still being composed (the node with this
    /// id). PyYAML builds a cycle; the value model cannot hold one, so it is
    /// an error if it is still there once the value is built.
    Recursive(usize),
}

pub struct Node {
    pub id: usize,
    pub tag: NodeTag,
    /// The node's `start_mark`.
    pub mark: Mark,
    pub body: Body,
}

const CORE: &str = "tag:yaml.org,2002:";

/// An explicit tag (handles already expanded) as a [`NodeTag`].
fn explicit(tag: &str) -> NodeTag {
    match tag.strip_prefix(CORE) {
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
        None => NodeTag::Other(tag.to_string()),
    }
}

/// `Resolver.resolve(ScalarNode, value, implicit)`.
fn resolve_scalar(value: &str, implicit: (bool, bool)) -> NodeTag {
    if !implicit.0 {
        return NodeTag::Scalar(ScalarTag::Str);
    }
    match resolve::implicit_tag(value) {
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

fn error(
    context: Option<&str>,
    context_mark: Option<Mark>,
    problem: &str,
    mark: Mark,
) -> YamlError {
    marked("ComposerError", context, context_mark, problem, mark)
}

/// An anchor seen so far: its node once composed, or the id of the
/// collection still being composed.
struct Anchor {
    node: Option<Rc<Node>>,
    id: usize,
    start: Mark,
}

/// A collection being composed: its items so far (a mapping's keys and
/// values alternate) and the event that ends it.
struct Open {
    id: usize,
    tag: NodeTag,
    start: Mark,
    anchor: Option<String>,
    map: bool,
    end: fn(&EventKind) -> bool,
    items: Vec<Rc<Node>>,
}

enum Composed {
    Node(Rc<Node>),
    Open(Open),
}

pub struct Composer {
    parser: Parser,
    anchors: HashMap<String, Anchor>,
    next_id: usize,
}

impl Composer {
    pub fn new(parser: Parser) -> Composer {
        Composer {
            parser,
            anchors: HashMap::new(),
            next_id: 0,
        }
    }

    fn get_event(&mut self) -> Result<Event, YamlError> {
        match self.parser.get_event()? {
            Some(event) => Ok(event),
            None => Err(error(
                None,
                None,
                "unexpected end of the event stream",
                Mark::default(),
            )),
        }
    }

    fn check_event(&mut self, choice: fn(&EventKind) -> bool) -> Result<bool, YamlError> {
        Ok(self
            .parser
            .peek_event()?
            .is_some_and(|event| choice(&event.kind)))
    }

    fn new_id(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }

    /// `get_single_node`: the root of the one document (`None` for an empty
    /// stream).
    pub fn get_single_node(&mut self) -> Result<Option<Rc<Node>>, YamlError> {
        self.get_event()?;
        let mut document = None;
        if !self.check_event(|e| matches!(e, EventKind::StreamEnd))? {
            document = Some(self.compose_document()?);
        }
        if !self.check_event(|e| matches!(e, EventKind::StreamEnd))? {
            let event = self.get_event()?;
            return Err(error(
                Some("expected a single document in the stream"),
                document.as_ref().map(|node| node.mark),
                "but found another document",
                event.start,
            ));
        }
        self.get_event()?;
        Ok(document)
    }

    fn compose_document(&mut self) -> Result<Rc<Node>, YamlError> {
        self.get_event()?;
        let node = self.compose_node()?;
        self.get_event()?;
        self.anchors.clear();
        Ok(node)
    }

    /// `compose_node` with an explicit stack instead of recursion: a
    /// collection being composed stays open until its end event.
    fn compose_node(&mut self) -> Result<Rc<Node>, YamlError> {
        let mut stack: Vec<Open> = Vec::new();
        loop {
            let node = match stack.last() {
                Some(open) if self.check_event(open.end)? => {
                    let Some(open) = stack.pop() else {
                        unreachable!("checked above")
                    };
                    self.get_event()?;
                    self.close(open)
                }
                _ => match self.compose_start(stack.len())? {
                    Composed::Node(node) => node,
                    Composed::Open(open) => {
                        stack.push(open);
                        continue;
                    }
                },
            };
            match stack.last_mut() {
                Some(open) => open.items.push(node),
                None => return Ok(node),
            }
        }
    }

    /// The start of `compose_node`: an alias, a scalar, or a collection
    /// that opens.
    fn compose_start(&mut self, depth: usize) -> Result<Composed, YamlError> {
        if self.check_event(|e| matches!(e, EventKind::Alias(_)))? {
            let event = self.get_event()?;
            let EventKind::Alias(name) = &event.kind else {
                unreachable!("checked above")
            };
            let Some(anchor) = self.anchors.get(name) else {
                return Err(error(
                    None,
                    None,
                    &format!("found undefined alias '{name}'"),
                    event.start,
                ));
            };
            return Ok(Composed::Node(match &anchor.node {
                Some(node) => Rc::clone(node),
                None => {
                    let target = anchor.id;
                    let id = self.new_id();
                    Rc::new(Node {
                        id,
                        tag: NodeTag::Seq,
                        mark: event.start,
                        body: Body::Recursive(target),
                    })
                }
            }));
        }
        let Some(event) = self.parser.peek_event()? else {
            return Err(error(
                None,
                None,
                "unexpected end of the event stream",
                Mark::default(),
            ));
        };
        let start = event.start;
        let anchor = match &event.kind {
            EventKind::Scalar { anchor, .. }
            | EventKind::SequenceStart { anchor, .. }
            | EventKind::MappingStart { anchor, .. } => anchor.clone(),
            _ => None,
        };
        if let Some(name) = &anchor
            && let Some(first) = self.anchors.get(name)
        {
            return Err(error(
                Some(&format!(
                    "found duplicate anchor '{name}'; first occurrence"
                )),
                Some(first.start),
                "second occurrence",
                start,
            ));
        }
        if depth >= MAX_DEPTH {
            return Err(YamlError::new(
                "RecursionError: maximum recursion depth exceeded".into(),
                start,
            ));
        }
        let event = self.get_event()?;
        let (tag, default, map, end): (_, _, _, fn(&EventKind) -> bool) = match event.kind {
            EventKind::Scalar {
                tag,
                implicit,
                value,
                surrogate,
                ..
            } => {
                // `compose_scalar_node`
                let tag = match tag.as_deref() {
                    None | Some("!") => resolve_scalar(&value, implicit),
                    Some(tag) => explicit(tag),
                };
                let id = self.new_id();
                let node = Rc::new(Node {
                    id,
                    tag,
                    mark: event.start,
                    body: Body::Scalar {
                        text: value,
                        surrogate,
                    },
                });
                self.register(anchor, id, event.start, Some(Rc::clone(&node)));
                return Ok(Composed::Node(node));
            }
            EventKind::SequenceStart { tag, .. } => (tag, NodeTag::Seq, false, |e| {
                matches!(e, EventKind::SequenceEnd)
            }),
            EventKind::MappingStart { tag, .. } => (tag, NodeTag::Map, true, |e| {
                matches!(e, EventKind::MappingEnd)
            }),
            other => {
                return Err(error(
                    None,
                    None,
                    &format!("expected a node, but found {other:?}"),
                    start,
                ));
            }
        };
        // `compose_sequence_node` / `compose_mapping_node`: the anchor names
        // the collection from its start, so an alias inside it is recursive.
        let tag = match tag.as_deref() {
            None | Some("!") => default,
            Some(tag) => explicit(tag),
        };
        let id = self.new_id();
        self.register(anchor.clone(), id, event.start, None);
        Ok(Composed::Open(Open {
            id,
            tag,
            start: event.start,
            anchor,
            map,
            end,
            items: Vec::new(),
        }))
    }

    fn register(&mut self, anchor: Option<String>, id: usize, start: Mark, node: Option<Rc<Node>>) {
        if let Some(name) = anchor {
            self.anchors.insert(name, Anchor { node, id, start });
        }
    }

    /// The node of a collection whose end event came.
    fn close(&mut self, open: Open) -> Rc<Node> {
        let body = if open.map {
            let mut items = open.items.into_iter();
            let mut pairs = Vec::new();
            while let (Some(key), Some(value)) = (items.next(), items.next()) {
                pairs.push((key, value));
            }
            Body::Map(pairs)
        } else {
            Body::Seq(open.items)
        };
        let node = Rc::new(Node {
            id: open.id,
            tag: open.tag,
            mark: open.start,
            body,
        });
        self.complete(open.anchor, &node);
        node
    }

    /// A collection's anchor names the finished node from now on.
    fn complete(&mut self, anchor: Option<String>, node: &Rc<Node>) {
        if let Some(name) = anchor
            && let Some(entry) = self.anchors.get_mut(&name)
            && entry.id == node.id
        {
            entry.node = Some(Rc::clone(node));
        }
    }
}
