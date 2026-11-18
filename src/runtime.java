// The Zeno runtime for Java.
//
// A program built with `zeno build --target java` is one `.java` file: the
// program, in the class `ZenoProgram`, and this runtime, in the class `Z`
// beside it. It holds what every Zeno value is, and what each operator and
// built-in does with it: what ops.rs and builtins.rs do for `zeno run`,
// step for step and message for message, so that the program prints what
// `zeno run` prints. runtime.go, runtime.js and runtime.py are the same
// for the other targets.
//
// How a Zeno value is held (every one is an Object):
//
//   int        Long, with every step checked for overflow
//   float      Double
//   str        String; an index counts characters, not UTF-16 units
//   bool       Boolean
//   none       null
//   list       Z.ZList
//   dict       Z.ZDict, which keeps the order the keys were put in
//   quantity   Z.Quantity: a number and its unit
//   function   Z.Func
//   object     Z.Instance of a Z.ZClass
//   enum case  Z.Case of a Z.ZEnum
//   error      Z.ZError
//
// Sine, logarithm and the rest are Math's, which agree with the C library
// that `zeno run` uses more often than StrictMath's (fdlibm) do.
//
// A variable that an inner function uses is kept in a Z.Cell, since a
// Java lambda can only see variables that never change.

final class Z {
    private Z() {}

    static final int LINES_A_FILE = 1000000;

    // A true that Java does not know is always true, so that `while (Z.TRUE)`
    // can be followed by more code.
    static boolean TRUE = true;

    // ------------------------------------------------------------ Values

    static final class Cell {
        Object v;
        Cell() {}
        Cell(Object v) { this.v = v; }
    }

    static final class ZList {
        java.util.ArrayList<Object> items;
        ZList(java.util.ArrayList<Object> items) { this.items = items; }
    }

    static final class ZDict {
        java.util.LinkedHashMap<Object, Object> map = new java.util.LinkedHashMap<>();
    }

    static final class Quantity {
        final double v;
        final String unit;
        Quantity(double v, String unit) { this.v = v; this.unit = unit; }
    }

    interface Body { Object call(Object[] args); }

    interface BuiltinBody { Object call(Object[] args, int line); }

    static final class Func {
        final String name;
        final String[] params;
        final boolean[] defaults;
        final Body body;
        // A function that comes with Zeno: its full name, and its body.
        final String builtin;
        final BuiltinBody bbody;
        Func(String name, String[] params, boolean[] defaults, Body body, String builtin, BuiltinBody bbody) {
            this.name = name; this.params = params; this.defaults = defaults; this.body = body;
            this.builtin = builtin; this.bbody = bbody;
        }
    }

    static final class ZClass {
        final String name;
        final String[] fields;
        final Object[] defaults;
        final java.util.HashMap<String, Func> methods;
        ZClass(String name, String[] fields, Object[] defaults, java.util.HashMap<String, Func> methods) {
            this.name = name; this.fields = fields; this.defaults = defaults; this.methods = methods;
        }
    }

    static final class Instance {
        final ZClass cls;
        final Object[] fields;
        Instance(ZClass cls, Object[] fields) { this.cls = cls; this.fields = fields; }
    }

    static final class ZEnum {
        final String name;
        final String[] cases;
        final String[][] fields;
        ZEnum(String name, String[] cases, String[][] fields) { this.name = name; this.cases = cases; this.fields = fields; }
    }

    static final class Case {
        final ZEnum def;
        final int index;
        final Object[] fields;
        Case(ZEnum def, int index, Object[] fields) { this.def = def; this.index = index; this.fields = fields; }
    }

    static final class Constructor {
        final ZEnum def;
        final int index;
        Constructor(ZEnum def, int index) { this.def = def; this.index = index; }
    }

    static final class ErrorType {
        final String kind;
        ErrorType(String kind) { this.kind = kind; }
    }

    static final class ZError {
        final String kind, message;
        final Object object;
        ZError(String kind, String message, Object object) { this.kind = kind; this.message = message; this.object = object; }
    }

    static final class Module {
        final String name;
        Module(String name) { this.name = name; }
    }

    // A method of a str, a list or a dict, read but not yet called.
    static final class Method {
        final Object receiver;
        final String name;
        Method(Object receiver, String name) { this.receiver = receiver; this.name = name; }
    }

    // A method of a class, read from an object.
    static final class Bound {
        final Object object;
        final Func fn;
        Bound(Object object, Func fn) { this.object = object; this.fn = fn; }
    }

    // What a call gives a parameter it leaves out.
    static final Object MISSING = new Object();

    // An error the program may catch, on its way to a `try`.
    static final class Raise extends RuntimeException {
        final ZError value;
        final int line;
        Raise(ZError value, int line) { super(null, null, false, false); this.value = value; this.line = line; }
    }

    // A mistake in the program, which nothing catches.
    static final class Fatal extends RuntimeException {
        final String text;
        final int line;
        Fatal(String text, int line) { super(null, null, false, false); this.text = text; this.line = line; }
    }

    static Fatal fatal(String message, int line) { throw new Fatal(message, line); }

    static Raise raiseError(String kind, String message, int line) { throw new Raise(new ZError(kind, message, null), line); }

    static Raise overflow(int line) { return raiseError("OverflowError", "the result is too big for an int", line); }

    static Raise byZero(int line) { return raiseError("ZeroDivisionError", "division by zero", line); }

    static boolean isNumber(Object v) { return v instanceof Long || v instanceof Double || v instanceof Float; }

    static double number(Object v) {
        if (v instanceof Long l) return (double) (long) l;
        if (v instanceof Float f) return (double) (float) f;
        return (Double) v;
    }

    // An f32 is a `Float` (decision 41).
    static double fround(double x) { return (double) (float) x; }

    // An f32, or an int as a float and then the nearest f32 (41.2); null for anything else.
    static Double single(Object v) {
        if (v instanceof Float f) return (double) (float) f;
        if (v instanceof Long l) return fround((double) (long) l);
        return null;
    }

    // The float with the fewest digits that reads back as this f32.
    static double f32Shown(float f) {
        double x = f;
        if (Double.isNaN(x) || x == 0 || Double.isInfinite(x)) return x;
        var exact = new java.math.BigDecimal(x);
        for (int digits = 1; digits <= 9; digits++) {
            double d = exact.round(new java.math.MathContext(digits, java.math.RoundingMode.HALF_EVEN)).doubleValue();
            if (fround(d) == x) return d;
        }
        return x;
    }

    static boolean isF32(Object a, Object b) { return a instanceof Float || b instanceof Float; }

    static Object f32Op(String op, Object a, Object b, int line) {
        Double x = single(a), y = single(b);
        if (x == null || y == null) {
            if (a instanceof Double || b instanceof Double) {
                throw fatal("`" + op + "` between " + typeName(a) + " and " + typeName(b) + " would have to choose between 32 bits and 64. Write `f32( )` around the float to work in 32, or `float( )` around the f32 to work in 64", line);
            }
            throw mismatch(op, a, b, line);
        }
        double r;
        switch (op) {
            case "+": r = x + y; break;
            case "-": r = x - y; break;
            case "*": r = x * y; break;
            case "/": r = (Double) div(x, y, line); break;
            case "//": r = (Double) floordiv(x, y, line); break;
            case "%": r = (Double) mod(x, y, line); break;
            default: r = (Double) pow(x, y, line); break;
        }
        return (float) r;
    }

    static String typeName(Object v) {
        if (v instanceof Long) return "an int";
        if (v instanceof Double) return "a float";
        if (v instanceof Float) return "an f32";
        if (v instanceof String) return "a str";
        if (v instanceof Boolean) return "a bool";
        if (v == null) return "none";
        if (v instanceof Quantity q) return "a quantity in " + q.unit;
        if (v instanceof ZList) return "a list";
        if (v instanceof ZDict) return "a dict";
        if (v instanceof Func || v instanceof Method || v instanceof Bound) return "a function";
        if (v instanceof ZClass) return "a class";
        if (v instanceof Instance i) return "a " + i.cls.name;
        if (v instanceof ZEnum) return "an enum";
        if (v instanceof Constructor) return "an enum case";
        if (v instanceof Case c) return "a " + c.def.name;
        if (v instanceof ErrorType) return "an error type";
        if (v instanceof ZError e) return "a " + e.kind;
        if (v instanceof Module) return "a module";
        return "a value";
    }

    // `Point`, not `shapes.Point`; `Stack`, not `Stack[int]`.
    static String shownName(String name) {
        int bracket = name.indexOf('[');
        if (bracket >= 0) name = name.substring(0, bracket);
        int dot = name.lastIndexOf('.');
        return dot < 0 ? name : name.substring(dot + 1);
    }

    // -------------------------------------------------------------- Text

    // A str's characters are code points; Java's String counts UTF-16
    // units. Most text has no character outside the first 65536, and then
    // the two agree; the last str asked about is remembered.
    static String lastText = null;
    static boolean lastPlain = true;

    static boolean plain(String s) {
        if (s == lastText) return lastPlain;
        boolean p = true;
        for (int i = 0; i < s.length(); i++) {
            if (Character.isSurrogate(s.charAt(i))) { p = false; break; }
        }
        lastText = s;
        lastPlain = p;
        return p;
    }

    static int textLength(String s) { return plain(s) ? s.length() : s.codePointCount(0, s.length()); }

    static String sub(String s, int from, int to) {
        if (plain(s)) return s.substring(from, to);
        int a = s.offsetByCodePoints(0, from);
        int b = s.offsetByCodePoints(a, to - from);
        return s.substring(a, b);
    }

    // ------------------------------------------------------------ Showing

    // A float as Rust's `{:?}` shows it: the shortest digits that read
    // back as the same number, `.0` on a whole one, and an exponent for one
    // below 1e-4 or from 1e16 up.
    static String showFloat(double x) {
        if (Double.isNaN(x)) return "NaN";
        if (x == Double.POSITIVE_INFINITY) return "inf";
        if (x == Double.NEGATIVE_INFINITY) return "-inf";
        if (x == 0) return (1 / x < 0) ? "-0.0" : "0.0";
        // Java writes two digits at least, so the smallest floats there are
        // come out as 4.9E-324 where one digit, 5e-324, names the float.
        if (Math.abs(x) < 1e-320) {
            var one = new java.math.BigDecimal(Math.abs(x)).round(new java.math.MathContext(1, java.math.RoundingMode.HALF_EVEN));
            if (one.doubleValue() == Math.abs(x)) return (x < 0 ? "-" : "") + one.unscaledValue() + "e" + (-one.scale());
        }
        String s = Double.toString(x);
        String sign = "";
        if (s.charAt(0) == '-') { sign = "-"; s = s.substring(1); }
        int e = s.indexOf('E');
        String mantissa = e < 0 ? s : s.substring(0, e);
        int exp = e < 0 ? 0 : Integer.parseInt(s.substring(e + 1));
        int point = mantissa.indexOf('.');
        String whole = mantissa.substring(0, point), fraction = mantissa.substring(point + 1);
        String digits = whole + fraction;
        exp += whole.length() - 1;
        int lead = 0;
        while (lead < digits.length() - 1 && digits.charAt(lead) == '0') { lead++; exp--; }
        digits = digits.substring(lead);
        int end = digits.length();
        while (end > 1 && digits.charAt(end - 1) == '0') end--;
        digits = digits.substring(0, end);
        if (exp < -4 || exp >= 16) {
            return sign + digits.charAt(0) + (digits.length() > 1 ? "." + digits.substring(1) : "") + "e" + exp;
        }
        if (exp < 0) return sign + "0." + "0".repeat(-exp - 1) + digits;
        if (digits.length() <= exp + 1) return sign + digits + "0".repeat(exp + 1 - digits.length()) + ".0";
        return sign + digits.substring(0, exp + 1) + "." + digits.substring(exp + 1);
    }

    // A float with a fixed number of decimals, as Rust's `{:.2}` writes it:
    // from its exact value, a half rounding to the even digit.
    static String fixed(double x, int places) {
        if (Double.isNaN(x)) return "NaN";
        if (Double.isInfinite(x)) return x > 0 ? "inf" : "-inf";
        String s = new java.math.BigDecimal(x).setScale(places, java.math.RoundingMode.HALF_EVEN).toPlainString();
        if ((x < 0 || (x == 0 && 1 / x < 0)) && !s.startsWith("-")) s = "-" + s;
        return s;
    }

