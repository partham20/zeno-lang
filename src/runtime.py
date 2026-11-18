# The Zeno runtime for Python.
#
# A program built with `zeno build --target py` is this runtime followed
# by the program. It holds what every Zeno value is, and what each
# operator and built-in does with it: what ops.rs and builtins.rs do for
# `zeno run`, step for step and message for message, so that the program
# prints what `zeno run` prints. runtime.js is the same for JavaScript.
#
# How a Zeno value is held:
#
#   int        a Python int, kept to the 64 bits of a Zeno int
#   float      a Python float
#   f32        an `F32`: the float it is exactly (decision 41)
#   str        a Python str
#   bool       True, False
#   none       None
#   list       a Python list
#   dict       a Python dict, which keeps the order the keys were put in
#   quantity   a `Quantity`: a number and its unit
#   function   a Python function, with its Zeno name and parameters on it
#   object     an `Instance` of a `Class`
#   enum case  a `Case` of an `Enum`
#   error      a `ZError`
#
# A variable that a function written inside another one uses is kept in a
# `Cell`, which the inner function is handed when it is made: Python's own
# closures would see one variable for every round of a loop, and Zeno
# gives each round its own.

import sys as _sys

# Python looks for a module in the folder of the program before its own
# library, so a program called `struct.py` or `re.py` would be taken for
# Python's module of that name. The runtime uses only Python's own.
if __name__ == "__main__" and _sys.path and not getattr(_sys.flags, "safe_path", False):
    del _sys.path[0]

import math as _math
import os as _os
import re as _re
import struct as _struct
import threading as _threading
import time as _time


MAX = 9223372036854775807
MIN = -9223372036854775808
LINES_A_FILE = 1000000


class Cell:
    __slots__ = ("v",)

    def __init__(self, v=None):
        self.v = v


class Quantity:
    __slots__ = ("v", "unit")

    def __init__(self, v, unit):
        self.v = v
        self.unit = unit


class F32:
    __slots__ = ("v",)

    def __init__(self, v):
        self.v = v


def _fround(x):
    """The nearest f32 to a float, as a float."""
    try:
        return _struct.unpack("f", _struct.pack("f", x))[0]
    except OverflowError:
        return _math.copysign(_math.inf, x)


def _single(v):
    """An f32, or an int as a float and then the nearest f32 (41.2)."""
    t = type(v)
    if t is F32:
        return v.v
    if t is int:
        return _fround(float(v))
    return None


def _f32_shown(x):
    """The float with the fewest digits that reads back as this f32."""
    if x != x or x == 0.0 or x in (_math.inf, -_math.inf):
        return x
    for digits in range(1, 10):
        d = float("%.*e" % (digits - 1, x))
        if _fround(d) == x:
            return d
    return x


def _f32_op(op, a, b, line):
    x, y = _single(a), _single(b)
    if x is None or y is None:
        if type(a) is float or type(b) is float:
            fatal("`" + op + "` between " + type_name(a) + " and " + type_name(b) + " would have to choose between 32 bits and 64. Write `f32( )` around the float to work in 32, or `float( )` around the f32 to work in 64", line)
        _mismatch(op, a, b, line)
    if op in ("/", "//", "%") and y == 0:
        _by_zero(line)
    if op == "+":
        r = x + y
    elif op == "-":
        r = x - y
    elif op == "*":
        r = x * y
    elif op == "/":
        r = x / y
    elif op == "//":
        r = _floor(x / y)
    elif op == "%":
        r = x - y * _floor(x / y)
    else:
        r = pow_(x, y, line)
    return F32(_fround(r))


class Class:
    def __init__(self, name, fields, defaults, methods):
        self.name, self.fields, self.defaults, self.methods = name, fields, defaults, methods


class Instance:
    __slots__ = ("cls", "fields")

    def __init__(self, cls, fields):
        self.cls = cls
        self.fields = fields


class Enum:
    def __init__(self, name, cases):
        self.name, self.cases = name, cases


class Case:
    __slots__ = ("defn", "index", "fields")

    def __init__(self, defn, index, fields):
        self.defn, self.index, self.fields = defn, index, fields


class Constructor:
    def __init__(self, defn, index):
        self.defn, self.index = defn, index


class ErrorType:
    def __init__(self, kind):
        self.kind = kind


class ZError:
    def __init__(self, kind, message, obj):
        self.kind, self.message, self.object = kind, message, obj


class Module:
    def __init__(self, name):
        self.name = name


class Method:
    """A method of a str, a list or a dict, read but not yet called."""

    def __init__(self, receiver, name):
        self.receiver, self.name = receiver, name


class Bound:
    """A method of a class, read from an object."""

    def __init__(self, obj, fn):
        self.object, self.fn = obj, fn


class _Missing:
    def __repr__(self):
        return "MISSING"


MISSING = _Missing()


class Raise(Exception):
    """An error the program may catch, on its way to a `try`."""

    def __init__(self, value, line):
        self.value, self.line = value, line


class Fatal(Exception):
    """A mistake in the program, which nothing catches."""

    def __init__(self, message, line):
        self.message, self.line = message, line


def fatal(message, line):
    raise Fatal(message, line)


def _raise(kind, message, line):
    raise Raise(ZError(kind, message, None), line)


def _overflow(line):
    _raise("OverflowError", "the result is too big for an int", line)


def _by_zero(line):
    _raise("ZeroDivisionError", "division by zero", line)


def _int(n, line):
    if n > MAX or n < MIN:
        _overflow(line)
    return n


def _is_int(v):
    return type(v) is int


def _number(v):
    t = type(v)
    if t is int or t is float:
        return float(v) if t is int else v
    if t is F32:
        return v.v
    return None


def _to_float(n):
    """An int as Rust's `as f64` makes it: the nearest float."""
    return float(n)


def type_name(v):
    t = type(v)
    if t is int:
        return "an int"
    if t is float:
        return "a float"
    if t is F32:
        return "an f32"
    if t is str:
        return "a str"
    if t is bool:
        return "a bool"
    if v is None:
        return "none"
    if t is Quantity:
        return "a quantity in " + v.unit
    if t is list:
        return "a list"
    if t is dict:
        return "a dict"
    if callable(v) and not isinstance(v, (Class, Constructor, ErrorType)) or t is Method or t is Bound:
        return "a function"
    if t is Class:
        return "a class"
    if t is Instance:
        return "a " + v.cls.name
    if t is Enum:
        return "an enum"
    if t is Constructor:
        return "an enum case"
    if t is Case:
        return "a " + v.defn.name
    if t is ErrorType:
        return "an error type"
    if t is ZError:
        return "a " + v.kind
    if t is Module:
        return "a module"
    return "a value"


def shown_name(name):
    name = name.split("[")[0]
    return name.rsplit(".", 1)[-1]


# ------------------------------------------------------------- Showing


def show_float(x):
    """A float as Rust's `{:?}` shows it: the shortest digits that read
    back as the same number, `.0` on a whole one, and an exponent for one
    below 1e-4 or from 1e16 up."""
    if x != x:
        return "NaN"
    if x == _math.inf:
        return "inf"
    if x == -_math.inf:
        return "-inf"
    if x == 0:
        return "-0.0" if _math.copysign(1.0, x) < 0 else "0.0"
    text = repr(x)
    sign = ""
    if text[0] == "-":
        sign, text = "-", text[1:]
    if "e" in text:
        mantissa, exp = text.split("e")
        exp = int(exp)
    else:
        mantissa, exp = text, None
    digits = mantissa.replace(".", "")
    if exp is None:
        # Python wrote it without an exponent: work the exponent out.
        whole, _, fraction = mantissa.partition(".")
        if whole != "0":
            exp = len(whole) - 1
            digits = (whole + fraction).rstrip("0") or "0"
        else:
            stripped = fraction.lstrip("0")
            exp = -(len(fraction) - len(stripped)) - 1
            digits = stripped
    else:
        digits = digits.rstrip("0") or "0"
        point = mantissa.find(".")
        exp += (point if point >= 0 else len(mantissa)) - 1
    digits = digits.lstrip("0") or "0"
    if exp < -4 or exp >= 16:
        body = digits[0] + ("." + digits[1:] if len(digits) > 1 else "")
        return sign + body + "e" + str(exp)
    if exp < 0:
        return sign + "0." + "0" * (-exp - 1) + digits
    if len(digits) <= exp + 1:
        return sign + digits + "0" * (exp + 1 - len(digits)) + ".0"
    return sign + digits[: exp + 1] + "." + digits[exp + 1 :]


def fixed(x, places):
    """A float with a fixed number of decimals, as Rust's `{:.2}` writes
    it: from its exact value, a half rounding to the even digit."""
    if x != x:
        return "NaN"
    if x in (_math.inf, -_math.inf):
        return "inf" if x > 0 else "-inf"
    return format(x, "." + str(places) + "f")


