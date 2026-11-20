//! `zeno test --build`: the tests of a file, run in the compiled program
//! (decision 60.2).
//!
//! `zeno test` runs a file and then its tests in the interpreter. Under
//! `--build` the file is compiled as `zeno build` compiles it, with its
//! tests in it, and the program that comes out runs the file and then
//! each test, and prints what `zeno test` prints, line for line. So a
//! test is held to the rules of the build: a test that shares a list is
//! refused with no collector, as the program would be.
//!
//! Three things make that happen, and this file holds what they share:
//!
//! - **Before the checker**, each test of the file becomes a function
//!   with no parameters that may raise any error, written at the end of
//!   the program, so that it sees every variable of the file as a test
//!   does (`with_tests`). The checker then checks it with the rules of
//!   the build, and each compiler compiles it as any other function.
//! - **In the compilers**, an `assert` in such a function works out its
//!   condition and, if it is false, hands the report to the runtime and
//!   leaves the function as `return` would: through every `try`, and
//!   freeing what the function owns, so that nothing catches it and
//!   nothing is left. After the program's own statements, `main` calls
//!   each test in turn (`TestBuild`).
//! - **In the runtime** of a test build only, a few functions that keep
//!   the reports in a room of a fixed size until the last test is over,
//!   and print them as `zeno test` does (`RUNTIME`). A program that is
//!   not built for its tests has none of this, and its C is what it was.

use crate::ast::{self, Expr, ExprKind, Function, Node, Program, Stmt, StmtKind};
use crate::error::Error;
use crate::token::Span;
use std::sync::Arc;

/// The tests a program was built with, in the order they run, and the
/// text of every file of the program, for the line a report shows.
#[derive(Debug, Clone, Default)]
pub struct TestBuild {
    /// For each test: the name of the function it became, and its own
    /// name, as `test "..."` wrote it.
    pub tests: Vec<(String, String)>,
    /// The source of each file of the program, the first file first.
    pub sources: Vec<String>,
    /// Built for a board: the program has no file of marks to write
    /// (see `RUNTIME`), and it runs by itself.
    pub board: bool,
}

/// The name of the function the test at this place in the file becomes.
/// A name has no space in it, so no program can write it or call it.
pub fn function_name(number: usize) -> String {
    format!("test {number}")
}

/// The program, with the tests of its first file made into functions at
/// its end and every other test taken out. With `only`, the tests that
/// do not have that text in their name are taken out too (17.5).
///
/// An `assert` inside a function written in a test is refused: a failed
/// `assert` leaves its test as `return` would, and from inside another
/// function that `return` would leave the wrong one.
pub fn with_tests(mut program: Program, only: Option<&str>) -> Result<(Program, Vec<(String, String)>), Error> {
    let mut tests = Vec::new();
    let mut functions = Vec::new();
    let mut kept = Vec::new();
    for statement in std::mem::take(&mut program.statements) {
        let span = statement.span;
        let StmtKind::Test { name, body } = statement.kind else {
            kept.push(statement);
            continue;
        };
        if span.file() != 0 || only.is_some_and(|text| !name.contains(text)) {
            continue;
        }
        if let Some(at) = assert_in_a_function(&body) {
            return Err(Error::new(
                "under `zeno test --build` an `assert` stands in the test itself, and not in a function written \
                 inside it. Let the function give back what it found, and `assert` that in the test",
                at,
            ));
        }
        let function_name = function_name(tests.len() + 1);
        tests.push((function_name.clone(), name));
        let function = Function {
            span,
            name: function_name,
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            // Whatever the test raises leaves it, and fails it.
            raises: vec!["Error".to_string()],
            body,
        };
        functions.push(Stmt { kind: StmtKind::Function(Arc::new(function)), span });
    }
    kept.extend(functions);
    program.statements = kept;
    Ok((program, tests))
}

/// Where the first `assert` is that stands in a function written in
/// these statements, if one does.
fn assert_in_a_function(body: &ast::Block) -> Option<Span> {
    let mut found = None;
    let mut body = body.clone();
    ast::visit(&mut body, &mut |node| {
        let inner = match node {
            Node::Stmt(Stmt { kind: StmtKind::Function(function), .. }) => function.clone(),
            Node::Expr(Expr { kind: ExprKind::Function(function), .. }) => function.clone(),
            _ => return,
        };
        if found.is_none() {
            found = first_assert(&inner.body);
        }
    });
    found
}

