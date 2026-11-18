//! A build for a board: `zeno build --board <folder> blink.zeno`.
//!
//! A program for a chip is not built by the computer's own C compiler,
//! and not with the computer's start and end. Four things are the
//! board's, and none of them is the program's: the compiler that makes
//! code for its processor, the flags that say which processor, the file
//! that runs before `main`, and the script that tells the linker where
//! the chip's flash and RAM are. A board is a folder that holds them,
//! with a file called `zeno.board` that names them:
//!
//! ```text
//! about "An ARM Cortex-M3 that QEMU emulates"
//!
//! compiler "arm-none-eabi-gcc"
//! flags "-mcpu=cortex-m3" "-mthumb"
//! header "board.h"
//! sources "startup.c" "system.c"
//! script "link.ld"
//! link "-nostartfiles"
//! run "qemu-system-arm" "-M" "lm3s6965evb" "-nographic" "-kernel"
//! ```
//!
//! This file reads a `zeno.board` and says what the compiler is asked.
//! `main.rs` writes the program's C and asks. docs/board-build.md walks
//! through it, and decision 51 has the reasons.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::lexer;
use crate::modules::shown;
use crate::token::{Span, TokenKind};

/// The file that makes a folder a board.
pub const FILE: &str = "zeno.board";

/// What a `zeno.board` says.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Board {
    /// `about "..."`: one line that says which board this is.
    pub about: Option<String>,
    /// `compiler "arm-none-eabi-gcc"`: the C compiler for the board's
    /// processor. The one line a board must have.
    pub compiler: String,
    /// `flags ...`: what the compiler is told for every file, and again
    /// when it links: which processor, which floating point.
    pub flags: Vec<String>,
    /// `header "board.h"`: the header with the board's own functions
    /// (docs/board.md), in the board's folder. With none, the program
    /// gets the simulated board, as on a computer.
    pub header: Option<String>,
    /// `sources ...`: files of C in the board's folder that are compiled
    /// with every program: what runs before `main`, mostly.
    pub sources: Vec<String>,
    /// `script "link.ld"`: the linker script, in the board's folder.
    pub script: Option<String>,
    /// `link ...`: what the compiler is told only when it links.
    pub link: Vec<String>,
    /// `run ...`: the command that runs a built program, or loads it
    /// onto the board. `zeno build` shows it and does not run it.
    pub run: Vec<String>,
}

const LINES: &str = "A line of `zeno.board` is one of:\n    \
    about \"which board this is, in one line\"\n    \
    compiler \"arm-none-eabi-gcc\"       (the C compiler for the board's processor)\n    \
    flags \"-mcpu=cortex-m3\" \"-mthumb\"  (what the compiler is always told)\n    \
    header \"board.h\"                   (the board's own functions: docs/board.md)\n    \
    sources \"startup.c\"                (files of C built with every program)\n    \
    script \"link.ld\"                   (the linker script)\n    \
    link \"-nostartfiles\"               (what the compiler is told when it links)\n    \
    run \"qemu-system-arm\" \"-kernel\"    (the command that runs or loads a built program)\n\
    Each word in quotes is one word of the command. A file is named as it is in the board's folder";

