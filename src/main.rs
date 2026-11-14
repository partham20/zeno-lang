//! The `zeno` command-line tool.
//!
//! Right now this only prints a version and usage message.
//! The compiler stages (lexer, parser, interpreter, ...) get added here one at a time.

mod lexer;
mod token;

use std::env;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage() {
    println!("zeno {VERSION}");
    println!("Every journey is halfway done. Forever.");
    println!();
    println!("Usage:");
    println!("  zeno run <file.zeno>     run a program (not implemented yet)");
    println!("  zeno build <file.zeno>   compile to C (not implemented yet)");
    println!("  zeno --version           print the version");
    println!("  zeno --help              print this message");
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
