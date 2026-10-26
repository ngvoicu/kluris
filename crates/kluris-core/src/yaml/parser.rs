//! Port of PyYAML 6.0.3's `parser.py` (and the events of `events.py`): the
//! state machine that turns tokens into events. Each `State` is one of the
//! Python `parse_*` methods; `states` is its stack of them.

use std::collections::HashMap;

use super::parse::{YamlError, marked};
use super::reader::Mark;
use super::scanner::{DirectiveValue, Scanner, Token, TokenKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    StreamStart,
    StreamEnd,
    DocumentStart,
    DocumentEnd,
    Alias(String),
    Scalar {
        anchor: Option<String>,
        tag: Option<String>,
        /// `(plain, quoted)`: which implicit resolution applies.
        implicit: (bool, bool),
        value: String,
        surrogate: bool,
    },
    SequenceStart {
        anchor: Option<String>,
        tag: Option<String>,
    },
    SequenceEnd,
    MappingStart {
        anchor: Option<String>,
        tag: Option<String>,
    },
    MappingEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub kind: EventKind,
    pub start: Mark,
    pub end: Mark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    StreamStart,
    ImplicitDocumentStart,
    DocumentStart,
    DocumentEnd,
    DocumentContent,
    BlockNode,
    BlockSequenceFirstEntry,
    BlockSequenceEntry,
    IndentlessSequenceEntry,
    BlockMappingFirstKey,
    BlockMappingKey,
    BlockMappingValue,
    FlowSequenceFirstEntry,
    FlowSequenceEntry,
    FlowSequenceEntryMappingKey,
    FlowSequenceEntryMappingValue,
    FlowSequenceEntryMappingEnd,
    FlowMappingFirstKey,
    FlowMappingKey,
    FlowMappingValue,
    FlowMappingEmptyValue,
}

type Parse<T> = Result<T, YamlError>;

fn error(
    context: Option<&str>,
    context_mark: Option<Mark>,
    problem: String,
    mark: Mark,
) -> YamlError {
    marked("ParserError", context, context_mark, &problem, mark)
}

macro_rules! token_is {
    ($($pattern:pat_param)|+) => {
        |kind: &TokenKind| matches!(kind, $($pattern)|+)
    };
}

pub struct Parser {
    scanner: Scanner,
    current_event: Option<Event>,
    tag_handles: HashMap<String, String>,
    states: Vec<State>,
    marks: Vec<Mark>,
    /// `None` once the stream has ended.
    state: Option<State>,
    yaml_version: Option<(String, String)>,
}

fn default_tags() -> HashMap<String, String> {
    HashMap::from([
        ("!".to_string(), "!".to_string()),
        ("!!".to_string(), "tag:yaml.org,2002:".to_string()),
    ])
}

impl Parser {
    pub fn new(scanner: Scanner) -> Parser {
        Parser {
            scanner,
            current_event: None,
            tag_handles: HashMap::new(),
            states: Vec::new(),
            marks: Vec::new(),
            state: Some(State::StreamStart),
            yaml_version: None,
        }
    }

    pub fn peek_event(&mut self) -> Parse<Option<&Event>> {
        if self.current_event.is_none()
            && let Some(state) = self.state
        {
            self.current_event = Some(self.dispatch(state)?);
        }
        Ok(self.current_event.as_ref())
    }

    pub fn get_event(&mut self) -> Parse<Option<Event>> {
        self.peek_event()?;
        Ok(self.current_event.take())
    }

    // --- tokens -----------------------------------------------------------

    fn check_token(&mut self, choice: fn(&TokenKind) -> bool) -> Parse<bool> {
        self.scanner.check_token(&[choice])
    }

    /// The next token; the scanner always ends with a stream end, after
    /// which the parser asks for no more.
    fn peek_token(&mut self) -> Parse<Token> {
        match self.scanner.peek_token()? {
            Some(token) => Ok(token.clone()),
            None => Err(error(
                None,
                None,
                "unexpected end of the token stream".into(),
                Mark::default(),
            )),
        }
    }

