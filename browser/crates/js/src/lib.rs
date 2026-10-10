//! In-house Level 1 JavaScript: bounded lexer/parser, AST interpreter and heap.
//! External crates supply JSON/regular-expression/date auxiliaries, not JS.

pub mod ast;
mod builtins;
mod lexer;
mod parser;
pub mod value;
mod vm;

pub use ast::Program;
pub use parser::{parse, ParseOpts};
pub use value::{JsValue, ObjectId, PropertyKey};
pub use vm::{HostHooks, HostResult, Limits, NullHost, Vm};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("SyntaxError at {line}:{column}: {message}")]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}
impl ParseError {
    pub(crate) fn at(span: Span, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            line: span.line,
            column: span.column,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    SyntaxError,
    TypeError,
    ReferenceError,
    RangeError,
    Error,
    Timeout,
    MemoryLimit,
    Cancelled,
}
#[derive(Debug, Clone, thiserror::Error)]
#[error("{kind:?} at {line}:{column}: {message}")]
pub struct JsError {
    pub kind: ErrorKind,
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub stack: Vec<String>,
    pub thrown: Option<JsValue>,
}
impl JsError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            line: 0,
            column: 0,
            stack: Vec::new(),
            thrown: None,
        }
    }
    pub fn is_resource_limit(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::Timeout | ErrorKind::MemoryLimit | ErrorKind::Cancelled
        )
    }
}
