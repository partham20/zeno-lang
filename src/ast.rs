//! The syntax tree: what a Zeno program looks like once the parser has
//! worked out its structure.
//!
//! Tokens are flat ("let", "x", "=", "1", "+", "2"). The tree is nested:
//! a `Let` statement that holds a `Binary` expression that holds two numbers.
//! Every later stage (interpreter, type checker, code generator) walks this tree.
//!
//! There are two main kinds of node:
//! - a **statement** does something: declare a variable, loop, return.
//! - an **expression** produces a value: `1 + 2`, `area(shape)`, `user.name`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::ints::Width;
use crate::token::Span;

/// A whole source file.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    /// True when the file begins with the line `strict`, which asks for
    /// every type to be known before the program runs.
    pub strict: bool,
    pub statements: Vec<Stmt>,
    /// The names the file declares with `private` in front: what it
    /// declares at its top and keeps to itself.
    pub private: Vec<String>,
    /// The other files of the program, when it has several: the files it
    /// imports, and the files those import. `modules.rs` fills this in,
    /// and puts their statements in front of this file's.
    pub modules: Vec<ModuleFile>,
}

impl Program {
    /// A build leaves the tests of a program out. They are not in what
    /// is built, and they are not held to the rules of the build: a
    /// test may share a list in a program that is built with no
    /// collector (decision 10.20).
    pub fn without_tests(mut self) -> Program {
        self.statements.retain(|statement| !matches!(statement.kind, StmtKind::Test { .. }));
        self
    }

    /// `count += 1` on a `u8` means `count = u8(count + 1)`: the sum is
    /// an int, and it is checked to fit where it is stored (decision
    /// 10.27). The checker finds each such statement and says where it
    /// is, with the size of what it stores into, and here each one is
    /// made the statement it means. Every engine then runs or compiles
    /// the long form, which it could do already, so there is nothing
    /// for the four of them to do alike.
    pub fn with_fits(mut self, fits: &HashMap<(usize, usize), Width>) -> Program {
        let mut visit = |statement: &mut Stmt| {
            let span = statement.span;
            let StmtKind::Assign { target, op, value } = &mut statement.kind else {
                return;
            };
            let (Some(operator), Some(width)) = (*op, fits.get(&(span.line, span.col))) else {
                return;
            };
            let nothing = Expr { kind: ExprKind::None, span };
            let sum = ExprKind::Binary {
                op: operator,
                left: Box::new(target.clone()),
                right: Box::new(std::mem::replace(value, nothing)),
            };
            let convert = Expr { kind: ExprKind::Name(width.name.to_string()), span };
            let args = vec![Arg { name: None, value: Expr { kind: sum, span } }];
            *value = Expr { kind: ExprKind::Call { callee: Box::new(convert), args }, span };
            *op = None;
        };
        fresh_block(&mut self.statements, &mut visit);
        self
    }
}

/// A file that the program imports.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleFile {
    /// Where the file is, as an error message names it.
    pub path: String,
    /// True when this file begins with the line `strict`.
    pub strict: bool,
}

/// The name a class, an enum or a case is printed by.
///
/// Inside the compiler a name from an imported file has the file's module
/// name in front, so that two files can each have a `Point`: the one in
/// `shapes.zeno` is `shapes.Point`. A copy of a generic class has its
/// types behind it: `Stack[int]`. A running program prints neither. It
/// prints `Point(x=1, y=2)` and `Stack(items=[1])`, the name as the
/// file that declares it wrote it.
pub fn shown_name(name: &str) -> &str {
    let name = name.split('[').next().unwrap_or(name);
    name.rsplit('.').next().unwrap_or(name)
}

/// The statements between `{` and `}`.
pub type Block = Vec<Stmt>;

