// The JavaScript end of `import node` (docs/node.md).
//
// `zeno run` starts Node with this file as its program. It connects
// back to Zeno over a socket of this machine, then does what each line
// it is sent asks for, and answers with one line. The lines are the
// ones bridge.py reads and writes:
//
//     none, a bool, a number, a str            as JSON writes them
//     a float JSON cannot write as a float     ["f", "nan"], "inf", "1e+21"
//     an int JSON cannot write exactly         ["i", "9007199254740993"]
//     a list                                   ["l", [item, ...]]
//     a dict                                   ["d", [[key, value], ...]]
//     anything else, which stays here          ["p", number]
//
// What stays here is kept in `kept` under its number, with the object
// it was read from, which is `this` when it is called.
"use strict";
const net = require("net");
const path = require("path");
const util = require("util");
const { createRequire } = require("module");
const vm = require("vm");

// Where modules are looked for from: the folder the program is in, as
// for a program of JavaScript there, or the folder Zeno was run in.
const home = path.join(process.env.ZENO_PROGRAM || process.cwd(), "zeno.js");
delete process.env.ZENO_PROGRAM;

const kept = new Map();
let count = 0;

/** Something this end has to say itself, as an error of the kind Zeno
 * knows it by. */
class Told extends Error {
    constructor(kind, text) {
        super(text);
        this.kind = kind;
    }
}

function keep(value, self) {
    count += 1;
    kept.set(count, { value, self });
    return ["p", count];
}

/** A value of JavaScript, as it goes to Zeno. `null` and `undefined`
 * are `none`. A number that is whole is an int, and any other a float.
 * Everything else stays here. */
function out(value, self) {
    if (value === null || value === undefined) return null;
    switch (typeof value) {
        case "boolean":
        case "string":
            return value;
        case "number":
            if (Number.isSafeInteger(value)) return value;
            if (Number.isNaN(value)) return ["f", "nan"];
            if (value === Infinity) return ["f", "inf"];
            if (value === -Infinity) return ["f", "-inf"];
            // Whole, and past what a number of JavaScript holds exactly.
            if (Number.isInteger(value)) return ["f", String(value)];
            return value;
        case "bigint":
            if (value >= -(2n ** 63n) && value < 2n ** 63n) return ["i", value.toString()];
            return keep(value);
    }
    return keep(value, self);
}

/** The same, but an array, a set, a map and a plain object go too,
 * item by item. */
function copied(value, depth) {
    if (depth > 200) {
        throw new Told("ValueError", "this is nested more than 200 deep, or holds itself, and cannot be copied");
    }
    if (Array.isArray(value) || value instanceof Set || (ArrayBuffer.isView(value) && !(value instanceof DataView))) {
        return ["l", Array.from(value, (item) => copied(item, depth + 1))];
    }
    if (value instanceof Map) {
        const pairs = [];
        for (const [key, item] of value) {
            if (typeof key !== "string" && !Number.isSafeInteger(key)) {
                throw new Told(
                    "ValueError",
                    "a dict in Zeno has keys that are str or int, and this one has the key " + util.inspect(key),
                );
            }
            pairs.push([key, copied(item, depth + 1)]);
        }
        return ["d", pairs];
    }
    if (value !== null && typeof value === "object") {
        const made_by = Object.getPrototypeOf(value);
        if (made_by === Object.prototype || made_by === null) {
            return ["d", Object.keys(value).map((key) => [key, copied(value[key], depth + 1)])];
        }
    }
    return out(value);
}

/** A value Zeno sent, as a value of JavaScript. */
function back(value) {
    if (!Array.isArray(value)) return value;
    const [tag, inside] = value;
    if (tag === "p") {
        const entry = kept.get(inside);
        if (entry === undefined) throw new Told("Error", "this value was kept by a Node that has stopped since");
        return entry.value;
    }
    if (tag === "l") return inside.map(back);
    if (tag === "d") {
        const made = {};
        for (const [key, item] of inside) made[back(key)] = back(item);
        return made;
    }
    if (tag === "i") return BigInt(inside);
    if (inside === "nan") return NaN;
    if (inside === "inf") return Infinity;
    if (inside === "-inf") return -Infinity;
    return Number(inside);
}

