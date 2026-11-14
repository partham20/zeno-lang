//! The `zeno` command-line tool.
//!
//! The compiler stages get added here one at a time. So far there is only
//! the lexer, which `zeno tokens <file>` lets you watch at work.

mod lexer;
mod token;

use std::env;
use std::fs;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage() {
    println!("zeno {VERSION}");
    println!("Every journey is halfway done. Forever.");
    println!();
    println!("Usage:");
    println!("  zeno tokens <file.zeno>  show the tokens the lexer finds");
    println!("  zeno run <file.zeno>     run a program (not implemented yet)");
    println!("  zeno build <file.zeno>   compile to C (not implemented yet)");
    println!("  zeno --version           print the version");
    println!("  zeno --help              print this message");
}

/// `zeno tokens <file>`: run the lexer and print one token per line.
fn tokens(path: &str) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("zeno: cannot read `{path}`: {err}");
            return ExitCode::FAILURE;
        }
    };

    match lexer::lex(&source) {
        Ok(tokens) => {
            for token in tokens {
                println!("{:>4}:{:<3} {:?}", token.span.line, token.span.col, token.kind);
            }
            ExitCode::SUCCESS
        }
        Err(errors) => {
            for error in &errors {
                report(path, &source, error);
            }
            ExitCode::FAILURE
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
fn report(path: &str, source: &str, error: &lexer::LexError) {
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
    let args: Vec<String> = env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") | Some("help") => {
            usage();
            ExitCode::SUCCESS
        }
        Some("-V") | Some("--version") | Some("version") => {
            println!("zeno {VERSION}");
            ExitCode::SUCCESS
        }
        Some("tokens") => match args.get(1) {
            Some(path) => tokens(path),
            None => {
                eprintln!("zeno: `tokens` needs a file, e.g. `zeno tokens examples/hello.zeno`");
                ExitCode::FAILURE
            }
        },
        Some(cmd @ ("run" | "build")) => {
            eprintln!("zeno: `{cmd}` is not implemented yet. Achilles is still catching the tortoise.");
            ExitCode::FAILURE
        }
        Some(other) => {
            eprintln!("zeno: unknown command `{other}`\n");
            usage();
            ExitCode::FAILURE
        }
    }
}