def quoted(s):
    out = ['"']
    for c in s:
        code = ord(c)
        if c == '"':
            out.append('\\"')
        elif c == "\\":
            out.append("\\\\")
        elif c == "\n":
            out.append("\\n")
        elif c == "\r":
            out.append("\\r")
        elif c == "\t":
            out.append("\\t")
        elif c == "\0":
            out.append("\\0")
        elif code < 0x20 or 0x7F <= code < 0xA0:
            out.append("\\u{" + format(code, "x") + "}")
        else:
            out.append(c)
    out.append('"')
    return "".join(out)


def show(v):
    t = type(v)
    if t is str:
        return v
    if t is int:
        return str(v)
    if t is float:
        return show_float(v)
    if t is F32:
        return show_float(_f32_shown(v.v))
    if t is bool:
        return "true" if v else "false"
    if v is None:
        return "none"
    if t is list:
        return "[" + ", ".join(repr_(item) for item in v) + "]"
    if t is dict:
        if not v:
            return "[:]"
        return "[" + ", ".join(repr_(k) + ": " + repr_(x) for k, x in v.items()) + "]"
    if t is Quantity:
        x = v.v
        if x == x and x not in (_math.inf, -_math.inf) and x == int(x) and abs(x) < 1e15:
            return str(int(x)) + " " + v.unit
        return show_float(x) + " " + v.unit
    if t is Instance:
        parts = [name + "=" + repr_(v.fields[i]) for i, name in enumerate(v.cls.fields)]
        return shown_name(v.cls.name) + "(" + ", ".join(parts) + ")"
    if t is Case:
        name = shown_name(v.defn.cases[v.index][0])
        if not v.fields:
            return name
        return name + "(" + ", ".join(repr_(x) for x in v.fields) + ")"
    if t is ZError:
        return v.message
    if t is Method:
        return "<function " + v.name + ">"
    if t is Bound:
        return "<function " + v.fn.zname + ">"
    if t is Class:
        return "<class " + v.name + ">"
    if t is Enum:
        return "<enum " + v.name + ">"
    if t is Constructor:
        return "<case " + v.defn.cases[v.index][0] + ">"
    if t is ErrorType:
        return "<error " + v.kind + ">"
    if t is Module:
        return "<module " + v.name + ">"
    if callable(v):
        name = getattr(v, "zname", "")
        return "<function " + name + ">" if name else "<function>"
    return str(v)


def repr_(v):
    return quoted(v) if type(v) is str else show(v)


# --------------------------------------------------------------- Units

UNITS = {
    "ns": ("time", 1.0), "us": ("time", 1e3), "ms": ("time", 1e6), "s": ("time", 1e9),
    "min": ("time", 60e9), "h": ("time", 3600e9),
    "Hz": ("frequency", 1.0), "kHz": ("frequency", 1e3), "MHz": ("frequency", 1e6),
    "uV": ("voltage", 1.0), "mV": ("voltage", 1e3), "V": ("voltage", 1e6), "kV": ("voltage", 1e9),
    "uA": ("current", 1.0), "mA": ("current", 1e3), "A": ("current", 1e6),
    "ohm": ("resistance", 1.0), "kohm": ("resistance", 1e3), "Mohm": ("resistance", 1e6),
    "mW": ("power", 1.0), "W": ("power", 1e3), "kW": ("power", 1e6),
    "J": ("energy", 1.0), "Wh": ("energy", 3600.0), "kWh": ("energy", 3600e3),
    "degC": ("temperature in Celsius", 1.0), "K": ("temperature in kelvin", 1.0),
    "mm": ("length", 1.0), "cm": ("length", 10.0), "m": ("length", 1e3), "km": ("length", 1e6),
    "mg": ("mass", 1.0), "g": ("mass", 1e3), "kg": ("mass", 1e6),
    "deg": ("angle in degrees", 1.0), "rad": ("angle in radians", 1.0), "rpm": ("rotation", 1.0),
}


def convert(frm, to):
    if frm == "deg" and to == "rad":
        return (_math.pi / 180.0, 1.0)
    if frm == "rad" and to == "deg":
        return (180.0 / _math.pi, 1.0)
    if frm not in UNITS or to not in UNITS:
        return None
    fk, fs = UNITS[frm]
    tk, ts = UNITS[to]
    if fk != tk:
        return None
    return (fs / ts, 1.0) if fs >= ts else (1.0, ts / fs)


def apply(scale, x):
    return x * scale[0] / scale[1]


def common(a, b):
    if a not in UNITS or b not in UNITS or UNITS[a][0] != UNITS[b][0]:
        return None
    unit = a if UNITS[a][1] <= UNITS[b][1] else b
    return unit, convert(a, unit), convert(b, unit)


def unit(v, name, line):
    if name not in UNITS:
        fatal("`" + name + "` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC", line)
    x = _number(v)
    if x is None:
        fatal("a unit can only follow a number", line)
    return Quantity(x, name)


# ------------------------------------------------------------ Equality


def eq(a, b):
    ta, tb = type(a), type(b)
    if ta is int and tb is int:
        return a == b
    if (ta is int or ta is float) and (tb is int or tb is float):
        return float(a) == float(b)
    if ta is F32 or tb is F32:
        x, y = _single(a), _single(b)
        if x is not None and y is not None:
            return x == y
        x, y = _number(a), _number(b)
        return x is not None and y is not None and x == y
    if ta is str and tb is str:
        return a == b
    if ta is bool and tb is bool:
        return a == b
    if a is None or b is None:
        return a is b
    if ta is Quantity and tb is Quantity:
        c = common(a.unit, b.unit)
        return c is not None and apply(c[1], a.v) == apply(c[2], b.v)
    if ta is list and tb is list:
        return len(a) == len(b) and all(eq(x, y) for x, y in zip(a, b))
    if ta is dict and tb is dict:
        if len(a) != len(b):
            return False
        for k, x in a.items():
            if k not in b or not eq(x, b[k]):
                return False
        return True
    if ta is Instance and tb is Instance:
        return a.cls is b.cls and all(eq(x, y) for x, y in zip(a.fields, b.fields))
    if ta is Case and tb is Case:
        return a.defn is b.defn and a.index == b.index and all(eq(x, y) for x, y in zip(a.fields, b.fields))
    if ta is ZError and tb is ZError:
        return a.kind == b.kind and a.message == b.message
    if ta is Constructor and tb is Constructor:
        return a.defn is b.defn and a.index == b.index
    if ta is not tb:
        return False
    return a is b


def ne(a, b):
    return not eq(a, b)


# ----------------------------------------------------------- Operators


def _mismatch(op, a, b, line):
    fatal("`" + op + "` does not work between " + type_name(a) + " and " + type_name(b), line)


def add(a, b, line):
    if type(a) is F32 or type(b) is F32:
        return _f32_op('+', a, b, line)
    ta, tb = type(a), type(b)
    if ta is int and tb is int:
        r = a + b
        if r > MAX or r < MIN:
            _overflow(line)
        return r
    if ta is str and tb is str:
        return a + b
    if ta is list and tb is list:
        return a + b
    if ta is Quantity and tb is Quantity:
        c = common(a.unit, b.unit)
        if c is None:
            _mismatch("+", a, b, line)
        return Quantity(apply(c[1], a.v) + apply(c[2], b.v), c[0])
    x, y = _number(a), _number(b)
    if x is None or y is None:
        _mismatch("+", a, b, line)
    return x + y


def sub(a, b, line):
    if type(a) is F32 or type(b) is F32:
        return _f32_op('-', a, b, line)
    ta, tb = type(a), type(b)
    if ta is int and tb is int:
        r = a - b
        if r > MAX or r < MIN:
            _overflow(line)
        return r
    if ta is Quantity and tb is Quantity:
        c = common(a.unit, b.unit)
        if c is None:
            _mismatch("-", a, b, line)
        return Quantity(apply(c[1], a.v) - apply(c[2], b.v), c[0])
    x, y = _number(a), _number(b)
    if x is None or y is None:
        _mismatch("-", a, b, line)
    return x - y


def mul(a, b, line):
    if type(a) is F32 or type(b) is F32:
        return _f32_op('*', a, b, line)
    ta, tb = type(a), type(b)
    if ta is list and tb is int:
        if b < 0:
            _raise("ValueError", "`*` repeats a list 0 or more times, and this is " + str(b), line)
        return a * b
    if ta is int and tb is int:
        r = a * b
        if r > MAX or r < MIN:
            _overflow(line)
        return r
    if ta is Quantity or tb is Quantity:
        q, other = (a, b) if ta is Quantity else (b, a)
        y = _number(other)
        if y is None:
            _mismatch("*", a, b, line)
        return Quantity(q.v * y, q.unit)
    x, y = _number(a), _number(b)
    if x is None or y is None:
        _mismatch("*", a, b, line)
    return x * y