// ---------------------------------------------------------------- Statements

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    /// Where the statement starts, for error messages.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// `let x = 1` (mutable: false) or `var x = 1` (mutable: true)
    Let { mutable: bool, name: String, ty: Option<Type>, value: Expr },
    /// `const PI = 3.14`
    Const { name: String, ty: Option<Type>, value: Expr },
    /// `x = 2`, or with an operator: `x += 2` has `op: Some(Add)`
    Assign { target: Expr, op: Option<BinaryOp>, value: Expr },
    /// An expression used as a statement, usually a call: `print("hi")`
    Expr(Expr),

    Function(Arc<Function>),
    Class(Class),
    Enum(Enum),

    /// `else if` is stored as an else block holding one `If` statement.
    If { condition: Expr, then_block: Block, else_block: Option<Block> },
    While { condition: Expr, body: Block },
    /// `for variable in iterable { body }`. With two names, `for key,
    /// value in ages`, `variable` is the key (or the position, in a
    /// list) and `second` is what is there.
    For { variable: String, second: Option<String>, iterable: Expr, body: Block },
    Match { subject: Expr, arms: Vec<Arm> },
    Try { body: Block, handlers: Vec<Handler> },
    /// `unsafe { ... }`: the only place where a pointer may be read
    /// through, written through or freed. See docs/unsafe.md.
    Unsafe(Block),

    /// `test "a new ring is empty" { ... }`, at the top of a file. Run
    /// by `zeno test`, and left out of a build. See docs/testing.md.
    Test { name: String, body: Block },
    /// `assert total == 6`: what must be so, in a test.
    Assert(Expr),
    /// `assert raises ValueError { ... }`: the block must raise that.
    AssertRaises { error: String, body: Block },

    Return(Option<Expr>),
    Raise(Expr),
    Break,
    Continue,

    /// `import a.b` has no names. `from a.b import x, y` has names x and y.
    Import { module: Vec<String>, names: Vec<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    /// Empty for a function written as a value: `function(n) { return n * 2 }`
    pub name: String,
    /// `function first[T](items: list[T]) -> T` has the one type
    /// parameter `T`: a type that each call fills in.
    pub type_params: Vec<String>,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    /// The errors named after `raises`.
    pub raises: Vec<String>,
    pub body: Block,
}

/// One parameter: `who`, `who: str`, or `excited: bool = false`.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub name: String,
    /// `class Stack[T]` has the one type parameter `T`.
    pub type_params: Vec<String>,
    /// `owned class`: every object of it has exactly one owner.
    pub owned: bool,
    pub fields: Vec<Field>,
    pub methods: Vec<Arc<Function>>,
    /// The fields and methods with `private` in front: only the file
    /// the class is written in can use them.
    pub private: Vec<String>,
    /// `class Circle is Shape, Named`: the interfaces the class says it
    /// is. It must have every function they list.
    pub is: Vec<String>,
    /// True for `interface Shape { ... }`. An interface is kept as a
    /// class with no fields, whose functions have no bodies: it is a
    /// type that objects of other classes are seen as.
    pub interface: bool,
}

impl Function {
    /// A copy of this function that shares nothing with it.
    ///
    /// The stages after the parser keep notes about a function by where
    /// its nodes are in memory. A generic function is checked and compiled
    /// once for each set of types it is used with, and each of those needs
    /// notes of its own, so each gets a copy of the tree. `clone` alone is
    /// not enough: a function written inside this one is behind an `Arc`,
    /// which `clone` shares instead of copying.
    pub fn fresh(&self) -> Function {
        self.fresh_with(&mut |_| {})
    }

    /// `fresh`, with every statement of the copy shown to `visit` on the
    /// way, which may change it.
    fn fresh_with(&self, visit: &mut dyn FnMut(&mut Stmt)) -> Function {
        let mut copy = self.clone();
        fresh_block(&mut copy.body, visit);
        copy.params.iter_mut().filter_map(|param| param.default.as_mut()).for_each(|value| fresh_expr(value, visit));
        copy
    }
}

impl Class {
    /// A copy of this class that shares nothing with it. See `Function::fresh`.
    pub fn fresh(&self) -> Class {
        self.fresh_with(&mut |_| {})
    }

    fn fresh_with(&self, visit: &mut dyn FnMut(&mut Stmt)) -> Class {
        let mut copy = self.clone();
        copy.fields.iter_mut().filter_map(|field| field.default.as_mut()).for_each(|value| fresh_expr(value, visit));
        for method in &mut copy.methods {
            *method = Arc::new(method.fresh_with(visit));
        }
        copy
    }
}