/// Where the first `assert` in these statements is, at any depth.
fn first_assert(body: &ast::Block) -> Option<Span> {
    let mut found = None;
    let mut body = body.clone();
    ast::visit(&mut body, &mut |node| {
        if let Node::Stmt(statement) = node {
            if found.is_none() && matches!(statement.kind, StmtKind::Assert(..) | StmtKind::AssertRaises { .. }) {
                found = Some(statement.span);
            }
        }
    });
    found
}

/// C for a string literal holding this text.
fn c_text(text: &str) -> String {
    let mut out = String::from("\"");
    for byte in text.bytes() {
        match byte {
            b'"' => out += "\\\"",
            b'\\' => out += "\\\\",
            b'\n' => out += "\\n\"\n    \"",
            b'\r' => out += "\\r",
            b'\t' => out += "\\t",
            // `??` followed by some marks is a trigraph in old C.
            b'?' => out += "\\?",
            0x20..=0x7e => out.push(byte as char),
            other => out += &format!("\\{other:03o}"),
        }
    }
    out + "\""
}

impl TestBuild {
    /// What goes after the runtime in a test build, in C and in the C
    /// runtime of an LLVM build alike: the functions below, and the text
    /// of each file of the program.
    /// With no collector the test frees an error that left it, which
    /// with one the collector does.
    pub fn runtime(&self, collector: bool) -> String {
        let mut c = String::from("\n/* The text of each file of the program, for the line a report of a test shows. */\n");
        c += "static const char *const zeno_test_sources[] = {\n";
        for source in &self.sources {
            c += &format!("    {},\n", c_text(source));
        }
        c += "};\n";
        c += &format!("static const int zeno_test_files = {};\n", self.sources.len());
        let drop = if collector { "" } else { "    zeno_error_drop(zeno_raised);\n" };
        let runtime = RUNTIME.replace("    ZENO_TEST_DROP_RAISED\n", drop);
        match self.board {
            true => c + &runtime.replace("#define ZENO_TEST_MARKS 1\n", ""),
            false => c + &runtime,
        }
    }
}

/// The part of the runtime that only a test build has.
///
/// A test that fails writes its report into a room of a fixed size, and
/// the reports are printed after the last test, where `zeno test` prints
/// them. A report that does not fit is left out whole, and counted. The
/// room is a static array, so this works with no collector and on a
/// board, where there is no `malloc`.
///
/// Run by `zeno test --build`, the program is given the name of a file in
/// `ZENO_TEST_MARKS`, and writes a mark there as its tests begin, as each
/// one ends, and when they are all over, so that the command can count
/// them, and can tell which test a program was stopped in. Run by itself
/// it prints the line of totals for its file instead.
const RUNTIME: &str = r#"
/* ------------------------------------------------------------------- Tests
 *
 * Only in a program built by `zeno test --build` (decision 60.2). */

#define ZENO_TEST_MARKS 1
#define ZENO_TEST_ROOM 8192

static char zeno_test_room[ZENO_TEST_ROOM];
static size_t zeno_test_used = 0;
static int zeno_tests_run = 0;
static int zeno_tests_failed = 0;
static int zeno_tests_left_out = 0;
static bool zeno_test_failing = false;
static bool zeno_tests_over = true;
static const char *zeno_test_running = NULL;
#ifdef ZENO_TEST_MARKS
static FILE *zeno_test_marks = NULL;

static void zeno_test_mark(char mark) {
    if (zeno_test_marks == NULL) return;
    fputc(mark, zeno_test_marks);
    fflush(zeno_test_marks);
}
#endif /* ZENO_TEST_MARKS */

/* Where the report being written began, and whether it still fits. */
static size_t zeno_test_began = 0;
static bool zeno_test_fits = true;

static void zeno_test_put(const char *text, size_t length) {
    if (!zeno_test_fits || length > ZENO_TEST_ROOM - zeno_test_used) {
        zeno_test_fits = false;
        return;
    }
    memcpy(zeno_test_room + zeno_test_used, text, length);
    zeno_test_used += length;
}

static void zeno_test_puts(const char *text) {
    zeno_test_put(text, strlen(text));
}

/* A whole number at least 0, written into `digits`, which it ends. */
static const char *zeno_test_digits(long number, char *end) {
    char *p = end;
    *--p = '\0';
    do {
        *--p = (char)('0' + number % 10);
        number /= 10;
    } while (number > 0);
    return p;
}