def _fdiv(x, y):
    try:
        return x / y
    except ZeroDivisionError:
        return _math.nan


def div(a, b, line):
    if type(a) is F32 or type(b) is F32:
        return _f32_op('/', a, b, line)
    ta, tb = type(a), type(b)
    if ta is Quantity:
        if (tb is Quantity or tb is float) and (b.v if tb is Quantity else b) == 0:
            _by_zero(line)
        if tb is int and b == 0:
            _by_zero(line)
        if tb is Quantity:
            c = common(a.unit, b.unit)
            if c is None:
                _mismatch("/", a, b, line)
            return apply(c[1], a.v) / apply(c[2], b.v)
        y = _number(b)
        if y is None:
            _mismatch("/", a, b, line)
        return Quantity(a.v / y, a.unit)
    x, y = _number(a), _number(b)
    if x is None or y is None:
        _mismatch("/", a, b, line)
    if y == 0:
        _by_zero(line)
    return x / y


def _floor(q):
    """A float rounded down, as a float: -0.0 stays -0.0, and what is not
    a number or has no end stays what it is."""
    if q != q or q in (_math.inf, -_math.inf):
        return q
    whole = float(_math.floor(q))
    return _math.copysign(whole, q) if whole == 0.0 else whole


def floordiv(a, b, line):
    if type(a) is F32 or type(b) is F32:
        return _f32_op('//', a, b, line)
    if type(a) is int and type(b) is int:
        if b == 0:
            _by_zero(line)
        return _int(a // b, line)
    x, y = _number(a), _number(b)
    if x is None or y is None:
        _mismatch("//", a, b, line)
    if y == 0:
        _by_zero(line)
    return _floor(x / y)


def mod(a, b, line):
    if type(a) is F32 or type(b) is F32:
        return _f32_op('%', a, b, line)
    if type(a) is int and type(b) is int:
        if b == 0:
            _by_zero(line)
        return a % b
    x, y = _number(a), _number(b)
    if x is None or y is None:
        _mismatch("%", a, b, line)
    if y == 0:
        _by_zero(line)
    return x - y * _floor(x / y)


def pow_(a, b, line):
    if type(a) is F32 or type(b) is F32:
        return _f32_op('**', a, b, line)
    if type(a) is int and type(b) is int:
        if b < 0:
            _raise("ValueError", "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`", line)
        if b > 4294967295:
            _overflow(line)
        if a in (0, 1):
            return 1 if b == 0 else a
        if a == -1:
            return 1 if b % 2 == 0 else -1
        if b >= 64:
            _overflow(line)
        return _int(a ** b, line)
    x, y = _number(a), _number(b)
    if x is None or y is None:
        _mismatch("**", a, b, line)
    if x == 1.0 or y == 0.0:
        return 1.0
    try:
        r = x ** y
    except ZeroDivisionError:
        return _math.inf if y < 0 and (int(y) != y or int(y) % 2 == 0 or _math.copysign(1, x) > 0) else -_math.inf
    except OverflowError:
        return _math.inf if x > 0 or int(y) % 2 == 0 else -_math.inf
    if type(r) is complex:
        return _math.nan
    return r


def _bits_mismatch(op, a, b, line):
    hint = ""
    if type(a) is bool and type(b) is bool:
        if op == "&":
            hint = ". For two bools Zeno spells this `and`"
        if op == "|":
            hint = ". For two bools Zeno spells this `or`"
    fatal("`" + op + "` works on the bits of whole numbers, and these are " + type_name(a) + " and " + type_name(b) + hint, line)


def bitand(a, b, line):
    if type(a) is not int or type(b) is not int:
        _bits_mismatch("&", a, b, line)
    return a & b


def bitor(a, b, line):
    if type(a) is not int or type(b) is not int:
        _bits_mismatch("|", a, b, line)
    return a | b


def bitxor(a, b, line):
    if type(a) is not int or type(b) is not int:
        _bits_mismatch("^", a, b, line)
    return a ^ b


def _wrap64(n):
    n &= 0xFFFFFFFFFFFFFFFF
    return n - (1 << 64) if n > MAX else n


def shl(a, n, line):
    if type(a) is not int or type(n) is not int:
        _bits_mismatch("<<", a, n, line)
    if not 0 <= n < 64:
        _raise("ValueError", "cannot shift by " + str(n) + ". An int has 64 bits, so a shift is 0 to 63", line)
    shifted = _wrap64(a << n)
    if shifted >> n != a:
        _overflow(line)
    return shifted


def shr(a, n, line):
    if type(a) is not int or type(n) is not int:
        _bits_mismatch(">>", a, n, line)
    if not 0 <= n < 64:
        _raise("ValueError", "cannot shift by " + str(n) + ". An int has 64 bits, so a shift is 0 to 63", line)
    return a >> n


def neg(v, line):
    t = type(v)
    if t is int:
        return _int(-v, line)
    if t is float:
        return -v
    if t is F32:
        return F32(-v.v)
    if t is Quantity:
        return Quantity(-v.v, v.unit)
    fatal("`-` does not work on " + type_name(v), line)


def bitnot(v, line):
    if type(v) is not int:
        fatal("`~` works on the bits of a whole number, and this is " + type_name(v), line)
    return ~v


def _order(op, a, b, line):
    ta, tb = type(a), type(b)
    if ta is F32 or tb is F32:
        x, y = _single(a), _single(b)
        if x is None or y is None:
            _f32_op(op, a, b, line)
        return -1 if x < y else (1 if x > y else (0 if x == y else None))
    if (ta is int and tb is int) or (ta is str and tb is str):
        return -1 if a < b else (1 if a > b else 0)
    if ta is Quantity and tb is Quantity:
        c = common(a.unit, b.unit)
        if c is None:
            _mismatch(op, a, b, line)
        x, y = apply(c[1], a.v), apply(c[2], b.v)
    else:
        x, y = _number(a), _number(b)
        if x is None or y is None:
            _mismatch(op, a, b, line)
    if x < y:
        return -1
    if x > y:
        return 1
    if x == y:
        return 0
    return None


def lt(a, b, line):
    o = _order("<", a, b, line)
    return o is not None and o < 0


def le(a, b, line):
    o = _order("<=", a, b, line)
    return o is not None and o <= 0


def gt(a, b, line):
    o = _order(">", a, b, line)
    return o is not None and o > 0


def ge(a, b, line):
    o = _order(">=", a, b, line)
    return o is not None and o >= 0


def _contains(op, item, container, line):
    t = type(container)
    if t is list:
        return any(eq(item, other) for other in container)
    if t is dict:
        if type(item) is not int and type(item) is not str:
            _mismatch(op, item, container, line)
        return item in container
    if type(item) is str and t is str:
        return item in container
    _mismatch(op, item, container, line)


def in_(a, b, line):
    return _contains("in", a, b, line)


def not_in(a, b, line):
    return not _contains("not in", a, b, line)


def truth(v, line):
    if v is True or v is False:
        return v
    fatal("a condition must be true or false, but this is " + type_name(v), line)


# ------------------------------------------------------ Lists and text


def _key(v, line):
    if type(v) is int or type(v) is str:
        return v
    fatal("a key of a dict is a str or an int, but this is " + type_name(v), line)


def _position(index, length, what, line):
    if type(index) is not int:
        fatal("an index must be an int, but this is " + type_name(index), line)
    if index < 0 or index >= length:
        _raise("IndexError", "index " + str(index) + " is out of range for a " + what + " of length " + str(length), line)
    return index


def index(obj, i, line):
    t = type(obj)
    if t is list:
        return obj[_position(i, len(obj), "list", line)]
    if t is str:
        return obj[_position(i, len(obj), "str", line)]
    if t is dict:
        k = _key(i, line)
        if k not in obj:
            _raise("KeyError", "this dict has no key " + repr_(i), line)
        return obj[k]
    fatal("cannot take an item from " + type_name(obj), line)


def set_index(obj, i, value, line):
    t = type(obj)
    if t is list:
        obj[_position(i, len(obj), "list", line)] = value
        return
    if t is dict:
        obj[_key(i, line)] = value
        return
    fatal("cannot set an item of " + type_name(obj), line)


def items(v, line):
    t = type(v)
    if t is list:
        return v[:]
    if t is dict:
        return list(v.keys())
    if t is str:
        return list(v)
    fatal("cannot loop over " + type_name(v), line)


def pairs(v, line):
    if type(v) is dict:
        return list(v.items())
    return list(enumerate(items(v, line)))


def list_(values):
    return values


def dict_(flat, line):
    d = {}
    for i in range(0, len(flat), 2):
        d[_key(flat[i], line)] = flat[i + 1]
    return d


def push(lst, value):
    lst.append(value)
    return None


def concat(pieces):
    return "".join(pieces)


# ------------------------------------------------- Objects and fields


def fn(name, params, defaults, function):
    function.zname = name
    function.zparams = params
    function.zdefaults = defaults
    return function


def class_(name, fields, defaults, methods):
    return Class(name, fields, defaults, methods)


def enum(name, cases):
    return Enum(name, cases)


def case(defn, i):
    return Case(defn, i, []) if not defn.cases[i][1] else Constructor(defn, i)


def is_case(value, ctor, count, line):
    if type(ctor) is not Constructor:
        fatal("this is not an enum case that carries values", line)
    if type(value) is not Case or value.defn is not ctor.defn or value.index != ctor.index:
        return False
    if len(value.fields) != count:
        fatal("`" + ctor.defn.cases[ctor.index][0] + "` carries " + str(len(value.fields)) + " values, but this pattern has " + str(count), line)
    return True


def case_field(value, i):
    return value.fields[i]


def field(obj, name, line):
    t = type(obj)
    if t is Instance:
        fields = obj.cls.fields
        if name in fields:
            return obj.fields[fields.index(name)]
        method = obj.cls.methods.get(name)
        if method is not None:
            return Bound(obj, method)
        fatal("`" + obj.cls.name + "` has no field or method `" + name + "`", line)
    if t is Case:
        names = obj.defn.cases[obj.index][1]
        if name in names:
            return obj.fields[names.index(name)]
        fatal("`" + obj.defn.cases[obj.index][0] + "` has no field `" + name + "`", line)
    if t is Enum:
        for at, c in enumerate(obj.cases):
            if c[0] == name:
                return case(obj, at)
        fatal("enum `" + obj.name + "` has no case `" + name + "`", line)
    if t is Module:
        return builtin(obj.name + "." + name, line)
    if t is Quantity and name in UNITS:
        scale = convert(obj.unit, name)
        if scale is None:
            fatal("a quantity in " + obj.unit + " cannot be turned into " + name + ". They measure different things", line)
        return Quantity(apply(scale, obj.v), name)
    if t is ZError and name == "message":
        return obj.message
    if t is ZError and name == "kind":
        return obj.kind
    if callable(obj) and getattr(obj, "zbuiltin", None) in WIDTHS and name == "wrap":
        return builtin(obj.zbuiltin + ".wrap", line)
    if _has_method(obj, name):
        return Method(obj, name)
    fatal(type_name(obj) + " has no field or method `" + name + "`", line)


def set_field(obj, name, value, line):
    if type(obj) is Instance:
        fields = obj.cls.fields
        if name not in fields:
            fatal("`" + obj.cls.name + "` has no field `" + name + "`", line)
        obj.fields[fields.index(name)] = value
        return
    fatal("cannot set a field on " + type_name(obj), line)


# ------------------------------------------------------------- Calls


def _counted(n, word):
    return str(n) + " " + word + ("" if n == 1 else "s")


def _were(n):
    return "1 was" if n == 1 else str(n) + " were"


def _bind(what, names, positional, named_names, named, line):
    if len(positional) > len(names):
        fatal("`" + what + "` takes " + _counted(len(names), "argument") + ", but " + _were(len(positional)) + " given", line)
    slots = list(positional) + [MISSING] * (len(names) - len(positional))
    for name, value in zip(named_names, named):
        if name not in names:
            fatal("`" + what + "` has no argument named `" + name + "`", line)
        at = names.index(name)
        if slots[at] is not MISSING:
            fatal("`" + what + "` was given `" + name + "` twice", line)
        slots[at] = value
    return slots


# How many calls are nested now. `zeno run` stops at 1000, so a program
# stops at the same call here.
_depth = 0


def _enter(function, line):
    global _depth
    if _depth >= 1000:
        fatal("more than 1000 calls are nested. Is `" + (function.zname or "function") + "` calling itself forever?", line)
    _depth += 1


def direct(function, line, *args):
    """A call of a function of the program known where it is written,
    with its arguments lined up already."""
    global _depth
    _enter(function, line)
    try:
        return function(*args)
    finally:
        _depth -= 1


def _call_function(function, positional, named_names, named, line):
    global _depth
    name = function.zname or "function"
    _enter(function, line)
    try:
        slots = _bind(name, function.zparams, positional, named_names, named, line)
        for i, slot in enumerate(slots):
            if slot is MISSING and not function.zdefaults[i]:
                fatal("`" + name + "` needs a value for `" + function.zparams[i] + "`", line)
        return function(*slots)
    finally:
        _depth -= 1


def _no_named(what, named_names, line):
    if named_names:
        fatal("`" + what + "` does not take a named argument `" + named_names[0] + "`", line)


def call_named(callee, positional, named_names, named, line):
    t = type(callee)
    if t is Bound:
        return _call_function(callee.fn, [callee.object] + list(positional), named_names, named, line)
    if t is Method:
        _no_named(callee.name, named_names, line)
        return _call_method_of(callee.receiver, callee.name, positional, line)
    if t is Class:
        slots = _bind(callee.name, callee.fields, positional, named_names, named, line)
        for i, slot in enumerate(slots):
            if slot is MISSING:
                if callee.defaults[i] is None:
                    fatal("`" + callee.name + "` needs a value for `" + callee.fields[i] + "`", line)
                slots[i] = callee.defaults[i]()
        return Instance(callee, slots)
    if t is Constructor:
        name, fields = callee.defn.cases[callee.index]
        slots = _bind(name, fields, positional, named_names, named, line)
        for i, slot in enumerate(slots):
            if slot is MISSING:
                fatal("`" + name + "` needs a value for `" + fields[i] + "`", line)
        return Case(callee.defn, callee.index, slots)
    if t is ErrorType:
        _no_named(callee.kind, named_names, line)
        return _new_error(callee.kind, positional, line)
    if callable(callee):
        if getattr(callee, "zbuiltin", None):
            _no_named(callee.zname, named_names, line)
            return callee(positional, line)
        return _call_function(callee, positional, named_names, named, line)
    fatal(type_name(callee) + " cannot be called", line)


def call(callee, args, line):
    return call_named(callee, args, (), (), line)


def _new_error(kind, args, line):
    if len(args) > 1:
        fatal("`" + kind + "` takes one message", line)
    return ZError(kind, show(args[0]) if args else "", None)


def call_method_named(obj, name, positional, named_names, named, line):
    t = type(obj)
    if t is Instance and name not in obj.cls.fields:
        method = obj.cls.methods.get(name)
        if method is not None:
            return _call_function(method, [obj] + list(positional), named_names, named, line)
    if t is Module:
        _no_named(name, named_names, line)
        return call_builtin(obj.name + "." + name, positional, line)
    if t is str or t is list or t is dict:
        _no_named(name, named_names, line)
        return _call_method_of(obj, name, positional, line)
    return call_named(field(obj, name, line), positional, named_names, named, line)


def call_method(obj, name, args, line):
    return call_method_named(obj, name, args, (), (), line)


# ------------------------------------------------------------- Errors


def raise_(value, line):
    if type(value) is ZError:
        raise Raise(value, line)
    if type(value) is Instance:
        raise Raise(ZError(value.cls.name, show(value), value), line)
    fatal("only an error can be raised, and this is " + type_name(value), line)


def catches(e, name):
    return name == "Error" or name == e.value.kind


def caught(e, name):
    if name != "" and name != "Error":
        return e.value.object if e.value.object is not None else e.value
    return e.value


def no_match(v, line):
    fatal("no `case` matches " + repr_(v), line)


def undefined(name, line):
    fatal("`" + name + "` is not defined", line)


# ------------------------------------------------- Built-in functions


def _arity(name, args, lo, hi, line):
    if lo <= len(args) <= hi:
        return
    wanted = _counted(lo, "argument") if lo == hi else str(lo) + " to " + str(hi) + " arguments"
    fatal("`" + name + "` takes " + wanted + ", but " + _were(len(args)) + " given", line)


def _wrong_type(name, wanted, got, line):
    fatal("`" + name + "` needs " + wanted + ", but this is " + type_name(got), line)


def _text(name, v, line):
    if type(v) is not str:
        _wrong_type(name, "a str", v, line)
    return v


def _whole(name, v, line):
    if type(v) is not int:
        _wrong_type(name, "a whole number", v, line)
    return v


_BLANKS = " \t\n\r\f"


def _strip(s):
    return s.strip(_BLANKS)


_out = []
_out_size = [0]


def flush():
    if _out:
        data = "".join(_out).encode("utf-8", "surrogatepass")
        _out.clear()
        _out_size[0] = 0
        try:
            _sys.stdout.buffer.write(data)
            _sys.stdout.buffer.flush()
        except BrokenPipeError:
            _os._exit(0)


def write(s):
    _out.append(s)
    _out_size[0] += len(s)
    if _out_size[0] > 8192:
        flush()


WIDTHS = {
    "u8": (0, 255, 8, False), "u16": (0, 65535, 16, False), "u32": (0, 4294967295, 32, False),
    "i8": (-128, 127, 8, True), "i16": (-32768, 32767, 16, True), "i32": (-2147483648, 2147483647, 32, True),
}


def _amount(v):
    return v.v if type(v) is Quantity else _number(v)


def _numbers(name, args, line):
    values = list(args[0]) if len(args) == 1 and type(args[0]) is list else list(args)
    u = None
    for place, item in enumerate(values):
        if type(item) is Quantity:
            if u is not None and u != item.unit:
                fatal("`" + name + "` needs quantities in one unit, but these are in " + u + " and in " + item.unit + ". Turn one into the other first, as in `x." + u + "`", line)
            if u is None and place > 0:
                fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line)
            u = item.unit
        elif _number(item) is None:
            _wrong_type(name, "numbers", item, line)
        elif u is not None:
            fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line)
    return values, u


