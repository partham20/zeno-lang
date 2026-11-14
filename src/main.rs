//! The `zeno` command-line tool.
//!
//! The compiler stages get added here one at a time. `zeno tokens`,
//! `zeno parse` and `zeno check` show what the lexer, the parser and the
//! type checker find. `zeno run` checks a program and then runs it, with
//! the interpreter or, given `--vm`, by compiling it to bytecode for the
//! VM. `zeno bytecode` shows that bytecode. `zeno build` compiles to C.

mod ast;
mod builtins;
mod bytecode;
mod checker;
mod emit_bytecode;
mod emit_c;
mod error;
mod interpreter;
mod lexer;
mod ops;
mod parser;
mod printer;
mod token;
mod units;
mod value;
mod vm;

use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage() {
    println!("zeno {VERSION}");
    println!("Every journey is halfway done. Forever.");
    println!();
    println!("Usage:");
    println!("  zeno run <file.zeno>      check a program, then run it");
    println!("  zeno check <file.zeno>    check a program for mistakes without running it");
    println!("  zeno tokens <file.zeno>   show the tokens the lexer finds");
    println!("  zeno parse <file.zeno>    show the syntax tree the parser builds");
    println!("  zeno bytecode <file.zeno> show the bytecode `zeno run --vm` runs");
    println!("  zeno build <file.zeno>    compile to C, then to a program you can run");
    println!("  zeno --version            print the version");
    println!("  zeno --help               print this message");
    println!();
    println!("Options:");
    println!("  --strict                 with check or run: every type must be known");
    println!("                           before the program runs, as firmware needs.");
    println!("                           `build` always works this way");
    println!("  --vm                     with run: compile the program to bytecode and");
    println!("                           run that, in place of walking the syntax tree.");
    println!("                           The program does the same, faster");
    println!("  --firmware               with build: errors travel as values that every");
    println!("                           caller tests, with no setjmp or longjmp, as");
    println!("                           coding standards for firmware require");
    println!("  --no-collector           with build: a firmware build with no garbage");
    println!("                           collector in it. Every list and object must");
    println!("                           be `owned`, and is freed where its owner's");
    println!("                           block ends");
    println!("  --memory                 with build: the program reports, as it ends,");
    println!("                           how much memory it used and how often the");
    println!("                           garbage collector ran");
}

/// Read a source file, or print why it could not be read.
fn read(path: &str) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(source) => Some(source),
        Err(err) => {
            eprintln!("zeno: cannot read `{path}`: {err}");
            None
        }
    }
}

/// Run the lexer, printing any errors it finds.
fn lex(path: &str, source: &str) -> Option<Vec<token::Token>> {
    match lexer::lex(source) {
        Ok(tokens) => Some(tokens),
        Err(errors) => {
            for error in &errors {
                report(path, source, error);
            }
            None
        }
    }
}

/// `zeno tokens <file>`: run the lexer and print one token per line.
fn tokens(path: &str) -> Option<()> {
    let source = read(path)?;
    for token in lex(path, &source)? {
        println!("{:>4}:{:<3} {:?}", token.span.line, token.span.col, token.kind);
    }
    Some(())
}

/// Run the lexer and the parser, printing any errors they find.
fn load(path: &str) -> Option<(String, ast::Program)> {
    let source = read(path)?;
    let tokens = lex(path, &source)?;
    match parser::parse(tokens) {
        Ok(program) => Some((source, program)),
        Err(error) => {
            report(path, &source, &error);
            None
        }
    }
}

/// `zeno parse <file>`: print the syntax tree.
fn parse(path: &str) -> Option<()> {
    let (_, program) = load(path)?;
    print!("{}", printer::print(&program));
    Some(())
}

/// `zeno check <file>`: run the type checker, printing every error it finds.
fn check(path: &str, strict: bool) -> Option<(String, ast::Program)> {
    let (source, program) = load(path)?;
    let errors = checker::check(&program, strict);
    for error in &errors {
        report(path, &source, error);
    }
    if errors.is_empty() {
        Some((source, program))
    } else {
        let s = if errors.len() == 1 { "" } else { "s" };
        eprintln!("{} error{s} found. Nothing was run.", errors.len());
        None
    }
}

/// `zeno bytecode <file>`: print the bytecode the program compiles to.
fn bytecode(path: &str) -> Option<()> {
    let (_, program) = load(path)?;
    print!("{}", bytecode::print(&emit_bytecode::compile(&program)));
    Some(())
}

/// `zeno run <file>`: check the program, and if it is sound, run it.
///
/// There are two engines to run it with. Both give the same result.
fn run(path: &str, strict: bool, on_vm: bool) -> Option<()> {
    let (source, program) = check(path, strict)?;
    let (_, result) = match on_vm {
        true => vm::run(&program, false),
        false => interpreter::run(&program, false),
    };
    match result {
        Ok(()) => Some(()),
        Err(error) => {
            report(path, &source, &error);
            None
        }
    }
}

