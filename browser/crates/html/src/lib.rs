//! `html` — in-house WHATWG HTML5 parser (plan/05).
//!
//! Bytes → chars → tokens → DOM. `html5ever` is a test oracle only, never
//! a dependency.

pub mod dom;
pub mod entities;
pub mod parser;
pub mod tokenizer;
pub mod tree;

pub use dom::{
    Attribute, Document, ElementData, Namespace, Node, NodeData, NodeId, ParseError, QuirksMode,
    Truncated,
};
pub use parser::{DomSink, NullSink, Parser};
pub use tokenizer::{Token, Tokenizer};
pub use tree::{Step, TreeBuilder};

/// Default node budget (plan/05 §5.3).
pub const DEFAULT_MAX_NODES: usize = 500_000;
/// Default nesting budget (plan/05 §5.3).
pub const DEFAULT_MAX_DEPTH: usize = 512;
/// Default attribute value budget in bytes (plan/05 §5.3).
pub const DEFAULT_MAX_ATTR_LEN: usize = 64 * 1024;

/// Parser resource budgets (fixed API from plan/05 §5.3).
#[derive(Debug, Clone, Copy)]
pub struct ParseOpts {
    /// Max arena nodes (document included).
    pub max_nodes: usize,
    /// Max open-element nesting.
    pub max_depth: usize,
    /// Max attribute value length in bytes.
    pub max_attr_len: usize,
    /// Script execution enabled (suspend/resume wiring).
    pub scripting_enabled: bool,
}

impl Default for ParseOpts {
    fn default() -> Self {
        Self {
            max_nodes: DEFAULT_MAX_NODES,
            max_depth: DEFAULT_MAX_DEPTH,
            max_attr_len: DEFAULT_MAX_ATTR_LEN,
            scripting_enabled: false,
        }
    }
}

/// Parse a whole document in one shot.
#[must_use]
pub fn parse_full(bytes: &[u8], url: &url::Url, opts: ParseOpts) -> Document {
    let mut parser = Parser::new(url.clone(), Box::new(NullSink), opts);
    parser.push(bytes);
    parser.finish()
}

/// Parse an `innerHTML`-style fragment under `context_tag`.
#[must_use]
pub fn parse_fragment(
    bytes: &[u8],
    url: &url::Url,
    context_tag: &str,
    opts: ParseOpts,
) -> Document {
    let mut parser = Parser::new_fragment(url.clone(), Box::new(NullSink), opts, context_tag);
    parser.push(bytes);
    parser.finish()
}
