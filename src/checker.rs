//! The type checker: finds mistakes before the program runs.
//!
//! It walks the syntax tree the same way the interpreter does, but instead of
//! working out *values* it works out *types*. Where the interpreter's `eval`
//! turns `1 + 2` into `3`, the checker's `expr` turns it into `int`.
//!
//! Zeno's types are optional, so the checker is "gradual":
//!
//! 1. A type that is written down is checked.
//! 2. A type that is left out is worked out from the value, when possible.
//! 3. Otherwise the value is `Dynamic`: the checker lets it through, and any
//!    mistake is found when the program runs.
//!
//! In strict mode step 3 is an error. Everything must be known in advance.
//!
//! A parameter or field with no type written is worked out from how it is
//! used across the whole program. That needs several passes, because one
//! answer can unlock another; see `check` and `Seen`.
//!
//! Unlike the parser, the checker does not stop at the first error. After
//! reporting one it treats the expression as `Invalid`, which is accepted
//! everywhere, so one mistake does not set off a string of others.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::ast::{self, *};
use crate::error::Error;
use crate::ints::{self, Width};
use crate::token::Span;
use crate::units;

/// How many passes to allow for working out untyped parameters and fields.
const MAX_ROUNDS: usize = 8;

/// Check a whole program. Returns every error found, in source order.
///
/// Strict mode is on if it is asked for here or by the file itself.
pub fn check(program: &Program, strict: bool) -> Vec<Error> {
    check_with_types(program, strict).0
}

/// A type, as the stages after the checker see it. The checker's own `Ty`
/// is full of links back into the checker; this is the plain answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Int,
    Float,
    Str,
    Bool,
    None,
    /// A number with a unit; the string is the unit, such as "ms".
    Quantity(String),
    List(Box<Kind>),
    /// An object of the class with this name.
    Class(String),
    /// A value of the enum with this name.
    Enum(String),
    /// `T or none`: a value of the kind inside, or nothing.
    Optional(Box<Kind>),
    /// A pin of the board: `led`, `pin(7)`. It is the pin's number.
    Pin,
    /// An error value, such as what `ValueError("bad")` makes.
    Error,
    /// A function held as a value.
    Function(Box<FunctionKind>),
    /// A list or an object with one owner, which frees it. Seen only on a
    /// parameter, a field or a return type; a value that is merely borrowed
    /// has the plain kind inside.
    Owned(Box<Kind>),
    /// Anything else, described in words for an error message.
    Other(String),
}

/// What a function takes and gives back.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionKind {
    pub params: Vec<Kind>,
    pub returns: Kind,
}

/// What the checker worked out that the compiler cannot work out for
/// itself: the signature of every function, including the parameter and
/// return types that were never written down.
///
/// A function is looked up by its node in the syntax tree (its address in
/// memory), so the tree that was checked must be the tree that is compiled.
#[derive(Default)]
pub struct Types {
    functions: HashMap<usize, FunctionKind>,
    /// The type of each field of each class, in the order they are listed.
    classes: HashMap<usize, Vec<Kind>>,
    /// For each class, which of its fields are sized whole numbers. Such a
    /// field is stored in that many bits; everywhere else a sized number
    /// travels as an ordinary int.
    widths: HashMap<usize, Vec<Option<Width>>>,
    /// For each enum, the types of the values each case carries.
    enums: HashMap<usize, Vec<Vec<Kind>>>,
    /// Each place a variable is read after a check such as `is not none`
    /// has narrowed it, with the narrower type it has there.
    narrowed: HashMap<usize, Kind>,
    /// Each place a variable is read in order to give its owned value
    /// away. The variable holds nothing afterwards.
    moves: HashSet<usize>,
    /// Each `let`, `var` or `const` whose variable owns its value, and so
    /// frees it when its block ends.
    owners: HashSet<usize>,
    /// Each statement that makes an owned value and does nothing with it.
    discarded: HashSet<usize>,
    /// Each list that begins life as an owned list: written out, built
    /// by a comprehension, or made by `split`, `map`, `filter` or `+`,
    /// where an owned list is wanted.
    fresh: HashSet<usize>,
    /// Each call of `split`, `map` or `filter`, and each `+` on two
    /// lists: an expression that makes a new list, which is nobody's yet.
    made: HashSet<usize>,
    /// Each owned value that is made in the middle of a statement and
    /// only looked at there, as in `print(size(make()))`. The statement
    /// owns it, and frees it when it is over.
    lent: HashSet<usize>,
    /// For each generic function, the copies made of it: one for each set
    /// of types it is called with. A copy is an ordinary function with a
    /// name such as `first[int]`, and the compilers compile it as one.
    function_copies: HashMap<usize, Vec<Arc<ast::Function>>>,
    /// The same for each generic class: `Stack[int]`, `Stack[str]`.
    class_copies: HashMap<usize, Vec<Arc<ast::Class>>>,
    /// Each call of a generic function or class, with the name of the
    /// copy that the call reaches.
    copy_called: HashMap<usize, String>,
    /// What each type written in the program stands for. Inside a copy,
    /// `T` is whatever that copy was made for.
    annotations: HashMap<usize, Kind>,
}

impl Types {
    /// Does reading the variable `expr` here give its owned value away?
    pub fn moves(&self, expr: &ast::Expr) -> bool {
        self.moves.contains(&(expr as *const ast::Expr as usize))
    }

    /// Does the variable this statement declares own its value?
    pub fn owner(&self, statement: &ast::Stmt) -> bool {
        self.owners.contains(&(statement as *const ast::Stmt as usize))
    }

    /// Is `expr`, written as a statement, an owned value nobody keeps?
    pub fn discarded(&self, expr: &ast::Expr) -> bool {
        self.discarded.contains(&(expr as *const ast::Expr as usize))
    }

    /// Does the list written at `expr` start out owned?
    pub fn fresh(&self, expr: &ast::Expr) -> bool {
        self.fresh.contains(&(expr as *const ast::Expr as usize))
    }

    /// Is `expr` an owned value that the statement around it owns?
    pub fn lent(&self, expr: &ast::Expr) -> bool {
        self.lent.contains(&(expr as *const ast::Expr as usize))
    }

    pub fn class(&self, decl: &ast::Class) -> Option<&Vec<Kind>> {
        self.classes.get(&(decl as *const ast::Class as usize))
    }

    /// The size of each field of the class, for the ones that have one:
    /// `level: u8` is stored in a byte.
    pub fn widths(&self, decl: &ast::Class) -> Option<&Vec<Option<Width>>> {
        self.widths.get(&(decl as *const ast::Class as usize))
    }

    pub fn enumeration(&self, decl: &ast::Enum) -> Option<&Vec<Vec<Kind>>> {
        self.enums.get(&(decl as *const ast::Enum as usize))
    }

    /// The copies of a generic function, in the order they were first needed.
    pub fn function_copies(&self, decl: &Arc<ast::Function>) -> &[Arc<ast::Function>] {
        self.function_copies.get(&(Arc::as_ptr(decl) as usize)).map_or(&[], Vec::as_slice)
    }

    /// The copies of a generic class.
    pub fn class_copies(&self, decl: &ast::Class) -> &[Arc<ast::Class>] {
        self.class_copies.get(&(decl as *const ast::Class as usize)).map_or(&[], Vec::as_slice)
    }

    /// If `call` calls a generic function or builds an object of a generic
    /// class, the name of the copy it means: `first[int]`.
    pub fn copy_called(&self, call: &ast::Expr) -> Option<&str> {
        self.copy_called.get(&(call as *const ast::Expr as usize)).map(String::as_str)
    }

    /// What the type written at `ty` stands for, if the checker got as far
    /// as reading it.
    pub fn annotation(&self, ty: &ast::Type) -> Option<&Kind> {
        self.annotations.get(&(ty as *const ast::Type as usize))
    }

    pub fn function(&self, decl: &Arc<ast::Function>) -> Option<&FunctionKind> {
        self.functions.get(&(Arc::as_ptr(decl) as usize))
    }

    /// If `expr` is a variable read where a check has narrowed it (`user`
    /// inside `if user is not none { ... }`), the type it has there.
    pub fn narrowed(&self, expr: &ast::Expr) -> Option<&Kind> {
        self.narrowed.get(&(expr as *const ast::Expr as usize))
    }
}

fn kind(ty: &Ty) -> Kind {
    kind_within(ty, 0)
}

/// `kind`, counting how many function types deep it has gone. A function
/// can return itself, and its type would then never end.
fn kind_within(ty: &Ty, depth: usize) -> Kind {
    let function = |params: Vec<&Ty>, returns: &Ty| {
        Kind::Function(Box::new(FunctionKind {
            params: params.into_iter().map(|param| kind_within(param, depth + 1)).collect(),
            returns: kind_within(returns, depth + 1),
        }))
    };
    match ty {
        Ty::Optional(inner) => Kind::Optional(Box::new(kind_within(inner, depth))),
        Ty::List(item) => Kind::List(Box::new(kind_within(item, depth))),
        Ty::Owned(inner) => Kind::Owned(Box::new(kind_within(inner, depth))),
        Ty::Instance(class) if class.owned => Kind::Owned(Box::new(Kind::Class(class.name.clone()))),
        // Looking at a value is not owning it.
        Ty::Borrowed(inner) => match kind_within(inner, depth) {
            Kind::Owned(inner) => *inner,
            inner => inner,
        },
        Ty::Error(_) => Kind::Error,
        Ty::Callable(callable) if depth < 4 => function(callable.params.iter().collect(), &callable.returns),
        Ty::Function(info) if depth < 4 && info.self_type.is_none() => match &*info.state.borrow() {
            State::Checked(signature) | State::Checking(signature) => {
                function(signature.params.iter().map(|slot| &slot.ty).collect(), &signature.returns)
            }
            State::NotChecked => Kind::Other(ty.to_string()),
        },
        other => plain_kind(other),
    }
}

fn plain_kind(ty: &Ty) -> Kind {
    match ty {
        // A sized number is an ordinary int to the stages after this one.
        // Only a field of an object stores it small: see `Types::widths`.
        Ty::Int | Ty::Sized(_) => Kind::Int,
        Ty::Float => Kind::Float,
        Ty::Str => Kind::Str,
        Ty::Bool => Kind::Bool,
        Ty::Pin => Kind::Pin,
        Ty::None => Kind::None,
        Ty::Quantity(unit) => Kind::Quantity(unit.to_string()),
        Ty::Instance(class) => Kind::Class(class.name.clone()),
        Ty::EnumValue(info) => Kind::Enum(info.name.clone()),
        other => Kind::Other(other.to_string()),
    }
}

/// `check`, also returning what was learned about the program's types.
pub fn check_with_types(program: &Program, strict: bool) -> (Vec<Error>, Types) {
    let strict = strict || program.strict;

    // Parameters and fields with no type written are worked out from how
    // they are used: `add(1, 2)` says both parameters of `add` are ints.
    // One answer can lead to another (if `f` passes its parameter on to
    // `g`, then `g` is only known once `f` is), so the program is checked
    // repeatedly, each pass starting from what the last one learned, until
    // a pass learns nothing new. Only the final pass reports errors.
    let mut known: HashMap<SlotId, Key> = HashMap::new();
    let mut open: HashSet<SlotId> = HashSet::new();
    let mut origins: HashMap<SlotId, usize> = HashMap::new();
    let mut settled = false;
    for _ in 0..MAX_ROUNDS {
        let mut checker = Checker::new(strict, false, known.clone(), open.clone());
        checker.run(program);
        origins = std::mem::take(&mut checker.origins);

        let mut next_known = HashMap::new();
        let mut next_open = open.clone(); // once open, always open
        for (id, seen) in checker.seen {
            match seen {
                Seen::Known(key) if !next_open.contains(&id) => {
                    next_known.insert(id, key);
                }
                Seen::Known(_) => {}
                Seen::Unknowable => {
                    next_open.insert(id);
                }
            }
        }
        next_known.retain(|id, _| !next_open.contains(id));

        settled = next_known == known && next_open == open;
        known = next_known;
        open = next_open;
        if settled {
            break;
        }
    }
    if !settled {
        // Still changing after that many passes. Rather than trust a
        // half-finished answer, treat everything worked out as unknown:
        // unknown is always safe, a wrong guess is not.
        open.extend(known.drain().map(|(id, _)| id));
    }

    let mut checker = Checker::new(strict, true, known, open);
    checker.known_origins = origins;
    checker.run(program);
    let mut errors = checker.errors;
    errors.sort_by_key(|error| (error.span.line, error.span.col));
    // Two copies of a generic can find the same mistake. Once is enough.
    errors.dedup_by(|a, b| a.span == b.span && a.message == b.message);
    (errors, checker.types)
}

// ---------------------------------------------------------------------- Types

/// The type of an expression, as far as the checker can tell.
#[derive(Clone)]
enum Ty {
    /// Not known until the program runs. Accepted everywhere.
    Dynamic,
    /// The type of something that already has an error reported against it.
    /// Accepted everywhere, so the same mistake is not reported twice.
    Invalid,

    Int,
    Float,
    Str,
    Bool,
    None,
    /// A number with a unit, such as `500 ms`.
    Quantity(Rc<str>),
    /// A pin of the board, from the `board` module.
    Pin,
    /// A whole number with a size: `u8`, `i32`. See src/ints.rs.
    Sized(Width),
    List(Box<Ty>),
    /// `T or none`
    Optional(Box<Ty>),
    /// An object of a class.
    Instance(Rc<ClassInfo>),
    /// A value of an enum.
    EnumValue(Rc<EnumInfo>),
    /// A built-in error value, such as a `ValueError`.
    Error(Rc<str>),

    /// `owned list[int]`: a list with exactly one owner. (An object of an
    /// `owned class` is an `Instance` whose class says so.)
    Owned(Box<Ty>),
    /// A look at a value that something else owns: what reading an owner
    /// gives, and the type of a `borrow` parameter. Inside is the plain
    /// type. It fits nowhere that would keep the value, which is the whole
    /// point: only the places that just look take the wrapper off.
    Borrowed(Box<Ty>),

    /// A function known only by its written type: `function(int) -> int`
    Callable(Rc<CallableType>),

    // Things that can be called.
    Function(Rc<FunctionInfo>),
    /// A method read from an object: `self` is already supplied.
    BoundMethod(Rc<FunctionInfo>),
    Builtin(&'static str),
    /// A method of a str or a list, with the type it was read from.
    BuiltinMethod(Box<Ty>, Rc<str>),
    Class(Rc<ClassInfo>),
    Enum(Rc<EnumInfo>),
    /// An enum case that still needs its values: `Circle`
    CaseConstructor(Rc<EnumInfo>, usize),
    ErrorType(Rc<str>),

    Module(&'static str),

    // Generics. See the section "Generic functions and classes".
    /// A function or a class with type parameters, by its name: `first`,
    /// `Stack`. It is not a type yet, and not something that can run:
    /// each use of it picks a copy.
    Generic(Rc<Generic>),
    /// A type parameter that has not been filled in: the `T` of
    /// `items: list[T]`, as the generic itself is written. Only seen
    /// while working out which copy a call means.
    Param(Rc<str>),
    /// A generic class given types that are not all filled in yet:
    /// `Stack[T]`, written in the parameters of a generic function.
    Applied(Rc<Generic>, Vec<Ty>),
    /// The name of a type parameter, inside a copy: there `T` is another
    /// name for the type the copy was made for.
    Alias(Box<Ty>),
}

/// A function or a class with type parameters: `function first[T](...)`,
/// `class Stack[T] { ... }`.
struct Generic {
    /// Where its declaration sits in the syntax tree. The compilers ask
    /// for its copies by this.
    at: usize,
    id: DeclId,
    name: String,
    /// The names of its type parameters: `T`.
    params: Vec<String>,
    template: Template,
    env: Env,
    span: Span,
    above: Rc<[String]>,
    /// Its parameters and what it returns, with the type parameters still
    /// in them. Worked out the first time it is called.
    outline: RefCell<Option<Rc<Outline>>>,
}

/// What a copy of a generic is copied from.
enum Template {
    Function(Arc<ast::Function>),
    Class(Rc<ast::Class>),
}

/// A generic as a call sees it before the types are known: `items:
/// list[T]` and `-> T`.
struct Outline {
    slots: Vec<Slot>,
    returns: Option<Ty>,
}

/// One copy of a generic.
#[derive(Clone)]
enum Made {
    Function(Rc<FunctionInfo>),
    Class(Rc<ClassInfo>),
}

struct CallableType {
    params: Vec<Ty>,
    returns: Ty,
}

/// Names a declaration: where it starts in the file, and what it is called.
/// Types from one checking pass are carried to the next by this, since the
/// passes do not share any of their working data.
type DeclId = (usize, usize, String);

/// Names one parameter or field: the declaration it belongs to, and its name.
type SlotId = (DeclId, String);

struct ClassInfo {
    id: DeclId,
    name: String,
    /// Declared `owned class`: every object of it has exactly one owner.
    owned: bool,
    fields: RefCell<Vec<Slot>>,
    methods: RefCell<HashMap<String, Rc<FunctionInfo>>>,
    /// As for a function (`FunctionInfo::above`): the defaults of the
    /// fields are worked out when an object is built, and may use these.
    above: Rc<[String]>,
    /// The scope the class is declared in.
    env: Env,
    /// Each field that has a default, with the names the default mentions.
    defaults: Vec<(String, Vec<String>)>,
    /// For a copy of a generic class, the generic and the types this copy
    /// was made for: `Stack` and `[int]`.
    origin: Option<(Rc<Generic>, Vec<Ty>)>,
    /// For a copy, the words that say so under an error found inside it.
    copy_note: Option<String>,
}

struct EnumInfo {
    id: DeclId,
    name: String,
    cases: RefCell<Vec<CaseInfo>>,
}

struct CaseInfo {
    name: String,
    fields: Vec<Slot>,
}

/// A named place for an argument: a parameter, a class field, an enum case field.
#[derive(Clone)]
struct Slot {
    name: String,
    ty: Ty,
    has_default: bool,
    /// Set when no type was written, so that its type is being worked out
    /// from the values that arrive here.
    id: Option<SlotId>,
}

/// A type in a form that outlives one checking pass. It is `Ty` without
/// the things that only exist during a pass.
#[derive(Clone, PartialEq, Debug)]
enum Key {
    Int,
    Float,
    Str,
    Bool,
    Pin,
    /// A sized whole number, by its name: `u8`.
    Sized(&'static str),
    None,
    Quantity(String),
    List(Box<Key>),
    Optional(Box<Key>),
    Class(DeclId),
    /// A copy of a generic class: the generic, and the types it was made for.
    Copy(DeclId, Vec<Key>),
    Enum(DeclId),
    Error(String),
}

/// What has been learned so far about a parameter or field with no type.
enum Seen {
    /// Every value that arrives here has this type.
    Known(Key),
    /// The values differ, or one of them is itself unknown until run time.
    Unknowable,
}

/// The one key that covers both, if there is one.
fn join_keys(a: &Key, b: &Key) -> Option<Key> {
    match (a, b) {
        _ if a == b => Some(a.clone()),
        // A parameter that is handed a u8 by one call and an int by
        // another is an int.
        (Key::Sized(_), Key::Int | Key::Sized(_)) | (Key::Int, Key::Sized(_)) => Some(Key::Int),
        (Key::None, Key::Optional(_)) => Some(b.clone()),
        (Key::Optional(_), Key::None) => Some(a.clone()),
        (Key::None, other) | (other, Key::None) => Some(Key::Optional(Box::new(other.clone()))),
        (Key::Optional(inner), other) | (other, Key::Optional(inner)) if **inner == *other => {
            Some(Key::Optional(inner.clone()))
        }
        _ => None,
    }
}

/// What a caller needs to know about a function.
struct Signature {
    name: String,
    params: Vec<Slot>,
    returns: Ty,
    raises: Vec<String>,
}

/// A function and where it was defined. Its signature is worked out the
/// first time it is needed, because a return type that is not written down
/// has to be found by checking the body.
struct FunctionInfo {
    decl: Arc<ast::Function>,
    env: Env,
    span: Span,
    /// For a method, the type of `self`.
    self_type: Option<Ty>,
    state: RefCell<State>,
    /// The variables its block declares above the line it is written on.
    /// The function may use them, so it must not run before they exist
    /// (see `Early`).
    above: Rc<[String]>,
    /// For a copy of a generic function, or a method of a copy of a
    /// generic class, the words that say so under an error found inside it.
    copy_note: Option<String>,
}

/// A function whose body is being checked, for catching a call that
/// comes too early.
///
/// A function can be called from above the line it is written on, and
/// its body is checked at the first place it is used. If that is above a
/// variable the function uses, the variable is not declared yet, so a
/// name in the body would be taken for a variable of the same name
/// further out. When the program ran, the name would mean the variable
/// of the function's own block, which has no value at that moment
/// (decision 3.6d).
struct Early {
    name: String,
    /// The function body being checked, and the one it is written in.
    body: usize,
    outer: usize,
    /// The variables above the function that are not declared yet.
    missing: Vec<String>,
    /// The ones of those that have been reported, so that each is
    /// reported once.
    told: Vec<String>,
}

#[derive(Clone)]
enum State {
    NotChecked,
    /// The body is being checked right now. A call that arrives in this
    /// state is recursion, and sees only what the signature declares.
    Checking(Rc<Signature>),
    Checked(Rc<Signature>),
}

impl Ty {
    fn list(item: Ty) -> Ty {
        Ty::List(Box::new(item))
    }

    /// True for the two types that are accepted everywhere.
    fn is_loose(&self) -> bool {
        matches!(self, Ty::Dynamic | Ty::Invalid)
    }

    fn is_number(&self) -> bool {
        matches!(self, Ty::Int | Ty::Float)
    }

    /// Does this type have a `Dynamic` anywhere inside it?
    fn has_dynamic(&self) -> bool {
        match self {
            Ty::Dynamic => true,
            Ty::List(inner) | Ty::Optional(inner) | Ty::Owned(inner) | Ty::Borrowed(inner) => inner.has_dynamic(),
            _ => false,
        }
    }

    /// Is this a value with exactly one owner?
    fn is_owned(&self) -> bool {
        match self {
            Ty::Owned(_) => true,
            Ty::Instance(class) => class.owned,
            // `Box[T]`, in the parameters of a generic, where `Box` is an
            // `owned class`.
            Ty::Applied(generic, _) => matches!(&generic.template, Template::Class(class) if class.owned),
            _ => false,
        }
    }

    /// Owned, or a look at something owned: a value that cannot be shared.
    fn is_tied(&self) -> bool {
        self.is_owned() || matches!(self, Ty::Borrowed(_))
    }

    /// The type as it is to somebody who only looks: an `owned list[int]`
    /// is read, indexed and printed exactly like a `list[int]`.
    fn looked_at(self) -> Ty {
        match self {
            Ty::Owned(inner) | Ty::Borrowed(inner) => *inner,
            other => other,
        }
    }

    /// The type as arithmetic sees it. Sums, comparisons, indexes and the
    /// built-in functions all work on plain ints, so a `u8` that takes
    /// part in one counts as an `int`, and the answer is an `int`.
    fn widened(&self) -> Ty {
        match self {
            Ty::Sized(_) => Ty::Int,
            other => other.clone(),
        }
    }

    /// `widened`, all the way down: a `list[u8]` holds the same values
    /// as a `list[int]`, which is what `==` and `in` ask about.
    fn as_ints(&self) -> Ty {
        match self {
            Ty::Sized(_) => Ty::Int,
            Ty::List(inner) => Ty::list(inner.as_ints()),
            Ty::Optional(inner) => Ty::Optional(Box::new(inner.as_ints())),
            other => other.clone(),
        }
    }

    /// Can a value of this type be `none`?
    fn accepts_none(&self) -> bool {
        matches!(self, Ty::None | Ty::Optional(_)) || self.is_loose()
    }
}

/// How a type is written in an error message.
impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Ty::Dynamic | Ty::Invalid => write!(f, "any"),
            Ty::Int => write!(f, "int"),
            Ty::Float => write!(f, "float"),
            Ty::Str => write!(f, "str"),
            Ty::Bool => write!(f, "bool"),
            Ty::Pin => write!(f, "Pin"),
            Ty::Sized(width) => write!(f, "{}", width.name),
            Ty::None => write!(f, "none"),
            Ty::Quantity(unit) => write!(f, "a quantity in {unit}"),
            Ty::List(item) => write!(f, "list[{item}]"),
            Ty::Optional(inner) => write!(f, "{inner} or none"),
            Ty::Owned(inner) => write!(f, "owned {inner}"),
            Ty::Borrowed(inner) => write!(f, "a borrowed {inner}"),
            Ty::Instance(class) => write!(f, "{}", class.name),
            Ty::EnumValue(info) => write!(f, "{}", info.name),
            Ty::Error(kind) => write!(f, "{kind}"),
            Ty::Callable(callable) => {
                let params: Vec<String> = callable.params.iter().map(Ty::to_string).collect();
                write!(f, "function({})", params.join(", "))?;
                match &callable.returns {
                    Ty::None => Ok(()),
                    returns => write!(f, " -> {returns}"),
                }
            }
            Ty::Function(_) | Ty::BoundMethod(_) | Ty::Builtin(_) | Ty::BuiltinMethod(..) => {
                write!(f, "a function")
            }
            Ty::Class(class) => write!(f, "the class {}", class.name),
            Ty::Enum(info) => write!(f, "the enum {}", info.name),
            Ty::CaseConstructor(info, index) => {
                write!(f, "the enum case {}", info.cases.borrow()[*index].name)
            }
            Ty::ErrorType(kind) => write!(f, "the error type {kind}"),
            Ty::Module(name) => write!(f, "the module {name}"),
            Ty::Generic(generic) => match generic.template {
                Template::Function(_) => write!(f, "the generic function {}", generic.name),
                Template::Class(_) => write!(f, "the generic class {}", generic.name),
            },
            Ty::Param(name) => write!(f, "{name}"),
            Ty::Applied(generic, types) => {
                let types: Vec<String> = types.iter().map(spelled).collect();
                write!(f, "{}[{}]", generic.name, types.join(", "))
            }
            Ty::Alias(inner) => write!(f, "the type {inner}"),
        }
    }
}

/// A type as it is spelled inside the name of a copy: `Stack[ms]`,
/// `first[list[int]]`. The same as in a message, but for a quantity,
/// which a message calls "a quantity in ms".
fn spelled(ty: &Ty) -> String {
    match ty {
        Ty::Quantity(unit) => unit.to_string(),
        Ty::List(item) => format!("list[{}]", spelled(item)),
        Ty::Optional(inner) => format!("{} or none", spelled(inner)),
        other => other.to_string(),
    }
}

/// Is there a type parameter in `ty` that has not been filled in?
fn has_param(ty: &Ty) -> bool {
    mentions(ty, None)
}

/// Is the type parameter `name` in `ty`? With no name, is any?
fn mentions(ty: &Ty, name: Option<&str>) -> bool {
    match ty {
        Ty::Param(param) => name.is_none_or(|name| name == param.as_ref()),
        Ty::List(inner) | Ty::Optional(inner) | Ty::Owned(inner) | Ty::Borrowed(inner) => mentions(inner, name),
        Ty::Applied(_, types) => types.iter().any(|ty| mentions(ty, name)),
        Ty::Callable(callable) => {
            callable.params.iter().any(|ty| mentions(ty, name)) || mentions(&callable.returns, name)
        }
        _ => false,
    }
}

/// Are these the same type?
fn same(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Int, Ty::Int)
        | (Ty::Float, Ty::Float)
        | (Ty::Str, Ty::Str)
        | (Ty::Bool, Ty::Bool)
        | (Ty::Pin, Ty::Pin)
        | (Ty::None, Ty::None) => true,
        (Ty::Quantity(a), Ty::Quantity(b)) => a == b,
        (Ty::Sized(a), Ty::Sized(b)) => a == b,
        (Ty::Param(a), Ty::Param(b)) => a == b,
        (Ty::List(a), Ty::List(b))
        | (Ty::Optional(a), Ty::Optional(b))
        | (Ty::Owned(a), Ty::Owned(b))
        | (Ty::Borrowed(a), Ty::Borrowed(b)) => same(a, b),
        (Ty::Instance(a), Ty::Instance(b)) => Rc::ptr_eq(a, b),
        (Ty::EnumValue(a), Ty::EnumValue(b)) => Rc::ptr_eq(a, b),
        (Ty::Error(a), Ty::Error(b)) => a == b,
        (Ty::Callable(a), Ty::Callable(b)) => {
            a.params.len() == b.params.len()
                && a.params.iter().zip(&b.params).all(|(a, b)| same(a, b))
                && same(&a.returns, &b.returns)
        }
        _ => false,
    }
}

