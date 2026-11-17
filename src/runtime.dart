// The Zeno runtime for Dart.
//
// A program built with `zeno build --target dart` is one `.dart` file:
// this runtime, and the program after it. It holds what every Zeno value
// is, and what each operator and built-in does with it: what ops.rs and
// builtins.rs do for `zeno run`, step for step and message for message,
// so that the program prints what `zeno run` prints. runtime.java,
// runtime.js and runtime.py are the same for the other targets.
//
// How a Zeno value is held (every one is an Object?):
//
//   int        int, with every step checked for overflow
//   float      double
//   str        String; an index counts characters, not UTF-16 units
//   bool       bool
//   none       null
//   list       List<Object?>
//   dict       Map<Object?, Object?>, which keeps the order the keys were put in
//   quantity   ZQuantity: a number and its unit
//   function   ZFunc
//   object     ZInstance of a ZClass
//   enum case  ZCase of a ZEnum
//   error      ZError
//
// The program is for the Dart VM (`dart run`, `dart compile exe`), where
// an int has 64 bits. It is not for the web, where an int is a float.
//
// Every name here begins with `Z`, and no name of a Zeno program does.

import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;
import 'dart:typed_data';

// ---------------------------------------------------------------- Values

class ZQuantity {
  final double v;
  final String unit;
  ZQuantity(this.v, this.unit);
}

typedef ZBody = Object? Function(List<Object?> args);
typedef ZBuiltinBody = Object? Function(List<Object?> args, int line);

class ZFunc {
  final String name;
  final List<String> params;
  final List<bool> defaults;
  final ZBody? body;
  // A function that comes with Zeno: its full name, and its body.
  final String? builtin;
  final ZBuiltinBody? bbody;
  ZFunc(this.name, this.params, this.defaults, this.body, this.builtin, this.bbody);
}

class ZClass {
  final String name;
  final List<String> fields;
  final List<Object?> defaults;
  final Map<String, ZFunc> methods;
  ZClass(this.name, this.fields, this.defaults, this.methods);
}

class ZInstance {
  final ZClass cls;
  final List<Object?> fields;
  ZInstance(this.cls, this.fields);
}

class ZEnum {
  final String name;
  final List<String> cases;
  final List<List<String>> fields;
  ZEnum(this.name, this.cases, this.fields);
}

class ZCase {
  final ZEnum def;
  final int index;
  final List<Object?> fields;
  ZCase(this.def, this.index, this.fields);
}

class ZConstructor {
  final ZEnum def;
  final int index;
  ZConstructor(this.def, this.index);
}

class ZErrorType {
  final String kind;
  ZErrorType(this.kind);
}

class ZError {
  final String kind, message;
  final Object? object;
  ZError(this.kind, this.message, this.object);
}

class ZModule {
  final String name;
  ZModule(this.name);
}

// A method of a str, a list or a dict, read but not yet called.
class ZMethod {
  final Object? receiver;
  final String name;
  ZMethod(this.receiver, this.name);
}

// A method of a class, read from an object.
class ZBound {
  final Object? object;
  final ZFunc fn;
  ZBound(this.object, this.fn);
}

// What a call gives a parameter it leaves out.
class ZMissing {
  const ZMissing();
}

// An error the program may catch, on its way to a `try`.
class ZRaise {
  final ZError value;
  final int line;
  ZRaise(this.value, this.line);
}

// A mistake in the program, which nothing catches.
class ZFatal {
  final String text;
  final int line;
  ZFatal(this.text, this.line);
}

class ZNotJson {
  const ZNotJson();
}

class ZUnitInfo {
  final String kind;
  final double size;
  const ZUnitInfo(this.kind, this.size);
}

class ZWidth {
  final int lo, hi, bits;
  final bool signed;
  const ZWidth(this.lo, this.hi, this.bits, this.signed);
}

// Reads JSON, a character at a time.
class ZJson {
  final List<int> cs;
  int at = 0;
  ZJson(String s) : cs = s.runes.toList();

  void spaces() {
    while (at < cs.length && (cs[at] == 0x20 || cs[at] == 0x09 || cs[at] == 0x0a || cs[at] == 0x0d)) at++;
  }

  bool word(String w) {
    final ws = w.codeUnits;
    if (at + ws.length > cs.length) return false;
    for (var i = 0; i < ws.length; i++) {
      if (cs[at + i] != ws[i]) return false;
    }
    at += ws.length;
    return true;
  }

  int fourHex() {
    if (at + 4 > cs.length) throw const ZNotJson();
    var n = 0;
    for (var i = 0; i < 4; i++) {
      final c = cs[at + i];
      int d;
      if (c >= 0x30 && c <= 0x39) {
        d = c - 0x30;
      } else if (c >= 0x61 && c <= 0x66) {
        d = c - 0x61 + 10;
      } else if (c >= 0x41 && c <= 0x46) {
        d = c - 0x41 + 10;
      } else {
        throw const ZNotJson();
      }
      n = n * 16 + d;
    }
    at += 4;
    return n;
  }

  String text() {
    if (at >= cs.length || cs[at] != 0x22) throw const ZNotJson();
    at++;
    final b = StringBuffer();
    while (true) {
      if (at >= cs.length) throw const ZNotJson();
      final c = cs[at++];
      if (c == 0x22) return b.toString();
      if (c != 0x5c) {
        b.writeCharCode(c);
        continue;
      }
      if (at >= cs.length) throw const ZNotJson();
      final e = cs[at++];
      if (e == 0x6e) {
        b.write("\n");
      } else if (e == 0x74) {
        b.write("\t");
      } else if (e == 0x72) {
        b.write("\r");
      } else if (e == 0x62) {
        b.write("\b");
      } else if (e == 0x66) {
        b.write("\f");
      } else if (e == 0x75) {
        var code = fourHex();
        if (code >= 0xD800 && code < 0xDC00 && word("\\u")) {
          final low = fourHex();
          if (low < 0xDC00 || low > 0xDFFF) throw const ZNotJson();
          code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
        }
        if ((code >= 0xD800 && code <= 0xDFFF) || code > 0x10FFFF) throw const ZNotJson();
        b.writeCharCode(code);
      } else {
        b.writeCharCode(e);
      }
    }
  }

  Object? value() {
    spaces();
    if (at >= cs.length) throw const ZNotJson();
    final c = cs[at];
    if (c == 0x6e) {
      if (word("null")) return null;
      throw const ZNotJson();
    }
    if (c == 0x74) {
      if (word("true")) return true;
      throw const ZNotJson();
    }
    if (c == 0x66) {
      if (word("false")) return false;
      throw const ZNotJson();
    }
    if (c == 0x22) return text();
    if (c == 0x5b) {
      at++;
      final out = <Object?>[];
      while (true) {
        spaces();
        if (word("]")) return out;
        if (out.isNotEmpty && !word(",")) throw const ZNotJson();
        out.add(value());
      }
    }
    if (c == 0x7b) {
      at++;
      final d = <Object?, Object?>{};
      var count = 0;
      while (true) {
        spaces();
        if (word("}")) return d;
        if (count > 0 && !word(",")) throw const ZNotJson();
        spaces();
        final name = text();
        spaces();
        if (!word(":")) throw const ZNotJson();
        final v = value();
        count++;
        if (v != null) d[name] = v;
      }
    }
    final start = at;
    while (at < cs.length && "0123456789+-.eE".codeUnits.contains(cs[at])) at++;
    final x = Z.parseFloat(String.fromCharCodes(cs, start, at));
    if (x == null) throw const ZNotJson();
    if (x.isFinite && x == x.floorToDouble() && x.abs() < 9.0e15) return x.toInt();
    return x;
  }
}

class Z {
  static const int LINES_A_FILE = 1000000;
  static const int MAX_INT = 9223372036854775807;
  static const int MIN_INT = -9223372036854775808;

  static const Object MISSING = ZMissing();

  static Never fatal(String message, int line) => throw ZFatal(message, line);

  static Never raiseError(String kind, String message, int line) => throw ZRaise(ZError(kind, message, null), line);

  static Never overflow(int line) => raiseError("OverflowError", "the result is too big for an int", line);

  static Never byZero(int line) => raiseError("ZeroDivisionError", "division by zero", line);

  static bool isNumber(Object? v) => v is int || v is double;

  static double number(Object? v) => v is int ? v.toDouble() : v as double;

  static bool isList(Object? v) => v is List<Object?>;

  static bool isDict(Object? v) => v is Map<Object?, Object?>;

  static String typeName(Object? v) {
    if (v is int) return "an int";
    if (v is double) return "a float";
    if (v is String) return "a str";
    if (v is bool) return "a bool";
    if (v == null) return "none";
    if (v is ZQuantity) return "a quantity in " + v.unit;
    if (v is List<Object?>) return "a list";
    if (v is Map<Object?, Object?>) return "a dict";
    if (v is ZFunc || v is ZMethod || v is ZBound) return "a function";
    if (v is ZClass) return "a class";
    if (v is ZInstance) return "a " + v.cls.name;
    if (v is ZEnum) return "an enum";
    if (v is ZConstructor) return "an enum case";
    if (v is ZCase) return "a " + v.def.name;
    if (v is ZErrorType) return "an error type";
    if (v is ZError) return "a " + v.kind;
    if (v is ZModule) return "a module";
    return "a value";
  }

  // `Point`, not `shapes.Point`; `Stack`, not `Stack[int]`.
  static String shownName(String name) {
    final bracket = name.indexOf("[");
    if (bracket >= 0) name = name.substring(0, bracket);
    final dot = name.lastIndexOf(".");
    return dot < 0 ? name : name.substring(dot + 1);
  }

  // ------------------------------------------------------------------ Text

  // A str's characters are code points; Dart's String counts UTF-16
  // units. Most text has no character outside the first 65536, and then
  // the two agree; the last str asked about is remembered.
  static String? lastText;
  static bool lastPlain = true;

  static bool plain(String s) {
    if (identical(s, lastText)) return lastPlain;
    var p = true;
    for (var i = 0; i < s.length; i++) {
      if ((s.codeUnitAt(i) & 0xF800) == 0xD800) {
        p = false;
        break;
      }
    }
    lastText = s;
    lastPlain = p;
    return p;
  }

  static int textLength(String s) => plain(s) ? s.length : s.runes.length;

  // Where the character numbered `count` begins, counting from the unit `from`.
  static int offset(String s, int from, int count) {
    var at = from;
    for (var i = 0; i < count; i++) {
      at += (s.codeUnitAt(at) & 0xFC00) == 0xD800 ? 2 : 1;
    }
    return at;
  }

  static String part(String s, int from, int to) {
    if (plain(s)) return s.substring(from, to);
    final a = offset(s, 0, from);
    return s.substring(a, offset(s, a, to - from));
  }