def _pick(name, args, line, better):
    values, _ = _numbers(name, args, line)
    if not values:
        _raise("ValueError", "`" + name + "` of nothing", line)
    if any(type(item) is F32 for item in values) and all(type(item) in (F32, int) for item in values):
        values = [F32(_single(item)) for item in values]
    best = values[0]
    for item in values[1:]:
        if better(_amount(item), _amount(best)):
            best = item
    return best


_FLOAT_RE = _re.compile(r"^[+-]?(\d+\.?\d*([eE][+-]?\d+)?|\.\d+([eE][+-]?\d+)?|inf|infinity|nan)$", _re.IGNORECASE | _re.ASCII)


def _parse_float(s):
    if not _FLOAT_RE.match(s):
        return None
    return float(s)


def _heapsort(n, before, swap):
    def sift(root, end):
        while True:
            child = 2 * root + 1
            if child >= end:
                return
            if child + 1 < end and before(child, child + 1):
                child += 1
            if not before(root, child):
                return
            swap(root, child)
            root = child

    for root in range(n // 2 - 1, -1, -1):
        sift(root, n)
    for end in range(n - 1, 0, -1):
        swap(0, end)
        sift(0, end)


def _sort_list(xs, line):
    s = xs[:]

    def swap(i, j):
        s[i], s[j] = s[j], s[i]

    _heapsort(len(s), lambda i, j: lt(s[i], s[j], line), swap)
    xs[:] = s


def _sort_by(xs, function, line):
    keys = []
    for item in xs[:]:
        k = call(function, [item], line)
        if type(k) not in (int, float, str, F32):
            fatal("the function given to `sort` must return a number or text to order by, but it returned " + type_name(k), line)
        keys.append(k)
    s = xs[:]
    if len(s) != len(keys):
        _raise("ValueError", "the function given to `sort` changed how many items the list has", line)
    places = list(range(len(s)))

    def before(a, b):
        if lt(keys[a], keys[b], line):
            return True
        if lt(keys[b], keys[a], line):
            return False
        return places[a] < places[b]

    def swap(i, j):
        s[i], s[j] = s[j], s[i]
        keys[i], keys[j] = keys[j], keys[i]
        places[i], places[j] = places[j], places[i]

    _heapsort(len(s), before, swap)
    xs[:] = s


def _slice(args, length, what, line):
    frm = _whole("slice", args[0], line)
    to = _whole("slice", args[1], line) if len(args) > 1 else length
    if frm < 0 or to < frm or to > length:
        _raise("IndexError", "slice " + str(frm) + " to " + str(to) + " is out of range for a " + what + " of length " + str(length), line)
    return frm, to


_STR_METHODS = ("upper", "lower", "strip", "split", "lines", "replace", "starts_with", "ends_with", "find", "slice", "code")
_LIST_METHODS = ("append", "pop", "map", "filter", "reduce", "insert", "reverse", "sort", "clear", "find", "slice", "join")
_DICT_METHODS = ("get", "pop", "keys", "values", "clear")


def _has_method(v, name):
    t = type(v)
    if t is str:
        return name in _STR_METHODS
    if t is list:
        return name in _LIST_METHODS
    if t is dict:
        return name in _DICT_METHODS
    return False


_UPPER = {c: c - 32 for c in range(97, 123)}
_LOWER = {c: c + 32 for c in range(65, 91)}


def _call_method_of(receiver, name, args, line):
    t = type(receiver)
    if t is str:
        s = receiver
        if name == "upper":
            _arity(name, args, 0, 0, line)
            return s.translate(_UPPER)
        if name == "lower":
            _arity(name, args, 0, 0, line)
            return s.translate(_LOWER)
        if name == "strip":
            _arity(name, args, 0, 0, line)
            return _strip(s)
        if name == "split":
            _arity(name, args, 0, 1, line)
            if not args:
                return [w for w in _re.split(r"[ \t\n\r\f]", s) if w]
            sep = _text(name, args[0], line)
            if sep == "":
                _raise("ValueError", "cannot split on an empty str", line)
            return s.split(sep)
        if name == "lines":
            _arity(name, args, 0, 0, line)
            # A "\r" goes with the "\n" just after it, and with nothing else.
            parts = s.split("\n")
            last = parts.pop()
            lines = [p[:-1] if p.endswith("\r") else p for p in parts]
            if last != "":
                lines.append(last)
            return lines
        if name == "replace":
            _arity(name, args, 2, 2, line)
            old, new = _text(name, args[0], line), _text(name, args[1], line)
            return s.replace(old, new)
        if name == "starts_with":
            _arity(name, args, 1, 1, line)
            return s.startswith(_text(name, args[0], line))
        if name == "ends_with":
            _arity(name, args, 1, 1, line)
            return s.endswith(_text(name, args[0], line))
        if name == "find":
            _arity(name, args, 1, 1, line)
            at = s.find(_text(name, args[0], line))
            return None if at < 0 else at
        if name == "slice":
            _arity(name, args, 1, 2, line)
            frm, to = _slice(args, len(s), "str", line)
            return s[frm:to]
        if name == "code":
            _arity(name, args, 0, 0, line)
            if len(s) != 1:
                _raise("ValueError", "`code` needs a str of exactly one character, and this one has " + str(len(s)), line)
            return ord(s)
        fatal("a str has no method `" + name + "`", line)
    if t is dict:
        d = receiver
        if name == "get":
            _arity(name, args, 1, 2, line)
            k = _key(args[0], line)
            if k in d:
                return d[k]
            return args[1] if len(args) > 1 else None
        if name == "pop":
            _arity(name, args, 1, 1, line)
            k = _key(args[0], line)
            if k not in d:
                _raise("KeyError", "this dict has no key " + repr_(args[0]), line)
            return d.pop(k)
        if name == "keys":
            _arity(name, args, 0, 0, line)
            return list(d.keys())
        if name == "values":
            _arity(name, args, 0, 0, line)
            return list(d.values())
        if name == "clear":
            _arity(name, args, 0, 0, line)
            d.clear()
            return None
        fatal("a dict has no method `" + name + "`", line)
    if t is list:
        xs = receiver
        if name == "append":
            _arity(name, args, 1, 1, line)
            xs.append(args[0])
            return None
        if name == "pop":
            _arity(name, args, 0, 1, line)
            if args:
                return xs.pop(_position(args[0], len(xs), "list", line))
            if not xs:
                _raise("IndexError", "cannot pop from an empty list", line)
            return xs.pop()
        if name == "insert":
            _arity(name, args, 2, 2, line)
            at = _whole(name, args[0], line)
            if at < 0 or at > len(xs):
                _raise("IndexError", "cannot insert at " + str(at) + " in a list of length " + str(len(xs)), line)
            xs.insert(at, args[1])
            return None
        if name == "reverse":
            _arity(name, args, 0, 0, line)
            xs.reverse()
            return None
        if name == "clear":
            _arity(name, args, 0, 0, line)
            xs.clear()
            return None
        if name == "sort":
            if len(args) == 1:
                _sort_by(xs, args[0], line)
                return None
            _arity(name, args, 0, 1, line)
            _sort_list(xs, line)
            return None
        if name == "find":
            _arity(name, args, 1, 1, line)
            for at, item in enumerate(xs):
                if eq(item, args[0]):
                    return at
            return None
        if name == "slice":
            _arity(name, args, 1, 2, line)
            frm, to = _slice(args, len(xs), "list", line)
            return xs[frm:to]
        if name == "join":
            _arity(name, args, 1, 1, line)
            sep = _text(name, args[0], line)
            for item in xs:
                if type(item) is not str:
                    fatal("`join` puts text together, and this list holds " + type_name(item) + ". Make each item text first, with `str( )`", line)
            return sep.join(xs)
        if name == "map":
            _arity(name, args, 1, 1, line)
            return [call(args[0], [item], line) for item in xs[:]]
        if name == "filter":
            _arity(name, args, 1, 1, line)
            kept = []
            for item in xs[:]:
                keep = call(args[0], [item], line)
                if keep is True:
                    kept.append(item)
                elif keep is not False:
                    fatal("the function given to `filter` must return true or false, but it returned " + type_name(keep), line)
            return kept
        if name == "reduce":
            _arity(name, args, 2, 2, line)
            total = args[1]
            for item in xs[:]:
                total = call(args[0], [total, item], line)
            return total
        fatal("a list has no method `" + name + "`", line)
    fatal(type_name(receiver) + " has no method `" + name + "`", line)


_FORMAT_RE = _re.compile(r"^([<>^])?(0)?(\d*)(?:\.(\d*))?(.)?$", _re.S)


def format_(v, spec, line):
    if spec == "":
        return show(v)
    m = _FORMAT_RE.match(spec)
    if not m or (m.group(5) is not None and m.group(5).isdigit()):
        fatal("`" + spec + "` is not a format Zeno understands", line)
    align, zero, width = m.group(1), m.group(2) is not None, int(m.group(3) or 0)
    precision = int(m.group(4) or 0) if m.group(4) is not None else None
    kind = m.group(5)
    t = type(v)
    if (kind == "f" or (kind is None and precision is not None)) and t is Quantity:
        body = fixed(v.v, 6 if precision is None else precision) + " " + v.unit
    elif kind == "f" or (kind is None and precision is not None and t is float):
        x = _number(v)
        if x is None:
            _wrong_type(spec, "a number", v, line)
        body = fixed(x, 6 if precision is None else precision)
    elif (kind is None and precision is None) or (kind == "d" and precision is None and t is int):
        body = show(v)
    elif kind in ("x", "X", "b") and precision is None and t is int:
        digits = format(abs(v), "b" if kind == "b" else kind)
        body = ("-" if v < 0 else "") + digits
    elif kind in ("d", "x", "X", "b") and precision is None:
        _wrong_type(spec, "an int", v, line)
    else:
        fatal("`" + spec + "` is not a format Zeno understands", line)
    padding = max(0, width - len(body))
    is_number = _number(v) is not None or t is Quantity
    if zero and is_number:
        if body.startswith("-"):
            return "-" + "0" * padding + body[1:]
        return "0" * padding + body
    a = align or (">" if is_number else "<")
    if a == "<":
        return body + " " * padding
    if a == ">":
        return " " * padding + body
    left = padding // 2
    return " " * left + body + " " * (padding - left)


# ------------------------------------------------------------ JSON


class _NotJson(Exception):
    pass


def _json_parse(s):
    cs = s
    at = [0]

    def spaces():
        while at[0] < len(cs) and cs[at[0]] in " \t\n\r":
            at[0] += 1

    def word(w):
        if cs[at[0]:at[0] + len(w)] == w:
            at[0] += len(w)
            return True
        return False

    def four_hex():
        digits = cs[at[0]:at[0] + 4]
        at[0] += 4
        if len(digits) != 4 or not all(c in "0123456789abcdefABCDEF" for c in digits):
            raise _NotJson()
        return int(digits, 16)

    def text():
        if at[0] >= len(cs) or cs[at[0]] != '"':
            raise _NotJson()
        at[0] += 1
        out = []
        while True:
            if at[0] >= len(cs):
                raise _NotJson()
            c = cs[at[0]]
            at[0] += 1
            if c == '"':
                return "".join(out)
            if c == "\\":
                if at[0] >= len(cs):
                    raise _NotJson()
                e = cs[at[0]]
                at[0] += 1
                if e == "n":
                    out.append("\n")
                elif e == "t":
                    out.append("\t")
                elif e == "r":
                    out.append("\r")
                elif e == "b":
                    out.append("\b")
                elif e == "f":
                    out.append("\f")
                elif e == "u":
                    code = four_hex()
                    if 0xD800 <= code < 0xDC00 and word("\\u"):
                        low = four_hex()
                        if not 0xDC00 <= low <= 0xDFFF:
                            raise _NotJson()
                        code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00)
                    if 0xD800 <= code <= 0xDFFF or code > 0x10FFFF:
                        raise _NotJson()
                    out.append(chr(code))
                else:
                    out.append(e)
            else:
                out.append(c)

    def value():
        spaces()
        if at[0] >= len(cs):
            raise _NotJson()
        c = cs[at[0]]
        if c == "n":
            if word("null"):
                return None
            raise _NotJson()
        if c == "t":
            if word("true"):
                return True
            raise _NotJson()
        if c == "f":
            if word("false"):
                return False
            raise _NotJson()
        if c == '"':
            return text()
        if c == "[":
            at[0] += 1
            out = []
            while True:
                spaces()
                if word("]"):
                    return out
                if out and not word(","):
                    raise _NotJson()
                out.append(value())
        if c == "{":
            at[0] += 1
            d = {}
            count = 0
            while True:
                spaces()
                if word("}"):
                    return d
                if count and not word(","):
                    raise _NotJson()
                spaces()
                name = text()
                spaces()
                if not word(":"):
                    raise _NotJson()
                v = value()
                count += 1
                if v is not None:
                    d[name] = v
        start = at[0]
        while at[0] < len(cs) and cs[at[0]] in "0123456789+-.eE":
            at[0] += 1
        x = _parse_float(cs[start:at[0]])
        if x is None:
            raise _NotJson()
        if x == int(x) if x == x and x not in (_math.inf, -_math.inf) else False:
            if abs(x) < 9.0e15:
                return int(x)
        return x

    try:
        v = value()
        spaces()
        if at[0] != len(cs):
            return MISSING
        return v
    except _NotJson:
        return MISSING