static void zeno_test_number(long number) {
    char digits[24];
    zeno_test_puts(zeno_test_digits(number, digits + sizeof digits));
}

/* What the program prints: where `print` writes. */
static void zeno_test_out(const char *text) {
    zeno_out(text, strlen(text));
}

static void zeno_test_out_number(long number) {
    char digits[24];
    zeno_test_out(zeno_test_digits(number, digits + sizeof digits));
}

/* The line of the source a report shows, as `zeno test` shows it. */
static void zeno_test_line(int line, const char **text, size_t *length) {
    int file = line / 1000000, number = line % 1000000;
    *text = "";
    *length = 0;
    if (file >= zeno_test_files) return;
    const char *at = zeno_test_sources[file];
    for (int n = 1; n < number && *at != '\0'; at++) {
        if (*at == '\n') n++;
    }
    if (number < 1) return;
    const char *end = at;
    while (*end != '\0' && *end != '\n') end++;
    if (end > at && end[-1] == '\r') end--;
    *text = at;
    *length = (size_t)(end - at);
}

/* A report begins: the name of the test that failed, and its message. */
static void zeno_test_begin_report(void) {
    zeno_test_failing = true;
    zeno_test_began = zeno_test_used;
    zeno_test_fits = true;
    zeno_test_puts("\nFAILED: ");
    zeno_test_puts(zeno_test_running);
    zeno_test_puts("\nerror: ");
}

/* A report ends: where it happened, the line, a caret under the place,
 * and a note if there is one. One that did not fit is taken out whole. */
static void zeno_test_end_report(int line, int col, const char *note) {
    const char *text;
    size_t length;
    zeno_test_line(line, &text, &length);
    int shown = line % 1000000, gutter = 1;
    for (int n = shown; n >= 10; n /= 10) gutter++;
    zeno_test_puts("\n");
    for (int i = 0; i < gutter; i++) zeno_test_puts(" ");
    zeno_test_puts("--> ");
    zeno_test_puts(zeno_file_of(line));
    zeno_test_puts(":");
    zeno_test_number(shown);
    zeno_test_puts(":");
    zeno_test_number(col);
    zeno_test_puts("\n");
    for (int i = 0; i < gutter; i++) zeno_test_puts(" ");
    zeno_test_puts(" |\n");
    zeno_test_number(shown);
    zeno_test_puts(" | ");
    zeno_test_put(text, length);
    zeno_test_puts("\n");
    for (int i = 0; i < gutter; i++) zeno_test_puts(" ");
    zeno_test_puts(" | ");
    /* Under the place: a tab where the line has one, so that the caret
     * lines up, and a space for every other character. */
    size_t at = 0;
    for (int c = 1; c < col && at < length; c++) {
        zeno_test_puts(text[at] == '\t' ? "\t" : " ");
        at++;
        while (at < length && ((unsigned char)text[at] & 0xC0) == 0x80) at++;
    }
    zeno_test_puts("^\n");
    if (note != NULL) {
        for (int i = 0; i < gutter; i++) zeno_test_puts(" ");
        zeno_test_puts(" = note: ");
        zeno_test_puts(note);
        zeno_test_puts("\n");
    }
    zeno_test_puts("\n");
    if (!zeno_test_fits) {
        zeno_test_used = zeno_test_began;
        zeno_tests_left_out++;
    }
}

/* `assert` found its condition false. `message` is the text after its
 * comma, or NULL; `op` the comparison, or NULL when it was not one, and
 * then `left` and `right` are not looked at. */
static void zeno_test_false(const char *message, const char *op, const char *left, const char *right, int line, int col) {
    zeno_test_begin_report();
    if (message != NULL) {
        zeno_test_puts(message);
        zeno_test_puts("\n");
    }
    zeno_test_puts("this is false");
    if (op != NULL) {
        zeno_test_puts(". The left of `");
        zeno_test_puts(op);
        zeno_test_puts("` is ");
        zeno_test_puts(left);
        zeno_test_puts(", and the right is ");
        zeno_test_puts(right);
    }
    zeno_test_end_report(line, col, NULL);
}

/* `assert raises Kind { ... }`, and the block raised nothing. */
static void zeno_test_raised_nothing(const char *kind, int line, int col) {
    zeno_test_begin_report();
    zeno_test_puts("this block was expected to raise ");
    zeno_test_puts(kind);
    zeno_test_puts(", and it raised nothing");
    zeno_test_end_report(line, col, NULL);
}