  // Bytes of UTF-8 as a String, or null if they are not UTF-8. Dart's
  // own reader drops a byte order mark at the front, and `zeno run`
  // keeps it.
  static String? decoded(List<int> bytes) {
    var marks = 0;
    while (bytes.length >= 3 * marks + 3 && bytes[3 * marks] == 0xEF && bytes[3 * marks + 1] == 0xBB && bytes[3 * marks + 2] == 0xBF) {
      marks++;
    }
    try {
      final rest = marks == 0 ? bytes : bytes.sublist(3 * marks);
      return "﻿" * marks + const Utf8Decoder().convert(rest);
    } on FormatException {
      return null;
    }
  }

  // --------------------------------------------------------------- Showing

  // A float as Rust's `{:?}` shows it: the shortest digits that read
  // back as the same number, `.0` on a whole one, and an exponent for one
  // below 1e-4 or from 1e16 up.
  static String showFloat(double x) {
    if (x.isNaN) return "NaN";
    if (x == double.infinity) return "inf";
    if (x == double.negativeInfinity) return "-inf";
    if (x == 0) return x.isNegative ? "-0.0" : "0.0";
    final parts = x.toStringAsExponential().split("e");
    final exp = int.parse(parts[1]);
    final sign = parts[0].startsWith("-") ? "-" : "";
    final digits = parts[0].replaceAll("-", "").replaceAll(".", "");
    if (exp < -4 || exp >= 16) {
      return sign + (digits.length > 1 ? digits[0] + "." + digits.substring(1) : digits) + "e" + exp.toString();
    }
    if (exp < 0) return sign + "0." + "0" * (-exp - 1) + digits;
    if (digits.length <= exp + 1) return sign + digits + "0" * (exp + 1 - digits.length) + ".0";
    return sign + digits.substring(0, exp + 1) + "." + digits.substring(exp + 1);
  }

  static int bitsOf(double x) => (ByteData(8)..setFloat64(0, x)).getInt64(0);

  static double floatOf(int bits) => (ByteData(8)..setInt64(0, bits)).getFloat64(0);

  // A float with a fixed number of decimals, as Rust's `{:.2}` writes it:
  // from its exact value, a half rounding to the even digit.
  static String fixed(double x, int places) {
    if (x.isNaN) return "NaN";
    if (x.isInfinite) return x > 0 ? "inf" : "-inf";
    // The float is exactly m * 2**e.
    final bits = bitsOf(x.abs());
    var e = (bits >> 52) & 0x7ff;
    var m = bits & 0xFFFFFFFFFFFFF;
    if (e == 0) {
      e = 1;
    } else {
      m |= 1 << 52;
    }
    e -= 1075;
    final scaled = BigInt.from(m) * BigInt.from(10).pow(places);
    BigInt q;
    if (e >= 0) {
      q = scaled << e;
    } else {
      final den = BigInt.one << (-e);
      q = scaled ~/ den;
      final twice = (scaled - q * den) << 1;
      if (twice > den || (twice == den && q.isOdd)) q += BigInt.one;
    }
    var digits = q.toString();
    if (places > 0) {
      digits = digits.padLeft(places + 1, "0");
      digits = digits.substring(0, digits.length - places) + "." + digits.substring(digits.length - places);
    }
    return (x.isNegative ? "-" : "") + digits;
  }

  // How Rust's `{:?}` shows a str: in quotes, with what cannot be seen escaped.
  static String quoted(String s) {
    final b = StringBuffer("\"");
    for (final c in s.runes) {
      if (c == 0x22) {
        b.write("\\\"");
      } else if (c == 0x5c) {
        b.write("\\\\");
      } else if (c == 0x0a) {
        b.write("\\n");
      } else if (c == 0x0d) {
        b.write("\\r");
      } else if (c == 0x09) {
        b.write("\\t");
      } else if (c == 0) {
        b.write("\\0");
      } else if (c < 0x20 || (c >= 0x7f && c < 0xa0)) {
        b.write("\\u{" + c.toRadixString(16) + "}");
      } else {
        b.writeCharCode(c);
      }
    }
    b.write("\"");
    return b.toString();
  }

  // How `print`, `str()` and f-strings show a value.
  static String show(Object? v) {
    if (v is String) return v;
    if (v is int) return v.toString();
    if (v is double) return showFloat(v);
    if (v is bool) return v ? "true" : "false";
    if (v == null) return "none";
    if (v is List<Object?>) {
      final b = StringBuffer("[");
      for (var i = 0; i < v.length; i++) {
        if (i > 0) b.write(", ");
        b.write(repr(v[i]));
      }
      b.write("]");
      return b.toString();
    }
    if (v is Map<Object?, Object?>) {
      if (v.isEmpty) return "[:]";
      final b = StringBuffer("[");
      var first = true;
      v.forEach((key, value) {
        if (!first) b.write(", ");
        first = false;
        b.write(repr(key));
        b.write(": ");
        b.write(repr(value));
      });
      b.write("]");
      return b.toString();
    }
    if (v is ZQuantity) {
      if (v.v.isFinite && v.v == v.v.floorToDouble() && v.v.abs() < 1e15) {
        return v.v.toInt().toString() + " " + v.unit;
      }
      return showFloat(v.v) + " " + v.unit;
    }
    if (v is ZInstance) {
      final b = StringBuffer(shownName(v.cls.name));
      b.write("(");
      for (var i = 0; i < v.cls.fields.length; i++) {
        if (i > 0) b.write(", ");
        b.write(v.cls.fields[i]);
        b.write("=");
        b.write(repr(v.fields[i]));
      }
      b.write(")");
      return b.toString();
    }
    if (v is ZCase) {
      final name = shownName(v.def.cases[v.index]);
      if (v.fields.isEmpty) return name;
      final b = StringBuffer(name);
      b.write("(");
      for (var i = 0; i < v.fields.length; i++) {
        if (i > 0) b.write(", ");
        b.write(repr(v.fields[i]));
      }
      b.write(")");
      return b.toString();
    }
    if (v is ZError) return v.message;
    if (v is ZMethod) return "<function " + v.name + ">";
    if (v is ZBound) return "<function " + v.fn.name + ">";
    if (v is ZClass) return "<class " + v.name + ">";
    if (v is ZEnum) return "<enum " + v.name + ">";
    if (v is ZConstructor) return "<case " + v.def.cases[v.index] + ">";
    if (v is ZErrorType) return "<error " + v.kind + ">";
    if (v is ZModule) return "<module " + v.name + ">";
    if (v is ZFunc) return v.name.isEmpty ? "<function>" : "<function " + v.name + ">";
    return "?";
  }

  static String repr(Object? v) => v is String ? quoted(v) : show(v);

  // ----------------------------------------------------------------- Units

  static final Map<String, ZUnitInfo> UNITS = _units();

  static Map<String, ZUnitInfo> _units() {
    final out = <String, ZUnitInfo>{};
    void units(String kind, List<Object> pairs) {
      for (var i = 0; i < pairs.length; i += 2) {
        out[pairs[i] as String] = ZUnitInfo(kind, (pairs[i + 1] as num).toDouble());
      }
    }

    units("time", ["ns", 1, "us", 1e3, "ms", 1e6, "s", 1e9, "min", 60e9, "h", 3600e9]);
    units("frequency", ["Hz", 1, "kHz", 1e3, "MHz", 1e6]);
    units("voltage", ["uV", 1, "mV", 1e3, "V", 1e6, "kV", 1e9]);
    units("current", ["uA", 1, "mA", 1e3, "A", 1e6]);
    units("resistance", ["ohm", 1, "kohm", 1e3, "Mohm", 1e6]);
    units("power", ["mW", 1, "W", 1e3, "kW", 1e6]);
    units("energy", ["J", 1, "Wh", 3600, "kWh", 3600e3]);
    units("temperature in Celsius", ["degC", 1]);
    units("temperature in kelvin", ["K", 1]);
    units("length", ["mm", 1, "cm", 10, "m", 1e3, "km", 1e6]);
    units("mass", ["mg", 1, "g", 1e3, "kg", 1e6]);
    units("angle in degrees", ["deg", 1]);
    units("angle in radians", ["rad", 1]);
    units("rotation", ["rpm", 1]);
    return out;
  }

  // How to turn a number in one unit into another: [times, over], or null.
  static List<double>? convert(String from, String to) {
    if (from == "deg" && to == "rad") return [math.pi / 180, 1];
    if (from == "rad" && to == "deg") return [180 / math.pi, 1];
    final f = UNITS[from], t = UNITS[to];
    if (f == null || t == null || f.kind != t.kind) return null;
    return f.size >= t.size ? [f.size / t.size, 1] : [1, t.size / f.size];
  }

  static double apply(List<double> scale, double x) => x * scale[0] / scale[1];

  // The unit two quantities meet in, the finer one, or null.
  static String? common(String a, String b) {
    final ua = UNITS[a], ub = UNITS[b];
    if (ua == null || ub == null || ua.kind != ub.kind) return null;
    return ua.size <= ub.size ? a : b;
  }

  static Object? unit(Object? v, String name, int line) {
    if (!UNITS.containsKey(name)) {
      fatal("`" + name + "` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC", line);
    }
    if (!isNumber(v)) fatal("a unit can only follow a number", line);
    return ZQuantity(number(v), name);
  }

  // -------------------------------------------------------------- Equality

  static bool equal(Object? a, Object? b) {
    if (a is int && b is int) return a == b;
    if (isNumber(a) && isNumber(b)) return number(a) == number(b);
    if (a is String && b is String) return a == b;
    if (a is bool && b is bool) return a == b;
    if (a == null || b == null) return a == null && b == null;
    if (a is ZQuantity && b is ZQuantity) {
      final u = common(a.unit, b.unit);
      return u != null && apply(convert(a.unit, u)!, a.v) == apply(convert(b.unit, u)!, b.v);
    }
    if (a is List<Object?> && b is List<Object?>) {
      if (a.length != b.length) return false;
      for (var i = 0; i < a.length; i++) {
        if (!equal(a[i], b[i])) return false;
      }
      return true;
    }
    if (a is Map<Object?, Object?> && b is Map<Object?, Object?>) {
      if (a.length != b.length) return false;
      for (final entry in a.entries) {
        if (!b.containsKey(entry.key) || !equal(entry.value, b[entry.key])) return false;
      }
      return true;
    }
    if (a is ZInstance && b is ZInstance) {
      if (!identical(a.cls, b.cls)) return false;
      for (var i = 0; i < a.fields.length; i++) {
        if (!equal(a.fields[i], b.fields[i])) return false;
      }
      return true;
    }
    if (a is ZCase && b is ZCase) {
      if (!identical(a.def, b.def) || a.index != b.index || a.fields.length != b.fields.length) return false;
      for (var i = 0; i < a.fields.length; i++) {
        if (!equal(a.fields[i], b.fields[i])) return false;
      }
      return true;
    }
    if (a is ZError && b is ZError) return a.kind == b.kind && a.message == b.message;
    if (a is ZConstructor && b is ZConstructor) return identical(a.def, b.def) && a.index == b.index;
    return identical(a, b);
  }

