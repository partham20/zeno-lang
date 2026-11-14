//! One error type shared by every stage of the compiler.

use crate::token::Span;

/// Something in the source that Zeno could not understand, and where.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub message: String,
    pub span: Span,
}

impl Error {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Error { message: message.into(), span }
    }
}
