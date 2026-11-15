//! The `zeno` command-line tool.
//!
//! The compiler stages get added here one at a time. `zeno tokens`,
//! `zeno parse` and `zeno check` show what the lexer, the parser and the
//! type checker find. `zeno join` shows a program of several files as
//! the one program the later stages get. `zeno run` checks a program and then runs it, with
//! the interpreter or, given `--vm`, by compiling it to bytecode for the
//! VM. `zeno bytecode` shows that bytecode. `zeno build` compiles to C,
//! or with `--llvm` to LLVM IR.

mod ast;
mod board;
mod builtins;
mod bytecode;
mod checker;
mod emit_bytecode;
mod emit_c;
mod emit_llvm;
mod error;
mod format;
mod interpreter;
mod json;
mod ints;
mod lexer;
mod lsp;
mod modules;
mod ops;
mod package;
mod parser;
mod printer;
mod token;
mod units;
mod value;
mod vm;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage() {
    println!("zeno {VERSION}");
    println!("Every journey is halfway done. Forever.");
    println!();
    println!("Usage:");
    println!("  zeno run <file.zeno>      check a program, then run it. Words after the");
    println!("                            file are the program's own arguments");
    println!("  zeno check <file.zeno>    check a program for mistakes without running it");
    println!("  zeno tokens <file.zeno>   show the tokens the lexer finds");
    println!("  zeno parse <file.zeno>    show the syntax tree the parser builds");
    println!("  zeno join <file.zeno>     show the one program that the file and the");
    println!("                            files it imports are joined into");
    println!("  zeno bytecode <file.zeno> show the bytecode `zeno run --vm` runs");
    println!("  zeno build <file.zeno>    compile to C, then to a program you can run");
    println!("  zeno test [file|folder]   run the tests written in a file: each");
    println!("                            `test \"name\" {{ ... }}`. A folder stands for");
    println!("                            the files in it, and no name for this folder");
    println!("  zeno format <file.zeno>   lay the file out the way Zeno lays programs");
    println!("                            out. Several files can be named, and a");
    println!("                            folder stands for the files in it");
    println!("  zeno lsp                  the language server: an editor starts this to");
    println!("                            show mistakes as you type (docs/lsp.md)");
    println!("  zeno new <name>           make a folder with a new program in it: a");
    println!("                            `main.zeno`, a module with a test, and a");
    println!("                            `zeno.package`");
    println!("  zeno new --package <name> make a folder with a new package in it: a");
    println!("                            folder of modules for other programs to need");
    println!("  zeno fetch [folder]       get the packages that the program in this");
    println!("                            folder (or in that one) needs, as the");
    println!("                            `needs` lines of its `zeno.package` list them");
    println!("  zeno --version            print the version");
    println!("  zeno --help               print this message");
    println!();
    println!("Options:");
    println!("  --strict                 with check or run: every type must be known");
    println!("                           before the program runs, as firmware needs.");
    println!("                           `build` always works this way");
    println!("  --no-collector           with check: check as `build --no-collector`");
    println!("                           would, without building");
    println!("  --vm                     with run: compile the program to bytecode and");
    println!("                           run that, in place of walking the syntax tree.");
    println!("                           The program does the same, faster");
    println!("  --check                  with format: change nothing, and say which");
    println!("                           files are not laid out that way");
    println!("  --package                with new: make a package, a folder of modules");
    println!("                           for other programs to need, with no `main.zeno`");
    println!("  --update                 with fetch: look every version up again, and");
    println!("                           not at the commits `zeno.lock` has pinned");
    println!("  --firmware               with build: errors travel as values that every");
    println!("                           caller tests, with no setjmp or longjmp, as");
    println!("                           coding standards for firmware require");
    println!("  --no-collector           with build: a firmware build with no garbage");
    println!("                           collector in it. Every list and object has");
    println!("                           one owner, and is freed where its owner's");
    println!("                           block ends. Memory comes from a pool of a");
    println!("                           fixed size, not from malloc");
    println!("  --llvm                   with build: compile to LLVM IR in place of C,");
    println!("                           and hand that to clang. It goes with");
    println!("                           --firmware, --no-collector and --memory");
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

/// The one file of a program that is written in one file, as `report`
/// wants it.
fn alone(path: &str, source: &str) -> [modules::File; 1] {
    [modules::File { path: path.to_string(), source: source.to_string() }]
}

/// Run the lexer, printing any errors it finds.
fn lex(path: &str, source: &str) -> Option<Vec<token::Token>> {
    match lexer::lex(source) {
        Ok(tokens) => Some(tokens),
        Err(errors) => {
            for error in &errors {
                report(&alone(path, source), error);
            }
            None
        }
    }
}

/// `zeno tokens <file>`: run the lexer and print one token per line.
fn tokens(path: &str) -> Option<()> {
    let source = read(path)?;
    for token in lex(path, &source)? {
        // `print_line` and not `println!`, so that `zeno tokens f | head`
        // ends quietly when `head` has seen enough.
        value::print_line(&format!("{:>4}:{:<3} {:?}", token.span.line, token.span.col, token.kind));
    }
    Some(())
}

/// Read a whole program: the file, the files it imports and the files
/// those import, joined into one program. Any errors found on the way
/// are printed. The files come back too, because an error found later
/// is shown with the line of the file it is in.
fn load(path: &str) -> Option<(Vec<modules::File>, ast::Program)> {
    let source = read(path)?;
    let loaded = modules::load(path, source, &|path| fs::read_to_string(path).ok());
    match loaded.program {
        Ok(program) => Some((loaded.files, program)),
        Err(errors) => {
            for error in &errors {
                report(&loaded.files, error);
            }
            None
        }
    }
}

/// `zeno parse <file>`: print the syntax tree of that one file. Its
/// imports are shown as the lines they are, and not followed.
fn parse(path: &str) -> Option<()> {
    let source = read(path)?;
    let tokens = lex(path, &source)?;
    match parser::parse(tokens) {
        Ok(program) => {
            printer::print(&program).lines().for_each(value::print_line);
            Some(())
        }
        Err(error) => {
            report(&alone(path, &source), &error);
            None
        }
    }
}

/// `zeno join <file>`: print the program as the stages after the parser
/// get it, with the files it imports joined to it and their names made
/// whole (`modules.rs`).
fn join(path: &str) -> Option<()> {
    let (_, program) = load(path)?;
    printer::print(&program).lines().for_each(value::print_line);
    Some(())
}

/// The `.zeno` files in a folder and the folders below it. The folder
/// `packages` is left out: what `zeno fetch` put there is somebody
/// else's, and is theirs to lay out.
fn zeno_files(folder: &Path, found: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(folder) else {
        return eprintln!("zeno: cannot read the folder `{}`", folder.display());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !path.ends_with(package::FOLDER) {
                zeno_files(&path, found);
            }
        } else if path.extension().is_some_and(|extension| extension == "zeno") {
            found.push(path.display().to_string());
        }
    }
}

