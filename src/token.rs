//! Tokens: the "words" of a Zeno program.
//!
//! The lexer turns source text into a flat list of these. Each token records
//! what it is (`TokenKind`) and where it came from (`Span`), so later stages
//! can report errors at the right line and column.

/// Where a token starts in the source file. Both numbers start at 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // --- Values written directly in the source ---
    Int(u64),
    Float(f64),
    /// A string. `formatted` is true for f"..." strings; the `{...}` parts
    /// inside are left as plain text here and picked apart by the parser.
    Str { value: String, formatted: bool },
    /// A name chosen by the programmer: a variable, function, type, ...
    Ident(String),

    // --- Keywords ---
    Let,
    Var,
    Const,
    Function,
    Return,
    If,
    Else,
    For,
    In,
    While,
    Unsafe,
    Break,
    Continue,
    Match,
    Case,
    Class,
    Enum,
    Try,
    Except,
    Raise,
    Raises,
    As,
    From,
    Import,
    And,
    Or,
    Not,
    Is,
    True,
    False,
    None,

    // --- Punctuation ---
    LParen,   // (
    RParen,   // )
    LBracket, // [
    RBracket, // ]
    LBrace,   // {
    RBrace,   // }
    Comma,    // ,
    Dot,      // .
    Colon,    // :
    Arrow,    // ->

    // --- Operators ---
    Plus,        // +
    Minus,       // -
    Star,        // *
    Slash,       // /
    SlashSlash,  // //
    Percent,     // %
    StarStar,    // **
    Assign,      // =
    PlusAssign,  // +=
    MinusAssign, // -=
    StarAssign,  // *=
    SlashAssign, // /=
    Amp,         // &   the bits both have
    Pipe,        // |   the bits either has
    Caret,       // ^   the bits one has and the other has not
    Tilde,       // ~   every bit turned over
    Shl,         // <<
    Shr,         // >>
    AmpAssign,   // &=
    PipeAssign,  // |=
    CaretAssign, // ^=
    ShlAssign,   // <<=
    ShrAssign,   // >>=
    Eq,          // ==
    NotEq,       // !=
    Lt,          // <
    LtEq,        // <=
    Gt,          // >
    GtEq,        // >=

    /// The end of a statement. Zeno has no semicolons, so a line break does this job.
    Newline,
    /// The end of the file. Always the last token.
    Eof,
}

/// If `word` is a keyword, return its token kind. Otherwise it is an identifier.
pub fn keyword(word: &str) -> Option<TokenKind> {
    use TokenKind::*;
    Some(match word {
        "let" => Let,
        "var" => Var,
        "const" => Const,
        "function" => Function,
        "return" => Return,
        "if" => If,
        "else" => Else,
        "for" => For,
        "in" => In,
        "while" => While,
        "unsafe" => Unsafe,
        "break" => Break,
        "continue" => Continue,
        "match" => Match,
        "case" => Case,
        "class" => Class,
        "enum" => Enum,
        "try" => Try,
        "except" => Except,
        "raise" => Raise,
        "raises" => Raises,
        "as" => As,
        "from" => From,
        "import" => Import,
        "and" => And,
        "or" => Or,
        "not" => Not,
        "is" => Is,
        "true" => True,
        "false" => False,
        "none" => None,
        _ => return Option::None,
    })
}

impl TokenKind {
    /// How to name this token in an error message, e.g. "`{`" or "the end of the line".
    pub fn describe(&self) -> String {
        use TokenKind::*;
        let text = match self {
            Int(_) | Float(_) => return "a number".to_string(),
            Str { .. } => return "a string".to_string(),
            Ident(name) => return format!("the name `{name}`"),
            Newline => return "the end of the line".to_string(),
            Eof => return "the end of the file".to_string(),

            Let => "let",
            Var => "var",
            Const => "const",
            Function => "function",
            Return => "return",
            If => "if",
            Else => "else",
            For => "for",
            In => "in",
            While => "while",
            Unsafe => "unsafe",
            Break => "break",
            Continue => "continue",
            Match => "match",
            Case => "case",
            Class => "class",
            Enum => "enum",
            Try => "try",
            Except => "except",
            Raise => "raise",
            Raises => "raises",
            As => "as",
            From => "from",
            Import => "import",
            And => "and",
            Or => "or",
            Not => "not",
            Is => "is",
            True => "true",
            False => "false",
            None => "none",

            LParen => "(",
            RParen => ")",
            LBracket => "[",
            RBracket => "]",
            LBrace => "{",
            RBrace => "}",
            Comma => ",",
            Dot => ".",
            Colon => ":",
            Arrow => "->",

            Plus => "+",
            Minus => "-",
            Star => "*",
            Slash => "/",
            SlashSlash => "//",
            Percent => "%",
            StarStar => "**",
            Assign => "=",
            PlusAssign => "+=",
            MinusAssign => "-=",
            StarAssign => "*=",
            SlashAssign => "/=",
            Amp => "&",
            Pipe => "|",
            Caret => "^",
            Tilde => "~",
            Shl => "<<",
            Shr => ">>",
            AmpAssign => "&=",
            PipeAssign => "|=",
            CaretAssign => "^=",
            ShlAssign => "<<=",
            ShrAssign => ">>=",
            Eq => "==",
            NotEq => "!=",
            Lt => "<",
            LtEq => "<=",
            Gt => ">",
            GtEq => ">=",
        };
        format!("`{text}`")
    }

    /// True if a statement cannot possibly end on this token, so a line break
    /// right after it means "the statement continues on the next line":
    ///
    /// ```text
    /// let total = price +
    ///     tax
    /// ```
    pub fn continues_line(&self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            Comma | Dot | Arrow
                | Plus | Minus | Star | Slash | SlashSlash | Percent | StarStar
                | Assign | PlusAssign | MinusAssign | StarAssign | SlashAssign
                | Amp | Pipe | Caret | Tilde | Shl | Shr
                | AmpAssign | PipeAssign | CaretAssign | ShlAssign | ShrAssign
                | Eq | NotEq | Lt | LtEq | Gt | GtEq
                | And | Or | Not | Is | In
        )
    }
}