def _json_text(s):
    out = ['"']
    for c in s:
        code = ord(c)
        if c == '"':
            out.append('\\"')
        elif c == "\\":
            out.append("\\\\")
        elif c == "\n":
            out.append("\\n")
        elif c == "\r":
            out.append("\\r")
        elif c == "\t":
            out.append("\\t")
        elif code < 0x20:
            out.append("\\u%04x" % code)
        else:
            out.append(c)
    out.append('"')
    return "".join(out)


def _json_write(v, line):
    t = type(v)
    if v is None:
        return "null"
    if v is True:
        return "true"
    if v is False:
        return "false"
    if t is int:
        return str(v)
    if t is float:
        if v != v or v in (_math.inf, -_math.inf):
            _raise("ValueError", "JSON has no way to write " + ("NaN" if v != v else ("inf" if v > 0 else "-inf")), line)
        return show_float(v)
    if t is str:
        return _json_text(v)
    if t is list:
        return "[" + ",".join(_json_write(item, line) for item in v) + "]"
    if t is dict:
        return "{" + ",".join(_json_text(show(k)) + ":" + _json_write(x, line) for k, x in v.items()) + "}"
    if t is Instance:
        return "{" + ",".join(_json_text(name) + ":" + _json_write(v.fields[i], line) for i, name in enumerate(v.cls.fields)) + "}"
    if t is Quantity:
        _raise("ValueError", "JSON has no units, and this is a quantity in " + v.unit + '. Write its number, and put the unit in the name: `["' + v.unit + '": ...]`', line)
    _raise("ValueError", "JSON has objects, lists, text, numbers, bools and none, and this is " + type_name(v), line)