/// `zeno format <file>`: write the file again in Zeno's one layout.
/// With `check`, nothing is written: the answer is whether it would be.
/// `None` if the file has a mistake, or (with `check`) is not laid out so.
fn format(path: &str, check: bool) -> Option<()> {
    let source = read(path)?;
    let formatted = match format::format(&source) {
        Ok(formatted) => formatted,
        Err(errors) => {
            for error in &errors {
                report(&alone(path, &source), error);
            }
            return None;
        }
    };
    if formatted == source {
        if !check {
            println!("{path} is laid out already");
        }
        return Some(());
    }
    if check {
        println!("{path} is not laid out as `zeno format` lays it out");
        return None;
    }
    match fs::write(path, formatted) {
        Ok(()) => {
            println!("Formatted {path}");
            Some(())
        }
        Err(err) => {
            eprintln!("zeno: cannot write `{path}`: {err}");
            None
        }
    }
}

/// The folder of the program that the file at `path` belongs to: the
/// nearest one, from the file's own upward, that has a `zeno.package`
/// or a `main.zeno` in it. With neither, the file's own folder. This
/// is where the file's imports are looked for, and the language
/// server finds it the same way.
fn program_folder(path: &str) -> PathBuf {
    let own = Path::new(path).parent().unwrap_or(Path::new("")).to_path_buf();
    let mut folder = own.clone();
    loop {
        if folder.join(package::FILE).is_file() || folder.join("main.zeno").is_file() {
            return folder;
        }
        // The empty path is the folder this is run in, and the last one
        // looked in: a file named `drivers/uart.zeno` is not looked for
        // above it.
        if folder.as_os_str().is_empty() || !folder.pop() {
            return own;
        }
    }
}