    fn get_token(&mut self) -> Parse<Token> {
        match self.scanner.get_token()? {
            Some(token) => Ok(token),
            None => Err(error(
                None,
                None,
                "unexpected end of the token stream".into(),
                Mark::default(),
            )),
        }
    }

    fn pop_state(&mut self) -> State {
        self.states.pop().unwrap_or(State::DocumentEnd)
    }

    fn event(kind: EventKind, start: Mark, end: Mark) -> Event {
        Event { kind, start, end }
    }

    fn dispatch(&mut self, state: State) -> Parse<Event> {
        match state {
            State::StreamStart => self.parse_stream_start(),
            State::ImplicitDocumentStart => self.parse_implicit_document_start(),
            State::DocumentStart => self.parse_document_start(),
            State::DocumentEnd => self.parse_document_end(),
            State::DocumentContent => self.parse_document_content(),
            State::BlockNode => self.parse_node(true, false),
            State::BlockSequenceFirstEntry => self.parse_block_sequence_first_entry(),
            State::BlockSequenceEntry => self.parse_block_sequence_entry(),
            State::IndentlessSequenceEntry => self.parse_indentless_sequence_entry(),
            State::BlockMappingFirstKey => self.parse_block_mapping_first_key(),
            State::BlockMappingKey => self.parse_block_mapping_key(),
            State::BlockMappingValue => self.parse_block_mapping_value(),
            State::FlowSequenceFirstEntry => self.parse_flow_sequence_first_entry(),
            State::FlowSequenceEntry => self.parse_flow_sequence_entry(false),
            State::FlowSequenceEntryMappingKey => self.parse_flow_sequence_entry_mapping_key(),
            State::FlowSequenceEntryMappingValue => self.parse_flow_sequence_entry_mapping_value(),
            State::FlowSequenceEntryMappingEnd => self.parse_flow_sequence_entry_mapping_end(),
            State::FlowMappingFirstKey => self.parse_flow_mapping_first_key(),
            State::FlowMappingKey => self.parse_flow_mapping_key(false),
            State::FlowMappingValue => self.parse_flow_mapping_value(),
            State::FlowMappingEmptyValue => self.parse_flow_mapping_empty_value(),
        }
    }

    // --- stream and documents ---------------------------------------------

    fn parse_stream_start(&mut self) -> Parse<Event> {
        let token = self.get_token()?;
        self.state = Some(State::ImplicitDocumentStart);
        Ok(Self::event(EventKind::StreamStart, token.start, token.end))
    }

    fn parse_implicit_document_start(&mut self) -> Parse<Event> {
        if !self.check_token(token_is!(
            TokenKind::Directive { .. } | TokenKind::DocumentStart | TokenKind::StreamEnd
        ))? {
            self.tag_handles = default_tags();
            let token = self.peek_token()?;
            self.states.push(State::DocumentEnd);
            self.state = Some(State::BlockNode);
            Ok(Self::event(
                EventKind::DocumentStart,
                token.start,
                token.start,
            ))
        } else {
            self.parse_document_start()
        }
    }

    fn parse_document_start(&mut self) -> Parse<Event> {
        while self.check_token(token_is!(TokenKind::DocumentEnd))? {
            self.get_token()?;
        }
        if !self.check_token(token_is!(TokenKind::StreamEnd))? {
            let token = self.peek_token()?;
            let start = token.start;
            self.process_directives()?;
            if !self.check_token(token_is!(TokenKind::DocumentStart))? {
                let token = self.peek_token()?;
                return Err(error(
                    None,
                    None,
                    format!(
                        "expected '<document start>', but found '{}'",
                        token.kind.id()
                    ),
                    token.start,
                ));
            }
            let token = self.get_token()?;
            self.states.push(State::DocumentEnd);
            self.state = Some(State::DocumentContent);
            Ok(Self::event(EventKind::DocumentStart, start, token.end))
        } else {
            let token = self.get_token()?;
            self.state = None;
            Ok(Self::event(EventKind::StreamEnd, token.start, token.end))
        }
    }

    fn parse_document_end(&mut self) -> Parse<Event> {
        let token = self.peek_token()?;
        let start = token.start;
        let mut end = token.start;
        if self.check_token(token_is!(TokenKind::DocumentEnd))? {
            end = self.get_token()?.end;
        }
        self.state = Some(State::DocumentStart);
        Ok(Self::event(EventKind::DocumentEnd, start, end))
    }