/* `assert raises Kind { ... }`, and the block raised another kind. */
static void zeno_test_raised_other(const char *kind, zeno_error *error, int line, int col) {
    char note[64] = "that error was raised on line ", digits[24];
    strcat(note, zeno_test_digits(zeno_line_of(zeno_raised_line), digits + sizeof digits));
    zeno_test_begin_report();
    zeno_test_puts("this block was expected to raise ");
    zeno_test_puts(kind);
    zeno_test_puts(", and it raised ");
    zeno_test_puts(zeno_kind_shown(error->kind));
    zeno_test_puts(": ");
    zeno_test_puts(error->message);
    zeno_test_end_report(line, col, note);
}

/* An error left the test, which fails it. It is shown where it was
 * raised, and it is over. */
static void zeno_test_uncaught(void) {
    zeno_test_begin_report();
    zeno_test_puts("uncaught ");
    zeno_test_puts(zeno_kind_shown(zeno_raised->kind));
    zeno_test_puts(": ");
    zeno_test_puts(zeno_raised->message);
    zeno_test_end_report(zeno_raised_line, zeno_raised_col, NULL);
    ZENO_TEST_DROP_RAISED
    zeno_raised = NULL;
}

/* What the room holds, and how much did not fit. */
static void zeno_tests_print_reports(void) {
    zeno_out(zeno_test_room, zeno_test_used);
    zeno_test_used = 0;
    if (zeno_tests_left_out > 0) {
        zeno_test_out("\n");
        zeno_test_out_number(zeno_tests_left_out);
        zeno_test_out(zeno_tests_left_out == 1 ? " more failure" : " more failures");
        zeno_test_out(" did not fit in the room a test build keeps for them. `zeno test` shows them all.\n");
        zeno_tests_left_out = 0;
    }
    zeno_out_now();
}

/* A program that stops before its last test is over (a fault, or
 * `program.exit`) still prints the reports of the tests before. */
static void zeno_tests_stopped(void) {
    if (zeno_tests_over) return;
    zeno_tests_over = true;
    zeno_tests_print_reports();
}

/* The program's own statements are over, and its tests begin. */
static void zeno_tests_begin(void) {
    zeno_tests_over = false;
    atexit(zeno_tests_stopped);
#ifdef ZENO_TEST_MARKS
    const char *marks = getenv("ZENO_TEST_MARKS");
    if (marks != NULL && *marks != '\0') zeno_test_marks = fopen(marks, "wb");
    zeno_test_mark('s');
#endif /* ZENO_TEST_MARKS */
}

static void zeno_test_begin(const char *name) {
    zeno_test_running = name;
    zeno_test_failing = false;
}

/* The test is over: say how it went, as `zeno test` does. */
static void zeno_test_end(void) {
    zeno_tests_run++;
    if (zeno_test_failing) zeno_tests_failed++;
    zeno_test_out(zeno_test_failing ? "  FAILED  " : "  ok      ");
    zeno_test_out(zeno_test_running);
    zeno_test_out("\n");
#ifdef ZENO_TEST_MARKS
    zeno_test_mark(zeno_test_failing ? 'f' : 'p');
#endif /* ZENO_TEST_MARKS */
}

/* Every test is over: the reports, and the totals when nobody counts
 * them. The program ends with 1 if any test failed. */