/// Give every function written inside these statements a copy of its own,
/// and show `visit` each statement on the way.
fn fresh_block(block: &mut Block, visit: &mut dyn FnMut(&mut Stmt)) {
    for statement in block {
        visit(statement);
        match &mut statement.kind {
            StmtKind::Let { value, .. } | StmtKind::Const { value, .. } => fresh_expr(value, visit),
            StmtKind::Assign { target, value, .. } => {
                fresh_expr(target, visit);
                fresh_expr(value, visit);
            }
            StmtKind::Expr(expr) | StmtKind::Raise(expr) => fresh_expr(expr, visit),
            StmtKind::Return(value) => value.iter_mut().for_each(|value| fresh_expr(value, visit)),
            StmtKind::Function(function) => *function = Arc::new(function.fresh_with(visit)),
            StmtKind::Class(class) => *class = class.fresh_with(visit),
            StmtKind::Enum(decl) => {
                let fields = decl.variants.iter_mut().flat_map(|case| &mut case.fields);
                fields.filter_map(|field| field.default.as_mut()).for_each(|value| fresh_expr(value, visit));
            }
            StmtKind::If { condition, then_block, else_block } => {
                fresh_expr(condition, visit);
                fresh_block(then_block, visit);
                else_block.iter_mut().for_each(|block| fresh_block(block, visit));
            }
            StmtKind::While { condition, body } => {
                fresh_expr(condition, visit);
                fresh_block(body, visit);
            }
            StmtKind::For { iterable, body, .. } => {
                fresh_expr(iterable, visit);
                fresh_block(body, visit);
            }
            StmtKind::Match { subject, arms } => {
                fresh_expr(subject, visit);
                arms.iter_mut().for_each(|arm| fresh_block(&mut arm.body, visit));
            }
            StmtKind::Try { body, handlers } => {
                fresh_block(body, visit);
                handlers.iter_mut().for_each(|handler| fresh_block(&mut handler.body, visit));
            }
            StmtKind::Unsafe(body) => fresh_block(body, visit),
            StmtKind::Test { body, .. } | StmtKind::AssertRaises { body, .. } => fresh_block(body, visit),
            StmtKind::Assert(condition) => fresh_expr(condition, visit),
            StmtKind::Break | StmtKind::Continue | StmtKind::Import { .. } => {}
        }
    }
}

fn fresh_expr(expr: &mut Expr, visit: &mut dyn FnMut(&mut Stmt)) {
    match &mut expr.kind {
        ExprKind::Function(function) => *function = Arc::new(function.fresh_with(visit)),
        ExprKind::FormatStr(parts) => {
            for part in parts {
                if let FormatPart::Value { value, .. } = part {
                    fresh_expr(value, visit);
                }
            }
        }
        ExprKind::Unit { value, .. } => fresh_expr(value, visit),
        ExprKind::List(items) => items.iter_mut().for_each(|value| fresh_expr(value, visit)),
        ExprKind::Dict(entries) => {
            for (key, value) in entries {
                fresh_expr(key, visit);
                fresh_expr(value, visit);
            }
        }
        ExprKind::ListComp { element, iterable, condition, .. } => {
            fresh_expr(element, visit);
            fresh_expr(iterable, visit);
            condition.iter_mut().for_each(|condition| fresh_expr(condition, visit));
        }
        ExprKind::Unary { operand, .. } => fresh_expr(operand, visit),
        ExprKind::Binary { left, right, .. } => {
            fresh_expr(left, visit);
            fresh_expr(right, visit);
        }
        ExprKind::Compare { first, rest } => {
            fresh_expr(first, visit);
            rest.iter_mut().for_each(|(_, operand)| fresh_expr(operand, visit));
        }
        ExprKind::Call { callee, args } => {
            fresh_expr(callee, visit);
            args.iter_mut().for_each(|arg| fresh_expr(&mut arg.value, visit));
        }
        ExprKind::Field { object, .. } => fresh_expr(object, visit),
        ExprKind::Index { object, index } => {
            fresh_expr(object, visit);
            fresh_expr(index, visit);
        }
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Str(_)
        | ExprKind::Bool(_)
        | ExprKind::None
        | ExprKind::Name(_) => {}
    }
}

/// A named slot: a class field, or one value carried by an enum case.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Enum {
    pub name: String,
    pub variants: Vec<Variant>,
}