    fn parse_document_content(&mut self) -> Parse<Event> {
        if self.check_token(token_is!(
            TokenKind::Directive { .. }
                | TokenKind::DocumentStart
                | TokenKind::DocumentEnd
                | TokenKind::StreamEnd
        ))? {
            let mark = self.peek_token()?.start;
            self.state = Some(self.pop_state());
            Ok(Self::empty_scalar(mark))
        } else {
            self.parse_node(true, false)
        }
    }

    fn process_directives(&mut self) -> Parse<()> {
        self.yaml_version = None;
        self.tag_handles = HashMap::new();
        while self.check_token(token_is!(TokenKind::Directive { .. }))? {
            let token = self.get_token()?;
            let TokenKind::Directive { name, value } = token.kind else {
                continue;
            };
            match (name.as_str(), value) {
                ("YAML", DirectiveValue::Yaml(major, minor)) => {
                    if self.yaml_version.is_some() {
                        return Err(error(
                            None,
                            None,
                            "found duplicate YAML directive".into(),
                            token.start,
                        ));
                    }
                    if major != "1" {
                        return Err(error(
                            None,
                            None,
                            "found incompatible YAML document (version 1.* is required)".into(),
                            token.start,
                        ));
                    }
                    self.yaml_version = Some((major, minor));
                }
                ("TAG", DirectiveValue::Tag(handle, prefix)) => {
                    if self.tag_handles.contains_key(&handle) {
                        return Err(error(
                            None,
                            None,
                            format!("duplicate tag handle '{handle}'"),
                            token.start,
                        ));
                    }
                    self.tag_handles.insert(handle, prefix);
                }
                _ => {}
            }
        }
        for (key, value) in default_tags() {
            self.tag_handles.entry(key).or_insert(value);
        }
        Ok(())
    }

    // --- nodes ------------------------------------------------------------