    // How Rust's `{:?}` shows a str: in quotes, with what cannot be seen escaped.
    static String quoted(String s) {
        StringBuilder b = new StringBuilder("\"");
        s.codePoints().forEach(c -> {
            if (c == '"') b.append("\\\"");
            else if (c == '\\') b.append("\\\\");
            else if (c == '\n') b.append("\\n");
            else if (c == '\r') b.append("\\r");
            else if (c == '\t') b.append("\\t");
            else if (c == 0) b.append("\\0");
            else if (c < 0x20 || (c >= 0x7f && c < 0xa0)) b.append("\\u{").append(Integer.toHexString(c)).append("}");
            else b.appendCodePoint(c);
        });
        return b.append('"').toString();
    }

    // How `print`, `str()` and f-strings show a value.
    static String show(Object v) {
        if (v instanceof String s) return s;
        if (v instanceof Long l) return Long.toString(l);
        if (v instanceof Double d) return showFloat(d);
        if (v instanceof Float f) return showFloat(f32Shown(f));
        if (v instanceof Boolean b) return b ? "true" : "false";
        if (v == null) return "none";
        if (v instanceof ZList l) {
            StringBuilder b = new StringBuilder("[");
            for (int i = 0; i < l.items.size(); i++) {
                if (i > 0) b.append(", ");
                b.append(repr(l.items.get(i)));
            }
            return b.append("]").toString();
        }
        if (v instanceof ZDict d) {
            if (d.map.isEmpty()) return "[:]";
            StringBuilder b = new StringBuilder("[");
            boolean first = true;
            for (var entry : d.map.entrySet()) {
                if (!first) b.append(", ");
                first = false;
                b.append(repr(entry.getKey())).append(": ").append(repr(entry.getValue()));
            }
            return b.append("]").toString();
        }
        if (v instanceof Quantity q) {
            if (!Double.isNaN(q.v) && !Double.isInfinite(q.v) && q.v == Math.floor(q.v) && Math.abs(q.v) < 1e15) {
                return (long) q.v + " " + q.unit;
            }
            return showFloat(q.v) + " " + q.unit;
        }
        if (v instanceof Instance o) {
            StringBuilder b = new StringBuilder(shownName(o.cls.name)).append("(");
            for (int i = 0; i < o.cls.fields.length; i++) {
                if (i > 0) b.append(", ");
                b.append(o.cls.fields[i]).append("=").append(repr(o.fields[i]));
            }
            return b.append(")").toString();
        }
        if (v instanceof Case c) {
            String name = shownName(c.def.cases[c.index]);
            if (c.fields.length == 0) return name;
            StringBuilder b = new StringBuilder(name).append("(");
            for (int i = 0; i < c.fields.length; i++) {
                if (i > 0) b.append(", ");
                b.append(repr(c.fields[i]));
            }
            return b.append(")").toString();
        }
        if (v instanceof ZError e) return e.message;
        if (v instanceof Method m) return "<function " + m.name + ">";
        if (v instanceof Bound b) return "<function " + b.fn.name + ">";
        if (v instanceof ZClass c) return "<class " + c.name + ">";
        if (v instanceof ZEnum e) return "<enum " + e.name + ">";
        if (v instanceof Constructor c) return "<case " + c.def.cases[c.index] + ">";
        if (v instanceof ErrorType e) return "<error " + e.kind + ">";
        if (v instanceof Module m) return "<module " + m.name + ">";
        if (v instanceof Func f) return f.name.isEmpty() ? "<function>" : "<function " + f.name + ">";
        return "?";
    }

    static String repr(Object v) { return v instanceof String s ? quoted(s) : show(v); }

    // -------------------------------------------------------------- Units

    record UnitInfo(String kind, double size) {}

    static final java.util.HashMap<String, UnitInfo> UNITS = new java.util.HashMap<>();

    static void units(String kind, Object... pairs) {
        for (int i = 0; i < pairs.length; i += 2) UNITS.put((String) pairs[i], new UnitInfo(kind, ((Number) pairs[i + 1]).doubleValue()));
    }

    static {
        units("time", "ns", 1, "us", 1e3, "ms", 1e6, "s", 1e9, "min", 60e9, "h", 3600e9);
        units("frequency", "Hz", 1, "kHz", 1e3, "MHz", 1e6);
        units("voltage", "uV", 1, "mV", 1e3, "V", 1e6, "kV", 1e9);
        units("current", "uA", 1, "mA", 1e3, "A", 1e6);
        units("resistance", "ohm", 1, "kohm", 1e3, "Mohm", 1e6);
        units("power", "mW", 1, "W", 1e3, "kW", 1e6);
        units("energy", "J", 1, "Wh", 3600, "kWh", 3600e3);
        units("temperature in Celsius", "degC", 1);
        units("temperature in kelvin", "K", 1);
        units("length", "mm", 1, "cm", 10, "m", 1e3, "km", 1e6);
        units("mass", "mg", 1, "g", 1e3, "kg", 1e6);
        units("angle in degrees", "deg", 1);
        units("angle in radians", "rad", 1);
        units("rotation", "rpm", 1);
    }

    // How to turn a number in one unit into another: {times, over}, or null.
    static double[] convert(String from, String to) {
        if (from.equals("deg") && to.equals("rad")) return new double[] {Math.PI / 180, 1};
        if (from.equals("rad") && to.equals("deg")) return new double[] {180 / Math.PI, 1};
        UnitInfo f = UNITS.get(from), t = UNITS.get(to);
        if (f == null || t == null || !f.kind.equals(t.kind)) return null;
        return f.size >= t.size ? new double[] {f.size / t.size, 1} : new double[] {1, t.size / f.size};
    }

    static double apply(double[] scale, double x) { return x * scale[0] / scale[1]; }

    // The unit two quantities meet in, the finer one, or null.
    static String common(String a, String b) {
        UnitInfo ua = UNITS.get(a), ub = UNITS.get(b);
        if (ua == null || ub == null || !ua.kind.equals(ub.kind)) return null;
        return ua.size <= ub.size ? a : b;
    }

    static Object unit(Object v, String name, int line) {
        if (!UNITS.containsKey(name)) {
            throw fatal("`" + name + "` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC", line);
        }
        if (!isNumber(v)) throw fatal("a unit can only follow a number", line);
        return new Quantity(number(v), name);
    }

    // ------------------------------------------------------------ Equality

    static boolean equal(Object a, Object b) {
        if (a instanceof Long x && b instanceof Long y) return (long) x == (long) y;
        if (isF32(a, b)) {
            Double x = single(a), y = single(b);
            if (x != null && y != null) return (double) x == (double) y;
            return isNumber(a) && isNumber(b) && number(a) == number(b);
        }
        if (isNumber(a) && isNumber(b)) return number(a) == number(b);
        if (a instanceof String x && b instanceof String y) return x.equals(y);
        if (a instanceof Boolean x && b instanceof Boolean y) return x.equals(y);
        if (a == null || b == null) return a == b;
        if (a instanceof Quantity x && b instanceof Quantity y) {
            String u = common(x.unit, y.unit);
            return u != null && apply(convert(x.unit, u), x.v) == apply(convert(y.unit, u), y.v);
        }
        if (a instanceof ZList x && b instanceof ZList y) {
            if (x.items.size() != y.items.size()) return false;
            for (int i = 0; i < x.items.size(); i++) if (!equal(x.items.get(i), y.items.get(i))) return false;
            return true;
        }
        if (a instanceof ZDict x && b instanceof ZDict y) {
            if (x.map.size() != y.map.size()) return false;
            for (var entry : x.map.entrySet()) {
                if (!y.map.containsKey(entry.getKey()) || !equal(entry.getValue(), y.map.get(entry.getKey()))) return false;
            }
            return true;
        }
        if (a instanceof Instance x && b instanceof Instance y) {
            if (x.cls != y.cls) return false;
            for (int i = 0; i < x.fields.length; i++) if (!equal(x.fields[i], y.fields[i])) return false;
            return true;
        }
        if (a instanceof Case x && b instanceof Case y) {
            if (x.def != y.def || x.index != y.index || x.fields.length != y.fields.length) return false;
            for (int i = 0; i < x.fields.length; i++) if (!equal(x.fields[i], y.fields[i])) return false;
            return true;
        }
        if (a instanceof ZError x && b instanceof ZError y) return x.kind.equals(y.kind) && x.message.equals(y.message);
        if (a instanceof Constructor x && b instanceof Constructor y) return x.def == y.def && x.index == y.index;
        return a == b;
    }

    static boolean eq(Object a, Object b) { return equal(a, b); }

    static boolean ne(Object a, Object b) { return !equal(a, b); }

    // ----------------------------------------------------------- Operators

    static Fatal mismatch(String op, Object a, Object b, int line) {
        return fatal("`" + op + "` does not work between " + typeName(a) + " and " + typeName(b), line);
    }

    static long addInt(long a, long b, int line) {
        long r = a + b;
        if (((a ^ r) & (b ^ r)) < 0) throw overflow(line);
        return r;
    }

    static long mulInt(long a, long b, int line) {
        long hi = Math.multiplyHigh(a, b), lo = a * b;
        if ((hi != 0 || lo < 0) && (hi != -1 || lo >= 0)) throw overflow(line);
        return lo;
    }

    static Object add(Object a, Object b, int line) {
        if (isF32(a, b)) return f32Op("+", a, b, line);
        if (a instanceof Long x && b instanceof Long y) return addInt(x, y, line);
        if (a instanceof String x && b instanceof String y) return x.concat(y);
        if (a instanceof ZList x && b instanceof ZList y) {
            var items = new java.util.ArrayList<Object>(x.items.size() + y.items.size());
            items.addAll(x.items);
            items.addAll(y.items);
            return new ZList(items);
        }
        if (a instanceof Quantity x && b instanceof Quantity y) {
            String u = common(x.unit, y.unit);
            if (u == null) throw mismatch("+", a, b, line);
            return new Quantity(apply(convert(x.unit, u), x.v) + apply(convert(y.unit, u), y.v), u);
        }
        if (!isNumber(a) || !isNumber(b)) throw mismatch("+", a, b, line);
        return number(a) + number(b);
    }

    static Object sub(Object a, Object b, int line) {
        if (isF32(a, b)) return f32Op("-", a, b, line);
        if (a instanceof Long x && b instanceof Long y) {
            long r = x - y;
            if (((x ^ y) & (x ^ r)) < 0) throw overflow(line);
            return r;
        }
        if (a instanceof Quantity x && b instanceof Quantity y) {
            String u = common(x.unit, y.unit);
            if (u == null) throw mismatch("-", a, b, line);
            return new Quantity(apply(convert(x.unit, u), x.v) - apply(convert(y.unit, u), y.v), u);
        }
        if (!isNumber(a) || !isNumber(b)) throw mismatch("-", a, b, line);
        return number(a) - number(b);
    }

    static Object mul(Object a, Object b, int line) {
        if (isF32(a, b)) return f32Op("*", a, b, line);
        if (a instanceof ZList list && b instanceof Long times) {
            if (times < 0) throw raiseError("ValueError", "`*` repeats a list 0 or more times, and this is " + times, line);
            var items = new java.util.ArrayList<Object>();
            for (long t = 0; t < times; t++) items.addAll(list.items);
            return new ZList(items);
        }
        if (a instanceof Long x && b instanceof Long y) return mulInt(x, y, line);
        if (a instanceof Quantity || b instanceof Quantity) {
            Quantity q = a instanceof Quantity qa ? qa : (Quantity) b;
            Object other = a instanceof Quantity ? b : a;
            if (!isNumber(other)) throw mismatch("*", a, b, line);
            return new Quantity(q.v * number(other), q.unit);
        }
        if (!isNumber(a) || !isNumber(b)) throw mismatch("*", a, b, line);
        return number(a) * number(b);
    }