/// Read a `zeno.board`. The words are split by Zeno's own lexer, as a
/// `zeno.package` is, so a comment starts with `#` and a string is
/// written as in a program.
pub fn parse(source: &str) -> Result<Board, Vec<Error>> {
    let tokens = lexer::lex_in(source, 0)?;
    let mut errors = Vec::new();
    let mut board = Board::default();
    let mut compiler: Option<String> = None;
    let mut ran = false;

    for line in tokens.split(|token| matches!(token.kind, TokenKind::Newline | TokenKind::Eof)) {
        let Some(first) = line.first() else { continue };
        let span = first.span;
        let TokenKind::Ident(word) = &first.kind else {
            errors.push(Error::new(format!("this is not a line of a `zeno.board`. {LINES}"), span));
            continue;
        };
        // Everything after the first word is a word in quotes, and there is one at least.
        let rest = &line[1..];
        let words: Vec<String> = rest
            .iter()
            .filter_map(|token| match &token.kind {
                TokenKind::Str { value, formatted: false } => Some(value.clone()),
                _ => None,
            })
            .collect();
        let known = ["about", "compiler", "flags", "header", "sources", "script", "link", "run"].contains(&word.as_str());
        if !known {
            errors.push(Error::new(format!("this is not a line of a `zeno.board`. {LINES}"), span));
            continue;
        }
        if words.is_empty() || words.len() != rest.len() {
            errors.push(Error::new(format!("`{word}` is followed by one or more words in quotes, and nothing else. {LINES}"), span));
            continue;
        }
        let said = match word.as_str() {
            "about" => one(&mut board.about, word, &words, "what the board is, as in `about \"A Nucleo-F103RB\"`"),
            "compiler" => one(&mut compiler, word, &words, "the C compiler for the board, as in `compiler \"arm-none-eabi-gcc\"`"),
            "header" => one(&mut board.header, word, &words, "the header with the board's functions, as in `header \"board.h\"`"),
            "script" => one(&mut board.script, word, &words, "the linker script, as in `script \"link.ld\"`"),
            "run" if ran => Err("a `zeno.board` says `run` once".to_string()),
            "run" => {
                ran = true;
                board.run = words;
                Ok(())
            }
            "flags" => Ok(board.flags.extend(words)),
            "sources" => Ok(board.sources.extend(words)),
            _ => Ok(board.link.extend(words)),
        };
        if let Err(why) = said {
            errors.push(Error::new(why, span));
        }
    }
    match compiler {
        Some(compiler) => board.compiler = compiler,
        None if errors.is_empty() => {
            return Err(vec![Error::new(
                "a `zeno.board` names the C compiler for the board's processor, as in \
                 `compiler \"arm-none-eabi-gcc\"`, and this one has no such line",
                Span { line: 1, col: 1 },
            )]);
        }
        None => {}
    }
    if errors.is_empty() { Ok(board) } else { Err(errors) }
}

/// A line that says one thing says it once, in one word.
fn one(place: &mut Option<String>, word: &str, words: &[String], what: &str) -> Result<(), String> {
    if place.is_some() {
        return Err(format!("a `zeno.board` says `{word}` once"));
    }
    if words.len() != 1 || words[0].trim().is_empty() {
        return Err(format!("`{word}` is followed by one thing in quotes: {what}"));
    }
    *place = Some(words[0].clone());
    Ok(())
}

impl Board {
    /// The files of the board's folder that the `zeno.board` names and
    /// that are not there.
    pub fn missing(&self, folder: &Path) -> Vec<String> {
        let named = self.header.iter().chain(&self.sources).chain(&self.script);
        named.filter(|file| !folder.join(file).is_file()).cloned().collect()
    }

    /// What the board's compiler is asked, after its own name, to make
    /// `program_file` of `c_file`.
    ///
    /// The order is the order a C compiler wants, and it gives the board
    /// the last word: `before` is what Zeno asks of every build (`-O2`),
    /// and the board's flags come after it, so a board that says `-Os`
    /// gets `-Os`. `after` is what the program itself brings: the
    /// libraries of C it declares, and `-lm`.
    pub fn asks(&self, folder: &Path, before: &[String], c_file: &str, program_file: &str, after: &[String]) -> Vec<String> {
        let here = |file: &String| shown(&folder.join(file));
        let mut asks: Vec<String> = before.to_vec();
        asks.extend(self.flags.iter().cloned());
        // The board's folder is where its header is, and whatever the
        // header and the sources include in their turn.
        asks.push(format!("-I{}", shown(&at_least_here(folder))));
        if let Some(header) = &self.header {
            asks.push(format!("-DZENO_BOARD=\"{header}\""));
        }
        asks.extend(["-o".to_string(), program_file.to_string(), c_file.to_string()]);
        asks.extend(self.sources.iter().map(here));
        if let Some(script) = &self.script {
            asks.extend(["-T".to_string(), here(script)]);
        }
        asks.extend(self.link.iter().cloned());
        asks.extend(after.iter().cloned());
        asks
    }

    /// The command that runs `program_file`, if the board says one: the
    /// file is its last word, or stands where a word says `{program}`.
    pub fn runs(&self, program_file: &str) -> Option<Vec<String>> {
        if self.run.is_empty() {
            return None;
        }
        let mut words: Vec<String> = self.run.iter().map(|word| word.replace("{program}", program_file)).collect();
        if !self.run.iter().any(|word| word.contains("{program}")) {
            words.push(program_file.to_string());
        }
        Some(words)
    }
}

/// A folder as a compiler can be told it: the folder you are in has a name too.
fn at_least_here(folder: &Path) -> PathBuf {
    if folder.as_os_str().is_empty() { PathBuf::from(".") } else { folder.to_path_buf() }
}

