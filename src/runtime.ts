// The Zeno runtime for TypeScript.
//
// A program built with `zeno build --target ts` is its own code after
// this. It is runtime.js, line for line, with a type where TypeScript
// asks for one, and the type is `any`: what a Zeno value is, is found
// out while the program runs, by the steps below, as `zeno run` finds
// it out. So the file passes `tsc --strict`, and taking the types away
// gives runtime.js again (a test in emit_js.rs holds the two together:
// change one, and make the same change in the other).
//
// How a Zeno value is held, and what each step does, is told at the top
// of runtime.js.

"use strict";

const Z = (function () {
  const MAX = 9223372036854775807n;
  const MIN = -9223372036854775808n;
  const SAFE = 9007199254740991n;
  const LINES_A_FILE = 1000000;
  // What the program runs in: Node, or a page. Node's own things are
  // taken from it by name, so the file runs as a script and as a module.
  const host: any = globalThis;
  const process = host.process, Buffer = host.Buffer;
  const inNode = typeof process !== "undefined" && process.versions && process.versions.node;
  const fs = inNode ? process.getBuiltinModule("fs") : null;

  // ---------------------------------------------------------------- Values

  class Float { declare v: any; constructor(v: any) { this.v = v; } }
  class Quantity { declare v: any; declare unit: any; constructor(v: any, unit: any) { this.v = v; this.unit = unit; } }
  class Dict { declare map: any; constructor() { this.map = new Map(); } }
  class Class { declare name: any; declare fields: any; declare defaults: any; declare methods: any; constructor(name: any, fields: any, defaults: any, methods: any) { this.name = name; this.fields = fields; this.defaults = defaults; this.methods = methods; } }
  class Instance { declare cls: any; declare fields: any; constructor(cls: any, fields: any) { this.cls = cls; this.fields = fields; } }
  class Enum { declare name: any; declare cases: any; constructor(name: any, cases: any) { this.name = name; this.cases = cases; } }
  class Case { declare def: any; declare index: any; declare fields: any; constructor(def: any, index: any, fields: any) { this.def = def; this.index = index; this.fields = fields; } }
  class Constructor { declare def: any; declare index: any; constructor(def: any, index: any) { this.def = def; this.index = index; } }
  class ErrorType { declare kind: any; constructor(kind: any) { this.kind = kind; } }
  class ZError { declare kind: any; declare message: any; declare object: any; constructor(kind: any, message: any, object: any) { this.kind = kind; this.message = message; this.object = object; } }
  class Module { declare name: any; constructor(name: any) { this.name = name; } }
  // A method of a str, a list or a dict, read but not yet called: "abc".upper
  class Method { declare receiver: any; declare name: any; constructor(receiver: any, name: any) { this.receiver = receiver; this.name = name; } }
  // A method of a class, read from an object: p.distance_to
  class Bound { declare object: any; declare fn: any; constructor(object: any, fn: any) { this.object = object; this.fn = fn; } }
  // What a call gives a parameter it leaves out.
  const MISSING = { missing: true };

  // An error the program may catch, on its way to a `try`.
  class Raise { declare value: any; declare line: any; constructor(value: any, line: any) { this.value = value; this.line = line; } }
  // A mistake in the program, which nothing catches.
  class Fatal { declare message: any; declare line: any; constructor(message: any, line: any) { this.message = message; this.line = line; } }
  // `program.exit` in a page: the run ends, and the page stays.
  class Exit { declare code: any; constructor(code: any) { this.code = code; } }

  function fatal(message: any, line: any) { throw new Fatal(message, line); }
  function raise(kind: any, message: any, line: any) { throw new Raise(new ZError(kind, message, null), line); }
  const overflow = (line: any) => raise("OverflowError", "the result is too big for an int", line);
  const byZero = (line: any) => raise("ZeroDivisionError", "division by zero", line);

  const isInt = (v: any) => typeof v === "number" || typeof v === "bigint";
  // A BigInt as a Zeno int: back to a number when it is small enough.
  function int(b: any, line?: any) {
    if (b > MAX || b < MIN) overflow(line);
    return b >= -SAFE && b <= SAFE ? Number(b) : b;
  }
  const big = (v: any) => (typeof v === "bigint" ? v : BigInt(v));
  // The number inside an int or a float, as a float.
  function number(v: any) {
    if (typeof v === "number") return v;
    if (v instanceof Float) return v.v;
    if (v instanceof F32) return v.v;
    if (v instanceof U64) return Number(v.v);
    if (typeof v === "bigint") return Number(v);
    return null;
  }
  const float = (x: any) => new Float(x);
  class F32 { declare v: any; constructor(v: any) { this.v = v; } }
  const f32 = (x: any) => new F32(x);
  // An f32, or an int as a float and then the nearest f32 (41.2).
  function single(v: any) {
    if (v instanceof F32) return v.v;
    if (isInt(v)) return Math.fround(Number(v));
    return null;
  }
  // The float with the fewest digits that reads back as this f32.
  function f32Shown(x: any) {
    if (x !== x || x === 0 || !Number.isFinite(x)) return x;
    for (let digits = 1; digits <= 9; digits++) {
      const d = Number(x.toExponential(digits - 1));
      if (Math.fround(d) === x) return d;
    }
    return x;
  }
  function f32Op(op: any, a: any, b: any, line: any): any {
    const x = single(a), y = single(b);
    if (x === null || y === null) {
      if (a instanceof Float || b instanceof Float) fatal("`" + op + "` between " + typeName(a) + " and " + typeName(b) + " would have to choose between 32 bits and 64. Write `f32( )` around the float to work in 32, or `float( )` around the f32 to work in 64", line);
      mismatch(op, a, b, line);
    }
    if ((op === "/" || op === "//" || op === "%") && y === 0) byZero(line);
    let r;
    if (op === "+") r = x + y;
    else if (op === "-") r = x - y;
    else if (op === "*") r = x * y;
    else if (op === "/") r = x / y;
    else if (op === "//") r = Math.floor(x / y);
    else if (op === "%") r = x - y * Math.floor(x / y);
    else r = pow(float(x), float(y), line).v;
    return f32(Math.fround(r));
  }

  class U64 { declare v: any; constructor(v: any) { this.v = v; } }
  const U64_MAX = 18446744073709551615n;
  const U64_TOO_BIG = "the result is too big for a u64";
  // A u64, or an int of 0 or more, as a BigInt; else null.
  function unsigned(v: any): any {
    if (v instanceof U64) return v.v;
    if (isInt(v) && v >= 0) return big(v);
    return null;
  }
  function u64ShiftCount(n: any, line: any): any {
    const c = n instanceof U64 ? n.v : big(n);
    if (c < 0n || c >= 64n) raise("ValueError", "cannot shift by " + c + ". A u64 has 64 bits, so a shift is 0 to 63", line);
    return c;
  }
  // `a op b` where one side is a u64 (decision 42).
  function u64Op(op: any, a: any, b: any, line: any) {
    if ((op === "<<" || op === ">>") && a instanceof U64 && (isInt(b) || b instanceof U64)) {
      const n = u64ShiftCount(b, line);
      if (op === ">>") return new U64(a.v >> n);
      if (a.v << n > U64_MAX) raise("OverflowError", U64_TOO_BIG, line);
      return new U64(a.v << n);
    }
    const x = unsigned(a), y = unsigned(b);
    if (x === null || y === null) fatal("`" + op + "` between " + typeName(a) + " and " + typeName(b) + " would have to choose between a u64 and something else. Write `u64( )` or `int( )` around one of them", line);
    let r: any;
    if (op === "+") r = x + y;
    else if (op === "-") {
      if (y > x) raise("OverflowError", "the result is below 0, and a u64 never is", line);
      r = x - y;
    } else if (op === "*") r = x * y;
    else if (op === "/") {
      if (y === 0n) byZero(line);
      return float(Number(x) / Number(y));
    } else if (op === "//" || op === "%") {
      if (y === 0n) byZero(line);
      r = op === "//" ? x / y : x % y;
    } else if (op === "**") {
      if (y === 0n) return new U64(1n);
      if (x < 2n) return new U64(x);
      if (y > 64n) raise("OverflowError", U64_TOO_BIG, line);
      r = x ** y;
    } else if (op === "&") r = x & y;
    else if (op === "|") r = x | y;
    else r = x ^ y;
    if (r > U64_MAX) raise("OverflowError", U64_TOO_BIG, line);
    return new U64(r);
  }

  function typeName(v: any) {
    if (isInt(v)) return "an int";
    if (v instanceof Float) return "a float";
    if (v instanceof F32) return "an f32";
    if (v instanceof U64) return "a u64";
    if (typeof v === "string") return "a str";
    if (typeof v === "boolean") return "a bool";
    if (v === null) return "none";
    if (v instanceof Quantity) return "a quantity in " + v.unit;
    if (Array.isArray(v)) return "a list";
    if (v instanceof Dict) return "a dict";
    if (typeof v === "function" || v instanceof Method || v instanceof Bound) return "a function";
    if (v instanceof Class) return "a class";
    if (v instanceof Instance) return "a " + v.cls.name;
    if (v instanceof Enum) return "an enum";
    if (v instanceof Constructor) return "an enum case";
    if (v instanceof Case) return "a " + v.def.name;
    if (v instanceof ErrorType) return "an error type";
    if (v instanceof ZError) return "a " + v.kind;
    if (v instanceof Module) return "a module";
    return "a value";
  }

  // `Point`, not `shapes.Point`; `Stack`, not `Stack[int]`.
  function shownName(name: any) {
    name = name.split("[")[0];
    const dot = name.lastIndexOf(".");
    return dot < 0 ? name : name.slice(dot + 1);
  }

  // ------------------------------------------------------------- Showing

  // A float as Rust's `{:?}` shows it: the shortest digits that read back
  // as the same number, with `.0` on a whole one, and an exponent for one
  // below 1e-4 or from 1e16 up.
  function showFloat(x: any) {
    if (Number.isNaN(x)) return "NaN";
    if (x === Infinity) return "inf";
    if (x === -Infinity) return "-inf";
    if (x === 0) return Object.is(x, -0) ? "-0.0" : "0.0";
    let [mantissa, exp] = x.toExponential().split("e");
    exp = Number(exp);
    const sign = mantissa[0] === "-" ? "-" : "";
    const digits = mantissa.replace("-", "").replace(".", "");
    if (exp < -4 || exp >= 16) {
      return sign + (digits.length > 1 ? digits[0] + "." + digits.slice(1) : digits) + "e" + exp;
    }
    if (exp < 0) return sign + "0." + "0".repeat(-exp - 1) + digits;
    if (digits.length <= exp + 1) return sign + digits + "0".repeat(exp + 1 - digits.length) + ".0";
    return sign + digits.slice(0, exp + 1) + "." + digits.slice(exp + 1);
  }

  // A float with a fixed number of decimals, as Rust's `{:.2}` writes it:
  // from the exact value of the float, a half rounding to the even digit.
  function fixed(x: any, places: any) {
    if (Number.isNaN(x)) return "NaN";
    if (!Number.isFinite(x)) return x > 0 ? "inf" : "-inf";
    const negative = x < 0 || Object.is(x, -0);
    x = Math.abs(x);
    // x = mantissa * 2**exponent, exactly.
    const view = new DataView(new ArrayBuffer(8));
    view.setFloat64(0, x);
    const bits = view.getBigUint64(0);
    let exponent = Number((bits >> 52n) & 0x7ffn);
    let mantissa = bits & 0xfffffffffffffn;
    if (exponent === 0) exponent = 1; else mantissa |= 1n << 52n;
    exponent -= 1075;
    // scaled = x * 10**places, as a fraction num / den.
    let num = mantissa * 10n ** BigInt(places);
    let den = 1n;
    if (exponent >= 0) num <<= BigInt(exponent); else den <<= BigInt(-exponent);
    let whole = num / den;
    const rest = num - whole * den;
    if (rest * 2n > den || (rest * 2n === den && whole % 2n === 1n)) whole += 1n;
    let digits = whole.toString();
    if (places > 0) {
      digits = digits.padStart(places + 1, "0");
      digits = digits.slice(0, digits.length - places) + "." + digits.slice(digits.length - places);
    }
    return (negative ? "-" : "") + digits;
  }

  // How Rust's `{:?}` shows a str: in quotes, with what cannot be seen escaped.
  function quoted(s: any) {
    let out = '"';
    for (const c of s) {
      const code = c.codePointAt(0);
      if (c === '"') out += '\\"';
      else if (c === "\\") out += "\\\\";
      else if (c === "\n") out += "\\n";
      else if (c === "\r") out += "\\r";
      else if (c === "\t") out += "\\t";
      else if (c === "\0") out += "\\0";
      else if (code < 0x20 || (code >= 0x7f && code < 0xa0)) out += "\\u{" + code.toString(16) + "}";
      else out += c;
    }
    return out + '"';
  }

  // How `print`, `str()` and f-strings show a value.
  function show(v: any): any {
    if (typeof v === "number" || typeof v === "bigint") return String(v);
    if (typeof v === "string") return v;
    if (v instanceof Float) return showFloat(v.v);
    if (v instanceof F32) return showFloat(f32Shown(v.v));
    if (v instanceof U64) return String(v.v);
    if (typeof v === "boolean") return v ? "true" : "false";
    if (v === null) return "none";
    if (v instanceof Quantity) {
      if (Number.isInteger(v.v) && Math.abs(v.v) < 1e15) return (Object.is(v.v, -0) ? "0" : String(v.v)) + " " + v.unit;
      return showFloat(v.v) + " " + v.unit;
    }
    if (Array.isArray(v)) return "[" + v.map(repr).join(", ") + "]";
    if (v instanceof Dict) {
      if (v.map.size === 0) return "[:]";
      const parts = [];
      for (const [key, value] of v.map) parts.push(repr(key) + ": " + repr(value));
      return "[" + parts.join(", ") + "]";
    }
    if (typeof v === "function") return v.zname ? "<function " + v.zname + ">" : "<function>";
    if (v instanceof Method) return "<function " + v.name + ">";
    if (v instanceof Bound) return "<function " + v.fn.zname + ">";
    if (v instanceof Class) return "<class " + v.name + ">";
    if (v instanceof Instance) {
      const parts = v.cls.fields.map((name: any, i: any) => name + "=" + repr(v.fields[i]));
      return shownName(v.cls.name) + "(" + parts.join(", ") + ")";
    }
    if (v instanceof Enum) return "<enum " + v.name + ">";
    if (v instanceof Constructor) return "<case " + v.def.cases[v.index][0] + ">";
    if (v instanceof Case) {
      const name = shownName(v.def.cases[v.index][0]);
      return v.fields.length === 0 ? name : name + "(" + v.fields.map(repr).join(", ") + ")";
    }
    if (v instanceof ErrorType) return "<error " + v.kind + ">";
    if (v instanceof ZError) return v.message;
    if (v instanceof Module) return "<module " + v.name + ">";
    return String(v);
  }

  const repr = (v: any): any => (typeof v === "string" ? quoted(v) : show(v));

  // ---------------------------------------------------------------- Units

  const UNITS: any = {
    ns: ["time", 1], us: ["time", 1e3], ms: ["time", 1e6], s: ["time", 1e9], min: ["time", 60e9], h: ["time", 3600e9],
    Hz: ["frequency", 1], kHz: ["frequency", 1e3], MHz: ["frequency", 1e6],
    uV: ["voltage", 1], mV: ["voltage", 1e3], V: ["voltage", 1e6], kV: ["voltage", 1e9],
    uA: ["current", 1], mA: ["current", 1e3], A: ["current", 1e6],
    ohm: ["resistance", 1], kohm: ["resistance", 1e3], Mohm: ["resistance", 1e6],
    mW: ["power", 1], W: ["power", 1e3], kW: ["power", 1e6],
    J: ["energy", 1], Wh: ["energy", 3600], kWh: ["energy", 3600e3],
    degC: ["temperature in Celsius", 1], K: ["temperature in kelvin", 1],
    mm: ["length", 1], cm: ["length", 10], m: ["length", 1e3], km: ["length", 1e6],
    mg: ["mass", 1], g: ["mass", 1e3], kg: ["mass", 1e6],
    deg: ["angle in degrees", 1], rad: ["angle in radians", 1], rpm: ["rotation", 1],
  };
  const isUnit = (name: any) => Object.prototype.hasOwnProperty.call(UNITS, name);

  // How to turn a number in one unit into another: [times, over], or null.
  function convert(from: any, to: any) {
    if (from === "deg" && to === "rad") return [Math.PI / 180, 1];
    if (from === "rad" && to === "deg") return [180 / Math.PI, 1];
    if (!isUnit(from) || !isUnit(to)) return null;
    const [fromKind, fromSize] = UNITS[from];
    const [toKind, toSize] = UNITS[to];
    if (fromKind !== toKind) return null;
    return fromSize >= toSize ? [fromSize / toSize, 1] : [1, toSize / fromSize];
  }
  const apply = (scale: any, x: any) => (x * scale[0]) / scale[1];

  // The unit two quantities meet in: the finer one. [unit, scaleA, scaleB] or null.
  function common(a: any, b: any): any {
    if (!isUnit(a) || !isUnit(b)) return null;
    if (UNITS[a][0] !== UNITS[b][0]) return null;
    const unit = UNITS[a][1] <= UNITS[b][1] ? a : b;
    return [unit, convert(a, unit), convert(b, unit)];
  }

  function unit(v: any, name: any, line: any) {
    if (!isUnit(name)) fatal("`" + name + "` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC", line);
    const x = number(v);
    if (x === null) fatal("a unit can only follow a number", line);
    return new Quantity(x, name);
  }

  // ------------------------------------------------------------ Equality

  function equal(a: any, b: any) {
    if (a === b) return !(a instanceof Float || a instanceof F32) || a.v === a.v;
    if (a instanceof U64 || b instanceof U64) {
      const x = unsigned(a), y = unsigned(b);
      return x !== null && x === y;
    }
    if (a instanceof F32 || b instanceof F32) {
      const x = single(a), y = single(b);
      if (x !== null && y !== null) return x === y;
      const p = number(a), q = number(b);
      return p !== null && q !== null && p === q;
    }
    if (isInt(a) && isInt(b)) return a == b;
    if ((isInt(a) || a instanceof Float) && (isInt(b) || b instanceof Float)) return number(a) === number(b);
    if (a instanceof Quantity && b instanceof Quantity) {
      const c = common(a.unit, b.unit);
      return c !== null && apply(c[1], a.v) === apply(c[2], b.v);
    }
    if (Array.isArray(a) && Array.isArray(b)) {
      if (a.length !== b.length) return false;
      for (let i = 0; i < a.length; i++) if (!equal(a[i], b[i])) return false;
      return true;
    }
    if (a instanceof Dict && b instanceof Dict) {
      if (a.map.size !== b.map.size) return false;
      for (const [key, value] of a.map) {
        if (!b.map.has(key) || !equal(value, b.map.get(key))) return false;
      }
      return true;
    }
    if (a instanceof Instance && b instanceof Instance) {
      if (a.cls !== b.cls) return false;
      for (let i = 0; i < a.fields.length; i++) if (!equal(a.fields[i], b.fields[i])) return false;
      return true;
    }
    if (a instanceof Case && b instanceof Case) {
      if (a.def !== b.def || a.index !== b.index) return false;
      for (let i = 0; i < a.fields.length; i++) if (!equal(a.fields[i], b.fields[i])) return false;
      return true;
    }
    if (a instanceof ZError && b instanceof ZError) return a.kind === b.kind && a.message === b.message;
    if (a instanceof Constructor && b instanceof Constructor) return a.def === b.def && a.index === b.index;
    return false;
  }

  // ----------------------------------------------------------- Operators

  // Two strs in the order of their characters, as Rust compares them.
  function compareStr(a: any, b: any) {
    const n = Math.min(a.length, b.length);
    for (let i = 0; i < n; i++) {
      let x = a.charCodeAt(i), y = b.charCodeAt(i);
      if (x === y) continue;
      // UTF-16 puts the characters above U+FFFF (two units, from 0xD800)
      // below those from U+E000. Move both so that the order is Unicode's.
      if (x >= 0xd800) x = x <= 0xdfff ? x + 0x2000 : x - 0x800;
      if (y >= 0xd800) y = y <= 0xdfff ? y + 0x2000 : y - 0x800;
      return x < y ? -1 : 1;
    }
    return a.length === b.length ? 0 : a.length < b.length ? -1 : 1;
  }

  function mismatch(op: any, a: any, b: any, line: any) {
    fatal("`" + op + "` does not work between " + typeName(a) + " and " + typeName(b), line);
  }

  function add(a: any, b: any, line: any) {
    if (a instanceof U64 || b instanceof U64) return u64Op("+", a, b, line);
    if (a instanceof F32 || b instanceof F32) return f32Op('+', a, b, line);
    if (typeof a === "number" && typeof b === "number") {
      const r = a + b;
      return Number.isSafeInteger(r) ? r : int(BigInt(a) + BigInt(b), line);
    }
    if (isInt(a) && isInt(b)) return int(big(a) + big(b), line);
    if (typeof a === "string" && typeof b === "string") return a + b;
    if (Array.isArray(a) && Array.isArray(b)) return a.concat(b);
    if (a instanceof Quantity && b instanceof Quantity) {
      const c = common(a.unit, b.unit);
      if (c === null) mismatch("+", a, b, line);
      return new Quantity(apply(c[1], a.v) + apply(c[2], b.v), c[0]);
    }
    const x = number(a), y = number(b);
    if (x === null || y === null) mismatch("+", a, b, line);
    return float(x + y);
  }

  function sub(a: any, b: any, line: any) {
    if (a instanceof U64 || b instanceof U64) return u64Op("-", a, b, line);
    if (a instanceof F32 || b instanceof F32) return f32Op('-', a, b, line);
    if (typeof a === "number" && typeof b === "number") {
      const r = a - b;
      return Number.isSafeInteger(r) ? r : int(BigInt(a) - BigInt(b), line);
    }
    if (isInt(a) && isInt(b)) return int(big(a) - big(b), line);
    if (a instanceof Quantity && b instanceof Quantity) {
      const c = common(a.unit, b.unit);
      if (c === null) mismatch("-", a, b, line);
      return new Quantity(apply(c[1], a.v) - apply(c[2], b.v), c[0]);
    }
    const x = number(a), y = number(b);
    if (x === null || y === null) mismatch("-", a, b, line);
    return float(x - y);
  }

  function mul(a: any, b: any, line: any) {
    if (a instanceof U64 || b instanceof U64) return u64Op("*", a, b, line);
    if (a instanceof F32 || b instanceof F32) return f32Op('*', a, b, line);
    if (Array.isArray(a) && isInt(b)) {
      if (b < 0) raise("ValueError", "`*` repeats a list 0 or more times, and this is " + b, line);
      const repeated = [];
      for (let t = 0; t < b; t++) for (const item of a) repeated.push(item);
      return repeated;
    }
    if (typeof a === "number" && typeof b === "number") {
      const r = a * b;
      return Number.isSafeInteger(r) ? (r === 0 ? 0 : r) : int(BigInt(a) * BigInt(b), line);
    }
    if (isInt(a) && isInt(b)) return int(big(a) * big(b), line);
    if (a instanceof Quantity || b instanceof Quantity) {
      const [q, other] = a instanceof Quantity ? [a, b] : [b, a];
      const y = number(other);
      if (y === null) mismatch("*", a, b, line);
      return new Quantity(q.v * y, q.unit);
    }
    const x = number(a), y = number(b);
    if (x === null || y === null) mismatch("*", a, b, line);
    return float(x * y);
  }

  function div(a: any, b: any, line: any) {
    if (a instanceof U64 || b instanceof U64) return u64Op("/", a, b, line);
    if (a instanceof F32 || b instanceof F32) return f32Op('/', a, b, line);
    if (a instanceof Quantity) {
      if ((b instanceof Quantity || b instanceof Float) && b.v === 0) byZero(line);
      if (b === 0) byZero(line);
      if (b instanceof Quantity) {
        const c = common(a.unit, b.unit);
        if (c === null) mismatch("/", a, b, line);
        return float(apply(c[1], a.v) / apply(c[2], b.v));
      }
      const y = number(b);
      if (y === null) mismatch("/", a, b, line);
      return new Quantity(a.v / y, a.unit);
    }
    const x = number(a), y = number(b);
    if (x === null || y === null) mismatch("/", a, b, line);
    if (y === 0) byZero(line);
    return float(x / y);
  }

  function floordiv(a: any, b: any, line: any) {
    if (a instanceof U64 || b instanceof U64) return u64Op("//", a, b, line);
    if (a instanceof F32 || b instanceof F32) return f32Op('//', a, b, line);
    if (isInt(a) && isInt(b)) {
      if (b == 0) byZero(line);
      if (typeof a === "number" && typeof b === "number") {
        // Exact: a - r is a whole number of b's.
        const r = a % b;
        let q = (a - r) / b;
        if (r !== 0 && (r < 0) !== (b < 0)) q -= 1;
        return q === 0 ? 0 : q;
      }
      const x = big(a), y = big(b);
      let q = x / y;
      if (x % y !== 0n && (x < 0n) !== (y < 0n)) q -= 1n;
      return int(q, line);
    }
    const x = number(a), y = number(b);
    if (x === null || y === null) mismatch("//", a, b, line);
    if (y === 0) byZero(line);
    return float(Math.floor(x / y));
  }

  function mod(a: any, b: any, line: any) {
    if (a instanceof U64 || b instanceof U64) return u64Op("%", a, b, line);
    if (a instanceof F32 || b instanceof F32) return f32Op("%", a, b, line);
    if (isInt(a) && isInt(b)) {
      if (b == 0) byZero(line);
      if (typeof a === "number" && typeof b === "number") {
        const r = a % b;
        return r !== 0 && (r < 0) !== (b < 0) ? r + b : r === 0 ? 0 : r;
      }
      const x = big(a), y = big(b);
      const r = x % y;
      return int(r !== 0n && (r < 0n) !== (y < 0n) ? r + y : r, line);
    }
    const x = number(a), y = number(b);
    if (x === null || y === null) mismatch("%", a, b, line);
    if (y === 0) byZero(line);
    return float(x - y * Math.floor(x / y));
  }

  function pow(a: any, b: any, line: any) {
    if (a instanceof U64 || b instanceof U64) return u64Op("**", a, b, line);
    if (a instanceof F32 || b instanceof F32) return f32Op('**', a, b, line);
    if (isInt(a) && isInt(b)) {
      if (b < 0) raise("ValueError", "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`", line);
      // As Rust's `checked_pow` with a u32 power.
      if (b > 4294967295) overflow(line);
      const x = big(a), y = big(b);
      if (x === 0n) return y === 0n ? 1 : 0;
      if (x === 1n) return 1;
      if (x === -1n) return y % 2n === 0n ? 1 : -1;
      if (y >= 64n) overflow(line);
      return int(x ** y, line);
    }
    const x = number(a), y = number(b);
    if (x === null || y === null) mismatch("**", a, b, line);
    // Rust's powf gives 1 for these, where JavaScript gives NaN.
    if (x === 1 || (x === -1 && !Number.isFinite(y) && !Number.isNaN(y))) return float(1);
    return float(Math.pow(x, y));
  }

  function bitsMismatch(op: any, a: any, b: any, line: any) {
    let hint = "";
    if (typeof a === "boolean" && typeof b === "boolean") {
      if (op === "&") hint = ". For two bools Zeno spells this `and`";
      if (op === "|") hint = ". For two bools Zeno spells this `or`";
    }
    fatal("`" + op + "` works on the bits of whole numbers, and these are " + typeName(a) + " and " + typeName(b) + hint, line);
  }

  function bitand(a: any, b: any, line: any) { if (a instanceof U64 || b instanceof U64) return u64Op("&", a, b, line); if (!isInt(a) || !isInt(b)) bitsMismatch("&", a, b, line); return int(big(a) & big(b), line); }
  function bitor(a: any, b: any, line: any) { if (a instanceof U64 || b instanceof U64) return u64Op("|", a, b, line); if (!isInt(a) || !isInt(b)) bitsMismatch("|", a, b, line); return int(big(a) | big(b), line); }
  function bitxor(a: any, b: any, line: any) { if (a instanceof U64 || b instanceof U64) return u64Op("^", a, b, line); if (!isInt(a) || !isInt(b)) bitsMismatch("^", a, b, line); return int(big(a) ^ big(b), line); }
  function shift(op: any, a: any, n: any, line: any) {
    if (a instanceof U64 || n instanceof U64) return u64Op(op, a, n, line);
    if (!isInt(a) || !isInt(n)) bitsMismatch(op, a, n, line);
    if (n < 0 || n >= 64) raise("ValueError", "cannot shift by " + n + ". An int has 64 bits, so a shift is 0 to 63", line);
    if (op === ">>") return int(big(a) >> big(n), line);
    const shifted = BigInt.asIntN(64, big(a) << big(n));
    if (shifted >> big(n) !== big(a)) overflow(line);
    return int(shifted, line);
  }
  const shl = (a: any, n: any, line: any) => shift("<<", a, n, line);
  const shr = (a: any, n: any, line: any) => shift(">>", a, n, line);

  function neg(v: any, line: any) {
    if (typeof v === "number") return v === 0 ? 0 : -v;
    if (typeof v === "bigint") return int(-v, line);
    if (v instanceof Float) return float(-v.v);
    if (v instanceof F32) return f32(-v.v);
    if (v instanceof U64) fatal("`-` does not work on a u64, which is never below 0. Make it an int first with `int( )`", line);
    if (v instanceof Quantity) return new Quantity(-v.v, v.unit);
    fatal("`-` does not work on " + typeName(v), line);
  }
  function bitnot(v: any, line: any) {
    if (v instanceof U64) return new U64(U64_MAX ^ v.v);
    if (!isInt(v)) fatal("`~` works on the bits of a whole number, and this is " + typeName(v), line);
    return int(~big(v), line);
  }

  // `<` and the others: -1, 0, 1, or null for a NaN.
  function order(op: any, a: any, b: any, line: any) {
    if (a instanceof U64 || b instanceof U64) {
      const x = unsigned(a), y = unsigned(b);
      if (x === null || y === null) u64Op(op, a, b, line);
      return x < y ? -1 : x > y ? 1 : 0;
    }
    if (a instanceof F32 || b instanceof F32) {
      const x = single(a), y = single(b);
      if (x === null || y === null) f32Op(op, a, b, line);
      return x < y ? -1 : x > y ? 1 : x === y ? 0 : null;
    }
    if (isInt(a) && isInt(b)) return a < b ? -1 : a > b ? 1 : 0;
    if (typeof a === "string" && typeof b === "string") return compareStr(a, b);
    if (a instanceof Quantity && b instanceof Quantity) {
      const c = common(a.unit, b.unit);
      if (c === null) mismatch(op, a, b, line);
      const x = apply(c[1], a.v), y = apply(c[2], b.v);
      return x < y ? -1 : x > y ? 1 : x === y ? 0 : null;
    }
    const x = number(a), y = number(b);
    if (x === null || y === null) mismatch(op, a, b, line);
    return x < y ? -1 : x > y ? 1 : x === y ? 0 : null;
  }
  function lt(a: any, b: any, line: any) { const o = order("<", a, b, line); return o !== null && o < 0; }
  function le(a: any, b: any, line: any) { const o = order("<=", a, b, line); return o !== null && o <= 0; }
  function gt(a: any, b: any, line: any) { const o = order(">", a, b, line); return o !== null && o > 0; }
  function ge(a: any, b: any, line: any) { const o = order(">=", a, b, line); return o !== null && o >= 0; }

  function contains(op: any, item: any, container: any, line: any) {
    if (Array.isArray(container)) return container.some((other: any) => equal(item, other));
    if (container instanceof Dict) {
      if (!isInt(item) && typeof item !== "string") mismatch(op, item, container, line);
      return container.map.has(item);
    }
    if (typeof item === "string" && typeof container === "string") return container.includes(item);
    mismatch(op, item, container, line);
  }
  const isIn = (a: any, b: any, line: any) => contains("in", a, b, line);
  const notIn = (a: any, b: any, line: any) => !contains("not in", a, b, line);

  // A condition must be a real bool.
  function truth(v: any, line: any) {
    if (v === true || v === false) return v;
    fatal("a condition must be true or false, but this is " + typeName(v), line);
  }

  // ------------------------------------------------------ Lists and text

  const hasSurrogates = (s: any) => /[\ud800-\udfff]/.test(s);
  const chars = (s: any) => (hasSurrogates(s) ? Array.from(s) : s.split(""));
  const strLen = (s: any) => (hasSurrogates(s) ? Array.from(s).length : s.length);

  function key(v: any, line: any): any {
    if (isInt(v) || typeof v === "string") return v;
    fatal("a key of a dict is a str or an int, but this is " + typeName(v), line);
  }

  function position(index: any, length: any, what: any, line: any) {
    if (!isInt(index)) fatal("an index must be an int, but this is " + typeName(index), line);
    if (index < 0 || index >= length) raise("IndexError", "index " + index + " is out of range for a " + what + " of length " + length, line);
    return Number(index);
  }

  function index(object: any, i: any, line: any) {
    if (Array.isArray(object)) return object[position(i, object.length, "list", line)];
    if (typeof object === "string") {
      const cs = chars(object);
      return cs[position(i, cs.length, "str", line)];
    }
    if (object instanceof Dict) {
      const k = key(i, line);
      if (!object.map.has(k)) raise("KeyError", "this dict has no key " + repr(i), line);
      return object.map.get(k);
    }
    fatal("cannot take an item from " + typeName(object), line);
  }

  function setIndex(object: any, i: any, value: any, line: any) {
    if (Array.isArray(object)) { object[position(i, object.length, "list", line)] = value; return; }
    if (object instanceof Dict) { object.map.set(key(i, line), value); return; }
    fatal("cannot set an item of " + typeName(object), line);
  }

  // What a `for` loop steps through: a copy, so that the loop is not upset
  // by a change to what it is walking.
  function items(v: any, line: any) {
    if (Array.isArray(v)) return v.slice();
    if (v instanceof Dict) return Array.from(v.map.keys());
    if (typeof v === "string") return chars(v);
    fatal("cannot loop over " + typeName(v), line);
  }
  // The same for a loop with two names: [key or position, value] each round.
  function pairs(v: any, line: any) {
    if (v instanceof Dict) return Array.from(v.map.entries());
    return items(v, line).map((item: any, at: any) => [at, item]);
  }

  function list(values: any) { return values; }
  function dict(flat: any, line: any) {
    const d = new Dict();
    for (let i = 0; i < flat.length; i += 2) d.map.set(key(flat[i], line), flat[i + 1]);
    return d;
  }
  function push(list: any, value: any) { list.push(value); return null; }
  function concat(pieces: any) { return pieces.join(""); }

  // ------------------------------------------------- Objects and fields

  function makeFunction(name: any, params: any, defaults: any, fn: any) {
    fn.zname = name;
    fn.zparams = params;
    fn.zdefaults = defaults;
    return fn;
  }

  function makeClass(name: any, fields: any, defaults: any, methods: any) {
    return new Class(name, fields, defaults, new Map(Object.entries(methods)));
  }

  function makeEnum(name: any, cases: any) { return new Enum(name, cases); }

  // A case with no values is a value; one with values needs calling.
  function caseOf(def: any, i: any) {
    return def.cases[i][1].length === 0 ? new Case(def, i, []) : new Constructor(def, i);
  }

  function isCase(value: any, ctor: any, count: any, line: any) {
    if (!(ctor instanceof Constructor)) fatal("this is not an enum case that carries values", line);
    if (!(value instanceof Case) || value.def !== ctor.def || value.index !== ctor.index) return false;
    if (value.fields.length !== count) fatal("`" + ctor.def.cases[ctor.index][0] + "` carries " + value.fields.length + " values, but this pattern has " + count, line);
    return true;
  }
  const caseField = (value: any, i: any) => value.fields[i];

  function field(object: any, name: any, line: any) {
    if (object instanceof Instance) {
      const at = object.cls.fields.indexOf(name);
      if (at >= 0) return object.fields[at];
      const method = object.cls.methods.get(name);
      if (method) return new Bound(object, method);
      fatal("`" + object.cls.name + "` has no field or method `" + name + "`", line);
    }
    if (object instanceof Case) {
      const at = object.def.cases[object.index][1].indexOf(name);
      if (at >= 0) return object.fields[at];
      fatal("`" + object.def.cases[object.index][0] + "` has no field `" + name + "`", line);
    }
    if (object instanceof Enum) {
      const at = object.cases.findIndex((c: any) => c[0] === name);
      if (at >= 0) return caseOf(object, at);
      fatal("enum `" + object.name + "` has no case `" + name + "`", line);
    }
    if (object instanceof Module) return builtin(object.name + "." + name, line);
    if (object instanceof Quantity && isUnit(name)) {
      const scale = convert(object.unit, name);
      if (scale === null) fatal("a quantity in " + object.unit + " cannot be turned into " + name + ". They measure different things", line);
      return new Quantity(apply(scale, object.v), name);
    }
    if (object instanceof ZError && name === "message") return object.message;
    if (object instanceof ZError && name === "kind") return object.kind;
    if (typeof object === "function" && object.zbuiltin && name === "wrap" && WIDTHS[object.zbuiltin]) return builtin(object.zbuiltin + ".wrap", line);
    if (hasMethod(object, name)) return new Method(object, name);
    fatal(typeName(object) + " has no field or method `" + name + "`", line);
  }

  function setField(object: any, name: any, value: any, line: any) {
    if (object instanceof Instance) {
      const at = object.cls.fields.indexOf(name);
      if (at < 0) fatal("`" + object.cls.name + "` has no field `" + name + "`", line);
      object.fields[at] = value;
      return;
    }
    fatal("cannot set a field on " + typeName(object), line);
  }

  // ------------------------------------------------------------- Calls

  function counted(n: any, word: any) { return n + " " + word + (n === 1 ? "" : "s"); }
  function were(n: any) { return n === 1 ? "1 was" : n + " were"; }

  // Line the arguments up with the parameters: one slot each, MISSING
  // where none was given.
  function bind(what: any, names: any, positional: any, namedNames: any, named: any, line: any) {
    if (positional.length > names.length) fatal("`" + what + "` takes " + counted(names.length, "argument") + ", but " + were(positional.length) + " given", line);
    const slots = positional.slice();
    while (slots.length < names.length) slots.push(MISSING);
    for (let i = 0; i < namedNames.length; i++) {
      const at = names.indexOf(namedNames[i]);
      if (at < 0) fatal("`" + what + "` has no argument named `" + namedNames[i] + "`", line);
      if (slots[at] !== MISSING) fatal("`" + what + "` was given `" + namedNames[i] + "` twice", line);
      slots[at] = named[i];
    }
    return slots;
  }

  // How many calls are nested now. `zeno run` stops at 1000, long before
  // JavaScript's own limit, so a program stops at the same call here.
  let depth = 0;
  const MAX_DEPTH = 1000;

  function enter(fn: any, line: any) {
    if (depth >= MAX_DEPTH) fatal("more than 1000 calls are nested. Is `" + (fn.zname || "function") + "` calling itself forever?", line);
    depth++;
  }

  // A call of a function of the program known where it is written, with
  // its arguments lined up already.
  function direct(fn: any, line: any, ...args: any[]) {
    enter(fn, line);
    try {
      return fn(...args);
    } finally {
      depth--;
    }
  }

  function callFunction(fn: any, positional: any, namedNames: any, named: any, line: any) {
    const name = fn.zname || "function";
    enter(fn, line);
    try {
      const slots = bind(name, fn.zparams, positional, namedNames, named, line);
      for (let i = 0; i < slots.length; i++) {
        if (slots[i] === MISSING && !fn.zdefaults[i]) fatal("`" + name + "` needs a value for `" + fn.zparams[i] + "`", line);
      }
      return fn.apply(null, slots);
    } finally {
      depth--;
    }
  }

  function noNamed(what: any, namedNames: any, line: any) {
    if (namedNames.length > 0) fatal("`" + what + "` does not take a named argument `" + namedNames[0] + "`", line);
  }

  function callNamed(callee: any, positional: any, namedNames: any, named: any, line: any): any {
    if (typeof callee === "function") {
      if (callee.zbuiltin) { noNamed(callee.zname, namedNames, line); return callee(positional, line); }
      return callFunction(callee, positional, namedNames, named, line);
    }
    if (callee instanceof Bound) return callFunction(callee.fn, [callee.object].concat(positional), namedNames, named, line);
    if (callee instanceof Method) { noNamed(callee.name, namedNames, line); return callMethodOf(callee.receiver, callee.name, positional, line); }
    if (callee instanceof Class) {
      const slots = bind(callee.name, callee.fields, positional, namedNames, named, line);
      for (let i = 0; i < slots.length; i++) {
        if (slots[i] !== MISSING) continue;
        if (callee.defaults[i] === null) fatal("`" + callee.name + "` needs a value for `" + callee.fields[i] + "`", line);
        slots[i] = callee.defaults[i]();
      }
      return new Instance(callee, slots);
    }
    if (callee instanceof Constructor) {
      const [name, fields] = callee.def.cases[callee.index];
      const slots = bind(name, fields, positional, namedNames, named, line);
      for (let i = 0; i < slots.length; i++) if (slots[i] === MISSING) fatal("`" + name + "` needs a value for `" + fields[i] + "`", line);
      return new Case(callee.def, callee.index, slots);
    }
    if (callee instanceof ErrorType) { noNamed(callee.kind, namedNames, line); return newError(callee.kind, positional, line); }
    fatal(typeName(callee) + " cannot be called", line);
  }

  const call = (callee: any, args: any, line: any): any => callNamed(callee, args, [], [], line);

  function newError(kind: any, args: any, line: any) {
    if (args.length > 1) fatal("`" + kind + "` takes one message", line);
    return new ZError(kind, args.length === 0 ? "" : show(args[0]), null);
  }

  // `object.name(args)`
  function callMethodNamed(object: any, name: any, positional: any, namedNames: any, named: any, line: any) {
    if (object instanceof Instance) {
      const at = object.cls.fields.indexOf(name);
      if (at < 0) {
        const method = object.cls.methods.get(name);
        if (method) return callFunction(method, [object].concat(positional), namedNames, named, line);
      }
    }
    if (object instanceof Module) { noNamed(name, namedNames, line); return callBuiltin(object.name + "." + name, positional, line); }
    if (typeof object === "string" || Array.isArray(object) || object instanceof Dict) {
      noNamed(name, namedNames, line);
      return callMethodOf(object, name, positional, line);
    }
    return callNamed(field(object, name, line), positional, namedNames, named, line);
  }
  const callMethod = (object: any, name: any, args: any, line: any) => callMethodNamed(object, name, args, [], [], line);

  // ------------------------------------------------------------- Errors

  function raiseValue(value: any, line: any) {
    if (value instanceof ZError) throw new Raise(value, line);
    if (value instanceof Instance) throw new Raise(new ZError(value.cls.name, show(value), value), line);
    fatal("only an error can be raised, and this is " + typeName(value), line);
  }

  // Is this what the program raised, or a mistake that nothing catches?
  function raised(e: any) {
    if (e instanceof Raise) return e;
    if (e instanceof RangeError && /call stack/.test(e.message)) {
      return null;
    }
    return null;
  }

  // `except name` catches this error?
  function catches(e: any, name: any) {
    const kind = e.value.kind;
    return name === "Error" || name === kind;
  }

  // What `except name as e` calls `e`.
  function caught(e: any, name: any) {
    if (name !== "" && name !== "Error") return e.value.object !== null ? e.value.object : e.value;
    return e.value;
  }

  // ------------------------------------------------- Built-in functions

  function arity(name: any, args: any, min: any, max: any, line: any) {
    if (args.length >= min && args.length <= max) return;
    const wanted = min === max ? counted(min, "argument") : min + " to " + max + " arguments";
    fatal("`" + name + "` takes " + wanted + ", but " + were(args.length) + " given", line);
  }
  function wrongType(name: any, wanted: any, got: any, line: any) { fatal("`" + name + "` needs " + wanted + ", but this is " + typeName(got), line); }
  function text(name: any, v: any, line: any) { if (typeof v !== "string") wrongType(name, "a str", v, line); return v; }
  function whole(name: any, v: any, line: any) { if (!isInt(v)) wrongType(name, "a whole number", v, line); return v; }
  const isBlank = (c: any) => c === " " || c === "\t" || c === "\n" || c === "\r" || c === "\f";
  function strip(s: any) {
    let a = 0, b = s.length;
    while (a < b && isBlank(s[a])) a++;
    while (b > a && isBlank(s[b - 1])) b--;
    return s.slice(a, b);
  }

  // What is printed goes out in large pieces, which is far faster than a
  // line at a time.
  let output = "";
  function flush() {
    if (output === "") return;
    if (inNode) {
      try { fs.writeSync(1, output); } catch (e: any) { if (e.code === "EPIPE") process.exit(0); if (e.code !== "EAGAIN") throw e; }
    } else {
      writePage(output);
    }
    output = "";
  }
  function write(s: any) {
    output += s;
    if (output.length > 8192) flush();
  }
  let writePage = (s: any) => { console.log(s); };

  // Lines typed at the program, read as it asks for them.
  let typed = inNode ? Buffer.alloc(0) : null;
  let typedEnd = false;
  function readLine() {
    if (!inNode) return null;
    for (;;) {
      const at = typed.indexOf(10);
      if (at >= 0) {
        // With its "\n", so that the caller knows the line was ended.
        const line = typed.subarray(0, at + 1);
        typed = typed.subarray(at + 1);
        return line;
      }
      if (typedEnd) {
        if (typed.length === 0) return null;
        const line = typed;
        typed = Buffer.alloc(0);
        return line;
      }
      const chunk = Buffer.alloc(65536);
      let n = 0;
      try { n = fs.readSync(0, chunk, 0, chunk.length, null); } catch (e: any) { if (e.code === "EAGAIN") continue; n = 0; }
      if (n === 0) typedEnd = true;
      else typed = Buffer.concat([typed, chunk.subarray(0, n)]);
    }
  }

  const WIDTHS: any = {
    u8: [0, 255, 8, false], u16: [0, 65535, 16, false], u32: [0, 4294967295, 32, false],
    i8: [-128, 127, 8, true], i16: [-32768, 32767, 16, true], i32: [-2147483648, 2147483647, 32, true],
  };

  function amount(v: any) { return v instanceof Quantity ? v.v : number(v); }

  function numbers(name: any, args: any, line: any) {
    const values = args.length === 1 && Array.isArray(args[0]) ? args[0].slice() : args;
    let u: any = null;
    values.forEach((item: any, place: any) => {
      if (item instanceof Quantity) {
        if (u !== null && u !== item.unit) fatal("`" + name + "` needs quantities in one unit, but these are in " + u + " and in " + item.unit + ". Turn one into the other first, as in `x." + u + "`", line);
        if (u === null && place > 0) fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line);
        u = item.unit;
      } else if (number(item) === null) wrongType(name, "numbers", item, line);
      else if (u !== null) fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line);
    });
    return [values, u];
  }

  function pick(name: any, args: any, line: any, better: any) {
    let [values] = numbers(name, args, line);
    if (values.length === 0) raise("ValueError", "`" + name + "` of nothing", line);
    if (values.some((item: any) => item instanceof F32) && values.every((item: any) => item instanceof F32 || isInt(item))) values = values.map((item: any) => f32(single(item)));
    let best = values[0];
    for (const item of values.slice(1)) {
      const wins = item instanceof U64 && best instanceof U64 ? (name === "min" ? item.v < best.v : item.v > best.v) : better(amount(item), amount(best));
      if (wins) best = item;
    }
    return best;
  }

  function parseFloatText(s: any) {
    if (!/^[+-]?(\d+\.?\d*([eE][+-]?\d+)?|\.\d+([eE][+-]?\d+)?|inf|infinity|nan)$/i.test(s)) return null;
    const lower = s.toLowerCase().replace(/^\+/, "");
    if (lower === "inf" || lower === "infinity") return Infinity;
    if (lower === "-inf" || lower === "-infinity") return -Infinity;
    if (lower === "nan" || lower === "-nan") return NaN;
    return Number(s);
  }

  // Heapsort, step for step as builtins.rs and runtime.c do it, so that
  // values that are neither before the other end up where they do there.
  function heapsort(xs: any, before: any) {
    const sift = (root: any, end: any) => {
      for (;;) {
        let child = 2 * root + 1;
        if (child >= end) return;
        if (child + 1 < end && before(child, child + 1)) child += 1;
        if (!before(root, child)) return;
        swap(root, child);
        root = child;
      }
    };
    let swap = (i: any, j: any) => { const t = xs[i]; xs[i] = xs[j]; xs[j] = t; };
    return { sift, setSwap: (s: any) => { swap = s; }, run: () => {
      const n = xs.length;
      for (let root = Math.floor(n / 2) - 1; root >= 0; root--) sift(root, n);
      for (let end = n - 1; end >= 1; end--) { swap(0, end); sift(0, end); }
    } };
  }

  function sortList(items: any, line: any) {
    const sorted = items.slice();
    const h = heapsort(sorted, (i: any, j: any) => lt(sorted[i], sorted[j], line));
    h.run();
    for (let i = 0; i < sorted.length; i++) items[i] = sorted[i];
  }

  function sortBy(items: any, fn: any, line: any) {
    const keys: any[] = [];
    for (const item of items.slice()) {
      const k = call(fn, [item], line);
      if (!isInt(k) && !(k instanceof Float) && !(k instanceof F32) && !(k instanceof U64) && typeof k !== "string") fatal("the function given to `sort` must return a number or text to order by, but it returned " + typeName(k), line);
      keys.push(k);
    }
    const sorted = items.slice();
    if (sorted.length !== keys.length) raise("ValueError", "the function given to `sort` changed how many items the list has", line);
    const places = sorted.map((_: any, i: any) => i);
    const before = (a: any, b: any) => (lt(keys[a], keys[b], line) ? true : lt(keys[b], keys[a], line) ? false : places[a] < places[b]);
    const h = heapsort(sorted, before);
    h.setSwap((i: any, j: any) => {
      let t = sorted[i]; sorted[i] = sorted[j]; sorted[j] = t;
      t = keys[i]; keys[i] = keys[j]; keys[j] = t;
      t = places[i]; places[i] = places[j]; places[j] = t;
    });
    h.run();
    items.length = 0;
    for (const item of sorted) items.push(item);
  }

  function slice(args: any, length: any, what: any, line: any) {
    const from = whole("slice", args[0], line);
    const to = args.length > 1 ? whole("slice", args[1], line) : length;
    if (from < 0 || to < from || to > length) raise("IndexError", "slice " + from + " to " + to + " is out of range for a " + what + " of length " + length, line);
    return [Number(from), Number(to)];
  }

  const STR_METHODS = ["upper", "lower", "strip", "split", "lines", "replace", "starts_with", "ends_with", "find", "slice", "code"];
  const LIST_METHODS = ["append", "pop", "map", "filter", "reduce", "insert", "reverse", "sort", "clear", "find", "slice", "join"];
  const DICT_METHODS = ["get", "pop", "keys", "values", "clear"];
  function hasMethod(v: any, name: any) {
    if (typeof v === "string") return STR_METHODS.includes(name);
    if (Array.isArray(v)) return LIST_METHODS.includes(name);
    if (v instanceof Dict) return DICT_METHODS.includes(name);
    return false;
  }

  function upperAscii(s: any) { return s.replace(/[a-z]+/g, (m: any) => m.toUpperCase()); }
  function lowerAscii(s: any) { return s.replace(/[A-Z]+/g, (m: any) => m.toLowerCase()); }

  function callMethodOf(receiver: any, name: any, args: any, line: any): any {
    if (typeof receiver === "string") {
      const s = receiver;
      switch (name) {
        case "upper": arity(name, args, 0, 0, line); return upperAscii(s);
        case "lower": arity(name, args, 0, 0, line); return lowerAscii(s);
        case "strip": arity(name, args, 0, 0, line); return strip(s);
        case "split": {
          arity(name, args, 0, 1, line);
          if (args.length === 0) return s.split(/[ \t\n\r\f]/).filter((w: any) => w !== "");
          const sep = text(name, args[0], line);
          if (sep === "") raise("ValueError", "cannot split on an empty str", line);
          return s.split(sep);
        }
        case "lines": {
          arity(name, args, 0, 0, line);
          // A "\r" goes with the "\n" just after it, and with nothing else.
          if (s === "") return [];
          const parts = s.split("\n");
          const last = parts.pop();
          const lines = parts.map((p: any) => (p.endsWith("\r") ? p.slice(0, -1) : p));
          if (last !== "") lines.push(last);
          return lines;
        }
        case "replace": {
          arity(name, args, 2, 2, line);
          const old = text(name, args[0], line), now = text(name, args[1], line);
          if (old === "") {
            // Rust puts the new text between every character, and at both ends.
            return now + chars(s).join(now) + (s === "" ? "" : now);
          }
          return s.split(old).join(now);
        }
        case "starts_with": arity(name, args, 1, 1, line); return s.startsWith(text(name, args[0], line));
        case "ends_with": arity(name, args, 1, 1, line); return s.endsWith(text(name, args[0], line));
        case "find": {
          arity(name, args, 1, 1, line);
          const at = s.indexOf(text(name, args[0], line));
          return at < 0 ? null : strLen(s.slice(0, at));
        }
        case "slice": {
          arity(name, args, 1, 2, line);
          const cs = chars(s);
          const [from, to] = slice(args, cs.length, "str", line);
          return cs.slice(from, to).join("");
        }
        case "code": {
          arity(name, args, 0, 0, line);
          const cs = chars(s);
          if (cs.length !== 1) raise("ValueError", "`code` needs a str of exactly one character, and this one has " + cs.length, line);
          return cs[0].codePointAt(0);
        }
      }
      fatal("a str has no method `" + name + "`", line);
    }
    if (receiver instanceof Dict) {
      const map = receiver.map;
      switch (name) {
        case "get": {
          arity(name, args, 1, 2, line);
          const k = key(args[0], line);
          if (map.has(k)) return map.get(k);
          return args.length > 1 ? args[1] : null;
        }
        case "pop": {
          arity(name, args, 1, 1, line);
          const k = key(args[0], line);
          if (!map.has(k)) raise("KeyError", "this dict has no key " + repr(args[0]), line);
          const value = map.get(k);
          map.delete(k);
          return value;
        }
        case "keys": arity(name, args, 0, 0, line); return Array.from(map.keys());
        case "values": arity(name, args, 0, 0, line); return Array.from(map.values());
        case "clear": arity(name, args, 0, 0, line); map.clear(); return null;
      }
      fatal("a dict has no method `" + name + "`", line);
    }
    if (Array.isArray(receiver)) {
      const xs = receiver;
      switch (name) {
        case "append": arity(name, args, 1, 1, line); xs.push(args[0]); return null;
        case "pop": {
          arity(name, args, 0, 1, line);
          if (args.length === 1) {
            const at = position(args[0], xs.length, "list", line);
            return xs.splice(at, 1)[0];
          }
          if (xs.length === 0) raise("IndexError", "cannot pop from an empty list", line);
          return xs.pop();
        }
        case "insert": {
          arity(name, args, 2, 2, line);
          const at = whole(name, args[0], line);
          if (at < 0 || at > xs.length) raise("IndexError", "cannot insert at " + at + " in a list of length " + xs.length, line);
          xs.splice(Number(at), 0, args[1]);
          return null;
        }
        case "reverse": arity(name, args, 0, 0, line); xs.reverse(); return null;
        case "clear": arity(name, args, 0, 0, line); xs.length = 0; return null;
        case "sort":
          if (args.length === 1) { sortBy(xs, args[0], line); return null; }
          arity(name, args, 0, 1, line);
          sortList(xs, line);
          return null;
        case "find": {
          arity(name, args, 1, 1, line);
          const at = xs.findIndex((item: any) => equal(item, args[0]));
          return at < 0 ? null : at;
        }
        case "slice": {
          arity(name, args, 1, 2, line);
          const [from, to] = slice(args, xs.length, "list", line);
          return xs.slice(from, to);
        }
        case "join": {
          arity(name, args, 1, 1, line);
          const sep = text(name, args[0], line);
          for (const item of xs) {
            if (typeof item !== "string") fatal("`join` puts text together, and this list holds " + typeName(item) + ". Make each item text first, with `str( )`", line);
          }
          return xs.join(sep);
        }
        case "map": {
          arity(name, args, 1, 1, line);
          return xs.slice().map((item: any) => call(args[0], [item], line));
        }
        case "filter": {
          arity(name, args, 1, 1, line);
          const kept = [];
          for (const item of xs.slice()) {
            const keep = call(args[0], [item], line);
            if (keep === true) kept.push(item);
            else if (keep !== false) fatal("the function given to `filter` must return true or false, but it returned " + typeName(keep), line);
          }
          return kept;
        }
        case "reduce": {
          arity(name, args, 2, 2, line);
          let total = args[1];
          for (const item of xs.slice()) total = call(args[0], [total, item], line);
          return total;
        }
      }
      fatal("a list has no method `" + name + "`", line);
    }
    fatal(typeName(receiver) + " has no method `" + name + "`", line);
  }

  // The format after the colon of `{value:format}`.
  function format(v: any, spec: any, line: any) {
    if (spec === "") return show(v);
    const bad = () => fatal("`" + spec + "` is not a format Zeno understands", line);
    const m: any = /^([<>^])?(0)?(\d*)(?:\.(\d*))?(.)?$/.exec(spec);
    if (!m) bad();
    const align = m[1], zero = m[2] !== undefined, width = m[3] ? Number(m[3]) : 0;
    const precision = m[4] !== undefined ? Number(m[4] || 0) : null;
    const kind = m[5];
    if (kind !== undefined && /\d/.test(kind)) bad();
    let body;
    if ((kind === "f" || (kind === undefined && precision !== null)) && v instanceof Quantity) {
      body = fixed(v.v, precision === null ? 6 : precision) + " " + v.unit;
    } else if (kind === "f" || (kind === undefined && precision !== null && v instanceof Float)) {
      const x = number(v);
      if (x === null) wrongType(spec, "a number", v, line);
      body = fixed(x, precision === null ? 6 : precision);
    } else if ((kind === undefined && precision === null) || (kind === "d" && precision === null && (isInt(v) || v instanceof U64))) {
      body = show(v);
    } else if ((kind === "x" || kind === "X" || kind === "b") && precision === null && v instanceof U64) {
      body = v.v.toString(kind === "b" ? 2 : 16);
      if (kind === "X") body = body.toUpperCase();
    } else if ((kind === "x" || kind === "X" || kind === "b") && precision === null && isInt(v)) {
      const n = big(v);
      const magnitude = n < 0n ? -n : n;
      let digits = magnitude.toString(kind === "b" ? 2 : 16);
      if (kind === "X") digits = digits.toUpperCase();
      body = (n < 0n ? "-" : "") + digits;
    } else if ((kind === "d" || kind === "x" || kind === "X" || kind === "b") && precision === null) {
      wrongType(spec, "an int", v, line);
    } else {
      bad();
    }
    const padding = Math.max(0, width - strLen(body));
    const isNumber = number(v) !== null || v instanceof Quantity;
    if (zero && isNumber) {
      const negative = body.startsWith("-");
      return (negative ? "-" : "") + "0".repeat(padding) + (negative ? body.slice(1) : body);
    }
    switch (align || (isNumber ? ">" : "<")) {
      case "<": return body + " ".repeat(padding);
      case ">": return " ".repeat(padding) + body;
      default: {
        const left = Math.floor(padding / 2);
        return " ".repeat(left) + body + " ".repeat(padding - left);
      }
    }
  }

  // ------------------------------------------------------------ JSON

  function jsonParse(s: any) {
    const cs: any[] = Array.from(s);
    let at = 0;
    const spaces = () => { while (at < cs.length && " \t\n\r".includes(cs[at])) at++; };
    const word = (w: any) => {
      const here = cs.slice(at, at + w.length).join("") === w;
      if (here) at += w.length;
      return here;
    };
    const fourHex = () => {
      const digits = cs.slice(at, at + 4).join("");
      at += 4;
      if (!/^[0-9a-fA-F]{4}$/.test(digits)) throw 0;
      return parseInt(digits, 16);
    };
    const textValue = () => {
      if (cs[at] !== '"') throw 0;
      at++;
      let out = "";
      for (;;) {
        if (at >= cs.length) throw 0;
        const c = cs[at++];
        if (c === '"') return out;
        if (c === "\\") {
          if (at >= cs.length) throw 0;
          const e = cs[at++];
          if (e === "n") out += "\n";
          else if (e === "t") out += "\t";
          else if (e === "r") out += "\r";
          else if (e === "b") out += "\b";
          else if (e === "f") out += "\f";
          else if (e === "u") {
            let code = fourHex();
            if (code >= 0xd800 && code < 0xdc00 && word("\\u")) {
              const low = fourHex();
              if (low < 0xdc00 || low > 0xdfff) throw 0;
              code = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
            }
            if (code >= 0xd800 && code <= 0xdfff) throw 0;
            out += String.fromCodePoint(code);
          } else out += e;
        } else out += c;
      }
    };
    const value = (): any => {
      spaces();
      if (at >= cs.length) throw 0;
      const c = cs[at];
      if (c === "n") { if (word("null")) return null; throw 0; }
      if (c === "t") { if (word("true")) return true; throw 0; }
      if (c === "f") { if (word("false")) return false; throw 0; }
      if (c === '"') return textValue();
      if (c === "[") {
        at++;
        const items = [];
        for (;;) {
          spaces();
          if (word("]")) return items;
          if (items.length > 0 && !word(",")) throw 0;
          items.push(value());
        }
      }
      if (c === "{") {
        at++;
        const d = new Dict();
        let count = 0;
        for (;;) {
          spaces();
          if (word("}")) return d;
          if (count > 0 && !word(",")) throw 0;
          spaces();
          const name = textValue();
          spaces();
          if (!word(":")) throw 0;
          const v = value();
          count++;
          // A member that is null is left out: a dict holds no none.
          if (v === null) continue;
          if (d.map.has(name)) d.map.set(name, v); else d.map.set(name, v);
        }
      }
      const from = at;
      while (at < cs.length && /[0-9+\-.eE]/.test(cs[at])) at++;
      const x = parseFloatText(cs.slice(from, at).join(""));
      if (x === null) throw 0;
      if (Number.isInteger(x) && Math.abs(x) < 9.0e15) return x === 0 ? 0 : x;
      return float(x);
    };
    try {
      const v = value();
      spaces();
      if (at !== cs.length) return undefined;
      return v;
    } catch (e: any) {
      if (e === 0) return undefined;
      throw e;
    }
  }

  function jsonText(s: any) {
    let out = '"';
    for (const c of s) {
      const code = c.codePointAt(0);
      if (c === '"') out += '\\"';
      else if (c === "\\") out += "\\\\";
      else if (c === "\n") out += "\\n";
      else if (c === "\r") out += "\\r";
      else if (c === "\t") out += "\\t";
      else if (code < 0x20) out += "\\u" + code.toString(16).padStart(4, "0");
      else out += c;
    }
    return out + '"';
  }

  function jsonWrite(v: any, line: any): any {
    if (v === null) return "null";
    if (v === true) return "true";
    if (v === false) return "false";
    if (isInt(v)) return String(v);
    if (v instanceof Float) {
      if (!Number.isFinite(v.v)) raise("ValueError", "JSON has no way to write " + (Number.isNaN(v.v) ? "NaN" : v.v > 0 ? "inf" : "-inf"), line);
      return showFloat(v.v);
    }
    if (typeof v === "string") return jsonText(v);
    if (Array.isArray(v)) return "[" + v.map((item: any) => jsonWrite(item, line)).join(",") + "]";
    if (v instanceof Dict) {
      const parts = [];
      for (const [k, value] of v.map) parts.push(jsonText(show(k)) + ":" + jsonWrite(value, line));
      return "{" + parts.join(",") + "}";
    }
    if (v instanceof Instance) {
      return "{" + v.cls.fields.map((name: any, i: any) => jsonText(name) + ":" + jsonWrite(v.fields[i], line)).join(",") + "}";
    }
    if (v instanceof Quantity) raise("ValueError", "JSON has no units, and this is a quantity in " + v.unit + '. Write its number, and put the unit in the name: `["' + v.unit + '": ...]`', line);
    raise("ValueError", "JSON has objects, lists, text, numbers, bools and none, and this is " + typeName(v), line);
  }

  // ----------------------------------------------------------- Files

  function at(path: any) {
    if (!inNode) return "nothing";
    let fd;
    try { fd = fs.openSync(path, "r"); } catch (e: any) { return e.code === "ENOENT" ? "nothing" : "other"; }
    try { fs.readSync(fd, Buffer.alloc(1), 0, 1, null); return "file"; } catch (e: any) { return "other"; } finally { fs.closeSync(fd); }
  }
  function paths(name: any, args: any, count: any, line: any) {
    arity(name, args, count, count, line);
    return args.map((a: any) => text(name, a, line));
  }
  const fileError = (message: any, line: any) => raise("FileError", message, line);

  // ------------------------------------------------------------ Clock

  const started = typeof performance !== "undefined" ? performance.now() : Date.now();
  const clock = () => (typeof performance !== "undefined" ? performance.now() : Date.now()) - started;

  // ------------------------------------------------- The built-in table

  // `u64.wrap(a + b)`: the answer cut to 64 bits (42.6).
  function u64Wrapping(op: any, args: any, line: any) {
    arity("u64.wrap", args, 2, 2, line);
    const [a, b] = args;
    if (op === "<<" && unsigned(a) !== null && (isInt(b) || b instanceof U64)) return new U64(BigInt.asUintN(64, (unsigned(a) << u64ShiftCount(b, line)) as any));
    const x = unsigned(a), y = unsigned(b);
    if (x === null || y === null) wrongType("u64.wrap", "u64s", x === null ? a : b, line);
    return new U64(BigInt.asUintN(64, (op === "+" ? x + y : op === "-" ? x - y : x * y) as any));
  }

  const BUILTINS: any = {
    print(args: any) { write(args.map(show).join(" ") + "\n"); return null; },
    input(args: any, line: any) {
      arity("input", args, 0, 1, line);
      const question = args.length > 0 ? text("input", args[0], line) : "";
      if (!inNode) {
        flush();
        const answer = typeof prompt === "function" ? prompt(question) : null;
        return answer === null ? null : answer;
      }
      write(question);
      flush();
      const bytes = readLine();
      if (bytes === null) return null;
      let s = Buffer.from(bytes).toString("utf8");
      // The line end is not part of the line: "\n", or "\r\n". A "\r"
      // at the end of input that no "\n" follows stays.
      if (s.endsWith("\n")) {
        s = s.slice(0, -1);
        if (s.endsWith("\r")) s = s.slice(0, -1);
      }
      return s.replace(/\0/g, "");
    },
    len(args: any, line: any) {
      arity("len", args, 1, 1, line);
      const v = args[0];
      if (typeof v === "string") return strLen(v);
      if (Array.isArray(v)) return v.length;
      if (v instanceof Dict) return v.map.size;
      wrongType("len", "a str, a list or a dict", v, line);
    },
    range(args: any, line: any) {
      arity("range", args, 1, 3, line);
      for (const a of args) if (!isInt(a)) wrongType("range", "ints", a, line);
      let [start, stop, step] = args.length === 1 ? [0, args[0], 1] : args.length === 2 ? [args[0], args[1], 1] : args;
      if (step == 0) raise("ValueError", "the step of a range cannot be 0", line);
      const out = [];
      if (Number.isSafeInteger(start) && Number.isSafeInteger(stop) && Number.isSafeInteger(step) && Number.isSafeInteger(stop + step)) {
        if (step > 0) for (let i = start; i < stop; i += step) out.push(i);
        else for (let i = start; i > stop; i += step) out.push(i);
        return out;
      }
      // Ints too big for a JavaScript number to hold exactly. A step
      // past the last int there is ends the range.
      start = big(start); stop = big(stop); step = big(step);
      for (let i = start; (step > 0n ? i < stop : i > stop) && i >= MIN && i <= MAX; i += step) out.push(int(i, line));
      return out;
    },
    int(args: any, line: any) {
      arity("int", args, 1, 1, line);
      const v = args[0];
      if (isInt(v)) return v;
      if (v instanceof F32) return BUILTINS.int([float(v.v)], line);
      if (v instanceof U64) {
        if (v.v > MAX) raise("OverflowError", v.v + " does not fit in an int (-9223372036854775808 to 9223372036854775807)", line);
        return int(v.v, line);
      }
      if (v instanceof Float) {
        if (!Number.isFinite(v.v)) raise("ValueError", "cannot turn " + (Number.isNaN(v.v) ? "NaN" : v.v > 0 ? "inf" : "-inf") + " into an int", line);
        const t = Math.trunc(v.v);
        if (t >= 9223372036854775807) return int(MAX, line);
        if (t <= -9223372036854775808) return int(MIN, line);
        return Number.isSafeInteger(t) ? (t === 0 ? 0 : t) : int(BigInt(t), line);
      }
      if (typeof v === "string") {
        const s = strip(v).replace(/_/g, "");
        if (/^[+-]?\d+$/.test(s)) {
          const b = BigInt(s);
          if (b <= MAX && b >= MIN) return int(b, line);
        }
        raise("ValueError", "cannot turn " + quoted(v) + " into an int", line);
      }
      wrongType("int", "a number or a str", v, line);
    },
    float(args: any, line: any): any {
      arity("float", args, 1, 1, line);
      const v = args[0];
      if (isInt(v)) return float(Number(v));
      if (v instanceof Float) return v;
      if (v instanceof F32) return float(v.v);
      if (v instanceof U64) return float(Number(v.v));
      if (typeof v === "string") {
        const x = parseFloatText(strip(v));
        if (x === null) raise("ValueError", "cannot turn " + quoted(v) + " into a float", line);
        return float(x);
      }
      wrongType("float", "a number or a str", v, line);
    },
    f32(args: any, line: any) {
      arity("f32", args, 1, 1, line);
      const v = args[0];
      if (v instanceof F32) return v;
      if (isInt(v)) return f32(single(v));
      if (v instanceof Float) {
        const r = Math.fround(v.v);
        if (!Number.isFinite(r) && Number.isFinite(v.v)) raise("OverflowError", showFloat(v.v) + " does not fit in an f32, whose largest is about 3.4e38", line);
        return f32(r);
      }
      wrongType("f32", "a number", v, line);
    },
    u64(args: any, line: any) {
      arity("u64", args, 1, 1, line);
      const v = args[0];
      if (v instanceof U64) return v;
      if (isInt(v)) {
        if (v < 0) raise("OverflowError", v + " does not fit in a u64 (0 to 18446744073709551615)", line);
        return new U64(big(v));
      }
      wrongType("u64", "a whole number", v, line);
    },
    // `u64.wrap(n)`: the 64 bits of an int read with no sign (42.6).
    "u64..of"(args: any, line: any) {
      arity("u64.wrap", args, 1, 1, line);
      const v = args[0];
      if (v instanceof U64) return v;
      if (isInt(v)) return new U64(BigInt.asUintN(64, big(v)));
      wrongType("u64.wrap", "a whole number", v, line);
    },
    "u64..add"(args: any, line: any) { return u64Wrapping("+", args, line); },
    "u64..sub"(args: any, line: any) { return u64Wrapping("-", args, line); },
    "u64..mul"(args: any, line: any) { return u64Wrapping("*", args, line); },
    "u64..shl"(args: any, line: any) { return u64Wrapping("<<", args, line); },
    str(args: any, line: any) { arity("str", args, 1, 1, line); return show(args[0]); },
    abs(args: any, line: any) {
      arity("abs", args, 1, 1, line);
      const v = args[0];
      if (isInt(v)) return v < 0 ? neg(v, line) : v;
      if (v instanceof Float) return float(Math.abs(v.v));
      if (v instanceof F32) return f32(Math.abs(v.v));
      if (v instanceof U64) return v;
      if (v instanceof Quantity) return new Quantity(Math.abs(v.v), v.unit);
      wrongType("abs", "a number", v, line);
    },
    min(args: any, line: any) { return pick("min", args, line, (a: any, b: any) => a < b); },
    max(args: any, line: any) { return pick("max", args, line, (a: any, b: any) => a > b); },
    sum(args: any, line: any) {
      const [values, u] = numbers("sum", args, line);
      if (u !== null) fatal("`sum` works on plain numbers, and these have a unit. Add quantities with `+`, which keeps the unit: start from `var total = 0 V` and add each in a loop", line);
      if (values.some((item: any) => item instanceof F32) && values.every((item: any) => item instanceof F32 || isInt(item))) {
        let single32 = 0;
        for (const item of values) single32 = Math.fround(single32 + single(item));
        return f32(single32);
      }
      if (values.some((item: any) => item instanceof U64)) {
        let total = 0n;
        for (const item of values) {
          total += unsigned(item) ?? 0n;
          if (total > U64_MAX) raise("OverflowError", U64_TOO_BIG, line);
        }
        return new U64(total);
      }
      let total = 0, allInts = true, floatTotal = 0;
      for (const item of values) {
        floatTotal += number(item);
        if (allInts && isInt(item)) total = add(total, item, line);
        else allInts = false;
      }
      return allInts ? total : float(floatTotal);
    },
    char(args: any, line: any) {
      arity("char", args, 1, 1, line);
      const code = whole("char", args[0], line);
      if (code <= 0 || code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff)) raise("ValueError", code + " is not the code of a character", line);
      return String.fromCodePoint(Number(code));
    },
    allocate(args: any, line: any) { fatal("manual memory is only for a program compiled to machine code", line); },
    free(args: any, line: any) { fatal("manual memory is only for a program compiled to machine code", line); },

    "math.sqrt"(args: any, line: any) { const x = oneNumber("sqrt", args, line); if (x < 0) raise("ValueError", "cannot take the square root of " + showFloat(x), line); return float(Math.sqrt(x)); },
    "math.floor"(args: any, line: any) { return toInt(Math.floor(oneNumber("floor", args, line))); },
    "math.ceil"(args: any, line: any) { return toInt(Math.ceil(oneNumber("ceil", args, line))); },
    "math.round"(args: any, line: any) { const x = oneNumber("round", args, line); return toInt(x < 0 ? -Math.round(-x) : Math.round(x)); },
    "math.sin"(args: any, line: any) { return float(Math.sin(angle("sin", args, line))); },
    "math.cos"(args: any, line: any) { return float(Math.cos(angle("cos", args, line))); },
    "math.tan"(args: any, line: any) { return float(Math.tan(angle("tan", args, line))); },
    "math.asin"(args: any, line: any) { return new Quantity(Math.asin(ratio("asin", args, line)), "rad"); },
    "math.acos"(args: any, line: any) { return new Quantity(Math.acos(ratio("acos", args, line)), "rad"); },
    "math.atan"(args: any, line: any) { return new Quantity(Math.atan(oneNumber("atan", args, line)), "rad"); },
    "math.atan2"(args: any, line: any) {
      arity("atan2", args, 2, 2, line);
      const y = number(args[0]), x = number(args[1]);
      if (y === null) wrongType("atan2", "two numbers", args[0], line);
      if (x === null) wrongType("atan2", "two numbers", args[1], line);
      return new Quantity(Math.atan2(y, x), "rad");
    },
    "math.exp"(args: any, line: any) { return float(Math.exp(oneNumber("exp", args, line))); },
    "math.log"(args: any, line: any) { return float(Math.log(aboveZero("log", args, line))); },
    "math.log2"(args: any, line: any) { return float(Math.log2(aboveZero("log2", args, line))); },
    "math.log10"(args: any, line: any) { return float(Math.log10(aboveZero("log10", args, line))); },

    "time.sleep"(args: any, line: any) {
      arity("sleep", args, 1, 1, line);
      const v = args[0];
      if (!(v instanceof Quantity)) wrongType("sleep", "a time with a unit, such as `500 ms`", v, line);
      const scale = convert(v.unit, "s");
      if (scale === null) fatal("`sleep` needs a time, such as `500 ms`, not a quantity in " + v.unit, line);
      const seconds = apply(scale, v.v);
      if (seconds > 0) {
        if (!inNode) fatal("a program in a page cannot `sleep`: the page would stand still until it woke. Say what is to happen later with `web.after(500 ms, ...)`", line);
        flush();
        Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, Math.min(seconds, 1e9) * 1000);
      }
      return null;
    },
    "time.now"(args: any, line: any) { arity("now", args, 0, 0, line); return new Quantity(clock(), "ms"); },

    "files.read"(args: any, line: any) {
      const [path] = paths("read", args, 1, line);
      const where = at(path);
      if (where === "nothing") fileError("there is no file `" + path + "`", line);
      if (where === "other") fileError("cannot read `" + path + "`", line);
      let bytes;
      try { bytes = fs.readFileSync(path); } catch (e: any) { fileError("cannot read `" + path + "`", line); }
      let s;
      try { s = new TextDecoder("utf-8", { fatal: true }).decode(bytes); } catch (e: any) { s = null; }
      if (s === null || s.includes("\0")) fileError("`" + path + "` does not hold text", line);
      return s;
    },
    "files.write"(args: any, line: any) {
      const [path, s] = paths("write", args, 2, line);
      if (!inNode) fileError("cannot write `" + path + "`", line);
      try { fs.writeFileSync(path, s); } catch (e: any) { fileError("cannot write `" + path + "`", line); }
      return null;
    },
    "files.read_bytes"(args: any, line: any) {
      const [path] = paths("read_bytes", args, 1, line);
      const where = at(path);
      if (where === "nothing") fileError("there is no file `" + path + "`", line);
      if (where === "other") fileError("cannot read `" + path + "`", line);
      let bytes: any;
      try { bytes = fs.readFileSync(path); } catch (e) { fileError("cannot read `" + path + "`", line); }
      return Array.from(bytes);
    },
    "files.write_bytes"(args: any, line: any) {
      arity("write_bytes", args, 2, 2, line);
      const path = text("write_bytes", args[0], line);
      const items = args[1];
      if (!Array.isArray(items)) wrongType("write_bytes", "a list[u8]", items, line);
      for (const n of items) {
        if (!isInt(n)) wrongType("write_bytes", "a list[u8]", n, line);
        if (n < 0 || n > 255) raise("ValueError", String(n) + " is not a byte: a byte is a number from 0 to 255", line);
      }
      if (!inNode) fileError("cannot write `" + path + "`", line);
      try { fs.writeFileSync(path, Uint8Array.from(items, Number)); } catch (e) { fileError("cannot write `" + path + "`", line); }
      return null;
    },
    "files.append"(args: any, line: any) {
      const [path, s] = paths("append", args, 2, line);
      if (!inNode) fileError("cannot write `" + path + "`", line);
      try { fs.appendFileSync(path, s); } catch (e: any) { fileError("cannot write `" + path + "`", line); }
      return null;
    },
    "files.exists"(args: any, line: any) { const [path] = paths("exists", args, 1, line); return at(path) === "file"; },
    "files.remove"(args: any, line: any): any {
      const [path] = paths("remove", args, 1, line);
      const where = at(path);
      if (where === "nothing") fileError("there is no file `" + path + "`", line);
      if (where === "file") { try { fs.unlinkSync(path); return null; } catch (e: any) {} }
      fileError("cannot remove `" + path + "`", line);
    },

    // `program.exit(code)`: what was printed goes out, and Node ends
    // with `code`. A page cannot end: the run stops, and the page stays.
    "program.exit"(args: any, line: any) {
      arity("exit", args, 1, 1, line);
      const code = whole("exit", args[0], line);
      if (code < 0 || code > 255) raise("ValueError", "a program ends with a number from 0 to 255, not " + code, line);
      flush();
      if (inNode) process.exit(Number(code));
      throw new Exit(code);
    },
    // `program.environment(name)`: the text, or none. A page has no environment.
    "program.environment"(args: any, line: any) {
      arity("environment", args, 1, 1, line);
      const name = text("environment", args[0], line);
      if (!inNode || !Object.prototype.hasOwnProperty.call(process.env, name)) return null;
      return process.env[name];
    },
    // `program.warn(text)`: one line on the error stream, after what was printed.
    "program.warn"(args: any, line: any) {
      arity("warn", args, 1, 1, line);
      const said = text("warn", args[0], line);
      flush();
      if (inNode) fs.writeSync(2, said + "\n");
      else writePage(said + "\n");
      return null;
    },
    "program.arguments"(args: any, line: any) { arity("arguments", args, 0, 0, line); return inNode ? process.argv.slice(2) : []; },

    "json.parse"(args: any, line: any) {
      arity("parse", args, 1, 1, line);
      const s = text("parse", args[0], line);
      const v = jsonParse(s);
      if (v === undefined) {
        const start = Array.from(s.replace(/^[ \t\n\r]+/, ""));
        raise("ValueError", "this text is not JSON: `" + start.slice(0, 30).join("") + (start.length > 30 ? "..." : "") + "`", line);
      }
      return v;
    },
    "json.write"(args: any, line: any) { arity("write", args, 1, 1, line); return jsonWrite(args[0], line); },
  };

  function toInt(x: any) {
    if (Number.isNaN(x)) return 0;
    if (x >= 9223372036854775807) return int(MAX);
    if (x <= -9223372036854775808) return int(MIN);
    return Number.isSafeInteger(x) ? (x === 0 ? 0 : x) : int(BigInt(x));
  }
  function oneNumber(name: any, args: any, line: any) {
    arity(name, args, 1, 1, line);
    const x = number(args[0]);
    if (x === null) wrongType(name, "a number", args[0], line);
    return x;
  }
  function angle(name: any, args: any, line: any) {
    arity(name, args, 1, 1, line);
    const v = args[0];
    if (v instanceof Quantity) {
      const scale = convert(v.unit, "rad");
      if (scale === null) fatal("`" + name + "` needs a number or an angle, such as `30 deg`, not a quantity in " + v.unit, line);
      return apply(scale, v.v);
    }
    const x = number(v);
    if (x === null) wrongType(name, "a number or an angle, such as `30 deg`", v, line);
    return x;
  }
  function ratio(name: any, args: any, line: any) {
    const x = oneNumber(name, args, line);
    if (x < -1 || x > 1) raise("ValueError", "`" + name + "` needs a number from -1 to 1, and this is " + showFloat(x), line);
    return x;
  }
  function aboveZero(name: any, args: any, line: any) {
    const x = oneNumber(name, args, line);
    if (x <= 0) raise("ValueError", "cannot take the logarithm of " + showFloat(x), line);
    return x;
  }

  // The sized whole numbers: `u8(n)` checks, `u8.wrap(n)` keeps the low bits.
  for (const [name, [lo, hi, bits, signed]] of Object.entries(WIDTHS) as any) {
    const article = signed ? "an" : "a";
    BUILTINS[name] = (args: any, line: any) => {
      arity(name, args, 1, 1, line);
      const n = args[0];
      if (!isInt(n)) wrongType(name, "a whole number", n, line);
      if (n < lo || n > hi) raise("OverflowError", n + " does not fit in " + article + " " + name + " (" + lo + " to " + hi + ")", line);
      return n;
    };
    BUILTINS[name + ".wrap"] = (args: any, line: any) => {
      arity(name + ".wrap", args, 1, 1, line);
      const n = args[0];
      if (!isInt(n)) wrongType(name + ".wrap", "a whole number", n, line);
      return int(signed ? BigInt.asIntN(bits, big(n)) : BigInt.asUintN(bits, big(n)));
    };
  }
  const ERROR_KINDS = ["Error", "ValueError", "IndexError", "KeyError", "ZeroDivisionError", "OverflowError", "FileError"];
  for (const kind of ERROR_KINDS) BUILTINS[kind] = (args: any, line: any) => newError(kind, args, line);

  function callBuiltin(name: any, args: any, line: any) {
    const fn = BUILTINS[name];
    if (!fn) fatal("there is no `" + name + "` in a program built for JavaScript", line);
    return fn(args, line);
  }

  // Something that comes with Zeno, as a value.
  const builtinValues = new Map();
  function builtin(name: any, line?: any) {
    if (name === "math.pi") return float(Math.PI);
    if (name === "math.e") return float(Math.E);
    if (ERROR_KINDS.includes(name)) return new ErrorType(name);
    let fn = builtinValues.get(name);
    if (fn) return fn;
    const body = BUILTINS[name];
    if (!body) {
      const dot = name.indexOf(".");
      if (dot > 0) fatal("module `" + name.slice(0, dot) + "` has no `" + name.slice(dot + 1) + "`", line);
      fatal("`" + name + "` is not defined", line);
    }
    fn = (args: any, line: any) => body(args, line);
    fn.zname = name.slice(name.indexOf(".") + 1);
    fn.zbuiltin = name;
    builtinValues.set(name, fn);
    return fn;
  }
  const module = (name: any) => new Module(name);

  // ------------------------------------------------------- The program

  // Run the program, and say what stopped it if something did.
  function start(files: any, main: any) {
    const where = (line: any) => {
      const file = files[Math.floor(line / LINES_A_FILE)] || files[0];
      return " --> " + file + ":" + (line % LINES_A_FILE);
    };
    try {
      main();
      flush();
    } catch (e: any) {
      flush();
      let message;
      if (e instanceof Exit) return;
      if (e instanceof Raise) {
        message = "error: uncaught " + shownName(e.value.kind) + ": " + show(e.value) + "\n" + where(e.line);
      } else if (e instanceof Fatal) {
        message = "error: " + e.message + "\n" + where(e.line);
      } else if (e instanceof RangeError) {
        message = "error: too many calls are nested. Is a function calling itself forever?";
      } else {
        throw e;
      }
      if (inNode) {
        process.stderr.write(message + "\n");
        process.exitCode = 1;
      } else {
        // In a page, under what it printed, and where a test can see it.
        writePage(message + "\n");
        host.zenoError = message;
      }
    }
  }

  return {
    // Values
    Float, Quantity, Dict, MISSING, Raise, Fatal,
    float, list, dict, unit, concat, format, push,
    fn: makeFunction, class: makeClass, enum: makeEnum, case: caseOf,
    // Operators
    add, sub, mul, div, floordiv, mod, pow, bitand, bitor, bitxor, shl, shr, neg, bitnot,
    eq: equal, ne: (a: any, b: any) => !equal(a, b), lt, le, gt, ge, in: isIn, not_in: notIn, truth,
    // Items, fields, loops
    index, setIndex, field, setField, items, pairs, isCase, caseField,
    // Calls
    direct, call, callNamed, callMethod, callMethodNamed, callBuiltin, builtin, module,
    // Errors
    raise: raiseValue, catches, caught, fatal, raised,
    noMatch: (v: any, line: any) => fatal("no `case` matches " + repr(v), line),
    undefined: (name: any, line: any) => fatal("`" + name + "` is not defined", line),
    // Showing
    show, repr,
    start, flush, setPage: (f: any) => { writePage = f; },
    // The built-in functions, by name, with `_` for the `.` of a module: Z.b.math_sqrt
    b: Object.fromEntries(Object.entries(BUILTINS).map(([name, fn]: any) => [name.replace(/\./g, "_"), fn])),
  };
})();