static int zeno_tests_end(void) {
    zeno_tests_over = true;
    zeno_tests_print_reports();
#ifdef ZENO_TEST_MARKS
    if (zeno_test_marks != NULL) {
        zeno_test_mark('e');
        fclose(zeno_test_marks);
        zeno_test_marks = NULL;
        return zeno_tests_failed > 0;
    }
#endif /* ZENO_TEST_MARKS */
    if (zeno_tests_run == 0) {
        zeno_test_out("\nNo tests were run.\n");
    } else {
        zeno_test_out("\n");
        zeno_test_out_number(zeno_tests_run);
        zeno_test_out(zeno_tests_run == 1 ? " test: " : " tests: ");
        zeno_test_out_number(zeno_tests_run - zeno_tests_failed);
        zeno_test_out(" passed, ");
        zeno_test_out_number(zeno_tests_failed);
        zeno_test_out(" failed.\n");
    }
    zeno_out_now();
    return zeno_tests_failed > 0;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit_c::{self, Build};
    use crate::{checker, emit_llvm, interpreter, lexer, modules, parser, value};
    use std::process::Command;

    /// What `zeno test test.zeno` prints for this file, as `main.rs`
    /// prints it, with the line of totals that a program built for its
    /// tests prints when it runs by itself.
    fn interpreted(source: &str) -> String {
        let program = parser::parse(lexer::lex(source).expect("it reads")).expect("it parses");
        let (program, errors, _) = checker::checked(program, checker::Rules::default());
        assert!(errors.is_empty(), "the checker refused the program: {:?}", errors[0].message);
        let (output, tested) = interpreter::run_tests(&program, true, value::testing::outside(), None);
        let tested = tested.expect("the program ran to its tests");
        let files = [modules::File { path: "test.zeno".to_string(), source: source.to_string() }];
        let mut out = format!("test.zeno\n{output}");
        for test in &tested {
            if let Some(failure) = &test.failure {
                out += &format!("\nFAILED: {}\n{}", test.name, crate::shown(&files, failure));
            }
        }
        let failed = tested.iter().filter(|test| test.failure.is_some()).count();
        out += &match tested.len() {
            0 => "\nNo tests were run.\n".to_string(),
            n => format!("\n{}: {} passed, {failed} failed.\n", crate::error::counted(n, "test"), n - failed),
        };
        out
    }

    /// The file built for its tests, the way asked, and run by itself:
    /// what it printed, what it wrote as errors, and the number it ended
    /// with. `None` if this machine has no compiler for it.
    fn compiled(name: &str, source: &str, build: Build, llvm: bool) -> Option<(String, String, i32)> {
        let program = parser::parse(lexer::lex(source).expect("it reads")).expect("it parses");
        let (program, tests) = with_tests(program, None).expect("the tests are made functions");
        let (program, errors, types) = checker::checked(program, build.rules());
        assert!(errors.is_empty(), "the build's checker refused the program: {:?}", errors[0].message);
        let built = TestBuild { tests, sources: vec![source.to_string()], board: false };
        let dir = std::env::temp_dir().join(format!("zeno-test-build-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary folder");
        let binary = dir.join(emit_c::program_file("tests"));
        let mut command = match llvm {
            false => {
                let c = emit_c::emit_tests(&program, &types, "test.zeno", build, emit_c::Options::default(), &built)
                    .unwrap_or_else(|error| panic!("the C build refused it: {}", error.message));
                std::fs::write(dir.join("tests.c"), c).expect("the C is written");
                let mut command = Command::new("cc");
                command.arg(dir.join("tests.c"));
                command
            }
            true => {
                let ir = emit_llvm::emit_tests(&program, &types, "test.zeno", build, true, emit_c::Options::default(), &built)
                    .unwrap_or_else(|error| panic!("the LLVM build refused it: {}", error.message));
                let runtime = emit_c::with_board_parts(&emit_llvm::runtime_for(build, false), &emit_c::board_parts_in_ir(&ir));
                let runtime = runtime + &emit_llvm::tests_runtime(&built, build);
                std::fs::write(dir.join("tests.ll"), ir).expect("the IR is written");
                std::fs::write(dir.join("tests_runtime.c"), runtime).expect("the runtime is written");
                let mut command = Command::new("clang");
                command.args(["-Wno-override-module"]).arg(dir.join("tests.ll")).arg(dir.join("tests_runtime.c"));
                command
            }
        };
        let made = command.arg("-DZENO_MEMORY_REPORT").arg("-o").arg(&binary).args(emit_c::LIBRARIES).output();
        let Ok(made) = made else {
            eprintln!("no compiler for this build here; skipping it");
            return None;
        };
        assert!(made.status.success(), "it does not compile:\n{}", String::from_utf8_lossy(&made.stderr));
        let ran = Command::new(&binary).env_remove("ZENO_TEST_MARKS").output().expect("the program runs");
        let _ = std::fs::remove_dir_all(&dir);
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        Some((text(&ran.stdout), text(&ran.stderr), ran.status.code().unwrap_or(-1)))
    }

    /// Every build of the file, C and LLVM, prints what `zeno test`
    /// prints, ends with 1 when a test failed, and with no collector
    /// gives back every block it took.
    fn same_every_way(name: &str, source: &str, builds: &[Build]) {
        let expected = interpreted(source);
        let failed = !expected.contains(" 0 failed.") && !expected.contains("No tests were run.");
        for &build in builds {
            for llvm in [false, true] {
                let way = format!("{name}-{build:?}-{}", if llvm { "llvm" } else { "c" });
                let Some((printed, errors, status)) = compiled(&way, source, build, llvm) else {
                    continue;
                };
                assert_eq!(printed, expected, "{way} prints something else\n{errors}");
                assert_eq!(status, i32::from(failed), "{way} ends with {status}\n{errors}");
                if build == Build::NoCollector {
                    assert!(errors.contains(", 0 blocks left at the end"), "{way} leaves memory:\n{errors}");
                }
            }
        }
    }

    const EVERY_BUILD: &[Build] = &[Build::Ordinary, Build::Firmware, Build::NoCollector];

    #[test]
    fn a_test_build_prints_what_zeno_test_prints() {
        same_every_way(
            "plain",
            "function half(n: int) -> int raises ValueError {\n    if n % 2 == 1 {\n        raise ValueError(\"odd\")\n    }\n    \
             return n // 2\n}\nvar calls = 0\nprint(\"the program\")\n\
             test \"even\" {\n    calls += 1\n    assert half(4) == 2\n    print(\"in the test\")\n}\n\
             test \"wrong\" {\n    assert half(4) == 3\n    print(\"never\")\n}\n\
             test \"odd\" {\n    assert raises ValueError {\n        half(3)\n    }\n}\n\
             test \"nothing raised\" {\n    assert raises ValueError {\n        half(2)\n    }\n}\n\
             test \"another kind\" {\n    assert raises KeyError {\n        half(3)\n    }\n}\n\
             test \"any kind\" {\n    assert raises Error {\n        half(3)\n    }\n}\n\
             test \"an error\" {\n    half(5)\n}\n\
             test \"plain\" {\n    assert calls > 0 and not false\n    assert \"a\" in [\"b\"]\n}\n\
             test \"with a message\" {\n    let xs = [1, 2]\n    assert len(xs) == 3, f\"xs is {xs}\"\n}\n\
             test \"a message alone\" {\n    assert calls == 5 or false, \"calls is not five\"\n}\n\
             test \"none\" {\n    let x: int or none = 3\n    assert x is none\n}\n\
             test \"not none\" {\n\tlet y: str or none = none\n\tassert y is not none\n}\n\
             test \"compared\" {\n    assert [1, 2] != [1, 2]\n}\n",
            EVERY_BUILD,
        );
    }

    #[test]
    fn a_failed_assert_is_caught_by_no_except() {
        // An ordinary build: a firmware build refuses a `try` that
        // nothing in it can raise into, as it refuses it in a program.
        same_every_way(
            "not-caught",
            "test \"not caught\" {\n    try {\n        assert 1 == 2\n    } except Error {\n        print(\"caught\")\n    }\n}\n\
             test \"after\" {\n    assert true\n}\n",
            &[Build::Ordinary],
        );
    }

    #[test]
    fn a_failed_assert_frees_what_the_test_owns() {
        // Lists made in the test, given to a loop and held in a block,
        // when an `assert` leaves the test half way.
        same_every_way(
            "owned",
            "let kept = [[1], [2, 3]]\n\
             function make(n: int) -> list[int] {\n    var out: list[int] = []\n    for i in range(n) {\n        out.append(i)\n    }\n    \
             return out\n}\n\
             test \"in a loop\" {\n    let made = make(3)\n    for item in kept {\n        let copy = make(len(item))\n        \
             assert len(copy) == 1, f\"made {copy} beside {made}\"\n    }\n}\n\
             test \"in a raises\" {\n    let first = make(2)\n    assert raises IndexError {\n        let inner = make(1)\n        \
             print(inner[0])\n    }\n    assert first == [0], \"first\"\n}\n\
             test \"passes\" {\n    let made = make(2)\n    assert made == [0, 1]\n    assert kept[1] == [2, 3]\n}\n",
            &[Build::Ordinary, Build::NoCollector],
        );
    }

    #[test]
    fn several_values_in_a_test() {
        // Where item 10 meets 13: several values received, swapped and
        // raised past in a test, with a list among them (decision 57).
        same_every_way(
            "several",
            r#"function divide(a: int, b: int) -> (int, int) raises ValueError {
    if b == 0 {
        raise ValueError("no number divides by 0")
    }
    return a // b, a % b
}

function names() -> (str, list[int]) {
    return "seven", [7, 2]
}

test "seven by two" {
    let q, r = divide(7, 2)
    assert q == 3, f"seven by two gave {q} and {r}"
    assert r == 1
}

test "a name and a list" {
    let name, xs = names()
    assert name == "seven" and len(xs) == 2, f"{name} {xs}"
}

test "by zero" {
    assert raises ValueError {
        let q, r = divide(7, 0)
        print(q, r)
    }
}

test "fails with a message" {
    var low, high = divide(9, 4)
    low, high = high, low
    assert low == 2, f"after the swap low is {low} and high is {high}"
}

test "raises the wrong kind" {
    assert raises KeyError {
        let q, r = divide(1, 0)
    }
}
"#,
            EVERY_BUILD,
        );
    }

    #[test]
    fn a_report_that_does_not_fit_is_left_out_and_counted() {
        let mut source = format!("let long = \"{}\"\n", "x".repeat(300));
        for n in 0..40 {
            source += &format!("test \"number {n}\" {{\n    assert long == \"y\", \"failure {n}\"\n}}\n");
        }
        let Some((printed, _, status)) = compiled("room", &source, Build::Ordinary, false) else {
            return;
        };
        assert_eq!(status, 1);
        let shown = printed.matches("\nFAILED: ").count();
        assert!(shown > 5 && shown < 40, "{shown} reports were shown");
        assert!(
            printed.contains(&format!(
                "\n{} more failures did not fit in the room a test build keeps for them. `zeno test` shows them all.\n",
                40 - shown
            )),
            "{printed}"
        );
        assert!(printed.ends_with("\n40 tests: 0 passed, 40 failed.\n"));
    }

    #[test]
    fn the_tests_of_the_first_file_become_functions_at_its_end() {
        let program = parser::parse(
            lexer::lex("test \"a\" {\n    assert true\n}\nlet x = 1\ntest \"b c\" {\n    assert x == 1\n}\n").unwrap(),
        )
        .unwrap();
        let (made, tests) = with_tests(program.clone(), None).unwrap();
        assert_eq!(tests, [("test 1".to_string(), "a".to_string()), ("test 2".to_string(), "b c".to_string())]);
        let kinds: Vec<String> = made
            .statements
            .iter()
            .map(|statement| match &statement.kind {
                StmtKind::Function(function) => format!("{} raises {:?}", function.name, function.raises),
                StmtKind::Let { name, .. } => format!("let {name}"),
                _ => "other".to_string(),
            })
            .collect();
        // After everything else, so that a test sees every variable.
        assert_eq!(kinds, ["let x", "test 1 raises [\"Error\"]", "test 2 raises [\"Error\"]"]);
        // `--only` keeps the tests that have the text in their name.
        let (_, tests) = with_tests(program, Some("b")).unwrap();
        assert_eq!(tests, [("test 1".to_string(), "b c".to_string())]);
    }

    #[test]
    fn an_assert_in_a_function_of_a_test_is_refused() {
        let refused = |source: &str| {
            let program = parser::parse(lexer::lex(source).unwrap()).unwrap();
            with_tests(program, None).map(|_| ()).map_err(|error| (error.message, error.span.line))
        };
        let message = "under `zeno test --build` an `assert` stands in the test itself, and not in a function written \
                       inside it. Let the function give back what it found, and `assert` that in the test"
            .to_string();
        assert_eq!(
            refused("test \"f\" {\n    let check = function(n: int) -> bool {\n        assert n > 0\n        return true\n    }\n}\n"),
            Err((message.clone(), 3))
        );
        assert_eq!(
            refused("test \"g\" {\n    function inner() {\n        assert raises ValueError {\n        }\n    }\n    inner()\n}\n"),
            Err((message, 3))
        );
        // In the test itself, at any depth of blocks, it is the test's.
        assert_eq!(refused("test \"h\" {\n    if true {\n        for i in range(2) {\n            assert i < 2\n        }\n    }\n}\n"), Ok(()));
    }

    #[test]
    fn text_is_written_into_c_as_it_is() {
        assert_eq!(c_text("a \"b\" \\ c\td??=\n"), "\"a \\\"b\\\" \\\\ c\\td\\?\\?=\\n\"\n    \"\"");
        assert_eq!(c_text("é"), "\"\\303\\251\"");
    }
}