  static bool eq(Object? a, Object? b) => equal(a, b);

  static bool ne(Object? a, Object? b) => !equal(a, b);

  // ------------------------------------------------------------- Operators

  static Never mismatch(String op, Object? a, Object? b, int line) {
    fatal("`" + op + "` does not work between " + typeName(a) + " and " + typeName(b), line);
  }

  static int addInt(int a, int b, int line) {
    final r = a + b;
    if (((a ^ r) & (b ^ r)) < 0) overflow(line);
    return r;
  }

  static int mulInt(int a, int b, int line) {
    if (a == 0 || b == 0) return 0;
    final r = a * b;
    if ((a == -1 && b == MIN_INT) || (b == -1 && a == MIN_INT) || r ~/ b != a) overflow(line);
    return r;
  }

  static Object? add(Object? a, Object? b, int line) {
    if (a is int && b is int) return addInt(a, b, line);
    if (a is String && b is String) return a + b;
    if (a is List<Object?> && b is List<Object?>) return <Object?>[...a, ...b];
    if (a is ZQuantity && b is ZQuantity) {
      final u = common(a.unit, b.unit);
      if (u == null) mismatch("+", a, b, line);
      return ZQuantity(apply(convert(a.unit, u)!, a.v) + apply(convert(b.unit, u)!, b.v), u);
    }
    if (!isNumber(a) || !isNumber(b)) mismatch("+", a, b, line);
    return number(a) + number(b);
  }

  static Object? sub(Object? a, Object? b, int line) {
    if (a is int && b is int) {
      final r = a - b;
      if (((a ^ b) & (a ^ r)) < 0) overflow(line);
      return r;
    }
    if (a is ZQuantity && b is ZQuantity) {
      final u = common(a.unit, b.unit);
      if (u == null) mismatch("-", a, b, line);
      return ZQuantity(apply(convert(a.unit, u)!, a.v) - apply(convert(b.unit, u)!, b.v), u);
    }
    if (!isNumber(a) || !isNumber(b)) mismatch("-", a, b, line);
    return number(a) - number(b);
  }

  static Object? mul(Object? a, Object? b, int line) {
    if (a is int && b is int) return mulInt(a, b, line);
    if (a is ZQuantity || b is ZQuantity) {
      final q = (a is ZQuantity ? a : b) as ZQuantity;
      final other = a is ZQuantity ? b : a;
      if (!isNumber(other)) mismatch("*", a, b, line);
      return ZQuantity(q.v * number(other), q.unit);
    }
    if (!isNumber(a) || !isNumber(b)) mismatch("*", a, b, line);
    return number(a) * number(b);
  }

  static Object? div(Object? a, Object? b, int line) {
    if (a is ZQuantity) {
      if (b is ZQuantity && b.v == 0) byZero(line);
      if (b is double && b == 0) byZero(line);
      if (b is int && b == 0) byZero(line);
      if (b is ZQuantity) {
        final u = common(a.unit, b.unit);
        if (u == null) mismatch("/", a, b, line);
        return apply(convert(a.unit, u)!, a.v) / apply(convert(b.unit, u)!, b.v);
      }
      if (!isNumber(b)) mismatch("/", a, b, line);
      return ZQuantity(a.v / number(b), a.unit);
    }
    if (!isNumber(a) || !isNumber(b)) mismatch("/", a, b, line);
    if (number(b) == 0) byZero(line);
    return number(a) / number(b);
  }

  static Object? floordiv(Object? a, Object? b, int line) {
    if (a is int && b is int) {
      if (b == 0) byZero(line);
      if (a == MIN_INT && b == -1) overflow(line);
      var q = a ~/ b;
      if (a.remainder(b) != 0 && ((a < 0) != (b < 0))) q--;
      return q;
    }
    if (!isNumber(a) || !isNumber(b)) mismatch("//", a, b, line);
    if (number(b) == 0) byZero(line);
    return (number(a) / number(b)).floorToDouble();
  }

  static Object? mod(Object? a, Object? b, int line) {
    if (a is int && b is int) {
      if (b == 0) byZero(line);
      if (b == -1) return 0;
      var r = a.remainder(b);
      if (r != 0 && ((r < 0) != (b < 0))) r += b;
      return r;
    }
    if (!isNumber(a) || !isNumber(b)) mismatch("%", a, b, line);
    final x = number(a), y = number(b);
    if (y == 0) byZero(line);
    final q = x / y;
    if (q.isNaN || q.isInfinite) return x - y * q;
    return x - y * q.floorToDouble();
  }

  static Object? pow(Object? a, Object? b, int line) {
    if (a is int && b is int) {
      if (b < 0) raiseError("ValueError", "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`", line);
      if (b > 4294967295) overflow(line);
      if (a == 0 || a == 1) return b == 0 ? 1 : a;
      if (a == -1) return b % 2 == 0 ? 1 : -1;
      if (b >= 64) overflow(line);
      var r = 1;
      for (var i = 0; i < b; i++) {
        r = mulInt(r, a, line);
      }
      return r;
    }
    if (!isNumber(a) || !isNumber(b)) mismatch("**", a, b, line);
    final x = number(a), y = number(b);
    if (x == 1 || y == 0) return 1.0;
    return math.pow(x, y).toDouble();
  }

  static Never bitsMismatch(String op, Object? a, Object? b, int line) {
    var hint = "";
    if (a is bool && b is bool) {
      if (op == "&") hint = ". For two bools Zeno spells this `and`";
      if (op == "|") hint = ". For two bools Zeno spells this `or`";
    }
    fatal("`" + op + "` works on the bits of whole numbers, and these are " + typeName(a) + " and " + typeName(b) + hint, line);
  }

  static int whole(String op, Object? v, Object? a, Object? b, int line) {
    if (v is! int) bitsMismatch(op, a, b, line);
    return v;
  }

  static Object? bitand(Object? a, Object? b, int line) => whole("&", a, a, b, line) & whole("&", b, a, b, line);

  static Object? bitor(Object? a, Object? b, int line) => whole("|", a, a, b, line) | whole("|", b, a, b, line);

  static Object? bitxor(Object? a, Object? b, int line) => whole("^", a, a, b, line) ^ whole("^", b, a, b, line);

  static void shift(int n, int line) {
    if (n < 0 || n >= 64) raiseError("ValueError", "cannot shift by " + n.toString() + ". An int has 64 bits, so a shift is 0 to 63", line);
  }

  static Object? shl(Object? a, Object? b, int line) {
    final x = whole("<<", a, a, b, line), n = whole("<<", b, a, b, line);
    shift(n, line);
    final s = x << n;
    if (s >> n != x) overflow(line);
    return s;
  }

  static Object? shr(Object? a, Object? b, int line) {
    final x = whole(">>", a, a, b, line), n = whole(">>", b, a, b, line);
    shift(n, line);
    return x >> n;
  }

  static Object? neg(Object? v, int line) {
    if (v is int) {
      if (v == MIN_INT) overflow(line);
      return -v;
    }
    if (v is double) return -v;
    if (v is ZQuantity) return ZQuantity(-v.v, v.unit);
    fatal("`-` does not work on " + typeName(v), line);
  }

  static Object? bitnot(Object? v, int line) {
    if (v is! int) fatal("`~` works on the bits of a whole number, and this is " + typeName(v), line);
    return ~v;
  }

  // -1, 0 or 1, or 2 when the two cannot be ordered (a NaN).
  static int order(String op, Object? a, Object? b, int line) {
    if (a is int && b is int) return a < b ? -1 : (a > b ? 1 : 0);
    if (a is String && b is String) {
      // By code points, as Rust compares, not by UTF-16 units.
      final x = a.runes.iterator, y = b.runes.iterator;
      while (true) {
        final more = x.moveNext(), other = y.moveNext();
        if (!more || !other) return more ? 1 : (other ? -1 : 0);
        if (x.current != y.current) return x.current < y.current ? -1 : 1;
      }
    }
    double x, y;
    if (a is ZQuantity && b is ZQuantity) {
      final u = common(a.unit, b.unit);
      if (u == null) mismatch(op, a, b, line);
      x = apply(convert(a.unit, u)!, a.v);
      y = apply(convert(b.unit, u)!, b.v);
    } else {
      if (!isNumber(a) || !isNumber(b)) mismatch(op, a, b, line);
      x = number(a);
      y = number(b);
    }
    if (x < y) return -1;
    if (x > y) return 1;
    if (x == y) return 0;
    return 2;
  }

  static bool lt(Object? a, Object? b, int line) => order("<", a, b, line) == -1;

  static bool le(Object? a, Object? b, int line) {
    final o = order("<=", a, b, line);
    return o == -1 || o == 0;
  }

  static bool gt(Object? a, Object? b, int line) => order(">", a, b, line) == 1;

  static bool ge(Object? a, Object? b, int line) {
    final o = order(">=", a, b, line);
    return o == 1 || o == 0;
  }

  static bool contains(String op, Object? item, Object? container, int line) {
    if (container is List<Object?>) {
      for (final other in container) {
        if (equal(item, other)) return true;
      }
      return false;
    }
    if (container is Map<Object?, Object?>) {
      if (item is! int && item is! String) mismatch(op, item, container, line);
      return container.containsKey(item);
    }
    if (container is String && item is String) return container.contains(item);
    mismatch(op, item, container, line);
  }

  static bool is_in(Object? a, Object? b, int line) => contains("in", a, b, line);

  static bool not_in(Object? a, Object? b, int line) => !contains("not in", a, b, line);

  static bool truth(Object? v, int line) {
    if (v is bool) return v;
    fatal("a condition must be true or false, but this is " + typeName(v), line);
  }

  // -------------------------------------------------------- Lists and text

  static Object key(Object? v, int line) {
    if (v is int || v is String) return v!;
    fatal("a key of a dict is a str or an int, but this is " + typeName(v), line);
  }

  static int position(Object? index, int length, String what, int line) {
    if (index is! int) fatal("an index must be an int, but this is " + typeName(index), line);
    if (index < 0 || index >= length) {
      raiseError("IndexError", "index " + index.toString() + " is out of range for a " + what + " of length " + length.toString(), line);
    }
    return index;
  }

  static Object? index(Object? obj, Object? i, int line) {
    if (obj is List<Object?>) return obj[position(i, obj.length, "list", line)];
    if (obj is String) {
      final at = position(i, textLength(obj), "str", line);
      return part(obj, at, at + 1);
    }
    if (obj is Map<Object?, Object?>) {
      final k = key(i, line);
      final v = obj[k];
      if (v == null && !obj.containsKey(k)) raiseError("KeyError", "this dict has no key " + repr(i), line);
      return v;
    }
    fatal("cannot take an item from " + typeName(obj), line);
  }

