//! The parser: turns tokens into a syntax tree.
//!
//! This is a "recursive descent" parser. There is one function per piece of
//! grammar (`statement`, `if_statement`, `block`, `expression`, ...), and
//! they call each other the same way the grammar nests: a block contains
//! statements, an `if` statement contains a block, and so on.
//!
//! Each function looks at the next token to decide what it is parsing,
//! consumes the tokens that belong to it, and returns a tree node.
//!
//! The parser stops at the first error. `?` after a call means "if that
//! failed, stop and pass the error up".

use std::sync::Arc;

use crate::ast::*;
use crate::error::Error;
use crate::lexer;
use crate::token::{Span, Token, TokenKind};

/// Turn the tokens of a whole file into a syntax tree.
pub fn parse(tokens: Vec<Token>) -> Result<Program, Error> {
    let mut parser = Parser { tokens, pos: 0, depth: 0, private: Vec::new(), in_test: false, tests: Vec::new() };
    let mut statements = Vec::new();

    parser.skip_newlines();

    // A first line that says just `strict` puts the whole file in strict
    // mode. The word is reserved, and this is its only place.
    let strict = parser.check(TokenKind::Strict) && matches!(parser.peek_at(1), TokenKind::Newline | TokenKind::Eof);
    if strict {
        parser.advance();
        parser.skip_newlines();
    }

    while !parser.check(TokenKind::Eof) {
        statements.push(parser.statement()?);
        parser.end_of_statement()?;
    }
    Ok(Program { strict, statements, private: parser.private, modules: Vec::new() })
}

struct Parser {
    /// Always ends with `Eof`.
    tokens: Vec<Token>,
    /// Index of the next unread token.
    pos: usize,
    /// How many `{ }` blocks of statements are open around here. At the
    /// top of the file it is 0.
    depth: usize,
    /// The names declared with `private` at the top of the file.
    private: Vec<String>,
    /// Is this inside a `test`? Only there is `assert` written.
    in_test: bool,
    /// The names of the tests of this file so far.
    tests: Vec<String>,
}

impl Parser {
    // ------------------------------------------------------- Reading tokens

    /// The next token, without consuming it.
    fn peek(&self) -> &TokenKind {
        self.peek_at(0)
    }

    /// The token `offset` places ahead. Past the end, this is always `Eof`.
    fn peek_at(&self, offset: usize) -> &TokenKind {
        let index = (self.pos + offset).min(self.tokens.len() - 1);
        &self.tokens[index].kind
    }

    /// Where the next token is, for error messages.
    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    /// Consume the next token. `Eof` is never consumed.
    fn advance(&mut self) -> TokenKind {
        let kind = self.peek().clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        kind
    }

    fn check(&self, kind: TokenKind) -> bool {
        *self.peek() == kind
    }

    /// Consume the next token only if it is `kind`.
    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Consume `kind`, or fail with "expected X, found Y".
    fn expect(&mut self, kind: TokenKind) -> Result<(), Error> {
        if self.check(kind.clone()) {
            self.advance();
            Ok(())
        } else {
            self.unexpected(&kind.describe())
        }
    }

    /// Consume a name and return it.
    fn name(&mut self, what: &str) -> Result<String, Error> {
        match self.peek().clone() {
            TokenKind::Ident(name) => {
                self.advance();
                Ok(name)
            }
            _ => self.unexpected(what),
        }
    }

    /// A name that may say which module it is from: `Point`, or
    /// `shapes.Point`. This is for the places where a type, an enum case
    /// or an error is named. In an expression `shapes.Point` is read as
    /// any other `a.b`.
    fn name_in_module(&mut self, what: &str) -> Result<String, Error> {
        let mut name = self.name(what)?;
        while self.check(TokenKind::Dot) && matches!(self.peek_at(1), TokenKind::Ident(_)) {
            self.advance();
            name = format!("{name}.{}", self.name(what)?);
        }
        Ok(name)
    }

    fn skip_newlines(&mut self) {
        while self.eat(TokenKind::Newline) {}
    }

    /// Allow `else` and `except` to start on the line after a `}`.
    fn skip_newline_before(&mut self, kind: TokenKind) {
        if self.check(TokenKind::Newline) && *self.peek_at(1) == kind {
            self.advance();
        }
    }

    // --------------------------------------------------------------- Errors

    fn error<T>(&self, message: impl Into<String>) -> Result<T, Error> {
        Err(Error::new(message, self.span()))
    }

    fn unexpected<T>(&self, wanted: &str) -> Result<T, Error> {
        // `owned`, `take` and `strict` each have one place. Anywhere else
        // somebody most likely meant a name, so say where the word goes.
        let place = match self.peek() {
            TokenKind::Owned => " `owned` is a word of the language, so it cannot be a name. It goes in front of \
                                 `class`, or in front of a type: `owned list[int]`",
            TokenKind::Take => " `take` is a word of the language, so it cannot be a name. It goes in front of the \
                                type of a parameter: `frame: take Frame`",
            TokenKind::Strict => " `strict` is a word of the language, so it cannot be a name. It goes alone on the \
                                  first line of a file",
            TokenKind::Assert => " `assert` is a word of the language, so it cannot be a name. It goes in a test: \
                                  `assert total == 6`",
            _ => "",
        };
        let stop = if place.is_empty() { "" } else { "." };
        self.error(format!("expected {wanted}, found {}{stop}{place}", self.peek().describe()))
    }

    // ----------------------------------------------------------- Statements