    static Object div(Object a, Object b, int line) {
        if (isF32(a, b)) return f32Op("/", a, b, line);
        if (a instanceof Quantity q) {
            if (b instanceof Quantity y && y.v == 0) throw byZero(line);
            if (b instanceof Double y && y == 0) throw byZero(line);
            if (b instanceof Long y && y == 0) throw byZero(line);
            if (b instanceof Quantity y) {
                String u = common(q.unit, y.unit);
                if (u == null) throw mismatch("/", a, b, line);
                return apply(convert(q.unit, u), q.v) / apply(convert(y.unit, u), y.v);
            }
            if (!isNumber(b)) throw mismatch("/", a, b, line);
            return new Quantity(q.v / number(b), q.unit);
        }
        if (!isNumber(a) || !isNumber(b)) throw mismatch("/", a, b, line);
        if (number(b) == 0) throw byZero(line);
        return number(a) / number(b);
    }

    static Object floordiv(Object a, Object b, int line) {
        if (isF32(a, b)) return f32Op("//", a, b, line);
        if (a instanceof Long x && b instanceof Long y) {
            if (y == 0) throw byZero(line);
            if (x == Long.MIN_VALUE && y == -1) throw overflow(line);
            return Math.floorDiv((long) x, (long) y);
        }
        if (!isNumber(a) || !isNumber(b)) throw mismatch("//", a, b, line);
        if (number(b) == 0) throw byZero(line);
        return Math.floor(number(a) / number(b));
    }

    static Object mod(Object a, Object b, int line) {
        if (isF32(a, b)) return f32Op("%", a, b, line);
        if (a instanceof Long x && b instanceof Long y) {
            if (y == 0) throw byZero(line);
            return Math.floorMod((long) x, (long) y);
        }
        if (!isNumber(a) || !isNumber(b)) throw mismatch("%", a, b, line);
        double x = number(a), y = number(b);
        if (y == 0) throw byZero(line);
        double q = x / y;
        if (Double.isNaN(q) || Double.isInfinite(q)) return x - y * q;
        return x - y * Math.floor(q);
    }

    static Object pow(Object a, Object b, int line) {
        if (isF32(a, b)) return f32Op("**", a, b, line);
        if (a instanceof Long xl && b instanceof Long nl) {
            long x = xl, n = nl;
            if (n < 0) throw raiseError("ValueError", "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`", line);
            if (n > 4294967295L) throw overflow(line);
            if (x == 0 || x == 1) return n == 0 ? 1L : x;
            if (x == -1) return n % 2 == 0 ? 1L : -1L;
            if (n >= 64) throw overflow(line);
            long r = 1;
            for (long i = 0; i < n; i++) r = mulInt(r, x, line);
            return r;
        }
        if (!isNumber(a) || !isNumber(b)) throw mismatch("**", a, b, line);
        double x = number(a), y = number(b);
        if (x == 1 || y == 0) return 1.0;
        return Math.pow(x, y);
    }

    static Fatal bitsMismatch(String op, Object a, Object b, int line) {
        String hint = "";
        if (a instanceof Boolean && b instanceof Boolean) {
            if (op.equals("&")) hint = ". For two bools Zeno spells this `and`";
            if (op.equals("|")) hint = ". For two bools Zeno spells this `or`";
        }
        return fatal("`" + op + "` works on the bits of whole numbers, and these are " + typeName(a) + " and " + typeName(b) + hint, line);
    }

    static long whole(String op, Object v, Object a, Object b, int line) {
        if (!(v instanceof Long l)) throw bitsMismatch(op, a, b, line);
        return l;
    }

    static Object bitand(Object a, Object b, int line) { return whole("&", a, a, b, line) & whole("&", b, a, b, line); }

    static Object bitor(Object a, Object b, int line) { return whole("|", a, a, b, line) | whole("|", b, a, b, line); }

    static Object bitxor(Object a, Object b, int line) { return whole("^", a, a, b, line) ^ whole("^", b, a, b, line); }

    static void shift(long n, int line) {
        if (n < 0 || n >= 64) throw raiseError("ValueError", "cannot shift by " + n + ". An int has 64 bits, so a shift is 0 to 63", line);
    }

    static Object shl(Object a, Object b, int line) {
        long x = whole("<<", a, a, b, line), n = whole("<<", b, a, b, line);
        shift(n, line);
        long s = x << n;
        if (s >> n != x) throw overflow(line);
        return s;
    }

    static Object shr(Object a, Object b, int line) {
        long x = whole(">>", a, a, b, line), n = whole(">>", b, a, b, line);
        shift(n, line);
        return x >> n;
    }

    static Object neg(Object v, int line) {
        if (v instanceof Long x) {
            if (x == Long.MIN_VALUE) throw overflow(line);
            return -x;
        }
        if (v instanceof Double x) return -x;
        if (v instanceof Float x) return -x;
        if (v instanceof Quantity q) return new Quantity(-q.v, q.unit);
        throw fatal("`-` does not work on " + typeName(v), line);
    }

    static Object bitnot(Object v, int line) {
        if (!(v instanceof Long x)) throw fatal("`~` works on the bits of a whole number, and this is " + typeName(v), line);
        return ~x;
    }

    // -1, 0 or 1, or 2 when the two cannot be ordered (a NaN).
    static int order(String op, Object a, Object b, int line) {
        if (isF32(a, b)) {
            Double x = single(a), y = single(b);
            if (x == null || y == null) f32Op(op, a, b, line);
            return x < y ? -1 : x > y ? 1 : x.doubleValue() == y.doubleValue() ? 0 : 2;
        }
        if (a instanceof Long x && b instanceof Long y) return Long.compare(x, y);
        if (a instanceof String x && b instanceof String y) {
            // By code points, as Rust compares, not by UTF-16 units.
            int i = 0, j = 0;
            while (i < x.length() && j < y.length()) {
                int c = x.codePointAt(i), d = y.codePointAt(j);
                if (c != d) return c < d ? -1 : 1;
                i += Character.charCount(c);
                j += Character.charCount(d);
            }
            return i < x.length() ? 1 : j < y.length() ? -1 : 0;
        }
        double x, y;
        if (a instanceof Quantity qa && b instanceof Quantity qb) {
            String u = common(qa.unit, qb.unit);
            if (u == null) throw mismatch(op, a, b, line);
            x = apply(convert(qa.unit, u), qa.v);
            y = apply(convert(qb.unit, u), qb.v);
        } else {
            if (!isNumber(a) || !isNumber(b)) throw mismatch(op, a, b, line);
            x = number(a);
            y = number(b);
        }
        if (x < y) return -1;
        if (x > y) return 1;
        if (x == y) return 0;
        return 2;
    }

    static boolean lt(Object a, Object b, int line) { return order("<", a, b, line) == -1; }

    static boolean le(Object a, Object b, int line) { int o = order("<=", a, b, line); return o == -1 || o == 0; }

    static boolean gt(Object a, Object b, int line) { return order(">", a, b, line) == 1; }

    static boolean ge(Object a, Object b, int line) { int o = order(">=", a, b, line); return o == 1 || o == 0; }

    static boolean contains(String op, Object item, Object container, int line) {
        if (container instanceof ZList l) {
            for (Object other : l.items) if (equal(item, other)) return true;
            return false;
        }
        if (container instanceof ZDict d) {
            if (!(item instanceof Long) && !(item instanceof String)) throw mismatch(op, item, container, line);
            return d.map.containsKey(item);
        }
        if (container instanceof String c && item instanceof String s) return c.contains(s);
        throw mismatch(op, item, container, line);
    }

    static boolean in(Object a, Object b, int line) { return contains("in", a, b, line); }

    static boolean not_in(Object a, Object b, int line) { return !contains("not in", a, b, line); }

    static boolean truth(Object v, int line) {
        if (v instanceof Boolean b) return b;
        throw fatal("a condition must be true or false, but this is " + typeName(v), line);
    }

    // ------------------------------------------------------ Lists and text

    static Object key(Object v, int line) {
        if (v instanceof Long || v instanceof String) return v;
        throw fatal("a key of a dict is a str or an int, but this is " + typeName(v), line);
    }

    static int position(Object index, int length, String what, int line) {
        if (!(index instanceof Long i)) throw fatal("an index must be an int, but this is " + typeName(index), line);
        if (i < 0 || i >= length) throw raiseError("IndexError", "index " + i + " is out of range for a " + what + " of length " + length, line);
        return (int) (long) i;
    }

    static Object index(Object obj, Object i, int line) {
        if (obj instanceof ZList l) return l.items.get(position(i, l.items.size(), "list", line));
        if (obj instanceof String s) {
            int at = position(i, textLength(s), "str", line);
            return sub(s, at, at + 1);
        }
        if (obj instanceof ZDict d) {
            Object k = key(i, line);
            Object v = d.map.get(k);
            if (v == null && !d.map.containsKey(k)) throw raiseError("KeyError", "this dict has no key " + repr(i), line);
            return v;
        }
        throw fatal("cannot take an item from " + typeName(obj), line);
    }

    static void set_index(Object obj, Object i, Object value, int line) {
        if (obj instanceof ZList l) { l.items.set(position(i, l.items.size(), "list", line), value); return; }
        if (obj instanceof ZDict d) { d.map.put(key(i, line), value); return; }
        throw fatal("cannot set an item of " + typeName(obj), line);
    }

    static java.util.ArrayList<Object> chars(String s) {
        var out = new java.util.ArrayList<Object>(s.length());
        s.codePoints().forEach(c -> out.add(new String(Character.toChars(c))));
        return out;
    }

    static java.util.ArrayList<Object> items(Object v, int line) {
        if (v instanceof ZList l) return new java.util.ArrayList<>(l.items);
        if (v instanceof ZDict d) return new java.util.ArrayList<>(d.map.keySet());
        if (v instanceof String s) return chars(s);
        throw fatal("cannot loop over " + typeName(v), line);
    }

    static java.util.ArrayList<Object[]> pairs(Object v, int line) {
        var out = new java.util.ArrayList<Object[]>();
        if (v instanceof ZDict d) {
            for (var entry : d.map.entrySet()) out.add(new Object[] {entry.getKey(), entry.getValue()});
            return out;
        }
        var all = items(v, line);
        for (int i = 0; i < all.size(); i++) out.add(new Object[] {(long) i, all.get(i)});
        return out;
    }

    static Object list(Object[] values) { return new ZList(new java.util.ArrayList<>(java.util.Arrays.asList(values))); }

    static Object dict(Object[] flat, int line) {
        ZDict d = new ZDict();
        for (int i = 0; i < flat.length; i += 2) d.map.put(key(flat[i], line), flat[i + 1]);
        return d;
    }

    static Object push(Object list, Object value) {
        ((ZList) list).items.add(value);
        return null;
    }

    static Object concat(Object[] pieces) {
        StringBuilder b = new StringBuilder();
        for (Object p : pieces) b.append((String) p);
        return b.toString();
    }

    // ------------------------------------------------- Objects and fields

    static Func fn(String name, String[] params, boolean[] defaults, Body body) {
        return new Func(name, params, defaults, body, null, null);
    }

    static java.util.HashMap<String, Func> methods(Object... pairs) {
        var out = new java.util.HashMap<String, Func>();
        for (int i = 0; i < pairs.length; i += 2) out.put((String) pairs[i], (Func) pairs[i + 1]);
        return out;
    }

    static Object cls(String name, String[] fields, Object[] defaults, java.util.HashMap<String, Func> methods) {
        return new ZClass(name, fields, defaults, methods);
    }

    static Object enumOf(String name, String[] cases, String[][] fields) { return new ZEnum(name, cases, fields); }

    static Object case_(Object def, int i) {
        ZEnum e = (ZEnum) def;
        return e.fields[i].length == 0 ? new Case(e, i, new Object[0]) : new Constructor(e, i);
    }

    static boolean is_case(Object value, Object ctor, int count, int line) {
        if (!(ctor instanceof Constructor c)) throw fatal("this is not an enum case that carries values", line);
        if (!(value instanceof Case v) || v.def != c.def || v.index != c.index) return false;
        if (v.fields.length != count) {
            throw fatal("`" + c.def.cases[c.index] + "` carries " + v.fields.length + " values, but this pattern has " + count, line);
        }
        return true;
    }

    static Object case_field(Object value, int i) { return ((Case) value).fields[i]; }

    static int indexOf(String[] names, String name) {
        for (int i = 0; i < names.length; i++) if (names[i].equals(name)) return i;
        return -1;
    }

