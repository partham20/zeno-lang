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

impl Error {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Error { message: message.into(), span, note: None }
    }
}