/** The kind of error Zeno raises for an error of JavaScript: a
 * `FileError` for one the system gave (no such file, no leave to read
 * it), and `Error` for the rest. */
function kind_of(error) {
    if (error instanceof Told) return error.kind;
    if (error && typeof error.errno === "number" && typeof error.syscall === "string") return "FileError";
    return "Error";
}

function said(error) {
    if (error instanceof Told) return error.message;
    if (error instanceof Error) {
        // Plain `Error` says nothing that Zeno's own `Error` does not.
        if (error.name === "Error" && error.message) return error.message;
        return error.message ? error.name + ": " + error.message : error.name;
    }
    return "thrown: " + util.inspect(error);
}

/** `import(name)`, looked for from the same folder as `require` does.
 * A plain `import()` here would look from the folder Zeno was run in.
 * Node has the other only as something still being tried out, and says
 * so each time: that is not the program's to hear. */
function imported(name) {
    const says = process.emitWarning;
    process.emitWarning = () => {};
    try {
        const from_home = new vm.Script("(name) => import(name)", {
            filename: home,
            importModuleDynamically: vm.constants.USE_MAIN_CONTEXT_DEFAULT_LOADER,
        }).runInThisContext();
        return from_home(name);
    } catch (error) {
        // A Node from before that: from the folder Zeno was run in.
        return import(name);
    } finally {
        process.emitWarning = says;
    }
}

const NOT_FOUND = ["MODULE_NOT_FOUND", "ERR_MODULE_NOT_FOUND"];
const ASK_AGAIN = ["ERR_REQUIRE_ESM", "ERR_REQUIRE_ASYNC_MODULE", "ERR_PACKAGE_PATH_NOT_EXPORTED"];

/** A module by its name: one that comes with Node, or one that `npm
 * install` put in `node_modules` of the folder the program is run in,
 * or of a folder above it. */
async function load(name) {
    const from_here = createRequire(home);
    let wrong;
    try {
        return from_here(name);
    } catch (error) {
        wrong = error;
    }
    if (wrong && ASK_AGAIN.includes(wrong.code)) {
        // A module that can only be imported, not required.
        try {
            return await imported(name);
        } catch (error) {
            wrong = error;
        }
    }
    if (wrong && NOT_FOUND.includes(wrong.code) && String(wrong.message).includes("'" + name + "'")) {
        throw new Told(
            "FileError",
            "Node has no module `" + name + "`. Is it installed in this folder (`npm install " + name + "`)? " +
                "This folder is " + path.dirname(home),
        );
    }
    throw wrong;
}

function length_of(thing) {
    if (typeof thing === "string") return Array.from(thing).length;
    if (thing !== null && typeof thing === "object" || typeof thing === "function") {
        if (typeof thing.length === "number") return thing.length;
        if (typeof thing.size === "number") return thing.size;
        return Object.keys(thing).length;
    }
    throw new TypeError("this has no length");
}

function has(thing, item) {
    if (typeof thing === "string" || Array.isArray(thing)) return thing.includes(item);
    if (thing instanceof Set || thing instanceof Map) return thing.has(item);
    return item in thing;
}

function items_of(thing) {
    if (thing !== null && thing !== undefined && typeof thing[Symbol.iterator] === "function") return Array.from(thing);
    // A plain object is gone through by its keys, as a dict is.
    return Object.keys(thing);
}

/** How `print` shows it: as JavaScript's own `console.log` would,
 * unless the value says how it is written (a date, an error, a URL). */