    fn parse_node(&mut self, block: bool, indentless_sequence: bool) -> Parse<Event> {
        if self.check_token(token_is!(TokenKind::Alias(_)))? {
            let token = self.get_token()?;
            self.state = Some(self.pop_state());
            let TokenKind::Alias(name) = token.kind else {
                unreachable!("checked above")
            };
            return Ok(Self::event(EventKind::Alias(name), token.start, token.end));
        }
        let mut anchor = None;
        let mut tag_token = None;
        let mut start = None;
        let mut end = None;
        let mut tag_mark = None;
        if self.check_token(token_is!(TokenKind::Anchor(_)))? {
            let token = self.get_token()?;
            start = Some(token.start);
            end = Some(token.end);
            if let TokenKind::Anchor(name) = token.kind {
                anchor = Some(name);
            }
            if self.check_token(token_is!(TokenKind::Tag(..)))? {
                let token = self.get_token()?;
                tag_mark = Some(token.start);
                end = Some(token.end);
                if let TokenKind::Tag(handle, suffix) = token.kind {
                    tag_token = Some((handle, suffix));
                }
            }
        } else if self.check_token(token_is!(TokenKind::Tag(..)))? {
            let token = self.get_token()?;
            start = Some(token.start);
            tag_mark = Some(token.start);
            end = Some(token.end);
            if let TokenKind::Tag(handle, suffix) = token.kind {
                tag_token = Some((handle, suffix));
            }
            if self.check_token(token_is!(TokenKind::Anchor(_)))? {
                let token = self.get_token()?;
                end = Some(token.end);
                if let TokenKind::Anchor(name) = token.kind {
                    anchor = Some(name);
                }
            }
        }
        let mut tag = None;
        if let Some((handle, suffix)) = tag_token {
            tag = Some(match handle {
                Some(handle) => match self.tag_handles.get(&handle) {
                    Some(prefix) => format!("{prefix}{suffix}"),
                    None => {
                        return Err(error(
                            Some("while parsing a node"),
                            start,
                            format!("found undefined tag handle '{handle}'"),
                            tag_mark.unwrap_or_default(),
                        ));
                    }
                },
                None => suffix,
            });
        }
        let (start, mut end) = match (start, end) {
            (Some(start), Some(end)) => (start, end),
            _ => {
                let mark = self.peek_token()?.start;
                (mark, mark)
            }
        };
        let implicit = tag.is_none() || tag.as_deref() == Some("!");
        if indentless_sequence && self.check_token(token_is!(TokenKind::BlockEntry))? {
            end = self.peek_token()?.end;
            self.state = Some(State::IndentlessSequenceEntry);
            return Ok(Self::event(
                EventKind::SequenceStart { anchor, tag },
                start,
                end,
            ));
        }
        let token = self.peek_token()?;
        match token.kind {
            TokenKind::Scalar {
                value,
                plain,
                surrogate,
                ..
            } => {
                self.get_token()?;
                let implicit = if (plain && tag.is_none()) || tag.as_deref() == Some("!") {
                    (true, false)
                } else if tag.is_none() {
                    (false, true)
                } else {
                    (false, false)
                };
                self.state = Some(self.pop_state());
                Ok(Self::event(
                    EventKind::Scalar {
                        anchor,
                        tag,
                        implicit,
                        value,
                        surrogate,
                    },
                    start,
                    token.end,
                ))
            }
            TokenKind::FlowSequenceStart => {
                self.state = Some(State::FlowSequenceFirstEntry);
                Ok(Self::event(
                    EventKind::SequenceStart { anchor, tag },
                    start,
                    token.end,
                ))
            }
            TokenKind::FlowMappingStart => {
                self.state = Some(State::FlowMappingFirstKey);
                Ok(Self::event(
                    EventKind::MappingStart { anchor, tag },
                    start,
                    token.end,
                ))
            }
            TokenKind::BlockSequenceStart if block => {
                self.state = Some(State::BlockSequenceFirstEntry);
                Ok(Self::event(
                    EventKind::SequenceStart { anchor, tag },
                    start,
                    token.start,
                ))
            }
            TokenKind::BlockMappingStart if block => {
                self.state = Some(State::BlockMappingFirstKey);
                Ok(Self::event(
                    EventKind::MappingStart { anchor, tag },
                    start,
                    token.start,
                ))
            }
            _ if anchor.is_some() || tag.is_some() => {
                self.state = Some(self.pop_state());
                Ok(Self::event(
                    EventKind::Scalar {
                        anchor,
                        tag,
                        implicit: (implicit, false),
                        value: String::new(),
                        surrogate: false,
                    },
                    start,
                    end,
                ))
            }
            other => Err(error(
                Some(if block {
                    "while parsing a block node"
                } else {
                    "while parsing a flow node"
                }),
                Some(start),
                format!("expected the node content, but found '{}'", other.id()),
                token.start,
            )),
        }
    }

    // --- block collections ------------------------------------------------

    fn parse_block_sequence_first_entry(&mut self) -> Parse<Event> {
        let token = self.get_token()?;
        self.marks.push(token.start);
        self.parse_block_sequence_entry()
    }

    fn parse_block_sequence_entry(&mut self) -> Parse<Event> {
        if self.check_token(token_is!(TokenKind::BlockEntry))? {
            let token = self.get_token()?;
            if !self.check_token(token_is!(TokenKind::BlockEntry | TokenKind::BlockEnd))? {
                self.states.push(State::BlockSequenceEntry);
                return self.parse_node(true, false);
            }
            self.state = Some(State::BlockSequenceEntry);
            return Ok(Self::empty_scalar(token.end));
        }
        if !self.check_token(token_is!(TokenKind::BlockEnd))? {
            let token = self.peek_token()?;
            return Err(error(
                Some("while parsing a block collection"),
                self.marks.last().copied(),
                format!("expected <block end>, but found '{}'", token.kind.id()),
                token.start,
            ));
        }
        let token = self.get_token()?;
        self.state = Some(self.pop_state());
        self.marks.pop();
        Ok(Self::event(EventKind::SequenceEnd, token.start, token.end))
    }

