//! The interpreter: runs a program by walking its syntax tree.
//!
//! There are two central functions, one for each kind of tree node:
//!
//! - `exec` runs a statement. It returns nothing, but changes things:
//!   declares a variable, prints, loops.
//! - `eval` works out an expression and returns its `Value`.
//!
//! Both are a `match` on the node kind, and both call themselves for the
//! nodes nested inside. That is all a "tree-walking" interpreter is.
//!
//! Types written in the source (`x: int`) are not checked here. The
//! interpreter finds mistakes only when it reaches them; the type checker,
//! the next stage, will find them before the program runs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;
use crate::board::Board;
use crate::page::Page;
use crate::builtins;
use crate::error::{counted, Error};
use crate::ops;
use crate::token::Span;
use crate::value::*;

/// How deep function calls may nest before we assume runaway recursion.
#[cfg(not(target_arch = "wasm32"))]
const MAX_DEPTH: usize = 1000;
/// In a browser the stack is the browser's, and far smaller. A program
/// that goes past the end of it is stopped by the browser, which cannot
/// say which line of Zeno did it, so the limit is set where Zeno can
/// still say so. Chromium was measured: a plain function that calls
/// itself got past 700 calls and not to 1000, and one that calls itself
/// from inside a loop, a `match` and a `map` got past 400 and not to
/// 500. 200 leaves room for a browser with half that stack.
#[cfg(target_arch = "wasm32")]
const MAX_DEPTH: usize = 200;
/// The interpreter calls itself for every nested node, so deep Zeno
/// recursion needs a deep Rust stack. It runs on its own thread to get one.
const STACK_SIZE: usize = 256 * 1024 * 1024;
/// In a browser, how long one turn of a program (its first run, or one
/// click, key or timer) may go on before it is stopped. A page does
/// nothing else while Zeno runs: not draw, not scroll, not let anyone
/// press a button to stop it. A loop that never ends would hold the
/// page, and the playground with it, for good (decisions 19.1).
#[cfg(target_arch = "wasm32")]
const TURN_MS: f64 = 5000.0;
/// The clock is asked once in this many loop rounds and calls: asking
/// it crosses into JavaScript, and that costs more than a round.
#[cfg(target_arch = "wasm32")]
const TICKS_PER_LOOK: u32 = 1 << 16;

/// Run a program. With `capture` the printed text is returned instead of
/// going to the screen, which is how the tests check output. `outside`
/// is what is typed at the program, and the words after the file on the
/// command line.
pub fn run_with(program: &Program, capture: bool, outside: Outside) -> (String, Result<(), Error>) {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, || {
                let mut interpreter =
                    Interpreter { output: capture.then(String::new), depth: 0, board: Board::new(), page: Page::simulated(outside.page.as_deref()), outside, ticks: 0, turn_began: 0.0 };
                // Built-ins live in the outermost scope; the program gets its own inside it.
                let env = Scope::child(&builtins::globals());
                let result = interpreter
                    .run_statements(&program.statements, &env)
                    .map_err(ops::escaped);
                (interpreter.output.unwrap_or_default(), result)
            })
            .expect("could not start the interpreter thread")
            .join()
            .expect("the interpreter crashed")
    })
}

/// One test, after it was run: its name, and what went wrong if
/// anything did.
pub struct Tested {
    pub name: String,
    pub failure: Option<Error>,
}

/// `zeno test`: run the program, and then each test that is written in
/// its first file, in the order written. The tests of the files it
/// imports are those files' own, and are run when they are tested.
/// With `only`, the tests that have that text in their name are run
/// and the others are passed over.
/// Each test says `ok` or `FAILED` on a line as it ends. `Err` if the
/// program itself stopped before its tests could run.
pub fn run_tests(program: &Program, capture: bool, outside: Outside, only: Option<&str>) -> (String, Result<Vec<Tested>, Error>) {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, || {
                let mut interpreter =
                    Interpreter { output: capture.then(String::new), depth: 0, board: Board::new(), page: Page::simulated(outside.page.as_deref()), outside, ticks: 0, turn_began: 0.0 };
                let env = Scope::child(&builtins::globals());
                let started = interpreter.run_statements(&program.statements, &env).map_err(ops::escaped);
                let mut tested = Vec::new();
                if started.is_ok() {
                    for statement in &program.statements {
                        let StmtKind::Test { name, body } = &statement.kind else {
                            continue;
                        };
                        if statement.span.file() != 0 {
                            continue;
                        }
                        // `zeno test --only "empty"`: the tests with
                        // that in their name, and no others.
                        if only.is_some_and(|text| !name.contains(text)) {
                            continue;
                        }
                        // A test that stopped half way may have left
                        // calls open.
                        interpreter.depth = 0;
                        let failure = interpreter.exec_block(body, &env).map_err(ops::escaped).err();
                        let word = if failure.is_some() { "FAILED" } else { "ok    " };
                        interpreter.write_line(&format!("  {word}  {name}"));
                        tested.push(Tested { name: name.clone(), failure });
                    }
                }
                (interpreter.output.unwrap_or_default(), started.map(|()| tested))
            })
            .expect("could not start the interpreter thread")
            .join()
            .expect("the interpreter crashed")
    })
}

/// A program that stays: one in a page.
///
/// `zeno run` runs a program from its first line to its last, and that
/// is the end of it. A page is not like that. The program runs to its
/// last line, and then the page is still there, and somebody presses a
/// button: the function the program gave to `on` has to be called, with
/// every variable as the program left it. So the interpreter and the
/// program's scope are kept, here, for as long as the page is open.
///
/// There is no thread with a deep stack here, as `run_with` has: a
/// browser gives a program one thread and one stack.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub struct Session {
    interpreter: Interpreter,
    env: Env,
    program: Program,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
impl Session {
    /// Run the program from its first line to its last, on this page.
    /// What it printed is given back if `capture` says to keep it.
    pub fn start(program: Program, page: Page, capture: bool) -> (Session, Result<(), Error>) {
        let interpreter = Interpreter {
            output: capture.then(String::new),
            depth: 0,
            board: Board::new(),
            page,
            outside: Outside::default(),
            ticks: 0,
            turn_began: 0.0,
        };
        let env = Scope::child(&builtins::globals());
        let mut session = Session { interpreter, env, program };
        session.interpreter.begin_turn();
        let result = session.interpreter.run_statements(&session.program.statements, &session.env).map_err(ops::escaped);
        (session, result)
    }

    /// The page calls one of the functions it was given: the one that
    /// `on`, `after` or `on_key` kept under this number.
    pub fn call(&mut self, handler: u32, arguments: Vec<Value>) -> Result<(), Error> {
        let Some(handler) = self.interpreter.page.handler(handler) else {
            return Ok(());
        };
        // The event goes to a function that asked for it.
        let arguments = arguments.into_iter().filter(|argument| handler.takes_event || !matches!(argument, Value::Event(_))).collect();
        let function = handler.function;
        // An error may have left calls open the last time.
        self.interpreter.depth = 0;
        self.interpreter.begin_turn();
        let span = Span { line: 1, col: 1 };
        self.interpreter.call(function, arguments, Vec::new(), span).map(|_| ()).map_err(ops::escaped)
    }

    /// What has been printed and kept since the last time this was asked.
    #[cfg(test)]
    pub fn printed(&mut self) -> String {
        self.interpreter.output.as_mut().map(std::mem::take).unwrap_or_default()
    }
}

// --------------------------------------------------------------------- Scopes

/// A scope is one set of variables. Scopes nest: each `{ }` block gets a new
/// one whose `parent` is the scope around it. Looking up a name walks outward
/// until it is found.
pub struct Scope {
    variables: RefCell<HashMap<String, Rc<Variable>>>,
    parent: Option<Env>,
    /// Set for a scope that is a *view* of another (see `seen`). A view
    /// has no variables of its own.
    view: Option<View>,
}

/// What a function can see of the block it is written in.
struct View {
    /// The block's scope.
    of: Env,
    /// How many of that scope's variables can be seen: the ones declared
    /// first.
    count: usize,
    /// The names of the variables among those that the block declares
    /// with `let`, `var`, `const` or `import`. When the function is called
    /// from above the line that declares one of them, the variable does
    /// not exist yet. The name still means that variable, and not one of
    /// the same name further out.
    lets: Rc<[String]>,
}

/// Scopes are shared (`Rc`) because a function keeps hold of the scope it
/// was defined in for as long as the function exists.
pub type Env = Rc<Scope>;

/// One variable. It is shared (`Rc`) because a function written inside
/// another keeps hold of the variables it uses: both must see the same
/// variable, so that a change made by one is seen by the other.
struct Variable {
    value: RefCell<Value>,
    /// True for `var`, false for `let`, `const`, parameters and loop variables.
    mutable: bool,
    /// How many variables its scope had before this one: 0 for the first
    /// to be declared there, 1 for the second.
    number: usize,
}

enum AssignError {
    NotDeclared,
    Immutable,
}

impl Scope {
    pub fn root() -> Env {
        Rc::new(Scope { variables: RefCell::new(HashMap::new()), parent: None, view: None })
    }

    pub fn child(parent: &Env) -> Env {
        Rc::new(Scope { variables: RefCell::new(HashMap::new()), parent: Some(parent.clone()), view: None })
    }

    /// Create a variable in this scope. Returns false if the name is taken here.
    pub fn declare(&self, name: &str, value: Value, mutable: bool) -> bool {
        let mut variables = self.variables.borrow_mut();
        if variables.contains_key(name) {
            return false;
        }
        let number = variables.len();
        variables.insert(name.to_string(), Rc::new(Variable { value: RefCell::new(value), mutable, number }));
        true
    }

    /// How many variables have been declared in this scope so far.
    fn count(&self) -> usize {
        self.variables.borrow().len()
    }

    /// The scope a function keeps: what can be seen from where it is
    /// written, and nothing that is declared later.
    ///
    /// Keeping `env` itself would be wrong. A variable declared further
    /// down the block would then be visible inside the function, and could
    /// hide an outer variable of the same name that the function was
    /// written to use.
    ///
    /// So the function gets a **view** of `env`: a scope with no variables
    /// of its own, which looks into `env` and sees only the first `count`
    /// variables declared there. They are the same variables, not copies,
    /// so changes still pass both ways. The scopes around `env` are treated
    /// the same way, each seen as it is at this moment.
    fn seen(env: &Env, count: usize, lets: &[String]) -> Env {
        // The built-ins never gain a variable, and a view never changes.
        let Some(parent) = &env.parent else {
            return env.clone();
        };
        if env.view.is_some() {
            return env.clone();
        }
        let parent = Scope::seen(parent, parent.count(), &[]);
        Rc::new(Scope {
            variables: RefCell::new(HashMap::new()),
            parent: Some(parent),
            view: Some(View { of: env.clone(), count, lets: Rc::from(lets) }),
        })
    }

    /// Find a variable by name, here or in a scope around this one.
    fn find(&self, name: &str) -> Option<Rc<Variable>> {
        let here = match &self.view {
            Some(view) => {
                let seen =
                    view.of.variables.borrow().get(name).filter(|variable| variable.number < view.count).cloned();
                if seen.is_none() && view.lets.iter().any(|declared| declared == name) {
                    // The block declares it, on a line that has not run yet.
                    return None;
                }
                seen
            }
            None => self.variables.borrow().get(name).cloned(),
        };
        here.or_else(|| self.parent.as_ref()?.find(name))
    }

    fn get(&self, name: &str) -> Option<Value> {
        Some(self.find(name)?.value.borrow().clone())
    }

    fn assign(&self, name: &str, value: Value) -> Result<(), AssignError> {
        let variable = self.find(name).ok_or(AssignError::NotDeclared)?;
        if !variable.mutable {
            return Err(AssignError::Immutable);
        }
        *variable.value.borrow_mut() = value;
        Ok(())
    }
}

// ---------------------------------------------------------------- Interpreter

pub struct Interpreter {
    /// `Some` collects printed text; `None` prints straight to the screen.
    output: Option<String>,
    /// The simulated board, for the `board` module.
    board: Board,
    /// The page, for the `web` module: simulated, unless the program is
    /// in a browser.
    page: Page,
    /// The keyboard and the command line, for `input` and `program`.
    outside: Outside,
    /// How many function calls are currently in progress.
    depth: usize,
    /// Loop rounds and calls since the clock was last looked at, and
    /// when this turn began (only looked at in a browser: see `tick`).
    ticks: u32,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    turn_began: f64,
}

/// What the built-ins ask of the interpreter (see `Machine` in value.rs).
impl Machine for Interpreter {
    fn write_line(&mut self, text: &str) {
        match &mut self.output {
            Some(buffer) => {
                buffer.push_str(text);
                buffer.push('\n');
            }
            None => print_line(text),
        }
    }

    fn write(&mut self, text: &str) {
        match &mut self.output {
            Some(buffer) => buffer.push_str(text),
            None => print_text(text),
        }
    }

    fn outside(&mut self) -> &mut Outside {
        &mut self.outside
    }

    fn board(&mut self) -> &mut Board {
        &mut self.board
    }

    fn page(&mut self) -> &mut Page {
        &mut self.page
    }

    fn call(
        &mut self,
        callee: Value,
        positional: Vec<Value>,
        named: Vec<(String, Value)>,
        span: Span,
    ) -> Result<Value, Control> {
        Interpreter::call(self, callee, positional, named, span)
    }
}

/// The scope a function or class made by the interpreter was defined in.
fn scope_of(home: &Home) -> Env {
    match home {
        Home::Scope(scope) => scope.clone(),
        Home::Code(..) => unreachable!("the interpreter met a function that the VM made"),
    }
}

impl Interpreter {

