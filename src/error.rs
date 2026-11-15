//! One error type shared by every stage of the compiler.

use crate::token::Span;

/// Something in the source that Zeno could not understand, and where.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub message: String,
    pub span: Span,
    /// Something more that helps, shown under the error. For example, where
    /// a type that was worked out came from.
    pub note: Option<String>,
}

/// "1 argument", "2 arguments", "0 arguments": a count and its word, so
/// that no message has to say "argument(s)".
pub fn counted(count: usize, word: &str) -> String {
    if count == 1 { format!("1 {word}") } else { format!("{count} {word}s") }
}

/// "1 was", "3 were", "0 were": the count and the verb that goes with it.
pub fn were(count: usize) -> String {
    if count == 1 { "1 was".to_string() } else { format!("{count} were") }
}

impl Error {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Error { message: message.into(), span, note: None }
    }
}
