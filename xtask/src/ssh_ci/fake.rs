//! A fake ssh and scp for the tests of `cargo xtask windows` and `cargo xtask
//! linux`: it answers each call with the next answer it was given, and keeps
//! the calls.

use std::collections::VecDeque;
use std::io;

use super::runner::{Call, Captured, Runner, Stream, Streamed};

/// What the fake answers a call with, in the order the calls come.
pub enum Answer {
    Capture(Result<Captured, String>),
    Stream(Vec<(Stream, &'static str)>, Result<Streamed, String>),
}

#[derive(Default)]
pub struct Fake {
    answers: VecDeque<Answer>,
    pub calls: Vec<Call>,
}

impl Fake {
    pub fn new(answers: Vec<Answer>) -> Self {
        Self {
            answers: answers.into(),
            calls: Vec::new(),
        }
    }

    fn next(&mut self, call: &Call) -> Answer {
        self.calls.push(call.clone());
        self.answers
            .pop_front()
            .unwrap_or_else(|| panic!("no answer left for {call:?}"))
    }

    /// The programs called, each with its last word (the host's command, or scp's target).
    pub fn called(&self) -> Vec<String> {
        self.calls
            .iter()
            .map(|call| format!("{} {}", call.program, call.args.join(" ")))
            .collect()
    }
}

impl Runner for Fake {
    fn capture(&mut self, call: &Call) -> Result<Captured, String> {
        match self.next(call) {
            Answer::Capture(answer) => answer,
            Answer::Stream(..) => panic!("{call:?} was captured, the fake had a stream for it"),
        }
    }

    fn stream(
        &mut self,
        call: &Call,
        sink: &mut dyn FnMut(Stream, &[u8]) -> io::Result<()>,
        _stop: &dyn Fn() -> bool,
    ) -> Result<Streamed, String> {
        match self.next(call) {
            Answer::Stream(chunks, ended) => {
                for (stream, text) in chunks {
                    sink(stream, text.as_bytes()).map_err(|e| e.to_string())?;
                }
                ended
            }
            Answer::Capture(_) => panic!("{call:?} was streamed, the fake had a capture for it"),
        }
    }
}

pub fn said(code: i32, stdout: &str) -> Answer {
    Answer::Capture(Ok(Captured {
        code: Some(code),
        stdout: stdout.into(),
        stderr: String::new(),
        timed_out: false,
    }))
}

pub fn timed_out() -> Answer {
    Answer::Capture(Ok(Captured {
        code: None,
        stdout: String::new(),
        stderr: String::new(),
        timed_out: true,
    }))
}