/// May a value of type `from` go where a `to` is wanted?
fn fits(from: &Ty, to: &Ty) -> bool {
    if from.is_loose() || to.is_loose() || same(from, to) {
        return true;
    }
    match (from, to) {
        // Lending. A `borrow` parameter takes a look at an owned value, or
        // a shared value, which anyone may look at. It does not take an
        // owned value that has no owner yet: nobody would ever free it.
        (Ty::Borrowed(from), Ty::Borrowed(to)) => fits(from, to),
        (from, Ty::Borrowed(to)) => !from.is_owned() && fits(from, to),
        (Ty::Borrowed(_), _) => false,
        (Ty::Owned(from), Ty::Owned(to)) => fits(from, to),
        // `none` and a plain T both fit `T or none`.
        (Ty::None, Ty::Optional(_)) => true,
        (from, Ty::Optional(inner)) => fits(from, inner),
        // A `list[u8]` is not a `list[int]`: whoever holds it as a list of
        // ints could append 300, and the first holder would then find a
        // u8 that is not one.
        (Ty::List(from), Ty::List(to)) => !sized_into_int(from, to) && fits(from, to),
        // Every u8 is an int, so a sized number goes where an int is
        // wanted and nothing has to be written. The other way round needs
        // a conversion, because not every int is a u8.
        (Ty::Sized(_), Ty::Int) => true,
        // Any error fits the catch-all `Error`.
        (Ty::Error(_), Ty::Error(to)) => &**to == "Error",
        // A function going where a function type is wanted is compared in
        // detail by `Checker::expect`, which can look inside it.
        (
            Ty::Function(_) | Ty::BoundMethod(_) | Ty::Builtin(_) | Ty::BuiltinMethod(..),
            Ty::Callable(_),
        ) => true,
        _ => false,
    }
}

/// Is this a sized number (perhaps inside lists) going where the same
/// shape of `int` is wanted?
fn sized_into_int(from: &Ty, to: &Ty) -> bool {
    match (from, to) {
        (Ty::Sized(_), Ty::Int) => true,
        (Ty::List(from), Ty::List(to)) | (Ty::Optional(from), Ty::Optional(to)) => sized_into_int(from, to),
        (from, Ty::Optional(to)) => sized_into_int(from, to),
        _ => false,
    }
}

/// A whole number, or a list of them, as it is written in the program:
/// `7`, `-1`, `[0x10, 0x20]`. Its value is known while checking, so it
/// can go where a sized number is wanted without a conversion: `let b: u8
/// = 7` is checked here, once, and costs nothing when the program runs.
#[derive(Clone)]
enum Written {
    Number(i128),
    List(Vec<Written>),
}

fn written(expr: &Expr) -> Option<Written> {
    match &expr.kind {
        ExprKind::Int(n) => Some(Written::Number(*n as i128)),
        ExprKind::Unary { op: UnaryOp::Neg, operand } => match written(operand)? {
            Written::Number(n) => Some(Written::Number(-n)),
            Written::List(_) => None,
        },
        ExprKind::List(items) if !items.is_empty() => {
            items.iter().map(written).collect::<Option<Vec<_>>>().map(Written::List)
        }
        _ => None,
    }
}

/// Does what is written fit where a `to` is wanted? `None` if `to` asks
/// for no sized number here, and the ordinary rules decide. Otherwise
/// yes, or the message for the first number that is too big.
fn written_fits(written: &Written, to: &Ty) -> Option<Result<(), String>> {
    match (written, to) {
        (Written::Number(n), Ty::Sized(width)) => {
            let fits = i64::try_from(*n).is_ok_and(|n| width.holds(n));
            Some(if fits { Ok(()) } else { Err(width.does_not_fit(n)) })
        }
        (_, Ty::Optional(inner) | Ty::Owned(inner) | Ty::Borrowed(inner)) => written_fits(written, inner),
        (Written::List(items), Ty::List(item)) => {
            for written in items {
                if let Err(message) = written_fits(written, item)? {
                    return Some(Err(message));
                }
            }
            Some(Ok(()))
        }
        _ => None,
    }
}

/// The type `from` with its ints read as the sized numbers `to` wants:
/// what `[1, 2]` is, written where a `list[u8]` goes.
fn sized_as(from: &Ty, to: &Ty) -> Ty {
    match (from, to) {
        (Ty::Int, Ty::Sized(_)) => to.clone(),
        (Ty::List(from), Ty::List(to)) => Ty::list(sized_as(from, to)),
        (Ty::Owned(from), Ty::Owned(to)) => Ty::Owned(Box::new(sized_as(from, to))),
        (from, Ty::Optional(to) | Ty::Borrowed(to) | Ty::Owned(to)) => sized_as(from, to),
        _ => from.clone(),
    }
}

/// The one type that covers both `a` and `b`: used for the items of a list
/// and the different `return`s of a function.
fn join(a: &Ty, b: &Ty) -> Ty {
    if same(a, b) {
        return a.clone();
    }
    match (a, b) {
        (Ty::Invalid, _) | (_, Ty::Invalid) => Ty::Invalid,
        (Ty::None, other) | (other, Ty::None) => match other {
            Ty::Optional(_) => other.clone(),
            _ => Ty::Optional(Box::new(other.clone())),
        },
        (Ty::Optional(inner), other) | (other, Ty::Optional(inner)) => {
            Ty::Optional(Box::new(join(inner, other)))
        }
        (Ty::List(a), Ty::List(b)) => Ty::list(join(a, b)),
        // A u8 beside an int, or beside an i16: what covers both is an int.
        (Ty::Sized(_), Ty::Int | Ty::Sized(_)) | (Ty::Int, Ty::Sized(_)) => Ty::Int,
        // Unrelated types: only known when the program runs.
        _ => Ty::Dynamic,
    }
}

/// The names in `above` that `env` has not declared yet.
fn not_yet(above: &[String], env: &Env) -> Vec<String> {
    above.iter().filter(|name| env.get_here(name).is_none()).cloned().collect()
}

/// Every name an expression mentions, for `ClassInfo::defaults`. It goes
/// by name alone and does not look inside a function written as a value.
fn names_in(expr: &Expr, names: &mut Vec<String>) {
    match &expr.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::None => {}
        ExprKind::Function(_) => {}
        ExprKind::Name(name) => names.push(name.clone()),
        ExprKind::FormatStr(parts) => {
            for part in parts {
                if let FormatPart::Value { value, .. } = part {
                    names_in(value, names);
                }
            }
        }
        ExprKind::Unit { value, .. } => names_in(value, names),
        ExprKind::List(items) => items.iter().for_each(|item| names_in(item, names)),
        ExprKind::ListComp { element, iterable, condition, .. } => {
            names_in(element, names);
            names_in(iterable, names);
            if let Some(condition) = condition {
                names_in(condition, names);
            }
        }
        ExprKind::Unary { operand, .. } => names_in(operand, names),
        ExprKind::Binary { left, right, .. } => {
            names_in(left, names);
            names_in(right, names);
        }
        ExprKind::Compare { first, rest } => {
            names_in(first, names);
            rest.iter().for_each(|(_, operand)| names_in(operand, names));
        }
        ExprKind::Call { callee, args } => {
            names_in(callee, names);
            args.iter().for_each(|arg| names_in(&arg.value, names));
        }
        ExprKind::Field { object, .. } => names_in(object, names),
        ExprKind::Index { object, index } => {
            names_in(object, names);
            names_in(index, names);
        }
    }
}

// --------------------------------------------------------------------- Scopes

/// The checker's scopes mirror the interpreter's, holding types in place of values.
struct Scope {
    variables: RefCell<HashMap<String, Variable>>,
    parent: Option<Env>,
    /// Which function body this scope is part of. An owned value may only
    /// be used by the function whose block it lives in.
    function: usize,
}

/// A number nothing else has: for telling variables, and function bodies,
/// apart. Later ones are larger, which says what was declared before what.
fn fresh_id() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

type Env = Rc<Scope>;

#[derive(Clone)]
struct Variable {
    ty: Ty,
    mutable: bool,
    /// Set for a bare enum case such as `Idle`: which case it is.
    case: Option<usize>,
    /// Set when `ty` is narrower than the type the variable was declared
    /// with, after a check such as `is not none`. This is the declared type.
    declared: Option<Ty>,
    /// A scope inside the one that declares a variable can hold its own
    /// view of it, to record what is known about it in there. This marks
    /// such a view, as opposed to the declaration itself.
    view: bool,
    /// Which variable this is, whatever it is called. See `fresh_id`.
    id: usize,
    /// The function body it was declared in.
    function: usize,
}

impl Scope {
    fn child(parent: &Env) -> Env {
        Rc::new(Scope {
            variables: RefCell::new(HashMap::new()),
            parent: Some(parent.clone()),
            function: parent.function,
        })
    }

    /// The outermost scope of a function's body.
    fn body(parent: &Env) -> Env {
        Rc::new(Scope { variables: RefCell::new(HashMap::new()), parent: Some(parent.clone()), function: fresh_id() })
    }

    fn declare(&self, name: &str, ty: Ty, mutable: bool) -> bool {
        let mut variables = self.variables.borrow_mut();
        if variables.contains_key(name) {
            return false;
        }
        let variable =
            Variable { ty, mutable, case: None, declared: None, view: false, id: fresh_id(), function: self.function };
        variables.insert(name.to_string(), variable);
        true
    }

    fn get(&self, name: &str) -> Option<Variable> {
        if let Some(variable) = self.variables.borrow().get(name) {
            return Some(variable.clone());
        }
        self.parent.as_ref()?.get(name)
    }

    fn get_here(&self, name: &str) -> Option<Variable> {
        self.variables.borrow().get(name).cloned()
    }

    /// Record that, from here on in this scope, `name` is known to have a
    /// narrower type: for example `str` instead of `str or none` after a check.
    fn narrow(&self, name: &str, ty: Ty) {
        if let Some(mut variable) = self.get(name) {
            variable.view = self.get_here(name).map_or(true, |here| here.view);
            if variable.declared.is_none() {
                variable.declared = Some(variable.ty.clone());
            }
            variable.ty = ty;
            self.variables.borrow_mut().insert(name.to_string(), variable);
        }
    }

    /// Give this scope a view of `name` that knows nothing beyond its
    /// declared type, whatever the scopes around it have worked out.
    fn forget(&self, name: &str) {
        if let Some(mut variable) = self.get(name) {
            if let Some(declared) = variable.declared.take() {
                variable.ty = declared;
                variable.view = true;
                self.variables.borrow_mut().insert(name.to_string(), variable);
            }
        }
    }

    /// `name` has just been given a value that may be `none`. Drop what
    /// checks have said about it, in this scope and in every scope around
    /// it, out to the one that declares it.
    fn widen(&self, name: &str) {
        let mut view = true;
        if let Some(variable) = self.variables.borrow_mut().get_mut(name) {
            if let Some(declared) = variable.declared.take() {
                variable.ty = declared;
            }
            view = variable.view;
        }
        if let (true, Some(parent)) = (view, &self.parent) {
            parent.widen(name);
        }
    }
}

/// The outermost scope: the built-in functions and error types.
fn globals() -> Env {
    let env = Rc::new(Scope { variables: RefCell::new(HashMap::new()), parent: None, function: 0 });
    for name in ["print", "len", "range", "int", "float", "str", "abs", "min", "max", "sum"] {
        env.declare(name, Ty::Builtin(name), false);
    }
    // The sized whole numbers: `u8(n)` converts, and `u8` is also a type.
    for width in ints::WIDTHS {
        env.declare(width.name, Ty::Builtin(width.name), false);
    }
    for kind in ["Error", "ValueError", "IndexError", "ZeroDivisionError", "OverflowError"] {
        env.declare(kind, Ty::ErrorType(Rc::from(kind)), false);
    }
    env
}

/// The type of `module.name`, for the built-in modules.
fn module_member(module: &str, name: &str) -> Option<Ty> {
    Some(match (module, name) {
        ("math", "pi") => Ty::Float,
        ("math", "sqrt") => Ty::Builtin("sqrt"),
        ("math", "floor") => Ty::Builtin("floor"),
        ("math", "ceil") => Ty::Builtin("ceil"),
        ("math", "sin") => Ty::Builtin("sin"),
        ("math", "cos") => Ty::Builtin("cos"),
        ("time", "sleep") => Ty::Builtin("sleep"),
        ("time", "now") => Ty::Builtin("now"),
        ("board", "led") => Ty::Pin,
        ("board", "pin") => Ty::Builtin("pin"),
        _ => return None,
    })
}

// -------------------------------------------------------------------- Checker

/// What the checker tracks for the function body it is currently inside.
/// The bottom frame stands for the top level of the file.
struct Frame {
    declared_return: Option<Ty>,
    /// The type of every `return` seen so far.
    returns: Vec<Ty>,
    /// How many loops we are inside, for `break` and `continue`.
    loops: usize,
}

/// One argument at a call, with where it is written.
struct Argument {
    name: Option<String>,
    ty: Ty,
    span: Span,
    /// The number, if the argument is one written out: `f(7)`.
    written: Option<Written>,
}

struct Checker {
    strict: bool,
    /// False while types are still being worked out over several passes.
    last_round: bool,
    /// What earlier passes worked out for untyped parameters and fields.
    known: HashMap<SlotId, Key>,
    /// The untyped parameters and fields that cannot be worked out.
    open: HashSet<SlotId>,
    /// What this pass has seen arrive at each untyped parameter and field.
    seen: HashMap<SlotId, Seen>,
    /// The line where each of them was first given a value: by this pass
    /// (`origins`), and by the pass before (`known_origins`).
    origins: HashMap<SlotId, usize>,
    known_origins: HashMap<SlotId, usize>,
    /// For each function body being checked, innermost last: its parameters
    /// whose types were worked out, with the type and the line it came from.
    worked_out: Vec<HashMap<String, (String, usize)>>,
    /// Which of those the statement being checked has mentioned so far.
    touched: Vec<String>,
    /// This pass's classes and enums, to turn a `Key` back into a `Ty`.
    classes: HashMap<DeclId, Rc<ClassInfo>>,
    enums: HashMap<DeclId, Rc<EnumInfo>>,
    /// What this pass worked out, kept for the compiler.
    types: Types,
    errors: Vec<Error>,
    frames: Vec<Frame>,
    /// The function bodies being checked right now, outermost first.
    early: Vec<Early>,
    /// The errors that can be raised by the code checked so far, innermost
    /// `try` body or function body last. Each entry is an error kind and
    /// where it comes from.
    raised: Vec<Vec<(String, Span)>>,
    /// Which owned values have been given away, in the function body being
    /// checked. See the section "Owned values".
    owned: Ownership,
    /// Every generic function and class the pass has met.
    generics: Vec<Rc<Generic>>,
    /// The copies made so far, by the generic and the types: `first` and
    /// `int`. Asking twice gives the same copy.
    copies: HashMap<(usize, String), Made>,
    /// The generics that something in the program uses.
    used: HashSet<usize>,
    /// Copies of classes whose bodies are still to be checked. A copy can
    /// be asked for in the middle of anything, so its body waits here
    /// until the checker is between statements again.
    pending: Vec<(Arc<ast::Class>, Rc<ClassInfo>)>,
    /// A call that is about to be checked, and the type wanted where its
    /// result goes: in `let s: Stack[int] = Stack()`, the call `Stack()`
    /// and `Stack[int]`. A generic that cannot tell its types from its
    /// arguments looks here.
    expected: Option<(usize, Ty)>,
    /// For each function body being checked, innermost last: if it is a
    /// copy of a generic, the words that say which.
    copy_notes: Vec<Option<String>>,
}

/// What the checker tracks about owned values in one function body.
#[derive(Default)]
struct Ownership {
    /// The owners that hold nothing at this point in the code, because
    /// their value was given away: each one's name, and where it went.
    moved: Moved,
    /// The owned and borrowed variables the statement being checked has
    /// looked at so far. None of them may be given away by the same
    /// statement: the look might still be in use.
    looking: Vec<usize>,
    /// For each loop around this point, what had been given away at each
    /// `break` and at each `continue` in it.
    loops: Vec<(Moved, Moved)>,
    /// For each `try` around this point, everything its body has given
    /// away so far. An error can leave the body after any of them.
    tries: Vec<Moved>,
}

type Moved = HashMap<usize, (String, Span)>;

impl Checker {
    fn new(strict: bool, last_round: bool, known: HashMap<SlotId, Key>, open: HashSet<SlotId>) -> Checker {
        Checker {
            strict,
            last_round,
            known,
            open,
            seen: HashMap::new(),
            origins: HashMap::new(),
            known_origins: HashMap::new(),
            worked_out: Vec::new(),
            touched: Vec::new(),
            classes: HashMap::new(),
            enums: HashMap::new(),
            types: Types::default(),
            errors: Vec::new(),
            frames: vec![Frame { declared_return: None, returns: Vec::new(), loops: 0 }],
            raised: vec![Vec::new()],
            owned: Ownership::default(),
            early: Vec::new(),
            generics: Vec::new(),
            copies: HashMap::new(),
            used: HashSet::new(),
            pending: Vec::new(),
            expected: None,
            copy_notes: Vec::new(),
        }
    }

    fn run(&mut self, program: &Program) {
        let env = Scope::child(&globals());
        self.statements(&program.statements, &env);
        self.finish_generics();
    }

    fn error(&mut self, message: impl Into<String>, span: Span) -> Ty {
        let mut error = Error::new(message, span);
        // If this statement used a parameter whose type was worked out from
        // a call, the real mistake may be that call. Say where it is.
        if let Some(params) = self.worked_out.last() {
            let mut notes = Vec::new();
            for name in &self.touched {
                if let Some((ty, line)) = params.get(name) {
                    let note = format!(
                        "`{name}` was worked out to be {ty} from line {line}. \
                         Give `{name}` a type to have each call checked against it instead"
                    );
                    if !notes.contains(&note) {
                        notes.push(note);
                    }
                }
            }
            if !notes.is_empty() {
                error.note = Some(notes.join("\n"));
            }
        }
        // An error inside a copy of a generic depends on the types the
        // copy was made for. Say which copy this is.
        if let Some(Some(copy)) = self.copy_notes.last() {
            error.note = Some(match error.note.take() {
                Some(note) => format!("{note}\n{copy}"),
                None => copy.clone(),
            });
        }
        self.errors.push(error);
        Ty::Invalid
    }

    fn frame(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("there is always a top-level frame")
    }

    fn can_raise(&mut self, kind: &str, span: Span) {
        self.raised.last_mut().expect("there is always a top-level list").push((kind.to_string(), span));
    }

    fn declare(&mut self, env: &Env, name: &str, ty: Ty, mutable: bool, span: Span) {
        if !env.declare(name, ty, mutable) {
            self.error(format!("`{name}` is already declared in this block"), span);
        }
    }

    /// Report an error unless a `from` can go where a `to` is wanted.
    fn expect(&mut self, from: &Ty, to: &Ty, what: &str, span: Span) {
        if !fits(from, to) {
            let mut message = format!("{what} should be {to}, but this is {from}");
            match (from, to) {
                (from, Ty::Borrowed(_)) if from.is_owned() => {
                    message = format!(
                        "this {from} has no owner yet, so it cannot be lent. \
                         Give it a name with `let` first, and pass the name"
                    )
                }
                (Ty::Borrowed(_), _) => {
                    message += ". A value with an owner can only go to a parameter that borrows it \
                                (`borrow`) or takes it over (an owned type)"
                }
                (Ty::List(_), Ty::Owned(_)) => {
                    message += ". Only a list made right here can become an owned one: one written out, \
                                as in `[1, 2]` or `[x for x in xs]`, or what `split`, `map`, `filter` or `+` gives"
                }
                (Ty::Int, Ty::Float) => message += ". Write the number with a decimal point, as in `1.0`",
                (Ty::List(from), Ty::List(to)) if sized_into_int(from, to) => {
                    message += ". The list could then be given a number too big for it. \
                                Make a list of ints from it with `[int(n) for n in ...]`"
                }
                (Ty::Int | Ty::Sized(_), Ty::Sized(width)) => {
                    message += &format!(". Convert it with `{}(...)`, which checks that it fits", width.name)
                }
                (Ty::Optional(_), _) => message += ". Check it with `is not none` first",
                (Ty::Quantity(from), Ty::Quantity(to)) if units::convert(from, to).is_some() => {
                    message += &format!(". Convert it with `.{to}`")
                }
                _ => {}
            }
            self.error(message, span);
            return;
        }
        self.arrives(from, to, what, span);
    }

    /// A number that is written out adapts to the sized type it is put in:
    /// `let b: u8 = 7` needs no `u8(7)`. This gives the type the value has
    /// there, after checking that the number fits. Anything that is not a
    /// written number keeps the type it was given.
    fn adapted(&mut self, written: Option<Written>, given: Ty, wanted: &Ty, span: Span) -> Ty {
        let Some(written) = written else {
            return given;
        };
        match written_fits(&written, wanted) {
            None => given,
            Some(Ok(())) => sized_as(&given, wanted),
            Some(Err(message)) => self.error(message, span),
        }
    }

    /// A value of type `from` goes where a `to` is wanted, and the two fit.
    /// What is left concerns functions, which may sit inside a list or an
    /// `or none`: one going where a function type is wanted is compared with
    /// it in detail, and one going where its type is not tracked `escapes`.
    fn arrives(&mut self, from: &Ty, to: &Ty, what: &str, span: Span) {
        match (from, to) {
            (_, to) if to.is_loose() => self.escapes(from),
            (Ty::List(from), Ty::List(to)) | (Ty::Optional(from), Ty::Optional(to)) => {
                self.arrives(from, to, what, span)
            }
            (from, Ty::Optional(to)) => self.arrives(from, to, what, span),
            (from, Ty::Callable(wanted)) => {
                if let Some(given) = self.callable(from) {
                    self.expect_function(&given, wanted, to, what, span);
                }
            }
            _ => {}
        }
    }

    // ----------------------------------------------- Working out left-out types

    /// The type of a parameter or field: what is written, or failing that,
    /// what has been worked out from how it is used.
    fn slot_type(
        &mut self,
        annotation: &Option<ast::Type>,
        id: &SlotId,
        what: &str,
        env: &Env,
        span: Span,
        parameter: bool,
    ) -> Ty {
        if let Some(annotation) = annotation {
            return self.resolve_for(annotation, env, span, parameter);
        }
        if let Some(key) = self.known.get(id).cloned() {
            return self.from_key(&key);
        }
        if !self.last_round && !self.open.contains(id) {
            // Not settled yet. `Invalid` is quiet and is not counted as
            // evidence about anything it is passed on to.
            return Ty::Invalid;
        }
        if self.strict {
            return self.error(
                format!("strict mode: {what} needs a type. It could not be worked out from how it is used"),
                span,
            );
        }
        Ty::Dynamic
    }

    /// Note that a value of type `ty` arrives at `slot`, and where.
    fn sees(&mut self, slot: &Slot, ty: &Ty, span: Span) {
        let Some(id) = &slot.id else {
            return;
        };
        // Remember the first place that says anything about this slot, so
        // that an error caused by what was worked out can point back to it.
        if !matches!(ty, Ty::Invalid) {
            self.origins.entry(id.clone()).or_insert(span.line);
        }
        let seen = match (self.key(ty), self.seen.remove(id)) {
            (None, previous) => previous, // no evidence either way
            (Some(Ok(key)), None) => Some(Seen::Known(key)),
            (Some(Ok(key)), Some(Seen::Known(old))) => {
                Some(join_keys(&old, &key).map_or(Seen::Unknowable, Seen::Known))
            }
            _ => Some(Seen::Unknowable),
        };
        if let Some(seen) = seen {
            self.seen.insert(id.clone(), seen);
        }
    }

    /// `ty` as a key. `None` means the type is not settled yet and says
    /// nothing; `Some(Err(()))` means it can never be pinned down.
    fn key(&self, ty: &Ty) -> Option<Result<Key, ()>> {
        Some(Ok(match ty {
            Ty::Invalid => return None,
            Ty::Int => Key::Int,
            Ty::Float => Key::Float,
            Ty::Str => Key::Str,
            Ty::Bool => Key::Bool,
            Ty::Pin => Key::Pin,
            Ty::Sized(width) => Key::Sized(width.name),
            Ty::None => Key::None,
            Ty::Quantity(unit) => Key::Quantity(unit.to_string()),
            Ty::Error(kind) => Key::Error(kind.to_string()),
            // Who owns a value is never guessed: it has to be written down.
            Ty::Instance(class) if class.owned => return Some(Err(())),
            // A copy of a generic class is named by what it is a copy of,
            // so that the next pass can make it again.
            Ty::Instance(class) if class.origin.is_some() => {
                let (generic, types) = class.origin.as_ref()?;
                let mut keys = Vec::new();
                for ty in types {
                    match self.key(ty)? {
                        Ok(key) => keys.push(key),
                        Err(()) => return Some(Err(())),
                    }
                }
                Key::Copy(generic.id.clone(), keys)
            }
            Ty::Instance(class) => Key::Class(class.id.clone()),
            Ty::EnumValue(info) => Key::Enum(info.id.clone()),
            Ty::List(inner) => match self.key(inner)? {
                Ok(inner) => Key::List(Box::new(inner)),
                Err(()) => return Some(Err(())),
            },
            Ty::Optional(inner) => match self.key(inner)? {
                Ok(inner) => Key::Optional(Box::new(inner)),
                Err(()) => return Some(Err(())),
            },
            _ => return Some(Err(())),
        }))
    }

    fn from_key(&mut self, key: &Key) -> Ty {
        match key {
            Key::Int => Ty::Int,
            Key::Float => Ty::Float,
            Key::Str => Ty::Str,
            Key::Bool => Ty::Bool,
            Key::Pin => Ty::Pin,
            Key::Sized(name) => ints::width(name).map_or(Ty::Int, Ty::Sized),
            Key::None => Ty::None,
            Key::Quantity(unit) => Ty::Quantity(Rc::from(unit.as_str())),
            Key::Error(kind) => Ty::Error(Rc::from(kind.as_str())),
            Key::List(inner) => Ty::list(self.from_key(inner)),
            Key::Optional(inner) => Ty::Optional(Box::new(self.from_key(inner))),
            Key::Class(id) => self.classes.get(id).map_or(Ty::Dynamic, |class| Ty::Instance(class.clone())),
            Key::Copy(id, keys) => {
                let types: Vec<Ty> = keys.iter().map(|key| self.from_key(key)).collect();
                let generic = self.generics.iter().find(|generic| generic.id == *id).cloned();
                match generic.and_then(|generic| self.copy_class(&generic, types, None)) {
                    Some(class) => Ty::Instance(class),
                    None => Ty::Dynamic,
                }
            }
            Key::Enum(id) => self.enums.get(id).map_or(Ty::Dynamic, |info| Ty::EnumValue(info.clone())),
        }
    }

    /// A function value has gone somewhere its type is not tracked, so it
    /// may be called from there with anything. Its untyped parameters can
    /// no longer be worked out from the calls that are visible.
    fn escapes(&mut self, ty: &Ty) {
        let function = match ty {
            Ty::Function(function) | Ty::BoundMethod(function) => function.clone(),
            Ty::List(inner) | Ty::Optional(inner) => return self.escapes(inner),
            _ => return,
        };
        for slot in &self.signature(&function).params {
            if let Some(id) = &slot.id {
                self.seen.insert(id.clone(), Seen::Unknowable);
            }
        }
    }

    /// `join`, for places where a function might be one of the things joined.
    fn join(&mut self, a: &Ty, b: &Ty) -> Ty {
        let joined = join(a, b);
        if joined.is_loose() {
            self.escapes(a);
            self.escapes(b);
        }
        joined
    }

    fn join_all(&mut self, types: &[Ty]) -> Option<Ty> {
        let (first, rest) = types.split_first()?;
        let mut joined = first.clone();
        for ty in rest {
            joined = self.join(&joined, ty);
        }
        Some(joined)
    }

    /// If `ty` is a function whose details are known, its parameters and
    /// what it returns.
    fn callable(&mut self, ty: &Ty) -> Option<(Vec<Slot>, Ty)> {
        match ty {
            Ty::Function(function) => {
                let signature = self.signature(function);
                Some((signature.params.clone(), signature.returns.clone()))
            }
            Ty::BoundMethod(function) => {
                let signature = self.signature(function);
                Some((signature.params.get(1..).unwrap_or_default().to_vec(), signature.returns.clone()))
            }
            _ => None,
        }
    }

    /// Check a function against a written function type.
    fn expect_function(
        &mut self,
        (params, returns): &(Vec<Slot>, Ty),
        wanted: &CallableType,
        wanted_type: &Ty,
        what: &str,
        span: Span,
    ) {
        let needed = params.iter().filter(|slot| !slot.has_default).count();
        if !(needed..=params.len()).contains(&wanted.params.len()) {
            self.error(
                format!(
                    "{what} should be {wanted_type}, but this function takes {} argument(s)",
                    params.len()
                ),
                span,
            );
            return;
        }
        for (slot, given) in params.iter().zip(&wanted.params) {
            // It will be called with these types, which is evidence for an
            // untyped parameter just as a direct call would be.
            self.sees(slot, given, span);
            if !fits(given, &slot.ty) {
                self.error(
                    format!(
                        "{what} should be {wanted_type}, but the parameter `{}` of this function is {}",
                        slot.name, slot.ty
                    ),
                    span,
                );
                return;
            }
        }
        if !fits(returns, &wanted.returns) {
            self.error(format!("{what} should be {wanted_type}, but this function returns {returns}"), span);
        }
    }