function shown(thing) {
    if (typeof thing === "function") return util.inspect(thing);
    if (thing !== null && typeof thing === "object") {
        const written = thing.toString;
        if (written === Object.prototype.toString || written === Array.prototype.toString || ArrayBuffer.isView(thing)) {
            return util.inspect(thing);
        }
    }
    return String(thing);
}

async function act(ask) {
    const op = ask.op;
    if (op === "import") return keep(await load(ask.name));
    if (op === "global") {
        if (!(ask.name in globalThis)) throw new Told("Error", "JavaScript has nothing called `" + ask.name + "`");
        return out(globalThis[ask.name], globalThis);
    }
    const entry = ask.on && ask.on[0] === "p" ? kept.get(ask.on[1]) : undefined;
    const thing = "on" in ask ? back(ask.on) : undefined;
    if (op === "get") return out(await thing[ask.name], thing);
    if (op === "set") {
        thing[ask.name] = back(ask.to);
        return null;
    }
    if (op === "call" || op === "new") {
        const args = ask.args.map(back);
        // JavaScript has no named arguments. The ones Zeno was given
        // are gathered into one object, which goes last: the way a
        // function of JavaScript takes its options.
        if (ask.named.length > 0) {
            const options = {};
            for (const [name, arg] of ask.named) options[name] = back(arg);
            args.push(options);
        }
        if (typeof thing !== "function") throw new TypeError("this is not a function, and cannot be called");
        if (op === "new") return out(Reflect.construct(thing, args));
        // A promise is waited for: Zeno goes on when the answer is there.
        return out(await Reflect.apply(thing, entry ? entry.self : undefined, args));
    }
    if (op === "item") {
        const at = back(ask.at);
        return out(await (thing instanceof Map ? thing.get(at) : thing[at]), thing);
    }
    if (op === "set_item") {
        const at = back(ask.at);
        if (thing instanceof Map) thing.set(at, back(ask.to));
        else thing[at] = back(ask.to);
        return null;
    }
    if (op === "len") return length_of(thing);
    if (op === "has") return has(thing, back(ask.item));
    if (op === "items") return ["l", items_of(thing).map((item) => out(item))];
    if (op === "copy") return copied(thing, 0);
    if (op === "str") return shown(thing);
    if (op === "repr") return util.inspect(thing);
    if (op === "equal") return thing === back(ask.to);
    throw new Told("Error", "Zeno asked for `" + op + "`, which this JavaScript end does not know");
}

/** What was printed is on the screen before Zeno goes on. */
function flushed(stream) {
    return new Promise((done) => {
        try {
            stream.write("", () => done());
        } catch (error) {
            done();
        }
    });
}

function main() {
    const [port, token, first] = process.env.ZENO_BRIDGE.split(" ");
    delete process.env.ZENO_BRIDGE;
    count = Number(first);
    const link = net.connect(Number(port), "127.0.0.1");
    link.setNoDelay(true);
    link.setEncoding("utf8");
    const send = (answer) => link.write(JSON.stringify(answer) + "\n");
    link.on("connect", () => send({ hello: token, version: process.versions.node, program: process.execPath }));

    // One thing at a time, in the order asked.
    let waiting = "";
    let busy = Promise.resolve();
    const answer = async (line) => {
        let reply;
        try {
            const ask = JSON.parse(line);
            for (const number of ask.free || []) kept.delete(number);
            reply = { ok: await act(ask) };
        } catch (error) {
            reply = { err: [kind_of(error), said(error)] };
        }
        await flushed(process.stdout);
        await flushed(process.stderr);
        send(reply);
    };
    link.on("data", (text) => {
        waiting += text;
        let end;
        while ((end = waiting.indexOf("\n")) >= 0) {
            const line = waiting.slice(0, end);
            waiting = waiting.slice(end + 1);
            busy = busy.then(() => answer(line));
        }
    });
    // Zeno is done, or gone: there is nobody to answer.
    const over = () => busy.then(() => process.exit(0));
    link.on("end", over);
    link.on("error", over);
}

main();