# ----------------------------------------------------------- Files


def _at(path):
    try:
        with open(path, "rb") as f:
            f.read(1)
        return "file"
    except FileNotFoundError:
        return "nothing"
    except OSError:
        return "other"


def _paths(name, args, count, line):
    _arity(name, args, count, count, line)
    return [_text(name, a, line) for a in args]


def _file_error(message, line):
    _raise("FileError", message, line)


_started = _time.monotonic()


# ------------------------------------------------- The built-in table


def _b_print(args, line):
    write(" ".join(show(a) for a in args) + "\n")
    return None


def _b_input(args, line):
    _arity("input", args, 0, 1, line)
    question = _text("input", args[0], line) if args else ""
    write(question)
    flush()
    data = _sys.stdin.buffer.readline()
    if not data:
        return None
    s = data.decode("utf-8", "replace")
    if s.endswith("\n"):
        s = s[:-1]
        if s.endswith("\r"):
            s = s[:-1]
    return s.replace("\0", "")


def _b_len(args, line):
    _arity("len", args, 1, 1, line)
    v = args[0]
    if type(v) in (str, list, dict):
        return len(v)
    _wrong_type("len", "a str, a list or a dict", v, line)


def _b_range(args, line):
    _arity("range", args, 1, 3, line)
    for a in args:
        if type(a) is not int:
            _wrong_type("range", "ints", a, line)
    if len(args) == 1:
        start, stop, step = 0, args[0], 1
    elif len(args) == 2:
        start, stop, step = args[0], args[1], 1
    else:
        start, stop, step = args
    if step == 0:
        _raise("ValueError", "the step of a range cannot be 0", line)
    return list(range(start, stop, step))