/// How the tests of one file went.
struct Tally {
    tests: usize,
    failed: usize,
}

/// `zeno test <file>`: run the file as `zeno run` would, and then each
/// test that is written in it. `asked` says that the file was named on
/// the command line, and not found in a folder: then a file with no
/// test in it is worth a line. `None` if the tests could not be run: a
/// mistake in the program, or an error before its tests.
fn test(path: &str, strict: bool, asked: bool) -> Option<Tally> {
    let source = read(path)?;
    // The file alone says whether it has tests. One with none is not run.
    let tokens = lex(path, &source)?;
    let has_tests = match parser::parse(tokens) {
        Ok(program) => program.statements.iter().any(|statement| matches!(statement.kind, ast::StmtKind::Test { .. })),
        Err(error) => {
            report(&alone(path, &source), &error);
            return None;
        }
    };
    if !has_tests {
        if asked {
            value::print_line(&format!("{path} has no tests. A test is written `test \"what it checks\" {{ ... }}`"));
        }
        return Some(Tally { tests: 0, failed: 0 });
    }
    let loaded = modules::load_in(&program_folder(path), path, source, &|path| fs::read_to_string(path).ok());
    let files = loaded.files;
    let program = match loaded.program {
        Ok(program) => program,
        Err(errors) => {
            errors.iter().for_each(|error| report(&files, error));
            return None;
        }
    };
    let (program, errors, _) = checker::checked(program, checker::Rules { strict, one_owner: false, firmware: false });
    if !errors.is_empty() {
        errors.iter().for_each(|error| report(&files, error));
        let s = if errors.len() == 1 { "" } else { "s" };
        eprintln!("{} error{s} found. No test of {path} was run.", errors.len());
        return None;
    }
    value::print_line(path);
    let (_, tested) = interpreter::run_tests(&program, false, value::Outside::started_with(Vec::new()));
    let tested = match tested {
        Ok(tested) => tested,
        Err(error) => {
            report(&files, &error);
            eprintln!("{path} stopped with this error before its tests. None of them was run.");
            return None;
        }
    };
    // What went wrong in each test that failed, under the list.
    for test in &tested {
        if let Some(failure) = &test.failure {
            value::print_line("");
            value::print_line(&format!("FAILED: {}", test.name));
            value::print_text(&shown(&files, failure));
        }
    }
    Some(Tally { tests: tested.len(), failed: tested.iter().filter(|test| test.failure.is_some()).count() })
}

/// `zeno check <file>`: run the type checker, printing every error it finds.
fn check(path: &str, rules: checker::Rules) -> Option<(Vec<modules::File>, ast::Program)> {
    let (files, program) = load(path)?;
    // `check --no-collector` checks what that build would build.
    let program = if rules.one_owner { program.without_tests() } else { program };
    let (program, errors, _) = checker::checked(program, rules);
    for error in &errors {
        report(&files, error);
    }
    if errors.is_empty() {
        Some((files, program))
    } else {
        let s = if errors.len() == 1 { "" } else { "s" };
        eprintln!("{} error{s} found. Nothing was run.", errors.len());
        None
    }
}