    /// A turn of the program begins: its first run, or a call from the
    /// page. The time it may take is counted from here.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn begin_turn(&mut self) {
        self.ticks = 0;
        #[cfg(target_arch = "wasm32")]
        {
            self.turn_began = crate::web_host::now();
        }
    }

    /// One more round of a loop, or one more call. In a browser, a turn
    /// that has gone on for longer than `TURN_MS` is stopped, with the
    /// line it was on. Anywhere else a program runs for as long as it
    /// likes, as programs do, and this does nothing.
    #[inline]
    fn tick(&mut self, span: Span) -> Result<(), Control> {
        #[cfg(target_arch = "wasm32")]
        {
            self.ticks = self.ticks.wrapping_add(1);
            if self.ticks % TICKS_PER_LOOK == 0 && crate::web_host::now() - self.turn_began > TURN_MS {
                let seconds = TURN_MS / 1000.0;
                return fatal(
                    format!(
                        "the program ran for {seconds} seconds without stopping, so it was stopped. \
                         A page can do nothing else while it runs. Is a loop going on forever?"
                    ),
                    span,
                );
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = span;
        Ok(())
    }

    fn declare(
        &self,
        env: &Env,
        name: &str,
        value: Value,
        mutable: bool,
        span: Span,
    ) -> Result<(), Control> {
        if env.declare(name, value, mutable) {
            Ok(())
        } else {
            fatal(format!("`{name}` is already declared in this block"), span)
        }
    }

    // ------------------------------------------------------------ Statements

    /// `assert total == 6`. A failure is not an error of the program
    /// that an `except` could catch: it ends the test at once, and says
    /// what was on each side of a comparison.
    fn assert(&mut self, condition: &Expr, env: &Env) -> Result<(), Control> {
        let span = condition.span;
        let compares = |op: &BinaryOp| {
            use BinaryOp::*;
            matches!(op, Eq | NotEq | Lt | LtEq | Gt | GtEq | Is | IsNot | In | NotIn)
        };
        match &condition.kind {
            ExprKind::Binary { op, left, right } if compares(op) => {
                let left = self.eval(left, env)?;
                let right = self.eval(right, env)?;
                let (was_left, was_right) = (left.repr(), right.repr());
                match ops::binary(*op, left, right, span)? {
                    Value::Bool(true) => Ok(()),
                    _ => fatal(
                        format!(
                            "this is false. The left of `{}` is {was_left}, and the right is {was_right}",
                            op.text()
                        ),
                        span,
                    ),
                }
            }
            _ => match self.eval(condition, env)? {
                Value::Bool(true) => Ok(()),
                _ => fatal("this is false", span),
            },
        }
    }

    /// Run a `{ }` block in a fresh scope of its own.
    fn exec_block(&mut self, statements: &[Stmt], env: &Env) -> Result<(), Control> {
        self.run_statements(statements, &Scope::child(env))
    }

    /// Run statements in the given scope, one after another.
    fn run_statements(&mut self, statements: &[Stmt], env: &Env) -> Result<(), Control> {
        self.hoist(statements, env)?;
        for statement in statements {
            self.exec(statement, env)?;
        }
        Ok(())
    }

    /// Declare the block's functions, classes and enums before running any
    /// of it, so they can be used above the line where they are written.
    ///
    /// Each sees the block as it will be when the program reaches the line
    /// it is written on: everything declared here, and the variables
    /// declared above that line. Those variables do not exist yet, so what
    /// is worked out is how many there will be (`above`) and what they are
    /// called (`lets`).
    fn hoist(&mut self, statements: &[Stmt], env: &Env) -> Result<(), Control> {
        let hoisted: usize = statements
            .iter()
            .map(|statement| match &statement.kind {
                StmtKind::Function(_) | StmtKind::Class(_) => 1,
                StmtKind::Enum(decl) => 1 + decl.variants.len(),
                _ => 0,
            })
            .sum();
        let mut above = env.count() + hoisted;
        let mut lets: Vec<String> = Vec::new();

        for statement in statements {
            let span = statement.span;
            match &statement.kind {
                StmtKind::Function(decl) => {
                    let home = Scope::seen(env, above, &lets);
                    let closure = Closure { decl: decl.clone(), home: Home::Scope(home) };
                    self.declare(env, &decl.name, Value::Function(Rc::new(closure)), false, span)?;
                }
                StmtKind::Class(class) => {
                    let home = Scope::seen(env, above, &lets);
                    let methods = class
                        .methods
                        .iter()
                        .map(|decl| {
                            let closure = Closure { decl: decl.clone(), home: Home::Scope(home.clone()) };
                            (decl.name.clone(), Rc::new(closure))
                        })
                        .collect();
                    let def = ClassDef {
                        name: class.name.clone(),
                        fields: class.fields.clone(),
                        methods,
                        home: Home::Scope(home),
                    };
                    self.declare(env, &class.name, Value::Class(Rc::new(def)), false, span)?;
                }
                StmtKind::Enum(decl) => {
                    let def = Rc::new(EnumDef { name: decl.name.clone(), cases: decl.variants.clone() });
                    self.declare(env, &decl.name, Value::Enum(def.clone()), false, span)?;
                    // Each case is also a name of its own, so you can write
                    // `Circle(1.0)` as well as `Shape.Circle(1.0)`.
                    for (index, case) in decl.variants.iter().enumerate() {
                        self.declare(env, &case.name, ops::case_value(&def, index), false, span)?;
                    }
                }
                // Each of these adds to what the functions below it can see.
                StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => {
                    above += 1;
                    lets.push(name.clone());
                }
                StmtKind::Import { module, names } => {
                    // `import a.b` declares one name, the module's.
                    above += names.len().max(1);
                    lets.extend(names.iter().cloned());
                    if names.is_empty() {
                        lets.extend(module.last().cloned());
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn exec(&mut self, statement: &Stmt, env: &Env) -> Result<(), Control> {
        let span = statement.span;

        match &statement.kind {
            StmtKind::Let { mutable, name, value, .. } => {
                let value = self.eval(value, env)?;
                self.declare(env, name, value, *mutable, span)
            }
            StmtKind::Const { name, value, .. } => {
                let value = self.eval(value, env)?;
                self.declare(env, name, value, false, span)
            }
            StmtKind::Assign { target, op, value } => {
                let new = match op {
                    None => self.eval(value, env)?,
                    // `x += 1` is `x = x + 1`: what is there is read,
                    // and then the value is worked out.
                    Some(op) => {
                        let old = self.eval(target, env)?;
                        let new = self.eval(value, env)?;
                        ops::binary(*op, old, new, span)?
                    }
                };
                self.assign(target, new, env)
            }
            StmtKind::Expr(expr) => {
                self.eval(expr, env)?;
                Ok(())
            }

            // Already declared by `hoist`.
            StmtKind::Function(_) | StmtKind::Class(_) | StmtKind::Enum(_) => Ok(()),

            StmtKind::If { condition, then_block, else_block } => {
                if self.condition(condition, env)? {
                    self.exec_block(then_block, env)
                } else if let Some(else_block) = else_block {
                    self.exec_block(else_block, env)
                } else {
                    Ok(())
                }
            }
            // The checker has seen to what may be written inside. To
            // run, it is a block like any other.
            StmtKind::Unsafe(body) => self.exec_block(body, env),
            StmtKind::While { condition, body } => {
                while self.condition(condition, env)? {
                    self.tick(span)?;
                    match self.exec_block(body, env) {
                        Ok(()) | Err(Control::Continue(_)) => {}
                        Err(Control::Break(_)) => break,
                        Err(other) => return Err(other),
                    }
                }
                Ok(())
            }
            StmtKind::For { variable, second, iterable, body } => {
                let iterable_span = iterable.span;
                let iterable = self.eval(iterable, env)?;
                // With one name each round is given an item. With two it
                // is given a key, or a position, and what is there.
                let rounds: Vec<(Value, Option<Value>)> = match second {
                    None => ops::items(&iterable, iterable_span)?.into_iter().map(|item| (item, None)).collect(),
                    Some(_) => {
                        ops::pairs(&iterable, iterable_span)?.into_iter().map(|(key, value)| (key, Some(value))).collect()
                    }
                };
                for (item, value) in rounds {
                    self.tick(span)?;
                    // A fresh scope and a fresh loop variable for every round.
                    let scope = Scope::child(env);
                    scope.declare(variable, item, false);
                    if let (Some(second), Some(value)) = (second, value) {
                        scope.declare(second, value, false);
                    }
                    match self.run_statements(body, &scope) {
                        Ok(()) | Err(Control::Continue(_)) => {}
                        Err(Control::Break(_)) => break,
                        Err(other) => return Err(other),
                    }
                }
                Ok(())
            }
            StmtKind::Match { subject, arms } => {
                let value = self.eval(subject, env)?;
                for arm in arms {
                    // The pattern's variables live in the arm's own scope.
                    let scope = Scope::child(env);
                    if self.matches(&arm.pattern, &value, &scope, span)? {
                        return self.run_statements(&arm.body, &scope);
                    }
                }
                fatal(format!("no `case` matches {}", value.repr()), span)
            }
            StmtKind::Try { body, handlers } => {
                let (error, raised_at) = match self.exec_block(body, env) {
                    Err(Control::Raise(error, span)) => (error, span),
                    other => return other,
                };
                let kind = ops::error_kind(&error);
                for handler in handlers {
                    if ops::catches(handler.error.as_deref(), &kind) {
                        let scope = Scope::child(env);
                        if let Some(binding) = &handler.binding {
                            scope.declare(binding, ops::caught(handler.error.as_deref(), &error), false);
                        }
                        return self.run_statements(&handler.body, &scope);
                    }
                }
                Err(Control::Raise(error, raised_at))
            }

            // The tests of a program are run by `zeno test`, after the
            // program (see `run_tests`). `zeno run` goes past them.
            StmtKind::Test { .. } => Ok(()),
            StmtKind::Assert(condition) => self.assert(condition, env),
            StmtKind::AssertRaises { error, body } => match self.exec_block(body, env) {
                Ok(()) => fatal(format!("this block was expected to raise {error}, and it raised nothing"), span),
                Err(Control::Raise(raised, at)) => {
                    let kind = ops::error_kind(&raised);
                    if ops::catches(Some(error), &kind) {
                        return Ok(());
                    }
                    let mut wrong = Error::new(
                        format!(
                            "this block was expected to raise {error}, and it raised {}: {raised}",
                            crate::ast::shown_name(&kind)
                        ),
                        span,
                    );
                    wrong.note = Some(format!("that error was raised on line {}", at.line_in_file()));
                    Err(Control::Fatal(wrong))
                }
                other => other,
            },

            StmtKind::Return(value) => {
                let value = match value {
                    Some(value) => self.eval(value, env)?,
                    None => Value::None,
                };
                Err(Control::Return(value, span))
            }
            StmtKind::Raise(value) => {
                let error = self.eval(value, env)?;
                Err(ops::raised(error, span))
            }
            StmtKind::Break => Err(Control::Break(span)),
            StmtKind::Continue => Err(Control::Continue(span)),

            StmtKind::Import { module, names } => {
                let path = module.join(".");
                let Some(found) = builtins::module(&path) else {
                    return fatal(
                        format!("there is no module `{path}`. {}", builtins::MODULES),
                        span,
                    );
                };
                if names.is_empty() {
                    // `import math` makes the name `math`.
                    return self.declare(env, found.name, Value::Module(found.clone()), false, span);
                }
                for name in names {
                    let Some(member) = found.members.get(name.as_str()) else {
                        return fatal(format!("module `{path}` has no `{name}`"), span);
                    };
                    self.declare(env, name, member.clone(), false, span)?;
                }
                Ok(())
            }
        }
    }

    /// Work out a condition. It has to be a real bool; see docs/decisions.md.
    fn condition(&mut self, condition: &Expr, env: &Env) -> Result<bool, Control> {
        let value = self.eval(condition, env)?;
        ops::condition(value, condition.span)
    }

    fn assign(&mut self, target: &Expr, value: Value, env: &Env) -> Result<(), Control> {
        let span = target.span;
        match &target.kind {
            ExprKind::Name(name) => match env.assign(name, value) {
                Ok(()) => Ok(()),
                Err(AssignError::NotDeclared) => fatal(
                    format!("`{name}` is not declared. Create it with `let` or `var` first"),
                    span,
                ),
                Err(AssignError::Immutable) => fatal(
                    format!("`{name}` cannot be changed. Declare it with `var` to allow that"),
                    span,
                ),
            },
            ExprKind::Field { object, name, .. } => {
                let object = self.eval(object, env)?;
                // `title.text = "Hello"` is a change to the page.
                match builtins::set_on_page(self, &object, name, &value, span) {
                    Some(done) => done,
                    None => ops::set_field(object, name, value, span),
                }
            }
            ExprKind::Index { object, index } => {
                let object = self.eval(object, env)?;
                let index = self.eval(index, env)?;
                ops::set_item(object, &index, value, span)
            }
            // The parser only lets the three kinds above through.
            _ => fatal("this cannot be assigned to", span),
        }
    }

    /// Does `value` fit `pattern`? Names in the pattern are declared in `scope`.
    fn matches(
        &mut self,
        pattern: &Pattern,
        value: &Value,
        scope: &Env,
        span: Span,
    ) -> Result<bool, Control> {
        match pattern {
            Pattern::Wildcard => Ok(true),
            Pattern::Literal(literal) => {
                let literal = self.eval(literal, scope)?;
                Ok(equal(&literal, value))
            }
            Pattern::Name(name) => {
                // A name that is already a bare enum case, like `Idle`, is
                // compared. Any other name becomes a new variable holding the value.
                if let Some(Value::Case(case)) = scope.get(name) {
                    if case.fields.is_empty() && case.name() == name {
                        return Ok(equal(&Value::Case(case), value));
                    }
                }
                scope.declare(name, value.clone(), false);
                Ok(true)
            }
            Pattern::Variant { name, fields } => {
                let Some(Value::CaseConstructor(def, index)) = scope.get(name) else {
                    return fatal(format!("`{name}` is not an enum case that carries values"), span);
                };
                let Value::Case(case) = value else {
                    return Ok(false);
                };
                if !Rc::ptr_eq(&case.def, &def) || case.index != index {
                    return Ok(false);
                }
                if fields.len() != case.fields.len() {
                    return fatal(
                        format!(
                            "`{name}` carries {}, but this pattern has {}",
                            counted(case.fields.len(), "value"),
                            fields.len()
                        ),
                        span,
                    );
                }
                for (pattern, value) in fields.iter().zip(&case.fields) {
                    if !self.matches(pattern, value, scope, span)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }

    // ----------------------------------------------------------- Expressions

    fn eval(&mut self, expr: &Expr, env: &Env) -> Result<Value, Control> {
        let span = expr.span;

        match &expr.kind {
            ExprKind::Int(n) => match i64::try_from(*n) {
                Ok(n) => Ok(Value::Int(n)),
                Err(_) => fatal("this number is too big for an int", span),
            },
            ExprKind::Float(x) => Ok(Value::Float(*x)),
            ExprKind::Str(text) => Ok(Value::str(text)),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::None => Ok(Value::None),

            ExprKind::FormatStr(parts) => {
                let mut text = String::new();
                for part in parts {
                    match part {
                        FormatPart::Text(plain) => text += plain,
                        FormatPart::Value { value, format } => {
                            let value = self.eval(value, env)?;
                            text += &builtins::format(&value, format.as_deref(), span)?;
                        }
                    }
                }
                Ok(Value::str(text))
            }

            ExprKind::Name(name) => match env.get(name) {
                Some(value) => Ok(value),
                None => fatal(format!("`{name}` is not defined"), span),
            },

            ExprKind::Unit { value, unit } => {
                let value = self.eval(value, env)?;
                ops::quantity(value, unit, span)
            }

            ExprKind::List(items) => {
                let mut values = Vec::new();
                for item in items {
                    values.push(self.eval(item, env)?);
                }
                Ok(Value::list(values))
            }
            // A key written twice keeps its first place and its last value.
            ExprKind::Dict(entries) => {
                let mut dict = Dict::default();
                for (key, value) in entries {
                    let at = key.span;
                    let key = self.eval(key, env)?;
                    let value = self.eval(value, env)?;
                    dict.set(ops::key(&key, at)?, value);
                }
                Ok(Value::dict(dict))
            }
            ExprKind::ListComp { element, variable, iterable, condition } => {
                let source = self.eval(iterable, env)?;
                let mut values = Vec::new();
                for item in ops::items(&source, iterable.span)? {
                    let scope = Scope::child(env);
                    scope.declare(variable, item, false);
                    let keep = match condition {
                        Some(condition) => self.condition(condition, &scope)?,
                        None => true,
                    };
                    if keep {
                        values.push(self.eval(element, &scope)?);
                    }
                }
                Ok(Value::list(values))
            }

            ExprKind::Unary { op, operand } => {
                let value = self.eval(operand, env)?;
                ops::unary(*op, value, span)
            }

            // `and` and `or` only work out their right side when they have to.
            ExprKind::Binary { op: op @ (BinaryOp::And | BinaryOp::Or), left, right } => {
                let left = self.logic_operand(*op, left, env)?;
                let decided = match op {
                    BinaryOp::And => !left, // false and ... is false
                    _ => left,              // true or ... is true
                };
                if decided {
                    return Ok(Value::Bool(left));
                }
                Ok(Value::Bool(self.logic_operand(*op, right, env)?))
            }
            ExprKind::Binary { op, left, right } => {
                let left = self.eval(left, env)?;
                let right = self.eval(right, env)?;
                ops::binary(*op, left, right, span)
            }
            ExprKind::Compare { first, rest } => {
                // `a < b < c`: compare neighbours, stop at the first false,
                // and work out each operand once.
                let mut left = self.eval(first, env)?;
                for (op, operand) in rest {
                    let right = self.eval(operand, env)?;
                    match ops::binary(*op, left, right.clone(), span)? {
                        Value::Bool(true) => left = right,
                        _ => return Ok(Value::Bool(false)),
                    }
                }
                Ok(Value::Bool(true))
            }

            // A function written as a value remembers the scope it was written in.
            ExprKind::Function(decl) => {
                let closure =
                    Closure { decl: decl.clone(), home: Home::Scope(Scope::seen(env, env.count(), &[])) };
                Ok(Value::Function(Rc::new(closure)))
            }

            ExprKind::Call { callee, args } => {
                let callee = self.eval(callee, env)?;
                let mut positional = Vec::new();
                let mut named = Vec::new();
                for arg in args {
                    let value = self.eval(&arg.value, env)?;
                    match &arg.name {
                        Some(name) => named.push((name.clone(), value)),
                        None if named.is_empty() => positional.push(value),
                        None => {
                            return fatal("plain arguments must come before named ones", arg.value.span)
                        }
                    }
                }
                self.call(callee, positional, named, span)
            }

            ExprKind::Field { object, name, .. } => {
                let object = self.eval(object, env)?;
                // `title.text` is asked of the page.
                match builtins::on_page(self, &object, name) {
                    Some(value) => Ok(value),
                    None => ops::field(object, name, span),
                }
            }
            ExprKind::Index { object, index } => {
                let object = self.eval(object, env)?;
                let index = self.eval(index, env)?;
                ops::item(&object, &index, span)
            }
        }
    }

    fn logic_operand(&mut self, op: BinaryOp, operand: &Expr, env: &Env) -> Result<bool, Control> {
        let value = self.eval(operand, env)?;
        ops::logic_operand(op, value, operand.span)
    }


    // ----------------------------------------------------------------- Calls

    fn call(
        &mut self,
        callee: Value,
        mut positional: Vec<Value>,
        named: Vec<(String, Value)>,
        span: Span,
    ) -> Result<Value, Control> {
        match callee {
            Value::Function(closure) => self.call_function(&closure, positional, named, span),
            Value::BoundMethod(instance, closure) => {
                // `p.distance_to(q)` passes `p` as the first argument, `self`.
                positional.insert(0, Value::Instance(instance));
                self.call_function(&closure, positional, named, span)
            }
            Value::Builtin(builtin) => {
                ops::no_named(builtin.name, &named, span)?;
                (builtin.call)(self, positional, span)
            }
            Value::Method(receiver, name) => {
                ops::no_named(&name, &named, span)?;
                builtins::call_method(self, &receiver, &name, positional, span)
            }
            Value::Class(class) => {
                let names: Vec<&str> = class.fields.iter().map(|f| f.name.as_str()).collect();
                let slots = ops::bind(&class.name, &names, positional, named, span)?;
                let scope = scope_of(&class.home);
                let mut fields = Vec::with_capacity(class.fields.len());
                for (field, slot) in class.fields.iter().zip(slots) {
                    let value = match (slot, &field.default) {
                        (Some(value), _) => value,
                        (None, Some(default)) => self.eval(default, &scope)?,
                        (None, None) => {
                            return fatal(
                                format!("`{}` needs a value for `{}`", class.name, field.name),
                                span,
                            )
                        }
                    };
                    fields.push(value);
                }
                Ok(Value::Instance(Rc::new(Instance { class, fields: RefCell::new(fields) })))
            }
            Value::CaseConstructor(def, index) => ops::new_case(&def, index, positional, named, span),
            Value::ErrorType(kind) => ops::new_error(&kind, positional, named, span),
            other => fatal(format!("{} cannot be called", other.type_name()), span),
        }
    }

    fn call_function(
        &mut self,
        closure: &Closure,
        positional: Vec<Value>,
        named: Vec<(String, Value)>,
        span: Span,
    ) -> Result<Value, Control> {
        let decl = &closure.decl;
        let name = if decl.name.is_empty() { "function" } else { &decl.name };

        if self.depth >= MAX_DEPTH {
            return fatal(
                format!("more than {MAX_DEPTH} calls are nested. Is `{name}` calling itself forever?"),
                span,
            );
        }

        // The body runs in a new scope inside the scope where the function
        // was *defined*, not the one it was called from.
        let scope = Scope::child(&scope_of(&closure.home));
        let names: Vec<&str> = decl.params.iter().map(|p| p.name.as_str()).collect();
        let slots = ops::bind(name, &names, positional, named, span)?;
        for (param, slot) in decl.params.iter().zip(slots) {
            let value = match (slot, &param.default) {
                (Some(value), _) => value,
                // Defaults are worked out fresh on every call.
                (None, Some(default)) => self.eval(default, &scope)?,
                (None, None) => {
                    return fatal(format!("`{name}` needs a value for `{}`", param.name), span)
                }
            };
            self.declare(&scope, &param.name, value, false, span)?;
        }

        self.tick(span)?;
        self.depth += 1;
        let result = self.run_statements(&decl.body, &scope);
        self.depth -= 1;

        match result {
            Ok(()) => Ok(Value::None), // fell off the end without a `return`
            Err(Control::Return(value, _)) => Ok(value),
            Err(Control::Break(span)) => fatal("`break` outside a loop", span),
            Err(Control::Continue(span)) => fatal("`continue` outside a loop", span),
            Err(other) => Err(other),
        }
    }

}


#[cfg(test)]
mod tests {
    use crate::{lexer, parser};

    /// Run a program and return what it printed and how it ended.
    fn run(source: &str) -> (String, Result<(), String>) {
        let program = parser::parse(lexer::lex(source).expect("lexing failed")).expect("parsing failed");
        // Both engines run it, and must agree.
        let (output, result) = crate::vm::run_both(&program, source);
        (output, result.map_err(|error| error.message))
    }

    /// What a working program prints. A program that runs correctly must
    /// also get through the type checker, so that is tested here too.
    fn output(source: &str) -> String {
        let program = parser::parse(lexer::lex(source).unwrap()).unwrap();
        let complaints = crate::checker::check(&program, false);
        assert!(complaints.is_empty(), "the checker rejected a working program: {complaints:?}\n{source}");

        let (output, result) = run(source);
        if let Err(message) = result {
            panic!("program failed: {message}\noutput so far:\n{output}");
        }
        output
    }

    /// The message a failing program stops with.
    fn error(source: &str) -> String {
        run(source).1.expect_err("the program should have failed")
    }

    // ----- Values and operators -----

    #[test]
    fn arithmetic() {
        assert_eq!(output("print(1 + 2 * 3, 10 - 4, 2 ** 10, -3 + 1)"), "7 6 1024 -2\n");
        assert_eq!(output("print(1.5 + 1, 2 * 0.5, 2.0 ** -1, 2.0 ** 3)"), "2.5 1.0 0.5 8.0\n");
        assert_eq!(
            error("print(2 ** -1)"),
            "uncaught ValueError: a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`"
        );
    }

    #[test]
    fn division_always_gives_a_float_and_floor_division_rounds_down() {
        assert_eq!(output("print(7 / 2, 6 / 2, 7 // 2, -7 // 2, 7.5 // 2)"), "3.5 3.0 3 -4 3.0\n");
        assert_eq!(output("print(7 % 3, -7 % 3, 7 % -3, 7.5 % 2)"), "1 2 -2 1.5\n");
    }

    #[test]
    fn comparisons_and_equality() {
        assert_eq!(output("print(1 < 2, 2 <= 2, 3 > 4, \"a\" < \"b\", 1 == 1.0, 1 != 2)"), "true true false true true true\n");
        assert_eq!(output("print([1, 2] == [1, 2], [1] == [2])"), "true false\n");
        // The checker refuses to compare an int with a str, but a value of
        // unknown type can still meet one when the program runs.
        assert_eq!(output("function same(a, b) { return a == b }\nprint(same(1, \"1\"), same(1, 1.0))"), "false true\n");
    }

    #[test]
    fn chained_comparisons() {
        assert_eq!(output("print(1 < 2 < 3, 1 < 5 < 3, 3 > 2 > 1 == 1)"), "true false true\n");
        // The middle operand runs once, and the chain stops at the first false.
        let source = "function mid() -> int {\n print(\"mid\")\n return 5\n}\nfunction last() -> int {\n print(\"last\")\n return 9\n}\nprint(1 < mid() < last())\nprint(9 < mid() < last())";
        assert_eq!(output(source), "mid\nlast\ntrue\nmid\nfalse\n");
    }

    #[test]
    fn logic_short_circuits() {
        let source = "function loud(x: bool) -> bool {\n print(\"called\")\n return x\n}\nprint(false and loud(true))\nprint(true or loud(true))\nprint(true and loud(false))\nprint(not true)";
        assert_eq!(output(source), "false\ntrue\ncalled\nfalse\nfalse\n");
    }

    #[test]
    fn strings() {
        assert_eq!(output("print(\"ab\" + \"cd\", len(\"héllo\"), \"hello\"[1], \"ell\" in \"hello\")"), "abcd 5 e true\n");
        assert_eq!(output("print(\" Hi \".strip().upper(), \"a,b\".split(\",\"), \"x\".replace(\"x\", \"y\"))"), "HI [\"a\", \"b\"] y\n");
        assert_eq!(output("print(\"zeno\".starts_with(\"ze\"), \"zeno\".ends_with(\"x\"))"), "true false\n");
    }

    #[test]
    fn text_and_list_methods_of_the_library() {
        // Positions count characters, not bytes.
        assert_eq!(
            output("let t = \"héllo wörld\"\nprint(t.find(\"w\"), t.find(\"z\"), t.slice(6), t.slice(1, 3), t.slice(11) == \"\")"),
            "6 none wörld él true\n"
        );
        assert_eq!(output("print(\"A\".code(), \"é\".code(), char(97), char(233), char(128512))"), "65 233 a é 😀\n");
        assert_eq!(
            output("let xs = [3, 1, 2]\nxs.insert(1, 9)\nprint(xs)\nprint(xs.pop(0), xs)\nxs.sort()\nprint(xs)\nxs.reverse()\nprint(xs, xs.find(2), xs.find(7), xs.slice(1), xs.slice(0, 1))\nxs.clear()\nprint(xs)"),
            "[3, 9, 1, 2]\n3 [9, 1, 2]\n[1, 2, 9]\n[9, 2, 1] 1 none [2, 1] [9]\n[]\n"
        );
        assert_eq!(output("print([\"b\", \"a\"].join(\", \"), [].join(\"-\") == \"\", [\"x\"].join(\"-\"))"), "b, a true x\n");
        // Ints and floats sort together, as `<` compares them.
        // With a function: by what it gives, and the same stay as they were.
        assert_eq!(
            output("let words = [\"pear\", \"fig\", \"kiwi\", \"apple\", \"plum\"]\nwords.sort(len)\nprint(words)\nlet xs = [3, -7, 1]\nxs.sort(abs)\nprint(xs)"),
            "[\"fig\", \"pear\", \"kiwi\", \"plum\", \"apple\"]\n[1, 3, -7]\n"
        );
        assert_eq!(
            error("let xs = [2, 1]\nxs.sort(function(n) { return [n] })"),
            "the function given to `sort` must return a number or text to order by, but it returned a list"
        );
        // A function given to `map` may change the list it is walking.
        assert_eq!(
            output("var xs = [1, 2]\nprint(xs.map(function(n: int) -> int {\n xs.append(n)\n return n * 2\n}), xs)"),
            "[2, 4] [1, 2, 1, 2]\n"
        );
        assert_eq!(output("function order(xs) {\n xs.sort()\n return xs\n}\nprint(order([2, 1.5, -3]), order([\"b\", \"B\", \"a\"]))"), "[-3, 1.5, 2] [\"B\", \"a\", \"b\"]\n");
        assert_eq!(error("print(\"abc\".slice(2, 9))"), "uncaught IndexError: slice 2 to 9 is out of range for a str of length 3");
        assert_eq!(error("print(\"abc\".slice(2, 1))"), "uncaught IndexError: slice 2 to 1 is out of range for a str of length 3");
        assert_eq!(error("print([1].slice(-1))"), "uncaught IndexError: slice -1 to 1 is out of range for a list of length 1");
        assert_eq!(error("let xs = [1]\nxs.insert(2, 0)"), "uncaught IndexError: cannot insert at 2 in a list of length 1");
        assert_eq!(error("print([1].pop(1))"), "uncaught IndexError: index 1 is out of range for a list of length 1");
        assert_eq!(error("print(\"ab\".code())"), "uncaught ValueError: `code` needs a str of exactly one character, and this one has 2");
        assert_eq!(error("print(char(0))"), "uncaught ValueError: 0 is not the code of a character");
        assert_eq!(error("print(char(55296))"), "uncaught ValueError: 55296 is not the code of a character");
    }

    // ----- Reading and writing -----

    /// A place for a test's file that no other test uses.
    fn scratch(name: &str) -> String {
        crate::modules::shown(&std::env::temp_dir().join(format!("zeno-test-{}-{name}", std::process::id())))
    }

    #[test]
    fn files_are_written_read_and_removed() {
        let path = scratch("files.txt");
        let source = format!(
            "import files\nlet path = \"{path}\"\n\
             print(files.exists(path))\n\
             files.write(path, \"one\\ntwo\\n\")\n\
             print(files.exists(path), len(files.read(path)))\n\
             files.append(path, \"three\")\n\
             print(files.read(path).lines())\n\
             files.write(path, \"again\")\n\
             print(files.read(path))\n\
             files.remove(path)\n\
             print(files.exists(path))\n"
        );
        assert_eq!(output(&source), "false\ntrue 8\n[\"one\", \"two\", \"three\"]\nagain\nfalse\n");
        // `from files import ...` gives the same functions.
        let source = format!(
            "from files import write, read, remove\nwrite(\"{path}\", \"x\")\nprint(read(\"{path}\"))\nremove(\"{path}\")"
        );
        assert_eq!(output(&source), "x\n");
    }

    #[test]
    fn a_file_that_cannot_be_used_is_a_file_error() {
        let path = scratch("missing.txt");
        let folder = crate::modules::shown(&std::env::temp_dir());
        assert_eq!(
            error(&format!("import files\nprint(files.read(\"{path}\"))")),
            format!("uncaught FileError: there is no file `{path}`")
        );
        assert_eq!(
            error(&format!("import files\nfiles.remove(\"{path}\")")),
            format!("uncaught FileError: there is no file `{path}`")
        );
        assert_eq!(
            error(&format!("import files\nfiles.write(\"{path}/x/y.txt\", \"\")")),
            format!("uncaught FileError: cannot write `{path}/x/y.txt`")
        );
        // A folder is not a file: it is there, and cannot be read.
        assert_eq!(
            output(&format!(
                "import files\nprint(files.exists(\"{folder}\"))\n\
                 try {{ print(files.read(\"{folder}\")) }} except FileError as error {{ print(error.message) }}"
            )),
            format!("false\ncannot read `{folder}`\n")
        );
        // A file that is not text: bytes that are not UTF-8, or a zero.
        for (name, bytes) in [("not-utf8.bin", &[b'a', 0xFF][..]), ("zero.bin", &[b'a', 0, b'b'][..])] {
            let path = scratch(name);
            std::fs::write(&path, bytes).expect("the file is written");
            let source = format!(
                "import files\ntry {{ print(files.read(\"{path}\")) }} except FileError as error {{ print(error.message) }}"
            );
            assert_eq!(output(&source), format!("`{path}` does not hold text\n"));
            std::fs::remove_file(&path).expect("the file is removed");
        }
        assert_eq!(error("import files\nfiles.read(5)"), "`read` needs a str, but this is an int");
    }

    #[test]
    fn the_lines_of_a_text() {
        assert_eq!(
            output(
                "print(\"\".lines(), \"one\".lines(), \"one\\n\".lines(), \"one\\ntwo\".lines())\n\
                 print(\"one\\r\\ntwo\\r\\n\".lines(), \"\\n\".lines(), \"a\\n\\nb\".lines())\n\
                 print(len(\"a\\r\".lines()[0]), len(\"a\\rb\\n\".lines()[0]))"
            ),
            "[] [\"one\"] [\"one\"] [\"one\", \"two\"]\n[\"one\", \"two\"] [\"\"] [\"a\", \"\", \"b\"]\n2 3\n"
        );
    }

    #[test]
    fn input_reads_a_line_or_none() {
        use crate::value::testing::with_outside;
        let source = "let name = input(\"Name: \")\n\
                      if name is none { print(\"nobody\") } else { print(f\"Hello, {name}!\") }\n\
                      var count = 0\n\
                      while true {\n\
                      \x20   let line = input()\n\
                      \x20   if line is none { break }\n\
                      \x20   count += 1\n\
                      \x20   print(count, len(line), line)\n\
                      }\n\
                      print(input() is none)";
        with_outside("Partha\none\r\n\nlast, with no end", &[], || {
            assert_eq!(output(source), "Name: Hello, Partha!\n1 3 one\n2 0 \n3 17 last, with no end\ntrue\n");
        });
        // Nothing typed at all: the first `input` is `none` already.
        assert_eq!(output(source), "Name: nobody\ntrue\n");
        assert_eq!(error("input(5)"), "`input` needs a str, but this is an int");
    }

    #[test]
    fn a_dict_keeps_one_value_under_each_key() {
        // Written out, read, stored into. A key that is there keeps its
        // place; a new one goes at the end.
        assert_eq!(
            output("let ages = [\"Ada\": 36, \"Alan\": 41]\nages[\"Grace\"] = 45\nages[\"Ada\"] = 37\nprint(ages, len(ages), ages[\"Alan\"])"),
            "[\"Ada\": 37, \"Alan\": 41, \"Grace\": 45] 3 41\n"
        );
        assert_eq!(output("let empty: dict[int, str] = [:]\nprint(empty, len(empty))\nempty[2] = \"two\"\nprint(empty)"), "[:] 0\n[2: \"two\"]\n");
        // `in` asks about keys, and a loop is given them in order.
        assert_eq!(
            output("let d = [\"b\": 1, \"a\": 2]\nprint(\"a\" in d, \"c\" in d, \"c\" not in d)\nfor k in d {\n print(k, d[k])\n}"),
            "true false true\nb 1\na 2\n"
        );
        // The loop walks the keys as they were when it began.
        assert_eq!(output("let d = [1: 1]\nfor k in d {\n d[k + 1] = 0\n d[k + 2] = 0\n}\nprint(d)"), "[1: 1, 2: 0, 3: 0]\n");
        // With two names it is given each key and what is under it, also
        // as they were when it began.
        assert_eq!(
            output("let d = [\"b\": 1, \"a\": 2]\nfor k, v in d {\n d[k] = v * 10\n d[k + k] = 0\n print(k, v)\n}\nprint(d)"),
            "b 1\na 2\n[\"b\": 10, \"a\": 20, \"bb\": 0, \"aa\": 0]\n"
        );
        // Over a list, text and a range the first name is the position,
        // from 0, and the second is what is there.
        assert_eq!(
            output(
                "for i, word in [\"zero\", \"one\"] {\n print(i, word)\n}\nfor i, c in \"h\u{e9}y\" {\n print(i, c)\n}\n\
                 for i, n in range(10, 0, -4) {\n if i == 2 {\n  break\n }\n print(i, n)\n}"
            ),
            "0 zero\n1 one\n0 h\n1 \u{e9}\n2 y\n0 10\n1 6\n"
        );
        // A value is anything a list can hold, and it is the thing
        // itself that is under the key, not a copy.
        assert_eq!(
            output(
                "class Point {\n x: int\n}\nlet lists = [\"a\": [1]]\nlists[\"a\"].append(2)\nlists[\"b\"] = []\n\
                 let points = [1: Point(5)]\npoints[1].x = 6\nlet nested = [\"in\": [\"x\": 1]]\nnested[\"in\"][\"y\"] = 2\n\
                 print(lists, points, nested, lists.values(), points.get(2))\nprint(points.pop(1), points)"
            ),
            "[\"a\": [1, 2], \"b\": []] [1: Point(x=6)] [\"in\": [\"x\": 1, \"y\": 2]] [[1, 2], []] none\nPoint(x=6) [:]\n"
        );
        // `get`: `none`, or what the second argument says, for a key
        // that is not there.
        assert_eq!(
            output("let d = [\"a\": 1.5]\nprint(d.get(\"a\"), d.get(\"z\"), d.get(\"z\", 0.0), d.get(\"a\", 0.0))"),
            "1.5 none 0.0 1.5\n"
        );
        assert_eq!(
            output("let d = [1: true, 2: false, 3: true]\nprint(d.pop(2), d, d.keys(), d.values())\nd.clear()\nprint(d, len(d))"),
            "false [1: true, 3: true] [1, 3] [true, true]\n[:] 0\n"
        );
        // Equal when the same keys have the same values, in any order.
        assert_eq!(
            output("let a = [\"x\": 1, \"y\": 2]\nlet b = [\"y\": 2, \"x\": 1]\nprint(a == b, a != b, a == [\"x\": 1], a == [\"x\": 1, \"y\": 3])"),
            "true false false false\n"
        );
        // A dict is shared, like a list: two names for one dict.
        assert_eq!(output("let a = [\"x\": 1]\nlet b = a\nb[\"y\"] = 2\nprint(a)"), "[\"x\": 1, \"y\": 2]\n");
        assert_eq!(output("print(str([\"k\": \"v\"]), f\"{[1: 2]}\")"), "[\"k\": \"v\"] [1: 2]\n");
        // A key that is not there is a KeyError, which can be caught.
        assert_eq!(error("let d = [\"a\": 1]\nprint(d[\"b\"])"), "uncaught KeyError: this dict has no key \"b\"");
        assert_eq!(error("let d = [1: 2]\nd.pop(5)"), "uncaught KeyError: this dict has no key 5");
        assert_eq!(
            output("let d = [1: 2]\ntry {\n print(d[3])\n} except KeyError as e {\n print(e.kind, e.message)\n}"),
            "KeyError this dict has no key 3\n"
        );
    }

    #[test]
    fn more_of_math() {
        // Angles in, as a plain number (radians) or with a unit.
        assert_eq!(
            output("import math\nprint(math.sin(90 deg), math.cos(180 deg), math.sin(0), math.tan(0 rad), math.cos(0.0 rad))"),
            "1.0 -1.0 0.0 0.0 1.0\n"
        );
        // Angles out, in radians, which `.deg` changes.
        assert_eq!(
            output(
                "import math\nfrom math import atan2\n\
                 print(math.asin(1).deg, math.acos(1), math.atan(1).deg, math.acos(-1) / 1 rad == math.pi)\n\
                 print(atan2(1, 1).deg, atan2(1, -1).deg, atan2(-1, -1).deg, atan2(-1, 1).deg, atan2(0, -1).deg)\n\
                 print((180 deg).rad / 1 rad == math.pi, (math.pi * 1 rad).deg, (90 deg).rad == math.asin(1))"
            ),
            "90 deg 0 rad 45 deg true\n45 deg 135 deg -135 deg -45 deg 180 deg\ntrue 180 deg true\n"
        );
        assert_eq!(
            output(
                "import math\nprint(math.exp(0), math.log(1), math.log(math.e), math.log2(8), math.log2(0.5), math.e > 2.718)\n\
                 print(math.round(2.5), math.round(-2.5), math.round(2.4), math.round(-2.6), math.round(0.5), math.round(7))"
            ),
            "1.0 0.0 1.0 3.0 -1.0 true\n3 -3 2 -3 1 7\n"
        );
        assert_eq!(error("import math\nprint(math.log(0))"), "uncaught ValueError: cannot take the logarithm of 0.0");
        assert_eq!(error("import math\nprint(math.log2(-8))"), "uncaught ValueError: cannot take the logarithm of -8.0");
        assert_eq!(
            error("import math\nprint(math.asin(1.5))"),
            "uncaught ValueError: `asin` needs a number from -1 to 1, and this is 1.5"
        );
        assert_eq!(
            error("import math\nprint(math.acos(-2))"),
            "uncaught ValueError: `acos` needs a number from -1 to 1, and this is -2.0"
        );
        // What the checker would have caught, met while running.
        assert_eq!(
            error("import math\nprint(math.sin(3 s))"),
            "`sin` needs a number or an angle, such as `30 deg`, not a quantity in s"
        );
        assert_eq!(error("import math\nprint(math.atan2(1, \"x\"))"), "`atan2` needs two numbers, but this is a str");
        assert_eq!(
            error("import math\nprint(math.atan(1) + 90 deg)"),
            "`+` does not work between a quantity in rad and a quantity in deg"
        );
    }

    #[test]
    fn the_arguments_of_the_program() {
        use crate::value::testing::with_outside;
        let source = "import program\nlet words = program.arguments()\nprint(len(words), words)\n\
                      words.append(\"mine\")\nprint(len(program.arguments()))";
        with_outside("", &["a.txt", "two words", "--strict"], || {
            assert_eq!(output(source), "3 [\"a.txt\", \"two words\", \"--strict\"]\n3\n");
        });
        assert_eq!(output(source), "0 []\n0\n");
    }

    #[test]
    fn format_strings() {
        assert_eq!(output("let n = 3\nprint(f\"{n} + {n} = {n + n}\")"), "3 + 3 = 6\n");
        assert_eq!(output("print(f\"{3.14159:.2f}|{42:5}|{'ab':5}|{'ab':>5}|{'a:}':^6}|\")"), "3.14|   42|ab   |   ab| a:}  |\n");
        assert_eq!(output("print(f\"{255:x} {255:X} {5:08b} {7:03} {-7:04} {2:.1f}\")"), "ff FF 00000101 007 -007 2.0\n");
        assert_eq!(output("print(f\"{{literal}} {1}\")"), "{literal} 1\n");
    }

    #[test]
    fn lists() {
        assert_eq!(output("let xs = [1, 2]\nxs.append(3)\nxs[0] = 9\nprint(xs, len(xs), xs.pop(), xs + [7], 2 in xs, 5 not in xs)"), "[9, 2] 3 3 [9, 2, 7] true true\n");
        assert_eq!(output("let a = [1]\nlet b = a\nb.append(2)\nprint(a)"), "[1, 2]\n");
        assert_eq!(output("print([n * n for n in range(6) if n % 2 == 0], range(2, 5), range(5, 0, -2))"), "[0, 4, 16] [2, 3, 4] [5, 3, 1]\n");
    }

    #[test]
    fn quantities() {
        assert_eq!(output("print(500 ms, 500 ms + 250 ms, 2 * 500 ms, 1.5 V / 3, 1 s / 250 s, 3 V > 2 V)"), "500 ms 750 ms 1000 ms 0.5 V 0.004 true\n");
        assert_eq!(error("500 ms + 3 V"), "`+` does not work between a quantity in ms and a quantity in V");
        assert_eq!(error("500 ms + 3"), "`+` does not work between a quantity in ms and an int");
        assert_eq!(
            error("print(500 mss)"),
            "`mss` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC"
        );
    }

    #[test]
    fn built_in_functions() {
        assert_eq!(output("print(int(\"42\"), int(3.9), float(\"1.5\"), str(12) + \"!\", abs(-3), min(3, 1, 2), max([1, 5.5]), sum([1, 2, 3]), sum([1, 0.5]))"), "42 3 1.5 12! 3 1 5.5 6 1.5\n");
    }

    #[test]
    fn modules() {
        assert_eq!(output("import math\nfrom math import sqrt, floor\nprint(sqrt(16), floor(2.7), math.ceil(2.1), math.pi > 3.14)"), "4.0 2 3 true\n");
        assert_eq!(
            error("import radio"),
            "there is no module `radio`. So far Zeno has `math`, `time`, `board`, `files`, `program`, `json` and `web`"
        );
        assert_eq!(error("from math import nope"), "module `math` has no `nope`");
        assert_eq!(error("from time import sleep\nsleep(500)"), "`sleep` needs a time with a unit, such as `500 ms`, but this is an int");
    }

    // ----- Variables and scopes -----

    #[test]
    fn let_cannot_change_and_var_can() {
        assert_eq!(output("var x = 1\nx = 2\nx += 3\nprint(x)"), "5\n");
        assert_eq!(error("let x = 1\nx = 2"), "`x` cannot be changed. Declare it with `var` to allow that");
        assert_eq!(error("const N = 1\nN += 1"), "`N` cannot be changed. Declare it with `var` to allow that");
        assert_eq!(error("x = 2"), "`x` is not declared. Create it with `let` or `var` first");
        assert_eq!(error("print(y)"), "`y` is not defined");
    }

    #[test]
    fn blocks_have_their_own_scope() {
        assert_eq!(output("let x = 1\nif true {\n let x = 2\n print(x)\n}\nprint(x)"), "2\n1\n");
        assert_eq!(error("if true {\n let inner = 1\n}\nprint(inner)"), "`inner` is not defined");
        assert_eq!(error("let x = 1\nlet x = 2"), "`x` is already declared in this block");
        assert_eq!(output("var total = 0\nfor i in range(3) {\n let double = i * 2\n total += double\n}\nprint(total)"), "6\n");
    }

    // ----- Control flow -----

    #[test]
    fn if_else() {
        let source = "function size(n: int) -> str {\n if n < 10 { return \"small\" } else if n < 100 { return \"medium\" } else { return \"large\" }\n}\nprint(size(5), size(50), size(500))";
        assert_eq!(output(source), "small medium large\n");
        assert_eq!(error("if 1 { }"), "a condition must be true or false, but this is an int");
        assert_eq!(error("print(1 and true)"), "`and` needs bools, but this is an int");
    }

    #[test]
    fn loops_with_break_and_continue() {
        let source = "var i = 0\nwhile true {\n i += 1\n if i == 2 { continue }\n if i > 4 { break }\n print(i)\n}";
        assert_eq!(output(source), "1\n3\n4\n");
        assert_eq!(output("for c in \"ab\" {\n print(c)\n}\nfor x in [1, 2, 3] {\n if x == 2 { break }\n print(x)\n}"), "a\nb\n1\n");
        assert_eq!(error("break"), "`break` outside a loop");
        assert_eq!(error("function f() { continue }\nf()"), "`continue` outside a loop");
        assert_eq!(error("for x in 5 { }"), "cannot loop over an int");
    }

    // ----- Functions -----

    #[test]
    fn functions_and_arguments() {
        let source = "function greet(who: str, excited: bool = false) -> str {\n if excited { return f\"Hello, {who}!\" }\n return f\"Hello, {who}.\"\n}\nprint(greet(\"a\"), greet(\"b\", true), greet(who=\"c\", excited=true), greet(\"d\", excited=true))";
        assert_eq!(output(source), "Hello, a. Hello, b! Hello, c! Hello, d!\n");
        assert_eq!(output("function nothing() { }\nprint(nothing())"), "none\n");
    }

    #[test]
    fn argument_mistakes() {
        let f = "function f(a: int, b: int = 2) -> int { return a + b }\n";
        assert_eq!(error(&format!("{f}f()")), "`f` needs a value for `a`");
        assert_eq!(error(&format!("{f}f(1, 2, 3)")), "`f` takes 2 arguments, but 3 were given");
        assert_eq!(error(&format!("{f}f(1, c=3)")), "`f` has no argument named `c`");
        assert_eq!(error(&format!("{f}f(1, a=3)")), "`f` was given `a` twice");
        assert_eq!(error(&format!("{f}f(a=1, 2)")), "plain arguments must come before named ones");
        assert_eq!(error("let x = 5\nx()"), "an int cannot be called");
        assert_eq!(error("print(len(1, 2))"), "`len` takes 1 argument, but 2 were given");
    }

    #[test]
    fn functions_can_be_used_before_they_are_written() {
        assert_eq!(output("print(double(4))\nfunction double(n: int) -> int { return n * 2 }"), "8\n");
        let source = "function even(n: int) -> bool {\n if n == 0 { return true }\n return odd(n - 1)\n}\nfunction odd(n: int) -> bool {\n if n == 0 { return false }\n return even(n - 1)\n}\nprint(even(10), odd(7))";
        assert_eq!(output(source), "true true\n");
    }

    #[test]
    fn recursion_and_its_limit() {
        assert_eq!(output("function fact(n: int) -> int {\n if n <= 1 { return 1 }\n return n * fact(n - 1)\n}\nprint(fact(20))"), "2432902008176640000\n");
        assert_eq!(output("function down(n: int) -> int {\n if n == 0 { return 0 }\n return down(n - 1)\n}\nprint(down(900))"), "0\n");
        assert_eq!(error("function forever() { forever() }\nforever()"), "more than 1000 calls are nested. Is `forever` calling itself forever?");
    }

    #[test]
    fn closures_remember_where_they_were_defined() {
        let source = "function counter() {\n var count = 0\n function next() -> int {\n  count += 1\n  return count\n }\n return next\n}\nlet a = counter()\nlet b = counter()\nprint(a(), a(), b())";
        assert_eq!(output(source), "1 2 1\n");
    }

    #[test]
    fn defaults_are_fresh_on_every_call() {
        let source = "function add(item: int, to: list[int] = []) -> list[int] {\n to.append(item)\n return to\n}\nprint(add(1), add(2))";
        assert_eq!(output(source), "[1] [2]\n");
    }

    #[test]
    fn parameters_cannot_be_reassigned() {
        assert_eq!(error("function f(n: int) { n = 2 }\nf(1)"), "`n` cannot be changed. Declare it with `var` to allow that");
    }

    #[test]
    fn functions_are_values() {
        let source = "function twice(f, x) { return f(f(x)) }\nfunction add_one(n) { return n + 1 }\nlet double = function(n) { return n * 2 }\nprint(twice(add_one, 5), twice(double, 5), function(a, b) { return a + b }(1, 2))";
        assert_eq!(output(source), "7 20 3\n");
        assert_eq!(output("function f() { }\nprint(f, function() { }, len)"), "<function f> <function> <function len>\n");
    }

    #[test]
    fn map_filter_reduce() {
        let source = "let xs = [1, 2, 3, 4, 5, 6]\nfunction even(n) { return n % 2 == 0 }\nprint(xs.filter(even).map(function(n) { return n * n }).reduce(function(total, n) { return total + n }, 0))\nprint(xs.map(str), [].map(even), [].reduce(function(a, b) { return a }, 7))";
        assert_eq!(output(source), "56\n[\"1\", \"2\", \"3\", \"4\", \"5\", \"6\"] [] 7\n");
        assert_eq!(
            error("[1].filter(function(n) { return n })"),
            "the function given to `filter` must return true or false, but it returned an int"
        );
        assert_eq!(error("[1].map(5)"), "an int cannot be called");
        assert_eq!(error("[1].map(function() { return 1 })"), "`function` takes 0 arguments, but 1 was given");
    }

    #[test]
    fn a_function_written_inside_a_call_can_span_lines() {
        let source = "let total = [1, 2, 3].reduce(function(sum, n) {\n    let bigger = n * 10\n    return sum + bigger\n}, 0)\nprint(total)";
        assert_eq!(output(source), "60\n");
    }

    #[test]
    fn types_are_optional() {
        let typed = "class Point {\n x: float\n y: float\n}\nfunction length(p: Point) -> float {\n let squared: float = p.x ** 2 + p.y ** 2\n return squared ** 0.5\n}\nprint(length(Point(3.0, 4.0)))";
        let untyped = "class Point {\n x\n y\n}\nfunction length(p) {\n let squared = p.x ** 2 + p.y ** 2\n return squared ** 0.5\n}\nprint(length(Point(3.0, 4.0)))";
        assert_eq!(output(typed), "5.0\n");
        assert_eq!(output(untyped), "5.0\n");
    }

    // ----- Classes -----

    #[test]
    fn classes() {
        let source = "class Point {\n x: float\n y: float = 0.0\n function shifted(self, dx: float) -> Point {\n  return Point(self.x + dx, self.y)\n }\n}\nlet p = Point(1.0)\nlet q = Point(y=5.0, x=2.0)\np.x = 3.0\nprint(p, q, p.shifted(1.0).x, p == Point(3.0, 0.0), p == q)";
        assert_eq!(output(source), "Point(x=3.0, y=0.0) Point(x=2.0, y=5.0) 4.0 true false\n");
    }

    #[test]
    fn class_mistakes() {
        let point = "class Point {\n x: int\n}\n";
        assert_eq!(error(&format!("{point}Point()")), "`Point` needs a value for `x`");
        assert_eq!(error(&format!("{point}Point(1).z")), "`Point` has no field or method `z`");
        assert_eq!(error(&format!("{point}let p = Point(1)\np.z = 2")), "`Point` has no field `z`");
        assert_eq!(error("let s = \"x\"\ns.nope()"), "a str has no field or method `nope`");
    }

    #[test]
    fn objects_are_shared_not_copied() {
        assert_eq!(output("class Box {\n n: int\n}\nlet a = Box(1)\nlet b = a\nb.n = 2\nprint(a.n)"), "2\n");
    }

    // ----- Enums and match -----

    #[test]
    fn enums_and_match() {
        let source = "enum Shape {\n Dot\n Circle(radius: float)\n Rect(w: float, h: float)\n}\nfunction area(s: Shape) -> float {\n match s {\n  case Circle(r) { return 3.0 * r * r }\n  case Rect(w, h) { return w * h }\n  case Dot { return 0.0 }\n }\n}\nprint(area(Circle(2.0)), area(Rect(w=2.0, h=3.0)), area(Dot), area(Shape.Dot))\nprint(Circle(1.5), Dot, Circle(1.0) == Circle(1.0), Dot == Circle(1.0))";
        assert_eq!(output(source), "12.0 6.0 0.0 0.0\nCircle(1.5) Dot true false\n");
    }

    #[test]
    fn match_on_plain_values() {
        let source = "function name(n: int) -> str {\n match n {\n  case 0 { return \"zero\" }\n  case 1 { return \"one\" }\n  case other { return f\"many ({other})\" }\n }\n}\nprint(name(0), name(1), name(7))\nmatch \"x\" {\n case \"y\" { print(\"no\") }\n case _ { print(\"fallback\") }\n}";
        assert_eq!(output(source), "zero one many (7)\nfallback\n");
    }

    #[test]
    fn match_mistakes() {
        assert_eq!(error("match 3 {\n case 1 { }\n}"), "no `case` matches 3");
        assert_eq!(error("enum E {\n A(x: int)\n}\nmatch A(1) {\n case A(x, y) { }\n}"), "`A` carries 1 value, but this pattern has 2");
        assert_eq!(error("match 1 {\n case Nope(x) { }\n}"), "`Nope` is not an enum case that carries values");
    }

    // ----- Errors -----

    #[test]
    fn raise_and_except() {
        let source = "function parse_port(text: str) -> int raises ValueError {\n let port = int(text)\n if not 1 <= port <= 65535 { raise ValueError(\"port out of range\") }\n return port\n}\nfor text in [\"8080\", \"0\", \"abc\"] {\n try {\n  print(parse_port(text))\n } except ValueError as err {\n  print(f\"error: {err}\")\n }\n}";
        assert_eq!(output(source), "8080\nerror: port out of range\nerror: cannot turn \"abc\" into an int\n");
    }

    #[test]
    fn except_picks_the_matching_handler() {
        let source = "function risky(n: int) {\n if n == 0 { raise ValueError(\"v\") }\n if n == 1 { print(1 // 0) }\n if n == 2 { print([1][5]) }\n}\nfor n in range(3) {\n try {\n  risky(n)\n } except ValueError {\n  print(\"value\")\n } except ZeroDivisionError as e {\n  print(\"zero:\", e.message)\n } except Error as e {\n  print(e.kind)\n }\n}";
        assert_eq!(output(source), "value\nzero: division by zero\nIndexError\n");
    }

    #[test]
    fn a_class_can_be_raised() {
        let source = "class Timeout {\n after: int\n}\ntry {\n raise Timeout(30)\n} except Timeout as t {\n print(t.after)\n}";
        assert_eq!(output(source), "30\n");
    }

    #[test]
    fn a_raised_object_is_an_error_to_those_who_ask_for_any_error() {
        // `except Timeout as t` gets the object. `except Error as e` gets
        // an error named after the class, and raising that again passes
        // the object on.
        let source = "class Timeout {\n after: int\n}\n\
                      try {\n try {\n  raise Timeout(30)\n } except Error as e {\n  print(e.kind, e.message, e)\n  raise e\n }\n\
                      } except Timeout as t {\n print(t.after)\n}";
        assert_eq!(output(source), "Timeout Timeout(after=30) Timeout(after=30)\n30\n");
    }

    #[test]
    fn text_methods_know_only_plain_letters_and_blanks() {
        // The compiled program has to do the same on a microcontroller,
        // with no table of every alphabet to look letters up in.
        assert_eq!(output("print(\"Stra\u{df}e \u{e9}\".upper(), \"\u{c9}A\".lower())"), "STRA\u{df}E \u{e9} \u{c9}a\n");
        assert_eq!(output("print(\"\\t a b \\n\".strip(), \" a  b\\tc \".split(), int(\" 1_000\\n\"))"), "a b [\"a\", \"b\", \"c\"] 1000\n");
    }

    #[test]
    fn a_function_value_keeps_the_variables_it_was_written_with() {
        // The `y` declared after `f` must not get into `f`, which was
        // written when `y` meant the one at the top.
        let source = "let y = 1\nfunction g() -> int {\n let f = function() { return y }\n let y = 2\n return f() + y\n}\nprint(g())";
        assert_eq!(output(source), "3\n");
        // But it does see changes to the variables it kept.
        let source = "function g() -> int {\n var n = 1\n let f = function() { return n }\n n = 5\n return f()\n}\nprint(g())";
        assert_eq!(output(source), "5\n");
    }

    #[test]
    fn a_named_function_sees_what_is_declared_above_it_and_no_more() {
        // The same rule for a function with a name. It is made at the top
        // of its block, so that it can be called from above, but the `x`
        // declared below it is still not the one it was written to use.
        let source = "let x = 1\nfunction outer() -> int {\n function inner() -> int { return x }\n let x = 5\n return inner() + x\n}\nprint(outer())";
        assert_eq!(output(source), "6\n");
        // That holds at the top level of a file too, where the name
        // hidden is a built-in.
        let source = "function size(items: list[int]) -> int { return len(items) }\nlet len = 5\nprint(size([1, 2]), len)";
        assert_eq!(output(source), "2 5\n");
        // What is declared above it is seen, wherever it is called from...
        assert_eq!(output("let a = 1\nprint(f())\nfunction f() -> int { return a }"), "1\n");
        // ...once the line that declares it has run.
        assert_eq!(error("print(f())\nlet a = 1\nfunction f() { return a }"), "`a` is not defined");
        // A name means one variable, whenever the function is called. Here
        // `inner` uses the `x` of its own block, which has no value at
        // the first call. It does not fall back on the `x` outside.
        let source = "let x = 1\nfunction outer() {\n print(inner())\n let x = 5\n function inner() { return x }\n}\nouter()";
        assert_eq!(error(source), "`x` is not defined");
        // A method follows the same rule.
        let source = "let k = 2\nfunction make() -> int {\n class Scale {\n  by: int = k\n  function apply(self, n: int) -> int { return n * self.by * k }\n }\n let k = 10\n return Scale().apply(k)\n}\nprint(make())";
        assert_eq!(output(source), "40\n");
    }

    #[test]
    fn uncaught_errors_stop_the_program() {
        assert_eq!(error("raise ValueError(\"bad\")"), "uncaught ValueError: bad");
        assert_eq!(error("try {\n raise ValueError(\"bad\")\n} except IndexError {\n}"), "uncaught ValueError: bad");
        assert_eq!(error("print(1 / 0)"), "uncaught ZeroDivisionError: division by zero");
        assert_eq!(error("print([1, 2][2])"), "uncaught IndexError: index 2 is out of range for a list of length 2");
        assert_eq!(error("print([1, 2][-1])"), "uncaught IndexError: index -1 is out of range for a list of length 2");
        assert_eq!(error("print(9223372036854775807 + 1)"), "uncaught OverflowError: the result is too big for an int");
        assert_eq!(error("raise 5"), "only an error can be raised, and this is an int");
    }

    #[test]
    fn bugs_cannot_be_caught() {
        assert_eq!(error("try {\n print(\"abc\" - 1)\n} except {\n print(\"caught\")\n}"), "`-` does not work between a str and an int");
        assert_eq!(error("try {\n print(missing)\n} except Error {\n}"), "`missing` is not defined");
    }

    /// What `zeno test` prints for a program, and how each test went:
    /// its name, and the message it failed with.
    fn tested(source: &str) -> (String, Vec<(String, Option<String>)>) {
        let program = parser::parse(lexer::lex(source).unwrap()).unwrap();
        let complaints = crate::checker::check(&program, false);
        assert!(complaints.is_empty(), "the checker rejected the program: {complaints:?}\n{source}");
        let (output, tested) = super::run_tests(&program, true, crate::value::testing::outside(), None);
        let tested = tested.expect("the program ran");
        (output, tested.into_iter().map(|test| (test.name, test.failure.map(|failure| failure.message))).collect())
    }

    #[test]
    fn tests_are_chosen_by_what_is_in_their_name() {
        let source = "print(\"the program\")\ntest \"an empty box\" {\n assert true\n}\n\
                      test \"a full box\" {\n assert false\n}\ntest \"empty again\" {\n assert true\n}";
        let program = parser::parse(lexer::lex(source).unwrap()).unwrap();
        let run = |only| super::run_tests(&program, true, crate::value::testing::outside(), only).0;
        // The program still runs whole: its tests may need what it made.
        assert_eq!(run(Some("empty")), "the program\n  ok      an empty box\n  ok      empty again\n");
        assert_eq!(run(Some("full box")), "the program\n  FAILED  a full box\n");
        assert_eq!(run(Some("Empty")), "the program\n");
    }

    #[test]
    fn the_tests_of_a_program_are_run_after_it() {
        let source = "var calls = 0\n\
             function half(n: int) -> int raises ValueError {\n calls += 1\n if n % 2 == 1 {\n  raise ValueError(\"odd\")\n }\n return n // 2\n}\n\
             test \"even\" {\n assert half(4) == 2\n print(\"in the test\")\n}\n\
             test \"wrong\" {\n assert half(4) == 3\n print(\"never\")\n}\n\
             print(\"the program\")\n\
             test \"odd\" {\n assert raises ValueError {\n  half(3)\n }\n}\n\
             test \"nothing raised\" {\n assert raises ValueError {\n  half(2)\n }\n}\n\
             test \"another kind\" {\n assert raises KeyError {\n  half(3)\n }\n}\n\
             test \"an error\" {\n half(5)\n}\n\
             test \"not caught\" {\n try {\n  assert calls == 0\n } except Error {\n  print(\"caught\")\n }\n}\n\
             test \"plain\" {\n assert calls > 0 and not false\n assert \"a\" in [\"b\"]\n}\n\
             test \"chain\" {\n assert 1 < 2 < 2\n}";
        let (output, tests) = tested(source);
        // The program runs first, whole, and then its tests in order.
        // Each says how it went as it ends.
        assert_eq!(
            output,
            "the program\nin the test\n  ok      even\n  FAILED  wrong\n  ok      odd\n  FAILED  nothing raised\n  \
             FAILED  another kind\n  FAILED  an error\n  FAILED  not caught\n  FAILED  plain\n  FAILED  chain\n"
        );
        let failures: Vec<&str> = tests.iter().map(|(_, failure)| failure.as_deref().unwrap_or("ok")).collect();
        assert_eq!(
            failures,
            [
                "ok",
                // A comparison says what was on each side.
                "this is false. The left of `==` is 2, and the right is 3",
                "ok",
                "this block was expected to raise ValueError, and it raised nothing",
                "this block was expected to raise KeyError, and it raised ValueError: odd",
                // An error that leaves a test fails it.
                "uncaught ValueError: odd",
                // A failed `assert` is not an error an `except` can catch.
                "this is false. The left of `==` is 6, and the right is 0",
                "this is false. The left of `in` is \"a\", and the right is [\"b\"]",
                "this is false",
            ]
        );
        // `zeno run` goes past the tests, in both engines.
        assert_eq!(output_of_run(source), "the program\n");
    }

    fn output_of_run(source: &str) -> String {
        output(source)
    }

    #[test]
    fn a_program_that_stops_runs_no_test() {
        let program = parser::parse(lexer::lex("print(1 // 0)\ntest \"x\" {\n assert true\n}").unwrap()).unwrap();
        let (output, tested) = super::run_tests(&program, true, crate::value::testing::outside(), None);
        assert_eq!(output, "");
        assert_eq!(tested.err().map(|error| error.message).as_deref(), Some("uncaught ZeroDivisionError: division by zero"));
    }

    #[test]
    fn output_before_an_error_is_kept() {
        let (output, result) = run("print(\"one\")\nprint(1 / 0)\nprint(\"two\")");
        assert_eq!(output, "one\n");
        assert!(result.is_err());
    }

    // ----- The bits of whole numbers -----

    #[test]
    fn bit_operators() {
        // Checked against Python, which gives the same answers.
        assert_eq!(output("print(0b1100 & 0b1010, 0b1100 | 0b1010, 0b1100 ^ 0b1010, ~0b1100, ~0, ~-1)"), "8 14 6 -13 -1 0\n");
        assert_eq!(output("print(1 << 5, 1 << 62, 0xFF >> 4, -8 >> 1, -7 >> 1, -1 >> 63, -1 << 63)"), "32 4611686018427387904 15 -4 -4 -1 -9223372036854775808\n");
        assert_eq!(output("print(-1 & 0xFF, ~5 & 0xFF, 6 | 1 ^ 3 & 2, 1 | 2 == 3, 1 << 2 + 1)"), "255 250 7 true 8\n");
        assert_eq!(output("var r = 0\nr |= 1 << 3\nr |= 0x81\nr &= ~1\nr ^= 0xFF\nr <<= 4\nr >>= 2\nprint(r)"), "476\n");
    }

    #[test]
    fn a_shift_must_fit() {
        assert_eq!(error("print(3 << 62)"), "uncaught OverflowError: the result is too big for an int");
        assert_eq!(error("let n = 64\nprint(1 << n)"), "uncaught ValueError: cannot shift by 64. An int has 64 bits, so a shift is 0 to 63");
        assert_eq!(error("let n = -1\nprint(1 >> n)"), "uncaught ValueError: cannot shift by -1. An int has 64 bits, so a shift is 0 to 63");
    }

    // ----- Sized whole numbers -----

    #[test]
    fn a_compound_assignment_is_the_long_form() {
        // What is there is read first, and then the value is worked out,
        // as in `count = count + bump()`.
        let source = "var count = 1\nfunction bump() -> int {\n    count = 10\n    return 1\n}\n\
                      count += bump()\nprint(count)\ncount = 1\ncount = count + bump()\nprint(count)\n\
                      let xs = [1, 2]\nfunction grow() -> int {\n    xs[0] = 10\n    return 1\n}\nxs[0] += grow()\nprint(xs)";
        assert_eq!(output(source), "2\n2\n[2, 2]\n");
        // Every operator on two numbers has an `=` form.
        let source = "var n = 17\nn //= 2\nprint(n)\nn %= 5\nprint(n)\nn **= 4\nprint(n)\nvar x = 7.5\nx //= 2\nx %= 2\nx **= 2\nprint(x)";
        assert_eq!(output(source), "8\n3\n81\n1.0\n");
    }

    #[test]
    fn a_quantity_takes_the_formats_of_a_float() {
        // The decimals are the number's, the unit follows, and the
        // width counts both.
        let source = "let angle = 30.456 deg\nlet supply = 3.3 V\nlet wait = 500 ms\n\
                      print(f\"{angle:.1f}|{supply:.2f}|{supply}|{wait:.0f}|{wait:f}|{wait:.3}\")\n\
                      print(f\"[{supply:10.2f}] [{supply:<10.2f}] [{supply:^10.2f}] [{supply:010.2f}] [{-supply:8.1f}]\")\n\
                      print(f\"[{wait:10}] [{wait:<10}] [{wait:2}]\")";
        assert_eq!(
            output(source),
            "30.5 deg|3.30 V|3.3 V|500 ms|500.000000 ms|500.000 ms\n\
             [    3.30 V] [3.30 V    ] [  3.30 V  ] [00003.30 V] [  -3.3 V]\n\
             [    500 ms] [500 ms    ] [500 ms]\n"
        );
        // It is not a whole number.
        let (_, result) = run("let supply = 3.3 V\nprint(f\"{supply:x}\")");
        assert_eq!(result.unwrap_err(), "`x` needs an int, but this is a quantity in V");
    }

    #[test]
    fn sized_ints() {
        // A sized number is an ordinary int that is known to be small:
        // arithmetic on two of them is never cut down to their size.
        assert_eq!(output("let a: u8 = 200\nlet b: u8 = 100\nprint(a + b, a * b, b - a, ~a, a << 8)"), "300 20000 -100 -201 51200\n");
        assert_eq!(output("print(u8(255), i8(-128), u16(65535), i16(-32768), u32(4294967295), i32(-2147483648))"), "255 -128 65535 -32768 4294967295 -2147483648\n");
        assert_eq!(output("print(u8.wrap(300), u8.wrap(-1), i8.wrap(200), i16.wrap(0xFFFF), u32.wrap(-1), i32.wrap(0x80000000))"), "44 255 -56 -1 4294967295 -2147483648\n");
        assert_eq!(output("var count: u8 = 254\nfor i in range(3) {\n    count = u8.wrap(count + 1)\n    print(count)\n}"), "255\n0\n1\n");
        assert_eq!(output("class Reg {\n    value: u8\n}\nlet r = Reg(value=1)\nr.value = u8(r.value | 0x80)\nprint(r, r.value >> 7)"), "Reg(value=129) 1\n");
        assert_eq!(output("let bytes: list[u8] = [1, 2, 255]\nbytes.append(7)\nprint(bytes, sum(bytes), bytes[2] << 8 | bytes[3])"), "[1, 2, 255, 7] 265 65287\n");
        // `u8` is a value like any built-in, and `wrap` belongs to it.
        assert_eq!(output("let make = u8\nlet cut = u8.wrap\nprint(make(7), cut(263))"), "7 7\n");
    }

    #[test]
    fn a_sized_int_must_fit() {
        assert_eq!(error("let n = 300\nprint(u8(n))"), "uncaught OverflowError: 300 does not fit in a u8 (0 to 255)");
        assert_eq!(error("let n = -1\nprint(u16(n))"), "uncaught OverflowError: -1 does not fit in a u16 (0 to 65535)");
        assert_eq!(error("let n = 128\nprint(i8(n))"), "uncaught OverflowError: 128 does not fit in an i8 (-128 to 127)");
        assert_eq!(
            output("let a: u8 = 200\ntry { print(u8(a + a)) } except OverflowError as e { print(e.message) }"),
            "400 does not fit in a u8 (0 to 255)\n"
        );
        assert_eq!(error("let x = \"7\"\nlet f = u8\nprint(f(x))"), "`u8` needs a whole number, but this is a str");
    }

    // ----- Generics -----

    #[test]
    fn generics() {
        // When a program runs, a generic is one function: values carry
        // their own types, so there is nothing to fill in.
        let first = "function first[T](items: list[T]) -> T {\n    return items[0]\n}\n";
        assert_eq!(output(&format!("{first}print(first([1, 2]), first([\"a\"]), first([[2.5]]))")), "1 a [2.5]\n");
        let stack = "class Stack[T] {\n    items: list[T]\n    function push(self, item: T) {\n        self.items.append(item)\n    }\n\
                     \x20   function pop(self) -> T {\n        return self.items.pop()\n    }\n}\n";
        assert_eq!(
            output(&format!(
                "{stack}let a: Stack[int] = Stack(items=[])\na.push(1)\na.push(2)\nlet b = Stack([\"x\"])\nb.push(\"y\")\n\
                 print(a.pop(), b.pop(), a, b)"
            )),
            "2 y Stack(items=[1]) Stack(items=[\"x\"])\n"
        );
        // Two type parameters, one of them worked out from a function.
        assert_eq!(
            output(
                "function convert[T, U](items: list[T], change: function(T) -> U) -> list[U] {\n\
                 \x20   var out: list[U] = []\n    for item in items {\n        out.append(change(item))\n    }\n    return out\n}\n\
                 print(convert([1, 2], function(n: int) -> str { return f\"<{n}>\" }), convert([\"ab\"], len))"
            ),
            "[\"<1>\", \"<2>\"] [2]\n"
        );
        // A generic that calls itself, and one that calls another.
        assert_eq!(
            output(
                "function count[T](items: list[T], at: int) -> int {\n    if at >= len(items) {\n        return 0\n    }\n\
                 \x20   return 1 + count(items, at + 1)\n}\n\
                 function wrap[T](x: T) -> list[T] {\n    return [x]\n}\n\
                 function twice[T](x: T) -> list[list[T]] {\n    return wrap(wrap(x))\n}\n\
                 print(count([1, 2, 3], 0), count([\"a\"], 0), twice(5), twice(\"x\"))"
            ),
            "3 1 [[5]] [[\"x\"]]\n"
        );
        // A class that mentions itself.
        assert_eq!(
            output(
                "class Node[T] {\n    value: T\n    next: Node[T] or none\n}\n\
                 function length[T](node: Node[T] or none) -> int {\n    var n = 0\n    var at = node\n\
                 \x20   while at is not none {\n        n += 1\n        at = at.next\n    }\n    return n\n}\n\
                 let chain = Node(1, Node(2, none))\nprint(length(chain), chain.next)"
            ),
            "2 Node(value=2, next=none)\n"
        );
        // With the types left out it still runs.
        assert_eq!(
            output("function id[T](x: T) -> T {\n    return x\n}\nfunction loose(x) {\n    return id(x)\n}\nprint(loose(3), loose(\"a\"))"),
            "3 a\n"
        );
    }

    #[test]
    fn the_generics_example() {
        assert_eq!(
            output(include_str!("../examples/generics.zeno")),
            "bytes: [108, 108, 111, 33] newest 33 dropped 2\n\
             above 0x60: 3\n\
             as hex: [\"6c\", \"6c\", \"6f\", \"21\"]\n\
             no reading yet: none\n\
             volts: [3.3 V, 2.9 V, 3.2 V] highest 3.3 V\n\
             low: 1\n\
             log: [\"sensor ready\", \"first reading\"] last in the alphabet: sensor ready\n\
             lengths: [12, 13]\n\
             Ring(slots=[\"first reading\", \"sensor ready\"], next=1, count=2, dropped=1)\n"
        );
    }

    // ----- The board and the clock -----

    #[test]
    fn a_pin_shows_a_line_when_its_level_changes() {
        let source = "from board import led, pin\nled.on()\nled.on()\nled.toggle()\npin(7).toggle()\nled.off()\n\
                      print(led, pin(7), led.is_on(), pin(7).is_on(), led.number, led == pin(13))";
        assert_eq!(output(source), "led on\nled off\npin 7 on\nPin(13) Pin(7) false true 13 true\n");
    }

    #[test]
    fn the_board_has_64_pins() {
        assert_eq!(output("from board import pin\npin(0).on()\npin(63).on()"), "pin 0 on\npin 63 on\n");
        assert_eq!(error("from board import pin\npin(64)"), "uncaught ValueError: this board has no pin 64. Its pins are 0 to 63");
        assert_eq!(output("from board import pin\ntry { pin(-1) } except ValueError as e { print(e.message) }"), "this board has no pin -1. Its pins are 0 to 63\n");
    }

    #[test]
    fn the_clock_only_goes_forward() {
        let source = "from time import sleep, now\nlet start = now()\nsleep(2 ms)\nsleep(0 s)\nsleep(-1 s)\n\
                      let took = now() - start\nprint(took >= 2 ms, took < 5 s)";
        assert_eq!(output(source), "true true\n");
    }

    #[test]
    fn return_outside_a_function() {
        assert_eq!(error("return 1"), "`return` outside a function");
    }

    // ----- The page -----
    //
    // These run on the simulated page (`page.rs`). The real one needs a
    // browser: docs/web.md says how it was tried in one.

    #[test]
    fn the_page_shows_a_line_for_each_change_that_could_be_seen() {
        let source = "import web\nlet title = web.element(\"#title\")\n\
                      title.text = \"Hello\"\ntitle.text = \"Hello\"\n\
                      title.add_class(\"big\")\ntitle.add_class(\"big\")\ntitle.remove_class(\"big\")\ntitle.remove_class(\"big\")\n\
                      title.set(\"lang\", \"ta\")\ntitle.set(\"lang\", \"ta\")\n\
                      title.hide()\ntitle.hide()\ntitle.show()\n\
                      print(title, title.text, title.get(\"lang\"), title.get(\"href\"), title.has_class(\"big\"))\n\
                      print(title == web.element(\"#title\"), title == web.element(\"#other\"))";
        assert_eq!(
            output(source),
            "#title text: Hello\n#title class +big\n#title class -big\n#title lang: ta\n#title hidden\n#title shown\n\
             Element(#title) Hello ta none false\ntrue false\n"
        );
    }

    #[test]
    fn an_element_is_seen_once_it_is_on_the_page() {
        let source = "import web\nlet list = web.element(\"#list\")\n\
                      let item = web.create(\"li\", \"milk\")\nitem.add_class(\"new\")\nitem.text = \"Milk\"\n\
                      print(item, item.text, len(web.elements(\"li\")))\n\
                      list.append(item)\nitem.text = \"Oat milk\"\n\
                      let more = web.create(\"li\")\nmore.append(web.create(\"b\", \"bread\"))\nlist.append(more)\n\
                      print(len(web.elements(\"li\")), len(web.elements(\".new\")), len(web.elements(\"b\")), list.text)\n\
                      item.remove()\nitem.text = \"gone\"\n\
                      print(len(web.elements(\"li\")), len(web.elements(\".new\")))\n\
                      list.clear()\nlist.clear()\nprint(len(web.elements(\"li\")), len(web.elements(\"b\")))";
        assert_eq!(
            output(source),
            "Element(<li>) Milk 0\n#list + <li> Milk\n<li> text: Oat milk\n#list + <li> bread\n2 1 1 Oat milkbread\n\
             <li> removed\n1 0\n#list cleared\n0 0\n"
        );
    }

    #[test]
    fn an_element_is_in_one_place() {
        // Appended somewhere else, it leaves where it was.
        let source = "import web\nlet a = web.element(\"#a\")\nlet b = web.element(\"#b\")\n\
                      let p = web.create(\"p\", \"one\")\na.append(p)\nb.append(p)\nprint(a.text == \"\", b.text)\n\
                      let box = web.create(\"div\")\nbox.append(p)\nprint(len(web.elements(\"p\")))\n\
                      a.append(box)\nprint(len(web.elements(\"p\")))";
        assert_eq!(output(source), "#a + <p> one\n#b + <p> one\ntrue one\n<p> removed\n0\n#a + <div> one\n1\n");
        let inside_itself = "uncaught ValueError: an element cannot be put inside itself, or inside something it holds";
        assert_eq!(error("import web\nlet a = web.element(\"#a\")\na.append(a)"), inside_itself);
        assert_eq!(
            error("import web\nlet a = web.element(\"#a\")\nlet p = web.create(\"p\")\na.append(p)\np.append(a)"),
            inside_itself
        );
    }

    #[test]
    fn the_simulated_page_reads_plain_selectors() {
        let source = "import web\nlet list = web.element(\"#list\")\nweb.element(\"#title\").add_class(\"big\")\n\
                      for word in [\"milk\", \"bread\", \"tea\"] {\n let item = web.create(\"li\", word)\n\
                      if word != \"bread\" { item.add_class(\"done\") }\n list.append(item)\n}\n\
                      let other = web.create(\"li\", \"outside\")\nother.add_class(\"done\")\nother.set(\"id\", \"last\")\n\
                      web.element(\"#other\").append(other)\n\
                      function count(selector: str) -> int { return len(web.elements(selector)) }\n\
                      print(count(\"li\"), count(\"li.done\"), count(\"#list li.done\"), count(\"#list .done\"))\n\
                      print(count(\"#other li\"), count(\"#list #last\"), count(\"li#last.done\"), count(\".big\"), count(\"p.big\"))\n\
                      print(count(\"#list\"), count(\"#nothing li\"), count(\"#list #list\"))";
        let printed = output(source);
        assert!(printed.ends_with("4 3 2 2\n1 0 1 1 0\n1 0 0\n"), "{printed}");
        // What only a browser can read is not answered with a guess.
        assert_eq!(
            error("import web\nweb.elements(\"ul > li\")"),
            "uncaught ValueError: the simulated page cannot read `ul > li`. It reads a tag, `.class` and `#id`, together \
             (`li.done`) and one inside another (`#list li`). A browser reads the rest, so the page that is built can use it"
        );
    }

    #[test]
    fn what_is_typed_into_an_element_is_its_value() {
        let source = "import web\nlet box = web.element(\"input\")\nbox.value = \"typed\"\nbox.value = \"typed\"\n\
                      print(box.value, box.text == \"\")\nbox.value += \"!\"\nprint(box.value.upper())";
        assert_eq!(output(source), "input value: typed\ntyped true\ninput value: typed!\nTYPED!\n");
    }

    #[test]
    fn the_simulated_page_calls_nothing() {
        // Nobody is there to press a button, and no time passes.
        let source = "import web\nfunction tick() { print(\"tick\") }\n\
                      web.after(500 ms, tick)\nweb.after(2 s, tick)\n\
                      web.on_key(function(key: str) { print(key) })\n\
                      web.element(\"#go\").on(\"click\", tick)\nprint(\"nothing was called\")";
        assert_eq!(output(source), "nothing was called\n");
    }

    #[test]
    fn the_page_says_what_it_cannot_do() {
        assert_eq!(
            error("import web\nweb.create(\"<li>\", \"x\")"),
            "uncaught ValueError: `<li>` is not the name of a kind of element. Write one such as `li` or `button`"
        );
        assert_eq!(
            error("import web\nweb.after(500, print)"),
            "`after` needs a time with a unit, such as `500 ms`, but this is an int"
        );
        assert_eq!(error("import web\nweb.after(1 V, print)"), "`after` needs a time, such as `500 ms`, not a quantity in V");
        assert_eq!(error("import web\nweb.on_key(5)"), "`on_key` needs a function to call, but this is an int");
        assert_eq!(
            error("import web\nweb.element(\"p\").shout()"),
            "an element has no field or method `shout`"
        );
    }

    #[test]
    fn a_page_calls_a_function_of_the_program_later() {
        // What a browser does when a button is pressed, done by hand:
        // the program has run to its last line, and is then called.
        let source = "import web\nlet count = web.element(\"#count\")\nvar presses = 0\n\
                      function pressed() {\n presses += 1\n count.text = f\"{presses} presses\"\n}\n\
                      web.element(\"#press\").on(\"click\", pressed)\n\
                      web.on_key(function(key: str) {\n if key == \"Enter\" { pressed() }\n print(key)\n})\n\
                      web.after(1 s, function() { print([1][presses]) })\n\
                      print(\"ready\")";
        let program = crate::modules::testing::program(source);
        let (mut session, result) = super::Session::start(program, crate::page::Page::simulated(None), true);
        assert!(result.is_ok());
        assert_eq!(session.printed(), "ready\n");
        // The first function it gave to the page was `pressed`.
        session.call(0, Vec::new()).expect("a press");
        session.call(0, Vec::new()).expect("a press");
        assert_eq!(session.printed(), "#count text: 1 presses\n#count text: 2 presses\n");
        // The second takes the key.
        session.call(1, vec![super::Value::str("a")]).expect("a key");
        session.call(1, vec![super::Value::str("Enter")]).expect("a key");
        assert_eq!(session.printed(), "a\n#count text: 3 presses\nEnter\n");
        // An error in one is the program's error, with its line.
        let error = session.call(2, Vec::new()).expect_err("out of range");
        assert_eq!(error.message, "uncaught IndexError: index 3 is out of range for a list of length 1");
        assert_eq!(error.span.line, 13);
        // A number the page was never given calls nothing.
        session.call(9, Vec::new()).expect("nothing to call");
        assert_eq!(session.printed(), "");
    }

    #[test]
    fn fire_calls_what_on_was_given() {
        // How a test presses a button: at once, in the order given, and
        // with the event for a function that asks for it.
        let source = "import web\nlet button = web.element(\"#press\")\nvar presses = 0\n\
                      button.on(\"click\", function() { presses += 1 })\n\
                      button.on(\"click\", function(event: Event) {\n print(event, event.kind, event.target, event.x, event.y, event.key == \"\")\n print(event.target == button)\n})\n\
                      button.on(\"input\", function() { print(\"typed\") })\n\
                      button.fire(\"click\")\nbutton.fire(\"click\")\nbutton.fire(\"keydown\")\nprint(presses)\n\
                      class Counter {\n n: int\n function heard(self, event: Event) { self.n += 1 }\n function plain(self) { self.n += 10 }\n}\n\
                      let counter = Counter(0)\nbutton.on(\"wheel\", counter.heard)\nbutton.on(\"wheel\", counter.plain)\nbutton.fire(\"wheel\")\nprint(counter.n)";
        assert_eq!(
            output(source),
            "Event(click) click Element(#press) 0.0 0.0 true\ntrue\nEvent(click) click Element(#press) 0.0 0.0 true\ntrue\n2\n11\n"
        );
        assert_eq!(
            error("import web\nweb.element(\"#a\").on(\"click\", function() { print([1][2]) })\nweb.element(\"#a\").fire(\"click\")"),
            "uncaught IndexError: index 2 is out of range for a list of length 1"
        );
    }

    #[test]
    fn styles_and_what_is_saved_and_what_is_drawn() {
        let source = "import web\nlet box = web.element(\"#box\")\n\
                      print(box.get_style(\"color\"))\nbox.set_style(\"color\", \"red\")\nbox.set_style(\"color\", \"red\")\n\
                      box.set_style(\"color\", \"blue\")\nprint(box.get_style(\"color\"))\n\
                      print(web.load(\"name\"))\nweb.save(\"name\", \"Partha\")\nweb.save(\"name\", \"Zeno\")\nprint(web.load(\"name\"))\n\
                      web.forget(\"name\")\nweb.forget(\"name\")\nprint(web.load(\"name\"))\n\
                      box.draw_rect(10, 10, 50.5, 20, \"red\")\nbox.draw_circle(5, 5, 2, \"#00f\")\nbox.draw_line(0, 0, 9, 9, \"black\")\n\
                      box.draw_text(1, 2, \"two words\", \"green\")\nbox.erase()\n\
                      web.every(1 s, function() { print(\"never on a desk\") })\n\
                      web.fetch(\"/prices\", function(status: int, body: str) { print(status) })";
        assert_eq!(
            output(source),
            "none\n#box style color: red\n#box style color: blue\nblue\nnone\nZeno\nnone\n\
             #box draw_rect 10 10 50.5 20 red\n#box draw_circle 5 5 2 #00f\n#box draw_line 0 0 9 9 black\n\
             #box draw_text 1 2 two words green\n#box erase\n"
        );
        assert_eq!(
            error("import web\nweb.create(\"p\", \"x\").draw_rect(0, 0, 1, 1, \"red\")"),
            "uncaught ValueError: `draw_rect` draws on a `<canvas>`, and this element is not one"
        );
        assert_eq!(error("import web\nweb.every(0 ms, function() { print(1) })"), "uncaught ValueError: `every` needs a time of at least 1 ms");
    }

    #[test]
    fn a_timer_can_be_stopped() {
        let source = "import web\nlet ticking = web.every(1 s, function() { print(\"tick\") })\n\
                      let once: Timer = web.after(500 ms, function() { print(\"late\") })\n\
                      print(ticking, ticking == ticking, ticking == once)\n\
                      ticking.stop()\nticking.stop()\nonce.stop()\nprint(\"stopped\")";
        assert_eq!(output(source), "Timer true false\nstopped\n");
    }

    #[test]
    fn a_page_that_was_written_has_the_elements_it_has() {
        let page = "<body><h1 id=title class=\"big\">Hello</h1>\n<form id=add><input id=what value=\"milk\"><textarea id=note>a note</textarea></form>\n\
                    <ul id=list>\n  <li>one<li class=done>two</ul>\n<p id=left style=\"color: red; width: 10px\" hidden></p>\n<canvas id=board></canvas></body>";
        crate::value::testing::on_page(page, || {
            let source = "import web\nlet title = web.element(\"#title\")\n\
                          print(title.text, title.has_class(\"big\"), title.get(\"class\"), web.element(\"h1\") == title)\n\
                          print(web.element(\"#what\").value, web.element(\"#note\").value, len(web.elements(\"li\")), len(web.elements(\"#list li.done\")))\n\
                          let list = web.element(\"#list\")\nprint(list.text)\nlist.append(web.create(\"li\", \"three\"))\n\
                          let left = web.element(\"#left\")\nprint(left.get_style(\"width\"), left.get_style(\"height\"))\nleft.show()\nleft.text = \"2 left\"\n\
                          web.element(\"#board\").draw_rect(0, 0, 1, 1, \"red\")\n\
                          print(len(web.elements(\"em\")))\nlist.clear()\nprint(len(web.elements(\"li\")))";
            assert_eq!(
                output(source),
                "Hello true big true\nmilk a note 2 1\nonetwo\n#list + <li> three\n10px none\n#left shown\n#left text: 2 left\n\
                 #board draw_rect 0 0 1 1 red\n0\n#list cleared\n0\n"
            );
            // The reason the page is read: a name spelt wrong is found on a desk.
            assert_eq!(
                error("import web\nlet count = web.element(\"#cuont\")"),
                "uncaught ValueError: the page has no element that answers to `#cuont`. `web.elements` gives a list, which may be empty"
            );
            assert_eq!(
                error("import web\nweb.element(\"#title\").draw_rect(0, 0, 1, 1, \"red\")"),
                "uncaught ValueError: `draw_rect` draws on a `<canvas>`, and this element is not one"
            );
        });
        // The to-do list says the same with its page as without.
        let without = output(include_str!("../examples/todo.zeno"));
        crate::value::testing::on_page(include_str!("../examples/todo.html"), || {
            assert_eq!(output(include_str!("../examples/todo.zeno")), without);
        });
    }

    #[test]
    fn the_page_hands_over_the_event_and_the_answer() {
        let source = "import web\nlet board = web.element(\"#board\")\n\
                      board.on(\"click\", function() { print(\"pressed\") })\n\
                      board.on(\"click\", function(event: Event) { print(event.kind, event.x, event.y, event.target) })\n\
                      web.fetch(\"/prices\", function(status: int, body: str) { print(status, body) })";
        let program = crate::modules::testing::program(source);
        let (mut session, result) = super::Session::start(program, crate::page::Page::simulated(None), true);
        assert!(result.is_ok());
        let event = || {
            super::Value::Event(std::rc::Rc::new(crate::value::Event {
                kind: std::rc::Rc::from("click"),
                target: super::Value::Element(0, std::rc::Rc::from("<b>")),
                x: 12.0,
                y: 3.5,
                key: std::rc::Rc::from(""),
            }))
        };
        // A function that did not ask for the event is not given it.
        session.call(0, vec![event()]).expect("a click");
        session.call(1, vec![event()]).expect("a click");
        session.call(2, vec![super::Value::Int(404), super::Value::str("not here")]).expect("an answer");
        assert_eq!(session.printed(), "pressed\nclick 12.0 3.5 Element(<b>)\n404 not here\n");
    }

    // ----- JSON (docs/json.md) -----

    #[test]
    fn json_is_read_into_values_and_written_from_them() {
        let source = "import json\n\
                      let data = json.parse(\"{\\\"name\\\": \\\"Ada\\\", \\\"age\\\": 36, \\\"tags\\\": [\\\"a\\\", \\\"b\\\"], \\\"score\\\": 9.5, \\\"gone\\\": null, \\\"ok\\\": true, \\\"in\\\": {\\\"n\\\": 2.0}}\")\n\
                      print(data)\n\
                      let name: str = data[\"name\"]\nlet age: int = data[\"age\"]\n\
                      print(name, age + 1, data[\"score\"] * 2, data.get(\"gone\"), \"gone\" in data, data[\"in\"][\"n\"])\n\
                      for tag in data[\"tags\"] {\n    print(tag.upper())\n}\n\
                      print(json.write(data))\n\
                      print(json.parse(\"[1, null, \\\"x\\\", -2.5e3, 1e300]\"), json.parse(\"null\"), json.parse(\" 12 \"), json.parse(\"\\\"t\\\\u00e9\\\"\"))\n\
                      class Line {\n    item: str\n    count: int\n}\n\
                      class Order {\n    who: str\n    lines: list[Line]\n    note: str or none\n    total: float\n}\n\
                      let order = Order(\"Ada \\\"A\\\"\\n\", [Line(\"tea\", 2), Line(\"milk\", 1)], none, 7.0)\n\
                      let sent = json.write(order)\nprint(sent)\nprint(json.parse(sent)[\"lines\"][1][\"item\"])\n\
                      print(json.write([\"a\": [1, 2], \"b\": [3]]), json.write([1, 2.5, none, true]), json.write(\"x\"), json.write([7: \"seven\"]))";
        assert_eq!(
            output(source),
            "[\"name\": \"Ada\", \"age\": 36, \"tags\": [\"a\", \"b\"], \"score\": 9.5, \"ok\": true, \"in\": [\"n\": 2]]\n\
             Ada 37 19.0 none false 2\nA\nB\n\
             {\"name\":\"Ada\",\"age\":36,\"tags\":[\"a\",\"b\"],\"score\":9.5,\"ok\":true,\"in\":{\"n\":2}}\n\
             [1, none, \"x\", -2500, 1e300] none 12 t\u{e9}\n\
             {\"who\":\"Ada \\\"A\\\"\\n\",\"lines\":[{\"item\":\"tea\",\"count\":2},{\"item\":\"milk\",\"count\":1}],\"note\":null,\"total\":7.0}\nmilk\n\
             {\"a\":[1,2],\"b\":[3]} [1,2.5,null,true] \"x\" {\"7\":\"seven\"}\n"
        );
        assert_eq!(error("import json\njson.parse(\"{oops\")"), "uncaught ValueError: this text is not JSON: `{oops`");
        assert_eq!(
            error("import json\njson.parse(\"  [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12\")"),
            "uncaught ValueError: this text is not JSON: `[1, 2, 3, 4, 5, 6, 7, 8, 9, 10...`"
        );
        assert_eq!(
            error("import json\njson.write([1 V])"),
            "uncaught ValueError: JSON has no units, and this is a quantity in V. Write its number, and put the unit in the name: `[\"V\": ...]`"
        );
        assert_eq!(
            error("import json\njson.write(print)"),
            "uncaught ValueError: JSON has objects, lists, text, numbers, bools and none, and this is a function"
        );
        assert_eq!(error("import json\njson.write(1.0 / 0.0)"), "uncaught ZeroDivisionError: division by zero");
        // An error that is caught, as any other.
        assert_eq!(
            output("import json\ntry {\n    print(json.parse(\"nope\"))\n} except ValueError as mistake {\n    print(mistake.message)\n}"),
            "this text is not JSON: `nope`\n"
        );
    }

    // ----- The examples -----

    #[test]
    fn hello_runs() {
        assert_eq!(output(include_str!("../examples/hello.zeno")), "Hello, Zeno!\n");
    }

    #[test]
    fn the_styles_example_gives_the_same_answer_four_ways() {
        assert_eq!(
            output(include_str!("../examples/styles.zeno")),
            "typed, with a class:   21.5\n\
             untyped, with a class: 21.5\n\
             functional:            21.5\n\
             comprehension:         21.5\n"
        );
    }

    #[test]
    fn units_of_one_kind_work_together() {
        assert_eq!(
            output("let t = 3 s\nlet d = 250 ms\nprint(t + d, t - d, t.ms, d.s, (90 min).h, (1 kWh).J)"),
            "3250 ms 2750 ms 3000 ms 0.25 s 1.5 h 3600000 J\n"
        );
        assert_eq!(output("print(1 s == 1000 ms, 3 s > 250 ms, 999 ms < 1 s, 1 h / 1 min)"), "true true true 60.0\n");
    }

    #[test]
    fn the_tour_runs() {
        assert_eq!(
            output(include_str!("../examples/tour.zeno")),
            "listening on 8080\n\
             area = 3.14\n\
             area = 6.00\n\
             Hello, Zeno! 4 [0, 4, 16, 36, 64] 5.0\n18 waiting 500 ms\n"
        );
    }

    // ----- Memory by hand (docs/unsafe.md) -----

    #[test]
    fn memory_by_hand() {
        assert_eq!(
            output(
                "let bytes = allocate(u8, 4)\n\
                 let wide = allocate(int, 2)\n\
                 let small = allocate(i8, 2)\n\
                 let samples = allocate(float, 3)\n\
                 unsafe {\n\
                 \x20   print(bytes[0], wide[1], samples[2])\n\
                 \x20   bytes[0] = 0x41\n\
                 \x20   bytes[3] = u8(bytes[0] + 1)\n\
                 \x20   wide[1] = -7000000000\n\
                 \x20   wide[1] += 5\n\
                 \x20   small[0] = -128\n\
                 \x20   samples[1] = 2.5\n\
                 \x20   print(bytes[0], bytes[3], wide[1], small[0], samples[1])\n\
                 \x20   let last = bytes + 3\n\
                 \x20   print(last[0], (last - 3)[0], last == bytes + 3, last == bytes)\n\
                 \x20   free(wide)\n\
                 \x20   free(small)\n\
                 \x20   free(samples)\n\
                 }\n\
                 print(bytes, [bytes, bytes + 1] == [bytes, bytes + 1])\n\
                 unsafe { free(bytes) }"
            ),
            "0 0 0.0\n65 66 -6999999995 -128 2.5\n66 65 true false\n<pointer[u8]> true\n"
        );
    }

    #[test]
    fn zeno_run_checks_every_use_of_a_pointer() {
        let p = "let p = allocate(u8, 4)\n";
        for (source, message) in [
            (
                format!("{p}unsafe {{ print(p[4]) }}"),
                "this reads item 4 of a block that has 4 (0 to 3). `zeno run` checks every use of a pointer. \
                 A compiled program does not: it reads whatever is beside the block",
            ),
            (
                format!("{p}unsafe {{ p[-1] = 1 }}"),
                "this writes over item -1 of a block that has 4 (0 to 3). `zeno run` checks every use of a pointer. \
                 A compiled program does not: it writes over whatever is beside the block",
            ),
            // Working out an address past the end is allowed. Using it is
            // what is caught.
            (
                format!("{p}let q = p + 6\nunsafe {{\n    print((q - 3)[0])\n    print(q[0])\n}}"),
                "this reads item 6 of a block that has 4 (0 to 3). `zeno run` checks every use of a pointer. \
                 A compiled program does not: it reads whatever is beside the block",
            ),
            (
                format!("{p}unsafe {{\n    free(p)\n    print(p[0])\n}}"),
                "this reads memory that was freed. `zeno run` checks every use of a pointer. \
                 A compiled program does not: it reads whatever is there now, which may be somebody else's",
            ),
            (
                format!("{p}unsafe {{\n    free(p)\n    p[0] = 1\n}}"),
                "this writes over memory that was freed. `zeno run` checks every use of a pointer. \
                 A compiled program does not: it writes over whatever is there now, which may be somebody else's",
            ),
            (
                format!("{p}unsafe {{\n    free(p)\n    free(p)\n}}"),
                "this memory was freed already. `zeno run` checks every `free`. \
                 A compiled program does not: freeing twice damages the memory it hands out next",
            ),
            (
                format!("{p}unsafe {{ free(p + 1) }}"),
                "`free` needs the pointer that `allocate` gave, and this one points 1 item further on. \
                 `zeno run` checks every `free`. A compiled program does not",
            ),
            ("let n = 0\nlet p = allocate(u8, n)".to_string(), "`allocate` needs at least one item, and this asks for 0"),
        ] {
            assert_eq!(error(&source), message, "\n{source}");
        }
        // A value that does not fit is an ordinary error, as it is for
        // any sized number, and can be caught.
        assert_eq!(
            output(
                "let p = allocate(u8, 2)\nlet n = 256\n\
                 try {\n    unsafe { p[0] = u8(n) }\n} except OverflowError as e {\n    print(e.message)\n}\n\
                 unsafe {\n    print(p[0])\n    free(p)\n}"
            ),
            "256 does not fit in a u8 (0 to 255)\n0\n"
        );
    }

    #[test]
    fn registers_on_the_simulated_board() {
        // A register nobody wrote to reads as 0. Every write prints a
        // line, also one that changes nothing.
        assert_eq!(
            output(
                "from board import register\n\
                 let mode = register(u32, 0x4002_0000)\n\
                 let data = register(u8, 0x4002_0014)\n\
                 unsafe {\n\
                 \x20   print(mode.read())\n\
                 \x20   mode.write(u32(mode.read() | 1 << 10))\n\
                 \x20   data.write(0x55)\n\
                 \x20   data.write(0x55)\n\
                 \x20   mode.write(u32(mode.read() & ~(1 << 10)))\n\
                 \x20   print(mode.read(), data.read())\n\
                 }\n\
                 print(mode, data.address, mode == register(u32, 0x4002_0004), mode == register(u32, 0x4002_0000))"
            ),
            "0\nregister 0x40020000 = 0x00000400\nregister 0x40020014 = 0x55\nregister 0x40020014 = 0x55\n\
             register 0x40020000 = 0x00000000\n0 85\nRegister(0x40020000) 1073872916 false true\n"
        );
        // The module can be used by its name as well.
        assert_eq!(
            output("import board\nlet r = board.register(u16, 0x10)\nunsafe { r.write(0xBEEF) }"),
            "register 0x00000010 = 0xbeef\n"
        );
    }

    #[test]
    fn the_driver_example_runs() {
        assert_eq!(
            output(include_str!("../examples/driver.zeno")),
            "register 0x40004408 = 0x0068\n\
             register 0x40004400 = 0x00000009\n\
             waiting: 4 refused: 2\n\
             checksum of all four: 156\n\
             checksum of the last three: 66\n\
             register 0x4000440c = 0x5a\n\
             register 0x4000440c = 0x65\n\
             register 0x4000440c = 0x6e\n\
             register 0x4000440c = 0x6f\n\
             sent: 4\n\
             register 0x4000440c = 0x21\n\
             register 0x4000440c = 0x0a\n\
             sent: 2\n\
             register 0x40004400 = 0x00000000\n\
             Register(0x40004400) Register(0x4000440c) <pointer[u8]>\n"
        );
    }
}
