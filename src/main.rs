//! The `zeno` command-line tool.
//!
//! The compiler stages get added here one at a time. `zeno tokens`,
//! `zeno parse` and `zeno check` show what the lexer, the parser and the
//! type checker find. `zeno join` shows a program of several files as
//! the one program the later stages get. `zeno run` checks a program and then runs it, with
//! the interpreter or, given `--vm`, by compiling it to bytecode for the
//! VM. `zeno bytecode` shows that bytecode. `zeno build` compiles to C,
//! or with `--llvm` to LLVM IR.

// Built for a browser, the keyboard and a few more things that only
// `main` reaches are still compiled, and nothing calls them.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

mod ast;
mod board;
mod builtins;
mod bytecode;
mod checker;
mod deflate;
mod emit_bytecode;
mod emit_c;
mod emit_llvm;
mod error;
mod format;
mod html;
mod interpreter;
mod json;
mod ints;
mod lexer;
mod lsp;
mod modules;
mod ops;
mod package;
mod page;
mod parser;
mod printer;
mod token;
mod units;
mod value;
mod vm;
#[cfg(target_arch = "wasm32")]
mod web_host;

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
    println!("  zeno playground           write `playground.html`: a page to write Zeno");
    println!("                            programs in and run them, in a browser, with");
    println!("                            nothing installed (docs/playground.md)");
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
    println!("  --only <text>            with test: run only the tests that have this");
    println!("                           text in their name");
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
    println!("  --owned                  with build, check or run: every list and object");
    println!("                           has one owner and there is no collector, as");
    println!("                           `memory owned` in zeno.package says. On a");
    println!("                           computer memory grows as the program needs it");
    println!("                           (docs/memory-modes.md)");
    println!("  --collector              with build, check or run: the garbage collector,");
    println!("                           whatever zeno.package says");
    println!("  --release                with build: `+`, `-` and `*` of whole numbers");
    println!("                           wrap round instead of being checked for");
    println!("                           overflow. Every other check stays");
    println!("  --llvm                   with build: compile to LLVM IR in place of C,");
    println!("                           and hand that to clang. It goes with");
    println!("                           --firmware, --no-collector and --memory");
    println!("  --web                    with build: write one page, an `.html` file,");
    println!("                           that runs the program in a browser. The");
    println!("                           program reaches the page with `import web`");
    println!("                           (docs/web.md)");
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
fn test(path: &str, strict: bool, asked: bool, only: Option<&str>) -> Option<Tally> {
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
    let (_, tested) = interpreter::run_tests(&program, false, value::Outside::started_with(Vec::new()).with_page(own_page(path)), only);
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
fn run(path: &str, strict: bool, on_vm: bool, arguments: Vec<String>, owned: bool) -> Option<()> {
    // A program whose memory is `owned` is checked by the rules its
    // build will be checked by, so that what runs also builds.
    let rules = match owned {
        true => emit_c::Build::NoCollector.rules(),
        false => checker::Rules { strict, one_owner: false, firmware: false },
    };
    let (files, program) = check(path, rules)?;
    // The words after the file are the program's (`program.arguments()`),
    // and what it reads with `input()` is the keyboard.
    let outside = value::Outside::started_with(arguments).with_page(own_page(path));
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

/// What the `memory` line of the `zeno.package` beside `path` says, if
/// there is one. A mistake in the file is not reported here: loading the
/// program reads the file again and reports it with the rest.
fn package_memory(path: &str) -> Option<package::Memory> {
    let folder = Path::new(path).parent().unwrap_or(Path::new(""));
    let source = fs::read_to_string(folder.join(package::FILE)).ok()?;
    package::parse(&source, 0).ok()?.memory
}

/// Which kind of build the options and the program's `memory` ask for.
///
/// `--no-collector` is a build for a chip: no collector, and a pool and a
/// text room of fixed sizes. `memory owned` is the same rules on a
/// computer, where the pool grows, unless `--firmware` says the program
/// is for a chip too. docs/memory-modes.md has the whole table.
fn how_built(
    no_collector: bool,
    firmware: bool,
    release: bool,
    memory: Option<package::Memory>,
) -> (emit_c::Build, emit_c::Options) {
    let (kind, grows) = match (no_collector, memory, firmware) {
        (true, _, _) => (emit_c::Build::NoCollector, false),
        (false, Some(package::Memory::Owned), _) => (emit_c::Build::NoCollector, !firmware),
        (false, _, true) => (emit_c::Build::Firmware, false),
        (false, _, false) => (emit_c::Build::Ordinary, false),
    };
    (kind, emit_c::Options { release, grows })
}

/// `zeno build <file>`: compile the program to C, then hand the C to the
/// machine's C compiler to make a program that runs by itself.
///
/// `hello.zeno` becomes `hello.c` and `hello`, in the folder you are in. The
/// C file is kept, because reading it is the best way to see what the
/// compiler did.
fn build(path: &str, memory_report: bool, kind: emit_c::Build, options: emit_c::Options) -> Option<()> {
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

    let c = match emit_c::emit_with(&program, &types, path, kind, options) {
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

    // `CC` is the usual way to say which C compiler to use. With no
    // `CC`, the first of these that is installed is used: `cc` is the
    // usual name for whichever one a system has, `gcc` is what MinGW
    // calls itself on Windows, which has no `cc`, and `clang` is the
    // one that comes with Visual Studio.
    let compilers = match env::var("CC") {
        Ok(named) => vec![named],
        Err(_) => vec!["cc".to_string(), "gcc".to_string(), "clang".to_string()],
    };
    let program_file = emit_c::program_file(name);
    for compiler in &compilers {
        let mut command = Command::new(compiler);
        if memory_report {
            // Turns on a few lines in the runtime that say, as the program
            // ends, how much memory it used and how often the collector ran.
            command.arg("-DZENO_MEMORY_REPORT");
        }
        match command.args(["-O2", "-o", &program_file, &c_file]).args(emit_c::LIBRARIES).status() {
            Ok(status) if status.success() => {
                println!("Built {name}. Run it with {}", how_to_run(name));
                return Some(());
            }
            Ok(_) => {
                eprintln!("zeno: `{compiler}` could not compile `{c_file}`. That is a bug in Zeno, not in your program");
                return None;
            }
            // There is no such program. The next name is tried.
            Err(_) => {}
        }
    }
    let tried: Vec<String> = compilers.clone();
    eprintln!("zeno: no C compiler found (tried {}), so only the C was written.", modules::listed(&tried));
    eprintln!("      Install one, or set CC, then run: cc -O2 -o {program_file} {c_file} {}", emit_c::LIBRARIES.join(" "));
    None
}

/// What to type to run the program a build made, in the folder it is in.
fn how_to_run(name: &str) -> String {
    if cfg!(windows) { format!(".\\{name}") } else { format!("./{name}") }
}

/// `zeno build --llvm <file>`: compile the program to LLVM IR, then hand
/// the IR to clang to make a program that runs by itself.
///
/// `hello.zeno` becomes `hello.ll`, `hello_runtime.c` and `hello`. The IR
/// holds the program. The C file is the runtime the program calls for
/// text, printing and memory, which a C build has pasted in front of it.
fn build_llvm(path: &str, memory_report: bool, kind: emit_c::Build, options: emit_c::Options) -> Option<()> {
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

    let ir = match emit_llvm::emit_with(&program, &types, path, kind, memory_report, options) {
        Ok(ir) => ir,
        Err(error) => {
            report(&files, &error);
            return None;
        }
    };
    let name = Path::new(path).file_stem().and_then(|stem| stem.to_str()).unwrap_or("program");
    let (ir_file, runtime_file) = (format!("{name}.ll"), format!("{name}_runtime.c"));
    for (file, text) in [(&ir_file, ir), (&runtime_file, emit_llvm::runtime_for(kind, options.grows))] {
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
    let program_file = emit_c::program_file(name);
    flags.extend(["-o", &program_file, &ir_file, &runtime_file]);
    flags.extend(emit_c::LIBRARIES);
    match Command::new(&compiler).args(&flags).status() {
        Ok(status) if status.success() => {
            println!("Built {name}. Run it with {}", how_to_run(name));
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

/// The page a program gets if nobody wrote one for it: a title, and a
/// place for what the program prints, where a long line goes on to the
/// next and not off the side.
const DEFAULT_PAGE: &str = "<!doctype html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
<title>NAME</title>
</head>
<body>
<pre id=\"zeno-output\" style=\"white-space: pre-wrap\"></pre>
</body>
</html>
";

/// Bytes as text, in base64: three bytes become four letters. It is how
/// `zeno.wasm` is written into a page, which is text.
fn base64(bytes: &[u8]) -> String {
    const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for three in bytes.chunks(3) {
        let bits = (three[0] as u32) << 16 | (*three.get(1).unwrap_or(&0) as u32) << 8 | *three.get(2).unwrap_or(&0) as u32;
        for place in 0..4 {
            // Three bytes fill four letters, two fill three, one fills two.
            // The rest is written `=`.
            match place <= three.len() {
                true => out.push(LETTERS[(bits >> (18 - 6 * place) & 63) as usize] as char),
                false => out.push('='),
            }
        }
    }
    out
}

/// `zeno.wasm`, which is Zeno itself built for a browser: where it is,
/// and its bytes. It is looked for where `ZENO_WASM` says, then beside
/// `zeno`, where a release has it, then where `cargo build --release
/// --target wasm32-unknown-unknown` leaves it when `zeno` was built by
/// cargo too.
fn zeno_wasm() -> Option<Vec<u8>> {
    let mut places = Vec::new();
    if let Ok(named) = env::var("ZENO_WASM") {
        places.push(PathBuf::from(named));
    } else if let Some(folder) = env::current_exe().ok().as_deref().and_then(Path::parent) {
        places.push(folder.join("zeno.wasm"));
        places.push(folder.join("../wasm32-unknown-unknown/release/zeno.wasm"));
    }
    if let Some(bytes) = places.iter().find_map(|place| fs::read(place).ok()) {
        return Some(bytes);
    }
    eprintln!("zeno: `build --web` puts `zeno.wasm` in the page, which is Zeno itself built for a browser,");
    eprintln!("      and there is none at:");
    for place in &places {
        eprintln!("        {}", modules::shown(place));
    }
    eprintln!("      A release of Zeno has it beside `zeno`. From the source, build it with");
    eprintln!("        cargo build --release --target wasm32-unknown-unknown");
    eprintln!("      and set ZENO_WASM to `target/wasm32-unknown-unknown/release/zeno.wasm`.");
    None
}

/// `zeno playground`: write `playground.html`, one page in which
/// anybody can write a Zeno program and run it, with nothing installed.
///
/// The page is `src/playground.html` with four things written into it:
/// `zeno.wasm` (packed, as in a built page), `src/web.js`, the page a
/// program gets when it has none of its own, and a few of the examples
/// to start from. docs/playground.md walks through it.
fn playground() -> Option<()> {
    let wasm = zeno_wasm()?;
    let html_file = "playground.html";
    if let Err(err) = fs::write(html_file, playground_page(&wasm)) {
        eprintln!("zeno: cannot write `{html_file}`: {err}");
        return None;
    }
    println!("Wrote {html_file}. Open it in a browser, or put it on a web server");
    Some(())
}

/// The playground, with Zeno and the examples in it.
fn playground_page(wasm: &[u8]) -> String {
    // Text as JavaScript reads it. `<` is written another way for the
    // reason `with_program` gives.
    let quoted = |json: json::Json| json.write().replace('<', "\\u003c");
    // Each example has a group, a name, one line on what it shows, the
    // program, and the page it is in ("" for one that only prints).
    let example = |group: &str, name: &str, about: &str, program: &str, page: &str| {
        json::Json::object([
            ("group", json::Json::text(group)),
            ("name", json::Json::text(name)),
            ("about", json::Json::text(about)),
            ("program", json::Json::text(program)),
            ("page", json::Json::text(page)),
        ])
    };
    let examples = json::Json::List(vec![
        example("Start here", "Hello", "The whole program is one line", include_str!("../examples/hello.zeno"), ""),
        example("Start here", "A tour of the language", "Most of Zeno on one page", include_str!("../examples/tour.zeno"), ""),
        example("Start here", "Primes", "Loops, lists and numbers", include_str!("../examples/primes.zeno"), ""),
        example("Start here", "Four ways to write it", "One job, typed, untyped, with a class, functional", include_str!("../examples/styles.zeno"), ""),
        example("The language", "Shapes", "Classes, enums and match", include_str!("../examples/shapes.zeno"), ""),
        example("The language", "Generics", "One ring buffer for many types", include_str!("../examples/generics.zeno"), ""),
        example("The language", "Counting faults", "A dict of counts, sorted", include_str!("../examples/faults.zeno"), ""),
        example("The language", "Handlers", "Functions as values, and errors", include_str!("../examples/handlers.zeno"), ""),
        example("The language", "Commands", "Enum cases that carry values", include_str!("../examples/commands.zeno"), ""),
        example("Firmware", "Packets", "Sized numbers and bit operators", include_str!("../examples/packet.zeno"), ""),
        example("Firmware", "Sensor sentences", "Reading GPS-style lines and checksums", include_str!("../examples/sentence.zeno"), ""),
        example("Firmware", "A scheduler", "Tasks that run every so many ticks", include_str!("../examples/scheduler.zeno"), ""),
        example("Firmware", "Owned buffers", "Memory given back at a line you can point to", include_str!("../examples/buffer.zeno"), ""),
        example("Pages", "Things to do", "A list you add to, in a page", include_str!("../examples/todo.zeno"), include_str!("../examples/todo.html")),
        example("Pages", "Sketch", "Draw with the mouse or a finger", include_str!("../examples/sketch.zeno"), include_str!("../examples/sketch.html")),
    ]);
    // Each marker is in the page once. The page is cut at them, so that
    // nothing written in is itself searched for a marker.
    let mut page = String::new();
    let mut rest = include_str!("playground.html");
    let parts = [
        ("__WEB_JS__", quoted(json::Json::text(include_str!("web.js")))),
        ("__WASM__", base64(&deflate::pack(wasm))),
        ("__EXAMPLES__", quoted(examples)),
    ];
    for (marker, value) in parts {
        let (before, after) = rest.split_once(marker).expect("the playground has each marker");
        page += before;
        page += &value;
        rest = after;
    }
    page + rest
}

/// The page that was written for a program: `todo.html` beside
/// `todo.zeno`, if there is one. `zeno build --web` builds on it, and
/// `zeno run` and `zeno test` give it to the simulated page, so that
/// asking for an element the page does not have is an error on a desk
/// as it is in a browser.
fn own_page(path: &str) -> Option<String> {
    fs::read_to_string(Path::new(path).with_extension("html")).ok()
}

/// `zeno build --web <file>`: write one page that runs the program in a
/// browser.
///
/// `counter.zeno` becomes `counter.web.html`, in the folder you are in.
/// The page holds three things: the program, as the text you wrote;
/// `zeno.wasm`, which is Zeno itself built for a browser; and
/// `src/web.js`, which starts the one with the other. Nothing is
/// compiled: the browser runs the program the way `zeno run` does.
///
/// If there is a `counter.html` beside `counter.zeno`, that is the page,
/// and the three are put at its end. If there is none, the page is an
/// empty one that shows what the program prints.
fn build_web(path: &str) -> Option<()> {
    // Checked as `zeno run` checks, since the page runs it as `zeno run`
    // does.
    let (files, program) = load(path)?;
    let errors = checker::checked(program, checker::Rules::default()).1;
    for error in &errors {
        report(&files, error);
    }
    if !errors.is_empty() {
        let s = if errors.len() == 1 { "" } else { "s" };
        eprintln!("{} error{s} found. Nothing was built.", errors.len());
        return None;
    }
    let wasm = zeno_wasm()?;

    let name = Path::new(path).file_stem().and_then(|stem| stem.to_str()).unwrap_or("program");
    let own_page = Path::new(path).with_extension("html");
    let page = match fs::read_to_string(&own_page) {
        Ok(page) => page,
        Err(_) => DEFAULT_PAGE.replace("NAME", name),
    };
    let html_file = format!("{name}.web.html");
    if let Err(err) = fs::write(&html_file, with_program(&page, &files_of(path)?, &wasm)) {
        eprintln!("zeno: cannot write `{html_file}`: {err}");
        return None;
    }
    match own_page.exists() {
        true => println!("Built {html_file} from {} and {path}. Open it in a browser", modules::shown(&own_page)),
        false => println!("Built {html_file}. Open it in a browser"),
    }
    Some(())
}

/// The files of a program as the page carries them, in JSON:
///
/// ```text
/// {"main": "todo.zeno", "files": {"todo.zeno": "import web\n...", "words.zeno": "..."}}
/// ```
///
/// The program is read once more for this, as if from inside its own
/// folder. A file is then named from that folder (`todo.zeno`, and not
/// `/home/partha/site/todo.zeno`), which is what the browser looks for,
/// what an error in the page shows, and all that anyone who opens the
/// page learns about the machine it was built on.
fn files_of(path: &str) -> Option<String> {
    let folder = Path::new(path).parent().unwrap_or(Path::new(""));
    let main = Path::new(path).file_name().and_then(|name| name.to_str())?;
    let found = std::cell::RefCell::new(vec![(main.to_string(), json::Json::Text(read(path)?))]);
    modules::load(main, read(path)?, &|path| {
        let text = fs::read_to_string(folder.join(path)).ok()?;
        found.borrow_mut().push((path.to_string(), json::Json::Text(text.clone())));
        Some(text)
    });
    let program =
        json::Json::object([("main", json::Json::Text(main.to_string())), ("files", json::Json::Object(found.into_inner()))]);
    Some(program.write())
}

/// The page, with the program, Zeno and what starts them written into it.
fn with_program(page: &str, program: &str, wasm: &[u8]) -> String {
    // A browser reads `</script>` as the end of the script, even inside
    // a str of the program. `<` written as `<` is the same str to
    // JSON, and nothing to the browser.
    let program = program.replace('<', "\\u003c");
    // `zeno.wasm` is packed, and then written as text (decision 16.11).
    let scripts = format!(
        "<script>\n{}</script>\n<script>Zeno.start(Zeno.unpacked(\"{}\"), {program});</script>\n",
        include_str!("web.js"),
        base64(&deflate::pack(wasm)),
    );
    // The scripts go last in the body, so that the page is all there
    // when the program starts. A page with no `</body>` gets them at
    // its end, which a browser reads the same way.
    let at = page.to_ascii_lowercase().rfind("</body>").unwrap_or(page.len());
    format!("{}{scripts}{}", &page[..at], &page[at..])
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
    // `--web` builds a page for a browser.
    let web = args.iter().any(|arg| arg == "--web");
    args.retain(|arg| arg != "--web");
    // `--owned` and `--collector` say how a program frees its memory,
    // over what its `zeno.package` says (docs/memory-modes.md).
    let owned = args.iter().any(|arg| arg == "--owned");
    let collector = args.iter().any(|arg| arg == "--collector");
    args.retain(|arg| arg != "--owned" && arg != "--collector");
    // `--release` builds without overflow checks.
    let release = args.iter().any(|arg| arg == "--release");
    args.retain(|arg| arg != "--release");
    if owned && collector {
        eprintln!("zeno: `--owned` and `--collector` are two ways to free memory. Choose one");
        return ExitCode::FAILURE;
    }
    if no_collector && collector {
        eprintln!("zeno: `--no-collector` leaves the collector out, and `--collector` asks for it. Choose one");
        return ExitCode::FAILURE;
    }
    // What the program's own file says, unless the command line says.
    let memory = |path: &str| match (owned, collector) {
        (true, _) => Some(package::Memory::Owned),
        (_, true) => Some(package::Memory::Collector),
        _ => package_memory(path),
    };
    let built = |path: &str| how_built(no_collector, firmware, release, memory(path));

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
                "check" if no_collector || memory(path) == Some(package::Memory::Owned) => {
                    check_no_collector(path).map(|_| println!("No errors found."))
                }
                "check" => check(path, checker::Rules { strict, one_owner: false, firmware: false })
                    .map(|_| println!("No errors found.")),
                _ => run(path, strict, on_vm, arguments, memory(path) == Some(package::Memory::Owned)),
            };
            match done {
                Some(()) => ExitCode::SUCCESS,
                None => ExitCode::FAILURE,
            }
        }
        Some("lsp") => lsp::run(),
        Some("playground") => match playground() {
            Some(()) => ExitCode::SUCCESS,
            None => ExitCode::FAILURE,
        },
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
            // `zeno test --only "empty box"`: only the tests that have
            // that in their name.
            let mut only = None;
            if let Some(at) = named.iter().position(|arg| arg == "--only") {
                if at + 1 >= named.len() {
                    eprintln!("zeno: `--only` needs the text to look for in the names of tests, e.g. `zeno test --only \"empty\"`");
                    return ExitCode::FAILURE;
                }
                only = Some(named.remove(at + 1));
                named.remove(at);
            }
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
                match test(path, strict, asked(path), only.as_deref()) {
                    Some(tally) => {
                        tests += tally.tests;
                        failed += tally.failed;
                    }
                    None => stopped += 1,
                }
            }
            value::print_line("");
            value::print_line(&match (tests, &only) {
                (0, Some(text)) => format!("No tests were run: none has `{text}` in its name."),
                (0, None) => "No tests were run.".to_string(),
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
            // A page is not compiled, so the options that say how to
            // compile have nothing to say about it.
            if web && (llvm || memory_report || firmware || no_collector || owned || release) {
                eprintln!("zeno: `build --web` writes a page, which the browser runs as `zeno run` would.");
                eprintln!("      `--llvm`, `--firmware`, `--no-collector`, `--owned`, `--release` and `--memory`");
                eprintln!("      are for a compiled program");
                return ExitCode::FAILURE;
            }
            let (kind, options) = built(path);
            let built = match (web, llvm) {
                (true, _) => build_web(path),
                (false, true) => build_llvm(path, memory_report, kind, options),
                (false, false) => build(path, memory_report, kind, options),
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
    fn the_memory_a_program_asks_for_chooses_the_build() {
        use emit_c::Build::*;
        use package::Memory::*;
        let built = |no_collector, firmware, memory| {
            let (kind, options) = how_built(no_collector, firmware, false, memory);
            (kind, options.grows)
        };
        // Nothing said: the collector.
        assert_eq!(built(false, false, None), (Ordinary, false));
        assert_eq!(built(false, false, Some(Collector)), (Ordinary, false));
        assert_eq!(built(false, true, None), (Firmware, false));
        // `memory owned`: no collector, and on a computer the memory grows.
        assert_eq!(built(false, false, Some(Owned)), (NoCollector, true));
        // On a chip (`--firmware`) the pool is fixed, as `--no-collector`.
        assert_eq!(built(false, true, Some(Owned)), (NoCollector, false));
        assert_eq!(built(true, false, None), (NoCollector, false));
        assert_eq!(built(true, false, Some(Owned)), (NoCollector, false));
        assert!(how_built(false, false, true, None).1.release);

        let root = folder("memory", &[("zeno.package", "package app\nmemory owned\n"), ("main.zeno", "print(1)\n")]);
        assert_eq!(package_memory(&root.join("main.zeno").to_string_lossy()), Some(Owned));
        let root = folder("no-memory", &[("zeno.package", "package app\n"), ("main.zeno", "print(1)\n")]);
        assert_eq!(package_memory(&root.join("main.zeno").to_string_lossy()), None);
        let root = folder("no-package", &[("main.zeno", "print(1)\n")]);
        assert_eq!(package_memory(&root.join("main.zeno").to_string_lossy()), None);
    }

    #[test]
    fn base64_is_three_bytes_in_four_letters() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"Z"), "Wg==");
        assert_eq!(base64(b"Ze"), "WmU=");
        assert_eq!(base64(b"Zeno"), "WmVubw==");
        assert_eq!(base64(&[0, 255, 254, 253]), "AP/+/Q==");
    }

    #[test]
    fn the_playground_is_one_page_with_everything_written_in() {
        let page = super::playground_page(b"not really zeno.wasm");
        for marker in ["__WEB_JS__", "__WASM__", "__EXAMPLES__"] {
            assert!(!page.contains(marker), "{marker} was left in the playground");
        }
        // Two scripts, each closed once (the theme, read before anything
        // is drawn, and the playground): nothing written into them ends
        // one early, not even an example that has a page with a
        // `<style>` of its own.
        assert_eq!(page.matches("</script>").count(), 2);
        assert!(page.contains("import web"));
    }

    #[test]
    fn a_page_carries_the_files_of_its_program_named_from_its_folder() {
        let root = folder(
            "web-files",
            &[
                ("site/main.zeno", "import words\nimport shout.loud\nprint(words.greeting, shout.loud.twice(\"</script>\"))\n"),
                ("site/words.zeno", "let greeting = \"வணக்கம்\"\n"),
                ("site/zeno.package", "package site\n\nneeds shout in \"../shout\"\n"),
                ("shout/zeno.package", "package shout\n"),
                ("shout/loud.zeno", "function twice(word: str) -> str { return word + word }\n"),
                ("site/unused.zeno", "print(\"never read\")\n"),
            ],
        );
        let files = files_of(&root.join("site/main.zeno").to_string_lossy()).expect("the files");
        let json::Json::Object(members) = json::Json::parse(&files).expect("JSON") else { panic!("an object") };
        assert_eq!(members[0], ("main".to_string(), json::Json::Text("main.zeno".to_string())));
        let json::Json::Object(carried) = &members[1].1 else { panic!("the files") };
        let mut names: Vec<&str> = carried.iter().map(|(name, _)| name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["../shout/loud.zeno", "../shout/zeno.package", "main.zeno", "words.zeno", "zeno.package"]);
        // Nothing in it says where on this machine the program is.
        assert!(!files.contains(&modules::shown(&root)), "{files}");

        // The browser is given the same files and joins them the same way.
        let read = |path: &str| carried.iter().find(|(name, _)| name == path).and_then(|(_, text)| text.as_str().map(String::from));
        let loaded = modules::load("main.zeno", read("main.zeno").expect("the first file"), &read);
        let program = loaded.program.unwrap_or_else(|errors| panic!("{}", errors[0].message));
        let (program, errors, _) = checker::checked(program, checker::Rules::default());
        assert!(errors.is_empty(), "{}", errors[0].message);
        let (output, result) = interpreter::run_with(&program, true, value::Outside::default());
        assert!(result.is_ok(), "{:?}", result.err().map(|error| error.message));
        assert_eq!(output, "வணக்கம் </script></script>\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_program_goes_last_in_the_body_of_its_page() {
        let program = r#"{"main":"a.zeno","files":{"a.zeno":"print(\"</script><b>\")"}}"#;
        let page = "<html><BODY>\n<h1 id=\"count\"></h1>\n</BODY></html>\n";
        let built = with_program(page, program, b"Zeno");
        let (before, after) = built.split_once("<script>").expect("a script");
        assert_eq!(before, "<html><BODY>\n<h1 id=\"count\"></h1>\n");
        assert!(after.ends_with("</script>\n</BODY></html>\n"), "{after}");
        // The program cannot end the script it is written in.
        let packed = base64(&deflate::pack(b"Zeno"));
        let start = r#"<script>Zeno.start(Zeno.unpacked("PACKED"), {"main":"a.zeno","files":{"a.zeno":"print(\"LESS/script>LESSb>\")"}});</script>"#
            .replace("LESS", "\\u003c")
            .replace("PACKED", &packed);
        assert!(built.contains(&start), "{}", &built[built.len() - 300..]);
        assert_eq!(built.matches("</script>").count(), 2);

        // A page with no `</body>` gets it at its end.
        let built = with_program("<p>Hello</p>\n", program, b"Zeno");
        assert!(built.starts_with("<p>Hello</p>\n<script>"), "{built}");
        assert!(DEFAULT_PAGE.contains("id=\"zeno-output\"") && DEFAULT_PAGE.contains("</body>"));
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
        assert!(build(&at("gaps.zeno"), false, emit_c::Build::NoCollector, emit_c::Options::default()).is_none());
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

        let counts = |path: &str| test(&at(path), false, true, None).map(|tally| (tally.tests, tally.failed));
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
            interpreter::run_tests(&program, true, value::Outside::started_with(Vec::new()), None).0
        };
        let counts = |path: &str| test(&at(path), true, true, None).map(|tally| (tally.tests, tally.failed));

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