/// `zeno check --no-collector <file>`: say what `zeno build
/// --no-collector` would say, and build nothing.
///
/// Most of what such a build refuses the checker finds: a type that is
/// not known, a list with two owners. A few things only the compiler
/// meets, as it works out where each value is kept (a list of numbers
/// that may be `none` is one). So this runs the compiler too and throws
/// the C away. "No errors found" then means the build will go through.
fn check_no_collector(path: &str) -> Option<()> {
    let kind = emit_c::Build::NoCollector;
    let (files, program) = load(path)?;
    let (program, mut errors, types) = checker::checked(program.without_tests(), kind.rules());
    if errors.is_empty() {
        errors.extend(emit_c::emit(&program, &types, path, kind).err());
    }
    for error in &errors {
        report(&files, error);
    }
    if errors.is_empty() {
        Some(())
    } else {
        let s = if errors.len() == 1 { "" } else { "s" };
        eprintln!("{} error{s} found. Nothing was run.", errors.len());
        None
    }
}

/// `zeno bytecode <file>`: print the bytecode the program compiles to.
fn bytecode(path: &str) -> Option<()> {
    let (_, program) = load(path)?;
    // What `zeno run --vm` runs. By then `count += 1` on a sized number
    // is `count = u8(count + 1)`, which only the checker can tell.
    let program = checker::checked(program, checker::Rules::default()).0;
    bytecode::print(&emit_bytecode::compile(&program)).lines().for_each(value::print_line);
    Some(())
}