  static void set_index(Object? obj, Object? i, Object? value, int line) {
    if (obj is List<Object?>) {
      obj[position(i, obj.length, "list", line)] = value;
      return;
    }
    if (obj is Map<Object?, Object?>) {
      obj[key(i, line)] = value;
      return;
    }
    fatal("cannot set an item of " + typeName(obj), line);
  }

  static List<Object?> chars(String s) {
    final out = <Object?>[];
    for (final c in s.runes) {
      out.add(String.fromCharCode(c));
    }
    return out;
  }

  static List<Object?> items(Object? v, int line) {
    if (v is List<Object?>) return List<Object?>.of(v);
    if (v is Map<Object?, Object?>) return List<Object?>.of(v.keys);
    if (v is String) return chars(v);
    fatal("cannot loop over " + typeName(v), line);
  }

  static List<List<Object?>> pairs(Object? v, int line) {
    final out = <List<Object?>>[];
    if (v is Map<Object?, Object?>) {
      v.forEach((key, value) {
        out.add(<Object?>[key, value]);
      });
      return out;
    }
    final all = items(v, line);
    for (var i = 0; i < all.length; i++) {
      out.add(<Object?>[i, all[i]]);
    }
    return out;
  }

  static Object? list(List<Object?> values) => values;

  static Object? dict(List<Object?> flat, int line) {
    final d = <Object?, Object?>{};
    for (var i = 0; i < flat.length; i += 2) {
      d[key(flat[i], line)] = flat[i + 1];
    }
    return d;
  }

  static Object? push(Object? list, Object? value) {
    (list as List<Object?>).add(value);
    return null;
  }

  static Object? concat(List<Object?> pieces) => pieces.join();

  // --------------------------------------------------- Objects and fields

  static ZFunc fn(String name, List<String> params, List<bool> defaults, ZBody body) {
    return ZFunc(name, params, defaults, body, null, null);
  }

  static Object? cls(String name, List<String> fields, List<Object?> defaults, Map<String, ZFunc> methods) {
    return ZClass(name, fields, defaults, methods);
  }

  static Object? enumOf(String name, List<String> cases, List<List<String>> fields) => ZEnum(name, cases, fields);

  // A case with no values is a value; one with values needs calling.
  static Object? case_of(Object? def, int i) {
    final e = def as ZEnum;
    return e.fields[i].isEmpty ? ZCase(e, i, <Object?>[]) : ZConstructor(e, i);
  }

  static bool is_case(Object? value, Object? ctor, int count, int line) {
    if (ctor is! ZConstructor) fatal("this is not an enum case that carries values", line);
    if (value is! ZCase || !identical(value.def, ctor.def) || value.index != ctor.index) return false;
    if (value.fields.length != count) {
      fatal("`" + ctor.def.cases[ctor.index] + "` carries " + value.fields.length.toString() + " values, but this pattern has " + count.toString(), line);
    }
    return true;
  }

  static Object? case_field(Object? value, int i) => (value as ZCase).fields[i];

  static Object? field(Object? obj, String name, int line) {
    if (obj is ZInstance) {
      final at = obj.cls.fields.indexOf(name);
      if (at >= 0) return obj.fields[at];
      final m = obj.cls.methods[name];
      if (m != null) return ZBound(obj, m);
      fatal("`" + obj.cls.name + "` has no field or method `" + name + "`", line);
    }
    if (obj is ZCase) {
      final at = obj.def.fields[obj.index].indexOf(name);
      if (at >= 0) return obj.fields[at];
      fatal("`" + obj.def.cases[obj.index] + "` has no field `" + name + "`", line);
    }
    if (obj is ZEnum) {
      final at = obj.cases.indexOf(name);
      if (at >= 0) return case_of(obj, at);
      fatal("enum `" + obj.name + "` has no case `" + name + "`", line);
    }
    if (obj is ZModule) return builtin(obj.name + "." + name, line);
    if (obj is ZQuantity && UNITS.containsKey(name)) {
      final s = convert(obj.unit, name);
      if (s == null) fatal("a quantity in " + obj.unit + " cannot be turned into " + name + ". They measure different things", line);
      return ZQuantity(apply(s, obj.v), name);
    }
    if (obj is ZError && name == "message") return obj.message;
    if (obj is ZError && name == "kind") return obj.kind;
    if (obj is ZFunc && obj.builtin != null && WIDTHS.containsKey(obj.builtin) && name == "wrap") {
      return builtin(obj.builtin! + ".wrap", line);
    }
    if (hasMethod(obj, name)) return ZMethod(obj, name);
    fatal(typeName(obj) + " has no field or method `" + name + "`", line);
  }

  static void set_field(Object? obj, String name, Object? value, int line) {
    if (obj is ZInstance) {
      final at = obj.cls.fields.indexOf(name);
      if (at < 0) fatal("`" + obj.cls.name + "` has no field `" + name + "`", line);
      obj.fields[at] = value;
      return;
    }
    fatal("cannot set a field on " + typeName(obj), line);
  }

  // ----------------------------------------------------------------- Calls

  static String counted(int n, String word) => n == 1 ? "1 " + word : n.toString() + " " + word + "s";

  static String were(int n) => n == 1 ? "1 was" : n.toString() + " were";

  static List<Object?> bind(String what, List<String> names, List<Object?> positional, List<String> namedNames, List<Object?> named, int line) {
    if (positional.length > names.length) {
      fatal("`" + what + "` takes " + counted(names.length, "argument") + ", but " + were(positional.length) + " given", line);
    }
    final slots = List<Object?>.filled(names.length, MISSING);
    for (var i = 0; i < positional.length; i++) {
      slots[i] = positional[i];
    }
    for (var i = 0; i < namedNames.length; i++) {
      final at = names.indexOf(namedNames[i]);
      if (at < 0) fatal("`" + what + "` has no argument named `" + namedNames[i] + "`", line);
      if (!identical(slots[at], MISSING)) fatal("`" + what + "` was given `" + namedNames[i] + "` twice", line);
      slots[at] = named[i];
    }
    return slots;
  }

  // How many calls are nested now. `zeno run` stops at 1000.
  static int depth = 0;

  static void enter(ZFunc f, int line) {
    if (depth >= 1000) {
      final name = f.name.isEmpty ? "function" : f.name;
      fatal("more than 1000 calls are nested. Is `" + name + "` calling itself forever?", line);
    }
    depth++;
  }

  // A call of a function of the program known where it is written, with
  // its arguments lined up already.
  static Object? direct(Object? f, int line, List<Object?> args) {
    final fn = f as ZFunc;
    enter(fn, line);
    try {
      return fn.body!(args);
    } finally {
      depth--;
    }
  }

  static const List<String> NO_NAMES = <String>[];
  static const List<Object?> NO_VALUES = <Object?>[];

  static Object? callFunction(ZFunc f, List<Object?> positional, List<String> namedNames, List<Object?> named, int line) {
    final name = f.name.isEmpty ? "function" : f.name;
    enter(f, line);
    try {
      final slots = bind(name, f.params, positional, namedNames, named, line);
      for (var i = 0; i < slots.length; i++) {
        if (identical(slots[i], MISSING) && !f.defaults[i]) fatal("`" + name + "` needs a value for `" + f.params[i] + "`", line);
      }
      return f.body!(slots);
    } finally {
      depth--;
    }
  }

  static void noNamed(String what, List<String> namedNames, int line) {
    if (namedNames.isNotEmpty) fatal("`" + what + "` does not take a named argument `" + namedNames[0] + "`", line);
  }

  static Object? call_named(Object? callee, List<Object?> positional, List<String> namedNames, List<Object?> named, int line) {
    if (callee is ZBound) return callFunction(callee.fn, <Object?>[callee.object, ...positional], namedNames, named, line);
    if (callee is ZMethod) {
      noNamed(callee.name, namedNames, line);
      return callMethodOf(callee.receiver, callee.name, positional, line);
    }
    if (callee is ZClass) {
      final slots = bind(callee.name, callee.fields, positional, namedNames, named, line);
      for (var i = 0; i < slots.length; i++) {
        if (identical(slots[i], MISSING)) {
          final byDefault = callee.defaults[i];
          if (byDefault == null) fatal("`" + callee.name + "` needs a value for `" + callee.fields[i] + "`", line);
          slots[i] = (byDefault as ZFunc).body!(NO_VALUES);
        }
      }
      return ZInstance(callee, slots);
    }
    if (callee is ZConstructor) {
      final name = callee.def.cases[callee.index];
      final fields = callee.def.fields[callee.index];
      final slots = bind(name, fields, positional, namedNames, named, line);
      for (var i = 0; i < slots.length; i++) {
        if (identical(slots[i], MISSING)) fatal("`" + name + "` needs a value for `" + fields[i] + "`", line);
      }
      return ZCase(callee.def, callee.index, slots);
    }
    if (callee is ZErrorType) {
      noNamed(callee.kind, namedNames, line);
      return newError(callee.kind, positional, line);
    }
    if (callee is ZFunc) {
      final bbody = callee.bbody;
      if (bbody != null) {
        noNamed(callee.name, namedNames, line);
        return bbody(positional, line);
      }
      return callFunction(callee, positional, namedNames, named, line);
    }
    fatal(typeName(callee) + " cannot be called", line);
  }

  static Object? call(Object? callee, List<Object?> args, int line) => call_named(callee, args, NO_NAMES, NO_VALUES, line);

  static Object? newError(String kind, List<Object?> args, int line) {
    if (args.length > 1) fatal("`" + kind + "` takes one message", line);
    return ZError(kind, args.length == 1 ? show(args[0]) : "", null);
  }

  static Object? call_method_named(Object? obj, String name, List<Object?> positional, List<String> namedNames, List<Object?> named, int line) {
    if (obj is ZInstance && obj.cls.fields.indexOf(name) < 0) {
      final m = obj.cls.methods[name];
      if (m != null) return callFunction(m, <Object?>[obj, ...positional], namedNames, named, line);
    }
    if (obj is ZModule) {
      noNamed(name, namedNames, line);
      return call_builtin(obj.name + "." + name, positional, line);
    }
    if (obj is String || obj is List<Object?> || obj is Map<Object?, Object?>) {
      noNamed(name, namedNames, line);
      return callMethodOf(obj, name, positional, line);
    }
    return call_named(field(obj, name, line), positional, namedNames, named, line);
  }

  static Object? call_method(Object? obj, String name, List<Object?> args, int line) {
    return call_method_named(obj, name, args, NO_NAMES, NO_VALUES, line);
  }

  // ---------------------------------------------------------------- Errors

  static void raise(Object? value, int line) {
    if (value is ZError) throw ZRaise(value, line);
    if (value is ZInstance) throw ZRaise(ZError(value.cls.name, show(value), value), line);
    fatal("only an error can be raised, and this is " + typeName(value), line);
  }

  static bool catches(ZRaise e, String name) => name == "Error" || name == e.value.kind;