    fn parse_indentless_sequence_entry(&mut self) -> Parse<Event> {
        if self.check_token(token_is!(TokenKind::BlockEntry))? {
            let token = self.get_token()?;
            if !self.check_token(token_is!(
                TokenKind::BlockEntry | TokenKind::Key | TokenKind::Value | TokenKind::BlockEnd
            ))? {
                self.states.push(State::IndentlessSequenceEntry);
                return self.parse_node(true, false);
            }
            self.state = Some(State::IndentlessSequenceEntry);
            return Ok(Self::empty_scalar(token.end));
        }
        let token = self.peek_token()?;
        self.state = Some(self.pop_state());
        Ok(Self::event(
            EventKind::SequenceEnd,
            token.start,
            token.start,
        ))
    }

    fn parse_block_mapping_first_key(&mut self) -> Parse<Event> {
        let token = self.get_token()?;
        self.marks.push(token.start);
        self.parse_block_mapping_key()
    }

    fn parse_block_mapping_key(&mut self) -> Parse<Event> {
        if self.check_token(token_is!(TokenKind::Key))? {
            let token = self.get_token()?;
            if !self.check_token(token_is!(
                TokenKind::Key | TokenKind::Value | TokenKind::BlockEnd
            ))? {
                self.states.push(State::BlockMappingValue);
                return self.parse_node(true, true);
            }
            self.state = Some(State::BlockMappingValue);
            return Ok(Self::empty_scalar(token.end));
        }
        if !self.check_token(token_is!(TokenKind::BlockEnd))? {
            let token = self.peek_token()?;
            return Err(error(
                Some("while parsing a block mapping"),
                self.marks.last().copied(),
                format!("expected <block end>, but found '{}'", token.kind.id()),
                token.start,
            ));
        }
        let token = self.get_token()?;
        self.state = Some(self.pop_state());
        self.marks.pop();
        Ok(Self::event(EventKind::MappingEnd, token.start, token.end))
    }

    fn parse_block_mapping_value(&mut self) -> Parse<Event> {
        if self.check_token(token_is!(TokenKind::Value))? {
            let token = self.get_token()?;
            if !self.check_token(token_is!(
                TokenKind::Key | TokenKind::Value | TokenKind::BlockEnd
            ))? {
                self.states.push(State::BlockMappingKey);
                return self.parse_node(true, true);
            }
            self.state = Some(State::BlockMappingKey);
            return Ok(Self::empty_scalar(token.end));
        }
        self.state = Some(State::BlockMappingKey);
        let mark = self.peek_token()?.start;
        Ok(Self::empty_scalar(mark))
    }

    // --- flow collections -------------------------------------------------

    fn parse_flow_sequence_first_entry(&mut self) -> Parse<Event> {
        let token = self.get_token()?;
        self.marks.push(token.start);
        self.parse_flow_sequence_entry(true)
    }

    fn parse_flow_sequence_entry(&mut self, first: bool) -> Parse<Event> {
        if !self.check_token(token_is!(TokenKind::FlowSequenceEnd))? {
            if !first {
                if self.check_token(token_is!(TokenKind::FlowEntry))? {
                    self.get_token()?;
                } else {
                    let token = self.peek_token()?;
                    return Err(error(
                        Some("while parsing a flow sequence"),
                        self.marks.last().copied(),
                        format!("expected ',' or ']', but got '{}'", token.kind.id()),
                        token.start,
                    ));
                }
            }
            if self.check_token(token_is!(TokenKind::Key))? {
                let token = self.peek_token()?;
                self.state = Some(State::FlowSequenceEntryMappingKey);
                return Ok(Self::event(
                    EventKind::MappingStart {
                        anchor: None,
                        tag: None,
                    },
                    token.start,
                    token.end,
                ));
            } else if !self.check_token(token_is!(TokenKind::FlowSequenceEnd))? {
                self.states.push(State::FlowSequenceEntry);
                return self.parse_node(false, false);
            }
        }
        let token = self.get_token()?;
        self.state = Some(self.pop_state());
        self.marks.pop();
        Ok(Self::event(EventKind::SequenceEnd, token.start, token.end))
    }