/// `zeno run <file>`: check the program, and if it is sound, run it.
///
/// There are two engines to run it with. Both give the same result.
fn run(path: &str, strict: bool, on_vm: bool, arguments: Vec<String>) -> Option<()> {
    let (files, program) = check(path, checker::Rules { strict, one_owner: false, firmware: false })?;
    // The words after the file are the program's (`program.arguments()`),
    // and what it reads with `input()` is the keyboard.
    let outside = value::Outside::started_with(arguments);
    let (_, result) = match on_vm {
        true => vm::run_with(&program, false, outside),
        false => interpreter::run_with(&program, false, outside),
    };
    match result {
        Ok(()) => Some(()),
        Err(error) => {
            report(&files, &error);
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
    // always checks in strict mode. A build with no collector also
    // checks that every list and object has one owner.
    let (files, program) = load(path)?;
    let (program, errors, types) = checker::checked(program.without_tests(), kind.rules());
    for error in &errors {
        report(&files, error);
    }
    if !errors.is_empty() {
        let s = if errors.len() == 1 { "" } else { "s" };
        eprintln!("{} error{s} found. Nothing was built.", errors.len());
        return None;
    }

    let c = match emit_c::emit(&program, &types, path, kind) {
        Ok(c) => c,
        Err(error) => {
            report(&files, &error);
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

/// `zeno build --llvm <file>`: compile the program to LLVM IR, then hand
/// the IR to clang to make a program that runs by itself.
///
/// `hello.zeno` becomes `hello.ll`, `hello_runtime.c` and `hello`. The IR
/// holds the program. The C file is the runtime the program calls for
/// text, printing and memory, which a C build has pasted in front of it.
fn build_llvm(path: &str, memory_report: bool, kind: emit_c::Build) -> Option<()> {
    let (files, program) = load(path)?;
    let (program, errors, types) = checker::checked(program.without_tests(), kind.rules());
    for error in &errors {
        report(&files, error);
    }
    if !errors.is_empty() {
        let s = if errors.len() == 1 { "" } else { "s" };
        eprintln!("{} error{s} found. Nothing was built.", errors.len());
        return None;
    }

    let ir = match emit_llvm::emit(&program, &types, path, kind, memory_report) {
        Ok(ir) => ir,
        Err(error) => {
            report(&files, &error);
            return None;
        }
    };
    let name = Path::new(path).file_stem().and_then(|stem| stem.to_str()).unwrap_or("program");
    let (ir_file, runtime_file) = (format!("{name}.ll"), format!("{name}_runtime.c"));
    for (file, text) in [(&ir_file, ir), (&runtime_file, emit_llvm::runtime(kind))] {
        if let Err(err) = fs::write(file, text) {
            eprintln!("zeno: cannot write `{file}`: {err}");
            return None;
        }
    }
    println!("Wrote {ir_file} and {runtime_file}");

    // Only clang reads IR, so `CC` is not looked at here. `CLANG` names
    // another one, such as `clang-18`.
    let compiler = env::var("CLANG").unwrap_or_else(|_| "clang".to_string());
    // The IR does not say which processor it is for, so that the same
    // file builds anywhere. clang fills in its own and would mention
    // doing so; `-Wno-override-module` says there is no need.
    let mut flags = vec!["-O2", "-Wno-override-module"];
    if memory_report {
        // Turns on the lines of the runtime that count memory and print
        // the report. The IR asks for the report to be printed at the end.
        flags.push("-DZENO_MEMORY_REPORT");
    }
    flags.extend(["-o", name, &ir_file, &runtime_file, "-lm"]);
    match Command::new(&compiler).args(&flags).status() {
        Ok(status) if status.success() => {
            println!("Built {name}. Run it with ./{name}");
            Some(())
        }
        Ok(_) => {
            eprintln!("zeno: `{compiler}` could not compile `{ir_file}`. That is a bug in Zeno, not in your program");
            None
        }
        Err(_) => {
            eprintln!("zeno: no clang found (tried `{compiler}`), so only the IR and the runtime were written.");
            eprintln!("      Install clang 15 or later, or set CLANG, then run:");
            eprintln!("      clang {}", flags.join(" "));
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
///
/// The position says which file of the program it is in (`Span::file`),
/// and the line shown is that file's.
fn report(files: &[modules::File], error: &error::Error) {
    eprint!("{}", shown(files, error));
}

/// An error as it is shown: what is wrong, where, and the line itself
/// with a mark under the place.
fn shown(files: &[modules::File], error: &error::Error) -> String {
    let file = &files[error.span.file().min(files.len() - 1)];
    let (path, line, col) = (&file.path, error.span.line_in_file(), error.span.col);
    let text = file.source.lines().nth(line.saturating_sub(1)).unwrap_or("");
    let gutter = line.to_string().len();
    // Copy tabs from the source line into the padding so the caret lines up.
    let pad: String = text
        .chars()
        .take(col - 1)
        .map(|c| if c == '\t' { '\t' } else { ' ' })
        .collect();
    let mut out = format!("error: {}\n", error.message);
    out += &format!("{:gutter$}--> {path}:{line}:{col}\n", "");
    out += &format!("{:gutter$} |\n", "");
    out += &format!("{line} | {text}\n");
    out += &format!("{:gutter$} | {pad}^\n", "");
    if let Some(note) = &error.note {
        for line in note.lines() {
            out += &format!("{:gutter$} = note: {line}\n", "");
        }
    }
    out + "\n"
}

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    // `zeno run copy.zeno a.txt --strict`: the words after the file of a
    // `run` are the program's own, whatever they look like. Zeno's
    // options go before the file.
    let mut arguments = Vec::new();
    if args.first().is_some_and(|command| command == "run") {
        if let Some(file) = args.iter().skip(1).position(|arg| !arg.starts_with("--")) {
            arguments = args.split_off(file + 2);
        }
    }
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
    // `--llvm` builds through LLVM IR in place of C.
    let llvm = args.iter().any(|arg| arg == "--llvm");
    args.retain(|arg| arg != "--llvm");
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
        Some(cmd @ ("tokens" | "parse" | "join" | "bytecode" | "check" | "run")) => {
            let Some(path) = args.get(1) else {
                eprintln!("zeno: `{cmd}` needs a file, e.g. `zeno {cmd} examples/hello.zeno`");
                return ExitCode::FAILURE;
            };
            let done = match cmd {
                "tokens" => tokens(path),
                "parse" => parse(path),
                "join" => join(path),
                "bytecode" => bytecode(path),
                "check" if no_collector => check_no_collector(path).map(|_| println!("No errors found.")),
                "check" => check(path, checker::Rules { strict, one_owner: false, firmware: false })
                    .map(|_| println!("No errors found.")),
                _ => run(path, strict, on_vm, arguments),
            };
            match done {
                Some(()) => ExitCode::SUCCESS,
                None => ExitCode::FAILURE,
            }
        }
        Some("lsp") => lsp::run(),
        Some("format") => {
            let check = args.iter().any(|arg| arg == "--check");
            args.retain(|arg| arg != "--check");
            if args.len() < 2 {
                eprintln!("zeno: `format` needs a file or a folder, e.g. `zeno format examples/hello.zeno`");
                return ExitCode::FAILURE;
            }
            // A folder stands for the `.zeno` files in it and below it.
            let mut files = Vec::new();
            for path in &args[1..] {
                match Path::new(path).is_dir() {
                    true => zeno_files(Path::new(path), &mut files),
                    false => files.push(path.clone()),
                }
            }
            files.sort();
            // Every file is tried, so that one run says everything.
            let done = files.iter().filter(|path| format(path, check).is_none()).count();
            match done {
                0 => ExitCode::SUCCESS,
                _ => ExitCode::FAILURE,
            }
        }
        Some("test") => {
            // `zeno test`, `zeno test ring.zeno`, `zeno test examples`. A
            // folder stands for the `.zeno` files in it and below it,
            // and no name at all for the folder this is run in.
            let mut named = args[1..].to_vec();
            if named.is_empty() {
                named.push(".".to_string());
            }
            let mut files = Vec::new();
            for path in &named {
                match Path::new(path).is_dir() {
                    true => zeno_files(Path::new(path), &mut files),
                    false => files.push(path.clone()),
                }
            }
            // `./ring.zeno`, found in the folder this is run in, is `ring.zeno`.
            let mut files: Vec<String> =
                files.iter().map(|path| path.strip_prefix("./").unwrap_or(path).to_string()).collect();
            files.sort();
            let asked = |path: &String| named.contains(path);
            // Every file is tried, so that one run says everything.
            let (mut tests, mut failed, mut stopped) = (0, 0, 0);
            for path in &files {
                match test(path, strict, asked(path)) {
                    Some(tally) => {
                        tests += tally.tests;
                        failed += tally.failed;
                    }
                    None => stopped += 1,
                }
            }
            value::print_line("");
            value::print_line(&match tests {
                0 => "No tests were run.".to_string(),
                _ => format!("{}: {} passed, {} failed.", error::counted(tests, "test"), tests - failed, failed),
            });
            if stopped > 0 {
                value::print_line(&format!("{} could not be run.", error::counted(stopped, "file")));
            }
            match failed + stopped {
                0 => ExitCode::SUCCESS,
                _ => ExitCode::FAILURE,
            }
        }
        Some("new") => {
            // `zeno new blinker`, `zeno new --package filters`
            let package = args.iter().any(|arg| arg == "--package");
            args.retain(|arg| arg != "--package");
            // An option it does not have is said, wherever it stands.
            let path = match (args.iter().skip(1).find(|arg| arg.starts_with('-')), args.get(1), args.get(2)) {
                (Some(option), _, _) => option,
                (None, Some(path), None) => path,
                (None, Some(_), Some(_)) => {
                    eprintln!("zeno: `new` makes one folder, so it takes one name, e.g. `zeno new blinker`");
                    return ExitCode::FAILURE;
                }
                (None, None, _) => {
                    eprintln!("zeno: `new` needs a name for the program, e.g. `zeno new blinker`");
                    return ExitCode::FAILURE;
                }
            };
            match package::new(Path::new(path), package) {
                Ok(said) => {
                    said.iter().for_each(|line| println!("{line}"));
                    ExitCode::SUCCESS
                }
                Err(why) => {
                    eprintln!("zeno: {why}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("fetch") => {
            // `zeno fetch`, `zeno fetch --update`, `zeno fetch examples/thermostat`
            let update = args.iter().any(|arg| arg == "--update");
            args.retain(|arg| arg != "--update");
            let folder = args.get(1).map(String::as_str).unwrap_or("");
            match package::fetch(Path::new(folder), update) {
                Ok(said) => {
                    said.iter().for_each(|line| println!("{line}"));
                    ExitCode::SUCCESS
                }
                Err(why) => {
                    eprintln!("zeno: {why}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("build") => {
            let Some(path) = args.get(1) else {
                eprintln!("zeno: `build` needs a file, e.g. `zeno build examples/hello.zeno`");
                return ExitCode::FAILURE;
            };
            let built = match llvm {
                true => build_llvm(path, memory_report, kind),
                false => build(path, memory_report, kind),
            };
            match built {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of files for one test, which no other test uses.
    fn folder(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = env::temp_dir().join(format!("zeno-main-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (path, text) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().expect("a folder")).expect("the folder can be made");
            fs::write(path, text).expect("the file can be written");
        }
        root
    }

    #[test]
    fn check_with_no_collector_says_what_that_build_would_say() {
        let root = folder(
            "check-no-collector",
            &[
                // The checker passes this, and the compiler refuses it.
                ("gaps.zeno", "var readings: list[int or none] = [1, none, 3]\nprint(len(readings))\n"),
                ("fine.zeno", "let readings = [1, 2, 3]\nprint(len(readings))\n"),
            ],
        );
        let at = |name: &str| root.join(name).display().to_string();
        assert!(check(&at("gaps.zeno"), checker::Rules { strict: true, one_owner: true, firmware: true }).is_some());
        assert!(check_no_collector(&at("gaps.zeno")).is_none());
        assert!(build(&at("gaps.zeno"), false, emit_c::Build::NoCollector).is_none());
        assert!(check_no_collector(&at("fine.zeno")).is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn zeno_test_runs_the_tests_of_each_file_in_its_own_program() {
        let root = folder(
            "tests",
            &[
                ("app/main.zeno", "import sensors\nprint(sensors.scale(1))\n"),
                ("app/sensors.zeno", "function scale(n: int) -> int {\n    return n * 2\n}\n\ntest \"scale\" {\n    assert scale(3) == 6\n}\n"),
                // A file in a folder below imports as it does in the
                // program: from the folder that has `main.zeno`.
                (
                    "app/drivers/uart.zeno",
                    "import sensors\n\ntest \"one\" {\n    assert sensors.scale(2) == 4\n}\n\ntest \"two\" {\n    assert sensors.scale(2) == 5\n}\n",
                ),
                ("app/broken.zeno", "test \"x\" {\n    assert nothing_here == 1\n}\n"),
                ("app/stops.zeno", "print(1 // 0)\n\ntest \"x\" {\n    assert true\n}\n"),
                ("alone/one.zeno", "test \"alone\" {\n    assert 1 + 1 == 2\n}\n"),
            ],
        );
        let at = |path: &str| root.join(path).display().to_string();
        assert_eq!(program_folder(&at("app/drivers/uart.zeno")), root.join("app"));
        assert_eq!(program_folder(&at("alone/one.zeno")), root.join("alone"));

        let counts = |path: &str| test(&at(path), false, true).map(|tally| (tally.tests, tally.failed));
        assert_eq!(counts("app/sensors.zeno"), Some((1, 0)));
        assert_eq!(counts("app/drivers/uart.zeno"), Some((2, 1)));
        assert_eq!(counts("alone/one.zeno"), Some((1, 0)));
        // A file with no test is not run, and is no failure.
        assert_eq!(counts("app/main.zeno"), Some((0, 0)));
        // A mistake in the file, or an error before its tests: none is run.
        assert_eq!(counts("app/broken.zeno"), None);
        assert_eq!(counts("app/stops.zeno"), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn zeno_new_makes_a_program_that_runs_and_a_package_it_can_need() {
        let root = folder("new", &[("taken/mine.txt", "keep me\n")]);
        let at = |path: &str| root.join(path).display().to_string();
        let text = |path: &str| fs::read_to_string(root.join(path)).expect("the file is there");
        let files = |folder: &str| {
            let mut names: Vec<String> =
                fs::read_dir(root.join(folder)).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().to_string()).collect();
            names.sort();
            names
        };
        // What a program prints, and how many of its tests pass and fail.
        let run = |path: &str| {
            let (_, program) = check(&at(path), checker::Rules { strict: true, one_owner: false, firmware: false }).expect("no mistake");
            interpreter::run_tests(&program, true, value::Outside::started_with(Vec::new())).0
        };
        let counts = |path: &str| test(&at(path), true, true).map(|tally| (tally.tests, tally.failed));

        // A program: its name, where it starts, and a module with a test.
        let said = package::new(&root.join("blinker"), false).expect("it is made");
        assert_eq!(said.len(), 5, "{said:?}");
        assert!(said[0].starts_with("Made the program `blinker` in ") && said[4].starts_with("Run it with `zeno run "), "{said:?}");
        assert_eq!(files("blinker"), ["greeting.zeno", "main.zeno", "zeno.package"]);
        let manifest = package::parse(&text("blinker/zeno.package"), 0).expect("it is read");
        assert_eq!((manifest.name.as_str(), manifest.needs.len()), ("blinker", 0));
        assert_eq!(run("blinker/main.zeno"), "Hello, world, from blinker.\n");
        assert_eq!(counts("blinker/greeting.zeno"), Some((1, 0)));
        // It is checked as a build with no collector checks, too.
        assert!(check(&at("blinker/main.zeno"), checker::Rules { strict: true, one_owner: true, firmware: true }).is_some());

        // A package: its name, and the module that has the name.
        let said = package::new(&root.join("filters"), true).expect("it is made");
        assert_eq!(said.len(), 4, "{said:?}");
        assert!(said[0].starts_with("Made the package `filters` in ") && said[3].starts_with("Run its tests with `zeno test "), "{said:?}");
        assert_eq!(files("filters"), ["filters.zeno", "zeno.package"]);
        assert_eq!(package::parse(&text("filters/zeno.package"), 0).expect("it is read").name, "filters");
        assert_eq!(counts("filters/filters.zeno"), Some((1, 0)));

        // Each file is laid out as `zeno format` lays it out.
        for file in ["blinker/main.zeno", "blinker/greeting.zeno", "filters/filters.zeno"] {
            assert_eq!(format::format(&text(file)).expect("it is read"), text(file), "{file}");
        }

        // The program can need the package, as the package's file says.
        fs::write(root.join("blinker/zeno.package"), "package blinker\nneeds filters in \"../filters\"\n").unwrap();
        fs::write(root.join("blinker/main.zeno"), "import filters\n\nprint(filters.hello(\"blinker\"))\n").unwrap();
        assert_eq!(run("blinker/main.zeno"), "Hello, blinker, from filters.\n");

        // A folder is made as deep as the path says.
        package::new(&root.join("deep/down/thing"), false).expect("it is made");
        assert_eq!(run("deep/down/thing/main.zeno"), "Hello, world, from thing.\n");

        // What cannot be made says why, and nothing is written.
        let refused = |path: &str, package: bool| package::new(&root.join(path), package).expect_err("it is refused");
        assert_eq!(
            refused("my-app", false),
            "`my-app` cannot be the name of a program. A name is letters, digits and `_`, and does not start with a digit, \
             as every name in a Zeno program is. Try `my_app`"
        );
        assert!(refused("2fast", true).ends_with("Try `my_2fast`"));
        assert_eq!(refused("while", false), "`while` is a word of the language, so a program cannot have that name");
        assert_eq!(refused("math", true), "`math` is a module that comes with Zeno, so a package cannot have that name");
        assert!(refused("--lib", false).starts_with("`zeno new` has no option `--lib`."));
        assert!(refused("taken", false).ends_with("`zeno new` makes a new folder, and writes nothing into one that is there"));
        assert_eq!(files("taken"), ["mine.txt"]);
        assert!(refused("blinker", true).contains("here already"));
        for gone in ["my-app", "2fast", "while", "math", "--lib"] {
            assert!(!root.join(gone).exists(), "{gone}");
        }
        assert!(package::new(Path::new(""), false).unwrap_err().starts_with("`zeno new` makes a new folder, so it needs a name"));
        let _ = fs::remove_dir_all(&root);
    }
}