/// `zeno build <file>`: compile the program to C, then hand the C to the
/// machine's C compiler to make a program that runs by itself.
///
/// `hello.zeno` becomes `hello.c` and `hello`, in the folder you are in. The
/// C file is kept, because reading it is the best way to see what the
/// compiler did.
fn build(path: &str, memory_report: bool, kind: emit_c::Build) -> Option<()> {
    // Compiled code has no room for values of unknown type, so `build`
    // always checks in strict mode.
    let (source, program) = load(path)?;
    let (errors, types) = checker::check_with_types(&program, true);
    for error in &errors {
        report(path, &source, error);
    }
    if !errors.is_empty() {
        let s = if errors.len() == 1 { "" } else { "s" };
        eprintln!("{} error{s} found. Nothing was built.", errors.len());
        return None;
    }

    let c = match emit_c::emit(&program, &types, path, kind) {
        Ok(c) => c,
        Err(error) => {
            report(path, &source, &error);
            return None;
        }
    };
    let name = Path::new(path).file_stem().and_then(|stem| stem.to_str()).unwrap_or("program");
    let c_file = format!("{name}.c");
    if let Err(err) = fs::write(&c_file, c) {
        eprintln!("zeno: cannot write `{c_file}`: {err}");
        return None;
    }
    println!("Wrote {c_file}");

    // `CC` is the usual way to say which C compiler to use; `cc` is the
    // usual name for whichever one is installed.
    let compiler = env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let mut command = Command::new(&compiler);
    if memory_report {
        // Turns on a few lines in the runtime that say, as the program
        // ends, how much memory it used and how often the collector ran.
        command.arg("-DZENO_MEMORY_REPORT");
    }
    match command.args(["-O2", "-o", name, &c_file, "-lm"]).status() {
        Ok(status) if status.success() => {
            println!("Built {name}. Run it with ./{name}");
            Some(())
        }
        Ok(_) => {
            eprintln!("zeno: `{compiler}` could not compile `{c_file}`. That is a bug in Zeno, not in your program");
            None
        }
        Err(_) => {
            eprintln!("zeno: no C compiler found (tried `{compiler}`), so only the C was written.");
            eprintln!("      Install one, or set CC, then run: cc -O2 -o {name} {c_file} -lm");
            None
        }
    }
}

/// Print an error with the offending line and a caret under the exact spot:
///
/// ```text
/// error: Zeno has no semicolons. A line break ends a statement
///   --> main.zeno:3:10
///    |
///  3 | let x = 1;
///    |          ^
/// ```
fn report(path: &str, source: &str, error: &error::Error) {
    let (line, col) = (error.span.line, error.span.col);
    let text = source.lines().nth(line - 1).unwrap_or("");
    let gutter = line.to_string().len();

    eprintln!("error: {}", error.message);
    eprintln!("{:gutter$}--> {path}:{line}:{col}", "");
    eprintln!("{:gutter$} |", "");
    eprintln!("{line} | {text}");
    // Copy tabs from the source line into the padding so the caret lines up.
    let pad: String = text
        .chars()
        .take(col - 1)
        .map(|c| if c == '\t' { '\t' } else { ' ' })
        .collect();
    eprintln!("{:gutter$} | {pad}^", "");
    if let Some(note) = &error.note {
        for line in note.lines() {
            eprintln!("{:gutter$} = note: {line}", "");
        }
    }
    eprintln!();
}

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    // `--strict` may appear anywhere after the command.
    let strict = args.iter().any(|arg| arg == "--strict");
    args.retain(|arg| arg != "--strict");
    // `--memory` makes a built program report how much memory it used.
    let memory_report = args.iter().any(|arg| arg == "--memory");
    args.retain(|arg| arg != "--memory");
    // `--firmware` builds C that firmware coding standards accept.
    let firmware = args.iter().any(|arg| arg == "--firmware");
    args.retain(|arg| arg != "--firmware");
    // `--no-collector` is a firmware build that also leaves the garbage
    // collector out. (It has to be a firmware build: when an error jumps,
    // it skips the lines that free memory, and with no collector nothing
    // would ever free it.)
    let no_collector = args.iter().any(|arg| arg == "--no-collector");
    args.retain(|arg| arg != "--no-collector");
    // `--vm` runs a program as bytecode.
    let on_vm = args.iter().any(|arg| arg == "--vm");
    args.retain(|arg| arg != "--vm");
    let kind = match (no_collector, firmware) {
        (true, _) => emit_c::Build::NoCollector,
        (false, true) => emit_c::Build::Firmware,
        (false, false) => emit_c::Build::Ordinary,
    };

    match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") | Some("help") => {
            usage();
            ExitCode::SUCCESS
        }
        Some("-V") | Some("--version") | Some("version") => {
            println!("zeno {VERSION}");
            ExitCode::SUCCESS
        }
        Some(cmd @ ("tokens" | "parse" | "bytecode" | "check" | "run")) => {
            let Some(path) = args.get(1) else {
                eprintln!("zeno: `{cmd}` needs a file, e.g. `zeno {cmd} examples/hello.zeno`");
                return ExitCode::FAILURE;
            };
            let done = match cmd {
                "tokens" => tokens(path),
                "parse" => parse(path),
                "bytecode" => bytecode(path),
                "check" => check(path, strict).map(|_| println!("No errors found.")),
                _ => run(path, strict, on_vm),
            };
            match done {
                Some(()) => ExitCode::SUCCESS,
                None => ExitCode::FAILURE,
            }
        }
        Some("build") => {
            let Some(path) = args.get(1) else {
                eprintln!("zeno: `build` needs a file, e.g. `zeno build examples/hello.zeno`");
                return ExitCode::FAILURE;
            };
            match build(path, memory_report, kind) {
                Some(()) => ExitCode::SUCCESS,
                None => ExitCode::FAILURE,
            }
        }
        Some(other) => {
            eprintln!("zeno: unknown command `{other}`\n");
            usage();
            ExitCode::FAILURE
        }
    }
}