/// A command as it is typed: a word with a space or a quote in it is
/// put in single quotes.
pub fn typed(words: &[String]) -> String {
    let word = |word: &String| match word.is_empty() || word.contains([' ', '"', '\'', '\\', '$', '*', '(', ')', '{', '}']) {
        true => format!("'{}'", word.replace('\'', "'\\''")),
        false => word.clone(),
    };
    words.iter().map(word).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHOLE: &str = "\
# A board for the tests.
about \"A board for the tests\"

compiler \"arm-none-eabi-gcc\"
flags \"-mcpu=cortex-m3\" \"-mthumb\"
flags \"-Os\"
header \"board.h\"
sources \"startup.c\"
sources \"system.c\"
script \"link.ld\"
link \"-nostartfiles\" \"-Wl,--gc-sections\"
run \"qemu-system-arm\" \"-M\" \"lm3s6965evb\" \"-kernel\"
";

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    fn mistake(source: &str) -> String {
        parse(source).expect_err("a mistake").iter().map(|error| error.message.clone()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn a_board_file_names_what_the_compiler_is_asked() {
        let board = parse(WHOLE).expect("a board");
        assert_eq!(board.about.as_deref(), Some("A board for the tests"));
        assert_eq!(board.compiler, "arm-none-eabi-gcc");
        assert_eq!(board.flags, words(&["-mcpu=cortex-m3", "-mthumb", "-Os"]));
        assert_eq!(board.sources, words(&["startup.c", "system.c"]));
        // Zeno's own flags first, then the board's, which so have the
        // last word; the board's files are named in its folder.
        let asks = board.asks(Path::new("boards/mine"), &words(&["-O2"]), "blink.c", "blink.elf", &words(&["-lm"]));
        assert_eq!(
            asks,
            words(&[
                "-O2",
                "-mcpu=cortex-m3",
                "-mthumb",
                "-Os",
                "-Iboards/mine",
                "-DZENO_BOARD=\"board.h\"",
                "-o",
                "blink.elf",
                "blink.c",
                "boards/mine/startup.c",
                "boards/mine/system.c",
                "-T",
                "boards/mine/link.ld",
                "-nostartfiles",
                "-Wl,--gc-sections",
                "-lm",
            ])
        );
        // The program is the last word of the command that runs it.
        assert_eq!(typed(&board.runs("blink.elf").expect("a command")), "qemu-system-arm -M lm3s6965evb -kernel blink.elf");
    }

    #[test]
    fn a_board_needs_only_a_compiler() {
        let board = parse("compiler \"cc\"\n").expect("a board");
        assert_eq!(board, Board { compiler: "cc".to_string(), ..Board::default() });
        // No header: the simulated board. No script, no sources.
        assert_eq!(board.asks(Path::new(""), &[], "a.c", "a.elf", &[]), words(&["-I.", "-o", "a.elf", "a.c"]));
        assert_eq!(board.runs("a.elf"), None);
        assert!(board.missing(Path::new("nowhere")).is_empty());
    }

    #[test]
    fn the_program_stands_where_the_run_line_says() {
        let board = parse("compiler \"cc\"\nrun \"openocd\" \"-c\" \"program {program} verify reset exit\"\n").expect("a board");
        assert_eq!(typed(&board.runs("my app.elf").expect("a command")), "openocd -c 'program my app.elf verify reset exit'");
    }

    #[test]
    fn a_file_the_board_names_must_be_there() {
        let board = parse(WHOLE).expect("a board");
        assert_eq!(board.missing(Path::new("no-such-folder")), words(&["board.h", "startup.c", "system.c", "link.ld"]));
    }

    #[test]
    fn mistakes_in_a_board_file_are_named() {
        assert!(mistake("about \"no compiler\"\n").contains("names the C compiler"));
        assert!(mistake("compiler \"cc\"\ncompiler \"gcc\"\n").contains("says `compiler` once"));
        assert!(mistake("compiler \"cc\" \"-O2\"\n").contains("`compiler` is followed by one thing in quotes"));
        assert!(mistake("compiler cc\n").contains("`compiler` is followed by one or more words in quotes"));
        assert!(mistake("compiler \"cc\"\nflags\n").contains("`flags` is followed by one or more words in quotes"));
        assert!(mistake("compiler \"cc\"\nprocessor \"m3\"\n").contains("this is not a line of a `zeno.board`"));
        assert!(mistake("compiler \"cc\"\nrun \"a\"\nrun \"b\"\n").contains("says `run` once"));
        assert!(mistake("compiler \"cc\"\n\"flags\"\n").contains("this is not a line of a `zeno.board`"));
    }
}