def _b_int(args, line):
    _arity("int", args, 1, 1, line)
    v = args[0]
    t = type(v)
    if t is int:
        return v
    if t is F32:
        v, t = v.v, float
    if t is float:
        if v != v or v in (_math.inf, -_math.inf):
            _raise("ValueError", "cannot turn " + ("NaN" if v != v else ("inf" if v > 0 else "-inf")) + " into an int", line)
        n = int(v)
        return max(MIN, min(MAX, n))
    if t is str:
        s = _strip(v).replace("_", "")
        if _re.match(r"^[+-]?[0-9]+$", s, _re.ASCII):
            n = int(s)
            if MIN <= n <= MAX:
                return n
        _raise("ValueError", "cannot turn " + quoted(v) + " into an int", line)
    _wrong_type("int", "a number or a str", v, line)


def _b_float(args, line):
    _arity("float", args, 1, 1, line)
    v = args[0]
    t = type(v)
    if t is int:
        return float(v)
    if t is float:
        return v
    if t is F32:
        return v.v
    if t is str:
        x = _parse_float(_strip(v))
        if x is None:
            _raise("ValueError", "cannot turn " + quoted(v) + " into a float", line)
        return x
    _wrong_type("float", "a number or a str", v, line)


def _b_f32(args, line):
    _arity("f32", args, 1, 1, line)
    v = args[0]
    t = type(v)
    if t is F32:
        return v
    if t is int:
        return F32(_single(v))
    if t is float:
        r = _fround(v)
        if r in (_math.inf, -_math.inf) and v == v and v not in (_math.inf, -_math.inf):
            _raise("OverflowError", show_float(v) + " does not fit in an f32, whose largest is about 3.4e38", line)
        return F32(r)
    _wrong_type("f32", "a number", v, line)


def _b_str(args, line):
    _arity("str", args, 1, 1, line)
    return show(args[0])


def _b_abs(args, line):
    _arity("abs", args, 1, 1, line)
    v = args[0]
    t = type(v)
    if t is int:
        return _int(abs(v), line)
    if t is float:
        return abs(v)
    if t is F32:
        return F32(abs(v.v))
    if t is Quantity:
        return Quantity(abs(v.v), v.unit)
    _wrong_type("abs", "a number", v, line)


def _b_min(args, line):
    return _pick("min", args, line, lambda a, b: a < b)


def _b_max(args, line):
    return _pick("max", args, line, lambda a, b: a > b)


def _b_sum(args, line):
    values, u = _numbers("sum", args, line)
    if u is not None:
        fatal("`sum` works on plain numbers, and these have a unit. Add quantities with `+`, which keeps the unit: start from `var total = 0 V` and add each in a loop", line)
    if any(type(item) is F32 for item in values) and all(type(item) in (F32, int) for item in values):
        single = 0.0
        for item in values:
            single = _fround(single + _single(item))
        return F32(single)
    total, all_ints, float_total = 0, True, 0.0
    for item in values:
        float_total += _number(item)
        if all_ints and type(item) is int:
            total = add(total, item, line)
        else:
            all_ints = False
    return total if all_ints else float_total


def _b_program_exit(args, line):
    """`program.exit(code)`: what was printed goes out, and the program
    ends with `code`. Not as an exception, which an `except` could stop."""
    _arity("exit", args, 1, 1, line)
    code = _whole("exit", args[0], line)
    if code < 0 or code > 255:
        _raise("ValueError", "a program ends with a number from 0 to 255, not " + str(code), line)
    flush()
    _sys.stderr.flush()
    _os._exit(code)


def _b_program_environment(args, line):
    """`program.environment(name)`: the text, or none. Bytes that are not
    UTF-8 are read as well as they can be."""
    _arity("environment", args, 1, 1, line)
    value = _os.environ.get(_text("environment", args[0], line))
    if value is None:
        return None
    return value.encode("utf-8", "surrogateescape").decode("utf-8", "replace")


def _b_program_warn(args, line):
    """`program.warn(text)`: one line on the error stream, after what was printed."""
    _arity("warn", args, 1, 1, line)
    said = _text("warn", args[0], line)
    flush()
    _sys.stderr.buffer.write((said + "\n").encode("utf-8", "surrogatepass"))
    _sys.stderr.buffer.flush()
    return None


def _b_char(args, line):
    _arity("char", args, 1, 1, line)
    code = _whole("char", args[0], line)
    if code <= 0 or code > 0x10FFFF or 0xD800 <= code <= 0xDFFF:
        _raise("ValueError", str(code) + " is not the code of a character", line)
    return chr(code)


def _b_memory(args, line):
    fatal("manual memory is only for a program compiled to machine code", line)


def _one_number(name, args, line):
    _arity(name, args, 1, 1, line)
    x = _number(args[0])
    if x is None:
        _wrong_type(name, "a number", args[0], line)
    return x


def _to_int(x):
    if x != x:
        return 0
    if x >= 9.223372036854775807e18:
        return MAX
    if x <= -9.223372036854775808e18:
        return MIN
    return int(x)


def _angle(name, args, line):
    _arity(name, args, 1, 1, line)
    v = args[0]
    if type(v) is Quantity:
        scale = convert(v.unit, "rad")
        if scale is None:
            fatal("`" + name + "` needs a number or an angle, such as `30 deg`, not a quantity in " + v.unit, line)
        return apply(scale, v.v)
    x = _number(v)
    if x is None:
        _wrong_type(name, "a number or an angle, such as `30 deg`", v, line)
    return x


def _ratio(name, args, line):
    x = _one_number(name, args, line)
    if x < -1 or x > 1:
        _raise("ValueError", "`" + name + "` needs a number from -1 to 1, and this is " + show_float(x), line)
    return x


def _above_zero(name, args, line):
    x = _one_number(name, args, line)
    if x <= 0:
        _raise("ValueError", "cannot take the logarithm of " + show_float(x), line)
    return x


def _safe(f, x):
    try:
        return f(x)
    except (ValueError, OverflowError):
        return _math.nan if x == x else x


def _round(x):
    if x != x or x in (_math.inf, -_math.inf):
        return x
    r = _math.floor(abs(x) + 0.5)
    if r - abs(x) == 0.5 and abs(x) + 0.5 != r:
        pass
    # A half goes away from zero, as Rust's `round` does.
    whole = _math.floor(abs(x))
    rounded = whole + 1 if abs(x) - whole >= 0.5 else whole
    return _math.copysign(rounded, x)


def _b_sqrt(args, line):
    x = _one_number("sqrt", args, line)
    if x < 0:
        _raise("ValueError", "cannot take the square root of " + show_float(x), line)
    return _math.sqrt(x)


def _b_sleep(args, line):
    _arity("sleep", args, 1, 1, line)
    v = args[0]
    if type(v) is not Quantity:
        _wrong_type("sleep", "a time with a unit, such as `500 ms`", v, line)
    scale = convert(v.unit, "s")
    if scale is None:
        fatal("`sleep` needs a time, such as `500 ms`, not a quantity in " + v.unit, line)
    seconds = apply(scale, v.v)
    if seconds > 0:
        flush()
        _time.sleep(min(seconds, 1e9))
    return None


def _b_now(args, line):
    _arity("now", args, 0, 0, line)
    return Quantity((_time.monotonic() - _started) * 1000.0, "ms")


def _b_files_read(args, line):
    (path,) = _paths("read", args, 1, line)
    where = _at(path)
    if where == "nothing":
        _file_error("there is no file `" + path + "`", line)
    if where == "other":
        _file_error("cannot read `" + path + "`", line)
    try:
        with open(path, "rb") as f:
            data = f.read()
    except OSError:
        _file_error("cannot read `" + path + "`", line)
    try:
        s = data.decode("utf-8")
    except UnicodeDecodeError:
        s = None
    if s is None or "\0" in s:
        _file_error("`" + path + "` does not hold text", line)
    return s


