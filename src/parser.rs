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

use crate::ast::*;
use crate::error::Error;
use crate::lexer;
use crate::token::{Span, Token, TokenKind};

/// Turn the tokens of a whole file into a syntax tree.
pub fn parse(tokens: Vec<Token>) -> Result<Program, Error> {
    let mut parser = Parser { tokens, pos: 0 };
    let mut statements = Vec::new();

    parser.skip_newlines();
    while !parser.check(TokenKind::Eof) {
        statements.push(parser.statement()?);
        parser.end_of_statement()?;
    }
    Ok(Program { statements })
}

struct Parser {
    /// Always ends with `Eof`.
    tokens: Vec<Token>,
    /// Index of the next unread token.
    pos: usize,
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
        self.error(format!("expected {wanted}, found {}", self.peek().describe()))
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

        let kind = match self.peek() {
            TokenKind::Let | TokenKind::Var | TokenKind::Const => self.declaration()?,
            TokenKind::Function => StmtKind::Function(self.function()?),
            TokenKind::Class => self.class()?,
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

        let mut statements = Vec::new();
        while !self.check(TokenKind::RBrace) {
            if self.check(TokenKind::Eof) {
                return Err(Error::new("this `{` is never closed", open));
            }
            statements.push(self.statement()?);
            self.end_of_statement()?;
        }
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
        self.expect(TokenKind::In)?;
        let iterable = self.expression()?;
        let body = self.block()?;
        Ok(StmtKind::For { variable, iterable, body })
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
            TokenKind::Ident(name) => {
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
                TokenKind::Ident(_) => Some(self.name("an error type")?),
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
        Ok(StmtKind::Import { module, names })
    }

    // ---------------------------------------- Functions, classes and enums

    /// `function name(params) -> type raises Error { }`
    fn function(&mut self) -> Result<Function, Error> {
        self.advance(); // function
        let name = self.name("a name for the function")?;

        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        while !self.check(TokenKind::RParen) {
            let name = self.name("a parameter name")?;
            let ty = if self.eat(TokenKind::Colon) { Some(self.type_()?) } else { None };
            let default = if self.eat(TokenKind::Assign) { Some(self.expression()?) } else { None };
            params.push(Param { name, ty, default });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RParen)?;

        let return_type = if self.eat(TokenKind::Arrow) { Some(self.type_()?) } else { None };

        let mut raises = Vec::new();
        if self.eat(TokenKind::Raises) {
            raises.push(self.name("an error type")?);
            while self.eat(TokenKind::Comma) {
                raises.push(self.name("an error type")?);
            }
        }

        let body = self.block()?;
        Ok(Function { name, params, return_type, raises, body })
    }

    /// `class Name { fields and methods }`
    fn class(&mut self) -> Result<StmtKind, Error> {
        self.advance(); // class
        let name = self.name("a name for the class")?;
        self.expect(TokenKind::LBrace)?;
        self.skip_newlines();

        let mut fields = Vec::new();
        let mut methods = Vec::new();
        while !self.check(TokenKind::RBrace) {
            if self.check(TokenKind::Function) {
                methods.push(self.function()?);
            } else if matches!(self.peek(), TokenKind::Ident(_)) {
                fields.push(self.field()?);
            } else {
                return self.unexpected("a field or a function");
            }
            self.end_of_statement()?;
        }
        self.advance(); // the }
        Ok(StmtKind::Class(Class { name, fields, methods }))
    }

    /// `name: type`, optionally with `= default`
    fn field(&mut self) -> Result<Field, Error> {
        let name = self.name("a field name")?;
        self.expect(TokenKind::Colon)?;
        let ty = self.type_()?;
        let default = if self.eat(TokenKind::Assign) { Some(self.expression()?) } else { None };
        Ok(Field { name, ty, default })
    }

    /// `enum Name { Case, Case(field: type) ... }`, one case per line
    fn enum_(&mut self) -> Result<StmtKind, Error> {
        self.advance(); // enum
        let name = self.name("a name for the enum")?;
        self.expect(TokenKind::LBrace)?;
        self.skip_newlines();

        let mut variants = Vec::new();
        while !self.check(TokenKind::RBrace) {
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

    /// `int`, `list[int]`, `str or none`
    fn type_(&mut self) -> Result<Type, Error> {
        let name = self.name("a type")?;

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
        let ty = Type::Named { name, args };

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
        let first = self.sum()?;

        let mut rest = Vec::new();
        while let Some(op) = self.comparison_op() {
            let right = self.sum()?;
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
                    let name = self.name("a name after `.`")?;
                    ExprKind::Field { object: Box::new(expr), name }
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = Box::new(self.expression()?);
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
            TokenKind::Ident(name) => {
                self.advance();
                ExprKind::Name(name)
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

    /// After a `[`: either a list `[a, b, c]` or a comprehension
    /// `[element for x in items if condition]`. They start the same way, so
    /// parse the first element and then see whether `for` follows.
    fn list(&mut self) -> Result<ExprKind, Error> {
        if self.eat(TokenKind::RBracket) {
            return Ok(ExprKind::List(Vec::new()));
        }
        let first = self.expression()?;

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

    let mut parser = Parser { tokens, pos: 0 };
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

    // ----- The examples -----

    #[test]
    fn the_example_programs_parse() {
        for (name, source) in [
            ("hello", include_str!("../examples/hello.zeno")),
            ("tour", include_str!("../examples/tour.zeno")),
            ("blink", include_str!("../examples/blink.zeno")),
        ] {
            if let Err(e) = parse_source(source) {
                panic!("examples/{name}.zeno:{}:{}: {}", e.span.line, e.span.col, e.message);
            }
        }
    }
}