  static Object? caught(ZRaise e, String name) {
    if (name.isNotEmpty && name != "Error" && e.value.object != null) return e.value.object;
    return e.value;
  }

  static void no_match(Object? v, int line) {
    fatal("no `case` matches " + repr(v), line);
  }

  static Object? undefined(String name, int line) {
    fatal("`" + name + "` is not defined", line);
  }

  // An expression whose value is not wanted.
  static void ignore(Object? v) {}

  // --------------------------------------------------- Built-in functions

  static void arity(String name, List<Object?> args, int lo, int hi, int line) {
    if (args.length >= lo && args.length <= hi) return;
    final wanted = lo == hi ? counted(lo, "argument") : lo.toString() + " to " + hi.toString() + " arguments";
    fatal("`" + name + "` takes " + wanted + ", but " + were(args.length) + " given", line);
  }

  static Never wrongType(String name, String wanted, Object? got, int line) {
    fatal("`" + name + "` needs " + wanted + ", but this is " + typeName(got), line);
  }

  static String text(String name, Object? v, int line) {
    if (v is! String) wrongType(name, "a str", v, line);
    return v;
  }

  static int wholeArg(String name, Object? v, int line) {
    if (v is! int) wrongType(name, "a whole number", v, line);
    return v;
  }

  static bool blank(int c) => c == 0x20 || c == 0x09 || c == 0x0a || c == 0x0d || c == 0x0c;

  static String strip(String s) {
    var a = 0, b = s.length;
    while (a < b && blank(s.codeUnitAt(a))) a++;
    while (b > a && blank(s.codeUnitAt(b - 1))) b--;
    return s.substring(a, b);
  }

  // What the program has printed and not yet handed on.
  static final StringBuffer OUT = StringBuffer();

  static void flush() {
    if (OUT.isEmpty) return;
    stdout.add(utf8.encode(OUT.toString()));
    OUT.clear();
  }

  static void write(String s) {
    OUT.write(s);
    if (OUT.length > 65536) flush();
  }

  static const Map<String, ZWidth> WIDTHS = {
    "u8": ZWidth(0, 255, 8, false),
    "u16": ZWidth(0, 65535, 16, false),
    "u32": ZWidth(0, 4294967295, 32, false),
    "i8": ZWidth(-128, 127, 8, true),
    "i16": ZWidth(-32768, 32767, 16, true),
    "i32": ZWidth(-2147483648, 2147483647, 32, true),
  };

  static double amount(Object? v) => v is ZQuantity ? v.v : number(v);

  // The numbers `min`, `max` and `sum` are given, and the unit they
  // are in, if they are quantities.
  static String? unitOfNumbers;