/// One case of an enum: `Idle`, or `Circle(radius: float)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<Field>,
}

/// One `case pattern { body }` inside a match.
#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    pub pattern: Pattern,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// `_` matches anything.
    Wildcard,
    /// A bare name. Whether it is an enum case (`Idle`) or a new variable
    /// that captures the value is decided later, when names are resolved.
    Name(String),
    /// `0`, `"yes"`, `true`, `none`
    Literal(Expr),
    /// `Circle(r)`, `Rect(w, h)`
    Variant { name: String, fields: Vec<Pattern> },
}

/// One `except Error as name { body }` after a try.
#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    /// `None` catches every error.
    pub error: Option<String>,
    pub binding: Option<String>,
    pub body: Block,
}

// --------------------------------------------------------------------- Types

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// `int`, `Point`, or with arguments: `list[int]`
    Named { name: String, args: Vec<Type> },
    /// `str or none`
    Optional(Box<Type>),
    /// The type of a function: `function(int, int) -> int`
    Function { params: Vec<Type>, returns: Option<Box<Type>> },
    /// `owned list[int]`: a value with exactly one owner.
    Owned(Box<Type>),
    /// `take Packet`, on a parameter: the function takes the value over,
    /// and the caller's variable holds nothing afterwards. A parameter
    /// without the word only looks.
    Take(Box<Type>),
}

// --------------------------------------------------------------- Expressions

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Int(u64),
    Float(f64),
    Str(String),
    /// `f"Hello, {who}!"`, split into its text and `{...}` parts.
    FormatStr(Vec<FormatPart>),
    Bool(bool),
    None,
    /// A variable, function or type, used by name.
    Name(String),
    /// A number with a unit: `500 ms`
    Unit { value: Box<Expr>, unit: String },
    /// `[1, 2, 3]`
    List(Vec<Expr>),
    /// `["a": 1, "b": 2]`, and `[:]` for one with nothing in it.
    Dict(Vec<(Expr, Expr)>),
    /// `[element for variable in iterable if condition]`
    ListComp {
        element: Box<Expr>,
        variable: String,
        iterable: Box<Expr>,
        condition: Option<Box<Expr>>,
    },
    /// `-x`, `not ready`
    Unary { op: UnaryOp, operand: Box<Expr> },
    /// `a + b`, `x == y`, `p and q`
    Binary { op: BinaryOp, left: Box<Expr>, right: Box<Expr> },
    /// A chain of two or more comparisons: `0 <= i < len`.
    /// It means `0 <= i and i < len`, with `i` worked out only once.
    Compare { first: Box<Expr>, rest: Vec<(BinaryOp, Expr)> },
    /// A function with no name, used as a value: `function(n) { return n * 2 }`
    Function(Arc<Function>),
    /// `callee(args)`
    Call { callee: Box<Expr>, args: Vec<Arg> },
    /// `object.name`
    Field { object: Box<Expr>, name: String },
    /// `object[index]`
    Index { object: Box<Expr>, index: Box<Expr> },
}

/// One piece of an f-string.
#[derive(Debug, Clone, PartialEq)]
pub enum FormatPart {
    Text(String),
    /// `{value}` or `{value:format}`, e.g. `{area:.2f}`
    Value { value: Expr, format: Option<String> },
}

/// One argument in a call: `3.0`, or named: `x=3.0`.
#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
    /// `~x`: every bit of a whole number turned over.
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    /// `&`, `|`, `^`, `<<`, `>>`: the bits of whole numbers.
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
    Is,
    IsNot,
    In,
    NotIn,
}

impl BinaryOp {
    /// The operator as it is written in source.
    pub fn text(self) -> &'static str {
        use BinaryOp::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            FloorDiv => "//",
            Mod => "%",
            Pow => "**",
            BitAnd => "&",
            BitOr => "|",
            BitXor => "^",
            Shl => "<<",
            Shr => ">>",
            Eq => "==",
            NotEq => "!=",
            Lt => "<",
            LtEq => "<=",
            Gt => ">",
            GtEq => ">=",
            And => "and",
            Or => "or",
            Is => "is",
            IsNot => "is not",
            In => "in",
            NotIn => "not in",
        }
    }
}