    static Object field(Object obj, String name, int line) {
        if (obj instanceof Instance o) {
            int at = indexOf(o.cls.fields, name);
            if (at >= 0) return o.fields[at];
            Func m = o.cls.methods.get(name);
            if (m != null) return new Bound(o, m);
            throw fatal("`" + o.cls.name + "` has no field or method `" + name + "`", line);
        }
        if (obj instanceof Case c) {
            int at = indexOf(c.def.fields[c.index], name);
            if (at >= 0) return c.fields[at];
            throw fatal("`" + c.def.cases[c.index] + "` has no field `" + name + "`", line);
        }
        if (obj instanceof ZEnum e) {
            int at = indexOf(e.cases, name);
            if (at >= 0) return case_(e, at);
            throw fatal("enum `" + e.name + "` has no case `" + name + "`", line);
        }
        if (obj instanceof Module m) return builtin(m.name + "." + name, line);
        if (obj instanceof Quantity q && UNITS.containsKey(name)) {
            double[] s = convert(q.unit, name);
            if (s == null) throw fatal("a quantity in " + q.unit + " cannot be turned into " + name + ". They measure different things", line);
            return new Quantity(apply(s, q.v), name);
        }
        if (obj instanceof ZError e && name.equals("message")) return e.message;
        if (obj instanceof ZError e && name.equals("kind")) return e.kind;
        if (obj instanceof Func f && f.builtin != null && WIDTHS.containsKey(f.builtin) && name.equals("wrap")) {
            return builtin(f.builtin + ".wrap", line);
        }
        if (hasMethod(obj, name)) return new Method(obj, name);
        throw fatal(typeName(obj) + " has no field or method `" + name + "`", line);
    }

    static void set_field(Object obj, String name, Object value, int line) {
        if (obj instanceof Instance o) {
            int at = indexOf(o.cls.fields, name);
            if (at < 0) throw fatal("`" + o.cls.name + "` has no field `" + name + "`", line);
            o.fields[at] = value;
            return;
        }
        throw fatal("cannot set a field on " + typeName(obj), line);
    }

    // -------------------------------------------------------------- Calls

    static String counted(int n, String word) { return n == 1 ? "1 " + word : n + " " + word + "s"; }

    static String were(int n) { return n == 1 ? "1 was" : n + " were"; }

    static Object[] bind(String what, String[] names, Object[] positional, String[] namedNames, Object[] named, int line) {
        if (positional.length > names.length) {
            throw fatal("`" + what + "` takes " + counted(names.length, "argument") + ", but " + were(positional.length) + " given", line);
        }
        Object[] slots = new Object[names.length];
        for (int i = 0; i < names.length; i++) slots[i] = i < positional.length ? positional[i] : MISSING;
        for (int i = 0; i < namedNames.length; i++) {
            int at = indexOf(names, namedNames[i]);
            if (at < 0) throw fatal("`" + what + "` has no argument named `" + namedNames[i] + "`", line);
            if (slots[at] != MISSING) throw fatal("`" + what + "` was given `" + namedNames[i] + "` twice", line);
            slots[at] = named[i];
        }
        return slots;
    }

    // How many calls are nested now. `zeno run` stops at 1000.
    static int depth = 0;

    static void enter(Func f, int line) {
        if (depth >= 1000) {
            String name = f.name.isEmpty() ? "function" : f.name;
            throw fatal("more than 1000 calls are nested. Is `" + name + "` calling itself forever?", line);
        }
        depth++;
    }

    // A call of a function of the program known where it is written, with
    // its arguments lined up already.
    static Object direct(Object f, int line, Object... args) {
        Func fn = (Func) f;
        enter(fn, line);
        try {
            return fn.body.call(args);
        } finally {
            depth--;
        }
    }

    static final String[] NO_NAMES = new String[0];
    static final Object[] NO_VALUES = new Object[0];

    static Object callFunction(Func f, Object[] positional, String[] namedNames, Object[] named, int line) {
        String name = f.name.isEmpty() ? "function" : f.name;
        enter(f, line);
        try {
            Object[] slots = bind(name, f.params, positional, namedNames, named, line);
            for (int i = 0; i < slots.length; i++) {
                if (slots[i] == MISSING && !f.defaults[i]) throw fatal("`" + name + "` needs a value for `" + f.params[i] + "`", line);
            }
            return f.body.call(slots);
        } finally {
            depth--;
        }
    }

    static void noNamed(String what, String[] namedNames, int line) {
        if (namedNames.length > 0) throw fatal("`" + what + "` does not take a named argument `" + namedNames[0] + "`", line);
    }

    static Object[] prepend(Object first, Object[] rest) {
        Object[] out = new Object[rest.length + 1];
        out[0] = first;
        System.arraycopy(rest, 0, out, 1, rest.length);
        return out;
    }

    static Object call_named(Object callee, Object[] positional, String[] namedNames, Object[] named, int line) {
        if (callee instanceof Bound b) return callFunction(b.fn, prepend(b.object, positional), namedNames, named, line);
        if (callee instanceof Method m) {
            noNamed(m.name, namedNames, line);
            return callMethodOf(m.receiver, m.name, positional, line);
        }
        if (callee instanceof ZClass c) {
            Object[] slots = bind(c.name, c.fields, positional, namedNames, named, line);
            for (int i = 0; i < slots.length; i++) {
                if (slots[i] == MISSING) {
                    if (c.defaults[i] == null) throw fatal("`" + c.name + "` needs a value for `" + c.fields[i] + "`", line);
                    slots[i] = ((Func) c.defaults[i]).body.call(NO_VALUES);
                }
            }
            return new Instance(c, slots);
        }
        if (callee instanceof Constructor c) {
            String name = c.def.cases[c.index];
            String[] fields = c.def.fields[c.index];
            Object[] slots = bind(name, fields, positional, namedNames, named, line);
            for (int i = 0; i < slots.length; i++) {
                if (slots[i] == MISSING) throw fatal("`" + name + "` needs a value for `" + fields[i] + "`", line);
            }
            return new Case(c.def, c.index, slots);
        }
        if (callee instanceof ErrorType e) {
            noNamed(e.kind, namedNames, line);
            return newError(e.kind, positional, line);
        }
        if (callee instanceof Func f) {
            if (f.bbody != null) {
                noNamed(f.name, namedNames, line);
                return f.bbody.call(positional, line);
            }
            return callFunction(f, positional, namedNames, named, line);
        }
        throw fatal(typeName(callee) + " cannot be called", line);
    }

    static Object call(Object callee, Object[] args, int line) { return call_named(callee, args, NO_NAMES, NO_VALUES, line); }

    static Object newError(String kind, Object[] args, int line) {
        if (args.length > 1) throw fatal("`" + kind + "` takes one message", line);
        return new ZError(kind, args.length == 1 ? show(args[0]) : "", null);
    }

    static Object call_method_named(Object obj, String name, Object[] positional, String[] namedNames, Object[] named, int line) {
        if (obj instanceof Instance o && indexOf(o.cls.fields, name) < 0) {
            Func m = o.cls.methods.get(name);
            if (m != null) return callFunction(m, prepend(o, positional), namedNames, named, line);
        }
        if (obj instanceof Module m) {
            noNamed(name, namedNames, line);
            return call_builtin(m.name + "." + name, positional, line);
        }
        if (obj instanceof String || obj instanceof ZList || obj instanceof ZDict) {
            noNamed(name, namedNames, line);
            return callMethodOf(obj, name, positional, line);
        }
        return call_named(field(obj, name, line), positional, namedNames, named, line);
    }

    static Object call_method(Object obj, String name, Object[] args, int line) {
        return call_method_named(obj, name, args, NO_NAMES, NO_VALUES, line);
    }

    // ------------------------------------------------------------- Errors

    static void raise(Object value, int line) {
        if (value instanceof ZError e) throw new Raise(e, line);
        if (value instanceof Instance o) throw new Raise(new ZError(o.cls.name, show(o), o), line);
        throw fatal("only an error can be raised, and this is " + typeName(value), line);
    }

    static boolean catches(Raise e, String name) { return name.equals("Error") || name.equals(e.value.kind); }

    static Object caught(Raise e, String name) {
        if (!name.isEmpty() && !name.equals("Error") && e.value.object != null) return e.value.object;
        return e.value;
    }

    static void no_match(Object v, int line) { throw fatal("no `case` matches " + repr(v), line); }

    static Object undefined(String name, int line) { throw fatal("`" + name + "` is not defined", line); }

    // An expression whose value is not wanted.
    static void ignore(Object v) {}

    // ------------------------------------------------- Built-in functions

    static void arity(String name, Object[] args, int lo, int hi, int line) {
        if (args.length >= lo && args.length <= hi) return;
        String wanted = lo == hi ? counted(lo, "argument") : lo + " to " + hi + " arguments";
        throw fatal("`" + name + "` takes " + wanted + ", but " + were(args.length) + " given", line);
    }

    static Fatal wrongType(String name, String wanted, Object got, int line) {
        return fatal("`" + name + "` needs " + wanted + ", but this is " + typeName(got), line);
    }

    static String text(String name, Object v, int line) {
        if (!(v instanceof String s)) throw wrongType(name, "a str", v, line);
        return s;
    }

    static long wholeArg(String name, Object v, int line) {
        if (!(v instanceof Long n)) throw wrongType(name, "a whole number", v, line);
        return n;
    }