  static List<Object?> numbers(String name, List<Object?> args, int line) {
    final first = args.length == 1 ? args[0] : null;
    final List<Object?> values = first is List<Object?> ? first : args;
    String? unit;
    for (var place = 0; place < values.length; place++) {
      final item = values[place];
      if (item is ZQuantity) {
        if (unit != null && unit != item.unit) {
          fatal("`" + name + "` needs quantities in one unit, but these are in " + unit + " and in " + item.unit + ". Turn one into the other first, as in `x." + unit + "`", line);
        }
        if (unit == null && place > 0) fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line);
        unit = item.unit;
      } else if (!isNumber(item)) {
        wrongType(name, "numbers", item, line);
      } else if (unit != null) {
        fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line);
      }
    }
    unitOfNumbers = unit;
    return values;
  }

  static Object? pick(String name, List<Object?> args, int line, bool smaller) {
    final values = numbers(name, args, line);
    if (values.isEmpty) raiseError("ValueError", "`" + name + "` of nothing", line);
    var best = values[0];
    for (var i = 1; i < values.length; i++) {
      final item = values[i];
      if (smaller ? amount(item) < amount(best) : amount(item) > amount(best)) best = item;
    }
    return best;
  }

  static final RegExp FLOAT = RegExp(r"^[+-]?([0-9]+\.?[0-9]*([eE][+-]?[0-9]+)?|\.[0-9]+([eE][+-]?[0-9]+)?|[iI][nN][fF]|[iI][nN][fF][iI][nN][iI][tT][yY]|[nN][aA][nN])$");

  // A float as Rust's `parse::<f64>` reads it, or null.
  static double? parseFloat(String s) {
    if (!FLOAT.hasMatch(s)) return null;
    final negative = s.startsWith("-");
    var body = (s.startsWith("-") || s.startsWith("+") ? s.substring(1) : s).toLowerCase();
    if (body == "inf" || body == "infinity") return negative ? double.negativeInfinity : double.infinity;
    if (body == "nan") return double.nan;
    // Dart does not read `1.` or `1.e5`: a digit must follow the point.
    final point = body.indexOf(".");
    if (point >= 0 && (point + 1 == body.length || body[point + 1] == "e")) {
      body = body.substring(0, point) + ".0" + body.substring(point + 1);
    }
    final x = double.parse(body);
    return negative ? -x : x;
  }

  // Heapsort, step for step as the Rust code sorts, so that a function
  // given to `sort` is called in the same order.
  static void heapsort(int n, bool Function(int, int) before, void Function(int, int) swap) {
    for (var root = n ~/ 2 - 1; root >= 0; root--) {
      sift(root, n, before, swap);
    }
    for (var end = n - 1; end > 0; end--) {
      swap(0, end);
      sift(0, end, before, swap);
    }
  }

  static void sift(int root, int end, bool Function(int, int) before, void Function(int, int) swap) {
    while (true) {
      var child = 2 * root + 1;
      if (child >= end) return;
      if (child + 1 < end && before(child, child + 1)) child++;
      if (!before(root, child)) return;
      swap(root, child);
      root = child;
    }
  }

  static void exchange(List<Object?> s, int i, int j) {
    final t = s[i];
    s[i] = s[j];
    s[j] = t;
  }

  static void sortList(List<Object?> l, int line) {
    final s = List<Object?>.of(l);
    heapsort(s.length, (i, j) => lt(s[i], s[j], line), (i, j) => exchange(s, i, j));
    l
      ..clear()
      ..addAll(s);
  }

  static void sortBy(List<Object?> l, Object? function, int line) {
    final keys = <Object?>[];
    for (final item in List<Object?>.of(l)) {
      final k = call(function, <Object?>[item], line);
      if (k is! int && k is! double && k is! String) {
        fatal("the function given to `sort` must return a number or text to order by, but it returned " + typeName(k), line);
      }
      keys.add(k);
    }
    final s = List<Object?>.of(l);
    if (s.length != keys.length) raiseError("ValueError", "the function given to `sort` changed how many items the list has", line);
    final places = List<Object?>.generate(s.length, (i) => i);
    heapsort(s.length, (a, b) {
      if (lt(keys[a], keys[b], line)) return true;
      if (lt(keys[b], keys[a], line)) return false;
      return (places[a] as int) < (places[b] as int);
    }, (i, j) {
      exchange(s, i, j);
      exchange(keys, i, j);
      exchange(places, i, j);
    });
    l
      ..clear()
      ..addAll(s);
  }

  // [from, to] of a slice.
  static List<int> slice(List<Object?> args, int length, String what, int line) {
    final from = wholeArg("slice", args[0], line);
    final to = args.length > 1 ? wholeArg("slice", args[1], line) : length;
    if (from < 0 || to < from || to > length) {
      raiseError("IndexError", "slice " + from.toString() + " to " + to.toString() + " is out of range for a " + what + " of length " + length.toString(), line);
    }
    return [from, to];
  }

  static const List<String> STR_METHODS = ["upper", "lower", "strip", "split", "lines", "replace", "starts_with", "ends_with", "find", "slice", "code"];
  static const List<String> LIST_METHODS = ["append", "pop", "map", "filter", "reduce", "insert", "reverse", "sort", "clear", "find", "slice", "join"];
  static const List<String> DICT_METHODS = ["get", "pop", "keys", "values", "clear"];

  static bool hasMethod(Object? v, String name) {
    if (v is String) return STR_METHODS.contains(name);
    if (v is List<Object?>) return LIST_METHODS.contains(name);
    if (v is Map<Object?, Object?>) return DICT_METHODS.contains(name);
    return false;
  }

  static List<Object?> splitOn(String s, String sep) {
    final out = <Object?>[];
    var from = 0;
    while (true) {
      final at = s.indexOf(sep, from);
      if (at < 0) break;
      out.add(s.substring(from, at));
      from = at + sep.length;
    }
    out.add(s.substring(from));
    return out;
  }

  static Object? callMethodOf(Object? receiver, String name, List<Object?> args, int line) {
    if (receiver is String) {
      final s = receiver;
      switch (name) {
        case "upper":
        case "lower":
          arity(name, args, 0, 0, line);
          final cs = List<int>.of(s.codeUnits);
          for (var i = 0; i < cs.length; i++) {
            if (name == "upper" && cs[i] >= 0x61 && cs[i] <= 0x7a) {
              cs[i] -= 32;
            } else if (name == "lower" && cs[i] >= 0x41 && cs[i] <= 0x5a) {
              cs[i] += 32;
            }
          }
          return String.fromCharCodes(cs);
        case "strip":
          arity(name, args, 0, 0, line);
          return strip(s);
        case "split":
          arity(name, args, 0, 1, line);
          if (args.isEmpty) {
            final out = <Object?>[];
            var i = 0;
            while (i < s.length) {
              while (i < s.length && blank(s.codeUnitAt(i))) i++;
              final start = i;
              while (i < s.length && !blank(s.codeUnitAt(i))) i++;
              if (i > start) out.add(s.substring(start, i));
            }
            return out;
          }
          final sep = text(name, args[0], line);
          if (sep.isEmpty) raiseError("ValueError", "cannot split on an empty str", line);
          return splitOn(s, sep);
        case "lines":
          arity(name, args, 0, 0, line);
          if (s.isEmpty) return <Object?>[];
          // A "\r" goes with the "\n" just after it, and with nothing else.
          final parts = splitOn(s, "\n");
          final last = parts.removeLast() as String;
          for (var i = 0; i < parts.length; i++) {
            final p = parts[i] as String;
            if (p.endsWith("\r")) parts[i] = p.substring(0, p.length - 1);
          }
          if (last.isNotEmpty) parts.add(last);
          return parts;
        case "replace":
          arity(name, args, 2, 2, line);
          final old = text(name, args[0], line), now = text(name, args[1], line);
          if (old.isEmpty) {
            final b = StringBuffer(now);
            for (final c in s.runes) {
              b.writeCharCode(c);
              b.write(now);
            }
            return b.toString();
          }
          return splitOn(s, old).join(now);
        case "starts_with":
          arity(name, args, 1, 1, line);
          return s.startsWith(text(name, args[0], line));
        case "ends_with":
          arity(name, args, 1, 1, line);
          return s.endsWith(text(name, args[0], line));
        case "find":
          arity(name, args, 1, 1, line);
          final at = s.indexOf(text(name, args[0], line));
          return at < 0 ? null : textLength(s.substring(0, at));
        case "slice":
          arity(name, args, 1, 2, line);
          final r = slice(args, textLength(s), "str", line);
          return part(s, r[0], r[1]);
        case "code":
          arity(name, args, 0, 0, line);
          final n = textLength(s);
          if (n != 1) raiseError("ValueError", "`code` needs a str of exactly one character, and this one has " + n.toString(), line);
          return s.runes.first;
        default:
          fatal("a str has no method `" + name + "`", line);
      }
    }
    if (receiver is Map<Object?, Object?>) {
      final d = receiver;
      switch (name) {
        case "get":
          arity(name, args, 1, 2, line);
          final k = key(args[0], line);
          if (d.containsKey(k)) return d[k];
          return args.length > 1 ? args[1] : null;
        case "pop":
          arity(name, args, 1, 1, line);
          final k = key(args[0], line);
          if (!d.containsKey(k)) raiseError("KeyError", "this dict has no key " + repr(args[0]), line);
          return d.remove(k);
        case "keys":
          arity(name, args, 0, 0, line);
          return List<Object?>.of(d.keys);
        case "values":
          arity(name, args, 0, 0, line);
          return List<Object?>.of(d.values);
        case "clear":
          arity(name, args, 0, 0, line);
          d.clear();
          return null;
        default:
          fatal("a dict has no method `" + name + "`", line);
      }
    }
    if (receiver is List<Object?>) {
      final l = receiver;
      switch (name) {
        case "append":
          arity(name, args, 1, 1, line);
          l.add(args[0]);
          return null;
        case "pop":
          arity(name, args, 0, 1, line);
          if (args.isNotEmpty) return l.removeAt(position(args[0], l.length, "list", line));
          if (l.isEmpty) raiseError("IndexError", "cannot pop from an empty list", line);
          return l.removeLast();
        case "insert":
          arity(name, args, 2, 2, line);
          final at = wholeArg(name, args[0], line);
          if (at < 0 || at > l.length) {
            raiseError("IndexError", "cannot insert at " + at.toString() + " in a list of length " + l.length.toString(), line);
          }
          l.insert(at, args[1]);
          return null;
        case "reverse":
          arity(name, args, 0, 0, line);
          final turned = List<Object?>.of(l.reversed);
          l
            ..clear()
            ..addAll(turned);
          return null;
        case "clear":
          arity(name, args, 0, 0, line);
          l.clear();
          return null;
        case "sort":
          if (args.length == 1) {
            sortBy(l, args[0], line);
            return null;
          }
          arity(name, args, 0, 1, line);
          sortList(l, line);
          return null;
        case "find":
          arity(name, args, 1, 1, line);
          for (var at = 0; at < l.length; at++) {
            if (equal(l[at], args[0])) return at;
          }
          return null;
        case "slice":
          arity(name, args, 1, 2, line);
          final r = slice(args, l.length, "list", line);
          return List<Object?>.of(l.getRange(r[0], r[1]));
        case "join":
          arity(name, args, 1, 1, line);
          final sep = text(name, args[0], line);
          for (final item in l) {
            if (item is! String) {
              fatal("`join` puts text together, and this list holds " + typeName(item) + ". Make each item text first, with `str( )`", line);
            }
          }
          return l.join(sep);
        case "map":
          arity(name, args, 1, 1, line);
          final out = <Object?>[];
          for (final item in List<Object?>.of(l)) {
            out.add(call(args[0], <Object?>[item], line));
          }
          return out;
        case "filter":
          arity(name, args, 1, 1, line);
          final kept = <Object?>[];
          for (final item in List<Object?>.of(l)) {
            final keep = call(args[0], <Object?>[item], line);
            if (keep is! bool) {
              fatal("the function given to `filter` must return true or false, but it returned " + typeName(keep), line);
            }
            if (keep) kept.add(item);
          }
          return kept;
        case "reduce":
          arity(name, args, 2, 2, line);
          var total = args[1];
          for (final item in List<Object?>.of(l)) {
            total = call(args[0], <Object?>[total, item], line);
          }
          return total;
        default:
          fatal("a list has no method `" + name + "`", line);
      }
    }
    fatal(typeName(receiver) + " has no method `" + name + "`", line);
  }

  static final RegExp FORMAT = RegExp(r"^([<>^])?(0)?([0-9]*)(?:\.([0-9]*))?(.)?$", dotAll: true, unicode: true);

  // `{x:>8.2f}`: [<>^]? 0? width? (.precision)? kind?
  static Object? format(Object? v, String spec, int line) {
    if (spec.isEmpty) return show(v);
    final m = FORMAT.firstMatch(spec);
    final last = m?.group(5);
    if (m == null || (last != null && last.codeUnitAt(0) >= 0x30 && last.codeUnitAt(0) <= 0x39)) {
      fatal("`" + spec + "` is not a format Zeno understands", line);
    }
    final align = m.group(1);
    final zero = m.group(2) != null;
    final width = m.group(3)!.isEmpty ? 0 : int.parse(m.group(3)!);
    final int? precision = m.group(4) == null ? null : (m.group(4)!.isEmpty ? 0 : int.parse(m.group(4)!));
    final kind = m.group(5);
    final places = precision ?? 6;
    String body;
    if ((kind == "f" || (kind == null && precision != null)) && v is ZQuantity) {
      body = fixed(v.v, places) + " " + v.unit;
    } else if (kind == "f" || (kind == null && precision != null && v is double)) {
      if (!isNumber(v)) wrongType(spec, "a number", v, line);
      body = fixed(number(v), places);
    } else if ((kind == null && precision == null) || (kind == "d" && precision == null && v is int)) {
      body = show(v);
    } else if ((kind == "x" || kind == "X" || kind == "b") && precision == null && v is int) {
      String digits;
      if (v == MIN_INT) {
        digits = kind == "b" ? "1" + "0" * 63 : "8000000000000000";
      } else {
        digits = v.abs().toRadixString(kind == "b" ? 2 : 16);
      }
      if (kind == "X") digits = digits.toUpperCase();
      body = (v < 0 ? "-" : "") + digits;
    } else if ((kind == "d" || kind == "x" || kind == "X" || kind == "b") && precision == null) {
      wrongType(spec, "an int", v, line);
    } else {
      fatal("`" + spec + "` is not a format Zeno understands", line);
    }
    final padding = math.max(0, width - textLength(body));
    final numeric = isNumber(v) || v is ZQuantity;
    if (zero && numeric) {
      if (body.startsWith("-")) return "-" + "0" * padding + body.substring(1);
      return "0" * padding + body;
    }
    final a = align ?? (numeric ? ">" : "<");
    if (a == "<") return body + " " * padding;
    if (a == ">") return " " * padding + body;
    final left = padding ~/ 2;
    return " " * left + body + " " * (padding - left);
  }

  // ------------------------------------------------------------------ JSON

  static const Object NOT_JSON = ZNotJson();

  static Object? jsonParse(String s) {
    try {
      final j = ZJson(s);
      final v = j.value();
      j.spaces();
      return j.at == j.cs.length ? v : NOT_JSON;
    } on ZNotJson {
      return NOT_JSON;
    }
  }

  static String jsonText(String s) {
    final b = StringBuffer("\"");
    for (final c in s.runes) {
      if (c == 0x22) {
        b.write("\\\"");
      } else if (c == 0x5c) {
        b.write("\\\\");
      } else if (c == 0x0a) {
        b.write("\\n");
      } else if (c == 0x0d) {
        b.write("\\r");
      } else if (c == 0x09) {
        b.write("\\t");
      } else if (c < 0x20) {
        b.write("\\u" + c.toRadixString(16).padLeft(4, "0"));
      } else {
        b.writeCharCode(c);
      }
    }
    b.write("\"");
    return b.toString();
  }

  static String jsonWrite(Object? v, int line) {
    if (v == null) return "null";
    if (v is bool) return v ? "true" : "false";
    if (v is int) return v.toString();
    if (v is double) {
      if (v.isNaN || v.isInfinite) raiseError("ValueError", "JSON has no way to write " + showFloat(v), line);
      return showFloat(v);
    }
    if (v is String) return jsonText(v);
    if (v is List<Object?>) {
      final parts = <String>[];
      for (final item in v) {
        parts.add(jsonWrite(item, line));
      }
      return "[" + parts.join(",") + "]";
    }
    if (v is Map<Object?, Object?>) {
      final parts = <String>[];
      for (final entry in v.entries) {
        parts.add(jsonText(show(entry.key)) + ":" + jsonWrite(entry.value, line));
      }
      return "{" + parts.join(",") + "}";
    }
    if (v is ZInstance) {
      final parts = <String>[];
      for (var i = 0; i < v.cls.fields.length; i++) {
        parts.add(jsonText(v.cls.fields[i]) + ":" + jsonWrite(v.fields[i], line));
      }
      return "{" + parts.join(",") + "}";
    }
    if (v is ZQuantity) {
      raiseError("ValueError", "JSON has no units, and this is a quantity in " + v.unit + ". Write its number, and put the unit in the name: `[\"" + v.unit + "\": ...]`", line);
    }
    raiseError("ValueError", "JSON has objects, lists, text, numbers, bools and none, and this is " + typeName(v), line);
  }

  // ----------------------------------------------------------------- Files

  // "file", "nothing" or "other": what is at a path.
  static String at(String path) {
    if (path.isEmpty) return "nothing";
    try {
      final file = File(path).openSync();
      try {
        file.readByteSync();
      } finally {
        file.closeSync();
      }
      return "file";
    } on FileSystemException {
      if (FileSystemEntity.typeSync(path) != FileSystemEntityType.notFound) return "other";
      // A path through a file (`notes.txt/x`) is not "nothing":
      // something is in the way, as `zeno run` says.
      var up = File(path).parent;
      while (true) {
        final type = FileSystemEntity.typeSync(up.path);
        if (type != FileSystemEntityType.notFound) return type == FileSystemEntityType.directory ? "nothing" : "other";
        final next = up.parent;
        if (next.path == up.path) return "nothing";
        up = next;
      }
    }
  }

  static List<String> paths(String name, List<Object?> args, int count, int line) {
    arity(name, args, count, count, line);
    return [for (final arg in args) text(name, arg, line)];
  }

  static Never fileError(String message, int line) => raiseError("FileError", message, line);

  static final Stopwatch STARTED = Stopwatch()..start();

  // --------------------------------------------------- The built-in table

  static Object? b_print(List<Object?> args, int line) {
    for (var i = 0; i < args.length; i++) {
      if (i > 0) write(" ");
      write(show(args[i]));
    }
    write("\n");
    return null;
  }

  // Text from outside that may not be UTF-8. Each run of bytes that
  // could have begun a character and did not finish one becomes one
  // U+FFFD, and so does each byte that could not begin one: the way
  // Rust, Python and the browsers read such text.
  static String lossy(List<int> b) {
    final out = <int>[];
    var i = 0;
    while (i < b.length) {
      final c = b[i];
      if (c < 0x80) {
        out.add(c);
        i++;
        continue;
      }
      var need = 0, lo = 0x80, hi = 0xBF;
      if (c >= 0xC2 && c <= 0xDF) {
        need = 1;
      } else if (c == 0xE0) {
        need = 2;
        lo = 0xA0;
      } else if (c == 0xED) {
        need = 2;
        hi = 0x9F;
      } else if (c >= 0xE1 && c <= 0xEF) {
        need = 2;
      } else if (c == 0xF0) {
        need = 3;
        lo = 0x90;
      } else if (c == 0xF4) {
        need = 3;
        hi = 0x8F;
      } else if (c >= 0xF1 && c <= 0xF3) {
        need = 3;
      }
      var j = i + 1;
      var whole = need > 0;
      for (var k = 0; whole && k < need; k++) {
        if (j < b.length && b[j] >= lo && b[j] <= hi) {
          j++;
        } else {
          whole = false;
        }
        lo = 0x80;
        hi = 0xBF;
      }
      if (whole) {
        out.addAll(b.getRange(i, j));
      } else {
        out.addAll(const [0xEF, 0xBF, 0xBD]);
      }
      i = j;
    }
    return decoded(out)!;
  }

  static Object? b_input(List<Object?> args, int line) {
    arity("input", args, 0, 1, line);
    if (args.isNotEmpty) write(text("input", args[0], line));
    flush();
    final bytes = <int>[];
    var any = false, ended = false;
    try {
      while (true) {
        final c = stdin.readByteSync();
        if (c < 0) break;
        any = true;
        if (c == 0x0a) {
          ended = true;
          break;
        }
        bytes.add(c);
      }
    } on StdinException {
      // As at the end of the input.
    }
    if (!any) return null;
    var s = lossy(bytes);
    if (ended && s.endsWith("\r")) s = s.substring(0, s.length - 1);
    return s.replaceAll("\x00", "");
  }

  static Object? b_len(List<Object?> args, int line) {
    arity("len", args, 1, 1, line);
    final v = args[0];
    if (v is String) return textLength(v);
    if (v is List<Object?>) return v.length;
    if (v is Map<Object?, Object?>) return v.length;
    wrongType("len", "a str, a list or a dict", v, line);
  }

  static Object? b_range(List<Object?> args, int line) {
    arity("range", args, 1, 3, line);
    final n = <int>[];
    for (final arg in args) {
      if (arg is! int) wrongType("range", "ints", arg, line);
      n.add(arg);
    }
    var start = 0, stop = 0, step = 1;
    if (n.length == 1) {
      stop = n[0];
    } else if (n.length == 2) {
      start = n[0];
      stop = n[1];
    } else {
      start = n[0];
      stop = n[1];
      step = n[2];
    }
    if (step == 0) raiseError("ValueError", "the step of a range cannot be 0", line);
    final out = <Object?>[];
    var i = start;
    while (step > 0 ? i < stop : i > stop) {
      out.add(i);
      final next = i + step;
      if (step > 0 ? next < i : next > i) break;
      i = next;
    }
    return out;
  }

  // A float as an int: one too big for an int is the biggest int there
  // is, and what is not a number is 0, as in `zeno run`.
  static int toInt(double x) {
    if (x.isNaN) return 0;
    if (x >= 9223372036854775808.0) return MAX_INT;
    if (x <= -9223372036854775808.0) return MIN_INT;
    return x.toInt();
  }

  static final RegExp WHOLE = RegExp(r"^[+-]?[0-9]+$");

  static Object? b_int(List<Object?> args, int line) {
    arity("int", args, 1, 1, line);
    final v = args[0];
    if (v is int) return v;
    if (v is double) {
      if (v.isNaN || v.isInfinite) raiseError("ValueError", "cannot turn " + showFloat(v) + " into an int", line);
      return toInt(v);
    }
    if (v is String) {
      final t = strip(v).replaceAll("_", "");
      if (WHOLE.hasMatch(t)) {
        final n = int.tryParse(t.startsWith("+") ? t.substring(1) : t, radix: 10);
        if (n != null) return n;
      }
      raiseError("ValueError", "cannot turn " + quoted(v) + " into an int", line);
    }
    wrongType("int", "a number or a str", v, line);
  }

  static Object? b_float(List<Object?> args, int line) {
    arity("float", args, 1, 1, line);
    final v = args[0];
    if (v is int) return v.toDouble();
    if (v is double) return v;
    if (v is String) {
      final x = parseFloat(strip(v));
      if (x == null) raiseError("ValueError", "cannot turn " + quoted(v) + " into a float", line);
      return x;
    }
    wrongType("float", "a number or a str", v, line);
  }

  static Object? b_str(List<Object?> args, int line) {
    arity("str", args, 1, 1, line);
    return show(args[0]);
  }

  static Object? b_abs(List<Object?> args, int line) {
    arity("abs", args, 1, 1, line);
    final v = args[0];
    if (v is int) {
      if (v == MIN_INT) overflow(line);
      return v.abs();
    }
    if (v is double) return v.abs();
    if (v is ZQuantity) return ZQuantity(v.v.abs(), v.unit);
    wrongType("abs", "a number", v, line);
  }

  static Object? b_min(List<Object?> args, int line) => pick("min", args, line, true);

  static Object? b_max(List<Object?> args, int line) => pick("max", args, line, false);

  static Object? b_sum(List<Object?> args, int line) {
    final values = numbers("sum", args, line);
    if (unitOfNumbers != null) {
      fatal("`sum` works on plain numbers, and these have a unit. Add quantities with `+`, which keeps the unit: start from `var total = 0 V` and add each in a loop", line);
    }
    var total = 0;
    var allInts = true;
    var floatTotal = 0.0;
    for (final item in values) {
      floatTotal += number(item);
      if (allInts && item is int) {
        total = addInt(total, item, line);
      } else {
        allInts = false;
      }
    }
    return allInts ? total : floatTotal;
  }

  static Object? b_char(List<Object?> args, int line) {
    arity("char", args, 1, 1, line);
    final code = wholeArg("char", args[0], line);
    if (code <= 0 || code > 0x10FFFF || (code >= 0xD800 && code <= 0xDFFF)) {
      raiseError("ValueError", code.toString() + " is not the code of a character", line);
    }
    return String.fromCharCode(code);
  }

  static Object? b_allocate(List<Object?> args, int line) {
    fatal("manual memory is only for a program compiled to machine code", line);
  }

  static Object? b_free(List<Object?> args, int line) => b_allocate(args, line);

  static double oneNumber(String name, List<Object?> args, int line) {
    arity(name, args, 1, 1, line);
    if (!isNumber(args[0])) wrongType(name, "a number", args[0], line);
    return number(args[0]);
  }

  static double angle(String name, List<Object?> args, int line) {
    arity(name, args, 1, 1, line);
    final v = args[0];
    if (v is ZQuantity) {
      final s = convert(v.unit, "rad");
      if (s == null) fatal("`" + name + "` needs a number or an angle, such as `30 deg`, not a quantity in " + v.unit, line);
      return apply(s, v.v);
    }
    if (!isNumber(v)) wrongType(name, "a number or an angle, such as `30 deg`", v, line);
    return number(v);
  }

  static double ratio(String name, List<Object?> args, int line) {
    final x = oneNumber(name, args, line);
    if (x < -1 || x > 1) raiseError("ValueError", "`" + name + "` needs a number from -1 to 1, and this is " + showFloat(x), line);
    return x;
  }

  static double aboveZero(String name, List<Object?> args, int line) {
    final x = oneNumber(name, args, line);
    if (x <= 0) raiseError("ValueError", "cannot take the logarithm of " + showFloat(x), line);
    return x;
  }

  static Object? b_math_sqrt(List<Object?> args, int line) {
    final x = oneNumber("sqrt", args, line);
    if (x < 0) raiseError("ValueError", "cannot take the square root of " + showFloat(x), line);
    return math.sqrt(x);
  }

  static Object? b_math_floor(List<Object?> args, int line) => toInt(oneNumber("floor", args, line).floorToDouble());

  static Object? b_math_ceil(List<Object?> args, int line) => toInt(oneNumber("ceil", args, line).ceilToDouble());

  // A half goes away from zero, as Rust's `round` does.
  static Object? b_math_round(List<Object?> args, int line) {
    final x = oneNumber("round", args, line);
    if (x.isNaN) return 0;
    var r = x.abs().floorToDouble();
    if (x.abs() - r >= 0.5) r += 1;
    return toInt(x.isNegative ? -r : r);
  }

  static Object? b_math_sin(List<Object?> args, int line) => math.sin(angle("sin", args, line));

  static Object? b_math_cos(List<Object?> args, int line) => math.cos(angle("cos", args, line));

  static Object? b_math_tan(List<Object?> args, int line) => math.tan(angle("tan", args, line));

  static Object? b_math_asin(List<Object?> args, int line) => ZQuantity(math.asin(ratio("asin", args, line)), "rad");

  static Object? b_math_acos(List<Object?> args, int line) => ZQuantity(math.acos(ratio("acos", args, line)), "rad");

  static Object? b_math_atan(List<Object?> args, int line) => ZQuantity(math.atan(oneNumber("atan", args, line)), "rad");

  static Object? b_math_atan2(List<Object?> args, int line) {
    arity("atan2", args, 2, 2, line);
    if (!isNumber(args[0])) wrongType("atan2", "two numbers", args[0], line);
    if (!isNumber(args[1])) wrongType("atan2", "two numbers", args[1], line);
    return ZQuantity(math.atan2(number(args[0]), number(args[1])), "rad");
  }

  static Object? b_math_exp(List<Object?> args, int line) => math.exp(oneNumber("exp", args, line));

  static Object? b_math_log(List<Object?> args, int line) => math.log(aboveZero("log", args, line));

  // Dart has the natural logarithm and no other. `x` is m * 2**k, with
  // m from 1 up to 2: [m, k].
  static List<double> split2(double x) {
    var k = 0;
    if (x < 2.2250738585072014e-308) {
      // Too small to hold its own exponent: 2**54 times it is not.
      x *= 18014398509481984.0;
      k = -54;
    }
    final bits = bitsOf(x);
    k += ((bits >> 52) & 0x7ff) - 1023;
    final m = floatOf((bits & 0xFFFFFFFFFFFFF) | (1023 << 52));
    return [m, k.toDouble()];
  }

  static Object? b_math_log2(List<Object?> args, int line) {
    final x = aboveZero("log2", args, line);
    if (x.isInfinite) return x;
    // Exact for the powers of two, as Rust's `log2` is.
    final parts = split2(x);
    if (parts[0] == 1.0) return parts[1];
    return math.log(x) / math.ln2;
  }

  // As the C library works it out, so that the last digit is the same:
  // from the logarithm of the part from 1 to 2, and the power of two
  // beside it, in two pieces.
  static Object? b_math_log10(List<Object?> args, int line) {
    final x = aboveZero("log10", args, line);
    if (x.isInfinite) return x;
    final parts = split2(x);
    var m = parts[0], k = parts[1];
    // The part is kept from 0.707 to 1.414, as the C library keeps it.
    final i = k < 0 ? 1.0 : 0.0;
    if (i == 1.0) m = m / 2;
    final y = k + i;
    final z = y * 3.69423907715893078616e-13 + 4.34294481903251816668e-01 * math.log(m);
    return z + y * 3.01029995663611771306e-01;
  }

  static Object? b_time_sleep(List<Object?> args, int line) {
    arity("sleep", args, 1, 1, line);
    final v = args[0];
    if (v is! ZQuantity) wrongType("sleep", "a time with a unit, such as `500 ms`", v, line);
    final s = convert(v.unit, "s");
    if (s == null) fatal("`sleep` needs a time, such as `500 ms`, not a quantity in " + v.unit, line);
    final seconds = apply(s, v.v);
    if (seconds > 0) {
      flush();
      sleep(Duration(microseconds: (math.min(seconds, 1e9) * 1e6).ceil()));
    }
    return null;
  }

  static Object? b_time_now(List<Object?> args, int line) {
    arity("now", args, 0, 0, line);
    return ZQuantity(STARTED.elapsedMicroseconds / 1e3, "ms");
  }

  static Object? b_files_read(List<Object?> args, int line) {
    final path = paths("read", args, 1, line)[0];
    final where = at(path);
    if (where == "nothing") fileError("there is no file `" + path + "`", line);
    if (where == "other") fileError("cannot read `" + path + "`", line);
    Uint8List data;
    try {
      data = File(path).readAsBytesSync();
    } on FileSystemException {
      fileError("cannot read `" + path + "`", line);
    }
    final s = decoded(data);
    if (s == null || s.contains("\x00")) fileError("`" + path + "` does not hold text", line);
    return s;
  }

  static Object? put(String name, List<Object?> args, bool append, int line) {
    final p = paths(name, args, 2, line);
    try {
      File(p[0]).writeAsBytesSync(utf8.encode(p[1]), mode: append ? FileMode.append : FileMode.write);
    } on FileSystemException {
      fileError("cannot write `" + p[0] + "`", line);
    }
    return null;
  }

  static Object? b_files_write(List<Object?> args, int line) => put("write", args, false, line);

  static Object? b_files_append(List<Object?> args, int line) => put("append", args, true, line);

  static Object? b_files_exists(List<Object?> args, int line) => at(paths("exists", args, 1, line)[0]) == "file";

  static Object? b_files_remove(List<Object?> args, int line) {
    final path = paths("remove", args, 1, line)[0];
    final where = at(path);
    if (where == "nothing") fileError("there is no file `" + path + "`", line);
    if (where == "file") {
      try {
        File(path).deleteSync();
        return null;
      } on FileSystemException {
        // As below.
      }
    }
    fileError("cannot remove `" + path + "`", line);
  }

  static List<String> arguments = const <String>[];

  static Object? b_program_arguments(List<Object?> args, int line) {
    arity("arguments", args, 0, 0, line);
    return List<Object?>.of(arguments);
  }

  static Object? b_json_parse(List<Object?> args, int line) {
    arity("parse", args, 1, 1, line);
    final s = text("parse", args[0], line);
    final v = jsonParse(s);
    if (identical(v, NOT_JSON)) {
      var a = 0;
      while (a < s.length && " \t\n\r".contains(s[a])) a++;
      final start = s.substring(a);
      final n = textLength(start);
      final shown = n > 30 ? part(start, 0, 30) + "..." : start;
      raiseError("ValueError", "this text is not JSON: `" + shown + "`", line);
    }
    return v;
  }

  static Object? b_json_write(List<Object?> args, int line) {
    arity("write", args, 1, 1, line);
    return jsonWrite(args[0], line);
  }

  static Object? sized(String name, bool wrap, List<Object?> args, int line) {
    final w = WIDTHS[name]!;
    if (!wrap) {
      arity(name, args, 1, 1, line);
      final n = args[0];
      if (n is! int) wrongType(name, "a whole number", n, line);
      if (n < w.lo || n > w.hi) {
        raiseError("OverflowError", n.toString() + " does not fit in " + (w.signed ? "an" : "a") + " " + name + " (" + w.lo.toString() + " to " + w.hi.toString() + ")", line);
      }
      return n;
    }
    arity(name + ".wrap", args, 1, 1, line);
    final n = args[0];
    if (n is! int) wrongType(name + ".wrap", "a whole number", n, line);
    final low = n & ((1 << w.bits) - 1);
    return w.signed && low > w.hi ? low - (1 << w.bits) : low;
  }

  static Object? b_u8(List<Object?> args, int line) => sized("u8", false, args, line);
  static Object? b_u8_wrap(List<Object?> args, int line) => sized("u8", true, args, line);
  static Object? b_u16(List<Object?> args, int line) => sized("u16", false, args, line);
  static Object? b_u16_wrap(List<Object?> args, int line) => sized("u16", true, args, line);
  static Object? b_u32(List<Object?> args, int line) => sized("u32", false, args, line);
  static Object? b_u32_wrap(List<Object?> args, int line) => sized("u32", true, args, line);
  static Object? b_i8(List<Object?> args, int line) => sized("i8", false, args, line);
  static Object? b_i8_wrap(List<Object?> args, int line) => sized("i8", true, args, line);
  static Object? b_i16(List<Object?> args, int line) => sized("i16", false, args, line);
  static Object? b_i16_wrap(List<Object?> args, int line) => sized("i16", true, args, line);
  static Object? b_i32(List<Object?> args, int line) => sized("i32", false, args, line);
  static Object? b_i32_wrap(List<Object?> args, int line) => sized("i32", true, args, line);

  static Object? b_Error(List<Object?> args, int line) => newError("Error", args, line);
  static Object? b_ValueError(List<Object?> args, int line) => newError("ValueError", args, line);
  static Object? b_IndexError(List<Object?> args, int line) => newError("IndexError", args, line);
  static Object? b_KeyError(List<Object?> args, int line) => newError("KeyError", args, line);
  static Object? b_ZeroDivisionError(List<Object?> args, int line) => newError("ZeroDivisionError", args, line);
  static Object? b_OverflowError(List<Object?> args, int line) => newError("OverflowError", args, line);
  static Object? b_FileError(List<Object?> args, int line) => newError("FileError", args, line);

  static const List<String> ERROR_KINDS = ["Error", "ValueError", "IndexError", "KeyError", "ZeroDivisionError", "OverflowError", "FileError"];

  static final Map<String, ZBuiltinBody> BUILTINS = {
    "print": b_print,
    "input": b_input,
    "len": b_len,
    "range": b_range,
    "int": b_int,
    "float": b_float,
    "str": b_str,
    "abs": b_abs,
    "min": b_min,
    "max": b_max,
    "sum": b_sum,
    "char": b_char,
    "allocate": b_allocate,
    "free": b_free,
    "math.sqrt": b_math_sqrt,
    "math.floor": b_math_floor,
    "math.ceil": b_math_ceil,
    "math.round": b_math_round,
    "math.sin": b_math_sin,
    "math.cos": b_math_cos,
    "math.tan": b_math_tan,
    "math.asin": b_math_asin,
    "math.acos": b_math_acos,
    "math.atan": b_math_atan,
    "math.atan2": b_math_atan2,
    "math.exp": b_math_exp,
    "math.log": b_math_log,
    "math.log2": b_math_log2,
    "math.log10": b_math_log10,
    "time.sleep": b_time_sleep,
    "time.now": b_time_now,
    "files.read": b_files_read,
    "files.write": b_files_write,
    "files.append": b_files_append,
    "files.exists": b_files_exists,
    "files.remove": b_files_remove,
    "program.arguments": b_program_arguments,
    "json.parse": b_json_parse,
    "json.write": b_json_write,
    "u8": b_u8,
    "u8.wrap": b_u8_wrap,
    "u16": b_u16,
    "u16.wrap": b_u16_wrap,
    "u32": b_u32,
    "u32.wrap": b_u32_wrap,
    "i8": b_i8,
    "i8.wrap": b_i8_wrap,
    "i16": b_i16,
    "i16.wrap": b_i16_wrap,
    "i32": b_i32,
    "i32.wrap": b_i32_wrap,
    "Error": b_Error,
    "ValueError": b_ValueError,
    "IndexError": b_IndexError,
    "KeyError": b_KeyError,
    "ZeroDivisionError": b_ZeroDivisionError,
    "OverflowError": b_OverflowError,
    "FileError": b_FileError,
  };

  static Object? call_builtin(String name, List<Object?> args, int line) {
    final f = BUILTINS[name];
    if (f == null) fatal("there is no `" + name + "` in a program built for Dart", line);
    return f(args, line);
  }

  static final Map<String, Object?> BUILTIN_VALUES = {};

  // Something that comes with Zeno, as a value: `print`, `math.pi`.
  static Object? builtin(String name, [int line = 0]) {
    if (name == "math.pi") return math.pi;
    if (name == "math.e") return math.e;
    if (ERROR_KINDS.contains(name)) return ZErrorType(name);
    final known = BUILTIN_VALUES[name];
    if (known != null) return known;
    final body = BUILTINS[name];
    final dot = name.indexOf(".");
    if (body == null) {
      if (dot >= 0) fatal("module `" + name.substring(0, dot) + "` has no `" + name.substring(dot + 1) + "`", line);
      fatal("`" + name + "` is not defined", line);
    }
    final v = ZFunc(dot >= 0 ? name.substring(dot + 1) : name, const <String>[], const <bool>[], null, name, body);
    BUILTIN_VALUES[name] = v;
    return v;
  }

  static Object? module(String name) => ZModule(name);

  // ----------------------------------------------------------- The program

  static String where(List<String> files, int line) {
    final index = line ~/ LINES_A_FILE;
    final file = index < files.length ? files[index] : files[0];
    return " --> " + file + ":" + (line % LINES_A_FILE).toString();
  }

  // Run the program, and say what stopped it if something did.
  static void start(List<String> files, List<String> args, Object? Function() main) {
    arguments = args;
    String message;
    try {
      main();
      flush();
      return;
    } on ZRaise catch (e) {
      message = "error: uncaught " + shownName(e.value.kind) + ": " + show(e.value) + "\n" + where(files, e.line) + "\n";
    } on ZFatal catch (e) {
      message = "error: " + e.text + "\n" + where(files, e.line) + "\n";
    }
    flush();
    stderr.add(utf8.encode(message));
    exit(1);
  }
}