    // ------------------------------------------------------- Type annotations

    /// Turn a type written in the source into a `Ty`.
    fn resolve(&mut self, ty: &ast::Type, env: &Env, span: Span) -> Ty {
        self.resolve_for(ty, env, span, false)
    }

    /// `resolve`, knowing whether the type is a parameter's. `owned` and
    /// `borrow` are words about the whole value, so they are only looked
    /// for here, at the front, and `borrow` only on a parameter.
    fn resolve_for(&mut self, ty: &ast::Type, env: &Env, span: Span, parameter: bool) -> Ty {
        match ty {
            ast::Type::Owned(inner) => match self.resolve_inside(inner, env, span, false) {
                list @ Ty::List(_) => Ty::Owned(Box::new(list)),
                // An object of an `owned class` is owned without saying so again.
                ty if ty.is_owned() || ty.is_loose() => ty,
                ty => self.error(
                    format!(
                        "`owned` goes in front of a list type, as in `owned list[int]`, and this is {ty}. \
                         A class is made owned where it is declared: `owned class`"
                    ),
                    span,
                ),
            },
            ast::Type::Borrow(_) if !parameter => self.error(
                "`borrow` can only be written on a parameter. A borrowed value cannot outlive the call it is lent to",
                span,
            ),
            ast::Type::Borrow(inner) => match self.resolve_inside(inner, env, span, false) {
                ty @ (Ty::List(_) | Ty::Instance(_) | Ty::Applied(..)) => Ty::Borrowed(Box::new(ty)),
                ty if ty.is_loose() => ty,
                ty => self.error(
                    format!("only a list or an object can be borrowed, and this is {ty}. Pass it as it is"),
                    span,
                ),
            },
            _ => self.resolve_inside(ty, env, span, false),
        }
    }

    /// The part of a type after any `owned` or `borrow`. `nested` is true
    /// inside `list[ ]`, `or none` and a function type: places that hold
    /// shared values, where an owned one may not go.
    fn resolve_inside(&mut self, ty: &ast::Type, env: &Env, span: Span, nested: bool) -> Ty {
        let written = ty as *const ast::Type as usize;
        let ty = match ty {
            ast::Type::Owned(_) | ast::Type::Borrow(_) => {
                return self.error(
                    "`owned` and `borrow` describe a whole value, so they go at the very front of a type, \
                     and not inside `list[ ]`, `or none` or a function type",
                    span,
                )
            }
            ast::Type::Optional(inner) => Ty::Optional(Box::new(self.resolve_inside(inner, env, span, true))),
            ast::Type::Function { params, returns } => {
                let params = params.iter().map(|param| self.resolve_inside(param, env, span, true)).collect();
                let returns = match returns {
                    Some(returns) => self.resolve_inside(returns, env, span, true),
                    None => Ty::None,
                };
                Ty::Callable(Rc::new(CallableType { params, returns }))
            }
            ast::Type::Named { name, args } => {
                let ty = match (name.as_str(), args.as_slice()) {
                    ("int", []) => Ty::Int,
                    ("float", []) => Ty::Float,
                    ("str", []) => Ty::Str,
                    ("bool", []) => Ty::Bool,
                    ("list", [item]) => Ty::list(self.resolve_inside(item, env, span, true)),
                    ("list", _) => {
                        return self.error("`list` takes one type, as in `list[int]`", span)
                    }
                    // `any` asks for a dynamic value on purpose.
                    ("any", []) if self.strict => {
                        return self.error("`any` is not allowed in strict mode", span)
                    }
                    ("any", []) => Ty::Dynamic,
                    // `Stack[int]`: the copy of a generic class for those types.
                    (_, written_types) if matches!(env.get(name), Some(Variable { ty: Ty::Generic(_), .. })) => {
                        let Some(Variable { ty: Ty::Generic(generic), .. }) = env.get(name) else {
                            return Ty::Invalid;
                        };
                        let example = format!("{name}[{}]", vec!["int"; generic.params.len()].join(", "));
                        if matches!(generic.template, Template::Function(_)) {
                            return self.error(format!("`{name}` is a function, and this place needs a type"), span);
                        }
                        if written_types.len() != generic.params.len() {
                            return self.error(
                                format!(
                                    "`{name}` takes {} type(s) in `[ ]`, as in `{example}`, but {} were written",
                                    generic.params.len(),
                                    written_types.len()
                                ),
                                span,
                            );
                        }
                        let types: Vec<Ty> =
                            written_types.iter().map(|ty| self.resolve_inside(ty, env, span, true)).collect();
                        if types.iter().any(has_param) {
                            // Still a pattern: the `Stack[T]` of a generic
                            // function's own parameters.
                            Ty::Applied(generic, types)
                        } else if types.iter().any(|ty| matches!(ty, Ty::Invalid)) {
                            Ty::Invalid
                        } else {
                            match self.copy_class(&generic, types, Some(span)) {
                                Some(class) => Ty::Instance(class),
                                None => Ty::Invalid,
                            }
                        }
                    }
                    (_, []) => match env.get(name).map(|variable| variable.ty) {
                        // Inside a copy, `T` is the type it was made for.
                        Some(Ty::Alias(ty)) => *ty,
                        Some(Ty::Class(class)) => Ty::Instance(class),
                        Some(Ty::Enum(info)) => Ty::EnumValue(info),
                        Some(Ty::ErrorType(kind)) => Ty::Error(kind),
                        // `u8` the type shares its name with `u8( )` the
                        // conversion, which is how it is found here.
                        Some(Ty::Builtin(name)) if ints::width(name).is_some() => {
                            ints::width(name).map_or(Ty::Int, Ty::Sized)
                        }
                        // A unit is the type of a quantity measured in it.
                        _ if units::is_unit(name) => Ty::Quantity(Rc::from(name.as_str())),
                        // A pin of the board. A class of the program's own
                        // called `Pin` was found above, and comes first.
                        _ if name == "Pin" => Ty::Pin,
                        _ if name == "i64" || name == "u64" => {
                            return self.error(
                                format!(
                                    "there is no type called `{name}`. An `int` is 64 bits with a sign, \
                                     and the sized whole numbers are u8, u16, u32, i8, i16 and i32"
                                ),
                                span,
                            )
                        }
                        _ => return self.error(format!("there is no type called `{name}`"), span),
                    },
                    _ => return self.error(format!("`{name}` does not take types in `[ ]`"), span),
                };
                ty
            }
        };
        if nested && ty.is_owned() {
            // A shared value can be reached from many places, and each of
            // them would think it could still use the owned one inside.
            return self.error(
                format!(
                    "an owned {ty} cannot be kept inside a list, an `or none` or a function type yet. \
                     It lives in a variable, a parameter, or a field of an owned class"
                ),
                span,
            );
        }
        // The compilers ask what a written type stands for by where it is
        // written. Inside a copy of a generic only the checker knows.
        if !has_param(&ty) {
            self.types.annotations.insert(written, kind(&ty));
        }
        ty
    }

    // ------------------------------------------------------------ Statements

    /// Check a `{ }` block. Returns true if control can never reach the end
    /// of it, because every path returns, raises, breaks or continues.
    fn block(&mut self, statements: &[Stmt], env: &Env) -> bool {
        self.statements(statements, &Scope::child(env))
    }

    fn statements(&mut self, statements: &[Stmt], env: &Env) -> bool {
        self.hoist(statements, env);
        let mut exits = false;
        for statement in statements {
            exits |= self.stmt(statement, env);
        }
        exits
    }

    /// Declare the block's functions, classes and enums before checking any
    /// of it, exactly as the interpreter does before running it.
    fn hoist(&mut self, statements: &[Stmt], env: &Env) {
        // What each statement has above it in the block: the variables
        // that a function or class written there is allowed to use.
        let mut lets: Vec<String> = Vec::new();
        let mut above: Vec<Rc<[String]>> = Vec::new();
        for statement in statements {
            above.push(Rc::from(lets.as_slice()));
            match &statement.kind {
                StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => lets.push(name.clone()),
                StmtKind::Import { module, names } if names.is_empty() => lets.extend(module.last().cloned()),
                StmtKind::Import { names, .. } => lets.extend(names.iter().cloned()),
                _ => {}
            }
        }

        // First the names of the new types, so that they can mention each other.
        let mut classes = Vec::new();
        let mut enums = Vec::new();
        for (statement, above) in statements.iter().zip(&above) {
            match &statement.kind {
                // A class with type parameters is not a class yet: each
                // set of types it is used with gets a copy that is one.
                StmtKind::Class(decl) if !decl.type_params.is_empty() => {
                    self.type_params(&decl.type_params, statement.span);
                    let generic = Rc::new(Generic {
                        at: decl as *const ast::Class as usize,
                        id: decl_id(statement.span, &decl.name),
                        name: decl.name.clone(),
                        params: decl.type_params.clone(),
                        template: Template::Class(Rc::new(decl.clone())),
                        env: env.clone(),
                        span: statement.span,
                        above: above.clone(),
                        outline: RefCell::new(None),
                    });
                    self.declare(env, &decl.name, Ty::Generic(generic.clone()), false, statement.span);
                    self.generics.push(generic);
                }
                StmtKind::Class(decl) => {
                    let info = class_info(decl, env, above, statement.span, None, None);
                    self.declare(env, &decl.name, Ty::Class(info.clone()), false, statement.span);
                    self.classes.insert(info.id.clone(), info.clone());
                    classes.push((decl, info, statement.span));
                }
                StmtKind::Enum(decl) => {
                    let info = Rc::new(EnumInfo {
                        id: decl_id(statement.span, &decl.name),
                        name: decl.name.clone(),
                        cases: RefCell::new(Vec::new()),
                    });
                    self.declare(env, &decl.name, Ty::Enum(info.clone()), false, statement.span);
                    self.enums.insert(info.id.clone(), info.clone());
                    enums.push((decl, info, statement.span));
                }
                _ => {}
            }
        }

        // Then what is inside them.
        for (decl, info, span) in classes {
            self.fill_class(decl, &info, env, span);
        }
        for (decl, info, span) in enums {
            let cases: Vec<CaseInfo> = decl
                .variants
                .iter()
                .map(|case| CaseInfo {
                    name: case.name.clone(),
                    fields: case.fields.iter().map(|f| self.slot(f, &case.name, env, span)).collect(),
                })
                .collect();
            if cases.iter().any(|case| case.fields.iter().any(|field| field.ty.is_owned())) {
                self.error(format!("a case of the enum `{}` cannot carry an owned value yet", decl.name), span);
            }
            for (index, case) in cases.iter().enumerate() {
                if case.fields.is_empty() {
                    // A bare case is a value of the enum.
                    self.declare(env, &case.name, Ty::EnumValue(info.clone()), false, span);
                    if let Some(variable) = env.variables.borrow_mut().get_mut(&case.name) {
                        variable.case = Some(index);
                    }
                } else {
                    self.declare(env, &case.name, Ty::CaseConstructor(info.clone(), index), false, span);
                }
            }
            self.types.enums.insert(
                decl as *const ast::Enum as usize,
                cases.iter().map(|case| case.fields.iter().map(|slot| kind(&slot.ty)).collect()).collect(),
            );
            *info.cases.borrow_mut() = cases;
        }

        for (statement, above) in statements.iter().zip(&above) {
            if let StmtKind::Function(decl) = &statement.kind {
                if !decl.type_params.is_empty() {
                    self.type_params(&decl.type_params, statement.span);
                    let generic = Rc::new(Generic {
                        at: Arc::as_ptr(decl) as usize,
                        id: decl_id(statement.span, &decl.name),
                        name: decl.name.clone(),
                        params: decl.type_params.clone(),
                        template: Template::Function(decl.clone()),
                        env: env.clone(),
                        span: statement.span,
                        above: above.clone(),
                        outline: RefCell::new(None),
                    });
                    self.declare(env, &decl.name, Ty::Generic(generic.clone()), false, statement.span);
                    self.generics.push(generic);
                    continue;
                }
                let function = FunctionInfo {
                    decl: decl.clone(),
                    env: env.clone(),
                    span: statement.span,
                    self_type: None,
                    state: RefCell::new(State::NotChecked),
                    above: above.clone(),
                    copy_note: None,
                };
                self.declare(env, &decl.name, Ty::Function(Rc::new(function)), false, statement.span);
            }
        }
    }

    /// Work out the fields of a class and note its methods. For a class
    /// written in the program this is part of `hoist`. For a copy of a
    /// generic class it happens when the copy is first asked for.
    fn fill_class(&mut self, decl: &ast::Class, info: &Rc<ClassInfo>, env: &Env, span: Span) {
        {
            let fields: Vec<Slot> =
                decl.fields.iter().map(|field| self.slot(field, &decl.name, env, span)).collect();
            if let Some(field) = fields.iter().find(|field| field.ty.is_owned() && !decl.owned) {
                // Objects of an ordinary class are shared. Each holder would
                // believe the owned value inside was still there.
                self.error(
                    format!(
                        "`{}` holds an owned value, so `{}` must be an `owned class` itself. \
                         A shared object cannot own anything",
                        field.name, decl.name
                    ),
                    span,
                );
            }
            self.types
                .classes
                .insert(decl as *const ast::Class as usize, fields.iter().map(|slot| kind(&slot.ty)).collect());
            self.types.widths.insert(
                decl as *const ast::Class as usize,
                fields
                    .iter()
                    .map(|slot| match &slot.ty {
                        Ty::Sized(width) => Some(*width),
                        _ => None,
                    })
                    .collect(),
            );
            *info.fields.borrow_mut() = fields;
            for method in &decl.methods {
                if !method.type_params.is_empty() {
                    self.error(
                        format!(
                            "the method `{}` cannot have type parameters of its own yet. \
                             The class can have them: `class {}[T]`",
                            method.name, decl.name
                        ),
                        span,
                    );
                }
                // So that the one mistake is not reported again for each
                // `T` in the method, its names stand for "anything".
                let env = if method.type_params.is_empty() {
                    env.clone()
                } else {
                    let inner = Scope::child(env);
                    for name in &method.type_params {
                        inner.declare(name, Ty::Alias(Box::new(Ty::Invalid)), false);
                    }
                    inner
                };
                let function = FunctionInfo {
                    decl: method.clone(),
                    env,
                    span,
                    // A method of an owned class borrows the object it is
                    // called on: the caller goes on owning it.
                    self_type: Some(match decl.owned {
                        true => Ty::Borrowed(Box::new(Ty::Instance(info.clone()))),
                        false => Ty::Instance(info.clone()),
                    }),
                    state: RefCell::new(State::NotChecked),
                    above: info.above.clone(),
                    copy_note: info.copy_note.clone(),
                };
                info.methods.borrow_mut().insert(method.name.clone(), Rc::new(function));
            }
        }
    }

    fn slot(&mut self, field: &ast::Field, owner: &str, env: &Env, span: Span) -> Slot {
        let what = format!("`{}` in `{owner}`", field.name);
        let id = (decl_id(span, owner), field.name.clone());
        Slot {
            name: field.name.clone(),
            ty: self.slot_type(&field.ty, &id, &what, env, span, false),
            has_default: field.default.is_some(),
            id: field.ty.is_none().then_some(id),
        }
    }

    /// Check one statement. Returns true if it always leaves the block.
    fn stmt(&mut self, statement: &Stmt, env: &Env) -> bool {
        let span = statement.span;
        self.touched.clear();
        self.owned.looking.clear();

        match &statement.kind {
            StmtKind::Let { name, ty, value, mutable } => {
                self.declaration(name, ty, value, *mutable, env, statement);
                false
            }
            StmtKind::Const { name, ty, value } => {
                self.declaration(name, ty, value, false, env, statement);
                false
            }
            StmtKind::Assign { target, op, value } => {
                let (wanted, holds, field) = self.assignment_target(target, env);
                let given = match op {
                    // `x = v` keeps `v`. `x += v` only reads it.
                    None => {
                        let given = self.kept(value, &wanted, env);
                        self.adapted(written(value), given, &wanted, value.span)
                    }
                    Some(op) => {
                        let given = self.look(value, env);
                        // `xs += [4]` is `xs = xs + [4]`. On an owned list,
                        // the list that `+` makes takes the old one's place.
                        match (&wanted, self.binary(*op, &holds.looked_at(), &given, span)) {
                            (Ty::Owned(_), made @ Ty::List(_)) => Ty::Owned(Box::new(made)),
                            (_, made) => made,
                        }
                    }
                };
                if let Some(field) = field {
                    // A value stored in an untyped field is evidence of its type.
                    self.sees(&field, &given, value.span);
                }
                self.expect(&given, &wanted, "the new value", value.span);
                // What a variable holds from here on is what was just put
                // in it. If that is certainly a value, the variable is
                // known to be present; if it may be `none`, whatever an
                // earlier check said about it no longer holds.
                if let (ExprKind::Name(name), Ty::Optional(inner)) = (&target.kind, &wanted) {
                    if !given.is_loose() && fits(&given, inner) {
                        env.narrow(name, (**inner).clone());
                    } else {
                        env.widen(name);
                    }
                }
                // An owner that gave its value away holds one again.
                if let (ExprKind::Name(name), true) = (&target.kind, wanted.is_owned()) {
                    if let Some(variable) = env.get(name) {
                        self.owned.moved.remove(&variable.id);
                    }
                }
                false
            }
            StmtKind::Expr(expr) => {
                // An owned value made here and kept by nobody is freed on
                // the spot. The compiler needs to know to do that.
                if self.expr(expr, env).is_owned() {
                    self.types.discarded.insert(expr as *const Expr as usize);
                }
                false
            }

            // Declared by `hoist`. Their bodies are checked here, so that a
            // function nobody calls is still checked.
            StmtKind::Function(decl) => {
                if let Some(Variable { ty: Ty::Function(function), .. }) = env.get_here(&decl.name) {
                    self.signature(&function);
                }
                false
            }
            StmtKind::Class(decl) => {
                if let Some(Variable { ty: Ty::Class(class), .. }) = env.get_here(&decl.name) {
                    self.class_body(decl, &class, env);
                }
                false
            }
            StmtKind::Enum(_) => false,

            StmtKind::If { condition, then_block, else_block } => {
                self.condition(condition, env);
                let (when_true, when_false) = self.narrowings(condition, env);

                let then_scope = Scope::child(env);
                narrow_all(&then_scope, &when_true);
                let before = self.owned.moved.clone();
                let then_exits = self.statements(then_block, &then_scope);
                let then_moved = std::mem::replace(&mut self.owned.moved, before);

                let else_scope = Scope::child(env);
                narrow_all(&else_scope, &when_false);
                let else_exits = match else_block {
                    Some(block) => self.statements(block, &else_scope),
                    None => false,
                };
                // What is given away after the `if` is what either branch
                // gave away, leaving out a branch that never gets there.
                match (then_exits, else_exits) {
                    (true, false) => {}
                    (false, true) => self.owned.moved = then_moved,
                    _ => self.owned.moved.extend(then_moved),
                }

                // If one branch always leaves, what follows the `if` only
                // runs when the other branch's condition held:
                //     if x is none { return }
                //     ... x is not none from here on ...
                //
                // The branch that carries on may itself have changed the
                // variable, so what is passed on is what that branch still
                // knows at its end.
                if then_exits && !else_exits {
                    keep_known(env, &else_scope, &when_false);
                } else if else_exits && !then_exits {
                    keep_known(env, &then_scope, &when_true);
                } else if !then_exits {
                    // Both branches carry on. What they agree on at their
                    // ends is known afterwards:
                    //     if name is none { name = "anon" }
                    //     ... name is a str from here on, either way ...
                    for (name, _) in when_true.iter().chain(&when_false) {
                        if let (Some(then), Some(other)) = (then_scope.get(name), else_scope.get(name)) {
                            if then.declared.is_some() && other.declared.is_some() && same(&then.ty, &other.ty) {
                                env.narrow(name, then.ty);
                            }
                        }
                    }
                }
                then_exits && else_exits
            }
            StmtKind::While { condition, body } => {
                // The condition runs again on every trip, so it is part of
                // the loop as far as owned values go.
                let entry = self.enter_loop();
                self.condition(condition, env);
                // Each trip through the body starts just after the
                // condition held, so what it says is known in there:
                // `while node is not none { ... node.next ... }`.
                let scope = Scope::child(env);
                forget_assigned(body, &scope);
                narrow_all(&scope, &self.narrowings(condition, &scope).0);
                self.frame().loops += 1;
                let exits = self.block(body, &scope);
                self.leave_loop(entry, exits);
                self.frame().loops -= 1;
                // `while true` with no `break` never finishes, so nothing follows it.
                condition.kind == ExprKind::Bool(true) && !has_break(body)
            }
            StmtKind::For { variable, iterable, body } => {
                let iterable_type = self.look(iterable, env);
                let item = self.item_type(&iterable_type, iterable.span);
                let scope = Scope::child(env);
                forget_assigned(body, &scope);
                scope.declare(variable, item, false);
                self.frame().loops += 1;
                let entry = self.enter_loop();
                let exits = self.statements(body, &scope);
                self.leave_loop(entry, exits);
                self.frame().loops -= 1;
                false
            }
            StmtKind::Match { subject, arms } => self.match_statement(subject, arms, env, span),
            StmtKind::Try { body, handlers } => self.try_statement(body, handlers, env, span),

            StmtKind::Return(value) => {
                let declared = self.frame().declared_return.clone();
                let ty = match value {
                    // The caller keeps what is returned.
                    Some(value) => {
                        let wanted = declared.as_ref().unwrap_or(&Ty::Invalid);
                        let given = self.kept(value, wanted, env);
                        self.adapted(written(value), given, wanted, value.span)
                    }
                    None => Ty::None,
                };
                if self.frames.len() == 1 {
                    self.error("`return` outside a function", span);
                    return true;
                }
                if let Some(declared) = self.frame().declared_return.clone() {
                    let at = value.as_ref().map_or(span, |value| value.span);
                    self.expect(&ty, &declared, "the returned value", at);
                }
                self.frame().returns.push(ty);
                true
            }
            StmtKind::Raise(value) => {
                match self.expr(value, env) {
                    // An error travels to wherever it is caught, and can be
                    // kept there for any length of time.
                    ty if ty.is_tied() => {
                        self.error(format!("an owned or borrowed value cannot be raised, and this is {ty}"), span);
                    }
                    Ty::Error(kind) => self.can_raise(&kind, span),
                    // `raises` and `except` name a class, and `Problem[int]`
                    // is not a name that can be written there.
                    Ty::Instance(class) if class.origin.is_some() => {
                        self.error(
                            format!("an object of a generic class cannot be raised yet, and this is a {}", class.name),
                            span,
                        );
                    }
                    Ty::Instance(class) => self.can_raise(&class.name, span),
                    ty if ty.is_loose() => self.can_raise("Error", span),
                    ty => {
                        self.error(format!("only an error can be raised, and this is {ty}"), span);
                    }
                }
                true
            }
            StmtKind::Break | StmtKind::Continue => {
                if self.frame().loops == 0 {
                    let word = if statement.kind == StmtKind::Break { "break" } else { "continue" };
                    self.error(format!("`{word}` outside a loop"), span);
                }
                // Remember what is given away by now: it still is, where
                // this jump lands.
                let moved = self.owned.moved.clone();
                if let Some((breaks, continues)) = self.owned.loops.last_mut() {
                    if statement.kind == StmtKind::Break { breaks.extend(moved) } else { continues.extend(moved) }
                }
                true
            }

            StmtKind::Import { module, names } => {
                let path = module.join(".");
                let module = match path.as_str() {
                    "math" => "math",
                    "time" => "time",
                    "board" => "board",
                    _ => {
                        self.error(
                            format!("there is no module `{path}`. So far Zeno has `math`, `time` and `board`"),
                            span,
                        );
                        // Declare the names anyway, so each use is not a second error.
                        for name in names.iter().chain(module.last().filter(|_| names.is_empty())) {
                            env.declare(name, Ty::Invalid, false);
                        }
                        return false;
                    }
                };
                if names.is_empty() {
                    self.declare(env, module, Ty::Module(module), false, span);
                }
                for name in names {
                    let ty = match module_member(module, name) {
                        Some(ty) => ty,
                        None => self.error(format!("module `{module}` has no `{name}`"), span),
                    };
                    self.declare(env, name, ty, false, span);
                }
                false
            }
        }
    }

    /// `let`, `var` and `const`.
    fn declaration(
        &mut self,
        name: &str,
        annotation: &Option<ast::Type>,
        value: &Expr,
        mutable: bool,
        env: &Env,
        statement: &Stmt,
    ) {
        let span = statement.span;
        // The variable keeps the value. If the value is an owned one held
        // by another variable, that one gives it up.
        let wanted = annotation.as_ref().map(|annotation| self.resolve(annotation, env, span));
        if let Some(wanted) = &wanted {
            self.expecting(value, wanted);
        }
        let given = self.take(value, env);
        let ty = match wanted {
            Some(wanted) => {
                let wanted_item = match &wanted {
                    Ty::List(item) => Some(item),
                    Ty::Owned(inner) => match &**inner {
                        Ty::List(item) => Some(item),
                        _ => None,
                    },
                    _ => None,
                };
                match (&value.kind, wanted_item) {
                    // A list written out in full is checked item by item, so
                    // that `[1, "two"]` is caught rather than passing as a
                    // list of mixed, unknown things.
                    (ExprKind::List(items), Some(wanted_item)) => {
                        if matches!(wanted, Ty::Owned(_)) {
                            self.types.fresh.insert(value as *const Expr as usize);
                        }
                        for item in items {
                            let item_type = self.expr(item, env);
                            let item_type = self.adapted(written(item), item_type, wanted_item, item.span);
                            if !fits(&item_type, wanted_item) {
                                self.error(
                                    format!("`{name}` should be {wanted}, but this item is {item_type}"),
                                    item.span,
                                );
                            } else {
                                self.arrives(&item_type, wanted_item, &format!("`{name}`"), item.span);
                            }
                        }
                    }
                    _ => {
                        let given = self.fresh_list(value, given, &wanted);
                        let given = self.adapted(written(value), given, &wanted, value.span);
                        self.expect(&given, &wanted, &format!("`{name}`"), value.span)
                    }
                }
                wanted
            }
            None if self.strict && given.has_dynamic() => self.error(
                format!("strict mode: the type of `{name}` cannot be worked out. Give it one, as in `{name}: list[int]`"),
                span,
            ),
            None => given,
        };
        if ty.is_owned() {
            self.types.owners.insert(statement as *const Stmt as usize);
        }
        self.declare(env, name, ty, mutable, span);
    }

    /// The type that may be assigned to `target`, after checking that it can
    /// be assigned to at all. For a field of an object, the field is
    /// returned as well.
    /// What can be stored in `target`, what it holds at this point (which
    /// is narrower after a check such as `is not none`), and the field it
    /// is, if it is one.
    fn assignment_target(&mut self, target: &Expr, env: &Env) -> (Ty, Ty, Option<Slot>) {
        let span = target.span;
        let ty = match &target.kind {
            ExprKind::Name(name) => match (env.get(name), self.too_early(name, env.get(name).as_ref(), env, span)) {
                (_, Some(mistake)) => mistake,
                (None, None) => self.error(
                    format!("`{name}` is not declared. Create it with `let` or `var` first"),
                    span,
                ),
                (Some(variable), None) if !variable.mutable => self.error(
                    format!("`{name}` cannot be changed. Declare it with `var` to allow that"),
                    span,
                ),
                (Some(variable), None) if variable.ty.is_owned() && variable.function != env.function => {
                    self.error(elsewhere(name), span)
                }
                (Some(variable), None) => {
                    let Some(declared) = variable.declared else {
                        return (variable.ty.clone(), variable.ty, None);
                    };
                    // `x += 1` reads `x` as well as writing it.
                    self.types.narrowed.insert(target as *const Expr as usize, kind(&variable.ty));
                    return (declared, variable.ty, None);
                }
            },
            ExprKind::Field { object, name } => match self.look(object, env) {
                Ty::Instance(class) => {
                    let field = class.fields.borrow().iter().find(|f| f.name == *name).cloned();
                    // Is `object` the variable that owns the object?
                    let owner = matches!(&object.kind, ExprKind::Name(name)
                        if env.get(name).is_some_and(|variable| variable.ty.is_owned()));
                    match field {
                        // Putting a new owned value in a field frees the
                        // old one. Somebody who was lent the object may be
                        // looking at the old one right now, so only the
                        // owner, who knows nobody is, may do it.
                        Some(field) if field.ty.is_owned() && !owner => self.error(
                            format!(
                                "`{name}` holds an owned value, and only the owner of the object can replace it. \
                                 Here the object is only borrowed"
                            ),
                            span,
                        ),
                        Some(field) => return (field.ty.clone(), field.ty.clone(), Some(field)),
                        None => self.error(format!("`{}` has no field `{name}`", class.name), span),
                    }
                }
                ty if ty.is_loose() => ty,
                ty => self.error(format!("a field cannot be set on {ty}"), span),
            },
            ExprKind::Index { object, index } => {
                let object = self.look(object, env);
                let index_type = self.look(index, env);
                self.expect(&index_type, &Ty::Int, "an index", index.span);
                match object {
                    Ty::List(item) => *item,
                    ty if ty.is_loose() => ty,
                    ty => self.error(format!("an item cannot be set on {ty}"), span),
                }
            }
            _ => self.error("this cannot be assigned to", span),
        };
        (ty.clone(), ty, None)
    }

