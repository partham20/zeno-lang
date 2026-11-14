//! The lexer: turns source text into tokens.
//!
//! It reads the source one character at a time, left to right, and never
//! looks more than two characters ahead. The whole thing is one loop
//! (`Lexer::run`) that looks at the next character and decides which kind of
//! token starts there.

use crate::error::Error;
use crate::token::{keyword, Span, Token, TokenKind};

/// Turn a whole source file into tokens.
///
/// On success the list always ends with `Eof`. On failure you get every error
/// found, not just the first, so the programmer can fix them in one go.
pub fn lex(source: &str) -> Result<Vec<Token>, Vec<Error>> {
    let mut lexer = Lexer::new(source);
    lexer.run();
    if lexer.errors.is_empty() {
        Ok(lexer.tokens)
    } else {
        Err(lexer.errors)
    }
}

struct Lexer {
    chars: Vec<char>,
    /// Index into `chars` of the next unread character.
    pos: usize,
    /// Line and column of the next unread character (both start at 1).
    line: usize,
    col: usize,
    /// How many `(` or `[` are currently open. Line breaks inside them are
    /// ignored, so a long call or list can be split over several lines.
    depth: usize,
    tokens: Vec<Token>,
    errors: Vec<Error>,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Lexer {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            depth: 0,
            tokens: Vec::new(),
            errors: Vec::new(),
        }
    }

    // ----- Reading characters -----

    /// The next character, without consuming it.
    fn peek(&self) -> Option<char> {
        self.peek_at(0)
    }

    /// The character `offset` places ahead, without consuming anything.
    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    /// Consume one character and keep the line/column counters in step.
    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    /// Consume the next character only if it is `expected`.
    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    // ----- Producing output -----

    fn push(&mut self, kind: TokenKind, span: Span) {
        self.tokens.push(Token { kind, span });
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.errors.push(Error::new(message, span));
    }

    // ----- The main loop -----

    fn run(&mut self) {
        while let Some(c) = self.peek() {
            // Remember where this token starts before consuming anything.
            let span = Span { line: self.line, col: self.col };

            match c {
                ' ' | '\t' | '\r' => {
                    self.advance();
                }
                '\n' => {
                    self.advance();
                    self.end_statement(span);
                }
                '#' => self.comment(),
                '"' | '\'' => self.string(false, span),
                'f' if matches!(self.peek_at(1), Some('"' | '\'')) => {
                    self.advance(); // the f
                    self.string(true, span);
                }
                c if c.is_ascii_digit() => self.number(span),
                c if c.is_alphabetic() || c == '_' => self.word(span),
                _ => self.symbol(span),
            }
        }

        // A file that doesn't end in a line break still ends its last statement.
        let end = Span { line: self.line, col: self.col };
        self.end_statement(end);
        self.push(TokenKind::Eof, end);
    }

    /// Called at every line break. Emits a `Newline` token only when the line
    /// break really ends a statement.
    fn end_statement(&mut self, span: Span) {
        if self.depth > 0 {
            return; // inside ( ) or [ ]
        }
        match self.tokens.last() {
            None => {}                                         // nothing before it
            Some(t) if t.kind == TokenKind::Newline => {}      // blank line
            Some(t) if t.kind.continues_line() => {}           // `a +` <newline> `b`
            Some(_) => self.push(TokenKind::Newline, span),
        }
    }

    /// `# ...` runs to the end of the line. The line break itself is left for
    /// the main loop, because it may still end a statement.
    fn comment(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.advance();
        }
    }

    /// A keyword or an identifier: letters, digits and underscores.
    fn word(&mut self, span: Span) {
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                text.push(c);
                self.advance();
            } else {
                break;
            }
        }
        let kind = keyword(&text).unwrap_or(TokenKind::Ident(text));
        self.push(kind, span);
    }

    /// A number: `42`, `1_000`, `3.14`, `1e-3`, `0xFF`, `0b1010`, `0o17`.
    fn number(&mut self, span: Span) {
        // Hex, binary and octal start with 0x, 0b, 0o.
        let radix = match (self.peek(), self.peek_at(1)) {
            (Some('0'), Some('x' | 'X')) => Some((16, "hexadecimal")),
            (Some('0'), Some('b' | 'B')) => Some((2, "binary")),
            (Some('0'), Some('o' | 'O')) => Some((8, "octal")),
            _ => None,
        };
        if let Some((radix, name)) = radix {
            self.advance(); // 0
            self.advance(); // x, b or o
            let digits = self.digits(|c| c.is_ascii_alphanumeric());
            match u64::from_str_radix(&digits, radix) {
                Ok(n) => self.push(TokenKind::Int(n), span),
                Err(_) => self.error(format!("invalid {name} number"), span),
            }
            return;
        }

        let mut text = self.digits(|c| c.is_ascii_digit());
        let mut is_float = false;

        // A fraction needs a digit after the dot, so `x.0` is a float
        // but `list.len` after a number is not swallowed.
        if self.peek() == Some('.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            self.advance();
            text.push('.');
            text += &self.digits(|c| c.is_ascii_digit());
        }

        // An exponent: e or E, an optional sign, then at least one digit.
        if matches!(self.peek(), Some('e' | 'E')) {
            let sign = matches!(self.peek_at(1), Some('+' | '-'));
            let first_digit = self.peek_at(if sign { 2 } else { 1 });
            if first_digit.is_some_and(|c| c.is_ascii_digit()) {
                is_float = true;
                text.push('e');
                self.advance();
                if sign {
                    text.push(self.advance().unwrap());
                }
                text += &self.digits(|c| c.is_ascii_digit());
            }
        }

        if is_float {
            match text.parse::<f64>() {
                Ok(x) => self.push(TokenKind::Float(x), span),
                Err(_) => self.error("invalid number", span),
            }
        } else {
            match text.parse::<u64>() {
                Ok(n) => self.push(TokenKind::Int(n), span),
                Err(_) => self.error("number is too large", span),
            }
        }
    }

    /// Consume characters while `accept` says yes, dropping `_` separators
    /// so that `1_000_000` reads as 1000000.
    fn digits(&mut self, accept: impl Fn(char) -> bool) -> String {
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if c == '_' {
                self.advance();
            } else if accept(c) {
                text.push(c);
                self.advance();
            } else {
                break;
            }
        }
        text
    }

    /// A string in single or double quotes. It must close on the same line.
    fn string(&mut self, formatted: bool, span: Span) {
        let quote = self.advance().unwrap();
        let mut value = String::new();

        loop {
            let here = Span { line: self.line, col: self.col };
            match self.peek() {
                None | Some('\n') => {
                    self.error("string is missing its closing quote", span);
                    return;
                }
                Some(c) if c == quote => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    self.advance();
                    match self.advance() {
                        Some('n') => value.push('\n'),
                        Some('t') => value.push('\t'),
                        Some('r') => value.push('\r'),
                        Some('0') => value.push('\0'),
                        Some(c @ ('\\' | '"' | '\'')) => value.push(c),
                        Some(c) => self.error(format!("unknown escape `\\{c}`"), here),
                        None => {} // reported as a missing quote on the next turn
                    }
                }
                Some(c) => {
                    value.push(c);
                    self.advance();
                }
            }
        }

        self.push(TokenKind::Str { value, formatted }, span);
    }

    /// Punctuation and operators. Two-character ones (`==`, `->`, `**`, ...)
    /// are found by checking the character after the first.
    fn symbol(&mut self, span: Span) {
        use TokenKind::*;
        let c = self.advance().unwrap();

        let kind = match c {
            '(' => {
                self.depth += 1;
                LParen
            }
            ')' => {
                self.depth = self.depth.saturating_sub(1);
                RParen
            }
            '[' => {
                self.depth += 1;
                LBracket
            }
            ']' => {
                self.depth = self.depth.saturating_sub(1);
                RBracket
            }
            '{' => LBrace,
            '}' => RBrace,
            ',' => Comma,
            '.' => Dot,
            ':' => Colon,
            '%' => Percent,
            '+' => if self.eat('=') { PlusAssign } else { Plus },
            '-' => {
                if self.eat('>') {
                    Arrow
                } else if self.eat('=') {
                    MinusAssign
                } else {
                    Minus
                }
            }
            '*' => {
                if self.eat('*') {
                    StarStar
                } else if self.eat('=') {
                    StarAssign
                } else {
                    Star
                }
            }
            '/' => {
                if self.eat('/') {
                    SlashSlash
                } else if self.eat('=') {
                    SlashAssign
                } else {
                    Slash
                }
            }
            '=' => if self.eat('=') { Eq } else { Assign },
            '<' => if self.eat('=') { LtEq } else { Lt },
            '>' => if self.eat('=') { GtEq } else { Gt },
            '!' => {
                if self.eat('=') {
                    NotEq
                } else {
                    return self.error("unexpected `!`. Zeno spells this `not`", span);
                }
            }

            // Habits from other languages get a hint instead of a bare error.
            '&' => {
                self.eat('&');
                return self.error("Zeno spells this `and`", span);
            }
            '|' => {
                self.eat('|');
                return self.error("Zeno spells this `or`", span);
            }
            ';' => {
                return self.error("Zeno has no semicolons. A line break ends a statement", span);
            }
            other => return self.error(format!("unexpected character `{other}`"), span),
        };

        self.push(kind, span);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind::*;

    /// Lex `source` and return just the kinds, without the trailing Eof.
    fn kinds(source: &str) -> Vec<TokenKind> {
        let mut kinds: Vec<_> = lex(source).unwrap().into_iter().map(|t| t.kind).collect();
        assert_eq!(kinds.pop(), Some(Eof));
        kinds
    }

    fn ident(name: &str) -> TokenKind {
        Ident(name.to_string())
    }

    fn string(value: &str) -> TokenKind {
        Str { value: value.to_string(), formatted: false }
    }

    #[test]
    fn empty_source_is_just_eof() {
        assert_eq!(kinds(""), vec![]);
        assert_eq!(kinds("\n\n   \n# only a comment\n"), vec![]);
    }

    #[test]
    fn let_statement() {
        assert_eq!(
            kinds("let count = 3"),
            vec![Let, ident("count"), Assign, Int(3), Newline]
        );
    }

    #[test]
    fn keywords_and_identifiers() {
        assert_eq!(
            kinds("function if_ready not nothing none"),
            vec![Function, ident("if_ready"), Not, ident("nothing"), None, Newline]
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(
            kinds("42 1_000 3.14 1e3 2.5e-3 0xFF 0b1010 0o17"),
            vec![
                Int(42), Int(1000), Float(3.14), Float(1000.0), Float(0.0025),
                Int(255), Int(10), Int(15), Newline
            ]
        );
    }

    #[test]
    fn number_followed_by_a_unit_or_method() {
        assert_eq!(kinds("500 ms"), vec![Int(500), ident("ms"), Newline]);
        assert_eq!(kinds("500ms"), vec![Int(500), ident("ms"), Newline]);
        assert_eq!(kinds("5.abs"), vec![Int(5), Dot, ident("abs"), Newline]);
    }

    #[test]
    fn strings() {
        assert_eq!(kinds(r#""hi" 'there'"#), vec![string("hi"), string("there"), Newline]);
        assert_eq!(kinds(r#""a\tb\n\"q\"""#), vec![string("a\tb\n\"q\""), Newline]);
        assert_eq!(kinds(r#""it's""#), vec![string("it's"), Newline]);
    }

    #[test]
    fn formatted_strings() {
        assert_eq!(
            kinds(r#"f"Hello, {who}!""#),
            vec![Str { value: "Hello, {who}!".to_string(), formatted: true }, Newline]
        );
        // A name that merely starts with f is still a name.
        assert_eq!(kinds("f foo"), vec![ident("f"), ident("foo"), Newline]);
    }

    #[test]
    fn operators() {
        assert_eq!(
            kinds("a + b - c * d / e % f ** g // h"),
            vec![
                ident("a"), Plus, ident("b"), Minus, ident("c"), Star, ident("d"),
                Slash, ident("e"), Percent, ident("f"), StarStar, ident("g"),
                SlashSlash, ident("h"), Newline
            ]
        );
        assert_eq!(
            kinds("== != < <= > >= = += -= *= /= ->  x"),
            vec![
                Eq, NotEq, Lt, LtEq, Gt, GtEq, Assign, PlusAssign, MinusAssign,
                StarAssign, SlashAssign, Arrow, ident("x"), Newline
            ]
        );
    }

    #[test]
    fn function_signature() {
        assert_eq!(
            kinds("function area(shape: Shape) -> float {"),
            vec![
                Function, ident("area"), LParen, ident("shape"), Colon, ident("Shape"),
                RParen, Arrow, ident("float"), LBrace, Newline
            ]
        );
    }

    #[test]
    fn a_line_break_ends_a_statement() {
        assert_eq!(
            kinds("let a = 1\nlet b = 2\n"),
            vec![Let, ident("a"), Assign, Int(1), Newline, Let, ident("b"), Assign, Int(2), Newline]
        );
    }

    #[test]
    fn blank_lines_and_comments_do_not_add_newlines() {
        assert_eq!(
            kinds("\n\na  # trailing comment\n\n# full line\n\nb"),
            vec![ident("a"), Newline, ident("b"), Newline]
        );
    }

    #[test]
    fn no_newline_inside_parentheses_or_brackets() {
        assert_eq!(
            kinds("f(\n  1,\n  2\n)"),
            vec![ident("f"), LParen, Int(1), Comma, Int(2), RParen, Newline]
        );
        assert_eq!(
            kinds("[\n1,\n2\n]"),
            vec![LBracket, Int(1), Comma, Int(2), RBracket, Newline]
        );
    }

    #[test]
    fn an_operator_at_the_end_of_a_line_continues_the_statement() {
        assert_eq!(
            kinds("let total = price +\n    tax"),
            vec![Let, ident("total"), Assign, ident("price"), Plus, ident("tax"), Newline]
        );
        assert_eq!(
            kinds("ready and\nwaiting"),
            vec![ident("ready"), And, ident("waiting"), Newline]
        );
    }

    #[test]
    fn positions_are_tracked() {
        let tokens = lex("let x\n  = 10").unwrap();
        let at: Vec<_> = tokens.iter().map(|t| (t.span.line, t.span.col)).collect();
        //             let     x       \n      =       10      \n(end)  eof
        assert_eq!(at, vec![(1, 1), (1, 5), (1, 6), (2, 3), (2, 5), (2, 7), (2, 7)]);
    }

    fn errors(source: &str) -> Vec<(String, usize, usize)> {
        lex(source)
            .unwrap_err()
            .into_iter()
            .map(|e| (e.message, e.span.line, e.span.col))
            .collect()
    }

    #[test]
    fn unterminated_string() {
        assert_eq!(
            errors("let s = \"oops\nlet t = 1"),
            vec![("string is missing its closing quote".to_string(), 1, 9)]
        );
    }

    #[test]
    fn hints_for_habits_from_other_languages() {
        let found = errors("let x = 1;\nif a && b || !c {");
        let messages: Vec<_> = found.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(
            messages,
            vec![
                "Zeno has no semicolons. A line break ends a statement",
                "Zeno spells this `and`",
                "Zeno spells this `or`",
                "unexpected `!`. Zeno spells this `not`",
            ]
        );
        assert_eq!((found[0].1, found[0].2), (1, 10));
        assert_eq!((found[1].1, found[1].2), (2, 6));
    }

    #[test]
    fn bad_numbers_and_characters() {
        assert_eq!(errors("0xZZ")[0].0, "invalid hexadecimal number");
        assert_eq!(errors("0b102")[0].0, "invalid binary number");
        assert_eq!(errors("99999999999999999999999")[0].0, "number is too large");
        assert_eq!(errors("a $ b"), vec![("unexpected character `$`".to_string(), 1, 3)]);
        assert_eq!(errors(r#""bad \q""#)[0].0, "unknown escape `\\q`");
    }

    #[test]
    fn all_reported_errors_not_just_the_first() {
        assert_eq!(errors("$\n@\n~").len(), 3);
    }

    #[test]
    fn the_example_programs_lex_cleanly() {
        for (name, source) in [
            ("hello", include_str!("../examples/hello.zeno")),
            ("tour", include_str!("../examples/tour.zeno")),
            ("blink", include_str!("../examples/blink.zeno")),
        ] {
            if let Err(errors) = lex(source) {
                panic!("examples/{name}.zeno: {errors:?}");
            }
        }
    }
}
