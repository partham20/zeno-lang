# The history of Zeno

This page lists every commit in the Zeno repository, oldest first, under
the release it went into. Read from the top, it is the story of how the
language was built, from an empty folder on 14 November 2026 to what is on
`main` today.

Each line says in plain words what one commit changed. The letters and
digits at its start are the commit's short name, and they open the commit on
GitHub, with its full message and every line of code it touched. A release
is a heading, with a few words under it on what the release brought. The
last heading, Unreleased, is what is on `main` and in no release yet.

Two other files go with this one. The reason for every choice in the
language is in [`docs/decisions.md`](docs/decisions.md), and what is still
to be built is in [`docs/missing.md`](docs/missing.md).

## The releases

- [**Zeno 0.1.0**](#zeno-010), 15 November 2026, 156 commits: the whole language, a book, and builds for ten kinds of machine
- [**Zeno 0.2.0**](#zeno-020), 16 November 2026, 19 commits: a program in a web page, installers and packages
- [**Zeno 0.3.0**](#zeno-030), 16 November 2026, 32 commits: speed, and a choice of how memory is freed
- [**Zeno 0.4.0**](#zeno-040), 16 November 2026, 18 commits: a build as fast as C and Rust
- [**Zeno 0.5.0**](#zeno-050), 17 November 2026, 12 commits: the program written in another language
- [**Zeno 0.5.1**](#zeno-051), 17 November 2026, 6 commits: the VM as fast as it was
- [**Zeno 0.5.2**](#zeno-052), 18 November 2026, 41 commits: four more targets, a registry, and the libraries of other languages
- [**Zeno 0.6.0**](#zeno-060), 19 November 2026, 33 commits: the release for firmware, and the libraries of five more languages
- [**Unreleased**](#unreleased), on `main` now, 2 commits: what has come since 0.6.0

## Zeno 0.1.0

*Released 15 November 2026 · 156 commits · [the release on GitHub](https://github.com/zeno-src/zeno-lang/releases/tag/v0.1.0)*

The first release is the largest part of the story: the whole language was
built on 14 and 15 November 2026. It begins with a README and a sketch of
the syntax, and then goes stage by stage. A lexer, a parser, an interpreter
and a type checker come first, then a compiler to C and a garbage collector,
then builds for firmware that have no collector at all, a second engine (a
bytecode VM) and a second backend (LLVM). After that come the library,
programs in several files, packages, a formatter, a language server for
editors, and tests written in Zeno.

Most stages are three commits: the code, a walkthrough of it in `docs/`, and
the reasons in `docs/decisions.md`. The release ends with The Zeno Book and
with a workflow that builds `zeno` for ten kinds of machine. Writing the
book and trying the release on a Mac and on Windows each found mistakes,
and the mends are here too.

### A name, a sketch and a syntax

*14 November 2026*

- [`de07d57`](https://github.com/zeno-src/zeno-lang/commit/de07d576adb39caeec2b4c2a56203039f2ae6638) The first commit: a README, an empty command-line program in Rust, and `hello.zeno`.
- [`8e8ea36`](https://github.com/zeno-src/zeno-lang/commit/8e8ea3627b08dca3f07bb259751b45c124e16343) The syntax is made to look like Python first, and the README lists what the language should be able to do.
- [`8e5cda9`](https://github.com/zeno-src/zeno-lang/commit/8e5cda971e4b0f5a51a4baf7c9266f2772f83832) The syntax changes to the one Zeno kept: braces, `let` and `var`, and whole words for keywords.

### Reading a program: the lexer and the parser

*14 November 2026*

- [`33669d7`](https://github.com/zeno-src/zeno-lang/commit/33669d790e9f86f91c5cf34590febc151cd12ca1) The lexer: it cuts a file into tokens and decides where a line ends, with tests.
- [`9fea775`](https://github.com/zeno-src/zeno-lang/commit/9fea7757aae33c6b7f33732d0e8597dd3884d701) `zeno tokens` prints the tokens of a file, and an error points at its place with a caret.
- [`0ee6898`](https://github.com/zeno-src/zeno-lang/commit/0ee6898e689ade868757a54aeeeb2fdab8593ac9) A walkthrough of how the lexer works.
- [`861e3b6`](https://github.com/zeno-src/zeno-lang/commit/861e3b638d39318d1d3883b91921789275de6ebf) Every stage shares one kind of error, and a message names a token in words.
- [`37b02f5`](https://github.com/zeno-src/zeno-lang/commit/37b02f5a8bb02afe5f573805b9f684c76a6bd3fd) The parser: it turns the tokens into a syntax tree, and `zeno parse` prints the tree.
- [`7e99c93`](https://github.com/zeno-src/zeno-lang/commit/7e99c93d4519c3c4cd195caf2c5b5f282c543dc4) A walkthrough of how the parser works.
- [`15f52e4`](https://github.com/zeno-src/zeno-lang/commit/15f52e41b82e026cab369cc611de75b45a9ce3f0) Comparisons chain as in Python (`0 < x < 10`), `//` divides whole numbers, and `is` is kept for `none`.

### Running a program: the interpreter

*14 November 2026*

- [`8c90b90`](https://github.com/zeno-src/zeno-lang/commit/8c90b90dd1ecfdd7bb04fa1a96733ad83caccf16) The interpreter: `zeno run` runs a program for the first time.
- [`52e8edc`](https://github.com/zeno-src/zeno-lang/commit/52e8edc722a39a5d96a2f47de18850170b70bb50) `docs/decisions.md` begins, the file that holds the reason for every choice, with a walkthrough of the interpreter.
- [`8fc07ff`](https://github.com/zeno-src/zeno-lang/commit/8fc07ff74223239277ec295417551a44dab4acf2) Functions can be used as values, `map`, `filter` and `reduce` arrive, and the type of a field may be left out.
- [`7a1956b`](https://github.com/zeno-src/zeno-lang/commit/7a1956b8cc9814569f16048f2a4327677dfa7d9d) The reasons for allowing several styles and optional types are written down, with an example.

### Checking a program before it runs: the type checker

*14 November 2026*

- [`d30c750`](https://github.com/zeno-src/zeno-lang/commit/d30c75073ac65d8ab5637d86070c337ab739a3b4) The type checker: `zeno check` finds mistakes before a program runs, and strict mode asks for every type to be written.
- [`45eb543`](https://github.com/zeno-src/zeno-lang/commit/45eb543f4ea96a2d45c812de0f76a7a3a480784a) Tests for the checker, and a list that is written out is checked item by item.
- [`a39031a`](https://github.com/zeno-src/zeno-lang/commit/a39031a07d50b9d13aa37a25ff3ded1ccf9dd30e) A walkthrough of the checker, and its reasons.
- [`7873651`](https://github.com/zeno-src/zeno-lang/commit/7873651c41647f97fcc68b035c0dcbf1ad960901) A type can be a function (`function(int) -> int`), a file can begin with `strict`, and units come from a fixed table, so a misspelt unit is caught.
- [`273b260`](https://github.com/zeno-src/zeno-lang/commit/273b26079ce6e24b290717b8697e02c79c6f612a) The checker works out the types of parameters and fields that have none written, by going over the program until nothing changes.
- [`0f32d01`](https://github.com/zeno-src/zeno-lang/commit/0f32d019f5ba75d9c916f5e8064d3f3f2ec73dbc) The docs for function types, unit types, the `strict` line and worked-out types.

### Compiling to C

*14 November 2026*

- [`26eb7eb`](https://github.com/zeno-src/zeno-lang/commit/26eb7eb240f4a042e1ce706d99c1e1220fe2cdba) The checker hands what it learned about each function to the stages after it.
- [`7ea2462`](https://github.com/zeno-src/zeno-lang/commit/7ea2462a10d8521407a34d8854b540a4cebd3295) `zeno build`: numbers, text, functions and control flow are compiled to C. The tests compare a compiled program's output with the interpreter's, byte for byte.
- [`505e4fc`](https://github.com/zeno-src/zeno-lang/commit/505e4fcfd102b244bb632b661f4f9336d8dfa72e) A walkthrough of the C backend, and its reasons.
- [`f470cb3`](https://github.com/zeno-src/zeno-lang/commit/f470cb3b8651f759d0095002b1c685e707b22ad1) An error caused by a worked-out type says which call the type came from, and units of one kind work together: `1 s + 500 ms` is `1500 ms`.
- [`6b5c1c5`](https://github.com/zeno-src/zeno-lang/commit/6b5c1c5a93d634f3b1fcdeae3b9cc52e3e8c74d1) The checker hands the types of classes, enums and lists to the compiler.
- [`e38aaae`](https://github.com/zeno-src/zeno-lang/commit/e38aaae615c79ab111f376568943b664ee93031b) Classes, lists, enums and `match` compile to C.
- [`cb32725`](https://github.com/zeno-src/zeno-lang/commit/cb327251282f7e2860e32b0987fa3bcb582d6781) The docs for how classes, enums, `match` and lists compile.
- [`252e4b4`](https://github.com/zeno-src/zeno-lang/commit/252e4b4394003055892e641dcac8bfe0d4c678e0) The interpreter is tightened so that a compiled program can agree with it: a function value sees only what was in view where it was written, a raised object is caught by its class, and text methods know plain letters only.
- [`be1f7cc`](https://github.com/zeno-src/zeno-lang/commit/be1f7cc15b9e7923135b0896243f0752df4526eb) `none`, errors (`try` and `raise`) and functions as values compile to C. The tour of the language now builds.
- [`c4c718b`](https://github.com/zeno-src/zeno-lang/commit/c4c718bffdb110e454bdc3798b9f8bef64b410d6) The docs for how `none`, errors and function values compile.

### Memory: a collector, a firmware build, and owned values

*14 November 2026*

- [`a199025`](https://github.com/zeno-src/zeno-lang/commit/a199025de543c0d9aacbc3f025c81199e5d9e042) Compiled programs free their memory with a mark-and-sweep garbage collector, and `--memory` reports what was used.
- [`feef6df`](https://github.com/zeno-src/zeno-lang/commit/feef6df4185a89943c82bb803b14fe60f8143efd) A walkthrough of the collector, and a plan to carry errors as plain values for firmware.
- [`a31c589`](https://github.com/zeno-src/zeno-lang/commit/a31c5897832cbc097aca6e702c42695643cb90d5) The plan for the next two stages: owned values with borrows, and a firmware build with no collector.
- [`44f6cc8`](https://github.com/zeno-src/zeno-lang/commit/44f6cc865c8b1a8e59d709f37f88d53493962ffe) `zeno build --firmware`: an error travels as a returned value, with no `setjmp`, as firmware rules such as MISRA C ask.
- [`83e3a58`](https://github.com/zeno-src/zeno-lang/commit/83e3a58e88c86bfe79a6c07846db0e16232a982f) A walkthrough of the firmware build, and its reasons.
- [`672487e`](https://github.com/zeno-src/zeno-lang/commit/672487e6faae04bcf22cd9075d5db05cc82839dc) Owned values: a value marked `owned` has one owner and is freed the moment its owner's block ends. A `borrow` parameter only looks.
- [`23fa3f5`](https://github.com/zeno-src/zeno-lang/commit/23fa3f5c68f76f8064e3309249f7e30d7703b680) A walkthrough of owned values, and decision 5.8.
- [`2747119`](https://github.com/zeno-src/zeno-lang/commit/27471194c8ad98cbc55284c68fb557d04db12e3d) The README and the older pages point at owned values.
- [`91fde19`](https://github.com/zeno-src/zeno-lang/commit/91fde19375494519e8e494f829618f461b00eb18) `zeno build --no-collector`: a firmware build with no garbage collector in it. Anything that would take memory nobody gives back is refused when the program is built.
- [`bc5f77c`](https://github.com/zeno-src/zeno-lang/commit/bc5f77c1c2040c5946299aef624ee66155e02690) A walkthrough of the build with no collector, and decision 5.9.
- [`393c071`](https://github.com/zeno-src/zeno-lang/commit/393c071a66ab633cdd5bad07852c3a1f91286ea2) The README and the older pages point at the build with no collector.

### A second engine: bytecode and a VM

*14 November 2026*

- [`216e852`](https://github.com/zeno-src/zeno-lang/commit/216e8525d93fefd33c9c6e99a93f4cc1c20ef254) What each operator does moves to a file of its own, so that a second engine can share it.
- [`b2ce242`](https://github.com/zeno-src/zeno-lang/commit/b2ce2426e67f4ede24d3f29bbd7aa8ee3516e154) A function sees what is declared above it and nothing below: a mistake in scoping, found while building the VM, is mended.
- [`e7e94c0`](https://github.com/zeno-src/zeno-lang/commit/e7e94c07a6e7e01e278af02f3cfe73e32ae2548e) Bytecode: a program as a flat list of small instructions, and a compiler from the syntax tree to it. `zeno bytecode` prints it.
- [`9ff462c`](https://github.com/zeno-src/zeno-lang/commit/9ff462c4e3aa55be0d4dada85528c047ccebbdd4) The VM: `zeno run --vm` runs the bytecode, two to three times as fast as the interpreter. Every test now runs on both engines and must give the same output.
- [`9a6e6ae`](https://github.com/zeno-src/zeno-lang/commit/9a6e6ae745c1fb7c5e27cd99aaefb301f8a89ec0) The bytecode listing names two hidden slots, and the help text lines up.
- [`1eadece`](https://github.com/zeno-src/zeno-lang/commit/1eadece1c53886c642ab7f92cc064b28b94f75e2) A walkthrough of the bytecode VM, and its reasons.
- [`6d108a8`](https://github.com/zeno-src/zeno-lang/commit/6d108a89f30aeccbe68f2bad45b8f488c374eeab) The checker refuses a function that is called before a variable it needs is declared.

### A second backend: LLVM

*14 November 2026*

- [`00513e1`](https://github.com/zeno-src/zeno-lang/commit/00513e117de723d52b3a1058dacabe9601da399b) The C backend's code for reading an f-string format is split off, so that another backend can use it.
- [`4a4f1de`](https://github.com/zeno-src/zeno-lang/commit/4a4f1de975ba86c3974fc0cd2cdbe7aa7d41f33f) `zeno build --llvm`: numbers, text, functions and control flow are written as LLVM IR, as plain text and with no dependency.
- [`2049bc3`](https://github.com/zeno-src/zeno-lang/commit/2049bc3aeb74508f0434e1fb8ec5ff1adbe47fcb) A walkthrough of the LLVM backend, and its reasons.
- [`798e928`](https://github.com/zeno-src/zeno-lang/commit/798e928a43f04c72d0ff0390e3d84bb323e1307d) `//`, `%` and `/` are written in IR and no longer call the runtime, which makes a loop of them more than twice as fast.
- [`384b5d6`](https://github.com/zeno-src/zeno-lang/commit/384b5d63656c5e3dbb17a315e56491106879a835) The LLVM backend compiles the rest of the language: classes, enums, `match`, lists, `none`, errors, function values and units.
- [`e524554`](https://github.com/zeno-src/zeno-lang/commit/e5245547e369e5d9c001bd69d9ffb2137a00c271) The walkthrough of the LLVM backend is rewritten for the whole language.

### For firmware: a board, bits, sized numbers and generics

*14 November 2026*

- [`fa3961a`](https://github.com/zeno-src/zeno-lang/commit/fa3961a648db8b500771acc92f036b099b874e2f) The `board` and `time` modules: pins, `sleep` and `now`. `blink.zeno` runs on a simulated board and builds for a real one.
- [`7f10b84`](https://github.com/zeno-src/zeno-lang/commit/7f10b84168e98aaaff8f31aa7a86983a512b97a6) The bit operators `&`, `|`, `^`, `~`, `<<` and `>>`.
- [`c1f941b`](https://github.com/zeno-src/zeno-lang/commit/c1f941bab94f1f57dbc3b653d780d8801c92e9d8) Whole numbers with a size: `u8`, `u16`, `u32`, `i8`, `i16` and `i32`. A number that does not fit is an error and never a wrong answer.
- [`2840eb6`](https://github.com/zeno-src/zeno-lang/commit/2840eb6febcb9eef8b8c6b70ea4787280e70a204) The docs for the bit operators and the sized numbers.
- [`1ff13ec`](https://github.com/zeno-src/zeno-lang/commit/1ff13ec5759e5e34ffd002bc651f0b12dd296329) `zeno tokens`, `parse` and `bytecode` end quietly when their output goes to a program that stops reading.
- [`0c3ee4d`](https://github.com/zeno-src/zeno-lang/commit/0c3ee4dbc5405e2a3e1087148fe5a3b0040e972f) Generics: `function first[T](...)` and `class Stack[T]`. The checker works the type out from the call.
- [`01a8585`](https://github.com/zeno-src/zeno-lang/commit/01a85858ce1ea65649ea565c2d9710fe670c17ff) The docs for generics.
- [`a90e33f`](https://github.com/zeno-src/zeno-lang/commit/a90e33f493e34e499f20fcf14b3c9bc9367af2f9) `--llvm --firmware`: the LLVM backend carries errors as values too.
- [`66410c0`](https://github.com/zeno-src/zeno-lang/commit/66410c0d8b6bbb7c4e2acbde90357ac7804f4701) The docs for `--llvm --firmware`.
- [`268c392`](https://github.com/zeno-src/zeno-lang/commit/268c392578aabfef5fd0c1b221921a128c2088d8) `--llvm` with `--no-collector` and `--memory`: the LLVM backend frees owned values and can leave the collector out.
- [`b330867`](https://github.com/zeno-src/zeno-lang/commit/b330867e8ea80949935eb9adc1cf0f2725816d5c) The docs for `--llvm` with `--no-collector` and `--memory`.

### No collector: a pool, text that is kept, and one owner for every value

*14 to 15 November 2026*

- [`2788a33`](https://github.com/zeno-src/zeno-lang/commit/2788a33f1cdbce0fb25bfae3f013241882906fbf) With no collector, memory comes from one fixed pool and not from `malloc`. Taking a block and giving it back take constant time, and the pool cannot break up.
- [`0e89346`](https://github.com/zeno-src/zeno-lang/commit/0e893465eb6d8a8dc70e85df6bbeaa1338610283) The docs for the memory pool.
- [`e07dc43`](https://github.com/zeno-src/zeno-lang/commit/e07dc43632d633463dbb4c7dec081f45440fb7cb) With no collector, text can be kept: whatever stores a text has a copy of its own and frees it.
- [`54031ad`](https://github.com/zeno-src/zeno-lang/commit/54031ad5962d32b6a1480cf2fa0905920a105b17) The docs for text that is kept, with an example.
- [`ce6bf6a`](https://github.com/zeno-src/zeno-lang/commit/ce6bf6a1d25e8e0a325dfac743342e420385f6f3) A value made in the middle of a statement and given to nobody belongs to the statement, which frees it. So `split`, `map`, `filter` and `+` on lists work with no collector.
- [`418337a`](https://github.com/zeno-src/zeno-lang/commit/418337a349f32ab7e34432b95eddee2245ca161d) The docs for what a statement owns.
- [`51ebc9b`](https://github.com/zeno-src/zeno-lang/commit/51ebc9bd977dae2d5fb9bb88b75db963de7332bd) With no collector, every list and object has one owner without the word `owned`, and a list owns its items.
- [`ac76f2f`](https://github.com/zeno-src/zeno-lang/commit/ac76f2ff24842cf3fc5794b575cfbb6be15f244e) A list cannot fill a type parameter where it has one owner, and the message for it is put right.
- [`96abe6b`](https://github.com/zeno-src/zeno-lang/commit/96abe6b392fa4b73f621bf23de84c1742c129581) A walkthrough of one owner.
- [`a06f534`](https://github.com/zeno-src/zeno-lang/commit/a06f534bc627070029fe022fbf8f0dd8a2bb67cb) Decision 5.15: whoever receives a value owns it.
- [`00cb81c`](https://github.com/zeno-src/zeno-lang/commit/00cb81cdc351793d1afd8f1322c90824132d865d) The older pages are brought up to date with one owner.
- [`d01a741`](https://github.com/zeno-src/zeno-lang/commit/d01a74152814313207f703951aca5be1c3784182) An owner cannot be given a new value while a loop walks its list: a way to read freed memory is closed.
- [`49c97d8`](https://github.com/zeno-src/zeno-lang/commit/49c97d85d304e89c698ef1f7c779afccfd3ea890) With no collector, enum values and values that may be `none` have one owner too.
- [`f3b8eff`](https://github.com/zeno-src/zeno-lang/commit/f3b8effea1c62faf9badea415c36dacea822b739) The docs for enums and `none` with one owner.
- [`082126b`](https://github.com/zeno-src/zeno-lang/commit/082126b5cb5dc6a9071793317b6dc176282b3757) The checker gives errors and function values one owner with no collector. They were the last two kinds of value without one.
- [`2cd94d1`](https://github.com/zeno-src/zeno-lang/commit/2cd94d14a7fbdaa4f70d26493827d91a272d6eca) Both backends free errors and function values through their owner.
- [`a626e31`](https://github.com/zeno-src/zeno-lang/commit/a626e312e25ecc7ad476574044cc582daaabec4a) The docs for functions and errors with one owner.
- [`914cc99`](https://github.com/zeno-src/zeno-lang/commit/914cc9984ef16dba654f284948a7e8fe9f7aea93) The older pages and the README are brought up to date.

### Memory by hand

*15 November 2026*

- [`9efd0f8`](https://github.com/zeno-src/zeno-lang/commit/9efd0f855d6e65724962447b5731524c513bae0f) Memory by hand: `unsafe` blocks, raw pointers, `allocate` and `free`, and registers at a fixed address. `zeno run` checks every use; a compiled program, like C, checks none.
- [`d8e411a`](https://github.com/zeno-src/zeno-lang/commit/d8e411a6b617455f90977c566ba19be8b8b1f644) An `unsafe` block that ends in `return` ends the function: the C backend wrote a second `return` after it.
- [`c3a1d12`](https://github.com/zeno-src/zeno-lang/commit/c3a1d129cdd259430f2bfe85fb26b2ab5cc10bc5) A walkthrough of memory by hand, and its reasons.

### The audit: is every rule unambiguous and intuitive?

*15 November 2026*

- [`0714358`](https://github.com/zeno-src/zeno-lang/commit/071435878f76a078bb565a24bc223f23d2dedd4c) A plain parameter only looks at a value, and `take` marks the one that takes it over. `borrow` is gone.
- [`a4c2d5f`](https://github.com/zeno-src/zeno-lang/commit/a4c2d5f9a7d861aaddb035db1905249b7ee89d4c) The docs follow that change, and `docs/audit.md` begins.
- [`41cef08`](https://github.com/zeno-src/zeno-lang/commit/41cef088985662edd50bceccbba93cdd2331a17f) Messages count in words: "1 argument", "2 arguments", and never "argument(s)".
- [`cc2853a`](https://github.com/zeno-src/zeno-lang/commit/cc2853a98ecd35fbd002bbebc8b258a7bf7680ef) `[1] + ["two"]` is refused, as `append` already refused it.
- [`40258d2`](https://github.com/zeno-src/zeno-lang/commit/40258d292e5a8b39011375fb57785721c5f8cf1c) After `case none`, the later cases of a `match` know the value is there.
- [`eb5e05b`](https://github.com/zeno-src/zeno-lang/commit/eb5e05ba85247d82ff603354682ff01b7072c406) With no collector, every function can look at a `let` at the top of the file, which is how firmware keeps a table.
- [`8d232e1`](https://github.com/zeno-src/zeno-lang/commit/8d232e1ed8741fc1e067b05c2beea0a40bb07718) A method can take out what its own object holds, as a queue does with `pop`.
- [`a0e4a6c`](https://github.com/zeno-src/zeno-lang/commit/a0e4a6c3a24c888dcec496b9c6002f40123f3d2b) A `var` that holds a function of the program can be given another function.
- [`d9e6c8b`](https://github.com/zeno-src/zeno-lang/commit/d9e6c8b9461de24a7c0a908a5267643db95bbc44) A firmware build refuses a `raises` it cannot honour, and the audit is written up.

### The library

*15 November 2026*

- [`f4bf63a`](https://github.com/zeno-src/zeno-lang/commit/f4bf63a9429a459e9439b1a07945fb6d44a674cf) Text and lists get the methods every program reaches for: `find`, `slice`, `insert`, `reverse`, `sort`, `join` and more.
- [`26830bd`](https://github.com/zeno-src/zeno-lang/commit/26830bdc05ee74652104bdf4d566a2ec0c27044c) The docs for the first part of the library.
- [`92c2197`](https://github.com/zeno-src/zeno-lang/commit/92c2197906b575d34387943ab34b3d8fc1ac5815) Programs read and write: files, the keyboard (`input`), and the arguments they were started with.
- [`5163d9e`](https://github.com/zeno-src/zeno-lang/commit/5163d9e13e1e1fc16f38f06d6387ba1fca8e84bd) The docs for reading and writing.
- [`36a4e17`](https://github.com/zeno-src/zeno-lang/commit/36a4e1752885ce4e712bc5bfb1edcc761b91ee97) More of `math`: angles with units (`30 deg`), logarithms, `exp` and rounding.
- [`683df64`](https://github.com/zeno-src/zeno-lang/commit/683df640f513a5c822a98a4ef4f4ca0156433d3c) The docs for the new `math`, with an example of five instruments.
- [`689e9cf`](https://github.com/zeno-src/zeno-lang/commit/689e9cf4530bc4f9e5c4faf7ad111f98894daa5d) A dictionary type: `["Ada": 36]`, `[:]` for an empty one, and the type `dict[str, int]`.
- [`8fb13a3`](https://github.com/zeno-src/zeno-lang/commit/8fb13a3a044bde1e2809a963240dccc8ca1f77f3) The docs for the dictionary.

### Bigger programs: modules, packages, a formatter and a language server

*15 November 2026*

- [`5275c4d`](https://github.com/zeno-src/zeno-lang/commit/5275c4d97d7165b5ffab0ba84c318a21c88d4275) Programs in several files: `import shapes` reads `shapes.zeno` as a module.
- [`65b4bd5`](https://github.com/zeno-src/zeno-lang/commit/65b4bd531863a171ce2968a9d98530ddc90a6f50) The docs for programs in several files.
- [`54d4554`](https://github.com/zeno-src/zeno-lang/commit/54d4554827abc9ce7aa92ceb5493104084c422be) Packages: a `zeno.package` file says what a program needs, and `zeno fetch` gets it from git.
- [`e235d96`](https://github.com/zeno-src/zeno-lang/commit/e235d96aec055c7b976d20b9e73fb0d75ec59c3d) The docs for packages.
- [`49209cb`](https://github.com/zeno-src/zeno-lang/commit/49209cb37a111110794a2d4859b1ee3e965f6068) `zeno format`: one layout for every Zeno file. It changes white space only, and checks that the program still means the same.
- [`bc6727e`](https://github.com/zeno-src/zeno-lang/commit/bc6727ead9c8506cf5d282c3b009f9e399747a45) The docs for the formatter.
- [`d2a1edd`](https://github.com/zeno-src/zeno-lang/commit/d2a1edd5901ef9e273d827e648bcf46db8113d68) `zeno lsp`: a language server, so that an editor shows mistakes as you type, with setup for VS Code, Vim, Neovim and Helix.
- [`580e838`](https://github.com/zeno-src/zeno-lang/commit/580e8384755f059e62c65ed94f8e2fef444881ca) The docs for the language server.
- [`3d651df`](https://github.com/zeno-src/zeno-lang/commit/3d651dff33a6da7807a8c9c5ac3936ad470d446e) The language server answers "go to declaration" and hover, from what the checker knows.
- [`1040b8e`](https://github.com/zeno-src/zeno-lang/commit/1040b8e9eb9432bb6442adca43b20f0b820aec1c) The docs for how the server knows what a name means.

### The finish line for 0.1

*15 November 2026*

- [`1acd4f0`](https://github.com/zeno-src/zeno-lang/commit/1acd4f09943a1b8a2a34b7dc0e4edd4523ea0e59) `private`: a declaration at the top of a file can be kept to that file.
- [`c1cbf21`](https://github.com/zeno-src/zeno-lang/commit/c1cbf21202a84bad481b3b9531e7fb73da6878c6) `private` fields and functions of a class.
- [`9f9ae5d`](https://github.com/zeno-src/zeno-lang/commit/9f9ae5d704bed97a8494a14aa27764cad88d26c5) An example for `private`: a queue of a fixed size.
- [`20f0423`](https://github.com/zeno-src/zeno-lang/commit/20f04230cd19e0dfd8afa37b96b8d5d171bb6ff0) The docs for `private`, and the list of what is left for version 0.1.
- [`dcaef4e`](https://github.com/zeno-src/zeno-lang/commit/dcaef4e7c0d1998972cb7cdef250f152e9cb9dfc) Interfaces: `interface Shape { ... }` and `class Circle is Shape`.
- [`04e9189`](https://github.com/zeno-src/zeno-lang/commit/04e918908f9ed98967d8b6deb29b56e051f1e2e8) The docs for interfaces.
- [`d93b0b5`](https://github.com/zeno-src/zeno-lang/commit/d93b0b53abd22c435c2047877f34ef0c359e91b3) A dict can hold any kind of value, and a loop can have two names: `for key, value in ages`.
- [`e9fc668`](https://github.com/zeno-src/zeno-lang/commit/e9fc6682b95841bd91beb9c07d783a6ea2ec18b2) The docs for both.
- [`be3c6a6`](https://github.com/zeno-src/zeno-lang/commit/be3c6a69cc98e391eb9c1672792fe5992f256bc6) `zeno test`: the tests of a program are written in Zeno, with `test "name" { ... }` and `assert`.
- [`6d3c3a3`](https://github.com/zeno-src/zeno-lang/commit/6d3c3a30eb8a59304543cb72d03a7ea55f4f8cc2) The docs for tests.
- [`0f8d365`](https://github.com/zeno-src/zeno-lang/commit/0f8d365d4d8e7293b775779f487ba7c0dc7e309e) `zeno new` makes a new program or package in one command.
- [`6f353bb`](https://github.com/zeno-src/zeno-lang/commit/6f353bb696d0166619fcae8c5798dea7ebee7987) The docs for `zeno new`.
- [`34078de`](https://github.com/zeno-src/zeno-lang/commit/34078de73031210eb59c5ec8f8b1e5459f4a5895) The last gaps: `+=` on a sized number, a format on a quantity (`{angle:.1f}`), the operators `//=`, `%=` and `**=`, and two mistakes in the order of `x += f()` mended.
- [`eb9c706`](https://github.com/zeno-src/zeno-lang/commit/eb9c706ba995057f7134f9838bb4f2fc3028e954) The docs for the last gaps.
- [`0700a2e`](https://github.com/zeno-src/zeno-lang/commit/0700a2e223c550aac22af87acb3b32553280a130) The version is 0.1.0: the list for it is done. Also a clearer message for `as` after an import.
- [`702a6de`](https://github.com/zeno-src/zeno-lang/commit/702a6dedc333fce6ff34d3f1b257cbd9b6163468) Every decision once marked provisional is read again and settled, and `docs/version-0.1.md` says what 0.1 is.

### The Zeno Book, and a release for ten kinds of machine

*15 November 2026*

- [`08d0d83`](https://github.com/zeno-src/zeno-lang/commit/08d0d83c2e8eb359f5db3cf34d9e568372dafbed) The Zeno Book begins: its LaTeX source, its build, and the first chapters.
- [`c531678`](https://github.com/zeno-src/zeno-lang/commit/c531678311f2a4056da698bb50be71181cddbd34) Book: values, numbers, text, control flow, functions, lists and dicts.
- [`e2d3ec3`](https://github.com/zeno-src/zeno-lang/commit/e2d3ec3281aa647f702aa1893632e69dad435be6) Book: classes, enums, `none`, errors and functions as values.
- [`f5807d4`](https://github.com/zeno-src/zeno-lang/commit/f5807d4269248d1f92510f78c94be0afe6093374) Book: types, units, sized numbers and bits.
- [`0b2272f`](https://github.com/zeno-src/zeno-lang/commit/0b2272feed5e375aa44eebed28c5bcfc8a796624) Book: generics, interfaces, modules and packages.
- [`4ca6d11`](https://github.com/zeno-src/zeno-lang/commit/4ca6d11864f4a92e568ac934ee6ff3e01e684030) Book: reading and writing, and mathematics.
- [`cdfed0c`](https://github.com/zeno-src/zeno-lang/commit/cdfed0c6020090675050472689373aa0d9299d4a) Book: testing and the tools.
- [`4443627`](https://github.com/zeno-src/zeno-lang/commit/4443627f7161a32fd40fd459f19f99037536f88e) Book: the board and compiling.
- [`0322055`](https://github.com/zeno-src/zeno-lang/commit/0322055eb3507905158af2ec84c99e8bad3d7998) Book: memory, ownership and firmware.
- [`3762242`](https://github.com/zeno-src/zeno-lang/commit/3762242a9fd17b311eddcc8c5959c09b154d0d02) Book: building with no collector, and unsafe code.
- [`f831a99`](https://github.com/zeno-src/zeno-lang/commit/f831a99915699434ab02729a43ccb463b39c2703) The release workflow: a tag builds `zeno` for ten kinds of machine (Linux, macOS and Windows, on several processors each) and checks every build.
- [`4e090eb`](https://github.com/zeno-src/zeno-lang/commit/4e090ebbe5405fb8f3a6cc94dea2644d8f03e230) Book: Inside Zeno, and why Zeno is as it is.
- [`ddf739e`](https://github.com/zeno-src/zeno-lang/commit/ddf739e2ac263d2e7ff5a84ea75816780ccb700f) The first trial of the release found that a compiled program rounded floats differently on a Mac. Compiled programs now round as `zeno run` does on every processor.
- [`903a7dd`](https://github.com/zeno-src/zeno-lang/commit/903a7ddcbbad6a1997b6971b1eb551fe541e9ab8) Book: the appendices, and the first full build: 226 pages of PDF and a page of HTML for each chapter, with all 300 samples run by the compiler.
- [`3751da3`](https://github.com/zeno-src/zeno-lang/commit/3751da321e43fbdf472c5a7930e80261fc2610dd) Release workflow: the C build and the LLVM build are each checked in a step of their own.
- [`ee19a64`](https://github.com/zeno-src/zeno-lang/commit/ee19a6459980f6c22c80ae70a720b8a3f67288e6) Release workflow: what a trial finds is written on the summary page of the run.
- [`814b967`](https://github.com/zeno-src/zeno-lang/commit/814b9673adbb22a4afbc0d048864284e85fc7467) The docs for how a release is made and how to install one.
- [`c07f157`](https://github.com/zeno-src/zeno-lang/commit/c07f157a5034b097a7d26b31b0ab591d0dfacd5f) Release workflow: a release can be made from the Actions page on GitHub.
- [`642c040`](https://github.com/zeno-src/zeno-lang/commit/642c040a16d0aa30eecf7112ddcb58f6555b44ad) Writing the book found programs that `zeno check` accepted and that failed later. The checker now refuses them, and several messages say more.
- [`797556b`](https://github.com/zeno-src/zeno-lang/commit/797556b3bde10c8c5b7328e484b96394f7d8cc9f) Book: the samples follow the checker's new refusals.
- [`1213f69`](https://github.com/zeno-src/zeno-lang/commit/1213f697a7d1ab77a275b6d7bed3eb289295c07d) Windows: a compiled program sleeps and reads the clock the Windows way, so `--llvm` builds there.
- [`da45a66`](https://github.com/zeno-src/zeno-lang/commit/da45a6689479265a339d5cc6e58769798db29134) A path that Zeno makes is written with `/` on every system.
- [`6b3ed16`](https://github.com/zeno-src/zeno-lang/commit/6b3ed160b97946a0f1f9a80a846bc68e5f54609c) Windows: arguments arrive whole in any script (a word in Tamil had come out as `?????`), and a fetched package keeps its line ends.
- [`9385ec0`](https://github.com/zeno-src/zeno-lang/commit/9385ec0b978df48c745a3bf1fe6085995ed5b555) The mend for Windows arguments is done again without the heap, so that a build with no collector stays without one.
- [`7cc9fe9`](https://github.com/zeno-src/zeno-lang/commit/7cc9fe96fd8d933677a9fa95d7e5f9f5679f78f6) The docs for what Windows found. Every test passes there now.
- [`be2248e`](https://github.com/zeno-src/zeno-lang/commit/be2248e3e98a840ea44546b9fc2541d9c7015033) Three clearer messages that the book asked for.
- [`446d355`](https://github.com/zeno-src/zeno-lang/commit/446d35525cf6ac393eec547ce388cd53b6bd5f23) Book: Zeno builds on Windows too. **This commit is Zeno 0.1.0.**

## Zeno 0.2.0

*Released 16 November 2026 · 19 commits · [the release on GitHub](https://github.com/zeno-src/zeno-lang/releases/tag/v0.2.0)*

A Zeno program can be a web page, and Zeno gets installers. `zeno build
--web` puts a program, its page and Zeno itself (built as WebAssembly) into
one HTML file that a browser opens. The same days brought a setup program
for Windows and one line to paste for macOS and Linux, packages for apt,
pacman, snap, flatpak, Homebrew and winget, a `json` module, the first
playground, and the MIT licence. With this release the repository became
public.

### A program in a web page

*15 November 2026*

- [`64e8f27`](https://github.com/zeno-src/zeno-lang/commit/64e8f27538db2ee72b9ca5bc6fb164914911a935) The `web` module and `zeno build --web`: a Zeno program becomes one HTML file that a browser opens. Zeno itself runs in the page, built as WebAssembly.
- [`90cc566`](https://github.com/zeno-src/zeno-lang/commit/90cc5660eb21744ba50d2ddd6e56640fb796db0d) The version moves to 0.2.0, with its release notes.
- [`5a2f9e2`](https://github.com/zeno-src/zeno-lang/commit/5a2f9e25a13d92551cf96c6ba937ac30059366e1) Book: install steps for ten machines, and a new chapter on web pages.

### Installers, packages, JSON and a playground

*16 November 2026*

- [`e3a17a1`](https://github.com/zeno-src/zeno-lang/commit/e3a17a169d5362826f17b0eb0d55e73f6c25bb61) Installers: a setup program for Windows, and one line to paste for macOS and Linux. `zeno build` finds the C compiler by itself.
- [`6b9361f`](https://github.com/zeno-src/zeno-lang/commit/6b9361f77bae69fed820a6da89236f9778a35ded) A page can do more: events, styles, drawing on a canvas, text kept between visits, and the network. `zeno playground` writes a page in which programs are written and run in a browser.
- [`c0a3563`](https://github.com/zeno-src/zeno-lang/commit/c0a3563a988f6f055a9138c6ee0360c84ec9b439) What 0.1 left: `min` and `max` of quantities, hover on a field in an editor, and `zeno test --only` to run tests by name.
- [`a0a1254`](https://github.com/zeno-src/zeno-lang/commit/a0a1254dbc406ac39dade449979cdb9cacc22775) A workflow adds installers to a version that is already released, and 0.1.0 gets both.
- [`b200af8`](https://github.com/zeno-src/zeno-lang/commit/b200af8ec7749f58ede4a44ff2c1d29598651a9e) The `json` module: `json.parse` and `json.write`.
- [`d60f0bd`](https://github.com/zeno-src/zeno-lang/commit/d60f0bd0af9ac7f429512bd70995b31faee31035) Book: the bigger `web` module, JSON, the installers, and tests by name.
- [`be32a88`](https://github.com/zeno-src/zeno-lang/commit/be32a88abc81c2c059c8577febf3a801194a5d4e) Packages: a `.deb`, a snap, a flatpak, a pacman package, and recipes for Homebrew and winget.
- [`4581bd7`](https://github.com/zeno-src/zeno-lang/commit/4581bd7ac22a228ad47dd0fbdd3b3e4722bddad4) Packages: the PKGBUILD goes into its folder and not into the archive beside it.
- [`0fe0238`](https://github.com/zeno-src/zeno-lang/commit/0fe0238ed3ba1541735691880a1c1896d052b908) A release now makes the packages and tries each one. Their walkthrough and their reasons.
- [`051062b`](https://github.com/zeno-src/zeno-lang/commit/051062b19761f01eef37da28d5b7da76fe12a29b) A timer that can be stopped: `web.after` and `web.every` give a `Timer`.
- [`66a1d5e`](https://github.com/zeno-src/zeno-lang/commit/66a1d5e5754dc9683765b43ab6d58bd678a929d8) The Homebrew, pacman and winget recipes for 0.1.0, with its checksums.
- [`1c8fce9`](https://github.com/zeno-src/zeno-lang/commit/1c8fce923f72f38609f8f09462001e5a4ae9b661) Book: the timer, and the whole `web` module in the library appendix.
- [`39d70e6`](https://github.com/zeno-src/zeno-lang/commit/39d70e63afef96c923d7c5e4a1e29726fd08d6ed) Book: installing with apt, pacman, snap, flatpak, Homebrew and winget.
- [`f998247`](https://github.com/zeno-src/zeno-lang/commit/f998247740e739ce41b71754179f888d10963477) The playground is made ready for zeno-lang.org: an endless loop is stopped after 5 seconds, errors show under the output, and it fits a phone.

### A licence, and a public repository

*16 November 2026*

- [`db80267`](https://github.com/zeno-src/zeno-lang/commit/db80267499ba1914d7ed52bfac825d423d43c61f) Zeno gets the MIT licence, in the repository and in everything a release carries.
- [`2367c89`](https://github.com/zeno-src/zeno-lang/commit/2367c89d375a1f74d9ea27f9ee3aa8dd313c4af3) The repository is made public, and the docs no longer say that the install line cannot download. **This commit is Zeno 0.2.0.**

## Zeno 0.3.0

*Released 16 November 2026 · 32 commits · [the release on GitHub](https://github.com/zeno-src/zeno-lang/releases/tag/v0.3.0)*

This release is about speed. The garbage collector is rebuilt around pages
and then given two generations, a program can ask for `memory owned` and
have no collector, `--release` drops the overflow checks, and the VM gets
faster. Six benchmarks against C, Rust and Python are added to measure all
of it. Also here: `sort` with a function, a new playground with a real
editor, and the groundwork for the package stores.

The rehearsal of the release hung on macOS, so 0.3.0 went out with the two
generations on Linux only. The cause was found in 0.4.0.

### After 0.2.0: sort with a function, a new playground, and the package stores

*16 November 2026*

- [`9a48258`](https://github.com/zeno-src/zeno-lang/commit/9a4825899396c0b70a9d69396b96cb8ac0f6e08e) The Homebrew, pacman and winget recipes for 0.2.0, with its checksums.
- [`8581065`](https://github.com/zeno-src/zeno-lang/commit/85810650ca164482bc365dacfc9074d48417b41e) Book: 0.2.0 is released, and chapter 1 installs it.
- [`6b54f7d`](https://github.com/zeno-src/zeno-lang/commit/6b54f7d08fead106cd2cff8f4e0ccde43c0e7220) `sort` with a function: `people.sort(age_of)` orders a list of anything by a key. A crash in `map`, `filter` and `reduce` is mended.
- [`9741037`](https://github.com/zeno-src/zeno-lang/commit/97410374e57121f90fd3abdeaf19e6b4d0f57db8) Book: `sort` with a function.
- [`4cea297`](https://github.com/zeno-src/zeno-lang/commit/4cea297de2df3c312cd0c5a33bc4aff92f657797) A new playground: an editor in colour that checks as you write, a Format button, problems that jump to their line, and fifteen examples.
- [`9c9a488`](https://github.com/zeno-src/zeno-lang/commit/9c9a488b2eb7325598deb70427f77edee1300937) The stores workflow: signed lists for apt, the files for the AUR, and winget files in the current form.
- [`bad5ee2`](https://github.com/zeno-src/zeno-lang/commit/bad5ee257a8d9fef49abf939584acbe5b66e9c2f) The sketch example can be drawn with a finger on a phone.
- [`3ad0c5e`](https://github.com/zeno-src/zeno-lang/commit/3ad0c5e705fd695d4b8c738fdb2ae78029d95cb4) On Arch Linux the package is named `zeno-lang-bin`, because the AUR already has another program called zeno.
- [`90558ce`](https://github.com/zeno-src/zeno-lang/commit/90558cea1b1e213a8ab709dfa0af4d3420b85698) The docs for the stores: where each package manager looks for Zeno, and what is left to do for each.
- [`6640134`](https://github.com/zeno-src/zeno-lang/commit/664013446427dbb39513dc707cc8604db8022df5) Book: the AUR name, and no Flathub.
- [`a9ba762`](https://github.com/zeno-src/zeno-lang/commit/a9ba7620f02a54ba1a8cd336db71878d7516ebd4) Book: a line that ran into the margin is mended.

### Speed: a faster collector, a choice of memory, and benchmarks

*16 November 2026*

- [`8d205de`](https://github.com/zeno-src/zeno-lang/commit/8d205deed537ebba00242389ff9ea61aed31fe3d) A faster collector: blocks live in 64 KB pages that are found through a hash table. One benchmark went from 57.8 seconds to 12.0.
- [`9219ac5`](https://github.com/zeno-src/zeno-lang/commit/9219ac59109348532e7e1bbb62f5f71b307c43db) A program chooses how its memory is freed: `memory owned` (one owner for each value, no collector) or `memory collector`. A `--release` build drops the overflow checks on whole numbers.
- [`d11af73`](https://github.com/zeno-src/zeno-lang/commit/d11af7353abe2991f2833f30ac603529453c62be) Benchmarks: six programs in Zeno, C, Rust and Python, and a script that runs and checks them. A walkthrough of the memory modes.
- [`1321033`](https://github.com/zeno-src/zeno-lang/commit/13210335e19b7cddf26eb9a9aed87e71ee8427f5) winget: the recipe files in the newer form that Microsoft's list asks for.
- [`8fc4af8`](https://github.com/zeno-src/zeno-lang/commit/8fc4af8a764a855a6486183d7aded93320694aee) Book: `memory owned`, `--owned`, `--collector`, `--release` and the paged collector.
- [`3d1c6db`](https://github.com/zeno-src/zeno-lang/commit/3d1c6dbb3bec7ef06ff1cb6415288431cacc7237) Benchmark results after those changes.
- [`dca4974`](https://github.com/zeno-src/zeno-lang/commit/dca4974f5d9cf0ec3e2651fa3a60c75af9aadf24) `for ch in text` walks the text and no longer makes a list of its characters first.
- [`79252ad`](https://github.com/zeno-src/zeno-lang/commit/79252adfa677a44c0a59d453838dae3ac01e7a21) A field that a function never writes is read once, and `math.sqrt` is inlined in the LLVM build.
- [`8fba764`](https://github.com/zeno-src/zeno-lang/commit/8fba764f610cc3d3bce6d2f538d1181ff69404da) Benchmark results after those two.
- [`9f0f67a`](https://github.com/zeno-src/zeno-lang/commit/9f0f67a6ed7bda9782a2ac111c13abede6ebf885) `memory owned` at the top of a file makes the classes of that file owned, also in a program with a collector.
- [`e15264e`](https://github.com/zeno-src/zeno-lang/commit/e15264e63aaaa588313e66f4775563f970b22d8e) The playground shows the C that `zeno build` writes for the program, in a tab of its own.
- [`49fb9dc`](https://github.com/zeno-src/zeno-lang/commit/49fb9dccad9af64482921c84d1983e1d50e55fea) The collector gets two generations, so that it need not go over old blocks at every collection. A leak in lists is ended on the way.
- [`54c9758`](https://github.com/zeno-src/zeno-lang/commit/54c9758f7023b2471a628a8038cdc10452504efc) Benchmark results after generations, and a script that keeps the times of each version.
- [`09fe44c`](https://github.com/zeno-src/zeno-lang/commit/09fe44ca79a058dfb6458bc8d5560eff3327b095) The VM gets faster: instructions joined into one, quick paths for int and float arithmetic, and fields found by their place.
- [`9a04d9f`](https://github.com/zeno-src/zeno-lang/commit/9a04d9f701b95247cba368524eaa3f384b6696f9) The playground takes the one accent colour of the website, blue.
- [`40cfa4e`](https://github.com/zeno-src/zeno-lang/commit/40cfa4e0c521787014def56d566301103db11bb0) Benchmark records for 0.1.0 and 0.2.0, to compare later versions with.

### Releasing 0.3.0, and a hang on macOS

*16 November 2026*

- [`06d0195`](https://github.com/zeno-src/zeno-lang/commit/06d019532a37c041b243cc58039211eb6889d6dd) Release notes for 0.3.0.
- [`1ff51bf`](https://github.com/zeno-src/zeno-lang/commit/1ff51bfa918cb031e54951c1aca7b6daf2500967) The rehearsal of 0.3.0 hung in the tests on macOS. Every workflow job gets a time limit, and a probe workflow is added to look into the hang.
- [`9630500`](https://github.com/zeno-src/zeno-lang/commit/96305003faa7e61bcd043e1ea1412e4ea6270fc8) Until the hang is understood, the two generations are used on Linux only.
- [`c920cd8`](https://github.com/zeno-src/zeno-lang/commit/c920cd860655c62168774154bb1fc51c0177ddbc) The probe runs the test suite under a clock on macOS and says where a hung program is.
- [`ca486cd`](https://github.com/zeno-src/zeno-lang/commit/ca486cd23c93e878ddae0ac86cdb6ce4485da030) The release notes say what 0.3.0 has: generations on Linux, and the paged collector on every system. **This commit is Zeno 0.3.0.**

## Zeno 0.4.0

*Released 16 November 2026 · 18 commits · [the release on GitHub](https://github.com/zeno-src/zeno-lang/releases/tag/v0.4.0)*

`zeno build --fast` is a build that runs as fast as C and Rust on the
benchmarks. The release also ends the hunt for the hang on macOS that held
0.3.0 back. It was a `try` that jumped with two C functions that do not
match on a Mac. Once that was mended, the collector's two generations came
back on macOS.

### Finding the hang on macOS

*16 November 2026*

- [`354aea1`](https://github.com/zeno-src/zeno-lang/commit/354aea1d97061f0613d12a6fb10d111777a25f3c) The Homebrew, pacman and winget recipes for 0.3.0, with its checksums.
- [`ba8aa27`](https://github.com/zeno-src/zeno-lang/commit/ba8aa278c6c7fc45b2ca6284b5a1713650b16a41) The benchmark record of 0.3.0.
- [`393212a`](https://github.com/zeno-src/zeno-lang/commit/393212a5b416e436a6a80a786a56d29b714c023b) The release had left the AUR's `.SRCINFO` file at 0.2.0, because a workflow drops files whose names begin with a dot. Mended.
- [`514613a`](https://github.com/zeno-src/zeno-lang/commit/514613a707dd590adc5fd9103b77aa45e98cfff6) The probe asks three questions of the runtime on macOS, to narrow the hang down.
- [`9e28b91`](https://github.com/zeno-src/zeno-lang/commit/9e28b91b5c39a22bb557069d72a7e26c4afef1f4) The probe runs the two hung tests with a runtime that writes what it does, and looks at a hung program with a debugger.
- [`aae13f0`](https://github.com/zeno-src/zeno-lang/commit/aae13f0624d85a2a29de469635dd17781d4318bb) The probe tries the cause it suspects: an LLVM build enters a `try` with `_setjmp` and `raise` leaves it with `longjmp`, and on macOS the two do not match.
- [`e548e7e`](https://github.com/zeno-src/zeno-lang/commit/e548e7e3179914a39e4a5ed3f37ffe99ccbb67e4) The mend: a `try` jumps with the matched pair `_setjmp` and `_longjmp`. The mismatch had left blocked the signal that the collector's two generations rely on.
- [`8744490`](https://github.com/zeno-src/zeno-lang/commit/874449098e4b2dac8779820fe2d57d667169b865) The probe checks that the whole suite passes on macOS with two generations.
- [`3c270a3`](https://github.com/zeno-src/zeno-lang/commit/3c270a3c4b1598941a67f71a22baf015f3cc7bf6) The two generations are back on macOS.
- [`c57afed`](https://github.com/zeno-src/zeno-lang/commit/c57afed2b08ad3946dff04624a24d9dba7c5db9b) Decision 22.11 records that.
- [`2a2aaef`](https://github.com/zeno-src/zeno-lang/commit/2a2aaef38f280d0b50ebd3f38eae7ff518073aa7) `main` becomes 0.3.1, with notes on what was mended.

### `zeno build --fast`

*16 November 2026*

- [`6132e28`](https://github.com/zeno-src/zeno-lang/commit/6132e284639ab85794000ac17a355a6731cdac10) `zeno build --fast`: a build with no checks on indexes, float division or square roots, compiled with `-O3`. Numbers become text without `snprintf`, and big owned lists grow with `realloc`.
- [`b04f80f`](https://github.com/zeno-src/zeno-lang/commit/b04f80f8df3d05eea4b2aa60e769ae3e74ee2801) Benchmarks with `--fast`.
- [`3e492d1`](https://github.com/zeno-src/zeno-lang/commit/3e492d1597928c417544a1d2c80ca7806e49522c) The next version is named 0.4.0, since a new way to build is more than a mend.
- [`1aa503b`](https://github.com/zeno-src/zeno-lang/commit/1aa503b9fe574bd63715e36174071f7526343fd5) `--fast` uses C's own whole-number arithmetic, so the loop benchmark compiles to the same instructions as C. In every build, `//` and `%` by a power of two are a shift and a mask.
- [`af7a738`](https://github.com/zeno-src/zeno-lang/commit/af7a7386fb5f0bec63cc846209635832aeea448e) The benchmark script can count instructions, which is steadier than a clock, and timed runs go in rounds.
- [`5b44520`](https://github.com/zeno-src/zeno-lang/commit/5b44520eafc78d42cd75a91a0ecd66cc863e5c03) Decision 23.9: the overflow rule of `--fast` stays, with what dropping it would cost, measured.
- [`d6b3deb`](https://github.com/zeno-src/zeno-lang/commit/d6b3debcb1bb848b17d44c3f447998b500fcc5de) Release notes for 0.4.0, and the README and the docs name its files. **This commit is Zeno 0.4.0.**

## Zeno 0.5.0

*Released 17 November 2026 · 12 commits · [the release on GitHub](https://github.com/zeno-src/zeno-lang/releases/tag/v0.5.0)*

Zeno writes your program in another language. `zeno build --target` gives
the same program in JavaScript, TypeScript, Python, PHP, Go, Java, C#, C++
or Rust, as a web page, or as WebAssembly, and each prints what `zeno run`
prints. A review of every target found about thirty corners where a
language did what is usual there and not what Zeno does. Each got a rule,
and a test program that holds every target to it.

### Eleven targets

*16 to 17 November 2026*

- [`79cf832`](https://github.com/zeno-src/zeno-lang/commit/79cf8322e1e1693b09698e4d27ea8b5ec0e48662) The Homebrew, pacman and winget recipes for 0.4.0, with its checksums.
- [`e043682`](https://github.com/zeno-src/zeno-lang/commit/e04368283e4377f73aac838a28860f1e03335f32) The benchmark record of 0.4.0.
- [`52c030e`](https://github.com/zeno-src/zeno-lang/commit/52c030eff0e4fdc6f9dd2c003008c7093b5fb9d6) `zeno build --target js`, `html` and `py`: Zeno writes the program in JavaScript or Python, and a script holds every target to what `zeno run` prints.
- [`2a96152`](https://github.com/zeno-src/zeno-lang/commit/2a96152b69bb40cf16f0982ad92a098103f53ebe) Targets: Go, Java and C#.
- [`90d713b`](https://github.com/zeno-src/zeno-lang/commit/90d713bde510ea60538ff442a8d4a101e1a92d86) Targets: C++.
- [`8939c1d`](https://github.com/zeno-src/zeno-lang/commit/8939c1d4a7149b294ab2a7194b820ce6562a15c6) Targets: Rust and WebAssembly, and a weekly check of every target against the newest version of its language.
- [`925d162`](https://github.com/zeno-src/zeno-lang/commit/925d162dac16266cc689a34844e56d06c7d0cca3) Book: rebuilt with a section on `zeno build --target`.
- [`98c6603`](https://github.com/zeno-src/zeno-lang/commit/98c6603c5c02e956666c4fec67817026e589f8c4) Book: other languages are marked as new in 0.5.
- [`60c5416`](https://github.com/zeno-src/zeno-lang/commit/60c5416396a9ed8a7eed3a296a97ed3b272172d4) A review of every target found about thirty corners where a target did what its own language does and not what `zeno run` does: order, large numbers, text, JSON, names. Each now has a rule and a program that tests it.
- [`52796d2`](https://github.com/zeno-src/zeno-lang/commit/52796d2fafd76feb3593d4559f99165ac8533342) Targets: TypeScript and PHP.
- [`5e88303`](https://github.com/zeno-src/zeno-lang/commit/5e88303d8c23f25fea63a090c673ae557c5e639f) The version moves to 0.5.0, with its release notes.
- [`878afd3`](https://github.com/zeno-src/zeno-lang/commit/878afd350e835e34a9901ffa81318d486e241873) The benchmarks can time the program written in each other language. The notes of 0.5.0 say how its speed compares with 0.4.0. **This commit is Zeno 0.5.0.**

## Zeno 0.5.1

*Released 17 November 2026 · 6 commits · [the release on GitHub](https://github.com/zeno-src/zeno-lang/releases/tag/v0.5.1)*

A small release with one purpose. In 0.5.0, `zeno run --vm` had become a
few percent slower, although no line of the VM had changed: with more code
in the program, the Rust compiler had laid it out differently. 0.5.1 is
0.5.0 built as one piece, and the VM is as fast as it was in 0.4.0.

- [`655d4b7`](https://github.com/zeno-src/zeno-lang/commit/655d4b793c8f61ab0060694fdcdaf9eb17e985b3) The Homebrew, pacman and winget recipes for 0.5.0, with its checksums.
- [`662d727`](https://github.com/zeno-src/zeno-lang/commit/662d72732876c25f77dfb033a7487a6b55105aa4) Benchmarks: a program that stops without a word is reported as that, and the script no longer falls over.
- [`ffbed00`](https://github.com/zeno-src/zeno-lang/commit/ffbed008a140606f19eef7a6e9a471d25e79b116) The benchmark record of 0.5.0, with the times of the program in nine other languages.
- [`8f3da1d`](https://github.com/zeno-src/zeno-lang/commit/8f3da1d060a1b68067a045dba786cdde21ab33d5) `zeno run --vm` was 1 to 9 % slower in 0.5.0 with no line of it changed: the Rust compiler had cut the program up differently. A release is now built as one piece, and the version moves to 0.5.1.
- [`e19e4f7`](https://github.com/zeno-src/zeno-lang/commit/e19e4f765a3d0c27c56f4e705cb885377e69fc10) Release notes for 0.5.1.
- [`4acd512`](https://github.com/zeno-src/zeno-lang/commit/4acd51237ecc72a38e1652d279d812fdb85303d9) The README and the docs name the files of 0.5.1. **This commit is Zeno 0.5.1.**

## Zeno 0.5.2

*Released 18 November 2026 · 41 commits · [the release on GitHub](https://github.com/zeno-src/zeno-lang/releases/tag/v0.5.2)*

This release adds to what is around the language and leaves the language
as it was. There are four more target languages (Lua, Ruby, Dart and
Kotlin), and a registry, so that a package is found by its name. `import
python`, `import node` and `import java` let a Zeno program use the
libraries of those languages. `zeno build --minimal` writes the least C
that runs a program, `program.run` starts another program, and the first
part of the library is written in Zeno itself.

The repository moved to the zeno-src organization on GitHub. Two mistakes
were mended: a float `%` that the C++ target got wrong, and a caught error
that could stop a program built on Windows.

### After 0.5.1: records, the book, and where to write

*17 November 2026*

- [`bfae8a9`](https://github.com/zeno-src/zeno-lang/commit/bfae8a9dff61a57c0e9c3d0f8d1d9cf3a7d92ea7) The Homebrew, pacman and winget recipes for 0.5.1, with its checksums.
- [`e23b2b7`](https://github.com/zeno-src/zeno-lang/commit/e23b2b74de544a1f897620ff7d83378cf02e81ed) The benchmark record of 0.5.1. A run that its host crashed is run again.
- [`441f348`](https://github.com/zeno-src/zeno-lang/commit/441f3484aa313f47b48dc975d4cc1d0ca5c6e8b3) Book: rebuilt for Zeno 0.5, with eleven target languages.
- [`7817c2e`](https://github.com/zeno-src/zeno-lang/commit/7817c2efb13cf7ac74ee5fe58b5897a143c292fe) Book: the title page names the version the book is for.
- [`d027849`](https://github.com/zeno-src/zeno-lang/commit/d0278499af2f909cf31978e7a73ebc1f1fa99e7a) The install and release pages still named the files of old versions. They name 0.5.1 now.
- [`139f757`](https://github.com/zeno-src/zeno-lang/commit/139f7573876e82b167130f3f845d88811d03f91e) `SECURITY.md`: where to report a security problem in private.
- [`1380d3c`](https://github.com/zeno-src/zeno-lang/commit/1380d3c4d51a82e5c39485b1361ceab28ec11a93) An address at zeno-lang.org for each kind of mail: help, issues, security and contact.
- [`1f8a079`](https://github.com/zeno-src/zeno-lang/commit/1f8a079f0bb3e064d098858f6aa565f494f65c1e) Book: a section on where to write when you are stuck.

### Starting another program, and a new home on GitHub

*17 November 2026*

- [`4d1a392`](https://github.com/zeno-src/zeno-lang/commit/4d1a392b04310d9af4486bc31970c4b02cb6d784) `program.run(name, words)` starts another program and waits for it.
- [`fae846c`](https://github.com/zeno-src/zeno-lang/commit/fae846cf2ea948faa5ff66a3d843ab2fbfeaea12) An example that saves a video with yt-dlp through `program.run`, with the walkthrough and the reasons.
- [`f7611fd`](https://github.com/zeno-src/zeno-lang/commit/f7611fd9de47b5ef1f5e2c1d780e10dfc5f13bf4) The repository moved to the zeno-src organization on GitHub, and every link points there.

### Four more target languages

*17 November 2026*

- [`3e2c68f`](https://github.com/zeno-src/zeno-lang/commit/3e2c68fed20f4975d2609e199a2baa00823abdff) `zeno build --target lua` and `--target ruby`.
- [`ab67e56`](https://github.com/zeno-src/zeno-lang/commit/ab67e561a685214391796ff9f6f21d90ff06eb0c) `zeno build --target dart`.
- [`76aff9b`](https://github.com/zeno-src/zeno-lang/commit/76aff9b3556967c58c377e83b6c220e068ad9e34) `zeno build --target kotlin`, and a test program of awkward names for the four new languages.
- [`966a2ad`](https://github.com/zeno-src/zeno-lang/commit/966a2ad5691cccfa74ac81be4b9da72dee3f7e68) The docs for Lua, Ruby, Dart and Kotlin.
- [`dc82309`](https://github.com/zeno-src/zeno-lang/commit/dc8230908fbb1ecf65d22118ea90c310cbca1a71) Book: Lua, Ruby, Dart and Kotlin are on their way.
- [`17959ff`](https://github.com/zeno-src/zeno-lang/commit/17959ff263c254b427a27d135f2d3c896f944e88) How fast the four new targets run.
- [`c66ef68`](https://github.com/zeno-src/zeno-lang/commit/c66ef68d8bb18ed02c751811ae30975a3424e619) With a new compiler the C++ target printed a wrong float for `7.0 % 0.1`, and Go could do the same. Both are mended. The weekly check had stayed green with that failure in it, and now fails when a program differs.

### A registry, and the libraries of Python, JavaScript and Java

*18 November 2026*

- [`ce5760d`](https://github.com/zeno-src/zeno-lang/commit/ce5760dbaf0a76d47ace09684e8a38d81ebb13cc) A registry: a package is found by its name (`needs units at "v1.2.0"`), with `zeno search` and `zeno publish`.
- [`11eef17`](https://github.com/zeno-src/zeno-lang/commit/11eef173e763e4bfdc15ec6f4c7d8e5e2bc0c51a) `import python`: a Zeno program uses the libraries of Python, which runs beside it.
- [`6257ea6`](https://github.com/zeno-src/zeno-lang/commit/6257ea61f3865ad59e218f765cc5d2592689975d) An example that uses yt-dlp as a library through `import python`, with the walkthrough and the plan for other languages.
- [`3998c64`](https://github.com/zeno-src/zeno-lang/commit/3998c64d8f06cc7ac160ca15ec18ebd4dd4a244f) The docs for the registry, and the book's chapter on packages.
- [`a3fa4bd`](https://github.com/zeno-src/zeno-lang/commit/a3fa4bd40f9070366d4df04ff69256eddb76c443) Book: rebuilt with the registry.
- [`0b680e8`](https://github.com/zeno-src/zeno-lang/commit/0b680e8ac8a5653ecc36aa747c23eef1ac64e08a) `import node`: a Zeno program uses the libraries of JavaScript.
- [`32732c6`](https://github.com/zeno-src/zeno-lang/commit/32732c665fd4a30767501496f4cbcd95a4f4aa67) `import java`: a Zeno program uses the classes and jars of Java.

### The least C

*18 November 2026*

- [`8db60a7`](https://github.com/zeno-src/zeno-lang/commit/8db60a787ea7047300d23136f7912c0518cf2209) `zeno build --minimal`: the least C that runs the program, with no checks and only the parts of the runtime the program uses.
- [`1832b7b`](https://github.com/zeno-src/zeno-lang/commit/1832b7b6fe1c0eb3efe2b88bd6be813a2f9aea44) A script builds every test program with and without `--minimal` and compares the two.
- [`4652734`](https://github.com/zeno-src/zeno-lang/commit/4652734777f8e11a769655d21a2cbfa72b560cfe) The docs for `--minimal`.
- [`4aabf71`](https://github.com/zeno-src/zeno-lang/commit/4aabf71f112312d851679b42e519f06e10c689af) Book: the least C, in the chapter on compiling.
- [`9d15172`](https://github.com/zeno-src/zeno-lang/commit/9d151728a7a03f45d473a153537584a1c639a262) Book: rebuilt with it.
- [`028e557`](https://github.com/zeno-src/zeno-lang/commit/028e5577f920cd6d3c318c5ba2f0723622915734) The `--minimal` comparison leaves out one sample whose result C does not define (memory given back twice).
- [`49b0e49`](https://github.com/zeno-src/zeno-lang/commit/49b0e49971db7005d9da2901144d02166f0648c8) A field may have the name of a class or an enum: the C that `zeno build` wrote for it did not compile.
- [`1b87690`](https://github.com/zeno-src/zeno-lang/commit/1b8769072c1cc41d44de702d042ce3845be647a2) A `--minimal` test failed on Windows without a word. The trial workflow now builds such programs there and reports where one stops.
- [`2f4fbac`](https://github.com/zeno-src/zeno-lang/commit/2f4fbac4e9837d9eb6672c6358eaa3be4fb4d484) The trial also reports how the program ended, at each level of the C compiler's optimising.

### A list of what is missing, a mend for Windows, and a library written in Zeno

*18 November 2026*

- [`f907a95`](https://github.com/zeno-src/zeno-lang/commit/f907a951c2f1dcfb691c59fa61ae8fb35feccfdb) `docs/missing.md`: what Zeno does not have yet, in the order to build it, firmware first.
- [`b1b4d82`](https://github.com/zeno-src/zeno-lang/commit/b1b4d82739c3db013eb88a592584620e70b9e82b) Adding `import java` had made a sort on the VM a tenth slower, with no line of the VM changed. The code of the bridge is kept apart from the functions every program runs, and the speed is back.
- [`fbf4aff`](https://github.com/zeno-src/zeno-lang/commit/fbf4aff5c46172e1a8936950c785b02b550e9e26) The failure on Windows is found and mended: a caught error could stop a program built there with gcc. A `try` is now a plain jump.
- [`3e677b4`](https://github.com/zeno-src/zeno-lang/commit/3e677b47c958d384c481ed82f113e9e9e359fdae) Part of the library is now written in Zeno itself: `math.hypot`, `sinh`, `cosh`, `tanh`, `gcd` and `clamp`, and a `random` module. Only the functions a program uses are added to it.
- [`265c21d`](https://github.com/zeno-src/zeno-lang/commit/265c21d9390b8bf20429bddab903160d80b2b5bc) The docs for the library written in Zeno, with an example.
- [`0c48e5f`](https://github.com/zeno-src/zeno-lang/commit/0c48e5f0629dd6b7493374a30d32181b5f8e4a86) Book: the functions written in Zeno, in the appendices.
- [`596a800`](https://github.com/zeno-src/zeno-lang/commit/596a800e7a471fdc1e3cc01d327f14c755be6d2b) The version is 0.5.2, with its release notes. Its speed was compared with 0.5.1 first. **This commit is Zeno 0.5.2.**

## Zeno 0.6.0

*Released 19 November 2026 · 33 commits · [the release on GitHub](https://github.com/zeno-src/zeno-lang/releases/tag/v0.6.0)*

The release for firmware. The work follows
[`docs/missing.md`](docs/missing.md), the list of what Zeno still lacks,
with firmware first. A program can now read a board: a button on an input
pin, bytes on a serial port, a voltage on an analog pin. It has the numbers
firmware works in: `f32`, `u64`, lists of a fixed length, bytes, enums that
stand for numbers, and units made of units. Beside that, a Zeno program can
use the libraries of Ruby, PHP, Lua, .NET and C, and through C those of Go
and Rust, and it can end with a number for the shell.

The last hours before the release went on trying all of it on Windows and
macOS, and on making sure that a program which uses none of the new things
runs as fast as it did in 0.5.2.

### Ending a program, enums as numbers, and bytes

*18 November 2026*

- [`5135b88`](https://github.com/zeno-src/zeno-lang/commit/5135b88334320de91a99856bf65509c0a22412a8) The Homebrew, pacman and winget recipes for 0.5.2, with its checksums.
- [`d7c378a`](https://github.com/zeno-src/zeno-lang/commit/d7c378a6e198bf200358e2065dac50768cbd2d5f) `program.exit`, `program.environment` and `program.warn`. A case of an enum can stand for a number: `enum Reply { Ack = 6  Nak = 21 }`.
- [`c1715a8`](https://github.com/zeno-src/zeno-lang/commit/c1715a87d736982f1de86b548c67da3e366efb0c) Bytes from and to a file: `files.read_bytes` and `files.write_bytes`.
- [`446c228`](https://github.com/zeno-src/zeno-lang/commit/446c228ece86a44d0ec2ab650c8663cf658b617c) A `list[u8]` is stored one byte to an item. An 8 MB file read and written back takes 13 MB of memory where it took 64.

### The libraries of Ruby, PHP, Lua and .NET

*18 November 2026*

- [`0f38b05`](https://github.com/zeno-src/zeno-lang/commit/0f38b05e9895de1517b361d4542e25c90a8c8a0e) `import ruby`: the library of Ruby and its gems.
- [`216f9b0`](https://github.com/zeno-src/zeno-lang/commit/216f9b039bbb6361f1388114feaa5ed265d59f0d) `import php`: what comes with PHP, and Composer packages.
- [`efc5668`](https://github.com/zeno-src/zeno-lang/commit/efc566813157d039a1ea32d256670366a71be179) `import lua`: the library of Lua and its rocks.
- [`e84a926`](https://github.com/zeno-src/zeno-lang/commit/e84a926085208d475e77db8fc3fae499dcc2590c) `import csharp`: the types of .NET and the packages of NuGet.
- [`6c5b61d`](https://github.com/zeno-src/zeno-lang/commit/6c5b61d64fe44af8db0dafaa0dab20fbf33a18b5) The decisions for those four become chapters 37 to 40, since `main` had taken 34 to 36.

### Numbers for firmware: fixed lists, `f32`, `u64`, and units made of units

*18 November 2026*

- [`dacdeef`](https://github.com/zeno-src/zeno-lang/commit/dacdeefe3c3cd09d64e03b9c96fbb4a8819dfcc3) The benchmark record of 0.5.2.
- [`670f4a6`](https://github.com/zeno-src/zeno-lang/commit/670f4a6565e9d60c8539216caa971ce52e1607f1) A list of a fixed length, `list[u8, 64]`, and `[0] * 64` to fill one.
- [`b2f8ad4`](https://github.com/zeno-src/zeno-lang/commit/b2f8ad4984744290450480c3098058e6a2aaa007) `docs/targets.md` records the last check of the targets: 349 programs in each language.
- [`778b502`](https://github.com/zeno-src/zeno-lang/commit/778b5023d9d0345db0c27186ca064a7afe2ee9ee) `f32`, a float of 32 bits, as C's `float`.
- [`399d7af`](https://github.com/zeno-src/zeno-lang/commit/399d7af3d28595fb332bf37b3a51c3dc63ab3e32) Book: a chapter on the libraries of Python, JavaScript and Java.
- [`222068f`](https://github.com/zeno-src/zeno-lang/commit/222068f437d26836e9fa6cfd2538b437ecb2566b) `u64`, a whole number of 64 bits with no sign, and `.max` and `.min` on every whole type (`u8.max`).
- [`dab0d00`](https://github.com/zeno-src/zeno-lang/commit/dab0d00370c4217782c3b74c359c78a09488fbab) Units made of units: voltage times current is power, so `3.3 V * 20 mA` is `0.066 W`.

### The libraries of C, and of Go and Rust through C

*18 November 2026*

- [`b747791`](https://github.com/zeno-src/zeno-lang/commit/b747791124e022d94ba6bbfe2d8cedc0fd1c37bb) `import c`: a Zeno program calls libraries of C, and of Go and Rust through their C interface.
- [`564dae0`](https://github.com/zeno-src/zeno-lang/commit/564dae00b6c434ad770920fd41f83a54b763c184) `zeno build` calls C directly: `import c` works in every C build, and the checker reads the declarations, so a wrong argument is found before the program runs.

### On a board: inputs, a serial port, and a voltage

*18 November 2026*

- [`9665982`](https://github.com/zeno-src/zeno-lang/commit/96659826d4e63f5f8a2ebce61449045e56b7a922) Inputs on a board: `input_pin(2, pull_up)` makes a pin that the program reads.
- [`d21a4a3`](https://github.com/zeno-src/zeno-lang/commit/d21a4a334fb3f6d117c0ead8cc5dd5b04082d8c4) The WebAssembly and Rust targets could wake from `sleep` a fraction of a millisecond early. They now wait until the time has passed.
- [`0635fba`](https://github.com/zeno-src/zeno-lang/commit/0635fba93cf53454bf603544ba6e88d4a4c0c9e1) A serial port on a board: `serial(1, 115200 baud)`, with `write`, `read_byte` and `has_byte`.
- [`a2447a7`](https://github.com/zeno-src/zeno-lang/commit/a2447a7e7a6e6b65df7d96e20acf6b4595d7ffc0) Reading a voltage on a board: `analog_pin(0)` gives a pin whose `voltage()` is a quantity in volts.

### Ready for 0.6.0: Windows, macOS, and the speed of 0.5.2

*18 to 19 November 2026*

- [`d41ce8a`](https://github.com/zeno-src/zeno-lang/commit/d41ce8a083d849f158572d449fd10c3d1040dd91) `import c` on Windows and macOS: a file of C beside the program is found on Windows, and on macOS a library that is missing is named in the error.
- [`9c4ae25`](https://github.com/zeno-src/zeno-lang/commit/9c4ae2523516711ffb827c818878f7fc836fd24d) Adding `program.exit` had made the VM run 1 to 3 % more instructions on every benchmark. The exit is carried another way now, and the VM is as fast as in 0.5.2.
- [`14b9d1b`](https://github.com/zeno-src/zeno-lang/commit/14b9d1bf04b501adac7d28da66a9baf72c0f43d5) On Windows and macOS a function that a library of C does not have is named, as on Linux. On Windows a built program reads its environment whole in any script.
- [`032ced2`](https://github.com/zeno-src/zeno-lang/commit/032ced2a970d28985d98fadc1c4f302bb106d3dd) The trial workflow reports what Ruby said when it stopped.
- [`4a6b559`](https://github.com/zeno-src/zeno-lang/commit/4a6b5599fdc3bdc3222a66eb0297cf37337a6774) `import ruby` did not start on Windows, where Ruby cut its command line short. The helper is handed over as a file there.
- [`92db930`](https://github.com/zeno-src/zeno-lang/commit/92db930d2998a4fc9102011eaf6425331385329b) `==` and the tests for `f32` and `u64` are made cheap again, so the new numbers cost older programs nothing.
- [`2af235c`](https://github.com/zeno-src/zeno-lang/commit/2af235cd281cb7f33ba1810d3390a384c0c49260) With no collector, a list that is the default of a field (`items: list[str] = []`) was freed while the new object still pointed at it. It now belongs to the object.
- [`f11a77a`](https://github.com/zeno-src/zeno-lang/commit/f11a77a953a416deffe132f398e192f3cfcc8adb) The VM keeps the output, the board, the page and the keyboard behind one pointer, so that adding to them cannot slow its loop. One benchmark had become 6 % slower by the clock.
- [`bc8a2a1`](https://github.com/zeno-src/zeno-lang/commit/bc8a2a1ba369ee3fdba6ada93199fefc90cdb9ab) Sums of two ints or two floats take a short path in the interpreter too, which now runs fewer instructions than in 0.5.2.
- [`4268b08`](https://github.com/zeno-src/zeno-lang/commit/4268b0896c8048e1243213df8d4b1909117a76aa) The version is 0.6.0, with its release notes.
- [`356e780`](https://github.com/zeno-src/zeno-lang/commit/356e780568da6234f3de1967104de5fd68c0f00d) Book: rebuilt for Zeno 0.6. **This commit is Zeno 0.6.0.**

## Unreleased

*Not in a release yet · 2 commits · 18 to 19 November 2026*

What is on `main` since 0.6.0 was released.

- [`8d44bb1`](https://github.com/zeno-src/zeno-lang/commit/8d44bb10b792f1bc60d33284b7cf85dac4d4238d) The Homebrew, pacman and winget recipes for 0.6.0, with its checksums.
- [`40ae73a`](https://github.com/zeno-src/zeno-lang/commit/40ae73a158ab3d10ba2475fcfd55ca5824f41243) `HISTORY.md`, this page: every commit under its release, written by `tools/history.py` from git and a file of notes. The website shows it at `/history/`.

---

This file is written by [`tools/history.py`](tools/history.py) from the commits themselves and the notes in [`tools/history-notes.txt`](tools/history-notes.txt). It lists 319 commits, up to `40ae73a` of 18 November 2026. A commit that only brings this file or its notes up to date is not listed.