    fn condition(&mut self, condition: &Expr, env: &Env) {
        let ty = self.look(condition, env);
        if !fits(&ty, &Ty::Bool) {
            self.error(format!("a condition must be true or false, but this is {ty}"), condition.span);
        }
    }

    /// The type of each item when looping over a value of type `ty`.
    fn item_type(&mut self, ty: &Ty, span: Span) -> Ty {
        match ty {
            Ty::List(item) => (**item).clone(),
            Ty::Str => Ty::Str,
            ty if ty.is_loose() => ty.clone(),
            ty => self.error(format!("{ty} cannot be looped over"), span),
        }
    }

    /// What a condition tells us about the variables in it. Returns the
    /// narrower types that hold when it is true, and when it is false.
    ///
    /// `x is not none` means `x` is a plain `T` when true and `none` when false.
    fn narrowings(&self, condition: &Expr, env: &Env) -> (Vec<(String, Ty)>, Vec<(String, Ty)>) {
        match &condition.kind {
            ExprKind::Binary { op: op @ (BinaryOp::Is | BinaryOp::IsNot), left, .. } => {
                let ExprKind::Name(name) = &left.kind else {
                    return (Vec::new(), Vec::new());
                };
                let Some(Variable { ty: Ty::Optional(inner), .. }) = env.get(name) else {
                    return (Vec::new(), Vec::new());
                };
                let present = vec![(name.clone(), *inner)];
                let absent = vec![(name.clone(), Ty::None)];
                if *op == BinaryOp::Is {
                    (absent, present)
                } else {
                    (present, absent)
                }
            }
            ExprKind::Unary { op: UnaryOp::Not, operand } => {
                let (when_true, when_false) = self.narrowings(operand, env);
                (when_false, when_true)
            }
            // `a and b` being true means both were true.
            ExprKind::Binary { op: BinaryOp::And, left, right } => {
                let mut when_true = self.narrowings(left, env).0;
                when_true.extend(self.narrowings(right, env).0);
                (when_true, Vec::new())
            }
            // `a or b` being false means both were false.
            ExprKind::Binary { op: BinaryOp::Or, left, right } => {
                let mut when_false = self.narrowings(left, env).1;
                when_false.extend(self.narrowings(right, env).1);
                (Vec::new(), when_false)
            }
            _ => (Vec::new(), Vec::new()),
        }
    }

    fn class_body(&mut self, decl: &ast::Class, class: &Rc<ClassInfo>, env: &Env) {
        let fields = class.fields.borrow().clone();
        for (field, slot) in decl.fields.iter().zip(&fields) {
            if let Some(default) = &field.default {
                let given = self.kept(default, &slot.ty, env);
                let given = self.adapted(written(default), given, &slot.ty, default.span);
                self.sees(slot, &given, default.span);
                self.expect(&given, &slot.ty, &format!("`{}`", field.name), default.span);
            }
        }
        let methods: Vec<Rc<FunctionInfo>> = class.methods.borrow().values().cloned().collect();
        for method in methods {
            if method.decl.params.is_empty() {
                self.error(
                    format!("the method `{}` needs `self` as its first parameter", method.decl.name),
                    method.span,
                );
            }
            self.signature(&method);
        }
    }