def _put(name, args, append, line):
    path, s = _paths(name, args, 2, line)
    try:
        with open(path, "ab" if append else "wb") as f:
            f.write(s.encode("utf-8"))
    except OSError:
        _file_error("cannot write `" + path + "`", line)
    return None


def _b_files_read_bytes(args, line):
    (path,) = _paths("read_bytes", args, 1, line)
    where = _at(path)
    if where == "nothing":
        _file_error("there is no file `" + path + "`", line)
    if where == "other":
        _file_error("cannot read `" + path + "`", line)
    try:
        with open(path, "rb") as f:
            return list(f.read())
    except OSError:
        _file_error("cannot read `" + path + "`", line)


def _b_files_write_bytes(args, line):
    _arity("write_bytes", args, 2, 2, line)
    path = _text("write_bytes", args[0], line)
    items = args[1]
    if type(items) is not list:
        _wrong_type("write_bytes", "a list[u8]", items, line)
    for n in items:
        if type(n) is not int:
            _wrong_type("write_bytes", "a list[u8]", n, line)
        if n < 0 or n > 255:
            _raise("ValueError", str(n) + " is not a byte: a byte is a number from 0 to 255", line)
    try:
        with open(path, "wb") as f:
            f.write(bytes(items))
    except OSError:
        _file_error("cannot write `" + path + "`", line)
    return None


def _b_files_remove(args, line):
    (path,) = _paths("remove", args, 1, line)
    where = _at(path)
    if where == "nothing":
        _file_error("there is no file `" + path + "`", line)
    if where == "file":
        try:
            _os.remove(path)
            return None
        except OSError:
            pass
    _file_error("cannot remove `" + path + "`", line)


def _b_json_parse(args, line):
    _arity("parse", args, 1, 1, line)
    s = _text("parse", args[0], line)
    v = _json_parse(s)
    if v is MISSING:
        start = s.lstrip(" \t\n\r")
        _raise("ValueError", "this text is not JSON: `" + start[:30] + ("..." if len(start) > 30 else "") + "`", line)
    return v


BUILTINS = {
    "print": _b_print,
    "input": _b_input,
    "len": _b_len,
    "range": _b_range,
    "int": _b_int,
    "float": _b_float,
    "f32": _b_f32,
    "str": _b_str,
    "abs": _b_abs,
    "min": _b_min,
    "max": _b_max,
    "sum": _b_sum,
    "char": _b_char,
    "allocate": _b_memory,
    "free": _b_memory,
    "math.sqrt": _b_sqrt,
    "math.floor": lambda args, line: _to_int(_math.floor(_one_number("floor", args, line)) if _math.isfinite(_one_number("floor", args, line)) else _one_number("floor", args, line)),
    "math.ceil": lambda args, line: _to_int(_math.ceil(_one_number("ceil", args, line)) if _math.isfinite(_one_number("ceil", args, line)) else _one_number("ceil", args, line)),
    "math.round": lambda args, line: _to_int(_round(_one_number("round", args, line))),
    "math.sin": lambda args, line: _safe(_math.sin, _angle("sin", args, line)),
    "math.cos": lambda args, line: _safe(_math.cos, _angle("cos", args, line)),
    "math.tan": lambda args, line: _safe(_math.tan, _angle("tan", args, line)),
    "math.asin": lambda args, line: Quantity(_math.asin(_ratio("asin", args, line)), "rad"),
    "math.acos": lambda args, line: Quantity(_math.acos(_ratio("acos", args, line)), "rad"),
    "math.atan": lambda args, line: Quantity(_math.atan(_one_number("atan", args, line)), "rad"),
    "math.exp": lambda args, line: _safe(lambda x: _math.exp(x) if x < 710 else _math.inf, _one_number("exp", args, line)),
    "math.log": lambda args, line: _math.log(_above_zero("log", args, line)),
    "math.log2": lambda args, line: _math.log2(_above_zero("log2", args, line)),
    "math.log10": lambda args, line: _math.log10(_above_zero("log10", args, line)),
    "time.sleep": _b_sleep,
    "time.now": _b_now,
    "files.read": _b_files_read,
    "files.write": lambda args, line: _put("write", args, False, line),
    "files.read_bytes": _b_files_read_bytes,
    "files.write_bytes": _b_files_write_bytes,
    "files.append": lambda args, line: _put("append", args, True, line),
    "files.exists": lambda args, line: _at(_paths("exists", args, 1, line)[0]) == "file",
    "files.remove": _b_files_remove,
    "program.exit": _b_program_exit,
    "program.environment": _b_program_environment,
    "program.warn": _b_program_warn,
    "program.arguments": lambda args, line: (_arity("arguments", args, 0, 0, line), list(_sys.argv[1:]))[1],
    "json.parse": _b_json_parse,
    "json.write": lambda args, line: (_arity("write", args, 1, 1, line), _json_write(args[0], line))[1],
}


def _atan2(args, line):
    _arity("atan2", args, 2, 2, line)
    y, x = _number(args[0]), _number(args[1])
    if y is None:
        _wrong_type("atan2", "two numbers", args[0], line)
    if x is None:
        _wrong_type("atan2", "two numbers", args[1], line)
    return Quantity(_math.atan2(y, x), "rad")


BUILTINS["math.atan2"] = _atan2


def _sized(name, lo, hi, bits, signed):
    article = "an" if signed else "a"

    def fit(args, line):
        _arity(name, args, 1, 1, line)
        n = args[0]
        if type(n) is not int:
            _wrong_type(name, "a whole number", n, line)
        if n < lo or n > hi:
            _raise("OverflowError", str(n) + " does not fit in " + article + " " + name + " (" + str(lo) + " to " + str(hi) + ")", line)
        return n

    def wrap(args, line):
        _arity(name + ".wrap", args, 1, 1, line)
        n = args[0]
        if type(n) is not int:
            _wrong_type(name + ".wrap", "a whole number", n, line)
        low = n & ((1 << bits) - 1)
        return low - (1 << bits) if signed and low > hi else low

    BUILTINS[name] = fit
    BUILTINS[name + ".wrap"] = wrap


for _name, (_lo, _hi, _bits, _signed) in WIDTHS.items():
    _sized(_name, _lo, _hi, _bits, _signed)

ERROR_KINDS = ("Error", "ValueError", "IndexError", "KeyError", "ZeroDivisionError", "OverflowError", "FileError")
for _kind in ERROR_KINDS:
    BUILTINS[_kind] = (lambda kind: lambda args, line: _new_error(kind, args, line))(_kind)


def call_builtin(name, args, line):
    function = BUILTINS.get(name)
    if function is None:
        fatal("there is no `" + name + "` in a program built for Python", line)
    return function(args, line)


_builtin_values = {}


def builtin(name, line=0):
    if name == "math.pi":
        return _math.pi
    if name == "math.e":
        return _math.e
    if name in ERROR_KINDS:
        return ErrorType(name)
    value = _builtin_values.get(name)
    if value is not None:
        return value
    body = BUILTINS.get(name)
    if body is None:
        if "." in name:
            module, member = name.split(".", 1)
            fatal("module `" + module + "` has no `" + member + "`", line)
        fatal("`" + name + "` is not defined", line)

    def value(args, line):
        return body(args, line)

    value.zname = name.split(".", 1)[-1]
    value.zbuiltin = name
    _builtin_values[name] = value
    return value


def module(name):
    return Module(name)


# The built-in functions, by name, with `_` for the `.` of a module: Z.b.math_sqrt
class b:
    pass


for _name, _function in BUILTINS.items():
    setattr(b, _name.replace(".", "_"), staticmethod(_function))


# ------------------------------------------------------- The program


def start(files, main):
    """Run the program, and say what stopped it if something did."""

    def where(line):
        index = line // LINES_A_FILE
        file = files[index] if index < len(files) else files[0]
        return " --> " + file + ":" + str(line % LINES_A_FILE)

    status = [0]

    def run():
        try:
            main()
            flush()
        except Raise as e:
            flush()
            _sys.stderr.write("error: uncaught " + shown_name(e.value.kind) + ": " + show(e.value) + "\n" + where(e.line) + "\n")
            status[0] = 1
        except Fatal as e:
            flush()
            _sys.stderr.write("error: " + e.message + "\n" + where(e.line) + "\n")
            status[0] = 1
        except RecursionError:
            flush()
            _sys.stderr.write("error: too many calls are nested. Is a function calling itself forever?\n")
            status[0] = 1

    # Deep recursion needs a deep stack: the program runs on a thread
    # with one of its own.
    _sys.setrecursionlimit(1000000)
    _threading.stack_size(512 * 1024 * 1024)
    thread = _threading.Thread(target=run)
    thread.start()
    thread.join()
    _sys.stdout.flush()
    _sys.exit(status[0])
