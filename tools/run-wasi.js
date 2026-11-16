// Run a program built with `zeno build --target wasm` in Node, which
// carries a WASI host: node tools/run-wasi.js hello.wasm [words...]
// The folder it runs in is the program's, so that it can read and
// write files there.
import { WASI } from "node:wasi";
import fs from "node:fs";

const [file, ...args] = process.argv.slice(2);
const wasi = new WASI({
  version: "preview1",
  args: [file, ...args],
  env: {},
  preopens: { ".": process.cwd() },
  returnOnExit: true,
});
const module = new WebAssembly.Module(fs.readFileSync(file));
const instance = new WebAssembly.Instance(module, wasi.getImportObject());
process.exitCode = wasi.start(instance);
