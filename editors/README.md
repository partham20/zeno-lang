# Zeno in your editor

Two things make an editor know Zeno:

- **The language server**, `zeno lsp`. It is part of the `zeno` program
  itself. The editor starts it, and from then on shows the mistakes in
  a file as you type, lays the file out when you ask, jumps to where a
  name was declared ("go to definition" in most editors) and shows a
  name's type when you rest on it. Any editor that speaks the Language
  Server Protocol can use it.
- **Colours.** These are per editor: a grammar for VS Code, a syntax
  file for Vim and Neovim.

`zeno` has to be somewhere the editor finds it. Build it with
`cargo build --release` and put `target/release/zeno` on your `PATH`.

How the server works is in [`docs/lsp.md`](../docs/lsp.md).

## VS Code

The folder [`vscode/`](vscode) is an extension: colours, brackets and
comments, and a few lines that start `zeno lsp`.

```sh
cd editors/vscode
npm install                 # gets the one library it uses, vscode-languageclient
ln -s "$PWD" ~/.vscode/extensions/zeno-lang.zeno-0.0.1
```

On Windows, copy the folder to `%USERPROFILE%\.vscode\extensions\zeno-lang.zeno-0.0.1`
in place of the `ln` line. Then restart VS Code and open a `.zeno`
file. If `zeno` is not on your `PATH`, set `zeno.path` in the settings
to where it is. A file is laid out when it is saved.

## Neovim

Copy [`vim/`](vim) into your configuration for the colours:

```sh
cp -r editors/vim/* ~/.config/nvim/
```

and add this to `init.lua` for the language server:

```lua
vim.api.nvim_create_autocmd("FileType", {
  pattern = "zeno",
  callback = function(args)
    vim.bo[args.buf].commentstring = "# %s"
    local found = vim.fs.find({ "zeno.package", "main.zeno" }, { upward = true, path = vim.fs.dirname(args.file) })[1]
    vim.lsp.start({
      name = "zeno",
      cmd = { "zeno", "lsp" },
      root_dir = found and vim.fs.dirname(found) or vim.fn.getcwd(),
    })
  end,
})
-- Lay a Zeno file out when it is saved.
vim.api.nvim_create_autocmd("BufWritePre", {
  pattern = "*.zeno",
  callback = function() vim.lsp.buf.format() end,
})
```

## Vim

```sh
cp -r editors/vim/* ~/.vim/
```

gives the colours. Vim has no language server client of its own. With
a plugin that adds one, the command to start is `zeno lsp`.

## Helix

Add to `~/.config/helix/languages.toml`:

```toml
[language-server.zeno]
command = "zeno"
args = ["lsp"]

[[language]]
name = "zeno"
scope = "source.zeno"
file-types = ["zeno"]
roots = ["zeno.package", "main.zeno"]
comment-token = "#"
indent = { tab-width = 4, unit = "    " }
language-servers = ["zeno"]
auto-format = true
```

Helix takes its colours from a tree-sitter grammar, and Zeno has none
yet, so the text is not coloured. The mistakes and the layout work.

## Any other editor

Tell it that files ending in `.zeno` are the language `zeno`, and that
the language server for it is the command `zeno lsp`, talking over
standard input and output.

## Trying the server with no editor

```sh
python3 editors/talk.py target/debug/zeno examples/thermostat/main.zeno
```

plays the part of an editor: it opens the file, types a mistake, takes
it out, asks what a name is and where it was declared, asks for the
layout, and prints every message both ways.

## What has been tried

- The server itself is tested message by message in `src/lsp.rs`, and
  end to end by `editors/talk.py`.
- The Vim syntax file was loaded in Vim and its colours checked.
- The VS Code extension, and the Neovim and Helix settings, are
  written from those editors' documentation and **have not been run in
  those editors yet**. If one does not work, the server's side can be
  checked with `editors/talk.py`.
