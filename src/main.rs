//! The `zeno` command-line tool.
//!
//! The compiler stages get added here one at a time. So far there are the
//! lexer, the parser, the type checker and the interpreter. `zeno tokens`,
//! `zeno parse` and `zeno check` show what the first three find, and
//! `zeno run` checks a program and then runs it.

mod ast;
mod builtins;
mod checker;
mod error;
mod interpreter;
mod lexer;
mod parser;
mod printer;
mod token;
mod units;
mod value;

use std::env;
use std::fs;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage() {
    println!("zeno {VERSION}");
    println!("Every journey is halfway done. Forever.");
    println!();
    println!("Usage:");
    println!("  zeno run <file.zeno>     check a program, then run it");
    println!("  zeno check <file.zeno>   check a program for mistakes without running it");
    println!("  zeno tokens <file.zeno>  show the tokens the lexer finds");
    println!("  zeno parse <file.zeno>   show the syntax tree the parser builds");
    println!("  zeno build <file.zeno>   compile to C (not implemented yet)");
    println!("  zeno --version           print the version");
    println!("  zeno --help              print this message");
    println!();
    println!("Options:");
    println!("  --strict                 with check or run: every type must be known");
    println!("                           before the program runs, as firmware needs");
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

/// `zeno run <file>`: check the program, and if it is sound, run it.
fn run(path: &str, strict: bool) -> Option<()> {
    let (source, program) = check(path, strict)?;
    let (_, result) = interpreter::run(&program, false);
    match result {
        Ok(()) => Some(()),
        Err(error) => {
            report(path, &source, &error);
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
    eprintln!();
}

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    // `--strict` may appear anywhere after the command.
    let strict = args.iter().any(|arg| arg == "--strict");
    args.retain(|arg| arg != "--strict");

    match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") | Some("help") => {
            usage();
            ExitCode::SUCCESS
        }
        Some("-V") | Some("--version") | Some("version") => {
            println!("zeno {VERSION}");
            ExitCode::SUCCESS
        }
        Some(cmd @ ("tokens" | "parse" | "check" | "run")) => {
            let Some(path) = args.get(1) else {
                eprintln!("zeno: `{cmd}` needs a file, e.g. `zeno {cmd} examples/hello.zeno`");
                return ExitCode::FAILURE;
            };
            let done = match cmd {
                "tokens" => tokens(path),
                "parse" => parse(path),
                "check" => check(path, strict).map(|_| println!("No errors found.")),
                _ => run(path, strict),
            };
            match done {
                Some(()) => ExitCode::SUCCESS,
                None => ExitCode::FAILURE,
            }
        }
        Some("build") => {
            eprintln!("zeno: `build` is not implemented yet. Achilles is still catching the tortoise.");
            ExitCode::FAILURE
        }
        Some(other) => {
            eprintln!("zeno: unknown command `{other}`\n");
            usage();
            ExitCode::FAILURE
        }
    }
}