    fn parse_flow_sequence_entry_mapping_key(&mut self) -> Parse<Event> {
        let token = self.get_token()?;
        if !self.check_token(token_is!(
            TokenKind::Value | TokenKind::FlowEntry | TokenKind::FlowSequenceEnd
        ))? {
            self.states.push(State::FlowSequenceEntryMappingValue);
            return self.parse_node(false, false);
        }
        self.state = Some(State::FlowSequenceEntryMappingValue);
        Ok(Self::empty_scalar(token.end))
    }

    fn parse_flow_sequence_entry_mapping_value(&mut self) -> Parse<Event> {
        if self.check_token(token_is!(TokenKind::Value))? {
            let token = self.get_token()?;
            if !self.check_token(token_is!(TokenKind::FlowEntry | TokenKind::FlowSequenceEnd))? {
                self.states.push(State::FlowSequenceEntryMappingEnd);
                return self.parse_node(false, false);
            }
            self.state = Some(State::FlowSequenceEntryMappingEnd);
            return Ok(Self::empty_scalar(token.end));
        }
        self.state = Some(State::FlowSequenceEntryMappingEnd);
        let mark = self.peek_token()?.start;
        Ok(Self::empty_scalar(mark))
    }

    fn parse_flow_sequence_entry_mapping_end(&mut self) -> Parse<Event> {
        self.state = Some(State::FlowSequenceEntry);
        let mark = self.peek_token()?.start;
        Ok(Self::event(EventKind::MappingEnd, mark, mark))
    }

    fn parse_flow_mapping_first_key(&mut self) -> Parse<Event> {
        let token = self.get_token()?;
        self.marks.push(token.start);
        self.parse_flow_mapping_key(true)
    }

    fn parse_flow_mapping_key(&mut self, first: bool) -> Parse<Event> {
        if !self.check_token(token_is!(TokenKind::FlowMappingEnd))? {
            if !first {
                if self.check_token(token_is!(TokenKind::FlowEntry))? {
                    self.get_token()?;
                } else {
                    let token = self.peek_token()?;
                    return Err(error(
                        Some("while parsing a flow mapping"),
                        self.marks.last().copied(),
                        format!("expected ',' or '}}', but got '{}'", token.kind.id()),
                        token.start,
                    ));
                }
            }
            if self.check_token(token_is!(TokenKind::Key))? {
                let token = self.get_token()?;
                if !self.check_token(token_is!(
                    TokenKind::Value | TokenKind::FlowEntry | TokenKind::FlowMappingEnd
                ))? {
                    self.states.push(State::FlowMappingValue);
                    return self.parse_node(false, false);
                }
                self.state = Some(State::FlowMappingValue);
                return Ok(Self::empty_scalar(token.end));
            } else if !self.check_token(token_is!(TokenKind::FlowMappingEnd))? {
                self.states.push(State::FlowMappingEmptyValue);
                return self.parse_node(false, false);
            }
        }
        let token = self.get_token()?;
        self.state = Some(self.pop_state());
        self.marks.pop();
        Ok(Self::event(EventKind::MappingEnd, token.start, token.end))
    }

    fn parse_flow_mapping_value(&mut self) -> Parse<Event> {
        if self.check_token(token_is!(TokenKind::Value))? {
            let token = self.get_token()?;
            if !self.check_token(token_is!(TokenKind::FlowEntry | TokenKind::FlowMappingEnd))? {
                self.states.push(State::FlowMappingKey);
                return self.parse_node(false, false);
            }
            self.state = Some(State::FlowMappingKey);
            return Ok(Self::empty_scalar(token.end));
        }
        self.state = Some(State::FlowMappingKey);
        let mark = self.peek_token()?.start;
        Ok(Self::empty_scalar(mark))
    }

    fn parse_flow_mapping_empty_value(&mut self) -> Parse<Event> {
        self.state = Some(State::FlowMappingKey);
        let mark = self.peek_token()?.start;
        Ok(Self::empty_scalar(mark))
    }

    /// `process_empty_scalar`.
    fn empty_scalar(mark: Mark) -> Event {
        Self::event(
            EventKind::Scalar {
                anchor: None,
                tag: None,
                implicit: (true, false),
                value: String::new(),
                surrogate: false,
            },
            mark,
            mark,
        )
    }
}