    fn match_statement(&mut self, subject: &Expr, arms: &[Arm], env: &Env, span: Span) -> bool {
        let mut subject_type = self.expr(subject, env);
        if subject_type.is_tied() {
            // `case name` would make a second name for the value.
            subject_type = self.error(
                format!("`match` cannot take an owned or borrowed value, and this is {subject_type}. Match on one of its fields"),
                subject.span,
            );
        }
        let mut all_exit = true;
        // What is given away after the `match` is what any case that gets
        // to its end gave away.
        let before = self.owned.moved.clone();
        let mut after: Option<Moved> = None;
        // Does some case accept anything at all?
        let mut catch_all = false;
        // For an enum, the cases that are fully handled.
        let mut covered = Vec::new();

        for arm in arms {
            let scope = Scope::child(env);
            match self.pattern(&arm.pattern, &subject_type, &scope, span) {
                Covers::Everything => catch_all = true,
                Covers::Case(index) => covered.push(index),
                Covers::Some => {}
            }
            let exits = self.statements(&arm.body, &scope);
            all_exit &= exits;
            let moved = std::mem::replace(&mut self.owned.moved, before.clone());
            if !exits {
                after.get_or_insert_with(HashMap::new).extend(moved);
            }
        }
        self.owned.moved.extend(after.unwrap_or_default());

        let complete = match &subject_type {
            _ if catch_all => true,
            Ty::EnumValue(info) => {
                let missing: Vec<String> = info
                    .cases
                    .borrow()
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| !covered.contains(index))
                    .map(|(_, case)| case.name.clone())
                    .collect();
                if !missing.is_empty() {
                    self.error(
                        format!(
                            "this `match` does not handle {}. Add a `case` for it, or a final `case _`",
                            missing.join(", ")
                        ),
                        span,
                    );
                }
                // Already reported. Treat it as complete so that the same
                // gap is not reported again as a missing `return`.
                true
            }
            ty if ty.is_loose() => false,
            _ => {
                self.error("this `match` needs a final `case _` to handle every other value", span);
                false
            }
        };
        all_exit && complete
    }

    /// Check a pattern against the type being matched, declaring the names
    /// it introduces. Reports how much of the type the pattern accepts.
    fn pattern(&mut self, pattern: &Pattern, subject: &Ty, scope: &Env, span: Span) -> Covers {
        match pattern {
            Pattern::Wildcard => Covers::Everything,
            Pattern::Literal(literal) => {
                let ty = self.expr(literal, scope);
                if !fits(&ty, subject) && !fits(subject, &ty) {
                    self.error(format!("this case is {ty}, which can never match {subject}"), span);
                }
                Covers::Some
            }
            Pattern::Name(name) => {
                // A bare enum case is compared; any other name takes the value.
                if let Some(Variable { ty: Ty::EnumValue(info), case: Some(index), .. }) = scope.get(name) {
                    if !fits(&Ty::EnumValue(info.clone()), subject) {
                        self.error(
                            format!("`{name}` is a case of {}, which can never match {subject}", info.name),
                            span,
                        );
                    }
                    return Covers::Case(index);
                }
                scope.declare(name, subject.clone(), false);
                Covers::Everything
            }
            Pattern::Variant { name, fields } => {
                let Some(Variable { ty: Ty::CaseConstructor(info, index), .. }) = scope.get(name) else {
                    self.error(format!("`{name}` is not an enum case that carries values"), span);
                    return Covers::Some;
                };
                if !fits(&Ty::EnumValue(info.clone()), subject) {
                    self.error(
                        format!("`{name}` is a case of {}, which can never match {subject}", info.name),
                        span,
                    );
                }
                let slots = info.cases.borrow()[index].fields.clone();
                if slots.len() != fields.len() {
                    self.error(
                        format!(
                            "`{name}` carries {} value(s), but this pattern has {}",
                            slots.len(),
                            fields.len()
                        ),
                        span,
                    );
                    return Covers::Some;
                }
                // The case is fully handled only if every inner pattern accepts anything.
                let mut whole_case = true;
                for (pattern, slot) in fields.iter().zip(&slots) {
                    whole_case &= matches!(self.pattern(pattern, &slot.ty, scope, span), Covers::Everything);
                }
                if whole_case {
                    Covers::Case(index)
                } else {
                    Covers::Some
                }
            }
        }
    }

    fn try_statement(&mut self, body: &[Stmt], handlers: &[Handler], env: &Env, span: Span) -> bool {
        // Collect what the body can raise separately, so the handlers can
        // take out what they catch.
        self.raised.push(Vec::new());
        let before = self.owned.moved.clone();
        self.owned.tries.push(HashMap::new());
        let mut all_exit = self.block(body, env);
        let raised = self.raised.pop().unwrap_or_default();
        // An error can leave the body after any of the things it gives
        // away, so an `except` block starts with all of them gone.
        let in_body = self.owned.tries.pop().unwrap_or_default();
        if let Some(outer) = self.owned.tries.last_mut() {
            outer.extend(in_body.clone());
        }
        let mut after = if all_exit { Moved::new() } else { self.owned.moved.clone() };
        let mut at_except = before;
        at_except.extend(in_body);

        let mut caught: Vec<&str> = Vec::new();
        for handler in handlers {
            let scope = Scope::child(env);
            let error_type = match &handler.error {
                None => Ty::Error(Rc::from("Error")),
                Some(name) => match env.get(name).map(|variable| variable.ty) {
                    Some(Ty::ErrorType(kind)) => Ty::Error(kind),
                    Some(Ty::Class(class)) if class.owned => {
                        self.error(format!("`{name}` is an owned class. Its objects cannot be raised, so none can be caught"), span)
                    }
                    Some(Ty::Class(class)) => Ty::Instance(class),
                    _ => self.error(format!("`{name}` is not a kind of error"), span),
                },
            };
            caught.push(handler.error.as_deref().unwrap_or("Error"));
            if let Some(binding) = &handler.binding {
                scope.declare(binding, error_type, false);
            }
            self.owned.moved = at_except.clone();
            let exits = self.statements(&handler.body, &scope);
            all_exit &= exits;
            if !exits {
                after.extend(std::mem::take(&mut self.owned.moved));
            }
        }
        self.owned.moved = after;

        // Whatever no handler catches carries on outward.
        for (kind, at) in raised {
            if !caught.iter().any(|name| *name == "Error" || *name == kind) {
                self.can_raise(&kind, at);
            }
        }
        all_exit
    }

    // ----------------------------------------------------------- Owned values
    //
    // Most values in Zeno are shared: `let b = a` makes a second name for
    // the same list, and the collector frees it when no name is left. An
    // owned value (an `owned list[T]`, or an object of an `owned class`)
    // has exactly one owner instead, and is freed when its owner's block
    // ends. For that to be safe the checker has to know, at every line,
    // who the owner is. Three ways of using a value keep that simple:
    //
    // - `take`: the value is about to be kept somewhere (a variable, a
    //   field, a parameter that takes it, a `return`). If it comes from a
    //   variable, that variable has given it away and may not be used again.
    // - `look`: the value is read where it stands (printed, indexed,
    //   compared, lent to a `borrow` parameter). Nothing changes hands.
    // - `expr`, used anywhere else, gives an owner's value the type
    //   `Borrowed`, which fits nowhere. So a place this section forgot is
    //   an error message, never a second owner.

    /// Is `name` being used in a function that was called too early, from
    /// above the line that declares the variable it means? If so this
    /// reports it, once for each name, and returns the type of a mistake.
    /// `variable` is what the name was taken for, if anything.
    fn too_early(&mut self, name: &str, variable: Option<&Variable>, env: &Env, span: Span) -> Option<Ty> {
        // Start at the function the name is written in and work outward
        // through the functions it is written inside.
        let mut body = env.function;
        let at = loop {
            // A variable of this very function is what the name means.
            if variable.is_some_and(|variable| variable.function == body) {
                return None;
            }
            let at = self.early.iter().rposition(|early| early.body == body)?;
            if self.early[at].missing.iter().any(|missing| missing == name) {
                break at;
            }
            body = self.early[at].outer;
        };
        let checking = &mut self.early[at];
        if checking.told.iter().any(|told| told == name) {
            return Some(Ty::Invalid);
        }
        checking.told.push(name.to_string());
        let function = checking.name.clone();
        Some(self.error(
            format!(
                "`{name}` is declared further down than the first use of `{function}`, which needs it. \
                 Use `{function}` below the line that declares `{name}`"
            ),
            span,
        ))
    }

    /// What reading the variable `name` gives. For an owner, or a `borrow`
    /// parameter, that is a look at the value and no more.
    fn read(&mut self, name: &str, variable: Variable, env: &Env, span: Span) -> Ty {
        if !variable.ty.is_tied() {
            return variable.ty;
        }
        if variable.function != env.function {
            return self.error(elsewhere(name), span);
        }
        if let Some((_, at)) = self.owned.moved.get(&variable.id) {
            return self.error(
                format!(
                    "`{name}` was given away on line {} and holds nothing now. \
                     Lend it instead (a `borrow` parameter), or use it before it goes",
                    at.line
                ),
                span,
            );
        }
        self.owned.looking.push(variable.id);
        Ty::Borrowed(Box::new(variable.ty.looked_at()))
    }

    /// The type of an expression whose value is about to be kept. An owner
    /// named here gives its value away.
    fn take(&mut self, expr: &Expr, env: &Env) -> Ty {
        let owner = match &expr.kind {
            ExprKind::Name(name) => env.get(name).filter(|variable| variable.ty.is_owned()).map(|v| (name, v)),
            _ => None,
        };
        if let Some((name, variable)) = owner {
            if self.owned.looking.contains(&variable.id) {
                return self.error(
                    format!(
                        "`{name}` is given away here, on a line that also uses it. \
                         Do the other thing on a line of its own first"
                    ),
                    expr.span,
                );
            }
            // The usual checks: that it still holds something, and that
            // this is the function it belongs to.
            if matches!(self.expr(expr, env), Ty::Invalid) {
                return Ty::Invalid;
            }
            let record = (name.clone(), expr.span);
            for body in &mut self.owned.tries {
                body.insert(variable.id, record.clone());
            }
            self.owned.moved.insert(variable.id, record);
            self.types.moves.insert(expr as *const Expr as usize);
            return variable.ty;
        }
        match self.expr(expr, env) {
            Ty::Borrowed(inner) => {
                let message = match &expr.kind {
                    ExprKind::Name(name) => format!(
                        "`{name}` is only borrowed here, so it cannot be kept, returned or given away. \
                         To keep it, take it as an owned parameter instead"
                    ),
                    ExprKind::Field { name, .. } => format!(
                        "`{name}` holds an owned value, which cannot be taken out of the object it is in. \
                         Use it where it is, or lend it to a `borrow` parameter"
                    ),
                    _ => format!("this is {}, which cannot be kept", Ty::Borrowed(inner)),
                };
                self.error(message, expr.span)
            }
            ty => ty,
        }
    }

    /// `take`, for a value going somewhere whose type is known.
    fn kept(&mut self, value: &Expr, wanted: &Ty, env: &Env) -> Ty {
        self.expecting(value, wanted);
        let given = self.take(value, env);
        self.fresh_list(value, given, wanted)
    }

    /// A list that was made right here is nobody's yet: one written out
    /// in full, one built by a comprehension, or what `split`, `map`,
    /// `filter` or `+` gives. So it can begin life as an owned list where
    /// one is wanted. The compiler is told, because a build with no
    /// collector has to know who frees each list.
    fn fresh_list(&mut self, value: &Expr, given: Ty, wanted: &Ty) -> Ty {
        let made = matches!(value.kind, ExprKind::List(_) | ExprKind::ListComp { .. })
            || self.types.made.contains(&(value as *const Expr as usize));
        match (wanted, given) {
            (Ty::Owned(_), given @ Ty::List(_)) if made => {
                self.types.fresh.insert(value as *const Expr as usize);
                Ty::Owned(Box::new(given))
            }
            (_, given) => given,
        }
    }

    /// The type of an expression that is only looked at.
    fn look(&mut self, expr: &Expr, env: &Env) -> Ty {
        let ty = self.expr(expr, env);
        self.looked(ty, expr)
    }

    /// What somebody who only looks sees of the value of `expr`, whose
    /// type is `ty`.
    fn looked(&mut self, ty: Ty, expr: &Expr) -> Ty {
        if ty.is_owned() {
            // Made right here, and given to nobody, as in
            // `print(size(make()))`. The statement it is in owns it, and
            // frees it when the statement is over.
            self.types.lent.insert(expr as *const Expr as usize);
        }
        ty.looked_at()
    }

    /// The type of an expression whose value goes somewhere shared, such
    /// as into a list. An owned value cannot.
    fn shared(&mut self, expr: &Expr, env: &Env) -> Ty {
        let ty = self.expr(expr, env);
        if ty.is_tied() {
            return self.error(format!("a list cannot hold an owned or borrowed value yet, and this is {ty}"), expr.span);
        }
        ty
    }

    /// The parameters a call's arguments are going to, when the thing
    /// called has parameters with names.
    fn slots(&mut self, callee: &Ty) -> Option<Vec<Slot>> {
        match callee {
            Ty::Function(function) => Some(self.signature(function).params.clone()),
            Ty::BoundMethod(function) => Some(self.signature(function).params.get(1..).unwrap_or_default().to_vec()),
            Ty::Class(class) => Some(class.fields.borrow().clone()),
            Ty::CaseConstructor(info, index) => Some(info.cases.borrow()[*index].fields.clone()),
            Ty::Generic(generic) => Some(self.outline(generic).slots.clone()),
            _ => None,
        }
    }

    /// A loop begins. Returns what is needed to check it when it ends.
    fn enter_loop(&mut self) -> (Moved, usize) {
        self.owned.loops.push((Moved::new(), Moved::new()));
        (self.owned.moved.clone(), fresh_id())
    }

    /// A loop has ended. A value given away on one trip round is not
    /// there on the next, so an owner from outside the loop may only be
    /// given away on a path that leaves the loop.
    fn leave_loop(&mut self, (before, first_inside): (Moved, usize), body_exits: bool) {
        let (breaks, continues) = self.owned.loops.pop().unwrap_or_default();
        let mut again = continues;
        if !body_exits {
            again.extend(std::mem::take(&mut self.owned.moved));
        }
        let mut lost: Vec<(String, Span)> = again
            .iter()
            .filter(|(id, _)| **id < first_inside && !before.contains_key(id))
            .map(|(_, record)| record.clone())
            .collect();
        lost.sort_by_key(|(_, at)| (at.line, at.col));
        for (name, at) in lost {
            self.error(
                format!(
                    "`{name}` is given away inside a loop, and the loop may come round again with nothing in it. \
                     Give it away after the loop, or just before a `break`"
                ),
                at,
            );
        }
        // After the loop: whatever was gone before it, during it, or at a
        // `break`. The loop's own variables no longer exist.
        self.owned.moved = before;
        self.owned.moved.extend(again);
        self.owned.moved.extend(breaks);
        self.owned.moved.retain(|id, _| *id < first_inside);
    }

    // -------------------------------------------------------------- Functions

    /// The signature of a function, checking its body first if that has not
    /// been done yet.
    fn signature(&mut self, function: &Rc<FunctionInfo>) -> Rc<Signature> {
        match function.state.borrow().clone() {
            State::Checked(signature) | State::Checking(signature) => return signature,
            State::NotChecked => {}
        }
        let decl = &function.decl;
        let span = function.span;
        let name = if decl.name.is_empty() { "function".to_string() } else { decl.name.clone() };

        let mut params = Vec::new();
        for (index, param) in decl.params.iter().enumerate() {
            let ty = match &function.self_type {
                // The first parameter of a method is the object itself.
                Some(self_type) if index == 0 && param.ty.is_none() => self_type.clone(),
                _ => {
                    let what = format!("the parameter `{}` of `{name}`", param.name);
                    let id = (decl_id(span, &decl.name), param.name.clone());
                    let ty = self.slot_type(&param.ty, &id, &what, &function.env, span, true);
                    let id = param.ty.is_none().then_some(id);
                    params.push(Slot { name: param.name.clone(), ty, has_default: param.default.is_some(), id });
                    continue;
                }
            };
            params.push(Slot { name: param.name.clone(), ty, has_default: param.default.is_some(), id: None });
        }
        let declared_return = decl.return_type.as_ref().map(|ty| self.resolve(ty, &function.env, span));
        for kind in &decl.raises {
            let known = function.env.get(kind).map(|variable| variable.ty);
            if !matches!(known, Some(Ty::ErrorType(_) | Ty::Class(_))) {
                self.error(format!("`{kind}` is not a kind of error"), span);
            }
        }

        // Until the body is checked, callers (that is, recursive calls) see
        // what is written down. If no return type is written, they see what
        // the last pass worked out, the same way untyped parameters do.
        let return_id: SlotId = (decl_id(span, &decl.name), "->".to_string());
        let returns_for_now = match &declared_return {
            Some(declared) => declared.clone(),
            None => match self.known.get(&return_id).cloned() {
                Some(key) => self.from_key(&key),
                None if !self.last_round && !self.open.contains(&return_id) => Ty::Invalid,
                None => Ty::Dynamic,
            },
        };
        let declared = Rc::new(Signature {
            name: name.clone(),
            params: params.clone(),
            returns: returns_for_now,
            raises: decl.raises.clone(),
        });
        *function.state.borrow_mut() = State::Checking(declared);

        let mut worked_out = HashMap::new();
        for slot in &params {
            if let Some(line) = slot.id.as_ref().and_then(|id| self.known_origins.get(id)) {
                if !slot.ty.is_loose() {
                    worked_out.insert(slot.name.clone(), (slot.ty.to_string(), *line));
                }
            }
        }
        let touched = std::mem::take(&mut self.touched);
        // Owned values belong to one function body, so what the body
        // around this one was tracking is set aside.
        let owned = std::mem::take(&mut self.owned);

        let scope = Scope::body(&function.env);
        self.early.push(Early {
            name: name.clone(),
            body: scope.function,
            outer: function.env.function,
            missing: not_yet(&function.above, &function.env),
            told: Vec::new(),
        });
        for (param, slot) in decl.params.iter().zip(&params) {
            if let Some(default) = &param.default {
                let given = self.kept(default, &slot.ty, &scope);
                let given = self.adapted(written(default), given, &slot.ty, default.span);
                self.sees(slot, &given, default.span);
                self.expect(&given, &slot.ty, &format!("`{}`", param.name), default.span);
            }
            self.declare(&scope, &param.name, slot.ty.clone(), false, span);
        }

        self.frames.push(Frame { declared_return: declared_return.clone(), returns: Vec::new(), loops: 0 });
        self.raised.push(Vec::new());
        self.worked_out.push(worked_out);
        self.copy_notes.push(function.copy_note.clone());
        let exits = self.statements(&decl.body, &scope);
        self.copy_notes.pop();
        self.early.pop();
        self.worked_out.pop();
        self.touched = touched;
        self.owned = owned;
        let raised = self.raised.pop().unwrap_or_default();
        let frame = self.frames.pop().expect("the frame pushed above");

        let returns = match declared_return {
            Some(declared) => {
                if !exits && !declared.accepts_none() {
                    self.error(
                        format!("`{name}` can reach its end without returning the {declared} it promises"),
                        span,
                    );
                }
                declared
            }
            // Not written down: it is whatever the `return`s give, plus
            // `none` if the end of the body can be reached.
            None => {
                let mut types = frame.returns;
                if !exits {
                    types.push(Ty::None);
                }
                if !self.last_round {
                    // A `return` whose type is not settled yet (it hangs on
                    // a recursive call, say) is left out of this pass.
                    let unsettled = types.len();
                    types.retain(|ty| !matches!(ty, Ty::Invalid));
                    if types.is_empty() && unsettled > 0 {
                        types.push(Ty::Invalid);
                    }
                }
                let mut returns = self.join_all(&types).unwrap_or(Ty::None);
                if self.strict && matches!(returns, Ty::Dynamic) && !types.iter().any(Ty::is_loose) {
                    // Each `return` is known, but they do not agree.
                    let mut kinds: Vec<String> = types.iter().map(Ty::to_string).collect();
                    kinds.dedup();
                    returns = self.error(
                        format!(
                            "strict mode: `{name}` returns different types ({}). Make them one type, as in `float(n)` or `1.0`",
                            kinds.join(", ")
                        ),
                        span,
                    );
                }
                match self.key(&returns) {
                    Some(Ok(key)) => {
                        self.seen.insert(return_id, Seen::Known(key));
                    }
                    Some(Err(())) => {
                        self.seen.insert(return_id, Seen::Unknowable);
                    }
                    None => {}
                }
                returns
            }
        };

        // In strict mode, or once a function says what it raises, the list
        // must be complete. Otherwise it is worked out, like a return type.
        let raises = if self.strict || !decl.raises.is_empty() {
            for (kind, at) in &raised {
                if !decl.raises.iter().any(|declared| declared == "Error" || declared == kind) {
                    self.error(
                        format!(
                            "`{name}` can raise {kind} here but does not say so. Add `raises {kind}` to it, or catch the error"
                        ),
                        *at,
                    );
                }
            }
            decl.raises.clone()
        } else {
            let mut kinds: Vec<String> = raised.into_iter().map(|(kind, _)| kind).collect();
            kinds.sort();
            kinds.dedup();
            kinds
        };

        self.types.functions.insert(
            Arc::as_ptr(&function.decl) as usize,
            FunctionKind { params: params.iter().map(|slot| kind(&slot.ty)).collect(), returns: kind(&returns) },
        );
        let signature = Rc::new(Signature { name, params, returns, raises });
        *function.state.borrow_mut() = State::Checked(signature.clone());
        signature
    }

    // ----------------------------------------------------------- Expressions

    /// The type of an expression.
    fn expr(&mut self, expr: &Expr, env: &Env) -> Ty {
        let ty = self.expr_inner(expr, env);
        if let Ty::Generic(generic) = &ty {
            // `let f = first` would need `f` to be every copy at once.
            return self.error(
                format!(
                    "`{0}` is generic. It can be called, as in `{0}(...)`, but it cannot be kept as a value or passed along yet",
                    generic.name
                ),
                expr.span,
            );
        }
        if self.strict && matches!(ty, Ty::Dynamic) {
            return self.error(
                "strict mode: the type of this cannot be worked out before the program runs",
                expr.span,
            );
        }
        ty
    }

    fn expr_inner(&mut self, expr: &Expr, env: &Env) -> Ty {
        let span = expr.span;

        match &expr.kind {
            ExprKind::Int(_) => Ty::Int,
            ExprKind::Float(_) => Ty::Float,
            ExprKind::Str(_) => Ty::Str,
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::None => Ty::None,

            ExprKind::FormatStr(parts) => {
                for part in parts {
                    if let FormatPart::Value { value, .. } = part {
                        self.look(value, env);
                    }
                }
                Ty::Str
            }

            ExprKind::Name(name) => match env.get(name) {
                Some(variable) => {
                    if self.worked_out.last().is_some_and(|params| params.contains_key(name)) {
                        self.touched.push(name.clone());
                    }
                    if variable.declared.is_some() {
                        // The compiler needs to know that here the value
                        // is the narrower type: it stores the two differently.
                        self.types.narrowed.insert(expr as *const Expr as usize, kind(&variable.ty));
                    }
                    if let Some(mistake) = self.too_early(name, Some(&variable), env, span) {
                        return mistake;
                    }
                    self.read(name, variable, env, span)
                }
                None => match self.too_early(name, None, env, span) {
                    Some(mistake) => mistake,
                    None => self.error(format!("`{name}` is not defined"), span),
                },
            },

            ExprKind::Unit { value, unit } => {
                self.look(value, env);
                if !units::is_unit(unit) {
                    return self.error(units::unknown(unit), span);
                }
                Ty::Quantity(Rc::from(unit.as_str()))
            }

            ExprKind::List(items) => {
                let types: Vec<Ty> = items.iter().map(|item| self.shared(item, env)).collect();
                // An empty list holds nothing yet, so its item type is open.
                Ty::list(self.join_all(&types).unwrap_or(Ty::Dynamic))
            }
            ExprKind::ListComp { element, variable, iterable, condition } => {
                let iterable_type = self.look(iterable, env);
                let item = self.item_type(&iterable_type, iterable.span);
                let scope = Scope::child(env);
                scope.declare(variable, item, false);
                // The element is worked out once for every item: a loop.
                let entry = self.enter_loop();
                if let Some(condition) = condition {
                    self.condition(condition, &scope);
                    narrow_all(&scope, &self.narrowings(condition, &scope).0);
                }
                let element = self.shared(element, &scope);
                self.leave_loop(entry, false);
                Ty::list(element)
            }

            ExprKind::Unary { op, operand } => {
                let ty = self.look(operand, env).widened();
                match (op, &ty) {
                    (_, ty) if ty.is_loose() => ty.clone(),
                    (UnaryOp::Neg, Ty::Int | Ty::Float | Ty::Quantity(_)) => ty,
                    (UnaryOp::Not, Ty::Bool) => Ty::Bool,
                    (UnaryOp::BitNot, Ty::Int) => Ty::Int,
                    (UnaryOp::BitNot, _) => {
                        self.error(format!("`~` works on the bits of a whole number, and this is {ty}"), span)
                    }
                    (UnaryOp::Neg, _) => self.error(format!("`-` does not work on {ty}"), span),
                    (UnaryOp::Not, _) => self.error(format!("`not` needs a bool, but this is {ty}"), span),
                }
            }

            ExprKind::Binary { op: op @ (BinaryOp::And | BinaryOp::Or), left, right } => {
                let left_type = self.look(left, env);
                // The right side only runs when the left side did not settle
                // it, so it can rely on what the left side established:
                //     x is not none and x.upper() == "A"
                let scope = Scope::child(env);
                let (when_true, when_false) = self.narrowings(left, env);
                narrow_all(&scope, if *op == BinaryOp::And { &when_true } else { &when_false });
                let right_type = self.look(right, &scope);

                for (ty, operand) in [(left_type, left), (right_type, right)] {
                    if !fits(&ty, &Ty::Bool) {
                        self.error(format!("`{}` needs bools, but this is {ty}", op.text()), operand.span);
                    }
                }
                Ty::Bool
            }
            ExprKind::Binary { op, left, right } => {
                let left = self.look(left, env);
                let right = self.look(right, env);
                let ty = self.binary(*op, &left, &right, span);
                if *op == BinaryOp::Add && matches!(ty, Ty::List(_)) {
                    // `a + b` on two lists makes a third.
                    self.types.made.insert(expr as *const Expr as usize);
                }
                ty
            }
            ExprKind::Compare { first, rest } => {
                let mut left = self.look(first, env);
                for (op, operand) in rest {
                    let right = self.look(operand, env);
                    self.binary(*op, &left, &right, span);
                    left = right;
                }
                Ty::Bool
            }

            ExprKind::Function(decl) => {
                let function = Rc::new(FunctionInfo {
                    decl: decl.clone(),
                    env: env.clone(),
                    span,
                    self_type: None,
                    state: RefCell::new(State::NotChecked),
                    // It is made where it is written, so nothing it can
                    // see is still to come.
                    above: Rc::from(Vec::new()),
                    copy_note: None,
                });
                self.signature(&function);
                Ty::Function(function)
            }

            ExprKind::Call { callee, args } => {
                // If the place this call's result goes was noted, take the
                // note now: the arguments may leave notes of their own.
                let wanted = match self.expected.take() {
                    Some((call, wanted)) if call == expr as *const Expr as usize => Some(wanted),
                    _ => None,
                };
                let callee = match &callee.kind {
                    // `object.method(...)`: the call looks at the object.
                    ExprKind::Field { object, name } => {
                        let object = self.look(object, env);
                        match self.field(&object, name, callee.span) {
                            Ty::Dynamic if self.strict => self.error(
                                "strict mode: the type of this cannot be worked out before the program runs",
                                callee.span,
                            ),
                            ty => ty,
                        }
                    }
                    // A generic is only ever met here, by its own name.
                    ExprKind::Name(_) => match self.expr_inner(callee, env) {
                        Ty::Dynamic if self.strict => self.expr(callee, env),
                        ty => ty,
                    },
                    _ => self.expr(callee, env),
                };
                // Where each argument is going decides what passing it
                // means, so find that out before looking at any of them.
                let slots = self.slots(&callee);
                let only_looks = matches!(callee, Ty::Builtin(_));
                let mut arguments = Vec::new();
                let mut named_seen = false;
                for (position, arg) in args.iter().enumerate() {
                    if arg.name.is_some() {
                        named_seen = true;
                    } else if named_seen {
                        self.error("plain arguments must come before named ones", arg.value.span);
                    }
                    let slot = slots.as_ref().and_then(|slots| match &arg.name {
                        None => slots.get(position),
                        Some(name) => slots.iter().find(|slot| slot.name == *name),
                    });
                    let ty = match slot {
                        // A parameter that takes an owned value keeps it:
                        // the caller's variable gives it up.
                        Some(slot) if slot.ty.is_owned() => self.kept(&arg.value, &slot.ty, env),
                        // `print`, `len` and the rest only look.
                        _ if only_looks => self.look(&arg.value, env),
                        // So does a parameter that says `borrow`. An owned
                        // value made in the argument itself is lent for
                        // the length of the statement, which owns it.
                        Some(slot) if matches!(slot.ty, Ty::Borrowed(_)) => {
                            self.expecting(&arg.value, &slot.ty);
                            match self.expr(&arg.value, env) {
                                ty if ty.is_owned() => {
                                    self.types.lent.insert(&arg.value as *const Expr as usize);
                                    Ty::Borrowed(Box::new(ty.looked_at()))
                                }
                                ty => ty,
                            }
                        }
                        // Anything else is checked against the parameter
                        // by `call`. A look at an owned value fits there
                        // only if the parameter says `borrow`.
                        _ => {
                            if let Some(slot) = slot {
                                self.expecting(&arg.value, &slot.ty);
                            }
                            self.expr(&arg.value, env)
                        }
                    };
                    arguments.push(Argument {
                        name: arg.name.clone(),
                        ty,
                        span: arg.value.span,
                        written: written(&arg.value),
                    });
                }
                if let Ty::Generic(generic) = &callee {
                    return self.call_generic(generic, arguments, expr, wanted, span);
                }
                let ty = self.call(&callee, arguments, span);
                if let (Ty::BuiltinMethod(_, name), Ty::List(_)) = (&callee, &ty) {
                    if matches!(&**name, "split" | "map" | "filter") {
                        // Each of these makes a new list.
                        self.types.made.insert(expr as *const Expr as usize);
                    }
                }
                ty
            }

            ExprKind::Field { object, name } => {
                let object_type = self.expr(object, env);
                let tied = object_type.is_tied();
                let object = self.looked(object_type, object);
                match self.field(&object, name, span) {
                    // The method would carry the object away inside it.
                    Ty::BoundMethod(_) | Ty::BuiltinMethod(..) if tied => self.error(
                        format!("a method of an owned or borrowed value cannot be kept as a value. Call it here: `.{name}(...)`"),
                        span,
                    ),
                    // An owned value in a field stays where it is. This
                    // is a look at it.
                    ty if ty.is_owned() => Ty::Borrowed(Box::new(ty.looked_at())),
                    ty => ty,
                }
            }
            ExprKind::Index { object, index } => {
                let object = self.look(object, env);
                let index_type = self.look(index, env);
                self.expect(&index_type, &Ty::Int, "an index", index.span);
                match object {
                    Ty::List(item) => *item,
                    Ty::Str => Ty::Str,
                    ty if ty.is_loose() => ty,
                    Ty::Optional(_) => self.error(
                        format!("this is {object}. Check it with `is not none` before taking an item"),
                        span,
                    ),
                    ty => self.error(format!("an item cannot be taken from {ty}"), span),
                }
            }
        }
    }

    /// The type of `object.name`.
    fn field(&mut self, object: &Ty, name: &str, span: Span) -> Ty {
        match object {
            Ty::Instance(class) => {
                if let Some(field) = class.fields.borrow().iter().find(|f| f.name == name) {
                    return field.ty.clone();
                }
                if let Some(method) = class.methods.borrow().get(name) {
                    return Ty::BoundMethod(method.clone());
                }
                self.error(format!("`{}` has no field or method `{name}`", class.name), span)
            }
            // Which case the value is, is only known when the program runs.
            Ty::EnumValue(info) => {
                let known = info.cases.borrow().iter().any(|case| case.fields.iter().any(|f| f.name == name));
                if known {
                    Ty::Dynamic
                } else {
                    self.error(format!("no case of `{}` has a field `{name}`", info.name), span)
                }
            }
            Ty::Enum(info) => {
                let index = info.cases.borrow().iter().position(|case| case.name == name);
                match index {
                    Some(index) if info.cases.borrow()[index].fields.is_empty() => Ty::EnumValue(info.clone()),
                    Some(index) => Ty::CaseConstructor(info.clone(), index),
                    None => self.error(format!("enum `{}` has no case `{name}`", info.name), span),
                }
            }
            Ty::Module(module) => match module_member(module, name) {
                Some(ty) => ty,
                None => self.error(format!("module `{module}` has no `{name}`"), span),
            },
            // `u8.wrap`: the conversion that keeps the low bits.
            Ty::Builtin(sized) if name == "wrap" && ints::width(sized).is_some() => {
                ints::width(sized).map_or(Ty::Invalid, |width| Ty::Builtin(width.wrap))
            }
            Ty::Error(_) if name == "message" || name == "kind" => Ty::Str,
            // `(3 s).ms`: the same quantity in another unit.
            Ty::Quantity(unit) if units::is_unit(name) => match units::convert(unit, name) {
                Some(_) => Ty::Quantity(Rc::from(name)),
                None => self.error(
                    format!("a quantity in {unit} cannot be turned into {name}. They measure different things"),
                    span,
                ),
            },
            Ty::Str if STR_METHODS.contains(&name) => Ty::BuiltinMethod(Box::new(Ty::Str), Rc::from(name)),
            Ty::List(_) if LIST_METHODS.contains(&name) => {
                Ty::BuiltinMethod(Box::new(object.clone()), Rc::from(name))
            }
            Ty::Pin if PIN_METHODS.contains(&name) => Ty::BuiltinMethod(Box::new(Ty::Pin), Rc::from(name)),
            // `led.number`: which pin this is.
            Ty::Pin if name == "number" => Ty::Int,
            Ty::Optional(_) => self.error(
                format!("this is {object}. Check it with `is not none` before using `.{name}`"),
                span,
            ),
            ty if ty.is_loose() => ty.clone(),
            ty => self.error(format!("{ty} has no field or method `{name}`"), span),
        }
    }

    // ----------------------------------------------------------------- Calls

    fn call(&mut self, callee: &Ty, arguments: Vec<Argument>, span: Span) -> Ty {
        match callee {
            Ty::Function(function) => {
                let signature = self.signature(function);
                self.arguments(&signature.name, &signature.params, &arguments, span);
                for kind in &signature.raises {
                    self.can_raise(kind, span);
                }
                signature.returns.clone()
            }
            Ty::BoundMethod(function) => {
                let signature = self.signature(function);
                // `self` is already supplied by the object.
                let params = signature.params.get(1..).unwrap_or_default();
                self.arguments(&signature.name, params, &arguments, span);
                for kind in &signature.raises {
                    self.can_raise(kind, span);
                }
                signature.returns.clone()
            }
            Ty::Class(class) => {
                let fields = class.fields.borrow().clone();
                self.arguments(&class.name, &fields, &arguments, span);
                // The defaults of the fields are worked out here, where
                // the object is built. One that uses a variable declared
                // further down has nothing to use yet.
                let missing = not_yet(&class.above, &class.env);
                for (field, names) in &class.defaults {
                    if let Some(name) = names.iter().find(|name| missing.contains(name)) {
                        self.error(
                            format!(
                                "`{}` cannot be built here yet. The default of its field `{field}` uses `{name}`, \
                                 which is declared further down. Build it below the line that declares `{name}`",
                                class.name
                            ),
                            span,
                        );
                        break;
                    }
                }
                Ty::Instance(class.clone())
            }
            Ty::CaseConstructor(info, index) => {
                let case_name = info.cases.borrow()[*index].name.clone();
                let fields = info.cases.borrow()[*index].fields.clone();
                self.arguments(&case_name, &fields, &arguments, span);
                Ty::EnumValue(info.clone())
            }
            Ty::ErrorType(kind) => {
                if arguments.len() > 1 || arguments.iter().any(|a| a.name.is_some()) {
                    self.error(format!("`{kind}` takes one message"), span);
                }
                Ty::Error(kind.clone())
            }
            Ty::Builtin(name) => {
                self.no_named(name, &arguments, span);
                self.builtin(name, &arguments, span)
            }
            Ty::BuiltinMethod(receiver, name) => {
                self.no_named(name, &arguments, span);
                self.method(receiver, name, &arguments, span)
            }
            Ty::Callable(callable) => {
                self.no_named("this function", &arguments, span);
                if arguments.len() != callable.params.len() {
                    return self.error(
                        format!(
                            "this function takes {} argument(s), but {} were given",
                            callable.params.len(),
                            arguments.len()
                        ),
                        span,
                    );
                }
                for (argument, wanted) in arguments.iter().zip(&callable.params) {
                    let given = self.adapted(argument.written.clone(), argument.ty.clone(), wanted, argument.span);
                    self.expect(&given, wanted, "an argument", argument.span);
                }
                callable.returns.clone()
            }
            ty if ty.is_loose() => {
                // Whatever is being called, it receives these untracked.
                for argument in &arguments {
                    self.escapes(&argument.ty);
                }
                ty.clone()
            }
            Ty::Generic(generic) => self.error(
                format!("`{}` is generic, so it is called by its own name and cannot be kept as a value yet", generic.name),
                span,
            ),
            ty => self.error(format!("{ty} cannot be called"), span),
        }
    }

    fn no_named(&mut self, what: &str, arguments: &[Argument], span: Span) {
        if let Some(name) = arguments.iter().find_map(|a| a.name.as_ref()) {
            self.error(format!("`{what}` does not take a named argument `{name}`"), span);
        }
    }

    /// Check the arguments of a call against the slots they are for. This
    /// follows the interpreter's `bind`, with types added.
    fn arguments(&mut self, what: &str, slots: &[Slot], arguments: &[Argument], span: Span) {
        let positional = arguments.iter().filter(|a| a.name.is_none()).count();
        if positional > slots.len() {
            self.error(
                format!("`{what}` takes {} argument(s), but {positional} were given", slots.len()),
                span,
            );
            return;
        }

        let mut filled = vec![false; slots.len()];
        let mut next = 0;
        for argument in arguments {
            let index = match &argument.name {
                None => {
                    next += 1;
                    next - 1
                }
                Some(name) => match slots.iter().position(|slot| slot.name == *name) {
                    Some(index) => index,
                    None => {
                        self.error(format!("`{what}` has no argument named `{name}`"), argument.span);
                        continue;
                    }
                },
            };
            if filled[index] {
                self.error(format!("`{what}` was given `{}` twice", slots[index].name), argument.span);
                continue;
            }
            filled[index] = true;
            self.sees(&slots[index], &argument.ty, argument.span);
            let label = format!("`{}` of `{what}`", slots[index].name);
            let given =
                self.adapted(argument.written.clone(), argument.ty.clone(), &slots[index].ty, argument.span);
            self.expect(&given, &slots[index].ty, &label, argument.span);
        }

        for (slot, filled) in slots.iter().zip(filled) {
            if !filled && !slot.has_default {
                self.error(format!("`{what}` needs a value for `{}`", slot.name), span);
            }
        }
    }

    /// Check the number of arguments given to a built-in.
    fn arity(&mut self, name: &str, arguments: &[Argument], min: usize, max: usize, span: Span) -> bool {
        if (min..=max).contains(&arguments.len()) {
            return true;
        }
        let wanted = if min == max { min.to_string() } else { format!("{min} to {max}") };
        self.error(
            format!("`{name}` takes {wanted} argument(s), but {} were given", arguments.len()),
            span,
        );
        false
    }

    /// The result of calling a built-in function.
    fn builtin(&mut self, name: &str, arguments: &[Argument], span: Span) -> Ty {
        // The built-ins work on plain ints: `abs` of an i8 is an int.
        let first = arguments.first().map(|a| a.ty.widened()).unwrap_or(Ty::Invalid);
        let wrong = |checker: &mut Checker, wanted: &str| {
            checker.error(format!("`{name}` needs {wanted}, but this is {first}"), span)
        };

        match name {
            "print" => Ty::None,
            "str" => {
                self.arity(name, arguments, 1, 1, span);
                Ty::Str
            }
            "len" => {
                if self.arity(name, arguments, 1, 1, span) && !matches!(first, Ty::Str | Ty::List(_)) && !first.is_loose() {
                    wrong(self, "a str or a list");
                }
                Ty::Int
            }
            "range" => {
                self.arity(name, arguments, 1, 3, span);
                for argument in arguments {
                    self.expect(&argument.ty, &Ty::Int, "an argument of `range`", argument.span);
                }
                Ty::list(Ty::Int)
            }
            "int" | "float" => {
                if self.arity(name, arguments, 1, 1, span) {
                    match first {
                        Ty::Int | Ty::Float => {}
                        // Text from outside may not be a number at all.
                        Ty::Str | Ty::Dynamic => self.can_raise("ValueError", span),
                        Ty::Invalid => {}
                        _ => {
                            wrong(self, "a number or a str");
                        }
                    }
                }
                if name == "int" { Ty::Int } else { Ty::Float }
            }
            "abs" => {
                if !self.arity(name, arguments, 1, 1, span) {
                    return Ty::Invalid;
                }
                match first {
                    Ty::Int | Ty::Float | Ty::Quantity(_) => first,
                    ty if ty.is_loose() => ty,
                    _ => wrong(self, "a number"),
                }
            }
            "min" | "max" | "sum" => {
                // Either one list, or the numbers themselves.
                let types: Vec<Ty> = match arguments {
                    [Argument { ty: Ty::List(item), .. }] => vec![item.widened()],
                    _ => arguments.iter().map(|a| a.ty.widened()).collect(),
                };
                let joined = self.join_all(&types).unwrap_or(Ty::Int);
                if !joined.is_number() && !joined.is_loose() {
                    return self.error(format!("`{name}` needs numbers, but this is {joined}"), span);
                }
                joined
            }
            "sqrt" | "sin" | "cos" | "floor" | "ceil" => {
                if self.arity(name, arguments, 1, 1, span) && !first.is_number() && !first.is_loose() {
                    wrong(self, "a number");
                }
                if matches!(name, "floor" | "ceil") { Ty::Int } else { Ty::Float }
            }
            "sleep" => {
                // A time, and not a voltage: `sleep(3 V)` is refused here
                // and not when the program reaches it.
                let time = matches!(&first, Ty::Quantity(unit) if units::convert(unit, "s").is_some());
                if self.arity(name, arguments, 1, 1, span) && !time && !first.is_loose() {
                    wrong(self, "a time with a unit, such as `500 ms`");
                }
                Ty::None
            }
            "now" => {
                self.arity(name, arguments, 0, 0, span);
                Ty::Quantity(Rc::from("ms"))
            }
            // `u8(n)` checks that `n` fits when the program runs, and
            // `u8.wrap(n)` keeps its low bits. Either way the answer is a u8.
            _ if ints::width(name).or(ints::wrapping(name)).is_some() => {
                let width = ints::width(name).or(ints::wrapping(name)).unwrap_or(ints::WIDTHS[0]);
                if !self.arity(name, arguments, 1, 1, span) {
                    return Ty::Invalid;
                }
                if !matches!(first, Ty::Int) && !first.is_loose() {
                    let hint = if matches!(first, Ty::Float) { ". Make it one with `int( )` first" } else { "" };
                    return self.error(format!("`{name}` needs a whole number, but this is {first}{hint}"), span);
                }
                // A number written out is checked now. Only `u8( )`
                // refuses one: `u8.wrap(300)` is asking for the 44.
                if let (Some(Written::Number(n)), true) = (&arguments[0].written, ints::width(name).is_some()) {
                    if !i64::try_from(*n).is_ok_and(|n| width.holds(n)) {
                        return self.error(width.does_not_fit(n), arguments[0].span);
                    }
                }
                Ty::Sized(width)
            }
            "pin" => {
                if self.arity(name, arguments, 1, 1, span) {
                    self.expect(&first, &Ty::Int, "the number of a pin", arguments[0].span);
                }
                Ty::Pin
            }
            _ => Ty::Dynamic,
        }
    }

    /// The result of calling a built-in method on a str or a list.
    fn method(&mut self, receiver: &Ty, name: &str, arguments: &[Argument], span: Span) -> Ty {
        let strs = |checker: &mut Checker| {
            for argument in arguments {
                checker.expect(&argument.ty, &Ty::Str, &format!("an argument of `{name}`"), argument.span);
            }
        };
        // Call a function argument with the given argument types.
        let apply = |checker: &mut Checker, function: &Argument, types: Vec<Ty>| {
            let arguments = types
                .into_iter()
                .map(|ty| Argument { name: None, ty, span: function.span, written: None })
                .collect();
            checker.call(&function.ty, arguments, function.span)
        };

        match (receiver, name) {
            (Ty::Pin, _) => {
                self.arity(name, arguments, 0, 0, span);
                if name == "is_on" { Ty::Bool } else { Ty::None }
            }
            (Ty::Str, "upper" | "lower" | "strip") => {
                self.arity(name, arguments, 0, 0, span);
                Ty::Str
            }
            (Ty::Str, "split") => {
                self.arity(name, arguments, 0, 1, span);
                strs(self);
                Ty::list(Ty::Str)
            }
            (Ty::Str, "replace") => {
                self.arity(name, arguments, 2, 2, span);
                strs(self);
                Ty::Str
            }
            (Ty::Str, _) => {
                self.arity(name, arguments, 1, 1, span);
                strs(self);
                Ty::Bool
            }

            (Ty::List(item), "append") => {
                if self.arity(name, arguments, 1, 1, span) {
                    let added = &arguments[0];
                    let given = self.adapted(added.written.clone(), added.ty.clone(), item, added.span);
                    self.expect(&given, item, "an item added to this list", added.span);
                }
                Ty::None
            }
            (Ty::List(item), "pop") => {
                self.arity(name, arguments, 0, 0, span);
                (**item).clone()
            }
            (Ty::List(item), "map") => {
                if !self.arity(name, arguments, 1, 1, span) {
                    return Ty::Invalid;
                }
                Ty::list(apply(self, &arguments[0], vec![(**item).clone()]))
            }
            (Ty::List(item), "filter") => {
                if self.arity(name, arguments, 1, 1, span) {
                    let result = apply(self, &arguments[0], vec![(**item).clone()]);
                    self.expect(&result, &Ty::Bool, "what the function given to `filter` returns", arguments[0].span);
                }
                receiver.clone()
            }
            (Ty::List(item), "reduce") => {
                if !self.arity(name, arguments, 2, 2, span) {
                    return Ty::Invalid;
                }
                let start = arguments[1].ty.clone();
                let result = apply(self, &arguments[0], vec![start.clone(), (**item).clone()]);
                self.join(&start, &result)
            }
            _ => Ty::Dynamic,
        }
    }

    // ------------------------------------------- Generic functions and classes
    //
    // `function first[T](items: list[T]) -> T` is not one function. It is
    // a pattern for as many as the program needs: one for lists of ints,
    // one for lists of strs. Each is a **copy**: the function's own
    // syntax tree, copied, with `T` standing for one type, under a name
    // such as `first[int]`. A copy is an ordinary function. It is checked
    // like one, and the compilers compile it like one, which is why none
    // of the stages after this one needed to learn what `T` is.
    //
    // The work here is in three steps:
    //
    // 1. `infer`: at a call, find what each type parameter must be, by
    //    laying the written parameter types over the types of the
    //    arguments (`bind`).
    // 2. `copy_function` / `copy_class`: make the copy for those types,
    //    or find the one made earlier.
    // 3. Carry on as if the copy had been called by name.
    //
    // A generic class works the same way, with its fields as the
    // parameters. `Stack[int]` written as a type asks for a copy too.

    /// Check the names in `[T, U]` after a function's or class's name.
    /// The capital letter keeps them apart from `int`, `str` and `list`.
    /// Inside the generic the name wins over anything else called the
    /// same, so `A` and `V` are type parameters there and not amperes
    /// and volts.
    fn type_params(&mut self, names: &[String], span: Span) {
        for (index, name) in names.iter().enumerate() {
            if !name.starts_with(|c: char| c.is_ascii_uppercase()) {
                self.error(
                    format!("a type parameter starts with a capital letter, as in `T` or `Item`, and this is `{name}`"),
                    span,
                );
            } else if names[..index].contains(name) {
                self.error(format!("the type parameter `{name}` is written twice"), span);
            }
        }
    }

    /// Note that `value` is going where a `wanted` is wanted, in case it
    /// is a call that needs to know: `let s: Stack[int] = Stack()`.
    fn expecting(&mut self, value: &Expr, wanted: &Ty) {
        let says_something = !matches!(wanted, Ty::Dynamic) && !has_param(wanted);
        if matches!(value.kind, ExprKind::Call { .. }) && says_something {
            // `Invalid` is noted too: a mistake in the written type was
            // reported there, and the call should not complain as well.
            self.expected = Some((value as *const Expr as usize, wanted.clone()));
        }
    }

    /// The parameters of a generic function (or the fields of a generic
    /// class) and what it returns, with the type parameters left in.
    fn outline(&mut self, generic: &Rc<Generic>) -> Rc<Outline> {
        if let Some(outline) = generic.outline.borrow().clone() {
            return outline;
        }
        // While this is worked out, `T` means "the type parameter T".
        let env = Scope::child(&generic.env);
        for name in &generic.params {
            env.declare(name, Ty::Alias(Box::new(Ty::Param(Rc::from(name.as_str())))), false);
        }
        let span = generic.span;
        let mut slots = Vec::new();
        let returns = match &generic.template {
            Template::Function(decl) => {
                for param in &decl.params {
                    let ty = match &param.ty {
                        Some(ty) => self.resolve_for(ty, &env, span, true),
                        None => Ty::Invalid,
                    };
                    slots.push(Slot { name: param.name.clone(), ty, has_default: param.default.is_some(), id: None });
                }
                decl.return_type.as_ref().map(|ty| self.resolve(ty, &env, span))
            }
            Template::Class(decl) => {
                for field in &decl.fields {
                    let ty = match &field.ty {
                        Some(ty) => self.resolve(ty, &env, span),
                        None => Ty::Invalid,
                    };
                    slots.push(Slot { name: field.name.clone(), ty, has_default: field.default.is_some(), id: None });
                }
                // Building one gives the class with every parameter in place.
                let params = generic.params.iter().map(|name| Ty::Param(Rc::from(name.as_str()))).collect();
                Some(Ty::Applied(generic.clone(), params))
            }
        };
        let outline = Rc::new(Outline { slots, returns });
        *generic.outline.borrow_mut() = Some(outline.clone());
        outline
    }

    /// A call of a generic function, or the building of an object of a
    /// generic class. `wanted` is the type of the place the result goes,
    /// if that is known.
    fn call_generic(
        &mut self,
        generic: &Rc<Generic>,
        arguments: Vec<Argument>,
        call: &Expr,
        wanted: Option<Ty>,
        span: Span,
    ) -> Ty {
        // The compilers find the copy by the call. `expr` has seen to it
        // that a generic is only ever called by its own name.
        let Some(types) = self.infer(generic, &arguments, wanted, span) else {
            return Ty::Invalid;
        };
        let copy = match &generic.template {
            Template::Function(_) => self.copy_function(generic, types, Some(span)).map(|copy| {
                self.types.copy_called.insert(call as *const Expr as usize, copy.decl.name.clone());
                Ty::Function(copy)
            }),
            Template::Class(_) => self.copy_class(generic, types, Some(span)).map(|copy| {
                self.types.copy_called.insert(call as *const Expr as usize, copy.name.clone());
                Ty::Class(copy)
            }),
        };
        match copy {
            // From here on it is an ordinary call of an ordinary function.
            Some(copy) => self.call(&copy, arguments, span),
            None => Ty::Invalid,
        }
    }

    /// What each type parameter of `generic` is at this call, in the order
    /// they are declared. `None` if they cannot all be told, which is an
    /// error unless a later pass may still settle it.
    fn infer(
        &mut self,
        generic: &Rc<Generic>,
        arguments: &[Argument],
        wanted: Option<Ty>,
        span: Span,
    ) -> Option<Vec<Ty>> {
        let outline = self.outline(generic);
        // Which parameter each argument is for, as `arguments` works it out.
        let mut pairs: Vec<(&Slot, &Argument)> = Vec::new();
        let mut next = 0;
        for argument in arguments {
            let slot = match &argument.name {
                None => {
                    next += 1;
                    outline.slots.get(next - 1)
                }
                Some(name) => outline.slots.iter().find(|slot| slot.name == *name),
            };
            pairs.extend(slot.map(|slot| (slot, argument)));
        }

        // First, what the arguments say. A number that is written out,
        // such as the `0` of `ring(4, 0)`, waits until last: it can be
        // an int or a u8, and something else may say which.
        let mut found: HashMap<String, Ty> = HashMap::new();
        for (slot, argument) in pairs.iter().filter(|(_, argument)| argument.written.is_none()) {
            self.bind(&slot.ty, &argument.ty, &mut found);
        }
        // An argument that is a function says what it returns only once it
        // is known what it will be called with: in `apply(5, double)`
        // with `f: function(T) -> U`, the 5 gives `T`, and `double` called
        // with a `T` gives `U`.
        for (slot, argument) in &pairs {
            let Ty::Callable(pattern) = &slot.ty else {
                continue;
            };
            let takes: Option<Vec<Ty>> = pattern.params.iter().map(|ty| self.filled(ty, &found, span)).collect();
            let (Some(takes), true) = (takes, self.filled(&pattern.returns, &found, span).is_none()) else {
                continue;
            };
            // A built-in such as `len` says what it returns when called too.
            let fits = matches!(argument.ty, Ty::Builtin(_))
                || self.callable(&argument.ty).is_some_and(|(slots, _)| slots.len() == takes.len());
            if fits {
                let takes =
                    takes.into_iter().map(|ty| Argument { name: None, ty, span: argument.span, written: None }).collect();
                let returns = self.call(&argument.ty, takes, argument.span);
                self.bind(&pattern.returns, &returns, &mut found);
            }
        }
        // Then, for what no argument has settled, the place the result
        // goes: `let bytes: Ring[u8] = ring(4, 0)`.
        if let (Some(returns), Some(wanted)) = (&outline.returns, &wanted) {
            let mut from_wanted = HashMap::new();
            self.bind(returns, wanted, &mut from_wanted);
            for (name, ty) in from_wanted {
                found.entry(name).or_insert(ty);
            }
        }
        // Last, the written numbers, as the ints they are if nothing
        // else has spoken.
        let mut from_written = HashMap::new();
        for (slot, argument) in pairs.iter().filter(|(_, argument)| argument.written.is_some()) {
            self.bind(&slot.ty, &argument.ty, &mut from_written);
        }
        for (name, ty) in from_written {
            found.entry(name).or_insert(ty);
        }

        let unsettled = !self.last_round
            || matches!(wanted, Some(Ty::Invalid))
            || arguments.iter().any(|argument| matches!(argument.ty, Ty::Invalid));
        let loose = arguments.iter().any(|argument| argument.ty.has_dynamic());
        let mut types = Vec::new();
        for name in &generic.params {
            let ty = match found.get(name) {
                Some(ty) => ty.clone(),
                // In a program with types left out, an argument whose type
                // is only known at run time makes `T` the same.
                None if loose && !self.strict => Ty::Dynamic,
                None if unsettled => return None,
                None => {
                    // An argument that was meant to say, and is the wrong
                    // shape to: a 5 where a `list[T]` goes. (An empty
                    // list is the right shape, and says nothing.)
                    let wrong = pairs
                        .iter()
                        .find(|(slot, argument)| mentions(&slot.ty, Some(name)) && !argument.ty.has_dynamic());
                    if let Some((slot, argument)) = wrong {
                        self.error(
                            format!(
                                "`{}` of `{}` should be {}, but this is {}",
                                slot.name, generic.name, slot.ty, argument.ty
                            ),
                            argument.span,
                        );
                        return None;
                    }
                    let example = match &generic.template {
                        Template::Class(_) => format!(
                            "let x: {0}[{1}] = {0}(...)",
                            generic.name,
                            vec!["int"; generic.params.len()].join(", ")
                        ),
                        Template::Function(_) => format!("let x: list[int] = {}(...)", generic.name),
                    };
                    self.error(
                        format!(
                            "`{}` cannot tell what `{name}` is from this call: no argument says. \
                             Put the result where its type is written, as in `{example}`",
                            generic.name
                        ),
                        span,
                    );
                    return None;
                }
            };
            if ty.is_tied() {
                self.error(format!("`{name}` would be {ty} here. An owned or borrowed value cannot fill a type parameter yet"), span);
                return None;
            }
            types.push(ty);
        }
        Some(types)
    }

    /// Lay the pattern `pattern` over the type `actual`, and note what each
    /// type parameter in it has to be: `list[T]` over `list[int]` says
    /// that `T` is `int`. Where the two do not line up, nothing is noted
    /// and nothing is reported: the ordinary check of the arguments
    /// against the copy will say what is wrong, in terms of real types.
    fn bind(&mut self, pattern: &Ty, actual: &Ty, found: &mut HashMap<String, Ty>) {
        match (pattern, actual) {
            // Not settled, or only known at run time: it says nothing.
            (_, Ty::Invalid | Ty::Dynamic) => {}
            (Ty::Param(name), _) => {
                // A function is known by what it takes and returns.
                let actual = match actual {
                    Ty::Function(_) | Ty::BoundMethod(_) => match self.callable(actual) {
                        Some((slots, returns)) if !slots.iter().any(|slot| slot.ty.is_loose()) && !returns.is_loose() => {
                            let params = slots.into_iter().map(|slot| slot.ty).collect();
                            Ty::Callable(Rc::new(CallableType { params, returns }))
                        }
                        _ => return,
                    },
                    other => other.clone(),
                };
                let merged = match found.get(name.as_ref()) {
                    None => actual,
                    // Two arguments, one `T`. A u8 and an int make it an
                    // int; `none` and an int make it `int or none`. Two
                    // types with nothing in common leave the first, and
                    // the check of the arguments reports the second.
                    Some(old) if fits(&actual, old) => return,
                    Some(old) if fits(old, &actual) => actual,
                    Some(old) if matches!(old, Ty::None) || matches!(actual, Ty::None) => join(old, &actual),
                    Some(_) => return,
                };
                found.insert(name.to_string(), merged);
            }
            (Ty::List(pattern), Ty::List(actual))
            | (Ty::Optional(pattern), Ty::Optional(actual))
            | (Ty::Owned(pattern), Ty::Owned(actual))
            | (Ty::Borrowed(pattern), Ty::Borrowed(actual)) => self.bind(pattern, actual, found),
            // A plain value where `T or none` is wanted, a value lent to a
            // `borrow`, and a result that goes into an `or none`.
            (Ty::Optional(pattern), actual) if !matches!(actual, Ty::None) => self.bind(pattern, actual, found),
            (Ty::Borrowed(pattern), actual) => self.bind(pattern, actual, found),
            (pattern, Ty::Optional(actual)) => self.bind(pattern, actual, found),
            // `Stack[T]` over `Stack[int]`.
            (Ty::Applied(generic, patterns), Ty::Instance(class)) => {
                if let Some((origin, types)) = &class.origin {
                    if Rc::ptr_eq(origin, generic) {
                        for (pattern, actual) in patterns.iter().zip(types) {
                            self.bind(pattern, actual, found);
                        }
                    }
                }
            }
            (Ty::Callable(pattern), Ty::Callable(actual)) => {
                for (pattern, actual) in pattern.params.iter().zip(&actual.params) {
                    self.bind(pattern, actual, found);
                }
                self.bind(&pattern.returns, &actual.returns, found);
            }
            (Ty::Callable(pattern), Ty::Function(_) | Ty::BoundMethod(_)) => {
                if let Some((slots, returns)) = self.callable(actual) {
                    for (pattern, slot) in pattern.params.iter().zip(&slots) {
                        self.bind(pattern, &slot.ty, found);
                    }
                    self.bind(&pattern.returns, &returns, found);
                }
            }
            _ => {}
        }
    }

    /// `pattern` with each type parameter replaced by what was found for
    /// it. `None` if one of them has not been found.
    fn filled(&mut self, pattern: &Ty, found: &HashMap<String, Ty>, span: Span) -> Option<Ty> {
        Some(match pattern {
            Ty::Param(name) => found.get(name.as_ref())?.clone(),
            Ty::List(inner) => Ty::list(self.filled(inner, found, span)?),
            Ty::Optional(inner) => Ty::Optional(Box::new(self.filled(inner, found, span)?)),
            Ty::Owned(inner) => Ty::Owned(Box::new(self.filled(inner, found, span)?)),
            Ty::Borrowed(inner) => Ty::Borrowed(Box::new(self.filled(inner, found, span)?)),
            Ty::Callable(callable) => {
                let params: Option<Vec<Ty>> = callable.params.iter().map(|ty| self.filled(ty, found, span)).collect();
                let returns = self.filled(&callable.returns, found, span)?;
                Ty::Callable(Rc::new(CallableType { params: params?, returns }))
            }
            Ty::Applied(generic, types) => {
                let types: Option<Vec<Ty>> = types.iter().map(|ty| self.filled(ty, found, span)).collect();
                Ty::Instance(self.copy_class(generic, types?, Some(span))?)
            }
            other => other.clone(),
        })
    }

    /// The name of the copy of `generic` for `types`, the scope in which
    /// its type parameters mean those types, and the note that goes under
    /// an error found inside it. `None` if the copies have got out of hand.
    fn copy_setting(&mut self, generic: &Rc<Generic>, types: &[Ty], asked: Option<Span>) -> Option<(String, Env, String)> {
        let spelled: Vec<String> = types.iter().map(spelled).collect();
        let name = format!("{}[{}]", generic.name, spelled.join(", "));
        // `class Node[T] { next: Node[list[T]] or none }` asks for
        // Node[int], Node[list[int]], Node[list[list[int]]], with no end.
        if name.len() > 300 || self.copies.len() > 2000 {
            let start: String = name.chars().take(60).collect();
            self.error(
                format!(
                    "`{}` needs a copy of itself for a bigger type every time, and that never ends. \
                     It got as far as `{start}...`",
                    generic.name
                ),
                asked.unwrap_or(generic.span),
            );
            return None;
        }
        let env = Scope::child(&generic.env);
        let mut means = Vec::new();
        for (param, ty) in generic.params.iter().zip(types) {
            env.declare(param, Ty::Alias(Box::new(ty.clone())), false);
            means.push(format!("{param} = {}", spelled.get(means.len()).cloned().unwrap_or_default()));
        }
        let mut note = format!("this is inside `{name}`: `{}` with {}", generic.name, means.join(", "));
        if let Some(asked) = asked {
            note += &format!(", which line {} asks for", asked.line);
        }
        Some((name, env, note))
    }

    /// The copy of a generic function for these types: an ordinary
    /// function called `first[int]`. `asked` is where the program asks
    /// for it. With every type `Invalid` it is the generic checked on its
    /// own (see `finish_generics`), which keeps its plain name.
    fn copy_function(&mut self, generic: &Rc<Generic>, types: Vec<Ty>, asked: Option<Span>) -> Option<Rc<FunctionInfo>> {
        let Template::Function(template) = &generic.template else {
            return None;
        };
        let alone = types.iter().all(|ty| matches!(ty, Ty::Invalid));
        let key = (generic.at, if alone { String::new() } else { types.iter().map(spelled).collect::<Vec<_>>().join(", ") });
        if let Some(Made::Function(copy)) = self.copies.get(&key) {
            return Some(copy.clone());
        }
        let (name, env, note) = self.copy_setting(generic, &types, asked)?;
        let mut decl = template.fresh();
        decl.type_params.clear();
        if !alone {
            decl.name = name;
        }
        let copy = Rc::new(FunctionInfo {
            decl: Arc::new(decl),
            env,
            span: generic.span,
            self_type: None,
            state: RefCell::new(State::NotChecked),
            above: generic.above.clone(),
            copy_note: (!alone).then_some(note),
        });
        self.copies.insert(key, Made::Function(copy.clone()));
        if !alone {
            self.used.insert(generic.at);
            self.types.function_copies.entry(generic.at).or_default().push(copy.decl.clone());
        }
        Some(copy)
    }

    /// The copy of a generic class for these types: an ordinary class
    /// called `Stack[int]`.
    fn copy_class(&mut self, generic: &Rc<Generic>, types: Vec<Ty>, asked: Option<Span>) -> Option<Rc<ClassInfo>> {
        let Template::Class(template) = &generic.template else {
            return None;
        };
        // Checking the generic on its own fills every parameter with
        // `Invalid`, the type that is accepted everywhere.
        let alone = types.iter().all(|ty| matches!(ty, Ty::Invalid));
        let key = (generic.at, if alone { String::new() } else { types.iter().map(spelled).collect::<Vec<_>>().join(", ") });
        if let Some(Made::Class(copy)) = self.copies.get(&key) {
            return Some(copy.clone());
        }
        let (name, env, note) = self.copy_setting(generic, &types, asked)?;
        let mut decl = template.fresh();
        decl.type_params.clear();
        if !alone {
            decl.name = name;
        }
        let decl = Arc::new(decl);
        let origin = (!alone).then(|| (generic.clone(), types));
        let copy = class_info(&decl, &env, &generic.above, generic.span, origin, (!alone).then_some(note));
        // Noted before its fields are worked out: a class may mention itself.
        self.copies.insert(key, Made::Class(copy.clone()));
        if !alone {
            self.used.insert(generic.at);
            self.classes.insert(copy.id.clone(), copy.clone());
            self.types.class_copies.entry(generic.at).or_default().push(decl.clone());
        }
        self.fill_class(&decl, &copy, &env, generic.span);
        self.pending.push((decl, copy.clone()));
        Some(copy)
    }

    /// The end of a pass: check the bodies of the class copies that were
    /// made along the way, and every generic that nothing used.
    fn finish_generics(&mut self) {
        let mut alone = 0;
        loop {
            if let Some((decl, class)) = self.pending.pop() {
                self.copy_notes.push(class.copy_note.clone());
                let env = class.env.clone();
                self.class_body(&decl, &class, &env);
                self.copy_notes.pop();
                continue;
            }
            // A generic nobody uses has no copy to check. So that a
            // mistake in it is still found, it is checked once with its
            // type parameters standing for "anything". That finds a
            // misspelt name or a wrong number of arguments, though not a
            // `+` that only some types have.
            let Some(generic) = self.generics.get(alone).cloned() else {
                break;
            };
            alone += 1;
            if !self.used.contains(&generic.at) {
                let anything = vec![Ty::Invalid; generic.params.len()];
                match &generic.template {
                    Template::Function(_) => {
                        if let Some(copy) = self.copy_function(&generic, anything, None) {
                            self.signature(&copy);
                        }
                    }
                    Template::Class(_) => {
                        self.copy_class(&generic, anything, None);
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------- Operators

    /// The type of `left op right`. The rules are the interpreter's, applied
    /// to types.
    fn binary(&mut self, op: BinaryOp, left: &Ty, right: &Ty, span: Span) -> Ty {
        use BinaryOp::*;
        let comparison = matches!(op, Eq | NotEq | Lt | LtEq | Gt | GtEq | In | NotIn | Is | IsNot);
        // A sized number takes part as the int it is, and the answer is an
        // int: `a + b` on two u8s is never cut down to eight bits.
        let (left, right) = (&left.widened(), &right.widened());

        // With a side that is only known at run time, so is the result,
        // except that a comparison is always a bool.
        if left.is_loose() || right.is_loose() {
            return match (left, right) {
                _ if comparison => Ty::Bool,
                (Ty::Invalid, _) | (_, Ty::Invalid) => Ty::Invalid,
                _ => Ty::Dynamic,
            };
        }

        let numbers = left.is_number() && right.is_number();
        let ints = matches!((left, right), (Ty::Int, Ty::Int));
        // Two quantities that measure the same thing, such as `s` and `ms`,
        // work together. The answer is in the finer unit of the two.
        let shared_unit = match (left, right) {
            (Ty::Quantity(a), Ty::Quantity(b)) => units::common(a, b).map(|(unit, _, _)| unit),
            _ => None,
        };
        let same_unit = shared_unit.is_some();

        let result = match op {
            Add | Sub if ints => Some(Ty::Int),
            Add | Sub if numbers => Some(Ty::Float),
            Add | Sub if same_unit => shared_unit.map(|unit| Ty::Quantity(Rc::from(unit))),
            Add => match (left, right) {
                (Ty::Str, Ty::Str) => Some(Ty::Str),
                (Ty::List(a), Ty::List(b)) => Some(Ty::list(join(a, b))),
                _ => None,
            },

            Mul if ints => Some(Ty::Int),
            Mul if numbers => Some(Ty::Float),
            Mul => match (left, right) {
                (Ty::Quantity(_), other) if other.is_number() => Some(left.clone()),
                (other, Ty::Quantity(_)) if other.is_number() => Some(right.clone()),
                _ => None,
            },

            Div if numbers || same_unit => Some(Ty::Float),
            Div => match (left, right) {
                (Ty::Quantity(_), other) if other.is_number() => Some(left.clone()),
                _ => None,
            },

            FloorDiv | Mod | Pow if ints => Some(Ty::Int),
            FloorDiv | Mod | Pow if numbers => Some(Ty::Float),

            BitAnd | BitOr | BitXor | Shl | Shr if ints => Some(Ty::Int),
            // Anything else has no bits to speak of. Two bools are the
            // likely slip, by someone who writes C.
            BitAnd | BitOr | BitXor | Shl | Shr => {
                let hint = match (op, left, right) {
                    (BitAnd, Ty::Bool, Ty::Bool) => ". For two bools Zeno spells this `and`",
                    (BitOr, Ty::Bool, Ty::Bool) => ". For two bools Zeno spells this `or`",
                    _ => "",
                };
                return self.error(
                    format!("`{}` works on the bits of whole numbers, and these are {left} and {right}{hint}", op.text()),
                    span,
                );
            }

            Eq | NotEq => {
                // Two types that can never hold the same value: the answer
                // is already known, which means the comparison is a mistake.
                let (left, right) = (&left.as_ints(), &right.as_ints());
                if !numbers && !same_unit && !fits(left, right) && !fits(right, left) {
                    let answer = if op == Eq { "false" } else { "true" };
                    self.error(
                        format!("`{}` between {left} and {right} is always {answer}", op.text()),
                        span,
                    );
                }
                Some(Ty::Bool)
            }
            Lt | LtEq | Gt | GtEq => {
                let both_strs = matches!((left, right), (Ty::Str, Ty::Str));
                (numbers || both_strs || same_unit).then_some(Ty::Bool)
            }
            In | NotIn => match (left, &right.as_ints()) {
                (item, Ty::List(items)) if fits(item, items) || fits(items, item) => Some(Ty::Bool),
                (Ty::Str, Ty::Str) => Some(Ty::Bool),
                _ => None,
            },
            Is | IsNot => Some(Ty::Bool),
            _ => None,
        };

        match result {
            Some(ty) => ty,
            None => {
                let mut message = format!("`{}` does not work between {left} and {right}", op.text());
                if matches!(left, Ty::Optional(_)) || matches!(right, Ty::Optional(_)) {
                    message += ". Check for `none` first";
                }
                let ty = self.error(message, span);
                if comparison { Ty::Bool } else { ty }
            }
        }
    }
}

const STR_METHODS: &[&str] = &["upper", "lower", "strip", "split", "replace", "starts_with", "ends_with"];
const LIST_METHODS: &[&str] = &["append", "pop", "map", "filter", "reduce"];
const PIN_METHODS: &[&str] = &["on", "off", "toggle", "is_on"];

/// How much of the matched type a pattern accepts.
enum Covers {
    /// Any value: `_`, or a new name.
    Everything,
    /// The whole of one enum case.
    Case(usize),
    /// Only some values.
    Some,
}

/// The error for an owned variable used from another function's body.
fn elsewhere(name: &str) -> String {
    format!(
        "`{name}` holds an owned value, which belongs to the block that declares it. \
         A function written elsewhere cannot reach it. Lend it to the function through a `borrow` parameter"
    )
}

/// What the checker keeps about a class, before its fields are worked out.
fn class_info(
    decl: &ast::Class,
    env: &Env,
    above: &Rc<[String]>,
    span: Span,
    origin: Option<(Rc<Generic>, Vec<Ty>)>,
    copy_note: Option<String>,
) -> Rc<ClassInfo> {
    let defaults = decl
        .fields
        .iter()
        .filter_map(|field| {
            let mut names = Vec::new();
            names_in(field.default.as_ref()?, &mut names);
            Some((field.name.clone(), names))
        })
        .collect();
    Rc::new(ClassInfo {
        id: decl_id(span, &decl.name),
        name: decl.name.clone(),
        owned: decl.owned,
        fields: RefCell::new(Vec::new()),
        methods: RefCell::new(HashMap::new()),
        above: above.clone(),
        env: env.clone(),
        defaults,
        origin,
        copy_note,
    })
}

fn decl_id(span: Span, name: &str) -> DeclId {
    (span.line, span.col, name.to_string())
}

/// A loop body runs more than once, so what was known about a variable on
/// the way into the loop is not known on the second trip if the body gives
/// the variable a new value. Start the body knowing nothing about those.
fn forget_assigned(body: &[Stmt], scope: &Env) {
    for statement in body {
        match &statement.kind {
            StmtKind::Assign { target: Expr { kind: ExprKind::Name(name), .. }, .. } => scope.forget(name),
            StmtKind::If { then_block, else_block, .. } => {
                forget_assigned(then_block, scope);
                forget_assigned(else_block.as_deref().unwrap_or_default(), scope);
            }
            StmtKind::While { body, .. } | StmtKind::For { body, .. } => forget_assigned(body, scope),
            StmtKind::Match { arms, .. } => arms.iter().for_each(|arm| forget_assigned(&arm.body, scope)),
            StmtKind::Try { body, handlers } => {
                forget_assigned(body, scope);
                handlers.iter().for_each(|handler| forget_assigned(&handler.body, scope));
            }
            _ => {}
        }
    }
}

/// Carry out to `env` what the scope of a branch still knows, at its end,
/// about the variables its condition narrowed.
fn keep_known(env: &Env, branch: &Env, narrowings: &[(String, Ty)]) {
    for (name, _) in narrowings {
        if let Some(Variable { ty, declared: Some(_), .. }) = branch.get(name) {
            env.narrow(name, ty);
        }
    }
}

fn narrow_all(scope: &Env, narrowings: &[(String, Ty)]) {
    for (name, ty) in narrowings {
        scope.narrow(name, ty.clone());
    }
}

/// Does this loop body contain a `break` that belongs to it? Breaks inside
/// an inner loop belong to that loop instead.
fn has_break(statements: &[Stmt]) -> bool {
    statements.iter().any(|statement| match &statement.kind {
        StmtKind::Break => true,
        StmtKind::If { then_block, else_block, .. } => {
            has_break(then_block) || else_block.as_deref().is_some_and(has_break)
        }
        StmtKind::Match { arms, .. } => arms.iter().any(|arm| has_break(&arm.body)),
        StmtKind::Try { body, handlers } => {
            has_break(body) || handlers.iter().any(|handler| has_break(&handler.body))
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use crate::{lexer, parser};

    fn check(source: &str, strict: bool) -> Vec<String> {
        let program = parser::parse(lexer::lex(source).expect("lexing failed")).expect("parsing failed");
        super::check(&program, strict).into_iter().map(|error| error.message).collect()
    }

    /// The messages the checker gives for a program.
    fn errors(source: &str) -> Vec<String> {
        check(source, false)
    }

    /// The single message expected from a program with one mistake.
    fn error(source: &str) -> String {
        let mut found = errors(source);
        assert_eq!(found.len(), 1, "expected exactly one error, got {found:?}");
        found.remove(0)
    }

    fn strict_error(source: &str) -> String {
        let mut found = check(source, true);
        assert_eq!(found.len(), 1, "expected exactly one error, got {found:?}");
        found.remove(0)
    }

    fn ok(source: &str) {
        assert_eq!(errors(source), Vec::<String>::new(), "\n{source}");
    }

    fn strict_ok(source: &str) {
        assert_eq!(check(source, true), Vec::<String>::new(), "\n{source}");
    }

    // ----- Written types are checked -----

    #[test]
    fn declared_types_must_match() {
        ok("let a: int = 1\nlet b: float = 1.5\nlet c: str = \"x\"\nlet d: bool = true\nlet e: list[int] = [1, 2]\nlet f: list[str] = []");
        assert_eq!(error("let x: int = \"hello\""), "`x` should be int, but this is str");
        assert_eq!(error("let xs: list[int] = [1, \"two\"]"), "`xs` should be list[int], but this item is str");
        assert_eq!(error("let xs: list[int] = [\"a\"]"), "`xs` should be list[int], but this item is str");
        assert_eq!(error("let ys = [\"a\"]\nlet xs: list[int] = ys"), "`xs` should be list[int], but this is list[str]");
        assert_eq!(
            error("let x: float = 1"),
            "`x` should be float, but this is int. Write the number with a decimal point, as in `1.0`"
        );
    }

    #[test]
    fn unknown_type_names() {
        assert_eq!(error("let x: Strng = \"a\""), "there is no type called `Strng`");
        assert_eq!(error("let x: list = []"), "`list` takes one type, as in `list[int]`");
        assert_eq!(error("let x: int[str] = 1"), "`int` does not take types in `[ ]`");
    }

    // ----- Types that are left out are worked out -----

    #[test]
    fn types_are_inferred_from_values() {
        assert_eq!(error("let n = 3\nlet s: str = n"), "`s` should be str, but this is int");
        assert_eq!(error("let xs = [1, 2]\nlet s: str = xs[0]"), "`s` should be str, but this is int");
        assert_eq!(error("let xs = [n * 2 for n in range(3)]\nlet s: str = xs"), "`s` should be str, but this is list[int]");
        assert_eq!(error("let q = 500 ms\nlet s: str = q"), "`s` should be str, but this is a quantity in ms");
        assert_eq!(error("let x = 7 / 2\nlet n: int = x"), "`n` should be int, but this is float");
        ok("let n: int = 7 // 2\nlet m: int = 7 % 2\nlet p: int = 2 ** 8\nlet f: float = 1 + 0.5");
    }

    #[test]
    fn return_types_are_inferred() {
        assert_eq!(error("function double(n: int) { return n * 2 }\nlet s: str = double(2)"), "`s` should be str, but this is int");
        assert_eq!(
            error("function find(n: int) {\n if n > 0 { return \"yes\" }\n}\nlet s: str = find(1)"),
            "`s` should be str, but this is str or none. Check it with `is not none` first"
        );
        ok("function nothing() { }\nlet x = nothing()");
        // A function used above where it is written still has its type worked out.
        assert_eq!(error("let s: str = later()\nfunction later() { return 1 }"), "`s` should be str, but this is int");
    }

    #[test]
    fn untyped_code_is_left_alone() {
        ok("function add(a, b) { return a + b }\nprint(add(1, 2), add(\"a\", \"b\"))\nlet x: int = add(1, 2)");
        ok("class Box {\n item\n}\nlet b = Box(1)\nb.item = \"now a str\"\nprint(b.item.upper())");
        ok("let x: any = 1\nprint(x.whatever)");
    }

    // ----- Names -----

    #[test]
    fn names_must_exist() {
        assert_eq!(error("print(missing)"), "`missing` is not defined");
        assert_eq!(error("x = 1"), "`x` is not declared. Create it with `let` or `var` first");
        assert_eq!(error("let x = 1\nlet x = 2"), "`x` is already declared in this block");
        assert_eq!(error("if true {\n let inner = 1\n}\nprint(inner)"), "`inner` is not defined");
        // One mistake, one message: the bad name does not set off more errors.
        assert_eq!(error("let y = missing + 1\nprint(y.upper())"), "`missing` is not defined");
    }

    #[test]
    fn let_cannot_be_reassigned() {
        ok("var x = 1\nx = 2\nx += 3");
        assert_eq!(error("let x = 1\nx = 2"), "`x` cannot be changed. Declare it with `var` to allow that");
        assert_eq!(error("function f(n: int) { n = 2 }"), "`n` cannot be changed. Declare it with `var` to allow that");
        assert_eq!(error("for i in range(3) { i = 0 }"), "`i` cannot be changed. Declare it with `var` to allow that");
        assert_eq!(error("var x = 1\nx = \"one\""), "the new value should be int, but this is str");
        assert_eq!(error("var x = 1\nx += 0.5"), "the new value should be int, but this is float");
    }

    // ----- Operators -----

    #[test]
    fn operators_need_matching_types() {
        assert_eq!(error("print(\"abc\" - 1)"), "`-` does not work between str and int");
        assert_eq!(error("print(1 + \"1\")"), "`+` does not work between int and str");
        assert_eq!(error("print(500 ms + 3 V)"), "`+` does not work between a quantity in ms and a quantity in V");
        assert_eq!(error("print(1 < \"2\")"), "`<` does not work between int and str");
        assert_eq!(error("print(-\"x\")"), "`-` does not work on str");
        assert_eq!(error("print(not 1)"), "`not` needs a bool, but this is int");
        assert_eq!(error("print(1 and true)"), "`and` needs bools, but this is int");
        assert_eq!(error("print(1 in 5)"), "`in` does not work between int and int");
        ok("print(1 + 2.5, \"a\" + \"b\", [1] + [2], 2 * 500 ms, 1 < 2 < 3, \"a\" in \"abc\", 1 in [1, 2])");
    }

    #[test]
    fn comparisons_that_can_never_be_true() {
        assert_eq!(error("print(1 == \"1\")"), "`==` between int and str is always false");
        assert_eq!(error("print([1] != \"x\")"), "`!=` between list[int] and str is always true");
        ok("print(1 == 1.0, none == none)\nlet x: int or none = 1\nprint(x == 1, x == none)");
    }

    #[test]
    fn conditions_must_be_bools() {
        assert_eq!(error("if 1 { }"), "a condition must be true or false, but this is int");
        assert_eq!(error("while \"yes\" { }"), "a condition must be true or false, but this is str");
        assert_eq!(error("let xs = [n for n in range(3) if n]"), "a condition must be true or false, but this is int");
    }

    // ----- Calls -----

    #[test]
    fn arguments_are_checked() {
        let f = "function f(a: int, b: str = \"x\") -> int { return a }\n";
        ok(&format!("{f}f(1)\nf(1, \"y\")\nf(a=1, b=\"y\")\nf(1, b=\"y\")"));
        assert_eq!(error(&format!("{f}f(\"one\")")), "`a` of `f` should be int, but this is str");
        assert_eq!(error(&format!("{f}f(1, 2)")), "`b` of `f` should be str, but this is int");
        assert_eq!(error(&format!("{f}f()")), "`f` needs a value for `a`");
        assert_eq!(error(&format!("{f}f(1, \"y\", 3)")), "`f` takes 2 argument(s), but 3 were given");
        assert_eq!(error(&format!("{f}f(1, c=3)")), "`f` has no argument named `c`");
        assert_eq!(error(&format!("{f}f(1, a=3)")), "`f` was given `a` twice");
        assert_eq!(error(&format!("{f}f(b=\"y\", 1)")), "plain arguments must come before named ones");
        assert_eq!(error("let x = 5\nx()"), "int cannot be called");
        assert_eq!(error("function f(a: int = \"no\") { }"), "`a` should be int, but this is str");
    }

    #[test]
    fn built_ins_are_checked() {
        assert_eq!(error("print(len(5))"), "`len` needs a str or a list, but this is int");
        assert_eq!(error("print(len(1, 2))"), "`len` takes 1 argument(s), but 2 were given");
        assert_eq!(error("let s: str = len(\"abc\")"), "`s` should be str, but this is int");
        assert_eq!(error("range(\"3\")"), "an argument of `range` should be int, but this is str");
        assert_eq!(error("from time import sleep\nsleep(500)"), "`sleep` needs a time with a unit, such as `500 ms`, but this is int");
        assert_eq!(error("print(\"x\".upper(1))"), "`upper` takes 0 argument(s), but 1 were given");
        assert_eq!(error("print(\"x\".nope())"), "str has no field or method `nope`");
        assert_eq!(error("let xs = [1]\nxs.append(\"two\")"), "an item added to this list should be int, but this is str");
        assert_eq!(error("import radio"), "there is no module `radio`. So far Zeno has `math`, `time` and `board`");
        assert_eq!(error("from math import nope"), "module `math` has no `nope`");
        ok("from math import sqrt\nimport math\nlet x: float = sqrt(2) + math.pi\nlet n: int = math.floor(x)\nlet parts: list[str] = \"a b\".split()");
    }

    #[test]
    fn functions_passed_to_map_filter_reduce_are_checked() {
        ok("let xs = [1, 2, 3]\nlet ys: list[str] = xs.map(str)\nlet total: int = xs.reduce(function(a: int, b: int) -> int { return a + b }, 0)\nlet evens: list[int] = xs.filter(function(n: int) -> bool { return n % 2 == 0 })");
        assert_eq!(
            error("let xs = [1, 2]\nxs.filter(function(n: int) -> int { return n })"),
            "what the function given to `filter` returns should be bool, but this is int"
        );
        assert_eq!(
            error("let xs = [1, 2]\nxs.map(function(s: str) -> str { return s })"),
            "`s` of `function` should be str, but this is int"
        );
        assert_eq!(error("[1].map(function() { return 1 })"), "`function` takes 0 argument(s), but 1 were given");
    }

    // ----- Functions -----

    #[test]
    fn returns_must_match_the_declared_type() {
        assert_eq!(error("function f() -> int { return \"x\" }"), "the returned value should be int, but this is str");
        assert_eq!(error("function f() -> int { return }"), "the returned value should be int, but this is none");
        assert_eq!(
            error("function f(n: int) -> int {\n if n > 0 { return 1 }\n}"),
            "`f` can reach its end without returning the int it promises"
        );
        ok("function f(n: int) -> int {\n if n > 0 { return 1 } else { return 2 }\n}");
        ok("function f(n: int) -> int {\n while true {\n  if n > 0 { return 1 }\n }\n}");
        ok("function f(n: int) -> int or none {\n if n > 0 { return 1 }\n}");
        ok("function f() -> int { raise ValueError(\"no\") }");
        assert_eq!(error("return 1"), "`return` outside a function");
        assert_eq!(error("break"), "`break` outside a loop");
        assert_eq!(error("while true {\n function f() { continue }\n}"), "`continue` outside a loop");
    }

    #[test]
    fn recursion_is_checked() {
        ok("function fact(n: int) -> int {\n if n <= 1 { return 1 }\n return n * fact(n - 1)\n}\nlet x: int = fact(5)");
        ok("function even(n: int) -> bool {\n if n == 0 { return true }\n return odd(n - 1)\n}\nfunction odd(n: int) -> bool {\n if n == 0 { return false }\n return even(n - 1)\n}");
        assert_eq!(error("function fact(n: int) -> int {\n return fact(\"x\")\n}"), "`n` of `fact` should be int, but this is str");
    }

    #[test]
    fn a_function_nobody_calls_is_still_checked() {
        assert_eq!(error("function unused() { print(missing) }"), "`missing` is not defined");
    }

    // ----- The bits of whole numbers -----

    #[test]
    fn bit_operators_need_whole_numbers() {
        ok("let a: int = 6 & 3\nlet b: int = 1 << 4 | 1\nlet c: int = ~a ^ b >> 1\nvar d = 0\nd |= 4\nd <<= 1");
        assert_eq!(
            error("print(true & false)"),
            "`&` works on the bits of whole numbers, and these are bool and bool. For two bools Zeno spells this `and`"
        );
        assert_eq!(
            error("print(true | false)"),
            "`|` works on the bits of whole numbers, and these are bool and bool. For two bools Zeno spells this `or`"
        );
        assert_eq!(error("print(1.5 << 2)"), "`<<` works on the bits of whole numbers, and these are float and int");
        assert_eq!(error("print(~1.5)"), "`~` works on the bits of a whole number, and this is float");
        assert_eq!(error("var x = 1.0\nx &= 3"), "`&` works on the bits of whole numbers, and these are float and int");
    }

    // ----- Sized whole numbers -----

    #[test]
    fn sized_ints_are_types() {
        strict_ok(include_str!("../examples/packet.zeno"));
        strict_ok(
            "let a: u8 = 200\nlet b: u8 = u8(100)\nlet c: i8 = i8.wrap(a)\nlet d: u16 = u16(a + b)\n\
             let e: i32 = -2147483648\nlet f: u32 = 4294967295\nlet g: i16 = -32768\n\
             let added: int = a + b\nlet bits: int = a << 8 | b\nlet same: bool = a == b or a < 3 or a in [1, 2]\n\
             let wide: int = a\nlet text: str = f\"{a:02x}\"\nlet biggest: int = max(a, b)\nlet size: int = abs(c)\n\
             function double(n: int) -> int { return n * 2 }\nlet twice: int = double(a)\n\
             class Reg {\n    value: u8\n    mask: u8 = 0xFF\n}\nlet r = Reg(value=1)\nr.value = u8(r.value | 4)\nr.mask = 15\n\
             let bytes: list[u8] = [1, 2, 255]\nbytes.append(a)\nbytes.append(9)\nbytes[0] = 7\nlet first: u8 = bytes[0]\n\
             let total: int = sum(bytes)\nvar maybe: u8 or none = none\nmaybe = 5\nmaybe = a\n\
             for i in range(a) { print(bytes[b]) }",
        );
        // A class of the program's own may take the name.
        ok("class u8 {\n    n: int\n}\nlet x: u8 = u8(n=300)");
    }

    #[test]
    fn a_written_number_must_fit_its_size() {
        assert_eq!(error("let a: u8 = 256"), "256 does not fit in a u8 (0 to 255)");
        assert_eq!(error("let a: u8 = -1"), "-1 does not fit in a u8 (0 to 255)");
        assert_eq!(error("let a: i8 = -129"), "-129 does not fit in an i8 (-128 to 127)");
        assert_eq!(error("let a: u16 or none = 65536"), "65536 does not fit in a u16 (0 to 65535)");
        assert_eq!(error("let a: list[u8] = [1, 2, 300]"), "300 does not fit in a u8 (0 to 255)");
        assert_eq!(error("var a: u8 = 0\na = 999"), "999 does not fit in a u8 (0 to 255)");
        assert_eq!(error("function f(n: u8) { print(n) }\nf(256)"), "256 does not fit in a u8 (0 to 255)");
        assert_eq!(error("function f(n: i8 = 128) { print(n) }"), "128 does not fit in an i8 (-128 to 127)");
        assert_eq!(error("function f() -> i8 { return 128 }"), "128 does not fit in an i8 (-128 to 127)");
        assert_eq!(error("class P {\n    v: u8 = 256\n}"), "256 does not fit in a u8 (0 to 255)");
        assert_eq!(error("class P {\n    v: u8\n}\nlet p = P(v=-1)"), "-1 does not fit in a u8 (0 to 255)");
        assert_eq!(error("let xs: list[u8] = [1]\nxs.append(256)"), "256 does not fit in a u8 (0 to 255)");
        assert_eq!(error("let xs: list[u8] = [1]\nxs[0] = 256"), "256 does not fit in a u8 (0 to 255)");
        // `u8( )` of a written number is checked here too. `wrap` is not:
        // it is asked for the low bits.
        assert_eq!(error("let a = u8(1000)"), "1000 does not fit in a u8 (0 to 255)");
        ok("let a: u8 = u8.wrap(1000)\nlet b: i32 = i32.wrap(-99999999999)");
    }

    #[test]
    fn an_int_does_not_become_sized_by_itself() {
        let hint = "Convert it with `u8(...)`, which checks that it fits";
        assert_eq!(error("let n = 5\nlet a: u8 = n"), format!("`a` should be u8, but this is int. {hint}"));
        // The answer of an operator is an int, whatever went in.
        assert_eq!(error("var a: u8 = 1\na += 1"), format!("the new value should be u8, but this is int. {hint}"));
        assert_eq!(error("var a: u8 = 1\na = a + 1"), format!("the new value should be u8, but this is int. {hint}"));
        assert_eq!(error("let a: u8 = 1\nlet b: u8 = ~a"), format!("`b` should be u8, but this is int. {hint}"));
        assert_eq!(
            error("let a: u8 = 1\nlet b: u16 = a"),
            "`b` should be u16, but this is u8. Convert it with `u16(...)`, which checks that it fits"
        );
        assert_eq!(
            error("function f(n: u8) { print(n) }\nlet n = 5\nf(n)"),
            format!("`n` of `f` should be u8, but this is int. {hint}")
        );
        assert_eq!(
            error("let xs: list[u8] = [1]\nlet n = 5\nxs.append(n)"),
            format!("an item added to this list should be u8, but this is int. {hint}")
        );
        assert_eq!(
            error("let xs: list[u8] = [1]\nlet ys: list[int] = xs"),
            "`ys` should be list[int], but this is list[u8]. The list could then be given a number too big for it. \
             Make a list of ints from it with `[int(n) for n in ...]`"
        );
        assert_eq!(error("let a = u8(2.5)"), "`u8` needs a whole number, but this is float. Make it one with `int( )` first");
        assert_eq!(error("let a = u8.wrap(\"x\")"), "`u8.wrap` needs a whole number, but this is str");
        assert_eq!(error("let a = u8(1, 2)"), "`u8` takes 1 argument(s), but 2 were given");
        assert_eq!(
            error("let a: u64 = 1"),
            "there is no type called `u64`. An `int` is 64 bits with a sign, \
             and the sized whole numbers are u8, u16, u32, i8, i16 and i32"
        );
    }

    #[test]
    fn a_size_left_out_is_worked_out() {
        // From the value, and for a parameter from the calls.
        strict_ok("let a = u8(5)\nlet b: u8 = a\nfunction same(n) { return n }\nlet c: u8 = same(a)");
        // A u8 from one call and an int from another: the parameter is an int.
        assert_eq!(
            strict_error("function same(n) { return n }\nlet a = same(u8(5))\nlet b: u8 = same(7)"),
            "`b` should be u8, but this is int. Convert it with `u8(...)`, which checks that it fits"
        );
        // A list of a u8 and an int is a list of ints.
        strict_ok("let a: u8 = 1\nlet xs: list[int] = [a, 300]\nlet ys = [a, u16(2)]\nlet zs: list[int] = ys");
    }

    // ----- Generics -----

    const FIRST: &str = "function first[T](items: list[T]) -> T {\n    return items[0]\n}\n";
    const STACK: &str = "class Stack[T] {\n    items: list[T]\n    function push(self, item: T) {\n        self.items.append(item)\n    }\n}\n";
    const SAME: &str = "function same[T](a: T, b: T) -> T {\n    return a\n}\n";

    #[test]
    fn generics_are_checked() {
        strict_ok(include_str!("../examples/generics.zeno"));
        // What comes back is the type that went in.
        strict_ok(&format!("{FIRST}let a: int = first([1, 2])\nlet b: str = first([\"a\"])\nlet c: list[float] = first([[1.5]])"));
        strict_ok(&format!(
            "{STACK}let a = Stack([1])\na.push(2)\nlet b: Stack[str] = Stack(items=[])\nb.push(\"x\")\n\
             let both: list[Stack[int]] = [a, Stack([3])]\nlet maybe: Stack[str] or none = b\n\
             function total(s: Stack[int]) -> int {{\n    return sum(s.items)\n}}\nlet n: int = total(a)"
        ));
        // Inside a generic the type parameter is a type like any other,
        // and it wins over a unit of the same name: `A` is not amperes.
        strict_ok(
            "class Pair[A, B] {\n    left: A\n    right: B\n}\n\
             function swap[A, B](p: Pair[A, B]) -> Pair[B, A] {\n    let left: A = p.left\n    return Pair(p.right, left)\n}\n\
             let q: Pair[str, int] = swap(Pair(1, \"x\"))\nlet amps: A = 2 A",
        );
        // A function argument is called with what the others gave.
        strict_ok(
            "function apply[T, U](value: T, f: function(T) -> U) -> U {\n    return f(value)\n}\n\
             let a: int = apply(5, function(n) { return n * 2 })\nlet b: str = apply(\"hi\", function(s) { return s.upper() })\n\
             let c: int = apply(\"abc\", len)",
        );
        // An owned class can be generic, and its copies are owned.
        strict_ok(
            "owned class Box[T] {\n    value: T\n}\nfunction look[T](box: borrow Box[T]) -> T {\n    return box.value\n}\n\
             function open[T](box: Box[T]) -> T {\n    return box.value\n}\nlet b = Box(5)\nlet n: int = look(b)\nlet m: int = open(b)",
        );
    }

    #[test]
    fn the_types_of_a_generic_come_from_the_call() {
        assert_eq!(error(&format!("{FIRST}let x: str = first([1, 2])")), "`items` of `first[str]` should be list[str], but this is list[int]");
        assert_eq!(error(&format!("{FIRST}let n = 5\nlet x: str = first([n])")), "`x` should be str, but this is int");
        assert_eq!(error(&format!("{FIRST}print(first(5))")), "`items` of `first` should be list[T], but this is int");
        assert_eq!(error(&format!("{STACK}let a: Stack[int] = Stack([1])\nlet b: Stack[str] = a")), "`b` should be Stack[str], but this is Stack[int]");
        assert_eq!(error(&format!("{STACK}let a = Stack([1])\na.push(\"x\")")), "`item` of `push` should be int, but this is str");
        assert_eq!(error(&format!("{SAME}let n = 1\nprint(same(n, \"x\"))")), "`b` of `same[int]` should be int, but this is str");
        // Nothing says what `T` is, until the place the result goes does.
        let make = "function make[T]() -> list[T] {\n    return []\n}\n";
        assert_eq!(
            error(&format!("{make}let a = make()")),
            "`make` cannot tell what `T` is from this call: no argument says. \
             Put the result where its type is written, as in `let x: list[int] = make(...)`"
        );
        assert_eq!(
            strict_error(&format!("{STACK}let s = Stack(items=[])")),
            "`Stack` cannot tell what `T` is from this call: no argument says. \
             Put the result where its type is written, as in `let x: Stack[int] = Stack(...)`"
        );
        strict_ok(&format!("{make}let a: list[int] = make()\nfunction f() -> list[str] {{\n    return make()\n}}"));
        // `none` and a value make an `or none`. A wider number wins.
        strict_ok(&format!("{SAME}let n = 3\nlet a: int or none = same(none, n)\nlet small: u8 = 4\nlet wide: int = same(small, n)"));
    }

    #[test]
    fn a_written_number_in_a_generic_call_takes_the_size_that_is_wanted() {
        let ring = "class Ring[T] {\n    slots: list[T]\n}\nfunction ring[T](size: int, empty: T) -> Ring[T] {\n    return Ring([empty])\n}\n";
        strict_ok(&format!(
            "{ring}{SAME}{FIRST}let bytes: Ring[u8] = ring(4, 0)\nlet ints: Ring[int] = ring(4, 0)\nlet small: u8 = 4\n\
             let x: u8 = same(small, 9)\nlet y: u8 = first([1, 2, 3])\nlet z: int = same(1, 2)"
        ));
        assert_eq!(error(&format!("{SAME}let small: u8 = 4\nprint(same(small, 300))")), "300 does not fit in a u8 (0 to 255)");
        assert_eq!(error(&format!("{ring}let bytes: Ring[u8] = ring(4, 256)")), "256 does not fit in a u8 (0 to 255)");
    }

    #[test]
    fn a_generic_is_checked_once_for_each_set_of_types() {
        // The body is checked with `T` as each type it is used with, so
        // a `*` that a str does not have is found, and the error says
        // which copy it is in.
        let source = "class Box[T] {\n    value: T\n    function double(self) -> T {\n        return self.value * 2\n    }\n}\n\
                      print(Box(2).double())\nprint(Box(2.5).double())\nprint(Box(\"ab\").double())";
        let program = parser::parse(lexer::lex(source).unwrap()).unwrap();
        let found = super::check(&program, false);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].message, "`*` does not work between str and int");
        assert_eq!((found[0].span.line, found[0].span.col), (4, 16));
        assert_eq!(found[0].note.as_deref(), Some("this is inside `Box[str]`: `Box` with T = str, which line 9 asks for"));

        // A generic nothing uses is still checked, with `T` as anything.
        assert_eq!(error("function unused[T](x: T) -> T {\n    return y\n}"), "`y` is not defined");
        ok("function unused[T](x: T) -> T {\n    return x * 2 + len(x)\n}");
        // A class that needs a bigger copy of itself each time never ends.
        assert_eq!(
            error("class Node[T] {\n    value: T\n    next: Node[list[T]] or none\n}\nlet n = Node(1, none)"),
            "`Node` needs a copy of itself for a bigger type every time, and that never ends. \
             It got as far as `Node[list[list[list[list[list[list[list[list[list[list[list[...`"
        );
    }

    #[test]
    fn what_a_generic_cannot_do_yet() {
        assert_eq!(
            error("function first[t](items: list[t]) -> t {\n    return items[0]\n}"),
            "a type parameter starts with a capital letter, as in `T` or `Item`, and this is `t`"
        );
        assert_eq!(error("function two[T, T](a: T) -> T {\n    return a\n}"), "the type parameter `T` is written twice");
        assert_eq!(
            error(&format!("{STACK}let s: Stack[int, str] = Stack(items=[])")),
            "`Stack` takes 1 type(s) in `[ ]`, as in `Stack[int]`, but 2 were written"
        );
        assert_eq!(
            error(&format!("{STACK}function f(s: Stack) {{ }}")),
            "`Stack` takes 1 type(s) in `[ ]`, as in `Stack[int]`, but 0 were written"
        );
        assert_eq!(error(&format!("{FIRST}let n: first = 4")), "`first` is a function, and this place needs a type");
        assert_eq!(
            error(&format!("{FIRST}let f = first")),
            "`first` is generic. It can be called, as in `first(...)`, but it cannot be kept as a value or passed along yet"
        );
        assert_eq!(
            error("class Box {\n    n: int\n    function get[T](self, x: T) -> T {\n        return x\n    }\n}"),
            "the method `get` cannot have type parameters of its own yet. The class can have them: `class Box[T]`"
        );
        assert_eq!(
            error("class Problem[T] {\n    what: T\n}\nfunction f() {\n    raise Problem(4)\n}"),
            "an object of a generic class cannot be raised yet, and this is a Problem[int]"
        );
        assert_eq!(
            error("function keep[T](x: T) -> T {\n    return x\n}\nlet xs: owned list[int] = [1]\nlet ys = keep(xs)"),
            "`T` would be a borrowed list[int] here. An owned or borrowed value cannot fill a type parameter yet"
        );
    }

    // ----- The board and the clock -----

    #[test]
    fn pins_are_checked() {
        ok("from board import led, pin\nled.on()\nlet relay: Pin = pin(7)\nlet lit: bool = relay.is_on()\nlet n: int = led.number");
        ok("import board\nfunction flash(light: Pin) { light.toggle() }\nflash(board.led)\nlet pins: list[Pin] = [board.pin(1), board.led]");
        strict_ok(include_str!("../examples/blink.zeno"));
        strict_ok(include_str!("../examples/signal.zeno"));
        assert_eq!(error("from board import pin\npin(\"a\")"), "the number of a pin should be int, but this is str");
        assert_eq!(error("from board import led\nled.blink()"), "Pin has no field or method `blink`");
        assert_eq!(error("from board import led\nled.on(1)"), "`on` takes 0 argument(s), but 1 were given");
        assert_eq!(error("from board import led\nlet n: int = led"), "`n` should be int, but this is Pin");
        assert_eq!(error("from board import led\nprint(led == 13)"), "`==` between Pin and int is always false");
        assert_eq!(error("from board import button"), "module `board` has no `button`");
        // A class of the program's own called `Pin` comes first.
        ok("class Pin {\n n: int\n}\nlet p: Pin = Pin(1)\nprint(p.n)");
    }

    #[test]
    fn the_clock_is_checked() {
        ok("from time import sleep, now\nlet start: ms = now()\nsleep(1 s)\nsleep(2 min)\nlet took: ms = now() - start");
        assert_eq!(error("from time import sleep\nsleep(3 V)"), "`sleep` needs a time with a unit, such as `500 ms`, but this is a quantity in V");
        assert_eq!(error("from time import now\nnow(1)"), "`now` takes 0 argument(s), but 1 were given");
        assert_eq!(error("from time import now\nlet t: s = now()"), "`t` should be a quantity in s, but this is a quantity in ms. Convert it with `.s`");
    }

    // ----- Classes -----

    #[test]
    fn classes_are_checked() {
        let point = "class Point {\n x: float\n y: float = 0.0\n function scaled(self, by: float) -> Point {\n  return Point(self.x * by, self.y * by)\n }\n}\n";
        ok(&format!("{point}let p = Point(1.0)\nlet q: Point = p.scaled(2.0)\np.x = 3.0\nlet x: float = q.x"));
        assert_eq!(error(&format!("{point}Point(\"one\")")), "`x` of `Point` should be float, but this is str");
        assert_eq!(error(&format!("{point}Point()")), "`Point` needs a value for `x`");
        assert_eq!(error(&format!("{point}print(Point(1.0).z)")), "`Point` has no field or method `z`");
        assert_eq!(error(&format!("{point}let p = Point(1.0)\np.z = 2.0")), "`Point` has no field `z`");
        assert_eq!(error(&format!("{point}let p = Point(1.0)\np.x = \"far\"")), "the new value should be float, but this is str");
        assert_eq!(error(&format!("{point}Point(1.0).scaled(\"big\")")), "`by` of `scaled` should be float, but this is str");
        assert_eq!(error(&format!("{point}Point(1.0).scaled()")), "`scaled` needs a value for `by`");
        assert_eq!(error(&format!("{point}let n: int = Point(1.0)")), "`n` should be int, but this is Point");
    }

    #[test]
    fn mistakes_inside_a_class() {
        assert_eq!(error("class A {\n n: int = \"no\"\n}"), "`n` should be int, but this is str");
        assert_eq!(error("class A {\n n: int\n function get(self) -> str { return self.n }\n}"), "the returned value should be str, but this is int");
        assert_eq!(error("class A {\n function lost() { }\n}"), "the method `lost` needs `self` as its first parameter");
        assert_eq!(error("class A {\n other: Missing\n}"), "there is no type called `Missing`");
        // Two classes can mention each other, whichever comes first.
        ok("class A {\n b: B or none\n}\nclass B {\n a: A or none\n}");
    }

    // ----- Enums and match -----

    const SHAPE: &str = "enum Shape {\n Dot\n Circle(radius: float)\n Rect(w: float, h: float)\n}\n";

    #[test]
    fn enums_are_checked() {
        ok(&format!("{SHAPE}let a: Shape = Circle(1.0)\nlet b: Shape = Dot\nlet c: Shape = Shape.Rect(1.0, h=2.0)\nlet d: list[Shape] = [Dot, Circle(2.0)]"));
        assert_eq!(error(&format!("{SHAPE}Circle(\"big\")")), "`radius` of `Circle` should be float, but this is str");
        assert_eq!(error(&format!("{SHAPE}Circle()")), "`Circle` needs a value for `radius`");
        assert_eq!(error(&format!("{SHAPE}print(Shape.Square)")), "enum `Shape` has no case `Square`");
        assert_eq!(error(&format!("{SHAPE}let n: int = Dot")), "`n` should be int, but this is Shape");
    }

    #[test]
    fn match_must_cover_every_case() {
        let area = "function area(s: Shape) -> float {\n match s {\n  case Circle(r) { return r }\n  case Rect(w, h) { return w * h }\n";
        ok(&format!("{SHAPE}{area}  case Dot {{ return 0.0 }}\n }}\n}}"));
        ok(&format!("{SHAPE}{area}  case _ {{ return 0.0 }}\n }}\n}}"));
        ok(&format!("{SHAPE}{area}  case other {{ return 0.0 }}\n }}\n}}"));
        assert_eq!(
            errors(&format!("{SHAPE}{area} }}\n}}")),
            vec!["this `match` does not handle Dot. Add a `case` for it, or a final `case _`"]
        );
        assert_eq!(
            error(&format!("{SHAPE}match Circle(1.0) {{\n case Dot {{ }}\n}}")),
            "this `match` does not handle Circle, Rect. Add a `case` for it, or a final `case _`"
        );
        // A case that only matches some values does not cover the whole case.
        assert_eq!(
            error(&format!("{SHAPE}match Dot {{\n case Dot {{ }}\n case Rect(w, h) {{ }}\n case Circle(1.0) {{ }}\n}}")),
            "this `match` does not handle Circle. Add a `case` for it, or a final `case _`"
        );
    }

    #[test]
    fn patterns_are_checked() {
        assert_eq!(error(&format!("{SHAPE}match Dot {{\n case Circle(a, b) {{ }}\n case _ {{ }}\n}}")), "`Circle` carries 1 value(s), but this pattern has 2");
        assert_eq!(error(&format!("{SHAPE}match Dot {{\n case Nope(x) {{ }}\n case _ {{ }}\n}}")), "`Nope` is not an enum case that carries values");
        assert_eq!(error(&format!("{SHAPE}match 5 {{\n case Dot {{ }}\n case _ {{ }}\n}}")), "`Dot` is a case of Shape, which can never match int");
        assert_eq!(error("match 5 {\n case \"five\" { }\n case _ { }\n}"), "this case is str, which can never match int");
        assert_eq!(error("match 5 {\n case 1 { }\n}"), "this `match` needs a final `case _` to handle every other value");
        // The values a case carries have the types the enum gave them.
        assert_eq!(
            error(&format!("{SHAPE}match Dot {{\n case Circle(r) {{ let s: str = r }}\n case _ {{ }}\n}}")),
            "`s` should be str, but this is float"
        );
    }

    // ----- none -----

    #[test]
    fn none_must_be_checked_before_use() {
        let find = "function find(id: int) -> str or none {\n if id == 1 { return \"partha\" }\n return none\n}\nlet user = find(1)\n";
        assert_eq!(
            error(&format!("{find}print(user.upper())")),
            "this is str or none. Check it with `is not none` before using `.upper`"
        );
        assert_eq!(
            error(&format!("{find}let name: str = user")),
            "`name` should be str, but this is str or none. Check it with `is not none` first"
        );
        assert_eq!(error(&format!("{find}print(user + \"!\")")), "`+` does not work between str or none and str. Check for `none` first");
        ok(&format!("{find}if user is not none {{\n print(user.upper())\n}}"));
        ok(&format!("{find}if user is none {{\n print(\"nobody\")\n}} else {{\n print(user.upper())\n}}"));
        ok(&format!("{find}print(user is not none and user.upper() == \"PARTHA\")"));
        ok(&format!("{find}print(user is none or user.upper() == \"PARTHA\")"));
        ok(&format!("{find}if not user is none {{\n print(user.upper())\n}}"));
    }

    #[test]
    fn an_early_exit_settles_the_rest_of_the_block() {
        let source = "function shout(text: str or none) -> str {\n if text is none {\n  return \"\"\n }\n return text.upper()\n}";
        ok(source);
        ok("function first(xs: list[str or none]) -> list[str] {\n return [x for x in xs if x is not none]\n}");
        assert_eq!(
            error("function shout(text: str or none) -> str {\n if text is none {\n  print(\"nothing\")\n }\n return text.upper()\n}"),
            "this is str or none. Check it with `is not none` before using `.upper`"
        );
    }

    #[test]
    fn a_new_value_changes_what_is_known() {
        let node = "class Node {\n value: int\n next: Node or none = none\n}\n";
        // Walking a linked list: `node` is checked by the loop, then moved on.
        ok(&format!(
            "{node}function total(head: Node or none) -> int {{\n var node = head\n var sum = 0\n while node is not none {{\n  sum += node.value\n  node = node.next\n }}\n return sum\n}}"
        ));
        // Filling in a missing value: both ways out of the `if` leave a str.
        ok("function f(given: str or none) -> str {\n var name = given\n if name is none { name = \"anon\" }\n return name\n}");
        ok("function f() -> int {\n var x: int or none = none\n x = 5\n return x + 1\n}");

        let gone = "`+` does not work between int or none and int. Check for `none` first";
        // Set back to none in a block inside the check.
        assert_eq!(error("var x: int or none = 5\nif x is not none {\n if x > 3 { x = none }\n print(x + 1)\n}"), gone);
        // Set back to none by an earlier trip round a loop.
        assert_eq!(error("var x: int or none = 5\nif x is not none {\n while true {\n  print(x + 1)\n  x = none\n }\n}"), gone);
        assert_eq!(
            error("function f(xs: list[int]) {\n var x: int or none = 1\n x = 2\n for y in xs {\n  print(x + y)\n  x = none\n }\n}"),
            gone
        );
        // Set back to none in a `try`, which may stop part way.
        assert_eq!(error("function f() -> int {\n var x: int or none = 1\n x = 2\n try { x = none } except { }\n return x + 1\n}"), gone);
        // The branch that carries on undid the check.
        assert_eq!(
            error("function f(x: int or none) -> int {\n var y = x\n if y is none { return 0 } else { y = none }\n return y + 1\n}"),
            gone
        );
    }

    #[test]
    fn none_only_goes_where_it_is_allowed() {
        ok("let a: int or none = none\nlet b: int or none = 5\nvar c: int or none = 1\nc = none");
        assert_eq!(error("let a: int = none"), "`a` should be int, but this is none");
        assert_eq!(error("function f(n: int) { }\nf(none)"), "`n` of `f` should be int, but this is none");
    }

    // ----- Errors and raises -----

    #[test]
    fn declared_raises_must_be_complete() {
        ok("function f(text: str) -> int raises ValueError { return int(text) }");
        ok("function f() raises ValueError { raise ValueError(\"no\") }");
        ok("function f() raises Error { raise IndexError(\"no\") }");
        assert_eq!(
            error("function f(n: int) raises ValueError {\n if n > 0 { raise ValueError(\"a\") }\n raise IndexError(\"b\")\n}"),
            "`f` can raise IndexError here but does not say so. Add `raises IndexError` to it, or catch the error"
        );
        let parse = "function parse(text: str) -> int raises ValueError { return int(text) }\n";
        assert_eq!(
            error(&format!("{parse}function load() -> int raises IndexError {{ return parse(\"1\") }}")),
            "`load` can raise ValueError here but does not say so. Add `raises ValueError` to it, or catch the error"
        );
        ok(&format!("{parse}function load() -> int raises IndexError {{\n try {{\n  return parse(\"1\")\n }} except ValueError {{\n  return 0\n }}\n}}"));
        assert_eq!(error("function f() raises Oops { }"), "`Oops` is not a kind of error");
        assert_eq!(error("try { } except Oops { }"), "`Oops` is not a kind of error");
        assert_eq!(error("raise 5"), "only an error can be raised, and this is int");
    }

    #[test]
    fn raises_left_out_is_worked_out() {
        // `parse` does not say what it raises, so that is inferred, and
        // `load`, which does say, must account for it.
        let source = "function parse(text: str) -> int { return int(text) }\nfunction load() -> int raises IndexError { return parse(\"1\") }";
        assert_eq!(
            error(source),
            "`load` can raise ValueError here but does not say so. Add `raises ValueError` to it, or catch the error"
        );
        ok("function parse(text: str) -> int { return int(text) }\nprint(parse(\"1\"))");
    }

    #[test]
    fn caught_errors_have_fields() {
        ok("try {\n raise ValueError(\"bad\")\n} except ValueError as e {\n let m: str = e.message\n} except Error as e {\n print(e.kind)\n}");
        ok("class Timeout {\n after: int\n}\ntry {\n raise Timeout(3)\n} except Timeout as t {\n let n: int = t.after\n}");
    }

    // ----- Strict mode -----

    #[test]
    fn strict_mode_needs_every_type_to_be_known() {
        assert_eq!(strict_error("function add(a, b: int) -> int { return b }"), "strict mode: the parameter `a` of `add` needs a type. It could not be worked out from how it is used"
        );
        assert_eq!(
            strict_error("class Box {\n item\n}"),
            "strict mode: `item` in `Box` needs a type. It could not be worked out from how it is used"
        );
        assert_eq!(
            strict_error("let xs = []"),
            "strict mode: the type of `xs` cannot be worked out. Give it one, as in `xs: list[int]`"
        );
        assert_eq!(strict_error("let x: any = 1"), "`any` is not allowed in strict mode");
        assert_eq!(
            strict_error("let mixed = [1, \"two\"]"),
            "strict mode: the type of `mixed` cannot be worked out. Give it one, as in `mixed: list[int]`"
        );
        strict_ok("let xs: list[int] = []\nlet n = 3\nlet names = [\"a\", \"b\"]\nfunction double(n: int) { return n * 2 }\nlet d: int = double(n)");
    }

    #[test]
    fn strict_mode_needs_raises_to_be_written() {
        assert_eq!(
            strict_error("function parse(text: str) -> int { return int(text) }"),
            "`parse` can raise ValueError here but does not say so. Add `raises ValueError` to it, or catch the error"
        );
        strict_ok("function parse(text: str) -> int raises ValueError { return int(text) }");
    }

    // ----- Function types -----

    #[test]
    fn function_types_can_be_written() {
        let twice = "function twice(f: function(int) -> int, x: int) -> int { return f(f(x)) }\n";
        ok(&format!("{twice}function inc(n: int) -> int {{ return n + 1 }}\nlet r: int = twice(inc, 1)"));
        ok(&format!("{twice}let r: int = twice(function(k) {{ return k * 2 }}, 5)"));
        assert_eq!(
            error(&format!("{twice}function shout(s: str) -> str {{ return s.upper() }}\ntwice(shout, 1)")),
            "`f` of `twice` should be function(int) -> int, but the parameter `s` of this function is str"
        );
        assert_eq!(
            error("let g: function(int) -> str = function(k: int) -> int { return k }"),
            "`g` should be function(int) -> str, but this function returns int"
        );
        assert_eq!(
            error(&format!("{twice}twice(5, 1)")),
            "`f` of `twice` should be function(int) -> int, but this is int"
        );
        // Calling through a function type is checked like any other call.
        assert_eq!(
            error("function apply(f: function(int) -> int) -> int { return f(\"x\") }"),
            "an argument should be int, but this is str"
        );
    }

    // ----- Units -----

    #[test]
    fn units_are_types() {
        ok("function wait(d: ms) { }\nwait(500 ms)\nlet v: mV = 20 mV");
        assert_eq!(
            error("function wait(d: ms) { }\nwait(3 s)"),
            "`d` of `wait` should be a quantity in ms, but this is a quantity in s. Convert it with `.ms`"
        );
        ok("function wait(d: ms) { }\nlet t = 3 s\nwait(t.ms)\nwait((2 min).ms)");
        // Units of one kind work together, and the finer one is the answer.
        ok("let total: ms = 1 s + 500 ms\nlet late: bool = 1500 ms > 1 s\nlet same: bool = 1 s == 1000 ms\nlet ratio: float = 1 h / 1 min");
        assert_eq!(error("let x = 1 s + 1 V"), "`+` does not work between a quantity in s and a quantity in V");
        assert_eq!(
            error("let v = (1 s).V"),
            "a quantity in s cannot be turned into V. They measure different things"
        );
        assert_eq!(
            error("function wait(d: ms) { }\nwait(3)"),
            "`d` of `wait` should be a quantity in ms, but this is int"
        );
        assert_eq!(
            error("let t = 5 sec"),
            "`sec` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC"
        );
    }

    // ----- Working out parameters and fields -----

    #[test]
    fn untyped_parameters_are_worked_out_from_the_calls() {
        assert_eq!(
            error("function add(a, b) { return a + b }\nlet s: str = add(1, 2)"),
            "`s` should be str, but this is int"
        );
        // What one function learns is passed on to the next.
        assert_eq!(
            error("function a(x) { return b(x) }\nfunction b(y) { return y + 1 }\nlet s: str = a(1)"),
            "`s` should be str, but this is int"
        );
        // A mistake shows up inside the function, where the value is used.
        assert_eq!(error("function f(a) { return a.upper() }\nf(1)"), "int has no field or method `upper`");
        // `none` at one call and an int at another makes `int or none`.
        strict_ok("function h(a) {\n if a is none { return 0 }\n return a + 1\n}\nprint(h(1), h(none))");
        // A default value counts as a use.
        assert_eq!(error("function f(a = 3) { return a }\nlet s: str = f()"), "`s` should be str, but this is int");
    }

    #[test]
    fn an_error_on_a_worked_out_parameter_names_the_call() {
        let program = |source: &str| {
            let tokens = crate::lexer::lex(source).expect("lexes");
            crate::parser::parse(tokens).expect("parses")
        };
        let found = super::check(&program("function f(a) {\n let n: str = 1\n return a.upper()\n}\n\nf(1)"), false);
        assert_eq!(found.len(), 2);
        // The mistake that has nothing to do with `a` says nothing about it.
        assert_eq!(found[0].note, None);
        assert_eq!(
            found[1].note.as_deref(),
            Some("`a` was worked out to be int from line 6. Give `a` a type to have each call checked against it instead")
        );
        // With the type written, the call is where the error is.
        let found = super::check(&program("function f(a: str) {\n return a.upper()\n}\n\nf(1)"), false);
        assert_eq!((found[0].span.line, &found[0].note), (5, &None));
    }

    #[test]
    fn calls_that_disagree_leave_a_parameter_dynamic() {
        ok("function id(x) { return x }\nlet a: int = id(1)\nlet b: str = id(\"s\")");
        assert_eq!(
            strict_error("function id(x) { return x }\nprint(id(1), id(\"s\"))"),
            "strict mode: the parameter `x` of `id` needs a type. It could not be worked out from how it is used"
        );
        // Never called, so there is nothing to go on.
        ok("function f(x) { return x.anything }");
        // Stored in a variable of unknown type, it may be called with anything.
        ok("function f(x) { return x.upper() }\nlet fs: list[any] = [f]\nprint(f(1))");
    }

    #[test]
    fn untyped_fields_are_worked_out_from_what_is_stored() {
        assert_eq!(
            error("class P {\n x\n}\nlet p = P(1)\np.x = 2\nlet s: str = p.x"),
            "`s` should be str, but this is int"
        );
        ok("class P {\n x\n}\nlet p = P(1)\np.x = \"s\"\nlet n: int = p.x");
        strict_ok("class C {\n total\n function add(self, n) { self.total = self.total + n }\n}\nlet c = C(0)\nc.add(5)\nlet t: int = c.total");
    }

    #[test]
    fn recursive_functions_have_their_return_type_worked_out() {
        let fact = "function fact(n) {\n if n <= 1 { return 1 }\n return n * fact(n - 1)\n}\n";
        strict_ok(&format!("{fact}let x: int = fact(5)"));
        assert_eq!(error(&format!("{fact}let s: str = fact(5)")), "`s` should be str, but this is int");
        let even = "function ev(n) {\n if n == 0 { return true }\n return od(n - 1)\n}\nfunction od(n) {\n if n == 0 { return false }\n return ev(n - 1)\n}\n";
        assert_eq!(error(&format!("{even}let s: str = ev(4)")), "`s` should be str, but this is bool");
    }

    #[test]
    fn an_int_is_not_quietly_made_a_float() {
        // Arithmetic mixes the two, but a function that returns an int on
        // one path and a float on another does not have one return type.
        strict_ok("function half(n: int) { return n / 2 }\nlet h: float = half(3) + 1");
        assert_eq!(
            strict_error("function pick(n: int) {\n if n > 0 { return 1 }\n return 2.5\n}\nlet p = pick(1)"),
            "strict mode: `pick` returns different types (int, float). Make them one type, as in `float(n)` or `1.0`"
        );
        strict_ok("function pick(n: int) {\n if n > 0 { return 1.0 }\n return 2.5\n}\nlet p: float = pick(1)");
    }

    // ----- Strict in the file -----

    #[test]
    fn a_file_can_ask_for_strict_mode() {
        ok("let xs = []");
        assert_eq!(
            error("strict\nlet xs = []"),
            "strict mode: the type of `xs` cannot be worked out. Give it one, as in `xs: list[int]`"
        );
    }

    // ----- Owned values -----

    /// A few declarations the tests below all use.
    const OWNED: &str = "owned class Packet {
    id: int
    data: owned list[int]
    function size(self) -> int { return len(self.data) }
}
function make() -> Packet { return Packet(1, [1, 2]) }
function send(p: Packet) -> int { return p.id }
function look(p: borrow Packet) -> int { return p.id }
function total(xs: borrow list[int]) -> int { return len(xs) }
";

    fn owned_ok(source: &str) {
        strict_ok(&format!("{OWNED}{source}"));
    }

    fn owned_error(source: &str) -> String {
        strict_error(&format!("{OWNED}{source}"))
    }

    const GONE: &str = "holds nothing now. Lend it instead (a `borrow` parameter), or use it before it goes";

    #[test]
    fn an_owned_value_can_be_made_lent_and_given_away() {
        owned_ok("let p = make()\nprint(look(p), p.size(), p.id, p.data[0], p)\nprint(send(p))");
        owned_ok("let xs: owned list[int] = [1, 2]\nprint(total(xs), len(xs), xs)\nxs.append(3)\nfor x in xs { print(x) }");
        // A shared list can be lent too: anyone may look at it.
        owned_ok("let xs = [1, 2]\nprint(total(xs), total([3]))");
        owned_ok("let p = make()\nlet q = p\nprint(send(q))");
        // An owned value nobody keeps is simply freed.
        owned_ok("make()\nPacket(2, [])");
        owned_ok("function build(n: int) -> owned list[int] {\n let xs: owned list[int] = [i for i in range(n)]\n return xs\n}\nlet ys = build(3)\nprint(ys)");
    }

    #[test]
    fn a_value_that_was_given_away_cannot_be_used() {
        assert_eq!(owned_error("let p = make()\nsend(p)\nprint(p.id)"), format!("`p` was given away on line 11 and {GONE}"));
        assert_eq!(owned_error("let p = make()\nlet q = p\nlet r = p"), format!("`p` was given away on line 11 and {GONE}"));
        // Given away on one path is not there afterwards on any path.
        assert_eq!(
            owned_error("let p = make()\nif p.id > 0 { send(p) }\nprint(look(p))"),
            format!("`p` was given away on line 11 and {GONE}")
        );
        // Unless that path never reaches what follows.
        owned_ok("function f(p: Packet) -> int {\n if p.id > 0 { return send(p) }\n return look(p)\n}");
        // A `var` can be given a new value, and holds something again.
        owned_ok("var p = make()\nsend(p)\np = make()\nprint(p.id)");
        // An error can leave a `try` after the value has gone.
        assert_eq!(
            owned_error("class Bad { }\nfunction risky(p: Packet) raises Bad { raise Bad() }\nlet p = make()\ntry { risky(p) } except Bad { print(p.id) }"),
            format!("`p` was given away on line 13 and {GONE}")
        );
    }

    #[test]
    fn giving_away_inside_a_loop() {
        let again = "is given away inside a loop, and the loop may come round again with nothing in it. \
                     Give it away after the loop, or just before a `break`";
        assert_eq!(owned_error("let p = make()\nfor i in range(3) { send(p) }"), format!("`p` {again}"));
        assert_eq!(owned_error("let p = make()\nwhile send(p) > 0 { }"), format!("`p` {again}"));
        assert_eq!(owned_error("let p = make()\nlet xs = [send(p) for i in range(3)]"), format!("`p` {again}"));
        assert_eq!(
            owned_error("let p = make()\nfor i in range(3) {\n if i == 1 {\n  send(p)\n  continue\n }\n}"),
            format!("`p` {again}")
        );
        // Just before a `break` is fine: the loop does not come round again.
        owned_ok("let p = make()\nwhile true {\n if p.id > 0 {\n  send(p)\n  break\n }\n}");
        // But it is gone after the loop.
        assert_eq!(
            owned_error("let p = make()\nwhile true {\n if p.id > 0 {\n  send(p)\n  break\n }\n}\nprint(p.id)"),
            format!("`p` was given away on line 13 and {GONE}")
        );
        // A value made inside the loop is a new one every time round.
        owned_ok("for i in range(3) {\n let p = make()\n send(p)\n}");
        owned_ok("var p = make()\nfor i in range(3) {\n send(p)\n p = make()\n}");
    }

    #[test]
    fn a_borrowed_value_cannot_be_kept() {
        assert_eq!(
            owned_error("function keep(p: borrow Packet) -> Packet { return p }"),
            "`p` is only borrowed here, so it cannot be kept, returned or given away. To keep it, take it as an owned parameter instead"
        );
        assert_eq!(
            owned_error("function pass_on(p: borrow Packet) -> int { return send(p) }"),
            "`p` is only borrowed here, so it cannot be kept, returned or given away. To keep it, take it as an owned parameter instead"
        );
        assert_eq!(
            owned_error("let p = make()\nlet d = p.data"),
            "`data` holds an owned value, which cannot be taken out of the object it is in. Use it where it is, or lend it to a `borrow` parameter"
        );
        assert_eq!(
            owned_error("function f(xs: list[int]) -> int { return len(xs) }\nlet ys: owned list[int] = [1]\nprint(f(ys))"),
            "`xs` of `f` should be list[int], but this is a borrowed list[int]. A value with an owner can only go to a parameter that borrows it (`borrow`) or takes it over (an owned type)"
        );
        assert_eq!(
            owned_error("let p = make()\nlet m = p.size"),
            "a method of an owned or borrowed value cannot be kept as a value. Call it here: `.size(...)`"
        );
        assert_eq!(
            owned_error("function reset(p: borrow Packet) { p.data = [] }"),
            "`data` holds an owned value, and only the owner of the object can replace it. Here the object is only borrowed"
        );
        owned_ok("let p = make()\np.data = [5, 6]\nprint(p)");
    }

    #[test]
    fn a_line_cannot_use_a_value_and_give_it_away() {
        assert_eq!(
            owned_error("let p = make()\nprint(look(p) + send(p))"),
            "`p` is given away here, on a line that also uses it. Do the other thing on a line of its own first"
        );
        assert_eq!(owned_error("let p = make()\nprint(send(p) + look(p))"), format!("`p` was given away on line 11 and {GONE}"));
    }

    #[test]
    fn an_owned_value_with_no_name_belongs_to_its_statement() {
        // Made in the middle of a statement and only looked at: the
        // statement owns it. (These were refused once: "has no owner yet".)
        owned_ok("print(look(make()))");
        owned_ok("print(make().size(), make().id, len(make().data))");
        owned_ok("if look(make()) > 0 { print(1) }");
        owned_ok("for x in make().data { print(x) }");
        // It still cannot be kept by something that is not an owner.
        assert_eq!(
            owned_error("let xs = [1, 2]\nlet ys: owned list[int] = xs"),
            "`ys` should be owned list[int], but this is list[int]. Only a list made right here can become an owned one: \
             one written out, as in `[1, 2]` or `[x for x in xs]`, or what `split`, `map`, `filter` or `+` gives"
        );
        assert_eq!(
            owned_error("let data = make().data"),
            "`data` holds an owned value, which cannot be taken out of the object it is in. \
             Use it where it is, or lend it to a `borrow` parameter"
        );
    }

    #[test]
    fn a_list_that_was_just_made_can_start_out_owned() {
        owned_ok("let words: owned list[str] = \"a b\".split()\nprint(words)");
        owned_ok("let parts: owned list[str] = \"a,b\".split(\",\")\nprint(total([len(p) for p in parts]))");
        owned_ok(
            "function double(n: int) -> int { return n * 2 }\nfunction big(n: int) -> bool { return n > 1 }\n\
             let xs: owned list[int] = [1, 2]\nlet ys: owned list[int] = xs.map(double)\n\
             let zs: owned list[int] = xs.filter(big)\nlet all: owned list[int] = xs + ys + zs\nprint(total(all), total(xs + ys))",
        );
        owned_ok("function words(text: str) -> owned list[str] { return text.split() }\nprint(len(words(\"a b\")))");
        // `xs += ys` is `xs = xs + ys`: the list that `+` makes takes the
        // old one's place.
        owned_ok("var xs: owned list[int] = [1]\nxs += [2]\nxs += xs\nprint(xs)");
        assert_eq!(
            owned_error("var xs: owned list[int] = [1]\nxs += 2"),
            "`+` does not work between list[int] and int"
        );
    }

    #[test]
    fn an_owned_value_stays_in_the_function_that_holds_it() {
        let elsewhere = "holds an owned value, which belongs to the block that declares it. \
                         A function written elsewhere cannot reach it. Lend it to the function through a `borrow` parameter";
        assert_eq!(
            owned_error("let p = make()\nfunction peek() -> int { return p.id }\nprint(peek())"),
            format!("`p` {elsewhere}")
        );
        assert_eq!(owned_error("var p = make()\nfunction swap() { p = make() }"), format!("`p` {elsewhere}"));
        assert_eq!(
            owned_error("function f(p: borrow Packet) -> function() -> int {\n return function() -> int { return p.id }\n}"),
            format!("`p` {elsewhere}")
        );
    }

    #[test]
    fn shared_things_cannot_hold_owned_ones() {
        let inside = "an owned Packet cannot be kept inside a list, an `or none` or a function type yet. \
                      It lives in a variable, a parameter, or a field of an owned class";
        assert_eq!(owned_error("let xs: list[Packet] = []"), inside);
        assert_eq!(owned_error("let x: Packet or none = none"), inside);
        assert_eq!(owned_error("let x: owned list[Packet] = []"), inside);
        assert_eq!(
            owned_error("let p = make()\nlet xs = [p]"),
            "a list cannot hold an owned or borrowed value yet, and this is a borrowed Packet"
        );
        assert_eq!(
            owned_error("class Box { inside: Packet }"),
            "`inside` holds an owned value, so `Box` must be an `owned class` itself. A shared object cannot own anything"
        );
        assert_eq!(owned_error("enum E { A(p: Packet) }"), "a case of the enum `E` cannot carry an owned value yet");
        assert_eq!(
            owned_error("let p = make()\nraise p"),
            "an owned or borrowed value cannot be raised, and this is a borrowed Packet"
        );
        assert_eq!(
            owned_error("let p = make()\nmatch p { case _ { print(1) } }"),
            "`match` cannot take an owned or borrowed value, and this is a borrowed Packet. Match on one of its fields"
        );
    }

    #[test]
    fn where_owned_and_borrow_can_be_written() {
        assert_eq!(
            owned_error("let x: borrow list[int] = [1]"),
            "`borrow` can only be written on a parameter. A borrowed value cannot outlive the call it is lent to"
        );
        assert_eq!(
            owned_error("function f() -> borrow Packet { return make() }"),
            "`borrow` can only be written on a parameter. A borrowed value cannot outlive the call it is lent to"
        );
        assert_eq!(
            owned_error("let x: owned int = 1"),
            "`owned` goes in front of a list type, as in `owned list[int]`, and this is int. A class is made owned where it is declared: `owned class`"
        );
        assert_eq!(
            owned_error("function f(n: borrow int) { }"),
            "only a list or an object can be borrowed, and this is int. Pass it as it is"
        );
        assert_eq!(
            owned_error("let x: list[owned list[int]] = []"),
            "`owned` and `borrow` describe a whole value, so they go at the very front of a type, and not inside `list[ ]`, `or none` or a function type"
        );
        // `owned` and `borrow` are ordinary names everywhere else.
        owned_ok("let owned = 1\nlet borrow = owned + 1\nprint(owned, borrow)");
    }

    #[test]
    fn who_owns_a_value_is_never_guessed() {
        assert_eq!(
            owned_error("function f(q) { print(q) }\nlet p = make()\nf(p)"),
            "strict mode: the parameter `q` of `f` needs a type. It could not be worked out from how it is used"
        );
    }

    // ----- A function is not used before what it needs exists -----

    #[test]
    fn a_function_is_not_called_above_a_variable_it_uses() {
        // `x` in `inner` means the `x` of `outer`, which has no value yet
        // at the call. It must not be taken for the `x` further out.
        assert_eq!(
            error(
                "let x = 1\nfunction outer() {\n    print(inner())\n    let x = 5\n    function inner() { return x }\n}\nouter()"
            ),
            "`x` is declared further down than the first use of `inner`, which needs it. \
             Use `inner` below the line that declares `x`"
        );
        // The same with nothing of that name further out.
        assert_eq!(
            error("print(show())\nlet name = \"zeno\"\nfunction show() { return name }"),
            "`name` is declared further down than the first use of `show`, which needs it. \
             Use `show` below the line that declares `name`"
        );
        // Reached through another function, and by an assignment. Each
        // name is reported once.
        assert_eq!(
            error(
                "var n = 1\nfunction outer() {\n    step()\n    var n = 5\n    function add() { n = n + 1 }\n    function step() { add() }\n}\nouter()"
            ),
            "`n` is declared further down than the first use of `add`, which needs it. \
             Use `add` below the line that declares `n`"
        );
        // A method is a function of the block its class is in.
        assert_eq!(
            error(
                "let size = 1\nfunction make() {\n    let b = Box(2)\n    print(b.grow())\n    let size = 9\n    class Box {\n        side: int\n        function grow(self) { return self.side + size }\n    }\n}\nmake()"
            ),
            "`size` is declared further down than the first use of `grow`, which needs it. \
             Use `grow` below the line that declares `size`"
        );
    }

    #[test]
    fn a_class_is_not_built_above_a_variable_its_defaults_use() {
        assert_eq!(
            error(
                "let size = 1\nfunction make() {\n    let b = Box()\n    let size = 9\n    class Box {\n        side: int = size\n    }\n    return b.side\n}\nprint(make())"
            ),
            "`Box` cannot be built here yet. The default of its field `side` uses `size`, which is declared \
             further down. Build it below the line that declares `size`"
        );
        ok("let size = 9\nclass Box {\n    side: int = size\n}\nprint(Box().side)");
    }

    #[test]
    fn a_function_may_be_used_once_what_it_needs_exists() {
        // Called below the variable, though written above the call.
        ok("let x = 1\nfunction outer() {\n    let x = 5\n    print(inner())\n    function inner() { return x }\n}\nouter()");
        // Named above the variable but called below it.
        ok("let x = 1\nfunction outer() {\n    let f = inner\n    let x = 5\n    function inner() { return x }\n    print(f())\n}\nouter()");
        // A variable of the function's own, and one further out.
        ok("let x = 1\nfunction outer() {\n    print(inner())\n    function inner() {\n        let y = x\n        return y\n    }\n}\nouter()");
        // Two functions that call each other.
        ok("print(even(4))\nfunction even(n) {\n    if n == 0 { return true }\n    return odd(n - 1)\n}\nfunction odd(n) {\n    if n == 0 { return false }\n    return even(n - 1)\n}");
    }

    // ----- Reporting -----

    #[test]
    fn every_error_is_reported_in_order() {
        assert_eq!(
            errors("let a: int = \"x\"\nprint(missing)\nlet b = 1\nb = 2"),
            vec![
                "`a` should be int, but this is str",
                "`missing` is not defined",
                "`b` cannot be changed. Declare it with `var` to allow that",
            ]
        );
    }

    #[test]
    fn the_examples_pass() {
        ok(include_str!("../examples/hello.zeno"));
        ok(include_str!("../examples/tour.zeno"));
        ok(include_str!("../examples/styles.zeno"));
        // The tour is fully typed, so it passes strict mode as well.
        strict_ok(include_str!("../examples/tour.zeno"));
        strict_ok(include_str!("../examples/buffer.zeno"));
        strict_ok(include_str!("../examples/labels.zeno"));
        strict_ok(include_str!("../examples/words.zeno"));
    }
}
