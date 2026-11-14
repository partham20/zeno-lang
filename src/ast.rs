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

use crate::token::Span;

/// A whole source file.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub statements: Vec<Stmt>,
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

    Function(Function),
    Class(Class),
    Enum(Enum),

    /// `else if` is stored as an else block holding one `If` statement.
    If { condition: Expr, then_block: Block, else_block: Option<Block> },
    While { condition: Expr, body: Block },
    /// `for variable in iterable { body }`
    For { variable: String, iterable: Expr, body: Block },
    Match { subject: Expr, arms: Vec<Arm> },
    Try { body: Block, handlers: Vec<Handler> },

    Return(Option<Expr>),
    Raise(Expr),
    Break,
    Continue,

    /// `import a.b` has no names. `from a.b import x, y` has names x and y.
    Import { module: Vec<String>, names: Vec<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
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
    pub fields: Vec<Field>,
    pub methods: Vec<Function>,
}

/// A named, typed slot: a class field, or one value carried by an enum case.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: Type,
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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
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
            Mod => "%",
            Pow => "**",
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