    /// Every statement ends at a line break, or just before a `}`.
    fn end_of_statement(&mut self) -> Result<(), Error> {
        if self.eat(TokenKind::Newline) {
            self.skip_newlines();
            Ok(())
        } else if matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            Ok(())
        } else {
            self.unexpected("the end of the line")
        }
    }

    fn statement(&mut self) -> Result<Stmt, Error> {
        let span = self.span();

        // `elif` at the start of a line, by somebody who writes Python.
        // (After a `}` on the same line, `if_statement` says the same.)
        if matches!(self.peek(), TokenKind::Ident(word) if word == "elif") && *self.peek_at(1) != TokenKind::Assign {
            return self.error("Zeno spells `elif` as `else if`");
        }

        let kind = match self.peek() {
            TokenKind::Let | TokenKind::Var | TokenKind::Const => self.declaration()?,
            // `function name(...)` declares. `function(...)` with no name is a
            // value, so it is handled with the other expressions below.
            TokenKind::Function if *self.peek_at(1) != TokenKind::LParen => {
                StmtKind::Function(Arc::new(self.function()?))
            }
            // `private function helper()`: the declaration after the word
            // is for this file only. The word is reserved, and stands in
            // front of a declaration at the top of a file or of a field
            // or a function in a class (decision 10.1).
            TokenKind::Private => {
                self.advance(); // private
                let what = "`private` goes in front of a declaration: `private function`, `private class`, \
                            `private enum`, `private let`, `private var` or `private const`";
                if self.check(TokenKind::Private) {
                    return self.error(what);
                }
                let declared = self.statement()?;
                let name = match &declared.kind {
                    StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => name.clone(),
                    StmtKind::Function(function) => function.name.clone(),
                    StmtKind::Class(class) => class.name.clone(),
                    StmtKind::Enum(decl) => decl.name.clone(),
                    _ => return Err(Error::new(what, span)),
                };
                if self.depth > 0 {
                    return Err(Error::new(
                        format!(
                            "`private` is for a declaration at the top of a file, and for a field or a function \
                             of a class. `{name}` is declared inside a block, and is seen only there as it is. \
                             Take the word away"
                        ),
                        span,
                    ));
                }
                self.private.push(name);
                declared.kind
            }
            TokenKind::Class => self.class(false)?,
            TokenKind::Interface => self.interface()?,
            // `owned class`. The word is reserved (decision 5.19c): it
            // stands here and in front of a type, and nowhere else.
            TokenKind::Owned if *self.peek_at(1) == TokenKind::Class => {
                self.advance(); // owned
                self.class(true)?
            }
            // `unsafe { ... }`: the lines that Zeno cannot check. The word
            // is reserved, so it never means anything else (decision 5.18a).
            TokenKind::Unsafe => {
                self.advance(); // unsafe
                StmtKind::Unsafe(self.block()?)
            }
            // `test "a new ring is empty" { ... }`. `test` is not a
            // reserved word: it starts a test only here, at the start of
            // a statement with text after it, which could be nothing
            // else. Everywhere else it is a name (decision 10.16).
            TokenKind::Ident(word) if word == "test" && matches!(self.peek_at(1), TokenKind::Str { .. }) => self.test()?,
            TokenKind::Ident(word)
                if word == "test"
                    && (*self.peek_at(1) == TokenKind::LBrace
                        || (matches!(self.peek_at(1), TokenKind::Ident(_)) && *self.peek_at(2) == TokenKind::LBrace)) =>
            {
                return self.error(
                    "a test has a name, in quotes, that says what it checks: `test \"a new ring is empty\" { ... }`",
                );
            }
            TokenKind::Assert => self.assert()?,
            TokenKind::Enum => self.enum_()?,
            TokenKind::If => self.if_statement()?,
            TokenKind::While => self.while_loop()?,
            TokenKind::For => self.for_loop()?,
            TokenKind::Match => self.match_statement()?,
            TokenKind::Try => self.try_statement()?,
            TokenKind::From | TokenKind::Import => self.import()?,
            TokenKind::Return => {
                self.advance();
                // A bare `return` is followed straight away by the end of the statement.
                let at_end = matches!(
                    self.peek(),
                    TokenKind::Newline | TokenKind::RBrace | TokenKind::Eof
                );
                StmtKind::Return(if at_end { None } else { Some(self.expression()?) })
            }
            TokenKind::Raise => {
                self.advance();
                StmtKind::Raise(self.expression()?)
            }
            TokenKind::Break => {
                self.advance();
                StmtKind::Break
            }
            TokenKind::Continue => {
                self.advance();
                StmtKind::Continue
            }

            // A hint for habits from other languages: `def name` / `fn name`.
            TokenKind::Ident(word)
                if matches!(word.as_str(), "def" | "fn" | "func" | "fun")
                    && matches!(self.peek_at(1), TokenKind::Ident(_)) =>
            {
                return self.error(format!("Zeno spells `{word}` as `function`"));
            }

            _ => self.assignment_or_expression()?,
        };

        Ok(Stmt { kind, span })
    }

    /// `let name = value`, `var name: type = value`, `const NAME = value`
    fn declaration(&mut self) -> Result<StmtKind, Error> {
        let keyword = self.advance();
        let name = self.name("a name for the variable")?;
        let ty = if self.eat(TokenKind::Colon) { Some(self.type_()?) } else { None };
        self.expect(TokenKind::Assign)?;
        let value = self.expression()?;

        Ok(match keyword {
            TokenKind::Const => StmtKind::Const { name, ty, value },
            TokenKind::Var => StmtKind::Let { mutable: true, name, ty, value },
            _ => StmtKind::Let { mutable: false, name, ty, value },
        })
    }

    /// Either `target = value` (or `+=` and friends), or a bare expression.
    /// Both start with an expression, so parse that first and then look at
    /// what follows.
    fn assignment_or_expression(&mut self) -> Result<StmtKind, Error> {
        let target = self.expression()?;

        let op = match self.peek() {
            TokenKind::Assign => None,
            TokenKind::PlusAssign => Some(BinaryOp::Add),
            TokenKind::MinusAssign => Some(BinaryOp::Sub),
            TokenKind::StarAssign => Some(BinaryOp::Mul),
            TokenKind::SlashAssign => Some(BinaryOp::Div),
            TokenKind::SlashSlashAssign => Some(BinaryOp::FloorDiv),
            TokenKind::PercentAssign => Some(BinaryOp::Mod),
            TokenKind::StarStarAssign => Some(BinaryOp::Pow),
            TokenKind::AmpAssign => Some(BinaryOp::BitAnd),
            TokenKind::PipeAssign => Some(BinaryOp::BitOr),
            TokenKind::CaretAssign => Some(BinaryOp::BitXor),
            TokenKind::ShlAssign => Some(BinaryOp::Shl),
            TokenKind::ShrAssign => Some(BinaryOp::Shr),
            _ => return Ok(StmtKind::Expr(target)),
        };

        // Only a variable, a field or an indexed item can be assigned to.
        let assignable = matches!(
            target.kind,
            ExprKind::Name(_) | ExprKind::Field { .. } | ExprKind::Index { .. }
        );
        if !assignable {
            return Err(Error::new("this cannot be assigned to", target.span));
        }

        self.advance(); // the = or += ...
        let value = self.expression()?;
        Ok(StmtKind::Assign { target, op, value })
    }

    /// `{ statements }`
    fn block(&mut self) -> Result<Block, Error> {
        let open = self.span();
        if self.check(TokenKind::Colon) {
            return self.error("expected `{`, found `:`. Zeno uses `{ }` around blocks");
        }
        self.expect(TokenKind::LBrace)?;
        self.skip_newlines();

        self.depth += 1;
        let mut statements = Vec::new();
        while !self.check(TokenKind::RBrace) {
            if self.check(TokenKind::Eof) {
                return Err(Error::new("this `{` is never closed", open));
            }
            statements.push(self.statement()?);
            self.end_of_statement()?;
        }
        self.depth -= 1;
        self.advance(); // the }
        Ok(statements)
    }

    /// `if condition { } else if condition { } else { }`
    fn if_statement(&mut self) -> Result<StmtKind, Error> {
        self.advance(); // if
        let condition = self.expression()?;
        let then_block = self.block()?;

        if matches!(self.peek(), TokenKind::Ident(word) if word == "elif") {
            return self.error("Zeno spells `elif` as `else if`");
        }

        self.skip_newline_before(TokenKind::Else);
        let else_block = if self.eat(TokenKind::Else) {
            if self.check(TokenKind::If) {
                // `else if`: an else block holding just the next `if`.
                let span = self.span();
                let kind = self.if_statement()?;
                Some(vec![Stmt { kind, span }])
            } else {
                Some(self.block()?)
            }
        } else {
            None
        };

        Ok(StmtKind::If { condition, then_block, else_block })
    }

    fn while_loop(&mut self) -> Result<StmtKind, Error> {
        self.advance(); // while
        let condition = self.expression()?;
        let body = self.block()?;
        Ok(StmtKind::While { condition, body })
    }

    /// `for variable in iterable { }`
    fn for_loop(&mut self) -> Result<StmtKind, Error> {
        self.advance(); // for
        let variable = self.name("a name for the loop variable")?;
        // `for key, value in ages`: the key and what is under it.
        let mut second = None;
        if self.eat(TokenKind::Comma) {
            let span = self.span();
            let name = self.name("a name for the value")?;
            if name == variable {
                return Err(Error::new(
                    format!("the two names of a loop are two variables, and both are called `{name}`. Give them different names"),
                    span,
                ));
            }
            if self.check(TokenKind::Comma) {
                return self.error(
                    "a loop has one name, or two: `for key, value in ages` is given each key and what is under it",
                );
            }
            second = Some(name);
        }
        self.expect(TokenKind::In)?;
        let iterable = self.expression()?;
        let body = self.block()?;
        Ok(StmtKind::For { variable, second, iterable, body })
    }

    /// `match subject { case pattern { } ... }`
    fn match_statement(&mut self) -> Result<StmtKind, Error> {
        self.advance(); // match
        let subject = self.expression()?;
        self.expect(TokenKind::LBrace)?;
        self.skip_newlines();

        let mut arms = Vec::new();
        while !self.check(TokenKind::RBrace) {
            self.expect(TokenKind::Case)?;
            let pattern = self.pattern()?;
            let body = self.block()?;
            arms.push(Arm { pattern, body });
            self.end_of_statement()?;
        }
        self.advance(); // the }
        Ok(StmtKind::Match { subject, arms })
    }

    /// What comes after `case`.
    fn pattern(&mut self) -> Result<Pattern, Error> {
        let span = self.span();
        let literal = |kind| Ok(Pattern::Literal(Expr { kind, span }));

        match self.advance() {
            TokenKind::Ident(name) if name == "_" => Ok(Pattern::Wildcard),
            TokenKind::Ident(mut name) => {
                // `case shapes.Circle(r)`: a case of an imported file.
                while self.check(TokenKind::Dot) && matches!(self.peek_at(1), TokenKind::Ident(_)) {
                    self.advance();
                    name = format!("{name}.{}", self.name("a name")?);
                }
                if !self.eat(TokenKind::LParen) {
                    return Ok(Pattern::Name(name));
                }
                // `Circle(r)`: a pattern for each value the case carries.
                let mut fields = Vec::new();
                while !self.check(TokenKind::RParen) {
                    fields.push(self.pattern()?);
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                Ok(Pattern::Variant { name, fields })
            }
            TokenKind::Int(n) => literal(ExprKind::Int(n)),
            TokenKind::Float(x) => literal(ExprKind::Float(x)),
            TokenKind::Str { value, .. } => literal(ExprKind::Str(value)),
            TokenKind::True => literal(ExprKind::Bool(true)),
            TokenKind::False => literal(ExprKind::Bool(false)),
            TokenKind::None => literal(ExprKind::None),
            other => Err(Error::new(
                format!("expected a pattern after `case`, found {}", other.describe()),
                span,
            )),
        }
    }

    /// `try { } except Error as name { }`
    fn try_statement(&mut self) -> Result<StmtKind, Error> {
        let span = self.span();
        self.advance(); // try
        let body = self.block()?;

        let mut handlers = Vec::new();
        loop {
            self.skip_newline_before(TokenKind::Except);
            if !self.eat(TokenKind::Except) {
                break;
            }
            let error = match self.peek() {
                TokenKind::Ident(_) => Some(self.name_in_module("an error type")?),
                _ => None,
            };
            let binding = if self.eat(TokenKind::As) {
                Some(self.name("a name for the error")?)
            } else {
                None
            };
            let body = self.block()?;
            handlers.push(Handler { error, binding, body });
        }

        if handlers.is_empty() {
            return Err(Error::new("`try` needs at least one `except`", span));
        }
        Ok(StmtKind::Try { body, handlers })
    }

    /// `test "a new ring is empty" { ... }`
    fn test(&mut self) -> Result<StmtKind, Error> {
        let span = self.span();
        self.advance(); // test
        let named = self.span();
        let name = match self.advance() {
            TokenKind::Str { value, formatted: false } => value,
            _ => {
                return Err(Error::new(
                    "the name of a test is plain text, with nothing worked out in it. Take the `f` away",
                    named,
                ))
            }
        };
        if self.depth > 0 {
            return Err(Error::new(
                "a test stands at the top of a file, and this one is inside a block. Move it out of the `{ }`",
                span,
            ));
        }
        if name.trim().is_empty() {
            return Err(Error::new("a test needs a name that says what it checks", named));
        }
        if self.tests.contains(&name) {
            return Err(Error::new(
                format!(
                    "two tests of this file are called \"{name}\". Give each its own name, so that a failure \
                     says which one it is"
                ),
                named,
            ));
        }
        self.tests.push(name.clone());
        self.in_test = true;
        let body = self.block();
        self.in_test = false;
        Ok(StmtKind::Test { name, body: body? })
    }

    /// `assert total == 6`, or `assert raises ValueError { ... }`
    fn assert(&mut self) -> Result<StmtKind, Error> {
        let span = self.span();
        self.advance(); // assert
        if !self.in_test {
            return Err(Error::new(
                "`assert` belongs in a test: `test \"name\" { assert total == 6 }`. In the program itself, \
                 check with `if`, and `raise` an error",
                span,
            ));
        }
        if self.eat(TokenKind::Raises) {
            let error = self.name_in_module("the kind of error the block must raise")?;
            let body = self.block()?;
            return Ok(StmtKind::AssertRaises { error, body });
        }
        if matches!(self.peek(), TokenKind::Newline | TokenKind::RBrace | TokenKind::Eof) {
            return self.error("`assert` needs what must be so after it: `assert total == 6`");
        }
        Ok(StmtKind::Assert(self.expression()?))
    }

    /// `import a.b` or `from a.b import x, y`
    fn import(&mut self) -> Result<StmtKind, Error> {
        let from = self.advance() == TokenKind::From;

        let mut module = vec![self.name("a module name")?];
        while self.eat(TokenKind::Dot) {
            module.push(self.name("a module name")?);
        }

        let mut names = Vec::new();
        if from {
            self.expect(TokenKind::Import)?;
            names.push(self.name("a name to import")?);
            while self.eat(TokenKind::Comma) {
                names.push(self.name("a name to import")?);
            }
        }
        // `import drivers.uart as uart`: a module has one name, and a
        // name it brings in has the name its file gave it (7.3).
        if *self.peek() == TokenKind::As {
            let (full, last) = (module.join("."), module.last().cloned().unwrap_or_default());
            let message = match names.first() {
                None => format!(
                    "a module has one name, so there is no `as` after an `import`. `{full}` is used as `{full}.name`. \
                     For a short name, import what is used: `from {full} import ...`, and then `{last}` is not written at all"
                ),
                Some(name) => {
                    format!("`{name}` keeps the name `{full}` gave it, so there is no `as` after an `import`. Use it as `{name}`")
                }
            };
            return Err(Error::new(message, self.span()));
        }
        Ok(StmtKind::Import { module, names })
    }

    // ---------------------------------------- Functions, classes and enums

    /// `function name(params) -> type raises Error { }`
    fn function(&mut self) -> Result<Function, Error> {
        let span = self.span();
        self.advance(); // function
        let name = self.name("a name for the function")?;
        let type_params = self.type_params()?;
        Ok(Function { span, ..self.function_after_name(name, type_params, false)? })
    }

    /// `[T]` or `[Key, Value]` after the name of a function or a class:
    /// the types that each use of it fills in. Usually there are none.
    fn type_params(&mut self) -> Result<Vec<String>, Error> {
        let mut names = Vec::new();
        if self.eat(TokenKind::LBracket) {
            loop {
                names.push(self.name("a name for a type, such as `T`")?);
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
            self.expect(TokenKind::RBracket)?;
        }
        Ok(names)
    }

    /// Everything from the `(` on, shared by named and nameless functions.
    fn function_after_name(
        &mut self,
        name: String,
        type_params: Vec<String>,
        in_interface: bool,
    ) -> Result<Function, Error> {
        if name.is_empty() && self.check(TokenKind::LBracket) {
            // `function[T](...)`
            return self.error("a function with no name cannot have type parameters. Give it a name");
        }

        let span = self.span();
        self.expect(TokenKind::LParen)?;
        let mut params: Vec<Param> = Vec::new();
        while !self.check(TokenKind::RParen) {
            let at = self.span();
            let name = self.name("a parameter name")?;
            let ty = if self.eat(TokenKind::Colon) { Some(self.type_()?) } else { None };
            let default = if self.eat(TokenKind::Assign) { Some(self.expression()?) } else { None };
            // Arguments are given in order, so a parameter that can be
            // left out has to be after every one that cannot.
            if let (None, Some(before)) = (&default, params.iter().find(|param| param.default.is_some())) {
                return Err(Error::new(
                    format!(
                        "`{name}` has no default, and comes after `{0}`, which has one. A call gives its arguments \
                         in order, so `{0}` could only be left out by naming every argument after it. \
                         Put the parameters that have a default last",
                        before.name
                    ),
                    at,
                ));
            }
            params.push(Param { name, ty, default });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RParen)?;

        let return_type = if self.eat(TokenKind::Arrow) { Some(self.type_()?) } else { None };

        let mut raises = Vec::new();
        if self.eat(TokenKind::Raises) {
            raises.push(self.name_in_module("an error type")?);
            while self.eat(TokenKind::Comma) {
                raises.push(self.name_in_module("an error type")?);
            }
        }

        // A function of an interface ends here: it has no body.
        let body = if in_interface { Vec::new() } else { self.block()? };
        Ok(Function { span, name, type_params, params, return_type, raises, body })
    }

    /// `interface Name { function ... }`: the functions a class must have
    /// to be used as a `Name`, one to a line and each without a body.
    fn interface(&mut self) -> Result<StmtKind, Error> {
        self.advance(); // interface
        let name = self.name("a name for the interface")?;
        if self.check(TokenKind::LBracket) {
            return self.error("an interface cannot have type parameters yet. A class can: `class Box[T]`");
        }
        if self.check(TokenKind::Is) {
            return self.error(
                "an interface cannot be another interface yet. Name both after `is` on the class: `class Circle is Shape, Named`",
            );
        }
        self.expect(TokenKind::LBrace)?;
        self.skip_newlines();

        let mut methods = Vec::new();
        while !self.check(TokenKind::RBrace) {
            let span = self.span();
            if self.check(TokenKind::Private) {
                return self.error(
                    "a function of an interface cannot be private: an interface lists what other code may ask of a class",
                );
            }
            if matches!(self.peek(), TokenKind::Ident(_)) {
                return self.error(
                    "an interface has functions and no fields. How a class keeps its data is the class's own business. \
                     If other code needs the value, give the interface a function that returns it",
                );
            }
            if !self.check(TokenKind::Function) {
                return self.unexpected("a function");
            }
            self.advance(); // function
            let function_name = self.name("a name for the function")?;
            if self.check(TokenKind::LBracket) {
                return self.error("a function of an interface cannot have type parameters yet");
            }
            let function = self.function_after_name(function_name, Vec::new(), true)?;
            if self.check(TokenKind::LBrace) {
                return self.error(
                    "a function of an interface has no body. It says what a class must have, and each class writes its own. \
                     Take the `{ }` away",
                );
            }
            // Everything a caller needs to know is written down here,
            // because there is no body to work it out from.
            let needs = |what: String| Err(Error::new(what, span));
            match function.params.first() {
                Some(first) if first.name == "self" && first.ty.is_none() => {}
                _ => {
                    return needs(format!(
                        "`{}` needs `self` as its first parameter: it is called on an object, as in `shape.{}()`",
                        function.name, function.name
                    ))
                }
            }
            for param in &function.params[1..] {
                if param.ty.is_none() {
                    return needs(format!(
                        "the parameter `{}` of `{}` needs a type, as in `{}: int`. A function of an interface has no body \
                         to work the type out from",
                        param.name, function.name, param.name
                    ));
                }
                if param.default.is_some() {
                    return needs(format!(
                        "the parameter `{}` of `{}` cannot have a default in an interface yet",
                        param.name, function.name
                    ));
                }
            }
            methods.push(Arc::new(function));
            self.end_of_statement()?;
        }
        self.advance(); // the }
        Ok(StmtKind::Class(Class {
            name,
            type_params: Vec::new(),
            owned: false,
            fields: Vec::new(),
            methods,
            private: Vec::new(),
            is: Vec::new(),
            interface: true,
        }))
    }

    /// `class Name { fields and methods }`, or `owned class Name { ... }`
    fn class(&mut self, owned: bool) -> Result<StmtKind, Error> {
        self.advance(); // class
        let name = self.name("a name for the class")?;
        let type_params = self.type_params()?;
        // `class Circle is Shape, Named`: the interfaces it is.
        let mut is = Vec::new();
        if self.eat(TokenKind::Is) {
            is.push(self.name_in_module("the name of an interface")?);
            while self.eat(TokenKind::Comma) {
                let span = self.span();
                let interface = self.name_in_module("the name of an interface")?;
                if is.contains(&interface) {
                    return Err(Error::new(format!("`{interface}` is written twice after `is`. Once is enough"), span));
                }
                is.push(interface);
            }
        }
        self.expect(TokenKind::LBrace)?;
        self.skip_newlines();

        let mut fields = Vec::new();
        let mut methods = Vec::new();
        let mut private = Vec::new();
        while !self.check(TokenKind::RBrace) {
            // `private items: list[int]`, `private function grow(self)`.
            let hidden = self.eat(TokenKind::Private);
            if self.check(TokenKind::Function) {
                methods.push(Arc::new(self.function()?));
                if hidden {
                    private.push(methods[methods.len() - 1].name.clone());
                }
            } else if matches!(self.peek(), TokenKind::Ident(_)) {
                fields.push(self.field()?);
                if hidden {
                    private.push(fields[fields.len() - 1].name.clone());
                }
            } else {
                return self.unexpected("a field or a function");
            }
            self.end_of_statement()?;
        }
        self.advance(); // the }
        Ok(StmtKind::Class(Class { name, type_params, owned, fields, methods, private, is, interface: false }))
    }

    /// `name`, optionally with `: type` and `= default`
    fn field(&mut self) -> Result<Field, Error> {
        let name = self.name("a field name")?;
        let ty = if self.eat(TokenKind::Colon) { Some(self.type_()?) } else { None };
        let default = if self.eat(TokenKind::Assign) { Some(self.expression()?) } else { None };
        Ok(Field { name, ty, default })
    }

    /// `enum Name { Case, Case(field: type) ... }`, one case per line
    fn enum_(&mut self) -> Result<StmtKind, Error> {
        self.advance(); // enum
        let name = self.name("a name for the enum")?;
        if self.check(TokenKind::LBracket) {
            return self.error("an enum cannot have type parameters yet. A class can: `class Box[T]`");
        }
        self.expect(TokenKind::LBrace)?;
        self.skip_newlines();

        let mut variants = Vec::new();
        while !self.check(TokenKind::RBrace) {
            if self.check(TokenKind::Private) {
                return self.error(
                    "a case of an enum cannot be private on its own: a `match` on the enum has to name every case. \
                     Mark the whole enum instead: `private enum`",
                );
            }
            let name = self.name("a name for the case")?;
            let mut fields = Vec::new();
            if self.eat(TokenKind::LParen) {
                while !self.check(TokenKind::RParen) {
                    fields.push(self.field()?);
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
            }
            variants.push(Variant { name, fields });
            self.end_of_statement()?;
        }
        self.advance(); // the }
        Ok(StmtKind::Enum(Enum { name, variants }))
    }

    // ---------------------------------------------------------------- Types

    /// `int`, `list[int]`, `str or none`, `function(int) -> int`
    fn type_(&mut self) -> Result<Type, Error> {
        // `owned list[int]`, `take Packet`: the word, then the type it is
        // about. Both words are reserved, so neither is ever a name
        // (decision 5.19c).
        if self.eat(TokenKind::Owned) {
            return Ok(Type::Owned(Box::new(self.type_()?)));
        }
        if self.eat(TokenKind::Take) {
            return Ok(Type::Take(Box::new(self.type_()?)));
        }
        let ty = if self.eat(TokenKind::Function) {
            // The type of a function: the types it takes, then what it returns.
            self.expect(TokenKind::LParen)?;
            let mut params = Vec::new();
            while !self.check(TokenKind::RParen) {
                params.push(self.type_()?);
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
            self.expect(TokenKind::RParen)?;
            let returns = if self.eat(TokenKind::Arrow) { Some(Box::new(self.type_()?)) } else { None };
            Type::Function { params, returns }
        } else {
            let name = self.name_in_module("a type")?;
            // `borrow` was the word for a parameter that only looks.
            // Every parameter only looks now, so the word is gone.
            if name == "borrow" && matches!(self.peek(), TokenKind::Ident(_) | TokenKind::Function) {
                return self.error(
                    "`borrow` is no longer a word. A parameter looks at what it is given without one: \
                     write the type alone. A parameter that takes the value over says `take`",
                );
            }
            let mut args = Vec::new();
            if self.eat(TokenKind::LBracket) {
                loop {
                    args.push(self.type_()?);
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(TokenKind::RBracket)?;
            }
            Type::Named { name, args }
        };

        if self.eat(TokenKind::Or) {
            if !self.eat(TokenKind::None) {
                return self.error("only `or none` can follow a type for now");
            }
            return Ok(Type::Optional(Box::new(ty)));
        }
        Ok(ty)
    }

    // ---------------------------------------------------------- Expressions
    //
    // Operators that bind loosely are parsed first and those that bind
    // tightly last. Each function below handles one level and calls the next
    // level down for its operands, which is all it takes to get precedence
    // right:
    //
    //   or
    //   and
    //   not
    //   == != < <= > >= is in
    //   + -
    //   * / // %
    //   - (negation)
    //   **
    //   calls, .field, [index]
    //   literals, names, ( ), [ ]

    fn expression(&mut self) -> Result<Expr, Error> {
        self.or()
    }

    fn binary(left: Expr, op: BinaryOp, right: Expr) -> Expr {
        let span = left.span;
        Expr { kind: ExprKind::Binary { op, left: Box::new(left), right: Box::new(right) }, span }
    }

    fn or(&mut self) -> Result<Expr, Error> {
        let mut left = self.and()?;
        while self.eat(TokenKind::Or) {
            let right = self.and()?;
            left = Self::binary(left, BinaryOp::Or, right);
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, Error> {
        let mut left = self.not()?;
        while self.eat(TokenKind::And) {
            let right = self.not()?;
            left = Self::binary(left, BinaryOp::And, right);
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<Expr, Error> {
        let span = self.span();
        if self.eat(TokenKind::Not) {
            let operand = Box::new(self.not()?);
            return Ok(Expr { kind: ExprKind::Unary { op: UnaryOp::Not, operand }, span });
        }
        self.comparison()
    }

    /// If the next tokens are a comparison operator, consume it.
    /// `is not` and `not in` are two tokens each.
    fn comparison_op(&mut self) -> Option<BinaryOp> {
        let op = match self.peek() {
            TokenKind::Eq => BinaryOp::Eq,
            TokenKind::NotEq => BinaryOp::NotEq,
            TokenKind::Lt => BinaryOp::Lt,
            TokenKind::LtEq => BinaryOp::LtEq,
            TokenKind::Gt => BinaryOp::Gt,
            TokenKind::GtEq => BinaryOp::GtEq,
            TokenKind::In => BinaryOp::In,
            TokenKind::Is if *self.peek_at(1) == TokenKind::Not => {
                self.advance();
                BinaryOp::IsNot
            }
            TokenKind::Is => BinaryOp::Is,
            TokenKind::Not if *self.peek_at(1) == TokenKind::In => {
                self.advance();
                BinaryOp::NotIn
            }
            _ => return None,
        };
        self.advance();
        Some(op)
    }

    /// One comparison is an ordinary binary operator. Two or more in a row,
    /// like `0 <= i < len`, become a single `Compare` chain.
    fn comparison(&mut self) -> Result<Expr, Error> {
        let first = self.bits()?;

        let mut rest = Vec::new();
        while let Some(op) = self.comparison_op() {
            let right = self.bits()?;
            // `is` tests for none and nothing else. See docs/decisions.md.
            let is = matches!(op, BinaryOp::Is | BinaryOp::IsNot);
            if is && right.kind != ExprKind::None {
                return Err(Error::new(
                    "`is` only tests for `none`. Use `==` to compare values",
                    right.span,
                ));
            }
            rest.push((op, right));
        }

        Ok(match rest.len() {
            0 => first,
            1 => {
                let (op, right) = rest.remove(0);
                Self::binary(first, op, right)
            }
            _ => {
                let span = first.span;
                Expr { kind: ExprKind::Compare { first: Box::new(first), rest }, span }
            }
        })
    }

    /// The operators on the bits of a whole number. They bind tighter
    /// than a comparison, so `flags & mask == 0` asks about `flags & mask`,
    /// and looser than arithmetic, so `1 << n + 1` shifts by `n + 1`.
    /// Among themselves, from loosest: `|`, then `^`, then `&`, then the
    /// two shifts. `level` counts up through those four.
    fn bits(&mut self) -> Result<Expr, Error> {
        self.bits_at(0)
    }

    fn bits_at(&mut self, level: usize) -> Result<Expr, Error> {
        if level == 4 {
            return self.sum();
        }
        let mut left = self.bits_at(level + 1)?;
        loop {
            let op = match (level, self.peek()) {
                (0, TokenKind::Pipe) => BinaryOp::BitOr,
                (1, TokenKind::Caret) => BinaryOp::BitXor,
                (2, TokenKind::Amp) => BinaryOp::BitAnd,
                (3, TokenKind::Shl) => BinaryOp::Shl,
                (3, TokenKind::Shr) => BinaryOp::Shr,
                _ => return Ok(left),
            };
            self.advance();
            let right = self.bits_at(level + 1)?;
            left = Self::binary(left, op, right);
        }
    }

    fn sum(&mut self) -> Result<Expr, Error> {
        let mut left = self.product()?;
        loop {
            let op = match self.peek() {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => return Ok(left),
            };
            self.advance();
            let right = self.product()?;
            left = Self::binary(left, op, right);
        }
    }

    fn product(&mut self) -> Result<Expr, Error> {
        let mut left = self.negation()?;
        loop {
            let op = match self.peek() {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::SlashSlash => BinaryOp::FloorDiv,
                TokenKind::Percent => BinaryOp::Mod,
                _ => return Ok(left),
            };
            self.advance();
            let right = self.negation()?;
            left = Self::binary(left, op, right);
        }
    }

    fn negation(&mut self) -> Result<Expr, Error> {
        let span = self.span();
        if self.eat(TokenKind::Minus) {
            let operand = Box::new(self.negation()?);
            return Ok(Expr { kind: ExprKind::Unary { op: UnaryOp::Neg, operand }, span });
        }
        if self.eat(TokenKind::Tilde) {
            let operand = Box::new(self.negation()?);
            return Ok(Expr { kind: ExprKind::Unary { op: UnaryOp::BitNot, operand }, span });
        }
        self.power()
    }

    /// `**` groups to the right: `2 ** 3 ** 2` is `2 ** (3 ** 2)`.
    /// It binds tighter than a minus on its left, so `-2 ** 2` is `-(2 ** 2)`.
    fn power(&mut self) -> Result<Expr, Error> {
        let base = self.postfix()?;
        if self.eat(TokenKind::StarStar) {
            let exponent = self.negation()?;
            return Ok(Self::binary(base, BinaryOp::Pow, exponent));
        }
        Ok(base)
    }

    /// Anything that follows a value directly: `f(x)`, `a.b`, `a[0]`, in any mix.
    fn postfix(&mut self) -> Result<Expr, Error> {
        let mut expr = self.primary()?;
        loop {
            let span = expr.span;
            let kind = match self.peek() {
                TokenKind::LParen => {
                    self.advance();
                    let args = self.arguments()?;
                    ExprKind::Call { callee: Box::new(expr), args }
                }
                TokenKind::Dot => {
                    self.advance();
                    let at = self.span();
                    let name = self.name("a name after `.`")?;
                    ExprKind::Field { object: Box::new(expr), name, at }
                }
                TokenKind::LBracket => {
                    self.advance();
                    // `xs[:3]` and `xs[1:3]` are how Python takes a part.
                    // Zeno has one way, and says what it is.
                    let part = "a part of a list or a str is taken with `.slice(from, to)`, as in `xs.slice(1, 3)`";
                    if self.check(TokenKind::Colon) {
                        return Err(Error::new(part, self.span()));
                    }
                    let index = Box::new(self.expression()?);
                    if self.check(TokenKind::Colon) {
                        return Err(Error::new(part, self.span()));
                    }
                    self.expect(TokenKind::RBracket)?;
                    ExprKind::Index { object: Box::new(expr), index }
                }
                _ => return Ok(expr),
            };
            expr = Expr { kind, span };
        }
    }

    /// The arguments of a call, after the `(`. Consumes the `)`.
    fn arguments(&mut self) -> Result<Vec<Arg>, Error> {
        let mut args = Vec::new();
        while !self.check(TokenKind::RParen) {
            // `name=value` is a named argument.
            let named = matches!(self.peek(), TokenKind::Ident(_))
                && *self.peek_at(1) == TokenKind::Assign;
            let name = if named {
                let name = self.name("an argument name")?;
                self.advance(); // the =
                Some(name)
            } else {
                None
            };
            let value = self.expression()?;
            args.push(Arg { name, value });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RParen)?;
        Ok(args)
    }

    /// The smallest expressions: a literal, a name, or something in brackets.
    fn primary(&mut self) -> Result<Expr, Error> {
        let span = self.span();
        let kind = match self.peek().clone() {
            TokenKind::Int(n) => {
                self.advance();
                return Ok(self.maybe_unit(Expr { kind: ExprKind::Int(n), span }));
            }
            TokenKind::Float(x) => {
                self.advance();
                return Ok(self.maybe_unit(Expr { kind: ExprKind::Float(x), span }));
            }
            TokenKind::Str { value, formatted: false } => {
                self.advance();
                ExprKind::Str(value)
            }
            TokenKind::Str { value, formatted: true } => {
                self.advance();
                ExprKind::FormatStr(format_parts(&value, span)?)
            }
            TokenKind::True => {
                self.advance();
                ExprKind::Bool(true)
            }
            TokenKind::False => {
                self.advance();
                ExprKind::Bool(false)
            }
            TokenKind::None => {
                self.advance();
                ExprKind::None
            }
            // `let x = unsafe { p[0] }`: the block is a statement.
            TokenKind::Unsafe => {
                return self.error(
                    "`unsafe { }` is a statement and gives no value. Put the whole line inside the block: \
                     `unsafe { x = p[0] }`",
                );
            }
            TokenKind::Ident(name) => {
                self.advance();
                ExprKind::Name(name)
            }
            TokenKind::Function => {
                self.advance();
                ExprKind::Function(Arc::new(self.function_after_name(String::new(), Vec::new(), false)?))
            }
            TokenKind::LParen => {
                // Brackets only group. They leave no node of their own in the tree.
                self.advance();
                let inner = self.expression()?;
                self.expect(TokenKind::RParen)?;
                return Ok(inner);
            }
            TokenKind::LBracket => {
                self.advance();
                self.list()?
            }
            _ => return self.unexpected("an expression"),
        };
        Ok(Expr { kind, span })
    }

    /// A number directly followed by a name is a quantity: `500 ms`.
    fn maybe_unit(&mut self, number: Expr) -> Expr {
        if let TokenKind::Ident(unit) = self.peek().clone() {
            self.advance();
            let span = number.span;
            return Expr { kind: ExprKind::Unit { value: Box::new(number), unit }, span };
        }
        number
    }

    /// After a `[`: a list `[a, b, c]`, a dict `[key: value, ...]`, or a
    /// comprehension `[element for x in items if condition]`. They start
    /// the same way, so parse the first element and then see what
    /// follows it: a `:` makes it a dict, and `for` a comprehension.
    fn list(&mut self) -> Result<ExprKind, Error> {
        if self.eat(TokenKind::RBracket) {
            return Ok(ExprKind::List(Vec::new()));
        }
        // `[:]` is the dict with nothing in it. `[]` is the empty list,
        // so each says which it is without a type beside it.
        if self.eat(TokenKind::Colon) {
            if self.eat(TokenKind::RBracket) {
                return Ok(ExprKind::Dict(Vec::new()));
            }
            return self.error("an entry of a dict is written `key: value`, and a dict with nothing in it is `[:]`");
        }
        let first = self.expression()?;

        if self.eat(TokenKind::Colon) {
            let mut entries = vec![(first, self.expression()?)];
            while self.eat(TokenKind::Comma) {
                if self.check(TokenKind::RBracket) {
                    break; // a trailing comma
                }
                let key = self.expression()?;
                if !self.eat(TokenKind::Colon) {
                    return self.error("every entry of a dict is written `key: value`, and this one has no `:`");
                }
                entries.push((key, self.expression()?));
            }
            self.expect(TokenKind::RBracket)?;
            return Ok(ExprKind::Dict(entries));
        }

        if self.eat(TokenKind::For) {
            let variable = self.name("a name for the loop variable")?;
            self.expect(TokenKind::In)?;
            let iterable = Box::new(self.expression()?);
            let condition = if self.eat(TokenKind::If) {
                Some(Box::new(self.expression()?))
            } else {
                None
            };
            self.expect(TokenKind::RBracket)?;
            return Ok(ExprKind::ListComp { element: Box::new(first), variable, iterable, condition });
        }

        let mut items = vec![first];
        while self.eat(TokenKind::Comma) {
            if self.check(TokenKind::RBracket) {
                break; // a trailing comma
            }
            items.push(self.expression()?);
            if self.check(TokenKind::Colon) {
                return self.error("every entry of a dict is written `key: value`, and the ones before this are not");
            }
        }
        self.expect(TokenKind::RBracket)?;
        Ok(ExprKind::List(items))
    }
}

// ------------------------------------------------------------------ f-strings

/// Split the text of an f-string into plain text and `{...}` parts.
///
/// The lexer hands over `Hello, {who}!` as one piece of text. Here each
/// `{...}` is cut out and run through the lexer and parser on its own, as a
/// tiny program holding one expression. `{{` and `}}` are literal braces.
fn format_parts(text: &str, span: Span) -> Result<Vec<FormatPart>, Error> {
    let fail = |message: &str| Err(Error::new(format!("in f-string: {message}"), span));

    let chars: Vec<char> = text.chars().collect();
    let mut parts = Vec::new();
    let mut plain = String::new();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        let doubled = chars.get(i + 1) == Some(&c);

        if (c == '{' || c == '}') && doubled {
            plain.push(c);
            i += 2;
        } else if c == '}' {
            return fail("`}` without a `{`. Write `}}` for a literal brace");
        } else if c == '{' {
            // Find the matching `}`, and the `:` that starts a format, if any.
            // Brackets and strings inside the expression are skipped over, so
            // that `{f(x)}` and `{names[':']}` are not cut short.
            let start = i + 1;
            let mut depth = 0;
            let mut colon = None;
            let mut end = None;
            let mut quote = None; // the quote character, while inside a string
            for (j, &c) in chars.iter().enumerate().skip(start) {
                if let Some(open) = quote {
                    if c == open {
                        quote = None;
                    }
                    continue;
                }
                match c {
                    '"' | '\'' => quote = Some(c),
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' => depth -= 1,
                    '}' if depth > 0 => depth -= 1,
                    '}' => {
                        end = Some(j);
                        break;
                    }
                    ':' if depth == 0 && colon.is_none() => colon = Some(j),
                    _ => {}
                }
            }
            let Some(end) = end else {
                return fail("`{` is never closed");
            };

            let source: String = chars[start..colon.unwrap_or(end)].iter().collect();
            let format = colon.map(|colon| chars[colon + 1..end].iter().collect());
            if source.trim().is_empty() {
                return fail("`{}` is empty. Put an expression inside");
            }

            if !plain.is_empty() {
                parts.push(FormatPart::Text(std::mem::take(&mut plain)));
            }
            parts.push(FormatPart::Value { value: embedded_expression(&source, span)?, format });
            i = end + 1;
        } else {
            plain.push(c);
            i += 1;
        }
    }

    if !plain.is_empty() {
        parts.push(FormatPart::Text(plain));
    }
    Ok(parts)
}

/// Lex and parse the expression inside one `{...}` of an f-string.
fn embedded_expression(source: &str, span: Span) -> Result<Expr, Error> {
    let in_string = |e: Error| Error::new(format!("in f-string: {}", e.message), span);

    let mut tokens = lexer::lex(source).map_err(|mut errors| in_string(errors.remove(0)))?;
    // Errors should point at the string, not at a position inside `source`.
    for token in &mut tokens {
        token.span = span;
    }

    let mut parser = Parser { tokens, pos: 0, depth: 0, private: Vec::new(), in_test: false, tests: Vec::new() };
    let value = parser.expression().map_err(in_string)?;
    parser.skip_newlines();
    if !parser.check(TokenKind::Eof) {
        return parser.unexpected("`}`").map_err(in_string);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer;

    fn parse_source(source: &str) -> Result<Program, Error> {
        parse(lexer::lex(source).expect("lexing failed"))
    }

    /// Parse and print the whole program as an outline.
    fn tree(source: &str) -> String {
        printer::print(&parse_source(source).unwrap())
    }

    /// Parse a single expression and print it with every operator bracketed.
    fn expr(source: &str) -> String {
        let program = parse_source(source).unwrap();
        match &program.statements[..] {
            [Stmt { kind: StmtKind::Expr(e), .. }] => printer::expr(e),
            other => panic!("expected one expression, got {other:?}"),
        }
    }

    fn error(source: &str) -> String {
        parse_source(source).unwrap_err().message
    }

    // ----- Expressions -----

    #[test]
    fn multiplication_binds_tighter_than_addition() {
        assert_eq!(expr("1 + 2 * 3"), "(1 + (2 * 3))");
        assert_eq!(expr("1 * 2 + 3"), "((1 * 2) + 3)");
        assert_eq!(expr("(1 + 2) * 3"), "((1 + 2) * 3)");
    }

    #[test]
    fn same_level_operators_group_left() {
        assert_eq!(expr("a - b - c"), "((a - b) - c)");
        assert_eq!(expr("a / b * c % d // e"), "((((a / b) * c) % d) // e)");
    }

    #[test]
    fn power_groups_right_and_beats_negation() {
        assert_eq!(expr("2 ** 3 ** 2"), "(2 ** (3 ** 2))");
        assert_eq!(expr("-2 ** 2"), "(-(2 ** 2))");
        assert_eq!(expr("2 ** -1"), "(2 ** (-1))");
        assert_eq!(expr("a * b ** c"), "(a * (b ** c))");
    }

    #[test]
    fn bit_operators_sit_between_comparison_and_arithmetic() {
        // Looser than `+`, tighter than `==`. In C `flags & mask == 0`
        // compares first, which is never what was meant.
        assert_eq!(expr("flags & mask == 0"), "((flags & mask) == 0)");
        assert_eq!(expr("1 << n + 1"), "(1 << (n + 1))");
        // Among themselves: `|`, then `^`, then `&`, then the shifts.
        assert_eq!(expr("a | b ^ c & d << e"), "(a | (b ^ (c & (d << e))))");
        assert_eq!(expr("a << 1 >> 2"), "((a << 1) >> 2)");
        assert_eq!(expr("a & b & c"), "((a & b) & c)");
        // `~` binds like the minus in front of a number.
        assert_eq!(expr("~a & b"), "((~a) & b)");
        assert_eq!(expr("~-a"), "(~(-a))");
        assert_eq!(expr("-~a ** 2"), "(-(~(a ** 2)))");
        assert_eq!(expr("0 <= x & 7 < 8"), "(0 <= (x & 7) < 8)");
        assert_eq!(expr("a | b and c | d"), "((a | b) and (c | d))");
    }

    #[test]
    fn logic_operators() {
        assert_eq!(expr("a or b and not c == d"), "(a or (b and (not (c == d))))");
        assert_eq!(expr("not a and b"), "((not a) and b)");
        assert_eq!(expr("port < 1 or port > 65535"), "((port < 1) or (port > 65535))");
    }

    #[test]
    fn word_comparisons() {
        assert_eq!(expr("user is none"), "(user is none)");
        assert_eq!(expr("user is not none"), "(user is not none)");
        assert_eq!(expr("x in items"), "(x in items)");
        assert_eq!(expr("x not in items"), "(x not in items)");
    }

    #[test]
    fn chained_comparisons() {
        assert_eq!(expr("0 <= i < len"), "(0 <= i < len)");
        assert_eq!(expr("a < b < c == d"), "(a < b < c == d)");
        assert_eq!(expr("not 1 <= port <= 65535"), "(not (1 <= port <= 65535))");
        assert_eq!(expr("a < b and b < c"), "((a < b) and (b < c))");
    }

    #[test]
    fn calls_fields_and_indexing_chain() {
        assert_eq!(expr("user.name.upper()"), "user.name.upper()");
        assert_eq!(expr("grid[1][2]"), "grid[1][2]");
        assert_eq!(expr("-x.y(1)[2]"), "(-x.y(1)[2])");
        assert_eq!(expr("f()"), "f()");
        assert_eq!(expr("f(1, 2,)"), "f(1, 2)");
    }

    #[test]
    fn named_arguments() {
        assert_eq!(expr("Point(x=3.0, y=4.0)"), "Point(x=3.0, y=4.0)");
        assert_eq!(expr("greet(name, excited=true)"), "greet(name, excited=true)");
        assert_eq!(expr("f(a == b)"), "f((a == b))");
    }

    #[test]
    fn literals() {
        assert_eq!(expr("42"), "42");
        assert_eq!(expr("0.5"), "0.5");
        assert_eq!(expr("\"hi\""), "\"hi\"");
        assert_eq!(expr("true"), "true");
        assert_eq!(expr("none"), "none");
    }

    #[test]
    fn units() {
        assert_eq!(expr("500 ms"), "500 ms");
        assert_eq!(expr("sleep(3.3 V)"), "sleep(3.3 V)");
        assert_eq!(expr("2 * 500 ms"), "(2 * 500 ms)");
    }

    #[test]
    fn lists_and_comprehensions() {
        assert_eq!(expr("[]"), "[]");
        assert_eq!(expr("[1, 2, 3,]"), "[1, 2, 3]");
        assert_eq!(expr("[[1], [2]]"), "[[1], [2]]");
        assert_eq!(expr("[n * n for n in range(10)]"), "[(n * n) for n in range(10)]");
        assert_eq!(
            expr("[n for n in items if n % 2 == 0]"),
            "[n for n in items if ((n % 2) == 0)]"
        );
    }

    #[test]
    fn format_strings() {
        assert_eq!(expr(r#"f"Hello, {who}!""#), r#"f"Hello, {who}!""#);
        assert_eq!(expr(r#"f"{a + b}""#), r#"f"{(a + b)}""#);
        assert_eq!(expr(r#"f"area = {area(shape):.2f}""#), r#"f"area = {area(shape):.2f}""#);
        assert_eq!(expr(r#"f"{items[0]} and {d.x}""#), r#"f"{items[0]} and {d.x}""#);
        assert_eq!(expr(r#"f"no values""#), r#"f"no values""#);
    }

    #[test]
    fn doubled_braces_in_format_strings_are_literal() {
        let program = parse_source(r#"f"{{x}} is {x}""#).unwrap();
        let StmtKind::Expr(Expr { kind: ExprKind::FormatStr(parts), .. }) =
            &program.statements[0].kind
        else {
            panic!("expected an f-string");
        };
        assert_eq!(parts[0], FormatPart::Text("{x} is ".to_string()));
        assert_eq!(parts.len(), 2);
    }

    // ----- Statements -----

    #[test]
    fn declarations_and_assignment() {
        assert_eq!(
            tree("let x: int = 1\nvar y = 2\nconst PI = 3.14\ny = x + 1\ny += 1\np.x = 0\na[0] *= 2"),
            "let x: int = 1\n\
             var y = 2\n\
             const PI = 3.14\n\
             y = (x + 1)\n\
             y += 1\n\
             p.x = 0\n\
             a[0] *= 2\n"
        );
    }

    #[test]
    fn types() {
        assert_eq!(
            tree("let a: list[int] = []\nlet b: dict[str, list[int]] = x\nlet c: str or none = none"),
            "let a: list[int] = []\n\
             let b: dict[str, list[int]] = x\n\
             let c: str or none = none\n"
        );
    }

    #[test]
    fn function_types() {
        assert_eq!(
            tree("function twice(f: function(int) -> int, x: int) -> int { return f(f(x)) }"),
            "function twice(f: function(int) -> int, x: int) -> int\n    return f(f(x))\n"
        );
        assert_eq!(
            tree("let a: function() = f\nlet b: function(str, list[int]) -> bool or none = g"),
            "let a: function() = f\nlet b: function(str, list[int]) -> bool or none = g\n"
        );
    }

    #[test]
    fn a_file_can_declare_itself_strict() {
        assert!(parse_source("strict\nlet x = 1").unwrap().strict);
        assert!(parse_source("# a comment first\n\nstrict\n").unwrap().strict);
        assert!(!parse_source("let x = 1").unwrap().strict);
        // The word is reserved, and the first line is its only place.
        assert_eq!(
            error("let strict = 1"),
            "expected a name for the variable, found `strict`. `strict` is a word of the language, so it cannot be \
             a name. It goes alone on the first line of a file"
        );
        assert_eq!(
            error("let x = 1\nstrict"),
            "expected an expression, found `strict`. `strict` is a word of the language, so it cannot be a name. \
             It goes alone on the first line of a file"
        );
        assert_eq!(tree("strict\nlet x = 1"), "strict\nlet x = 1\n");
    }

    #[test]
    fn functions() {
        assert_eq!(
            tree("function greet(who: str, excited: bool = false) -> str {\n    return who\n}"),
            "function greet(who: str, excited: bool = false) -> str\n    return who\n"
        );
        assert_eq!(
            tree("function parse(text: str) -> int raises ValueError, IoError { return 1 }"),
            "function parse(text: str) -> int raises ValueError, IoError\n    return 1\n"
        );
        assert_eq!(tree("function nothing() {\n}"), "function nothing()\n");
        assert_eq!(tree("function stop() { return }"), "function stop()\n    return\n");
    }

    #[test]
    fn functions_as_values() {
        assert_eq!(
            tree("let double = function(n) { return n * 2 }"),
            "let double = function(n) { return (n * 2) }\n"
        );
        assert_eq!(
            expr("items.map(function(n: int) -> int {\n    let m = n + 1\n    return m\n})"),
            "items.map(function(n: int) -> int { let m = (n + 1); return m })"
        );
        assert_eq!(expr("function() { }()"), "function() { }()");
    }

    #[test]
    fn types_can_be_left_out() {
        assert_eq!(
            tree("function add(a, b) { return a + b }\nclass Point {\n    x\n    y = 0\n}\nenum Shape {\n    Circle(radius)\n}"),
            "function add(a, b)\n    return (a + b)\nclass Point\n    field x\n    field y = 0\nenum Shape\n    Circle(radius)\n"
        );
    }

    #[test]
    fn if_else_chain() {
        let source = "if a {\n    x()\n} else if b {\n    y()\n} else {\n    z()\n}";
        assert_eq!(tree(source), "if a\n    x()\nelse if b\n    y()\nelse\n    z()\n");
    }

    #[test]
    fn else_may_start_on_its_own_line() {
        assert_eq!(tree("if a {\n    x()\n}\nelse {\n    y()\n}\nz()"), "if a\n    x()\nelse\n    y()\nz()\n");
    }

    #[test]
    fn loops() {
        assert_eq!(
            tree("while tries < 3 and not done() {\n    tries += 1\n    if bad { break }\n    continue\n}"),
            "while ((tries < 3) and (not done()))\n    tries += 1\n    if bad\n        break\n    continue\n"
        );
        assert_eq!(tree("for s in [1, 2] {\n    print(s)\n}"), "for s in [1, 2]\n    print(s)\n");
        // With two names: the key and what is under it, or the position
        // and what is there.
        assert_eq!(
            tree("for name, age in ages {\n    print(name, age)\n}"),
            "for name, age in ages\n    print(name, age)\n"
        );
        assert_eq!(
            error("for a, a in ages {\n}"),
            "the two names of a loop are two variables, and both are called `a`. Give them different names"
        );
        assert_eq!(
            error("for a, b, c in ages {\n}"),
            "a loop has one name, or two: `for key, value in ages` is given each key and what is under it"
        );
        assert_eq!(error("for a, in ages {\n}"), "expected a name for the value, found `in`");
    }

    #[test]
    fn classes() {
        let source = "class Point {\n    x: float\n    y: float = 0.0\n\n    function norm(self) -> float {\n        return self.x\n    }\n}";
        assert_eq!(
            tree(source),
            "class Point\n    field x: float\n    field y: float = 0.0\n    function norm(self) -> float\n        return self.x\n"
        );
    }

    #[test]
    fn private_declarations() {
        // The word stands in front of a declaration at the top of a file.
        assert_eq!(
            tree("private let limit = 3\nprivate function helper() { }\nprivate enum Mode {\n    Fast\n}"),
            "private let limit = 3\nprivate function helper()\nprivate enum Mode\n    Fast\n"
        );
        // And in front of a field or a function of a class.
        assert_eq!(
            tree("private owned class Box {\n    private value: int\n    size: int\n\n    private function grow(self) { }\n}"),
            "private owned class Box\n    private field value: int\n    field size: int\n    private function grow(self)\n"
        );
        let goes = "`private` goes in front of a declaration: `private function`, `private class`, \
                    `private enum`, `private let`, `private var` or `private const`";
        assert_eq!(error("private print(1)"), goes);
        assert_eq!(error("private import math"), goes);
        assert_eq!(error("private private let x = 1"), goes);
        // What a block declares is seen only in the block as it is.
        assert_eq!(
            error("function f() {\n    private let x = 1\n}"),
            "`private` is for a declaration at the top of a file, and for a field or a function of a class. \
             `x` is declared inside a block, and is seen only there as it is. Take the word away"
        );
        assert_eq!(
            error("enum Mode {\n    private Fast\n}"),
            "a case of an enum cannot be private on its own: a `match` on the enum has to name every case. \
             Mark the whole enum instead: `private enum`"
        );
        // The word is reserved, so it is never a name.
        assert_eq!(error("let private = 1"), "expected a name for the variable, found `private`");
    }

    #[test]
    fn tests_and_asserts() {
        assert_eq!(
            tree(
                "test \"a new ring is empty\" {\n    let r = ring(4)\n    assert r.size() == 0\n    \
                 assert raises ValueError {\n        ring(0)\n    }\n}"
            ),
            "test \"a new ring is empty\"\n    let r = ring(4)\n    assert (r.size() == 0)\n    \
             assert raises ValueError\n        ring(0)\n"
        );
        // `test` is a word of the language only at the start of a test.
        // Anywhere else it is a name like any other.
        assert_eq!(tree("let test = 1\nprint(test)"), "let test = 1\nprint(test)\n");
        assert_eq!(tree("test(5)"), "test(5)\n");
        let named = "a test has a name, in quotes, that says what it checks: `test \"a new ring is empty\" { ... }`";
        assert_eq!(error("test ring {\n}"), named);
        assert_eq!(error("test {\n}"), named);
        assert_eq!(error("test \"\" {\n}"), "a test needs a name that says what it checks");
        assert_eq!(
            error("test f\"ring {n}\" {\n}"),
            "the name of a test is plain text, with nothing worked out in it. Take the `f` away"
        );
        assert_eq!(
            error("test \"a\" {\n}\ntest \"a\" {\n}"),
            "two tests of this file are called \"a\". Give each its own name, so that a failure says which one it is"
        );
        assert_eq!(
            error("function f() {\n    test \"x\" {\n    }\n}"),
            "a test stands at the top of a file, and this one is inside a block. Move it out of the `{ }`"
        );
        // `assert` is a word of the language, and a test is its place.
        assert_eq!(
            error("assert 1 == 1"),
            "`assert` belongs in a test: `test \"name\" { assert total == 6 }`. In the program itself, check with \
             `if`, and `raise` an error"
        );
        assert_eq!(
            error("test \"x\" {\n    assert\n}"),
            "`assert` needs what must be so after it: `assert total == 6`"
        );
        assert_eq!(
            error("let assert = 1"),
            "expected a name for the variable, found `assert`. `assert` is a word of the language, so it cannot \
             be a name. It goes in a test: `assert total == 6`"
        );
        // Inside a function that is written in a test it is still in the test.
        tree("test \"x\" {\n    let f = function(n: int) -> bool {\n        assert n > 0\n        return true\n    }\n}");
    }

    #[test]
    fn interfaces() {
        // An interface lists functions, each without a body. In the tree
        // it is a class that says it is an interface.
        assert_eq!(
            tree(
                "interface Port {\n    function send(self, text: str) -> int raises Full\n    function name(self) -> str\n}\n\
                 class Uart is Port, sys.Named {\n    speed: int\n}"
            ),
            "interface Port\n    function send(self, text: str) -> int raises Full\n    function name(self) -> str\n\
             class Uart is Port, sys.Named\n    field speed: int\n"
        );
        assert_eq!(tree("private interface Empty {\n}"), "private interface Empty\n");
        assert_eq!(tree("owned class Box[T] is Shape {\n}"), "owned class Box[T] is Shape\n");
        let inside = |body: &str| error(&format!("interface Shape {{\n    {body}\n}}"));
        // Only functions, and nothing about how they are done.
        assert_eq!(
            inside("radius: float"),
            "an interface has functions and no fields. How a class keeps its data is the class's own business. \
             If other code needs the value, give the interface a function that returns it"
        );
        assert_eq!(
            inside("function area(self) -> float {\n        return 1.0\n    }"),
            "a function of an interface has no body. It says what a class must have, and each class writes its own. \
             Take the `{ }` away"
        );
        assert_eq!(
            inside("private function area(self) -> float"),
            "a function of an interface cannot be private: an interface lists what other code may ask of a class"
        );
        assert_eq!(inside("let x = 1"), "expected a function, found `let`");
        // There is no body to work anything out from, so all of it is
        // written.
        assert_eq!(
            inside("function area() -> float"),
            "`area` needs `self` as its first parameter: it is called on an object, as in `shape.area()`"
        );
        assert_eq!(
            inside("function scale(self, by)"),
            "the parameter `by` of `scale` needs a type, as in `by: int`. A function of an interface has no body \
             to work the type out from"
        );
        // What is left out for now says so.
        assert_eq!(
            inside("function scale(self, by: float = 2.0)"),
            "the parameter `by` of `scale` cannot have a default in an interface yet"
        );
        assert_eq!(inside("function scale[T](self, by: T)"), "a function of an interface cannot have type parameters yet");
        assert_eq!(
            error("interface Shape[T] {\n}"),
            "an interface cannot have type parameters yet. A class can: `class Box[T]`"
        );
        assert_eq!(
            error("interface Shape is Named {\n}"),
            "an interface cannot be another interface yet. Name both after `is` on the class: \
             `class Circle is Shape, Named`"
        );
        // After `is` come the names of interfaces, each once.
        assert_eq!(error("class Circle is {\n}"), "expected the name of an interface, found `{`");
        assert_eq!(error("class Circle is Shape, Shape {\n}"), "`Shape` is written twice after `is`. Once is enough");
        // The word is reserved, so it is never a name.
        assert_eq!(error("let interface = 1"), "expected a name for the variable, found `interface`");
    }

    #[test]
    fn generics() {
        // The type parameters go in square brackets after the name.
        assert_eq!(
            tree("function first[T](items: list[T]) -> T {\n    return items[0]\n}"),
            "function first[T](items: list[T]) -> T\n    return items[0]\n"
        );
        assert_eq!(
            tree("class Pair[A, B] {\n    left: A\n    right: B\n}"),
            "class Pair[A, B]\n    field left: A\n    field right: B\n"
        );
        assert_eq!(tree("owned class Box[T] {\n    value: T\n}"), "owned class Box[T]\n    field value: T\n");
        // A type can be a generic class with its types filled in.
        assert_eq!(
            tree("let p: Pair[int, list[str]] or none = none"),
            "let p: Pair[int, list[str]] or none = none\n"
        );
        // Square brackets after a name in an expression are still an index.
        assert_eq!(expr("first[0](x)"), "first[0](x)");

        assert_eq!(error("function f[](x: int) { }"), "expected a name for a type, such as `T`, found `]`");
        assert_eq!(error("function f[T, 3](x: T) { }"), "expected a name for a type, such as `T`, found a number");
        assert_eq!(
            error("enum Maybe[T] {\n    Nothing\n}"),
            "an enum cannot have type parameters yet. A class can: `class Box[T]`"
        );
        assert_eq!(
            error("let f = function[T](x: T) -> T { return x }"),
            "a function with no name cannot have type parameters. Give it a name"
        );
    }

    #[test]
    fn enums_and_match() {
        assert_eq!(
            tree("enum Shape {\n    Dot\n    Circle(radius: float)\n    Rect(w: float, h: float)\n}"),
            "enum Shape\n    Dot\n    Circle(radius: float)\n    Rect(w: float, h: float)\n"
        );
        let source = "match shape {\n    case Circle(r) { return r }\n    case Rect(w, _) { return w }\n    case Dot { return 0 }\n    case 1 { a() }\n    case \"x\" { b() }\n    case _ {\n        c()\n    }\n}";
        assert_eq!(
            tree(source),
            "match shape\n    case Circle(r)\n        return r\n    case Rect(w, _)\n        return w\n    case Dot\n        return 0\n    case 1\n        a()\n    case \"x\"\n        b()\n    case _\n        c()\n"
        );
    }

    #[test]
    fn try_except_and_raise() {
        let source = "try {\n    risky()\n} except ValueError as err {\n    print(err)\n} except {\n    raise Fatal(\"no\")\n}";
        assert_eq!(
            tree(source),
            "try\n    risky()\nexcept ValueError as err\n    print(err)\nexcept\n    raise Fatal(\"no\")\n"
        );
    }

    #[test]
    fn imports() {
        assert_eq!(
            tree("import math\nimport os.path\nfrom board import led, button"),
            "import math\nimport os.path\nfrom board import led, button\n"
        );
    }

    #[test]
    fn a_module_has_one_name() {
        assert_eq!(
            error("import drivers.uart as uart"),
            "a module has one name, so there is no `as` after an `import`. `drivers.uart` is used as `drivers.uart.name`. \
             For a short name, import what is used: `from drivers.uart import ...`, and then `uart` is not written at all"
        );
        assert_eq!(
            error("from drivers.uart import send as put"),
            "`send` keeps the name `drivers.uart` gave it, so there is no `as` after an `import`. Use it as `send`"
        );
    }

    #[test]
    fn statements_remember_where_they_start() {
        let program = parse_source("let a = 1\n\n  print(a)").unwrap();
        let at: Vec<_> = program.statements.iter().map(|s| (s.span.line, s.span.col)).collect();
        assert_eq!(at, vec![(1, 1), (3, 3)]);
    }

    // ----- Errors -----

    #[test]
    fn errors_say_what_was_expected() {
        assert_eq!(error("let = 3"), "expected a name for the variable, found `=`");
        assert_eq!(error("let x 3"), "expected `=`, found a number");
        assert_eq!(error("1 +"), "expected an expression, found the end of the file");
        assert_eq!(error("f(1"), "expected `)`, found the end of the file");
        assert_eq!(error("let x = 1 let y = 2"), "expected the end of the line, found `let`");
        assert_eq!(error("if x\n{ }"), "expected `{`, found the end of the line");
    }

    #[test]
    fn errors_point_at_the_right_place() {
        let e = parse_source("let a = 1\nlet b = * 2").unwrap_err();
        assert_eq!((e.span.line, e.span.col), (2, 9));
    }

    #[test]
    fn hints_for_habits_from_other_languages() {
        assert_eq!(error("if x:"), "expected `{`, found `:`. Zeno uses `{ }` around blocks");
        assert_eq!(error("def greet() { }"), "Zeno spells `def` as `function`");
        assert_eq!(error("fn greet() { }"), "Zeno spells `fn` as `function`");
        assert_eq!(error("if a { } elif b { }"), "Zeno spells `elif` as `else if`");
    }

    #[test]
    fn structural_errors() {
        assert_eq!(error("if x {\n  y()"), "this `{` is never closed");
        assert_eq!(error("try { x() }"), "`try` needs at least one `except`");
        assert_eq!(error("1 = 2"), "this cannot be assigned to");
        assert_eq!(error("f() = 2"), "this cannot be assigned to");
        assert_eq!(error("a is b"), "`is` only tests for `none`. Use `==` to compare values");
        assert_eq!(error("match x { y }"), "expected `case`, found the name `y`");
        assert_eq!(error("let x: int or str = 1"), "only `or none` can follow a type for now");
    }

    #[test]
    fn format_string_errors() {
        assert_eq!(error(r#"f"{}""#), "in f-string: `{}` is empty. Put an expression inside");
        assert_eq!(error(r#"f"{x""#), "in f-string: `{` is never closed");
        assert_eq!(error(r#"f"x}""#), "in f-string: `}` without a `{`. Write `}}` for a literal brace");
        assert_eq!(error(r#"f"{1 +}""#), "in f-string: expected an expression, found the end of the file");
        assert_eq!(error(r#"f"{a b}""#), "in f-string: expected `}`, found the name `b`");
    }

    #[test]
    fn a_parameter_with_a_default_comes_after_those_without() {
        assert_eq!(
            error("function f(a = 1, b) { }"),
            "`b` has no default, and comes after `a`, which has one. A call gives its arguments in order, \
             so `a` could only be left out by naming every argument after it. Put the parameters that have a default last"
        );
        assert!(parse_source("function f(a, b = 1, c = 2) { }").is_ok());
        // A class says what it holds in the order it holds it.
        assert!(parse_source("class P {\n x: int = 0\n y: int\n}").is_ok());
    }

    #[test]
    fn elif_at_the_start_of_a_line_gets_the_hint_too() {
        assert_eq!(error("if a {\n}\nelif b {\n}"), "Zeno spells `elif` as `else if`");
        // It is still a name a program may use.
        assert!(parse_source("let elif = 3\nelif = 4").is_ok());
    }

    // ----- The examples -----

    #[test]
    fn the_example_programs_parse() {
        for (name, source) in [
            ("hello", include_str!("../examples/hello.zeno")),
            ("tour", include_str!("../examples/tour.zeno")),
            ("blink", include_str!("../examples/blink.zeno")),
            ("packet", include_str!("../examples/packet.zeno")),
            ("generics", include_str!("../examples/generics.zeno")),
        ] {
            if let Err(e) = parse_source(source) {
                panic!("examples/{name}.zeno:{}:{}: {}", e.span.line, e.span.col, e.message);
            }
        }
    }

    #[test]
    fn a_name_can_say_which_module_it_is_from() {
        // Where a type, an enum case or an error is named, `a.b` is one
        // name. `modules.rs` checks that there is such a module.
        assert_eq!(
            tree(
                "let p: shapes.Point or none = none\n\
                 function load(at: drivers.spi.Port) -> list[shapes.Point] raises shapes.Bad, IoError { return [] }\n\
                 match s {\n    case shapes.Dot { }\n    case shapes.Circle(r) { }\n}\n\
                 try { } except shapes.Bad as bad { }"
            ),
            "let p: shapes.Point or none = none\n\
             function load(at: drivers.spi.Port) -> list[shapes.Point] raises shapes.Bad, IoError\n    return []\n\
             match s\n    case shapes.Dot\n    case shapes.Circle(r)\n\
             try\nexcept shapes.Bad as bad\n"
        );
        // In an expression it stays what it always was: a field.
        assert_eq!(expr("shapes.area(s)"), "shapes.area(s)");
    }

    #[test]
    fn a_dict_is_written_in_square_brackets() {
        assert_eq!(tree("let ages = [\"ann\": 31, \"bo\": 27]"), "let ages = [\"ann\": 31, \"bo\": 27]\n");
        assert_eq!(tree("let none_yet = [:]"), "let none_yet = [:]\n");
        assert_eq!(tree("let squares = [1: 1, 2: 4,]"), "let squares = [1: 1, 2: 4]\n");
        assert_eq!(tree("let n = [a + 1: b * 2][2]"), "let n = [(a + 1): (b * 2)][2]\n");
        assert_eq!(tree("let both = [[1, 2], [\"a\": [3]]]"), "let both = [[1, 2], [\"a\": [3]]]\n");
        // One kind of thing between a pair of brackets.
        assert_eq!(
            error("let d = [\"a\": 1, 2]"),
            "every entry of a dict is written `key: value`, and this one has no `:`"
        );
        assert_eq!(
            error("let d = [1, \"a\": 2]"),
            "every entry of a dict is written `key: value`, and the ones before this are not"
        );
        assert_eq!(
            error("let d = [: 1]"),
            "an entry of a dict is written `key: value`, and a dict with nothing in it is `[:]`"
        );
    }

    #[test]
    fn a_part_is_taken_with_slice() {
        // Python's `xs[1:3]` is not Zeno. The message says what is.
        let part = "a part of a list or a str is taken with `.slice(from, to)`, as in `xs.slice(1, 3)`";
        assert_eq!(error("let ys = xs[1:3]"), part);
        assert_eq!(error("let ys = xs[:3]"), part);
        assert_eq!(error("let ys = xs[1:]"), part);
        assert_eq!(tree("let ys = xs.slice(1, 3)"), "let ys = xs.slice(1, 3)\n");
    }

    #[test]
    fn a_parameter_that_takes_says_take() {
        // The word goes in front of the type, as `owned` does.
        assert_eq!(
            tree("function send(frame: take Frame, log: list[str]) {\n}"),
            "function send(frame: take Frame, log: list[str])\n"
        );
        assert_eq!(tree("let f: function(take Frame, int) -> int = g"), "let f: function(take Frame, int) -> int = g\n");
        assert_eq!(tree("let xs: owned list[int] = []"), "let xs: owned list[int] = []\n");
        // `owned` and `take` are reserved, like every word of the
        // language, so neither has to be told apart from a name.
        assert_eq!(
            error("let take = 3"),
            "expected a name for the variable, found `take`. `take` is a word of the language, so it cannot be a \
             name. It goes in front of the type of a parameter: `frame: take Frame`"
        );
        assert_eq!(
            error("let n = owned + 1"),
            "expected an expression, found `owned`. `owned` is a word of the language, so it cannot be a name. It \
             goes in front of `class`, or in front of a type: `owned list[int]`"
        );
        assert_eq!(
            error("queue.take()"),
            "expected a name after `.`, found `take`. `take` is a word of the language, so it cannot be a name. It \
             goes in front of the type of a parameter: `frame: take Frame`"
        );
        // `borrow` was the word for a parameter that only looks. Every
        // parameter looks now, so writing it says what to write instead.
        assert_eq!(
            error("function total(xs: borrow list[int]) -> int { return len(xs) }"),
            "`borrow` is no longer a word. A parameter looks at what it is given without one: write the type \
             alone. A parameter that takes the value over says `take`"
        );
        // It is an ordinary name everywhere else.
        assert_eq!(tree("let borrow = 3"), "let borrow = 3\n");
    }

    #[test]
    fn unsafe_is_a_block_and_a_word_of_its_own() {
        assert_eq!(tree("unsafe {\n    p[0] = 1\n    free(p)\n}\nprint(1)"), "unsafe\n    p[0] = 1\n    free(p)\nprint(1)\n");
        assert_eq!(tree("if x { unsafe { free(p) } }"), "if x\n    unsafe\n        free(p)\n");
        assert_eq!(tree("let p: pointer[u8] or none = none"), "let p: pointer[u8] or none = none\n");
        // The word is reserved: it is never a name, so it never has to
        // be told apart from one.
        assert_eq!(error("let unsafe = 3"), "expected a name for the variable, found `unsafe`");
        assert_eq!(error("unsafe print(1)"), "expected `{`, found the name `print`");
        // And the block is a statement. It gives no value.
        assert_eq!(
            error("let x = unsafe { p[0] }"),
            "`unsafe { }` is a statement and gives no value. Put the whole line inside the block: `unsafe { x = p[0] }`"
        );
    }
}