    static boolean blank(int c) { return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\f'; }

    static String strip(String s) {
        int a = 0, b = s.length();
        while (a < b && blank(s.charAt(a))) a++;
        while (b > a && blank(s.charAt(b - 1))) b--;
        return s.substring(a, b);
    }

    static final java.io.PrintStream OUT =
        new java.io.PrintStream(new java.io.BufferedOutputStream(new java.io.FileOutputStream(java.io.FileDescriptor.out), 65536), false, java.nio.charset.StandardCharsets.UTF_8);

    static void flush() { OUT.flush(); }

    record Width(long lo, long hi, int bits, boolean signed) {}

    static final java.util.HashMap<String, Width> WIDTHS = new java.util.HashMap<>();

    static {
        WIDTHS.put("u8", new Width(0, 255, 8, false));
        WIDTHS.put("u16", new Width(0, 65535, 16, false));
        WIDTHS.put("u32", new Width(0, 4294967295L, 32, false));
        WIDTHS.put("i8", new Width(-128, 127, 8, true));
        WIDTHS.put("i16", new Width(-32768, 32767, 16, true));
        WIDTHS.put("i32", new Width(-2147483648L, 2147483647L, 32, true));
    }

    static double amount(Object v) { return v instanceof Quantity q ? q.v : number(v); }

    static java.util.List<Object> numbers(String name, Object[] args, int line, String[] unitOut) {
        java.util.List<Object> values = args.length == 1 && args[0] instanceof ZList l ? l.items : java.util.Arrays.asList(args);
        String unit = null;
        for (int place = 0; place < values.size(); place++) {
            Object item = values.get(place);
            if (item instanceof Quantity q) {
                if (unit != null && !unit.equals(q.unit)) {
                    throw fatal("`" + name + "` needs quantities in one unit, but these are in " + unit + " and in " + q.unit + ". Turn one into the other first, as in `x." + unit + "`", line);
                }
                if (unit == null && place > 0) throw fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line);
                unit = q.unit;
            } else if (!isNumber(item)) {
                throw wrongType(name, "numbers", item, line);
            } else if (unit != null) {
                throw fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line);
            }
        }
        unitOut[0] = unit;
        return values;
    }

    static Object pick(String name, Object[] args, int line, boolean smaller) {
        var values = numbers(name, args, line, new String[1]);
        if (values.isEmpty()) throw raiseError("ValueError", "`" + name + "` of nothing", line);
        if (values.stream().anyMatch(v -> v instanceof Float) && values.stream().allMatch(v -> v instanceof Float || v instanceof Long)) {
            var widened = new java.util.ArrayList<Object>();
            for (Object v : values) widened.add((float) (double) single(v));
            values = widened;
        }
        Object best = values.get(0);
        for (int i = 1; i < values.size(); i++) {
            Object item = values.get(i);
            if (smaller ? amount(item) < amount(best) : amount(item) > amount(best)) best = item;
        }
        return best;
    }

    static final java.util.regex.Pattern FLOAT = java.util.regex.Pattern.compile(
        "[+-]?([0-9]+\\.?[0-9]*([eE][+-]?[0-9]+)?|\\.[0-9]+([eE][+-]?[0-9]+)?|(?i:inf|infinity|nan))");

    // A float as Rust's `parse::<f64>` reads it, or null.
    static Double parseFloat(String s) {
        if (!FLOAT.matcher(s).matches()) return null;
        String body = s.replaceFirst("^[+-]", "").toLowerCase();
        boolean negative = s.startsWith("-");
        if (body.equals("inf") || body.equals("infinity")) return negative ? Double.NEGATIVE_INFINITY : Double.POSITIVE_INFINITY;
        if (body.equals("nan")) return Double.NaN;
        return Double.parseDouble(s);
    }

    interface Before { boolean test(int i, int j); }

    interface Swap { void swap(int i, int j); }

    // Heapsort, step for step as the Rust code sorts, so that a function
    // given to `sort` is called in the same order.
    static void heapsort(int n, Before before, Swap swap) {
        for (int root = n / 2 - 1; root >= 0; root--) sift(root, n, before, swap);
        for (int end = n - 1; end > 0; end--) {
            swap.swap(0, end);
            sift(0, end, before, swap);
        }
    }

    static void sift(int root, int end, Before before, Swap swap) {
        while (true) {
            int child = 2 * root + 1;
            if (child >= end) return;
            if (child + 1 < end && before.test(child, child + 1)) child++;
            if (!before.test(root, child)) return;
            swap.swap(root, child);
            root = child;
        }
    }

    static void sortList(ZList l, int line) {
        var s = new java.util.ArrayList<Object>(l.items);
        heapsort(s.size(), (i, j) -> lt(s.get(i), s.get(j), line), (i, j) -> java.util.Collections.swap(s, i, j));
        l.items = s;
    }

    static void sortBy(ZList l, Object function, int line) {
        var keys = new java.util.ArrayList<Object>();
        for (Object item : new java.util.ArrayList<>(l.items)) {
            Object k = call(function, new Object[] {item}, line);
            if (!(k instanceof Long) && !(k instanceof Double) && !(k instanceof Float) && !(k instanceof String)) {
                throw fatal("the function given to `sort` must return a number or text to order by, but it returned " + typeName(k), line);
            }
            keys.add(k);
        }
        var s = new java.util.ArrayList<Object>(l.items);
        if (s.size() != keys.size()) throw raiseError("ValueError", "the function given to `sort` changed how many items the list has", line);
        int[] places = new int[s.size()];
        for (int i = 0; i < places.length; i++) places[i] = i;
        heapsort(s.size(), (a, b) -> {
            if (lt(keys.get(a), keys.get(b), line)) return true;
            if (lt(keys.get(b), keys.get(a), line)) return false;
            return places[a] < places[b];
        }, (i, j) -> {
            java.util.Collections.swap(s, i, j);
            java.util.Collections.swap(keys, i, j);
            int t = places[i];
            places[i] = places[j];
            places[j] = t;
        });
        l.items = s;
    }

    static int[] slice(Object[] args, int length, String what, int line) {
        long from = wholeArg("slice", args[0], line);
        long to = args.length > 1 ? wholeArg("slice", args[1], line) : length;
        if (from < 0 || to < from || to > length) {
            throw raiseError("IndexError", "slice " + from + " to " + to + " is out of range for a " + what + " of length " + length, line);
        }
        return new int[] {(int) from, (int) to};
    }

    static final java.util.List<String> STR_METHODS = java.util.List.of("upper", "lower", "strip", "split", "lines", "replace", "starts_with", "ends_with", "find", "slice", "code");
    static final java.util.List<String> LIST_METHODS = java.util.List.of("append", "pop", "map", "filter", "reduce", "insert", "reverse", "sort", "clear", "find", "slice", "join");
    static final java.util.List<String> DICT_METHODS = java.util.List.of("get", "pop", "keys", "values", "clear");

    static boolean hasMethod(Object v, String name) {
        if (v instanceof String) return STR_METHODS.contains(name);
        if (v instanceof ZList) return LIST_METHODS.contains(name);
        if (v instanceof ZDict) return DICT_METHODS.contains(name);
        return false;
    }

    static ZList strings(java.util.List<String> parts) { return new ZList(new java.util.ArrayList<Object>(parts)); }

    static java.util.List<String> splitOn(String s, String sep) {
        var out = new java.util.ArrayList<String>();
        int from = 0;
        while (true) {
            int at = s.indexOf(sep, from);
            if (at < 0) break;
            out.add(s.substring(from, at));
            from = at + sep.length();
        }
        out.add(s.substring(from));
        return out;
    }

    static Object callMethodOf(Object receiver, String name, Object[] args, int line) {
        if (receiver instanceof String s) {
            switch (name) {
                case "upper", "lower": {
                    arity(name, args, 0, 0, line);
                    char[] cs = s.toCharArray();
                    for (int i = 0; i < cs.length; i++) {
                        if (name.equals("upper") && cs[i] >= 'a' && cs[i] <= 'z') cs[i] -= 32;
                        else if (name.equals("lower") && cs[i] >= 'A' && cs[i] <= 'Z') cs[i] += 32;
                    }
                    return new String(cs);
                }
                case "strip":
                    arity(name, args, 0, 0, line);
                    return strip(s);
                case "split": {
                    arity(name, args, 0, 1, line);
                    if (args.length == 0) {
                        var out = new java.util.ArrayList<String>();
                        int i = 0;
                        while (i < s.length()) {
                            while (i < s.length() && blank(s.charAt(i))) i++;
                            int start = i;
                            while (i < s.length() && !blank(s.charAt(i))) i++;
                            if (i > start) out.add(s.substring(start, i));
                        }
                        return strings(out);
                    }
                    String sep = text(name, args[0], line);
                    if (sep.isEmpty()) throw raiseError("ValueError", "cannot split on an empty str", line);
                    return strings(splitOn(s, sep));
                }
                case "lines": {
                    arity(name, args, 0, 0, line);
                    if (s.isEmpty()) return new ZList(new java.util.ArrayList<>());
                    // A "\r" goes with the "\n" just after it, and with nothing else.
                    var parts = splitOn(s, "\n");
                    String last = parts.remove(parts.size() - 1);
                    for (int i = 0; i < parts.size(); i++) {
                        String p = parts.get(i);
                        if (p.endsWith("\r")) parts.set(i, p.substring(0, p.length() - 1));
                    }
                    if (!last.isEmpty()) parts.add(last);
                    return strings(parts);
                }
                case "replace": {
                    arity(name, args, 2, 2, line);
                    String old = text(name, args[0], line), now = text(name, args[1], line);
                    if (old.isEmpty()) {
                        StringBuilder b = new StringBuilder(now);
                        s.codePoints().forEach(c -> b.appendCodePoint(c).append(now));
                        return b.toString();
                    }
                    return s.replace(old, now);
                }
                case "starts_with":
                    arity(name, args, 1, 1, line);
                    return s.startsWith(text(name, args[0], line));
                case "ends_with":
                    arity(name, args, 1, 1, line);
                    return s.endsWith(text(name, args[0], line));
                case "find": {
                    arity(name, args, 1, 1, line);
                    int at = s.indexOf(text(name, args[0], line));
                    return at < 0 ? null : (Object) (long) s.codePointCount(0, at);
                }
                case "slice": {
                    arity(name, args, 1, 2, line);
                    int[] r = slice(args, textLength(s), "str", line);
                    return sub(s, r[0], r[1]);
                }
                case "code": {
                    arity(name, args, 0, 0, line);
                    int n = textLength(s);
                    if (n != 1) throw raiseError("ValueError", "`code` needs a str of exactly one character, and this one has " + n, line);
                    return (long) s.codePointAt(0);
                }
                default:
                    throw fatal("a str has no method `" + name + "`", line);
            }
        }
        if (receiver instanceof ZDict d) {
            switch (name) {
                case "get": {
                    arity(name, args, 1, 2, line);
                    Object k = key(args[0], line);
                    if (d.map.containsKey(k)) return d.map.get(k);
                    return args.length > 1 ? args[1] : null;
                }
                case "pop": {
                    arity(name, args, 1, 1, line);
                    Object k = key(args[0], line);
                    if (!d.map.containsKey(k)) throw raiseError("KeyError", "this dict has no key " + repr(args[0]), line);
                    return d.map.remove(k);
                }
                case "keys":
                    arity(name, args, 0, 0, line);
                    return new ZList(new java.util.ArrayList<>(d.map.keySet()));
                case "values":
                    arity(name, args, 0, 0, line);
                    return new ZList(new java.util.ArrayList<>(d.map.values()));
                case "clear":
                    arity(name, args, 0, 0, line);
                    d.map.clear();
                    return null;
                default:
                    throw fatal("a dict has no method `" + name + "`", line);
            }
        }
        if (receiver instanceof ZList l) {
            switch (name) {
                case "append":
                    arity(name, args, 1, 1, line);
                    l.items.add(args[0]);
                    return null;
                case "pop": {
                    arity(name, args, 0, 1, line);
                    if (args.length > 0) return l.items.remove(position(args[0], l.items.size(), "list", line));
                    if (l.items.isEmpty()) throw raiseError("IndexError", "cannot pop from an empty list", line);
                    return l.items.remove(l.items.size() - 1);
                }
                case "insert": {
                    arity(name, args, 2, 2, line);
                    long at = wholeArg(name, args[0], line);
                    if (at < 0 || at > l.items.size()) {
                        throw raiseError("IndexError", "cannot insert at " + at + " in a list of length " + l.items.size(), line);
                    }
                    l.items.add((int) at, args[1]);
                    return null;
                }
                case "reverse":
                    arity(name, args, 0, 0, line);
                    java.util.Collections.reverse(l.items);
                    return null;
                case "clear":
                    arity(name, args, 0, 0, line);
                    l.items.clear();
                    return null;
                case "sort":
                    if (args.length == 1) {
                        sortBy(l, args[0], line);
                        return null;
                    }
                    arity(name, args, 0, 1, line);
                    sortList(l, line);
                    return null;
                case "find": {
                    arity(name, args, 1, 1, line);
                    for (int at = 0; at < l.items.size(); at++) if (equal(l.items.get(at), args[0])) return (long) at;
                    return null;
                }
                case "slice": {
                    arity(name, args, 1, 2, line);
                    int[] r = slice(args, l.items.size(), "list", line);
                    return new ZList(new java.util.ArrayList<>(l.items.subList(r[0], r[1])));
                }
                case "join": {
                    arity(name, args, 1, 1, line);
                    String sep = text(name, args[0], line);
                    var parts = new java.util.ArrayList<String>();
                    for (Object item : l.items) {
                        if (!(item instanceof String p)) {
                            throw fatal("`join` puts text together, and this list holds " + typeName(item) + ". Make each item text first, with `str( )`", line);
                        }
                        parts.add(p);
                    }
                    return String.join(sep, parts);
                }
                case "map": {
                    arity(name, args, 1, 1, line);
                    var out = new java.util.ArrayList<Object>();
                    for (Object item : new java.util.ArrayList<>(l.items)) out.add(call(args[0], new Object[] {item}, line));
                    return new ZList(out);
                }
                case "filter": {
                    arity(name, args, 1, 1, line);
                    var kept = new java.util.ArrayList<Object>();
                    for (Object item : new java.util.ArrayList<>(l.items)) {
                        Object keep = call(args[0], new Object[] {item}, line);
                        if (!(keep instanceof Boolean b)) {
                            throw fatal("the function given to `filter` must return true or false, but it returned " + typeName(keep), line);
                        }
                        if (b) kept.add(item);
                    }
                    return new ZList(kept);
                }
                case "reduce": {
                    arity(name, args, 2, 2, line);
                    Object total = args[1];
                    for (Object item : new java.util.ArrayList<>(l.items)) total = call(args[0], new Object[] {total, item}, line);
                    return total;
                }
                default:
                    throw fatal("a list has no method `" + name + "`", line);
            }
        }
        throw fatal(typeName(receiver) + " has no method `" + name + "`", line);
    }

    static final java.util.regex.Pattern FORMAT = java.util.regex.Pattern.compile("([<>^])?(0)?(\\d*)(?:\\.(\\d*))?(.)?", java.util.regex.Pattern.DOTALL);

    // `{x:>8.2f}`: [<>^]? 0? width? (.precision)? kind?
    static Object format(Object v, String spec, int line) {
        if (spec.isEmpty()) return show(v);
        var m = FORMAT.matcher(spec);
        if (!m.matches() || (m.group(5) != null && Character.isDigit(m.group(5).charAt(0)))) {
            throw fatal("`" + spec + "` is not a format Zeno understands", line);
        }
        String align = m.group(1);
        boolean zero = m.group(2) != null;
        int width = m.group(3).isEmpty() ? 0 : Integer.parseInt(m.group(3));
        Integer precision = m.group(4) == null ? null : (m.group(4).isEmpty() ? 0 : Integer.parseInt(m.group(4)));
        String kind = m.group(5);
        int places = precision == null ? 6 : precision;
        String body;
        if (("f".equals(kind) || (kind == null && precision != null)) && v instanceof Quantity q) {
            body = fixed(q.v, places) + " " + q.unit;
        } else if ("f".equals(kind) || (kind == null && precision != null && v instanceof Double)) {
            if (!isNumber(v)) throw wrongType(spec, "a number", v, line);
            body = fixed(number(v), places);
        } else if ((kind == null && precision == null) || ("d".equals(kind) && precision == null && v instanceof Long)) {
            body = show(v);
        } else if (("x".equals(kind) || "X".equals(kind) || "b".equals(kind)) && precision == null && v instanceof Long n) {
            String digits = Long.toUnsignedString(n < 0 ? -n : n, "b".equals(kind) ? 2 : 16);
            if (n == Long.MIN_VALUE) digits = "b".equals(kind) ? "1" + "0".repeat(63) : "8000000000000000";
            if ("X".equals(kind)) digits = digits.toUpperCase();
            body = (n < 0 ? "-" : "") + digits;
        } else if (("d".equals(kind) || "x".equals(kind) || "X".equals(kind) || "b".equals(kind)) && precision == null) {
            throw wrongType(spec, "an int", v, line);
        } else {
            throw fatal("`" + spec + "` is not a format Zeno understands", line);
        }
        int padding = Math.max(0, width - textLength(body));
        boolean isNumber = isNumber(v) || v instanceof Quantity;
        if (zero && isNumber) {
            if (body.startsWith("-")) return "-" + "0".repeat(padding) + body.substring(1);
            return "0".repeat(padding) + body;
        }
        String a = align != null ? align : (isNumber ? ">" : "<");
        if (a.equals("<")) return body + " ".repeat(padding);
        if (a.equals(">")) return " ".repeat(padding) + body;
        int left = padding / 2;
        return " ".repeat(left) + body + " ".repeat(padding - left);
    }

    // -------------------------------------------------------------- JSON

    static final class NotJson extends RuntimeException {
        NotJson() { super(null, null, false, false); }
    }

    static final class Json {
        final int[] cs;
        int at = 0;
        Json(String s) { cs = s.codePoints().toArray(); }

        void spaces() { while (at < cs.length && (cs[at] == ' ' || cs[at] == '\t' || cs[at] == '\n' || cs[at] == '\r')) at++; }

        boolean word(String w) {
            int[] ws = w.codePoints().toArray();
            if (at + ws.length > cs.length) return false;
            for (int i = 0; i < ws.length; i++) if (cs[at + i] != ws[i]) return false;
            at += ws.length;
            return true;
        }

        int fourHex() {
            if (at + 4 > cs.length) throw new NotJson();
            int n = 0;
            for (int i = 0; i < 4; i++) {
                int d = Character.digit(cs[at + i], 16);
                if (d < 0 || cs[at + i] > 'f') throw new NotJson();
                n = n * 16 + d;
            }
            at += 4;
            return n;
        }

        String text() {
            if (at >= cs.length || cs[at] != '"') throw new NotJson();
            at++;
            StringBuilder b = new StringBuilder();
            while (true) {
                if (at >= cs.length) throw new NotJson();
                int c = cs[at++];
                if (c == '"') return b.toString();
                if (c != '\\') { b.appendCodePoint(c); continue; }
                if (at >= cs.length) throw new NotJson();
                int e = cs[at++];
                switch (e) {
                    case 'n' -> b.append('\n');
                    case 't' -> b.append('\t');
                    case 'r' -> b.append('\r');
                    case 'b' -> b.append('\b');
                    case 'f' -> b.append('\f');
                    case 'u' -> {
                        int code = fourHex();
                        if (code >= 0xD800 && code < 0xDC00 && word("\\u")) {
                            int low = fourHex();
                            if (low < 0xDC00 || low > 0xDFFF) throw new NotJson();
                            code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                        }
                        if ((code >= 0xD800 && code <= 0xDFFF) || code > 0x10FFFF) throw new NotJson();
                        b.appendCodePoint(code);
                    }
                    default -> b.appendCodePoint(e);
                }
            }
        }

        Object value() {
            spaces();
            if (at >= cs.length) throw new NotJson();
            int c = cs[at];
            if (c == 'n') { if (word("null")) return null; throw new NotJson(); }
            if (c == 't') { if (word("true")) return true; throw new NotJson(); }
            if (c == 'f') { if (word("false")) return false; throw new NotJson(); }
            if (c == '"') return text();
            if (c == '[') {
                at++;
                var out = new java.util.ArrayList<Object>();
                while (true) {
                    spaces();
                    if (word("]")) return new ZList(out);
                    if (!out.isEmpty() && !word(",")) throw new NotJson();
                    out.add(value());
                }
            }
            if (c == '{') {
                at++;
                ZDict d = new ZDict();
                int count = 0;
                while (true) {
                    spaces();
                    if (word("}")) return d;
                    if (count > 0 && !word(",")) throw new NotJson();
                    spaces();
                    String name = text();
                    spaces();
                    if (!word(":")) throw new NotJson();
                    Object v = value();
                    count++;
                    if (v != null) d.map.put(name, v);
                }
            }
            int start = at;
            while (at < cs.length && "0123456789+-.eE".indexOf(cs[at]) >= 0) at++;
            Double x = parseFloat(new String(cs, start, at - start));
            if (x == null) throw new NotJson();
            if (!Double.isInfinite(x) && !Double.isNaN(x) && x == Math.floor(x) && Math.abs(x) < 9.0e15) return (long) (double) x;
            return x;
        }
    }

    static final Object NOT_JSON = new Object();

    static Object jsonParse(String s) {
        try {
            Json j = new Json(s);
            Object v = j.value();
            j.spaces();
            return j.at == j.cs.length ? v : NOT_JSON;
        } catch (NotJson e) {
            return NOT_JSON;
        }
    }

    static String jsonText(String s) {
        StringBuilder b = new StringBuilder("\"");
        s.codePoints().forEach(c -> {
            if (c == '"') b.append("\\\"");
            else if (c == '\\') b.append("\\\\");
            else if (c == '\n') b.append("\\n");
            else if (c == '\r') b.append("\\r");
            else if (c == '\t') b.append("\\t");
            else if (c < 0x20) b.append(String.format("\\u%04x", c));
            else b.appendCodePoint(c);
        });
        return b.append('"').toString();
    }

    static String jsonWrite(Object v, int line) {
        if (v == null) return "null";
        if (v instanceof Boolean b) return b ? "true" : "false";
        if (v instanceof Long n) return Long.toString(n);
        if (v instanceof Double x) {
            if (Double.isNaN(x) || Double.isInfinite(x)) throw raiseError("ValueError", "JSON has no way to write " + showFloat(x), line);
            return showFloat(x);
        }
        if (v instanceof String s) return jsonText(s);
        if (v instanceof ZList l) {
            var parts = new java.util.ArrayList<String>();
            for (Object item : l.items) parts.add(jsonWrite(item, line));
            return "[" + String.join(",", parts) + "]";
        }
        if (v instanceof ZDict d) {
            var parts = new java.util.ArrayList<String>();
            for (var entry : d.map.entrySet()) parts.add(jsonText(show(entry.getKey())) + ":" + jsonWrite(entry.getValue(), line));
            return "{" + String.join(",", parts) + "}";
        }
        if (v instanceof Instance o) {
            var parts = new java.util.ArrayList<String>();
            for (int i = 0; i < o.cls.fields.length; i++) parts.add(jsonText(o.cls.fields[i]) + ":" + jsonWrite(o.fields[i], line));
            return "{" + String.join(",", parts) + "}";
        }
        if (v instanceof Quantity q) {
            throw raiseError("ValueError", "JSON has no units, and this is a quantity in " + q.unit + ". Write its number, and put the unit in the name: `[\"" + q.unit + "\": ...]`", line);
        }
        throw raiseError("ValueError", "JSON has objects, lists, text, numbers, bools and none, and this is " + typeName(v), line);
    }

    // -------------------------------------------------------------- Files

    // "file", "nothing" or "other": what is at a path.
    static String at(String path) {
        // Java takes a path with nothing in it for the folder it runs in.
        if (path.isEmpty()) return "nothing";
        try (var in = new java.io.FileInputStream(path)) {
            in.read();
            return "file";
        } catch (java.io.FileNotFoundException e) {
            if (java.nio.file.Files.exists(java.nio.file.Path.of(path))) return "other";
            // A path through a file (`notes.txt/x`) is not "nothing":
            // something is in the way, as `zeno run` says.
            for (var up = new java.io.File(path).getParentFile(); up != null; up = up.getParentFile()) {
                if (up.exists()) return up.isDirectory() ? "nothing" : "other";
            }
            return "nothing";
        } catch (java.io.IOException e) {
            return "other";
        }
    }

    static String[] paths(String name, Object[] args, int count, int line) {
        arity(name, args, count, count, line);
        String[] out = new String[args.length];
        for (int i = 0; i < args.length; i++) out[i] = text(name, args[i], line);
        return out;
    }

    static Raise fileError(String message, int line) { return raiseError("FileError", message, line); }

    static final long STARTED = System.nanoTime();

    // ------------------------------------------------- The built-in table

    static Object b_print(Object[] args, int line) {
        for (int i = 0; i < args.length; i++) {
            if (i > 0) OUT.print(' ');
            OUT.print(show(args[i]));
        }
        OUT.print('\n');
        return null;
    }

    static final java.io.InputStream IN = new java.io.BufferedInputStream(System.in);

    // Text from outside that may not be UTF-8. Each run of bytes that
    // could have begun a character and did not finish one becomes one
    // U+FFFD, and so does each byte that could not begin one: the way
    // Rust, Python and the browsers read such text. Java's own reader
    // counts some of them otherwise.
    static String lossy(byte[] b) {
        var out = new java.io.ByteArrayOutputStream();
        for (int i = 0; i < b.length; ) {
            int c = b[i] & 0xFF;
            if (c < 0x80) { out.write(c); i++; continue; }
            int need = 0, lo = 0x80, hi = 0xBF;
            if (c >= 0xC2 && c <= 0xDF) need = 1;
            else if (c == 0xE0) { need = 2; lo = 0xA0; }
            else if (c == 0xED) { need = 2; hi = 0x9F; }
            else if (c >= 0xE1 && c <= 0xEF) need = 2;
            else if (c == 0xF0) { need = 3; lo = 0x90; }
            else if (c == 0xF4) { need = 3; hi = 0x8F; }
            else if (c >= 0xF1 && c <= 0xF3) need = 3;
            int j = i + 1;
            boolean whole = need > 0;
            for (int k = 0; whole && k < need; k++) {
                if (j < b.length && (b[j] & 0xFF) >= lo && (b[j] & 0xFF) <= hi) j++;
                else whole = false;
                lo = 0x80;
                hi = 0xBF;
            }
            if (whole) out.write(b, i, j - i);
            else { out.write(0xEF); out.write(0xBF); out.write(0xBD); }
            i = j;
        }
        return out.toString(java.nio.charset.StandardCharsets.UTF_8);
    }

    static Object b_input(Object[] args, int line) {
        arity("input", args, 0, 1, line);
        if (args.length > 0) OUT.print(text("input", args[0], line));
        flush();
        var bytes = new java.io.ByteArrayOutputStream();
        boolean any = false, ended = false;
        try {
            int c;
            while ((c = IN.read()) >= 0) {
                any = true;
                if (c == '\n') { ended = true; break; }
                bytes.write(c);
            }
        } catch (java.io.IOException e) {
            // As at the end of the input.
        }
        if (!any) return null;
        String s = lossy(bytes.toByteArray());
        if (ended && s.endsWith("\r")) s = s.substring(0, s.length() - 1);
        return s.replace("\0", "");
    }

    static Object b_len(Object[] args, int line) {
        arity("len", args, 1, 1, line);
        Object v = args[0];
        if (v instanceof String s) return (long) textLength(s);
        if (v instanceof ZList l) return (long) l.items.size();
        if (v instanceof ZDict d) return (long) d.map.size();
        throw wrongType("len", "a str, a list or a dict", v, line);
    }

    static Object b_range(Object[] args, int line) {
        arity("range", args, 1, 3, line);
        long[] n = new long[args.length];
        for (int i = 0; i < args.length; i++) {
            if (!(args[i] instanceof Long x)) throw wrongType("range", "ints", args[i], line);
            n[i] = x;
        }
        long start = 0, stop, step = 1;
        if (n.length == 1) stop = n[0];
        else if (n.length == 2) { start = n[0]; stop = n[1]; }
        else { start = n[0]; stop = n[1]; step = n[2]; }
        if (step == 0) throw raiseError("ValueError", "the step of a range cannot be 0", line);
        var out = new java.util.ArrayList<Object>();
        for (long i = start; step > 0 ? i < stop : i > stop; ) {
            out.add(i);
            long next = i + step;
            if (step > 0 ? next < i : next > i) break;
            i = next;
        }
        return new ZList(out);
    }

    static long toInt(double x) {
        if (Double.isNaN(x)) return 0;
        return (long) x;
    }

    static Object b_int(Object[] args, int line) {
        arity("int", args, 1, 1, line);
        Object v = args[0];
        if (v instanceof Long) return v;
        if (v instanceof Double || v instanceof Float) {
            double x = number(v);
            if (Double.isNaN(x) || Double.isInfinite(x)) throw raiseError("ValueError", "cannot turn " + showFloat(x) + " into an int", line);
            return toInt(x);
        }
        if (v instanceof String s) {
            String t = strip(s).replace("_", "");
            if (t.matches("[+-]?[0-9]+")) {
                try {
                    return Long.parseLong(t.startsWith("+") ? t.substring(1) : t);
                } catch (NumberFormatException e) {
                    // Too big: as below.
                }
            }
            throw raiseError("ValueError", "cannot turn " + quoted(s) + " into an int", line);
        }
        throw wrongType("int", "a number or a str", v, line);
    }

    static Object b_float(Object[] args, int line) {
        arity("float", args, 1, 1, line);
        Object v = args[0];
        if (v instanceof Long n) return (double) n;
        if (v instanceof Double) return v;
        if (v instanceof Float f) return (double) (float) f;
        if (v instanceof String s) {
            Double x = parseFloat(strip(s));
            if (x == null) throw raiseError("ValueError", "cannot turn " + quoted(s) + " into a float", line);
            return x;
        }
        throw wrongType("float", "a number or a str", v, line);
    }

    static Object b_f32(Object[] args, int line) {
        arity("f32", args, 1, 1, line);
        Object v = args[0];
        if (v instanceof Float) return v;
        if (v instanceof Long) return (float) (double) single(v);
        if (v instanceof Double x) {
            double r = fround(x);
            if (Double.isInfinite(r) && !Double.isInfinite(x) && !Double.isNaN(x)) {
                throw raiseError("OverflowError", showFloat(x) + " does not fit in an f32, whose largest is about 3.4e38", line);
            }
            return (float) r;
        }
        throw wrongType("f32", "a number", v, line);
    }

    static Object b_str(Object[] args, int line) {
        arity("str", args, 1, 1, line);
        return show(args[0]);
    }

    static Object b_abs(Object[] args, int line) {
        arity("abs", args, 1, 1, line);
        Object v = args[0];
        if (v instanceof Long n) {
            if (n == Long.MIN_VALUE) throw overflow(line);
            return Math.abs(n);
        }
        if (v instanceof Double x) return Math.abs(x);
        if (v instanceof Float x) return Math.abs(x);
        if (v instanceof Quantity q) return new Quantity(Math.abs(q.v), q.unit);
        throw wrongType("abs", "a number", v, line);
    }

    static Object b_min(Object[] args, int line) { return pick("min", args, line, true); }

    static Object b_max(Object[] args, int line) { return pick("max", args, line, false); }

    static Object b_sum(Object[] args, int line) {
        String[] unit = new String[1];
        var values = numbers("sum", args, line, unit);
        if (unit[0] != null) {
            throw fatal("`sum` works on plain numbers, and these have a unit. Add quantities with `+`, which keeps the unit: start from `var total = 0 V` and add each in a loop", line);
        }
        if (values.stream().anyMatch(v -> v instanceof Float) && values.stream().allMatch(v -> v instanceof Float || v instanceof Long)) {
            double single = 0;
            for (Object item : values) single = fround(single + single(item));
            return (float) single;
        }
        long total = 0;
        boolean allInts = true;
        double floatTotal = 0;
        for (Object item : values) {
            floatTotal += number(item);
            if (allInts && item instanceof Long n) total = addInt(total, n, line);
            else allInts = false;
        }
        return allInts ? (Object) total : (Object) floatTotal;
    }

    static Object b_char(Object[] args, int line) {
        arity("char", args, 1, 1, line);
        long code = wholeArg("char", args[0], line);
        if (code <= 0 || code > 0x10FFFF || (code >= 0xD800 && code <= 0xDFFF)) {
            throw raiseError("ValueError", code + " is not the code of a character", line);
        }
        return new String(Character.toChars((int) code));
    }

    static Object b_allocate(Object[] args, int line) { throw fatal("manual memory is only for a program compiled to machine code", line); }

    static Object b_free(Object[] args, int line) { return b_allocate(args, line); }

    static double oneNumber(String name, Object[] args, int line) {
        arity(name, args, 1, 1, line);
        if (!isNumber(args[0])) throw wrongType(name, "a number", args[0], line);
        return number(args[0]);
    }

    static double angle(String name, Object[] args, int line) {
        arity(name, args, 1, 1, line);
        if (args[0] instanceof Quantity q) {
            double[] s = convert(q.unit, "rad");
            if (s == null) throw fatal("`" + name + "` needs a number or an angle, such as `30 deg`, not a quantity in " + q.unit, line);
            return apply(s, q.v);
        }
        if (!isNumber(args[0])) throw wrongType(name, "a number or an angle, such as `30 deg`", args[0], line);
        return number(args[0]);
    }

    static double ratio(String name, Object[] args, int line) {
        double x = oneNumber(name, args, line);
        if (x < -1 || x > 1) throw raiseError("ValueError", "`" + name + "` needs a number from -1 to 1, and this is " + showFloat(x), line);
        return x;
    }

    static double aboveZero(String name, Object[] args, int line) {
        double x = oneNumber(name, args, line);
        if (x <= 0) throw raiseError("ValueError", "cannot take the logarithm of " + showFloat(x), line);
        return x;
    }

    // A whole float as an int; infinity and NaN stay as they are.
    // What `floor` and `ceil` give is an int: one too big for an int is the
    // biggest int there is, and what is not a number is 0, as in `zeno run`.
    static Object wholeOf(double x) { return toInt(x); }

    static Object b_math_sqrt(Object[] args, int line) {
        double x = oneNumber("sqrt", args, line);
        if (x < 0) throw raiseError("ValueError", "cannot take the square root of " + showFloat(x), line);
        return Math.sqrt(x);
    }

    static Object b_math_floor(Object[] args, int line) { return wholeOf(Math.floor(oneNumber("floor", args, line))); }

    static Object b_math_ceil(Object[] args, int line) { return wholeOf(Math.ceil(oneNumber("ceil", args, line))); }

    // A half goes away from zero, as Rust's `round` does.
    static Object b_math_round(Object[] args, int line) {
        double x = oneNumber("round", args, line);
        if (Double.isNaN(x)) return 0L;
        double r = Math.floor(Math.abs(x));
        if (Math.abs(x) - r >= 0.5) r += 1;
        return toInt(Math.copySign(r, x));
    }

    static Object b_math_sin(Object[] args, int line) { return Math.sin(angle("sin", args, line)); }

    static Object b_math_cos(Object[] args, int line) { return Math.cos(angle("cos", args, line)); }

    static Object b_math_tan(Object[] args, int line) { return Math.tan(angle("tan", args, line)); }

    static Object b_math_asin(Object[] args, int line) { return new Quantity(Math.asin(ratio("asin", args, line)), "rad"); }

    static Object b_math_acos(Object[] args, int line) { return new Quantity(Math.acos(ratio("acos", args, line)), "rad"); }

    static Object b_math_atan(Object[] args, int line) { return new Quantity(Math.atan(oneNumber("atan", args, line)), "rad"); }

    static Object b_math_atan2(Object[] args, int line) {
        arity("atan2", args, 2, 2, line);
        if (!isNumber(args[0])) throw wrongType("atan2", "two numbers", args[0], line);
        if (!isNumber(args[1])) throw wrongType("atan2", "two numbers", args[1], line);
        return new Quantity(Math.atan2(number(args[0]), number(args[1])), "rad");
    }

    static Object b_math_exp(Object[] args, int line) { return Math.exp(oneNumber("exp", args, line)); }

    static Object b_math_log(Object[] args, int line) { return Math.log(aboveZero("log", args, line)); }

    static Object b_math_log2(Object[] args, int line) {
        double x = aboveZero("log2", args, line);
        // Exact for the powers of two, as Rust's `log2` is.
        int e = Math.getExponent(x);
        if (x == Math.scalb(1.0, e)) return (double) e;
        return Math.log(x) / Math.log(2);
    }

    static Object b_math_log10(Object[] args, int line) { return Math.log10(aboveZero("log10", args, line)); }

    static Object b_time_sleep(Object[] args, int line) {
        arity("sleep", args, 1, 1, line);
        if (!(args[0] instanceof Quantity q)) throw wrongType("sleep", "a time with a unit, such as `500 ms`", args[0], line);
        double[] s = convert(q.unit, "s");
        if (s == null) throw fatal("`sleep` needs a time, such as `500 ms`, not a quantity in " + q.unit, line);
        double seconds = apply(s, q.v);
        if (seconds > 0) {
            flush();
            try {
                long nanos = (long) (Math.min(seconds, 1e9) * 1e9);
                Thread.sleep(nanos / 1000000, (int) (nanos % 1000000));
            } catch (InterruptedException e) {
                // Woken early: go on.
            }
        }
        return null;
    }

    static Object b_time_now(Object[] args, int line) {
        arity("now", args, 0, 0, line);
        return new Quantity((System.nanoTime() - STARTED) / 1e6, "ms");
    }

    static Object b_files_read(Object[] args, int line) {
        String path = paths("read", args, 1, line)[0];
        String where = at(path);
        if (where.equals("nothing")) throw fileError("there is no file `" + path + "`", line);
        if (where.equals("other")) throw fileError("cannot read `" + path + "`", line);
        byte[] data;
        try {
            data = java.nio.file.Files.readAllBytes(java.nio.file.Path.of(path));
        } catch (java.io.IOException e) {
            throw fileError("cannot read `" + path + "`", line);
        }
        String s;
        try {
            s = java.nio.charset.StandardCharsets.UTF_8.newDecoder().decode(java.nio.ByteBuffer.wrap(data)).toString();
        } catch (java.nio.charset.CharacterCodingException e) {
            s = null;
        }
        if (s == null || s.indexOf('\0') >= 0) throw fileError("`" + path + "` does not hold text", line);
        return s;
    }

    static Object put(String name, Object[] args, boolean append, int line) {
        String[] p = paths(name, args, 2, line);
        try (var out = new java.io.FileOutputStream(p[0], append)) {
            out.write(p[1].getBytes(java.nio.charset.StandardCharsets.UTF_8));
        } catch (java.io.IOException e) {
            throw fileError("cannot write `" + p[0] + "`", line);
        }
        return null;
    }

    static Object b_files_write(Object[] args, int line) { return put("write", args, false, line); }

    static Object b_files_read_bytes(Object[] args, int line) {
        String path = paths("read_bytes", args, 1, line)[0];
        String where = at(path);
        if (where.equals("nothing")) throw fileError("there is no file `" + path + "`", line);
        if (where.equals("other")) throw fileError("cannot read `" + path + "`", line);
        byte[] data;
        try {
            data = java.nio.file.Files.readAllBytes(java.nio.file.Path.of(path));
        } catch (java.io.IOException e) {
            throw fileError("cannot read `" + path + "`", line);
        }
        var items = new java.util.ArrayList<Object>(data.length);
        for (byte b : data) items.add((long) (b & 0xFF));
        return new ZList(items);
    }

    static Object b_files_write_bytes(Object[] args, int line) {
        arity("write_bytes", args, 2, 2, line);
        String path = text("write_bytes", args[0], line);
        if (!(args[1] instanceof ZList list)) throw wrongType("write_bytes", "a list[u8]", args[1], line);
        byte[] data = new byte[list.items.size()];
        for (int i = 0; i < data.length; i++) {
            Object item = list.items.get(i);
            if (!(item instanceof Long n)) throw wrongType("write_bytes", "a list[u8]", item, line);
            if (n < 0 || n > 255) throw raiseError("ValueError", n + " is not a byte: a byte is a number from 0 to 255", line);
            data[i] = (byte) (long) n;
        }
        try (var out = new java.io.FileOutputStream(path, false)) {
            out.write(data);
        } catch (java.io.IOException e) {
            throw fileError("cannot write `" + path + "`", line);
        }
        return null;
    }

    static Object b_files_append(Object[] args, int line) { return put("append", args, true, line); }

    static Object b_files_exists(Object[] args, int line) { return at(paths("exists", args, 1, line)[0]).equals("file"); }

    static Object b_files_remove(Object[] args, int line) {
        String path = paths("remove", args, 1, line)[0];
        String where = at(path);
        if (where.equals("nothing")) throw fileError("there is no file `" + path + "`", line);
        if (where.equals("file") && new java.io.File(path).delete()) return null;
        throw fileError("cannot remove `" + path + "`", line);
    }

    static String[] arguments = new String[0];

    // `program.exit(code)`: what was printed goes out, and the program
    // ends with `code`.
    static Object b_program_exit(Object[] args, int line) {
        arity("exit", args, 1, 1, line);
        long code = wholeArg("exit", args[0], line);
        if (code < 0 || code > 255) {
            throw raiseError("ValueError", "a program ends with a number from 0 to 255, not " + code, line);
        }
        flush();
        System.err.flush();
        System.exit((int) code);
        return null;
    }

    // `program.environment(name)`: the text, or none. A name with `=` or
    // a zero byte in it is in no environment.
    static Object b_program_environment(Object[] args, int line) {
        arity("environment", args, 1, 1, line);
        String name = text("environment", args[0], line);
        if (name.isEmpty() || name.indexOf('=') >= 0 || name.indexOf('\0') >= 0) return null;
        return System.getenv(name);
    }

    // `program.warn(text)`: one line on the error stream, after what was printed.
    static Object b_program_warn(Object[] args, int line) {
        arity("warn", args, 1, 1, line);
        String said = text("warn", args[0], line);
        flush();
        System.err.print(said + "\n");
        System.err.flush();
        return null;
    }

    static Object b_program_arguments(Object[] args, int line) {
        arity("arguments", args, 0, 0, line);
        return new ZList(new java.util.ArrayList<Object>(java.util.Arrays.asList((Object[]) arguments)));
    }

    static Object b_json_parse(Object[] args, int line) {
        arity("parse", args, 1, 1, line);
        String s = text("parse", args[0], line);
        Object v = jsonParse(s);
        if (v == NOT_JSON) {
            int a = 0;
            while (a < s.length() && " \t\n\r".indexOf(s.charAt(a)) >= 0) a++;
            String start = s.substring(a);
            int n = textLength(start);
            String shown = n > 30 ? sub(start, 0, 30) + "..." : start;
            throw raiseError("ValueError", "this text is not JSON: `" + shown + "`", line);
        }
        return v;
    }

    static Object b_json_write(Object[] args, int line) {
        arity("write", args, 1, 1, line);
        return jsonWrite(args[0], line);
    }

    static BuiltinBody sized(String name, boolean wrap) {
        Width w = WIDTHS.get(name);
        String article = w.signed ? "an" : "a";
        if (!wrap) {
            return (args, line) -> {
                arity(name, args, 1, 1, line);
                if (!(args[0] instanceof Long n)) throw wrongType(name, "a whole number", args[0], line);
                if (n < w.lo || n > w.hi) throw raiseError("OverflowError", n + " does not fit in " + article + " " + name + " (" + w.lo + " to " + w.hi + ")", line);
                return n;
            };
        }
        return (args, line) -> {
            arity(name + ".wrap", args, 1, 1, line);
            if (!(args[0] instanceof Long n)) throw wrongType(name + ".wrap", "a whole number", args[0], line);
            long low = n & ((1L << w.bits) - 1);
            return w.signed && low > w.hi ? low - (1L << w.bits) : low;
        };
    }

    static final BuiltinBody b_u8 = sized("u8", false), b_u8_wrap = sized("u8", true);
    static final BuiltinBody b_u16 = sized("u16", false), b_u16_wrap = sized("u16", true);
    static final BuiltinBody b_u32 = sized("u32", false), b_u32_wrap = sized("u32", true);
    static final BuiltinBody b_i8 = sized("i8", false), b_i8_wrap = sized("i8", true);
    static final BuiltinBody b_i16 = sized("i16", false), b_i16_wrap = sized("i16", true);
    static final BuiltinBody b_i32 = sized("i32", false), b_i32_wrap = sized("i32", true);

    static final java.util.List<String> ERROR_KINDS = java.util.List.of("Error", "ValueError", "IndexError", "KeyError", "ZeroDivisionError", "OverflowError", "FileError");

    static final java.util.HashMap<String, BuiltinBody> BUILTINS = new java.util.HashMap<>();

    static {
        BUILTINS.put("print", Z::b_print);
        BUILTINS.put("input", Z::b_input);
        BUILTINS.put("len", Z::b_len);
        BUILTINS.put("range", Z::b_range);
        BUILTINS.put("int", Z::b_int);
        BUILTINS.put("float", Z::b_float);
        BUILTINS.put("f32", Z::b_f32);
        BUILTINS.put("str", Z::b_str);
        BUILTINS.put("abs", Z::b_abs);
        BUILTINS.put("min", Z::b_min);
        BUILTINS.put("max", Z::b_max);
        BUILTINS.put("sum", Z::b_sum);
        BUILTINS.put("char", Z::b_char);
        BUILTINS.put("allocate", Z::b_allocate);
        BUILTINS.put("free", Z::b_free);
        BUILTINS.put("math.sqrt", Z::b_math_sqrt);
        BUILTINS.put("math.floor", Z::b_math_floor);
        BUILTINS.put("math.ceil", Z::b_math_ceil);
        BUILTINS.put("math.round", Z::b_math_round);
        BUILTINS.put("math.sin", Z::b_math_sin);
        BUILTINS.put("math.cos", Z::b_math_cos);
        BUILTINS.put("math.tan", Z::b_math_tan);
        BUILTINS.put("math.asin", Z::b_math_asin);
        BUILTINS.put("math.acos", Z::b_math_acos);
        BUILTINS.put("math.atan", Z::b_math_atan);
        BUILTINS.put("math.atan2", Z::b_math_atan2);
        BUILTINS.put("math.exp", Z::b_math_exp);
        BUILTINS.put("math.log", Z::b_math_log);
        BUILTINS.put("math.log2", Z::b_math_log2);
        BUILTINS.put("math.log10", Z::b_math_log10);
        BUILTINS.put("time.sleep", Z::b_time_sleep);
        BUILTINS.put("time.now", Z::b_time_now);
        BUILTINS.put("files.read", Z::b_files_read);
        BUILTINS.put("files.write", Z::b_files_write);
        BUILTINS.put("files.read_bytes", Z::b_files_read_bytes);
        BUILTINS.put("files.write_bytes", Z::b_files_write_bytes);
        BUILTINS.put("files.append", Z::b_files_append);
        BUILTINS.put("files.exists", Z::b_files_exists);
        BUILTINS.put("files.remove", Z::b_files_remove);
        BUILTINS.put("program.arguments", Z::b_program_arguments);
        BUILTINS.put("program.exit", Z::b_program_exit);
        BUILTINS.put("program.environment", Z::b_program_environment);
        BUILTINS.put("program.warn", Z::b_program_warn);
        BUILTINS.put("json.parse", Z::b_json_parse);
        BUILTINS.put("json.write", Z::b_json_write);
        for (String name : WIDTHS.keySet()) {
            BUILTINS.put(name, sized(name, false));
            BUILTINS.put(name + ".wrap", sized(name, true));
        }
        for (String kind : ERROR_KINDS) BUILTINS.put(kind, (args, line) -> newError(kind, args, line));
    }

    static Object b_Error(Object[] args, int line) { return newError("Error", args, line); }
    static Object b_ValueError(Object[] args, int line) { return newError("ValueError", args, line); }
    static Object b_IndexError(Object[] args, int line) { return newError("IndexError", args, line); }
    static Object b_KeyError(Object[] args, int line) { return newError("KeyError", args, line); }
    static Object b_ZeroDivisionError(Object[] args, int line) { return newError("ZeroDivisionError", args, line); }
    static Object b_OverflowError(Object[] args, int line) { return newError("OverflowError", args, line); }
    static Object b_FileError(Object[] args, int line) { return newError("FileError", args, line); }

    static Object call_builtin(String name, Object[] args, int line) {
        BuiltinBody f = BUILTINS.get(name);
        if (f == null) throw fatal("there is no `" + name + "` in a program built for Java", line);
        return f.call(args, line);
    }

    static final java.util.HashMap<String, Object> BUILTIN_VALUES = new java.util.HashMap<>();

    // Something that comes with Zeno, as a value: `print`, `math.pi`.
    static Object builtin(String name, int line) {
        if (name.equals("math.pi")) return Math.PI;
        if (name.equals("math.e")) return Math.E;
        if (ERROR_KINDS.contains(name)) return new ErrorType(name);
        Object known = BUILTIN_VALUES.get(name);
        if (known != null) return known;
        BuiltinBody body = BUILTINS.get(name);
        int dot = name.indexOf('.');
        if (body == null) {
            if (dot >= 0) throw fatal("module `" + name.substring(0, dot) + "` has no `" + name.substring(dot + 1) + "`", line);
            throw fatal("`" + name + "` is not defined", line);
        }
        Func v = new Func(dot >= 0 ? name.substring(dot + 1) : name, new String[0], new boolean[0], null, name, body);
        BUILTIN_VALUES.put(name, v);
        return v;
    }

    static Object module(String name) { return new Module(name); }

    // ------------------------------------------------------- The program

    interface Main { Object run(); }

    // Run the program, and say what stopped it if something did.
    static void start(String[] files, String[] args, Main main) {
        arguments = args;
        int[] status = {0};
        Thread thread = new Thread(null, () -> {
            try {
                main.run();
                flush();
            } catch (Raise e) {
                flush();
                System.err.print("error: uncaught " + shownName(e.value.kind) + ": " + show(e.value) + "\n" + where(files, e.line) + "\n");
                status[0] = 1;
            } catch (Fatal e) {
                flush();
                System.err.print("error: " + e.text + "\n" + where(files, e.line) + "\n");
                status[0] = 1;
            }
        }, "zeno", 512L * 1024 * 1024);
        thread.start();
        try {
            thread.join();
        } catch (InterruptedException e) {
            status[0] = 1;
        }
        System.err.flush();
        System.exit(status[0]);
    }

    static String where(String[] files, int line) {
        int index = line / LINES_A_FILE;
        String file = index < files.length ? files[index] : files[0];
        return " --> " + file + ":" + (line % LINES_A_FILE);
    }
}
