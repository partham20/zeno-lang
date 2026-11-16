// The Zeno runtime for C#.
//
// A program built with `zeno build --target cs` is one `.cs` file: the
// program, as top-level statements, then this runtime, the class `Z`. It
// holds what every Zeno value is, and what each operator and built-in does
// with it: what ops.rs and builtins.rs do for `zeno run`, step for step
// and message for message, so that the program prints what `zeno run`
// prints. runtime.java is the same for Java, and the two are close.
//
// How a Zeno value is held (every one is an object):
//
//   int        long, with every step checked for overflow
//   float      double
//   str        string; an index counts characters, not UTF-16 units
//   bool       bool
//   none       null
//   list       Z.ZList
//   dict       Z.ZDict, which keeps the order the keys were put in
//   quantity   Z.Quantity: a number and its unit
//   function   Z.Func
//   object     Z.Instance of a Z.ZClass
//   enum case  Z.Case of a Z.ZEnum
//   error      Z.ZError

static class Z
{
    const int LINES_A_FILE = 1000000;

    // ------------------------------------------------------------ Values

    public sealed class ZList
    {
        public System.Collections.Generic.List<object> items;
        public ZList(System.Collections.Generic.List<object> items) { this.items = items; }
    }

    // A dict: its keys in the order they were put in, and their values.
    public sealed class ZDict
    {
        public System.Collections.Generic.List<object> keys = new();
        public System.Collections.Generic.Dictionary<object, object> map = new();

        public void Set(object k, object v)
        {
            if (!map.ContainsKey(k)) keys.Add(k);
            map[k] = v;
        }

        public void Remove(object k)
        {
            map.Remove(k);
            keys.RemoveAt(keys.FindIndex(key => key.Equals(k)));
        }
    }

    public sealed class Quantity
    {
        public readonly double v;
        public readonly string unit;
        public Quantity(double v, string unit) { this.v = v; this.unit = unit; }
    }

    public delegate object Body(object[] args);

    public delegate object BuiltinBody(object[] args, int line);

    public sealed class Func
    {
        public readonly string name;
        public readonly string[] parameters;
        public readonly bool[] defaults;
        public readonly Body body;
        // A function that comes with Zeno: its full name, and its body.
        public readonly string builtin;
        public readonly BuiltinBody bbody;

        public Func(string name, string[] parameters, bool[] defaults, Body body, string builtin, BuiltinBody bbody)
        {
            this.name = name; this.parameters = parameters; this.defaults = defaults; this.body = body;
            this.builtin = builtin; this.bbody = bbody;
        }
    }

    public sealed class ZClass
    {
        public readonly string name;
        public readonly string[] fields;
        public readonly object[] defaults;
        public readonly System.Collections.Generic.Dictionary<string, Func> methods;

        public ZClass(string name, string[] fields, object[] defaults, System.Collections.Generic.Dictionary<string, Func> methods)
        {
            this.name = name; this.fields = fields; this.defaults = defaults; this.methods = methods;
        }
    }

    public sealed class Instance
    {
        public readonly ZClass cls;
        public readonly object[] fields;
        public Instance(ZClass cls, object[] fields) { this.cls = cls; this.fields = fields; }
    }

    public sealed class ZEnum
    {
        public readonly string name;
        public readonly string[] cases;
        public readonly string[][] fields;
        public ZEnum(string name, string[] cases, string[][] fields) { this.name = name; this.cases = cases; this.fields = fields; }
    }

    public sealed class Case
    {
        public readonly ZEnum def;
        public readonly int index;
        public readonly object[] fields;
        public Case(ZEnum def, int index, object[] fields) { this.def = def; this.index = index; this.fields = fields; }
    }

    public sealed class Constructor
    {
        public readonly ZEnum def;
        public readonly int index;
        public Constructor(ZEnum def, int index) { this.def = def; this.index = index; }
    }

    public sealed class ErrorType
    {
        public readonly string kind;
        public ErrorType(string kind) { this.kind = kind; }
    }

    public sealed class ZError
    {
        public readonly string kind, message;
        public readonly object obj;
        public ZError(string kind, string message, object obj) { this.kind = kind; this.message = message; this.obj = obj; }
    }

    public sealed class Module
    {
        public readonly string name;
        public Module(string name) { this.name = name; }
    }

    // A method of a str, a list or a dict, read but not yet called.
    public sealed class Method
    {
        public readonly object receiver;
        public readonly string name;
        public Method(object receiver, string name) { this.receiver = receiver; this.name = name; }
    }

    // A method of a class, read from an object.
    public sealed class Bound
    {
        public readonly object obj;
        public readonly Func fn;
        public Bound(object obj, Func fn) { this.obj = obj; this.fn = fn; }
    }

    // What a call gives a parameter it leaves out.
    public static readonly object MISSING = new object();

    // An error the program may catch, on its way to a `try`.
    public sealed class Raise : System.Exception
    {
        public readonly ZError value;
        public readonly int line;
        public Raise(ZError value, int line) { this.value = value; this.line = line; }
    }

    // A mistake in the program, which nothing catches.
    public sealed class Fatal : System.Exception
    {
        public readonly string text;
        public readonly int line;
        public Fatal(string text, int line) { this.text = text; this.line = line; }
    }

    public static System.Exception fatal(string message, int line) => throw new Fatal(message, line);

    static System.Exception raiseError(string kind, string message, int line) => throw new Raise(new ZError(kind, message, null), line);

    static System.Exception overflow(int line) => raiseError("OverflowError", "the result is too big for an int", line);

    static System.Exception byZero(int line) => raiseError("ZeroDivisionError", "division by zero", line);

    static bool isNumber(object v) => v is long || v is double;

    static double number(object v) => v is long l ? l : (double)v;

    static string typeName(object v)
    {
        switch (v)
        {
            case long: return "an int";
            case double: return "a float";
            case string: return "a str";
            case bool: return "a bool";
            case null: return "none";
            case Quantity q: return "a quantity in " + q.unit;
            case ZList: return "a list";
            case ZDict: return "a dict";
            case Func or Method or Bound: return "a function";
            case ZClass: return "a class";
            case Instance i: return "a " + i.cls.name;
            case ZEnum: return "an enum";
            case Constructor: return "an enum case";
            case Case c: return "a " + c.def.name;
            case ErrorType: return "an error type";
            case ZError e: return "a " + e.kind;
            case Module: return "a module";
        }
        return "a value";
    }

    // `Point`, not `shapes.Point`; `Stack`, not `Stack[int]`.
    static string shownName(string name)
    {
        int bracket = name.IndexOf('[');
        if (bracket >= 0) name = name.Substring(0, bracket);
        int dot = name.LastIndexOf('.');
        return dot < 0 ? name : name.Substring(dot + 1);
    }

    // -------------------------------------------------------------- Text

    // A str's characters are code points; a C# string counts UTF-16 units.
    // Most text has no character outside the first 65536, and then the two
    // agree; the last str asked about is remembered.
    static string lastText = null;
    static bool lastPlain = true;

    static bool plain(string s)
    {
        if ((object)s == lastText) return lastPlain;
        bool p = true;
        foreach (char c in s) if (char.IsSurrogate(c)) { p = false; break; }
        lastText = s;
        lastPlain = p;
        return p;
    }

    static System.Collections.Generic.List<string> codePoints(string s)
    {
        var out_ = new System.Collections.Generic.List<string>(s.Length);
        for (int i = 0; i < s.Length; i++)
        {
            if (char.IsHighSurrogate(s[i]) && i + 1 < s.Length && char.IsLowSurrogate(s[i + 1])) { out_.Add(s.Substring(i, 2)); i++; }
            else out_.Add(s.Substring(i, 1));
        }
        return out_;
    }

    static int textLength(string s)
    {
        if (plain(s)) return s.Length;
        return codePoints(s).Count;
    }

    static string sub(string s, int from, int to)
    {
        if (plain(s)) return s.Substring(from, to - from);
        return string.Concat(codePoints(s).GetRange(from, to - from));
    }

    static int codePointAt(string s, int i) => char.IsHighSurrogate(s[i]) && i + 1 < s.Length ? char.ConvertToUtf32(s[i], s[i + 1]) : s[i];

    static string fromCode(long code) => char.ConvertFromUtf32((int)code);

    // ------------------------------------------------------------ Showing

    static readonly System.Globalization.CultureInfo C = System.Globalization.CultureInfo.InvariantCulture;

    // A float as Rust's `{:?}` shows it: the shortest digits that read
    // back as the same number, `.0` on a whole one, and an exponent for one
    // below 1e-4 or from 1e16 up.
    public static string showFloat(double x)
    {
        if (double.IsNaN(x)) return "NaN";
        if (double.IsPositiveInfinity(x)) return "inf";
        if (double.IsNegativeInfinity(x)) return "-inf";
        if (x == 0) return double.IsNegative(x) ? "-0.0" : "0.0";
        // The shortest digits: "R" gives them, in its own layout.
        string r = x.ToString("R", C);
        string sign = "";
        if (r[0] == '-') { sign = "-"; r = r.Substring(1); }
        int e = r.IndexOfAny(new[] { 'E', 'e' });
        string mantissa = e < 0 ? r : r.Substring(0, e);
        int exp = e < 0 ? 0 : int.Parse(r.Substring(e + 1), C);
        int point = mantissa.IndexOf('.');
        string whole = point < 0 ? mantissa : mantissa.Substring(0, point);
        string fraction = point < 0 ? "" : mantissa.Substring(point + 1);
        string digits = whole + fraction;
        exp += whole.Length - 1;
        int lead = 0;
        while (lead < digits.Length - 1 && digits[lead] == '0') { lead++; exp--; }
        digits = digits.Substring(lead);
        digits = digits.TrimEnd('0');
        if (digits.Length == 0) digits = "0";
        if (exp < -4 || exp >= 16)
        {
            return sign + digits[0] + (digits.Length > 1 ? "." + digits.Substring(1) : "") + "e" + exp;
        }
        if (exp < 0) return sign + "0." + new string('0', -exp - 1) + digits;
        if (digits.Length <= exp + 1) return sign + digits + new string('0', exp + 1 - digits.Length) + ".0";
        return sign + digits.Substring(0, exp + 1) + "." + digits.Substring(exp + 1);
    }

    // A float with a fixed number of decimals, as Rust's `{:.2}` writes it:
    // from its exact value, a half rounding to the even digit.
    static string fixedPlaces(double x, int places)
    {
        if (double.IsNaN(x)) return "NaN";
        if (double.IsInfinity(x)) return x > 0 ? "inf" : "-inf";
        bool negative = x < 0 || double.IsNegative(x);
        x = System.Math.Abs(x);
        long bits = System.BitConverter.DoubleToInt64Bits(x);
        int exponent = (int)((bits >> 52) & 0x7ff);
        var mantissa = new System.Numerics.BigInteger(bits & 0xfffffffffffffL);
        if (exponent == 0) exponent = 1; else mantissa += System.Numerics.BigInteger.One << 52;
        exponent -= 1075;
        var num = mantissa * System.Numerics.BigInteger.Pow(10, places);
        var den = System.Numerics.BigInteger.One;
        if (exponent >= 0) num <<= exponent; else den <<= -exponent;
        var whole = System.Numerics.BigInteger.DivRem(num, den, out var rest);
        var twice = rest * 2;
        if (twice > den || (twice == den && !whole.IsEven)) whole += 1;
        string digits = whole.ToString(C);
        if (places > 0)
        {
            digits = digits.PadLeft(places + 1, '0');
            digits = digits.Substring(0, digits.Length - places) + "." + digits.Substring(digits.Length - places);
        }
        return (negative ? "-" : "") + digits;
    }

    // How Rust's `{:?}` shows a str: in quotes, with what cannot be seen escaped.
    static string quoted(string s)
    {
        var b = new System.Text.StringBuilder("\"");
        foreach (string ch in codePoints(s))
        {
            int c = codePointAt(ch, 0);
            if (c == '"') b.Append("\\\"");
            else if (c == '\\') b.Append("\\\\");
            else if (c == '\n') b.Append("\\n");
            else if (c == '\r') b.Append("\\r");
            else if (c == '\t') b.Append("\\t");
            else if (c == 0) b.Append("\\0");
            else if (c < 0x20 || (c >= 0x7f && c < 0xa0)) b.Append("\\u{").Append(c.ToString("x", C)).Append('}');
            else b.Append(ch);
        }
        return b.Append('"').ToString();
    }

    // How `print`, `str()` and f-strings show a value.
    public static string show(object v)
    {
        switch (v)
        {
            case string s: return s;
            case long l: return l.ToString(C);
            case double d: return showFloat(d);
            case bool b: return b ? "true" : "false";
            case null: return "none";
            case ZList l:
                {
                    var parts = new System.Collections.Generic.List<string>();
                    foreach (var item in l.items) parts.Add(repr(item));
                    return "[" + string.Join(", ", parts) + "]";
                }
            case ZDict d:
                {
                    if (d.keys.Count == 0) return "[:]";
                    var parts = new System.Collections.Generic.List<string>();
                    foreach (var k in d.keys) parts.Add(repr(k) + ": " + repr(d.map[k]));
                    return "[" + string.Join(", ", parts) + "]";
                }
            case Quantity q:
                if (!double.IsNaN(q.v) && !double.IsInfinity(q.v) && q.v == System.Math.Floor(q.v) && System.Math.Abs(q.v) < 1e15)
                {
                    return ((long)q.v).ToString(C) + " " + q.unit;
                }
                return showFloat(q.v) + " " + q.unit;
            case Instance o:
                {
                    var parts = new System.Collections.Generic.List<string>();
                    for (int i = 0; i < o.cls.fields.Length; i++) parts.Add(o.cls.fields[i] + "=" + repr(o.fields[i]));
                    return shownName(o.cls.name) + "(" + string.Join(", ", parts) + ")";
                }
            case Case c:
                {
                    string name = shownName(c.def.cases[c.index]);
                    if (c.fields.Length == 0) return name;
                    var parts = new System.Collections.Generic.List<string>();
                    foreach (var f in c.fields) parts.Add(repr(f));
                    return name + "(" + string.Join(", ", parts) + ")";
                }
            case ZError e: return e.message;
            case Method m: return "<function " + m.name + ">";
            case Bound b: return "<function " + b.fn.name + ">";
            case ZClass c: return "<class " + c.name + ">";
            case ZEnum e: return "<enum " + e.name + ">";
            case Constructor c: return "<case " + c.def.cases[c.index] + ">";
            case ErrorType e: return "<error " + e.kind + ">";
            case Module m: return "<module " + m.name + ">";
            case Func f: return f.name.Length == 0 ? "<function>" : "<function " + f.name + ">";
        }
        return "?";
    }

    static string repr(object v) => v is string s ? quoted(s) : show(v);

    // -------------------------------------------------------------- Units

    static readonly System.Collections.Generic.Dictionary<string, (string kind, double size)> UNITS = MakeUnits();

    static System.Collections.Generic.Dictionary<string, (string, double)> MakeUnits()
    {
        var u = new System.Collections.Generic.Dictionary<string, (string, double)>();
        void add(string kind, params object[] pairs)
        {
            for (int i = 0; i < pairs.Length; i += 2) u[(string)pairs[i]] = (kind, System.Convert.ToDouble(pairs[i + 1], C));
        }
        add("time", "ns", 1, "us", 1e3, "ms", 1e6, "s", 1e9, "min", 60e9, "h", 3600e9);
        add("frequency", "Hz", 1, "kHz", 1e3, "MHz", 1e6);
        add("voltage", "uV", 1, "mV", 1e3, "V", 1e6, "kV", 1e9);
        add("current", "uA", 1, "mA", 1e3, "A", 1e6);
        add("resistance", "ohm", 1, "kohm", 1e3, "Mohm", 1e6);
        add("power", "mW", 1, "W", 1e3, "kW", 1e6);
        add("energy", "J", 1, "Wh", 3600, "kWh", 3600e3);
        add("temperature in Celsius", "degC", 1);
        add("temperature in kelvin", "K", 1);
        add("length", "mm", 1, "cm", 10, "m", 1e3, "km", 1e6);
        add("mass", "mg", 1, "g", 1e3, "kg", 1e6);
        add("angle in degrees", "deg", 1);
        add("angle in radians", "rad", 1);
        add("rotation", "rpm", 1);
        return u;
    }

    // How to turn a number in one unit into another: {times, over}, or null.
    static double[] convert(string from, string to)
    {
        if (from == "deg" && to == "rad") return new[] { System.Math.PI / 180, 1 };
        if (from == "rad" && to == "deg") return new[] { 180 / System.Math.PI, 1 };
        if (!UNITS.TryGetValue(from, out var f) || !UNITS.TryGetValue(to, out var t) || f.kind != t.kind) return null;
        return f.size >= t.size ? new[] { f.size / t.size, 1 } : new[] { 1, t.size / f.size };
    }

    static double apply(double[] scale, double x) => x * scale[0] / scale[1];

    // The unit two quantities meet in, the finer one, or null.
    static string common(string a, string b)
    {
        if (!UNITS.TryGetValue(a, out var ua) || !UNITS.TryGetValue(b, out var ub) || ua.kind != ub.kind) return null;
        return ua.size <= ub.size ? a : b;
    }

    public static object unit(object v, string name, int line)
    {
        if (!UNITS.ContainsKey(name))
        {
            throw fatal("`" + name + "` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC", line);
        }
        if (!isNumber(v)) throw fatal("a unit can only follow a number", line);
        return new Quantity(number(v), name);
    }

    // ------------------------------------------------------------ Equality

    static bool equal(object a, object b)
    {
        if (a is long x && b is long y) return x == y;
        if (isNumber(a) && isNumber(b)) return number(a) == number(b);
        if (a is string s && b is string t) return s == t;
        if (a is bool p && b is bool q) return p == q;
        if (a == null || b == null) return a == b;
        if (a is Quantity qa && b is Quantity qb)
        {
            string u = common(qa.unit, qb.unit);
            return u != null && apply(convert(qa.unit, u), qa.v) == apply(convert(qb.unit, u), qb.v);
        }
        if (a is ZList la && b is ZList lb)
        {
            if (la.items.Count != lb.items.Count) return false;
            for (int i = 0; i < la.items.Count; i++) if (!equal(la.items[i], lb.items[i])) return false;
            return true;
        }
        if (a is ZDict da && b is ZDict db)
        {
            if (da.keys.Count != db.keys.Count) return false;
            foreach (var k in da.keys)
            {
                if (!db.map.TryGetValue(k, out var other) || !equal(da.map[k], other)) return false;
            }
            return true;
        }
        if (a is Instance ia && b is Instance ib)
        {
            if (ia.cls != ib.cls) return false;
            for (int i = 0; i < ia.fields.Length; i++) if (!equal(ia.fields[i], ib.fields[i])) return false;
            return true;
        }
        if (a is Case ca && b is Case cb)
        {
            if (ca.def != cb.def || ca.index != cb.index || ca.fields.Length != cb.fields.Length) return false;
            for (int i = 0; i < ca.fields.Length; i++) if (!equal(ca.fields[i], cb.fields[i])) return false;
            return true;
        }
        if (a is ZError ea && b is ZError eb) return ea.kind == eb.kind && ea.message == eb.message;
        if (a is Constructor ka && b is Constructor kb) return ka.def == kb.def && ka.index == kb.index;
        return ReferenceEquals(a, b);
    }

    public static bool eq(object a, object b) => equal(a, b);

    public static bool ne(object a, object b) => !equal(a, b);

    // ----------------------------------------------------------- Operators

    static System.Exception mismatch(string op, object a, object b, int line) =>
        fatal("`" + op + "` does not work between " + typeName(a) + " and " + typeName(b), line);

    static long addInt(long a, long b, int line)
    {
        long r = unchecked(a + b);
        if (((a ^ r) & (b ^ r)) < 0) throw overflow(line);
        return r;
    }

    static long mulInt(long a, long b, int line)
    {
        long hi = System.Math.BigMul(a, b, out long lo);
        if ((hi != 0 || lo < 0) && (hi != -1 || lo >= 0)) throw overflow(line);
        return lo;
    }

    public static object add(object a, object b, int line)
    {
        if (a is long x && b is long y) return addInt(x, y, line);
        if (a is string s && b is string t) return s + t;
        if (a is ZList la && b is ZList lb)
        {
            var items = new System.Collections.Generic.List<object>(la.items.Count + lb.items.Count);
            items.AddRange(la.items);
            items.AddRange(lb.items);
            return new ZList(items);
        }
        if (a is Quantity qa && b is Quantity qb)
        {
            string u = common(qa.unit, qb.unit);
            if (u == null) throw mismatch("+", a, b, line);
            return new Quantity(apply(convert(qa.unit, u), qa.v) + apply(convert(qb.unit, u), qb.v), u);
        }
        if (!isNumber(a) || !isNumber(b)) throw mismatch("+", a, b, line);
        return number(a) + number(b);
    }

    public static object sub(object a, object b, int line)
    {
        if (a is long x && b is long y)
        {
            long r = unchecked(x - y);
            if (((x ^ y) & (x ^ r)) < 0) throw overflow(line);
            return r;
        }
        if (a is Quantity qa && b is Quantity qb)
        {
            string u = common(qa.unit, qb.unit);
            if (u == null) throw mismatch("-", a, b, line);
            return new Quantity(apply(convert(qa.unit, u), qa.v) - apply(convert(qb.unit, u), qb.v), u);
        }
        if (!isNumber(a) || !isNumber(b)) throw mismatch("-", a, b, line);
        return number(a) - number(b);
    }

    public static object mul(object a, object b, int line)
    {
        if (a is long x && b is long y) return mulInt(x, y, line);
        if (a is Quantity || b is Quantity)
        {
            Quantity q = a as Quantity ?? (Quantity)b;
            object other = a is Quantity ? b : a;
            if (!isNumber(other)) throw mismatch("*", a, b, line);
            return new Quantity(q.v * number(other), q.unit);
        }
        if (!isNumber(a) || !isNumber(b)) throw mismatch("*", a, b, line);
        return number(a) * number(b);
    }

    public static object div(object a, object b, int line)
    {
        if (a is Quantity q)
        {
            if (b is Quantity bq && bq.v == 0) throw byZero(line);
            if (b is double bd && bd == 0) throw byZero(line);
            if (b is long bl && bl == 0) throw byZero(line);
            if (b is Quantity y)
            {
                string u = common(q.unit, y.unit);
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

    public static object floordiv(object a, object b, int line)
    {
        if (a is long x && b is long y)
        {
            if (y == 0) throw byZero(line);
            if (x == long.MinValue && y == -1) throw overflow(line);
            long q = x / y;
            if (x % y != 0 && (x < 0) != (y < 0)) q--;
            return q;
        }
        if (!isNumber(a) || !isNumber(b)) throw mismatch("//", a, b, line);
        if (number(b) == 0) throw byZero(line);
        return System.Math.Floor(number(a) / number(b));
    }

    public static object mod(object a, object b, int line)
    {
        if (a is long x && b is long y)
        {
            if (y == 0) throw byZero(line);
            if (y == -1) return 0L;
            long r = x % y;
            if (r != 0 && (r < 0) != (y < 0)) r += y;
            return r;
        }
        if (!isNumber(a) || !isNumber(b)) throw mismatch("%", a, b, line);
        double fx = number(a), fy = number(b);
        if (fy == 0) throw byZero(line);
        double fq = fx / fy;
        if (double.IsNaN(fq) || double.IsInfinity(fq)) return fx - fy * fq;
        return fx - fy * System.Math.Floor(fq);
    }

    public static object pow(object a, object b, int line)
    {
        if (a is long x && b is long n)
        {
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
        double fx = number(a), fy = number(b);
        if (fx == 1 || fy == 0) return 1.0;
        return System.Math.Pow(fx, fy);
    }

    static System.Exception bitsMismatch(string op, object a, object b, int line)
    {
        string hint = "";
        if (a is bool && b is bool)
        {
            if (op == "&") hint = ". For two bools Zeno spells this `and`";
            if (op == "|") hint = ". For two bools Zeno spells this `or`";
        }
        return fatal("`" + op + "` works on the bits of whole numbers, and these are " + typeName(a) + " and " + typeName(b) + hint, line);
    }

    static long whole(string op, object v, object a, object b, int line)
    {
        if (v is long l) return l;
        throw bitsMismatch(op, a, b, line);
    }

    public static object bitand(object a, object b, int line) => whole("&", a, a, b, line) & whole("&", b, a, b, line);

    public static object bitor(object a, object b, int line) => whole("|", a, a, b, line) | whole("|", b, a, b, line);

    public static object bitxor(object a, object b, int line) => whole("^", a, a, b, line) ^ whole("^", b, a, b, line);

    static void shift(long n, int line)
    {
        if (n < 0 || n >= 64) throw raiseError("ValueError", "cannot shift by " + n + ". An int has 64 bits, so a shift is 0 to 63", line);
    }

    public static object shl(object a, object b, int line)
    {
        long x = whole("<<", a, a, b, line), n = whole("<<", b, a, b, line);
        shift(n, line);
        long s = x << (int)n;
        if (s >> (int)n != x) throw overflow(line);
        return s;
    }

    public static object shr(object a, object b, int line)
    {
        long x = whole(">>", a, a, b, line), n = whole(">>", b, a, b, line);
        shift(n, line);
        return x >> (int)n;
    }

    public static object neg(object v, int line)
    {
        if (v is long x)
        {
            if (x == long.MinValue) throw overflow(line);
            return -x;
        }
        if (v is double d) return -d;
        if (v is Quantity q) return new Quantity(-q.v, q.unit);
        throw fatal("`-` does not work on " + typeName(v), line);
    }

    public static object bitnot(object v, int line)
    {
        if (v is long x) return ~x;
        throw fatal("`~` works on the bits of a whole number, and this is " + typeName(v), line);
    }

    // -1, 0 or 1, or 2 when the two cannot be ordered (a NaN).
    static int order(string op, object a, object b, int line)
    {
        if (a is long lx && b is long ly) return lx.CompareTo(ly);
        if (a is string sx && b is string sy)
        {
            // By code points, as Rust compares, not by UTF-16 units.
            int i = 0, j = 0;
            while (i < sx.Length && j < sy.Length)
            {
                int c = codePointAt(sx, i), d = codePointAt(sy, j);
                if (c != d) return c < d ? -1 : 1;
                i += c > 0xffff ? 2 : 1;
                j += d > 0xffff ? 2 : 1;
            }
            return i < sx.Length ? 1 : j < sy.Length ? -1 : 0;
        }
        double x, y;
        if (a is Quantity qa && b is Quantity qb)
        {
            string u = common(qa.unit, qb.unit);
            if (u == null) throw mismatch(op, a, b, line);
            x = apply(convert(qa.unit, u), qa.v);
            y = apply(convert(qb.unit, u), qb.v);
        }
        else
        {
            if (!isNumber(a) || !isNumber(b)) throw mismatch(op, a, b, line);
            x = number(a);
            y = number(b);
        }
        if (x < y) return -1;
        if (x > y) return 1;
        if (x == y) return 0;
        return 2;
    }

    public static bool lt(object a, object b, int line) => order("<", a, b, line) == -1;

    public static bool le(object a, object b, int line) { int o = order("<=", a, b, line); return o == -1 || o == 0; }

    public static bool gt(object a, object b, int line) => order(">", a, b, line) == 1;

    public static bool ge(object a, object b, int line) { int o = order(">=", a, b, line); return o == 1 || o == 0; }

    static bool contains(string op, object item, object container, int line)
    {
        if (container is ZList l)
        {
            foreach (var other in l.items) if (equal(item, other)) return true;
            return false;
        }
        if (container is ZDict d)
        {
            if (!(item is long) && !(item is string)) throw mismatch(op, item, container, line);
            return d.map.ContainsKey(item);
        }
        if (container is string c && item is string s) return c.Contains(s, System.StringComparison.Ordinal);
        throw mismatch(op, item, container, line);
    }

    public static bool @in(object a, object b, int line) => contains("in", a, b, line);

    public static bool not_in(object a, object b, int line) => !contains("not in", a, b, line);

    public static bool truth(object v, int line)
    {
        if (v is bool b) return b;
        throw fatal("a condition must be true or false, but this is " + typeName(v), line);
    }

    // ------------------------------------------------------ Lists and text

    static object key(object v, int line)
    {
        if (v is long || v is string) return v;
        throw fatal("a key of a dict is a str or an int, but this is " + typeName(v), line);
    }

    static int position(object index, int length, string what, int line)
    {
        if (!(index is long i)) throw fatal("an index must be an int, but this is " + typeName(index), line);
        if (i < 0 || i >= length) throw raiseError("IndexError", "index " + i + " is out of range for a " + what + " of length " + length, line);
        return (int)i;
    }

    public static object index(object obj, object i, int line)
    {
        if (obj is ZList l) return l.items[position(i, l.items.Count, "list", line)];
        if (obj is string s)
        {
            int at = position(i, textLength(s), "str", line);
            return sub(s, at, at + 1);
        }
        if (obj is ZDict d)
        {
            object k = key(i, line);
            if (!d.map.TryGetValue(k, out var v)) throw raiseError("KeyError", "this dict has no key " + repr(i), line);
            return v;
        }
        throw fatal("cannot take an item from " + typeName(obj), line);
    }

    public static void set_index(object obj, object i, object value, int line)
    {
        if (obj is ZList l) { l.items[position(i, l.items.Count, "list", line)] = value; return; }
        if (obj is ZDict d) { d.Set(key(i, line), value); return; }
        throw fatal("cannot set an item of " + typeName(obj), line);
    }

    public static System.Collections.Generic.List<object> items(object v, int line)
    {
        if (v is ZList l) return new System.Collections.Generic.List<object>(l.items);
        if (v is ZDict d) return new System.Collections.Generic.List<object>(d.keys);
        if (v is string s) return new System.Collections.Generic.List<object>(codePoints(s));
        throw fatal("cannot loop over " + typeName(v), line);
    }

    public static System.Collections.Generic.List<object[]> pairs(object v, int line)
    {
        var out_ = new System.Collections.Generic.List<object[]>();
        if (v is ZDict d)
        {
            foreach (var k in d.keys) out_.Add(new[] { k, d.map[k] });
            return out_;
        }
        var all = items(v, line);
        for (int i = 0; i < all.Count; i++) out_.Add(new[] { (object)(long)i, all[i] });
        return out_;
    }

    public static object list(object[] values) => new ZList(new System.Collections.Generic.List<object>(values));

    public static object dict(object[] flat, int line)
    {
        var d = new ZDict();
        for (int i = 0; i < flat.Length; i += 2) d.Set(key(flat[i], line), flat[i + 1]);
        return d;
    }

    public static object push(object list, object value)
    {
        ((ZList)list).items.Add(value);
        return null;
    }

    public static object concat(object[] pieces)
    {
        var b = new System.Text.StringBuilder();
        foreach (var p in pieces) b.Append((string)p);
        return b.ToString();
    }

    // ------------------------------------------------- Objects and fields

    public static Func fn(string name, string[] parameters, bool[] defaults, Body body) => new Func(name, parameters, defaults, body, null, null);

    public static System.Collections.Generic.Dictionary<string, Func> methods(params object[] pairs)
    {
        var out_ = new System.Collections.Generic.Dictionary<string, Func>();
        for (int i = 0; i < pairs.Length; i += 2) out_[(string)pairs[i]] = (Func)pairs[i + 1];
        return out_;
    }

    public static object cls(string name, string[] fields, object[] defaults, System.Collections.Generic.Dictionary<string, Func> methods) =>
        new ZClass(name, fields, defaults, methods);

    public static object enumOf(string name, string[] cases, string[][] fields) => new ZEnum(name, cases, fields);

    public static object @case(object def, int i)
    {
        var e = (ZEnum)def;
        return e.fields[i].Length == 0 ? new Case(e, i, new object[0]) : new Constructor(e, i);
    }

    public static bool is_case(object value, object ctor, int count, int line)
    {
        if (!(ctor is Constructor c)) throw fatal("this is not an enum case that carries values", line);
        if (!(value is Case v) || v.def != c.def || v.index != c.index) return false;
        if (v.fields.Length != count)
        {
            throw fatal("`" + c.def.cases[c.index] + "` carries " + v.fields.Length + " values, but this pattern has " + count, line);
        }
        return true;
    }

    public static object case_field(object value, int i) => ((Case)value).fields[i];

    static int indexOf(string[] names, string name) => System.Array.IndexOf(names, name);

    public static object field(object obj, string name, int line)
    {
        if (obj is Instance o)
        {
            int at = indexOf(o.cls.fields, name);
            if (at >= 0) return o.fields[at];
            if (o.cls.methods.TryGetValue(name, out var m)) return new Bound(o, m);
            throw fatal("`" + o.cls.name + "` has no field or method `" + name + "`", line);
        }
        if (obj is Case c)
        {
            int at = indexOf(c.def.fields[c.index], name);
            if (at >= 0) return c.fields[at];
            throw fatal("`" + c.def.cases[c.index] + "` has no field `" + name + "`", line);
        }
        if (obj is ZEnum e)
        {
            int at = indexOf(e.cases, name);
            if (at >= 0) return @case(e, at);
            throw fatal("enum `" + e.name + "` has no case `" + name + "`", line);
        }
        if (obj is Module mod) return builtin(mod.name + "." + name, line);
        if (obj is Quantity q && UNITS.ContainsKey(name))
        {
            var s = convert(q.unit, name);
            if (s == null) throw fatal("a quantity in " + q.unit + " cannot be turned into " + name + ". They measure different things", line);
            return new Quantity(apply(s, q.v), name);
        }
        if (obj is ZError err && name == "message") return err.message;
        if (obj is ZError err2 && name == "kind") return err2.kind;
        if (obj is Func f && f.builtin != null && WIDTHS.ContainsKey(f.builtin) && name == "wrap") return builtin(f.builtin + ".wrap", line);
        if (hasMethod(obj, name)) return new Method(obj, name);
        throw fatal(typeName(obj) + " has no field or method `" + name + "`", line);
    }

    public static void set_field(object obj, string name, object value, int line)
    {
        if (obj is Instance o)
        {
            int at = indexOf(o.cls.fields, name);
            if (at < 0) throw fatal("`" + o.cls.name + "` has no field `" + name + "`", line);
            o.fields[at] = value;
            return;
        }
        throw fatal("cannot set a field on " + typeName(obj), line);
    }

    // -------------------------------------------------------------- Calls

    static string counted(int n, string word) => n == 1 ? "1 " + word : n + " " + word + "s";

    static string were(int n) => n == 1 ? "1 was" : n + " were";

    static object[] bind(string what, string[] names, object[] positional, string[] namedNames, object[] named, int line)
    {
        if (positional.Length > names.Length)
        {
            throw fatal("`" + what + "` takes " + counted(names.Length, "argument") + ", but " + were(positional.Length) + " given", line);
        }
        var slots = new object[names.Length];
        for (int i = 0; i < names.Length; i++) slots[i] = i < positional.Length ? positional[i] : MISSING;
        for (int i = 0; i < namedNames.Length; i++)
        {
            int at = indexOf(names, namedNames[i]);
            if (at < 0) throw fatal("`" + what + "` has no argument named `" + namedNames[i] + "`", line);
            if (slots[at] != MISSING) throw fatal("`" + what + "` was given `" + namedNames[i] + "` twice", line);
            slots[at] = named[i];
        }
        return slots;
    }

    // How many calls are nested now. `zeno run` stops at 1000.
    static int depth = 0;

    static void enter(Func f, int line)
    {
        if (depth >= 1000)
        {
            string name = f.name.Length == 0 ? "function" : f.name;
            throw fatal("more than 1000 calls are nested. Is `" + name + "` calling itself forever?", line);
        }
        depth++;
    }

    // A call of a function of the program known where it is written, with
    // its arguments lined up already.
    public static object direct(object f, int line, object[] args)
    {
        var fn = (Func)f;
        enter(fn, line);
        try { return fn.body(args); }
        finally { depth--; }
    }

    public static readonly string[] NO_NAMES = new string[0];
    public static readonly object[] NO_VALUES = new object[0];

    static object callFunction(Func f, object[] positional, string[] namedNames, object[] named, int line)
    {
        string name = f.name.Length == 0 ? "function" : f.name;
        enter(f, line);
        try
        {
            var slots = bind(name, f.parameters, positional, namedNames, named, line);
            for (int i = 0; i < slots.Length; i++)
            {
                if (slots[i] == MISSING && !f.defaults[i]) throw fatal("`" + name + "` needs a value for `" + f.parameters[i] + "`", line);
            }
            return f.body(slots);
        }
        finally { depth--; }
    }

    static void noNamed(string what, string[] namedNames, int line)
    {
        if (namedNames.Length > 0) throw fatal("`" + what + "` does not take a named argument `" + namedNames[0] + "`", line);
    }

    static object[] prepend(object first, object[] rest)
    {
        var out_ = new object[rest.Length + 1];
        out_[0] = first;
        System.Array.Copy(rest, 0, out_, 1, rest.Length);
        return out_;
    }

    public static object call_named(object callee, object[] positional, string[] namedNames, object[] named, int line)
    {
        switch (callee)
        {
            case Bound b:
                return callFunction(b.fn, prepend(b.obj, positional), namedNames, named, line);
            case Method m:
                noNamed(m.name, namedNames, line);
                return callMethodOf(m.receiver, m.name, positional, line);
            case ZClass c:
                {
                    var slots = bind(c.name, c.fields, positional, namedNames, named, line);
                    for (int i = 0; i < slots.Length; i++)
                    {
                        if (slots[i] == MISSING)
                        {
                            if (c.defaults[i] == null) throw fatal("`" + c.name + "` needs a value for `" + c.fields[i] + "`", line);
                            slots[i] = ((Func)c.defaults[i]).body(NO_VALUES);
                        }
                    }
                    return new Instance(c, slots);
                }
            case Constructor k:
                {
                    string name = k.def.cases[k.index];
                    string[] fields = k.def.fields[k.index];
                    var slots = bind(name, fields, positional, namedNames, named, line);
                    for (int i = 0; i < slots.Length; i++)
                    {
                        if (slots[i] == MISSING) throw fatal("`" + name + "` needs a value for `" + fields[i] + "`", line);
                    }
                    return new Case(k.def, k.index, slots);
                }
            case ErrorType e:
                noNamed(e.kind, namedNames, line);
                return newError(e.kind, positional, line);
            case Func f:
                if (f.bbody != null)
                {
                    noNamed(f.name, namedNames, line);
                    return f.bbody(positional, line);
                }
                return callFunction(f, positional, namedNames, named, line);
        }
        throw fatal(typeName(callee) + " cannot be called", line);
    }

    public static object call(object callee, object[] args, int line) => call_named(callee, args, NO_NAMES, NO_VALUES, line);

    static object newError(string kind, object[] args, int line)
    {
        if (args.Length > 1) throw fatal("`" + kind + "` takes one message", line);
        return new ZError(kind, args.Length == 1 ? show(args[0]) : "", null);
    }

    public static object call_method_named(object obj, string name, object[] positional, string[] namedNames, object[] named, int line)
    {
        if (obj is Instance o && indexOf(o.cls.fields, name) < 0 && o.cls.methods.TryGetValue(name, out var m))
        {
            return callFunction(m, prepend(o, positional), namedNames, named, line);
        }
        if (obj is Module mod)
        {
            noNamed(name, namedNames, line);
            return call_builtin(mod.name + "." + name, positional, line);
        }
        if (obj is string || obj is ZList || obj is ZDict)
        {
            noNamed(name, namedNames, line);
            return callMethodOf(obj, name, positional, line);
        }
        return call_named(field(obj, name, line), positional, namedNames, named, line);
    }

    public static object call_method(object obj, string name, object[] args, int line) => call_method_named(obj, name, args, NO_NAMES, NO_VALUES, line);

    // ------------------------------------------------------------- Errors

    public static void raise(object value, int line)
    {
        if (value is ZError e) throw new Raise(e, line);
        if (value is Instance o) throw new Raise(new ZError(o.cls.name, show(o), o), line);
        throw fatal("only an error can be raised, and this is " + typeName(value), line);
    }

    public static bool catches(Raise e, string name) => name == "Error" || name == e.value.kind;

    public static object caught(Raise e, string name)
    {
        if (name.Length > 0 && name != "Error" && e.value.obj != null) return e.value.obj;
        return e.value;
    }

    public static void no_match(object v, int line) => throw fatal("no `case` matches " + repr(v), line);

    public static object undefined(string name, int line) => throw fatal("`" + name + "` is not defined", line);

    // An expression whose value is not wanted.
    public static void ignore(object v) { }

    // ------------------------------------------------- Built-in functions

    static void arity(string name, object[] args, int lo, int hi, int line)
    {
        if (args.Length >= lo && args.Length <= hi) return;
        string wanted = lo == hi ? counted(lo, "argument") : lo + " to " + hi + " arguments";
        throw fatal("`" + name + "` takes " + wanted + ", but " + were(args.Length) + " given", line);
    }

    static System.Exception wrongType(string name, string wanted, object got, int line) =>
        fatal("`" + name + "` needs " + wanted + ", but this is " + typeName(got), line);

    static string text(string name, object v, int line) => v is string s ? s : throw wrongType(name, "a str", v, line);

    static long wholeArg(string name, object v, int line) => v is long n ? n : throw wrongType(name, "a whole number", v, line);

    static bool blank(char c) => c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\f';

    static string strip(string s)
    {
        int a = 0, b = s.Length;
        while (a < b && blank(s[a])) a++;
        while (b > a && blank(s[b - 1])) b--;
        return s.Substring(a, b - a);
    }

    static readonly System.IO.Stream STDOUT = System.Console.OpenStandardOutput();
    static readonly System.IO.StreamWriter OUT = new System.IO.StreamWriter(STDOUT, new System.Text.UTF8Encoding(false), 65536) { AutoFlush = false, NewLine = "\n" };

    static void flush() => OUT.Flush();

    static readonly System.Collections.Generic.Dictionary<string, (long lo, long hi, int bits, bool signed)> WIDTHS = new()
    {
        ["u8"] = (0, 255, 8, false),
        ["u16"] = (0, 65535, 16, false),
        ["u32"] = (0, 4294967295L, 32, false),
        ["i8"] = (-128, 127, 8, true),
        ["i16"] = (-32768, 32767, 16, true),
        ["i32"] = (-2147483648L, 2147483647L, 32, true),
    };

    static double amount(object v) => v is Quantity q ? q.v : number(v);

    static System.Collections.Generic.IList<object> numbers(string name, object[] args, int line, out string unit)
    {
        System.Collections.Generic.IList<object> values = args.Length == 1 && args[0] is ZList l ? l.items : args;
        unit = null;
        for (int place = 0; place < values.Count; place++)
        {
            object item = values[place];
            if (item is Quantity q)
            {
                if (unit != null && unit != q.unit)
                {
                    throw fatal("`" + name + "` needs quantities in one unit, but these are in " + unit + " and in " + q.unit + ". Turn one into the other first, as in `x." + unit + "`", line);
                }
                if (unit == null && place > 0) throw fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line);
                unit = q.unit;
            }
            else if (!isNumber(item)) throw wrongType(name, "numbers", item, line);
            else if (unit != null) throw fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line);
        }
        return values;
    }

    static object pick(string name, object[] args, int line, bool smaller)
    {
        var values = numbers(name, args, line, out _);
        if (values.Count == 0) throw raiseError("ValueError", "`" + name + "` of nothing", line);
        object best = values[0];
        for (int i = 1; i < values.Count; i++)
        {
            object item = values[i];
            if (smaller ? amount(item) < amount(best) : amount(item) > amount(best)) best = item;
        }
        return best;
    }

    static readonly System.Text.RegularExpressions.Regex FLOAT = new(
        "^[+-]?([0-9]+\\.?[0-9]*([eE][+-]?[0-9]+)?|\\.[0-9]+([eE][+-]?[0-9]+)?|(?i:inf|infinity|nan))$");

    // A float as Rust's `parse::<f64>` reads it, or null.
    static double? parseFloat(string s)
    {
        if (!FLOAT.IsMatch(s) || s.EndsWith("\n")) return null;
        string body = s.TrimStart('+', '-').ToLowerInvariant();
        bool negative = s.StartsWith("-");
        if (body == "inf" || body == "infinity") return negative ? double.NegativeInfinity : double.PositiveInfinity;
        if (body == "nan") return double.NaN;
        return double.Parse(s, System.Globalization.NumberStyles.Float, C);
    }

    // Heapsort, step for step as the Rust code sorts, so that a function
    // given to `sort` is called in the same order.
    static void heapsort(int n, System.Func<int, int, bool> before, System.Action<int, int> swap)
    {
        void sift(int root, int end)
        {
            while (true)
            {
                int child = 2 * root + 1;
                if (child >= end) return;
                if (child + 1 < end && before(child, child + 1)) child++;
                if (!before(root, child)) return;
                swap(root, child);
                root = child;
            }
        }
        for (int root = n / 2 - 1; root >= 0; root--) sift(root, n);
        for (int end = n - 1; end > 0; end--)
        {
            swap(0, end);
            sift(0, end);
        }
    }

    static void swapIn<T>(System.Collections.Generic.List<T> l, int i, int j) { var t = l[i]; l[i] = l[j]; l[j] = t; }

    static void sortList(ZList l, int line)
    {
        var s = new System.Collections.Generic.List<object>(l.items);
        heapsort(s.Count, (i, j) => lt(s[i], s[j], line), (i, j) => swapIn(s, i, j));
        l.items = s;
    }

    static void sortBy(ZList l, object function, int line)
    {
        var keys = new System.Collections.Generic.List<object>();
        foreach (var item in new System.Collections.Generic.List<object>(l.items))
        {
            object k = call(function, new[] { item }, line);
            if (!(k is long) && !(k is double) && !(k is string))
            {
                throw fatal("the function given to `sort` must return a number or text to order by, but it returned " + typeName(k), line);
            }
            keys.Add(k);
        }
        var s = new System.Collections.Generic.List<object>(l.items);
        if (s.Count != keys.Count) throw raiseError("ValueError", "the function given to `sort` changed how many items the list has", line);
        var places = new System.Collections.Generic.List<int>();
        for (int i = 0; i < s.Count; i++) places.Add(i);
        heapsort(s.Count, (a, b) =>
        {
            if (lt(keys[a], keys[b], line)) return true;
            if (lt(keys[b], keys[a], line)) return false;
            return places[a] < places[b];
        }, (i, j) =>
        {
            swapIn(s, i, j);
            swapIn(keys, i, j);
            swapIn(places, i, j);
        });
        l.items = s;
    }

    static (int, int) slice(object[] args, int length, string what, int line)
    {
        long from = wholeArg("slice", args[0], line);
        long to = args.Length > 1 ? wholeArg("slice", args[1], line) : length;
        if (from < 0 || to < from || to > length)
        {
            throw raiseError("IndexError", "slice " + from + " to " + to + " is out of range for a " + what + " of length " + length, line);
        }
        return ((int)from, (int)to);
    }

    static readonly string[] STR_METHODS = { "upper", "lower", "strip", "split", "lines", "replace", "starts_with", "ends_with", "find", "slice", "code" };
    static readonly string[] LIST_METHODS = { "append", "pop", "map", "filter", "reduce", "insert", "reverse", "sort", "clear", "find", "slice", "join" };
    static readonly string[] DICT_METHODS = { "get", "pop", "keys", "values", "clear" };

    static bool hasMethod(object v, string name) => v switch
    {
        string => System.Array.IndexOf(STR_METHODS, name) >= 0,
        ZList => System.Array.IndexOf(LIST_METHODS, name) >= 0,
        ZDict => System.Array.IndexOf(DICT_METHODS, name) >= 0,
        _ => false,
    };

    static ZList strings(System.Collections.Generic.IEnumerable<string> parts)
    {
        var items = new System.Collections.Generic.List<object>();
        foreach (var p in parts) items.Add(p);
        return new ZList(items);
    }

    static object callMethodOf(object receiver, string name, object[] args, int line)
    {
        if (receiver is string s)
        {
            switch (name)
            {
                case "upper":
                case "lower":
                    {
                        arity(name, args, 0, 0, line);
                        var cs = s.ToCharArray();
                        for (int i = 0; i < cs.Length; i++)
                        {
                            if (name == "upper" && cs[i] >= 'a' && cs[i] <= 'z') cs[i] = (char)(cs[i] - 32);
                            else if (name == "lower" && cs[i] >= 'A' && cs[i] <= 'Z') cs[i] = (char)(cs[i] + 32);
                        }
                        return new string(cs);
                    }
                case "strip":
                    arity(name, args, 0, 0, line);
                    return strip(s);
                case "split":
                    {
                        arity(name, args, 0, 1, line);
                        if (args.Length == 0)
                        {
                            var words = new System.Collections.Generic.List<string>();
                            int i = 0;
                            while (i < s.Length)
                            {
                                while (i < s.Length && blank(s[i])) i++;
                                int start = i;
                                while (i < s.Length && !blank(s[i])) i++;
                                if (i > start) words.Add(s.Substring(start, i - start));
                            }
                            return strings(words);
                        }
                        string sep = text(name, args[0], line);
                        if (sep.Length == 0) throw raiseError("ValueError", "cannot split on an empty str", line);
                        return strings(s.Split(sep));
                    }
                case "lines":
                    {
                        arity(name, args, 0, 0, line);
                        if (s.Length == 0) return new ZList(new System.Collections.Generic.List<object>());
                        var parts = new System.Collections.Generic.List<string>(s.Split('\n'));
                        if (parts[parts.Count - 1].Length == 0) parts.RemoveAt(parts.Count - 1);
                        for (int i = 0; i < parts.Count; i++) if (parts[i].EndsWith("\r")) parts[i] = parts[i].Substring(0, parts[i].Length - 1);
                        return strings(parts);
                    }
                case "replace":
                    {
                        arity(name, args, 2, 2, line);
                        string old = text(name, args[0], line), now = text(name, args[1], line);
                        if (old.Length == 0)
                        {
                            var b = new System.Text.StringBuilder(now);
                            foreach (var c in codePoints(s)) b.Append(c).Append(now);
                            return b.ToString();
                        }
                        return s.Replace(old, now, System.StringComparison.Ordinal);
                    }
                case "starts_with":
                    arity(name, args, 1, 1, line);
                    return s.StartsWith(text(name, args[0], line), System.StringComparison.Ordinal);
                case "ends_with":
                    arity(name, args, 1, 1, line);
                    return s.EndsWith(text(name, args[0], line), System.StringComparison.Ordinal);
                case "find":
                    {
                        arity(name, args, 1, 1, line);
                        int at = s.IndexOf(text(name, args[0], line), System.StringComparison.Ordinal);
                        return at < 0 ? null : (object)(long)textLength(s.Substring(0, at));
                    }
                case "slice":
                    {
                        arity(name, args, 1, 2, line);
                        var (from, to) = slice(args, textLength(s), "str", line);
                        return sub(s, from, to);
                    }
                case "code":
                    {
                        arity(name, args, 0, 0, line);
                        int n = textLength(s);
                        if (n != 1) throw raiseError("ValueError", "`code` needs a str of exactly one character, and this one has " + n, line);
                        return (long)codePointAt(s, 0);
                    }
            }
            throw fatal("a str has no method `" + name + "`", line);
        }
        if (receiver is ZDict d)
        {
            switch (name)
            {
                case "get":
                    {
                        arity(name, args, 1, 2, line);
                        object k = key(args[0], line);
                        if (d.map.TryGetValue(k, out var v)) return v;
                        return args.Length > 1 ? args[1] : null;
                    }
                case "pop":
                    {
                        arity(name, args, 1, 1, line);
                        object k = key(args[0], line);
                        if (!d.map.TryGetValue(k, out var v)) throw raiseError("KeyError", "this dict has no key " + repr(args[0]), line);
                        d.Remove(k);
                        return v;
                    }
                case "keys":
                    arity(name, args, 0, 0, line);
                    return new ZList(new System.Collections.Generic.List<object>(d.keys));
                case "values":
                    {
                        arity(name, args, 0, 0, line);
                        var values = new System.Collections.Generic.List<object>();
                        foreach (var k in d.keys) values.Add(d.map[k]);
                        return new ZList(values);
                    }
                case "clear":
                    arity(name, args, 0, 0, line);
                    d.keys.Clear();
                    d.map.Clear();
                    return null;
            }
            throw fatal("a dict has no method `" + name + "`", line);
        }
        if (receiver is ZList l)
        {
            switch (name)
            {
                case "append":
                    arity(name, args, 1, 1, line);
                    l.items.Add(args[0]);
                    return null;
                case "pop":
                    {
                        arity(name, args, 0, 1, line);
                        if (args.Length > 0)
                        {
                            int at = position(args[0], l.items.Count, "list", line);
                            var v = l.items[at];
                            l.items.RemoveAt(at);
                            return v;
                        }
                        if (l.items.Count == 0) throw raiseError("IndexError", "cannot pop from an empty list", line);
                        var last = l.items[l.items.Count - 1];
                        l.items.RemoveAt(l.items.Count - 1);
                        return last;
                    }
                case "insert":
                    {
                        arity(name, args, 2, 2, line);
                        long at = wholeArg(name, args[0], line);
                        if (at < 0 || at > l.items.Count)
                        {
                            throw raiseError("IndexError", "cannot insert at " + at + " in a list of length " + l.items.Count, line);
                        }
                        l.items.Insert((int)at, args[1]);
                        return null;
                    }
                case "reverse":
                    arity(name, args, 0, 0, line);
                    l.items.Reverse();
                    return null;
                case "clear":
                    arity(name, args, 0, 0, line);
                    l.items.Clear();
                    return null;
                case "sort":
                    if (args.Length == 1)
                    {
                        sortBy(l, args[0], line);
                        return null;
                    }
                    arity(name, args, 0, 1, line);
                    sortList(l, line);
                    return null;
                case "find":
                    arity(name, args, 1, 1, line);
                    for (int at = 0; at < l.items.Count; at++) if (equal(l.items[at], args[0])) return (long)at;
                    return null;
                case "slice":
                    {
                        arity(name, args, 1, 2, line);
                        var (from, to) = slice(args, l.items.Count, "list", line);
                        return new ZList(l.items.GetRange(from, to - from));
                    }
                case "join":
                    {
                        arity(name, args, 1, 1, line);
                        string sep = text(name, args[0], line);
                        var parts = new System.Collections.Generic.List<string>();
                        foreach (var item in l.items)
                        {
                            if (!(item is string p))
                            {
                                throw fatal("`join` puts text together, and this list holds " + typeName(item) + ". Make each item text first, with `str( )`", line);
                            }
                            parts.Add(p);
                        }
                        return string.Join(sep, parts);
                    }
                case "map":
                    {
                        arity(name, args, 1, 1, line);
                        var out_ = new System.Collections.Generic.List<object>();
                        foreach (var item in new System.Collections.Generic.List<object>(l.items)) out_.Add(call(args[0], new[] { item }, line));
                        return new ZList(out_);
                    }
                case "filter":
                    {
                        arity(name, args, 1, 1, line);
                        var kept = new System.Collections.Generic.List<object>();
                        foreach (var item in new System.Collections.Generic.List<object>(l.items))
                        {
                            object keep = call(args[0], new[] { item }, line);
                            if (!(keep is bool b))
                            {
                                throw fatal("the function given to `filter` must return true or false, but it returned " + typeName(keep), line);
                            }
                            if (b) kept.Add(item);
                        }
                        return new ZList(kept);
                    }
                case "reduce":
                    {
                        arity(name, args, 2, 2, line);
                        object total = args[1];
                        foreach (var item in new System.Collections.Generic.List<object>(l.items)) total = call(args[0], new[] { total, item }, line);
                        return total;
                    }
            }
            throw fatal("a list has no method `" + name + "`", line);
        }
        throw fatal(typeName(receiver) + " has no method `" + name + "`", line);
    }

    static readonly System.Text.RegularExpressions.Regex FORMAT = new("^([<>^])?(0)?(\\d*)(?:\\.(\\d*))?(.)?$", System.Text.RegularExpressions.RegexOptions.Singleline);

    // `{x:>8.2f}`: [<>^]? 0? width? (.precision)? kind?
    public static object format(object v, string spec, int line)
    {
        if (spec.Length == 0) return show(v);
        var m = FORMAT.Match(spec);
        if (!m.Success || (m.Groups[5].Success && char.IsDigit(m.Groups[5].Value[0])))
        {
            throw fatal("`" + spec + "` is not a format Zeno understands", line);
        }
        string align = m.Groups[1].Success ? m.Groups[1].Value : null;
        bool zero = m.Groups[2].Success;
        int width = m.Groups[3].Value.Length == 0 ? 0 : int.Parse(m.Groups[3].Value, C);
        int? precision = m.Groups[4].Success ? (m.Groups[4].Value.Length == 0 ? 0 : int.Parse(m.Groups[4].Value, C)) : null;
        string kind = m.Groups[5].Success ? m.Groups[5].Value : null;
        int places = precision ?? 6;
        string body;
        if ((kind == "f" || (kind == null && precision != null)) && v is Quantity q)
        {
            body = fixedPlaces(q.v, places) + " " + q.unit;
        }
        else if (kind == "f" || (kind == null && precision != null && v is double))
        {
            if (!isNumber(v)) throw wrongType(spec, "a number", v, line);
            body = fixedPlaces(number(v), places);
        }
        else if ((kind == null && precision == null) || (kind == "d" && precision == null && v is long))
        {
            body = show(v);
        }
        else if ((kind == "x" || kind == "X" || kind == "b") && precision == null && v is long n)
        {
            ulong mag = n < 0 ? (ulong)(-(n + 1)) + 1 : (ulong)n;
            string digits = kind == "b" ? System.Convert.ToString((long)mag, 2) : mag.ToString("x", C);
            if (kind == "b" && n == long.MinValue) digits = "1" + new string('0', 63);
            if (kind == "X") digits = digits.ToUpperInvariant();
            body = (n < 0 ? "-" : "") + digits;
        }
        else if ((kind == "d" || kind == "x" || kind == "X" || kind == "b") && precision == null)
        {
            throw wrongType(spec, "an int", v, line);
        }
        else
        {
            throw fatal("`" + spec + "` is not a format Zeno understands", line);
        }
        int padding = System.Math.Max(0, width - textLength(body));
        bool isNum = isNumber(v) || v is Quantity;
        if (zero && isNum)
        {
            if (body.StartsWith("-")) return "-" + new string('0', padding) + body.Substring(1);
            return new string('0', padding) + body;
        }
        string a = align ?? (isNum ? ">" : "<");
        if (a == "<") return body + new string(' ', padding);
        if (a == ">") return new string(' ', padding) + body;
        int left = padding / 2;
        return new string(' ', left) + body + new string(' ', padding - left);
    }

    // -------------------------------------------------------------- JSON

    sealed class NotJson : System.Exception { }

    sealed class Json
    {
        public readonly int[] cs;
        public int at = 0;

        public Json(string s)
        {
            var points = new System.Collections.Generic.List<int>();
            foreach (var c in codePoints(s)) points.Add(codePointAt(c, 0));
            cs = points.ToArray();
        }

        public void spaces() { while (at < cs.Length && (cs[at] == ' ' || cs[at] == '\t' || cs[at] == '\n' || cs[at] == '\r')) at++; }

        public bool word(string w)
        {
            if (at + w.Length > cs.Length) return false;
            for (int i = 0; i < w.Length; i++) if (cs[at + i] != w[i]) return false;
            at += w.Length;
            return true;
        }

        int fourHex()
        {
            if (at + 4 > cs.Length) throw new NotJson();
            int n = 0;
            for (int i = 0; i < 4; i++)
            {
                int c = cs[at + i];
                int d = c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : c >= 'A' && c <= 'F' ? c - 'A' + 10 : -1;
                if (d < 0) throw new NotJson();
                n = n * 16 + d;
            }
            at += 4;
            return n;
        }

        string text()
        {
            if (at >= cs.Length || cs[at] != '"') throw new NotJson();
            at++;
            var b = new System.Text.StringBuilder();
            while (true)
            {
                if (at >= cs.Length) throw new NotJson();
                int c = cs[at++];
                if (c == '"') return b.ToString();
                if (c != '\\') { b.Append(char.ConvertFromUtf32(c)); continue; }
                if (at >= cs.Length) throw new NotJson();
                int e = cs[at++];
                switch (e)
                {
                    case 'n': b.Append('\n'); break;
                    case 't': b.Append('\t'); break;
                    case 'r': b.Append('\r'); break;
                    case 'b': b.Append('\b'); break;
                    case 'f': b.Append('\f'); break;
                    case 'u':
                        {
                            int code = fourHex();
                            if (code >= 0xD800 && code < 0xDC00 && word("\\u"))
                            {
                                int low = fourHex();
                                if (low < 0xDC00) throw new NotJson();
                                code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                            }
                            if ((code >= 0xD800 && code <= 0xDFFF) || code > 0x10FFFF) throw new NotJson();
                            b.Append(char.ConvertFromUtf32(code));
                            break;
                        }
                    default: b.Append(char.ConvertFromUtf32(e)); break;
                }
            }
        }

        public object value()
        {
            spaces();
            if (at >= cs.Length) throw new NotJson();
            int c = cs[at];
            if (c == 'n') { if (word("null")) return null; throw new NotJson(); }
            if (c == 't') { if (word("true")) return true; throw new NotJson(); }
            if (c == 'f') { if (word("false")) return false; throw new NotJson(); }
            if (c == '"') return text();
            if (c == '[')
            {
                at++;
                var out_ = new System.Collections.Generic.List<object>();
                while (true)
                {
                    spaces();
                    if (word("]")) return new ZList(out_);
                    if (out_.Count > 0 && !word(",")) throw new NotJson();
                    out_.Add(value());
                }
            }
            if (c == '{')
            {
                at++;
                var d = new ZDict();
                int count = 0;
                while (true)
                {
                    spaces();
                    if (word("}")) return d;
                    if (count > 0 && !word(",")) throw new NotJson();
                    spaces();
                    string name = text();
                    spaces();
                    if (!word(":")) throw new NotJson();
                    object v = value();
                    count++;
                    if (v != null) d.Set(name, v);
                }
            }
            int start = at;
            while (at < cs.Length && "0123456789+-.eE".IndexOf((char)cs[at]) >= 0 && cs[at] < 128) at++;
            var chars = new System.Text.StringBuilder();
            for (int i = start; i < at; i++) chars.Append((char)cs[i]);
            double? x = parseFloat(chars.ToString());
            if (x == null) throw new NotJson();
            double f = x.Value;
            if (!double.IsInfinity(f) && !double.IsNaN(f) && f == System.Math.Floor(f) && System.Math.Abs(f) < 9.0e15) return (long)f;
            return f;
        }
    }

    static readonly object NOT_JSON = new object();

    static object jsonParse(string s)
    {
        try
        {
            var j = new Json(s);
            object v = j.value();
            j.spaces();
            return j.at == j.cs.Length ? v : NOT_JSON;
        }
        catch (NotJson)
        {
            return NOT_JSON;
        }
    }

    static string jsonText(string s)
    {
        var b = new System.Text.StringBuilder("\"");
        foreach (var ch in codePoints(s))
        {
            int c = codePointAt(ch, 0);
            if (c == '"') b.Append("\\\"");
            else if (c == '\\') b.Append("\\\\");
            else if (c == '\n') b.Append("\\n");
            else if (c == '\r') b.Append("\\r");
            else if (c == '\t') b.Append("\\t");
            else if (c < 0x20) b.Append("\\u").Append(c.ToString("x4", C));
            else b.Append(ch);
        }
        return b.Append('"').ToString();
    }

    static string jsonWrite(object v, int line)
    {
        switch (v)
        {
            case null: return "null";
            case bool b: return b ? "true" : "false";
            case long n: return n.ToString(C);
            case double x:
                if (double.IsNaN(x) || double.IsInfinity(x)) throw raiseError("ValueError", "JSON has no way to write " + showFloat(x), line);
                return showFloat(x);
            case string s: return jsonText(s);
            case ZList l:
                {
                    var parts = new System.Collections.Generic.List<string>();
                    foreach (var item in l.items) parts.Add(jsonWrite(item, line));
                    return "[" + string.Join(",", parts) + "]";
                }
            case ZDict d:
                {
                    var parts = new System.Collections.Generic.List<string>();
                    foreach (var k in d.keys) parts.Add(jsonText(show(k)) + ":" + jsonWrite(d.map[k], line));
                    return "{" + string.Join(",", parts) + "}";
                }
            case Instance o:
                {
                    var parts = new System.Collections.Generic.List<string>();
                    for (int i = 0; i < o.cls.fields.Length; i++) parts.Add(jsonText(o.cls.fields[i]) + ":" + jsonWrite(o.fields[i], line));
                    return "{" + string.Join(",", parts) + "}";
                }
            case Quantity q:
                throw raiseError("ValueError", "JSON has no units, and this is a quantity in " + q.unit + ". Write its number, and put the unit in the name: `[\"" + q.unit + "\": ...]`", line);
        }
        throw raiseError("ValueError", "JSON has objects, lists, text, numbers, bools and none, and this is " + typeName(v), line);
    }

    // -------------------------------------------------------------- Files

    // "file", "nothing" or "other": what is at a path.
    static string at(string path)
    {
        try
        {
            using var f = System.IO.File.OpenRead(path);
            f.ReadByte();
            return "file";
        }
        catch (System.IO.FileNotFoundException) { return "nothing"; }
        catch (System.IO.DirectoryNotFoundException) { return "nothing"; }
        catch (System.Exception) { return "other"; }
    }

    static string[] paths(string name, object[] args, int count, int line)
    {
        arity(name, args, count, count, line);
        var out_ = new string[args.Length];
        for (int i = 0; i < args.Length; i++) out_[i] = text(name, args[i], line);
        return out_;
    }

    static System.Exception fileError(string message, int line) => raiseError("FileError", message, line);

    static readonly System.Diagnostics.Stopwatch STARTED = System.Diagnostics.Stopwatch.StartNew();

    // ------------------------------------------------- The built-in table

    public static object b_print(object[] args, int line)
    {
        for (int i = 0; i < args.Length; i++)
        {
            if (i > 0) OUT.Write(' ');
            OUT.Write(show(args[i]));
        }
        OUT.Write('\n');
        return null;
    }

    static readonly System.IO.Stream IN = new System.IO.BufferedStream(System.Console.OpenStandardInput());

    public static object b_input(object[] args, int line)
    {
        arity("input", args, 0, 1, line);
        if (args.Length > 0) OUT.Write(text("input", args[0], line));
        flush();
        var bytes = new System.IO.MemoryStream();
        bool any = false, ended = false;
        int c;
        while ((c = IN.ReadByte()) >= 0)
        {
            any = true;
            if (c == '\n') { ended = true; break; }
            bytes.WriteByte((byte)c);
        }
        if (!any) return null;
        string s = new System.Text.UTF8Encoding(false).GetString(bytes.ToArray());
        if (ended && s.EndsWith("\r")) s = s.Substring(0, s.Length - 1);
        return s.Replace("\0", "");
    }

    public static object b_len(object[] args, int line)
    {
        arity("len", args, 1, 1, line);
        switch (args[0])
        {
            case string s: return (long)textLength(s);
            case ZList l: return (long)l.items.Count;
            case ZDict d: return (long)d.keys.Count;
        }
        throw wrongType("len", "a str, a list or a dict", args[0], line);
    }

    public static object b_range(object[] args, int line)
    {
        arity("range", args, 1, 3, line);
        var n = new long[args.Length];
        for (int i = 0; i < args.Length; i++) n[i] = args[i] is long x ? x : throw wrongType("range", "ints", args[i], line);
        long start = 0, stop, step = 1;
        if (n.Length == 1) stop = n[0];
        else if (n.Length == 2) { start = n[0]; stop = n[1]; }
        else { start = n[0]; stop = n[1]; step = n[2]; }
        if (step == 0) throw raiseError("ValueError", "the step of a range cannot be 0", line);
        var out_ = new System.Collections.Generic.List<object>();
        for (long i = start; step > 0 ? i < stop : i > stop;)
        {
            out_.Add(i);
            long next = unchecked(i + step);
            if (step > 0 ? next < i : next > i) break;
            i = next;
        }
        return new ZList(out_);
    }

    // A float to an int, as Rust's `as i64` does: NaN is 0, and the ends hold.
    static long toInt(double x)
    {
        if (double.IsNaN(x)) return 0;
        if (x >= 9.223372036854775807e18) return long.MaxValue;
        if (x <= -9.223372036854775808e18) return long.MinValue;
        return (long)x;
    }

    public static object b_int(object[] args, int line)
    {
        arity("int", args, 1, 1, line);
        switch (args[0])
        {
            case long n: return n;
            case double x:
                if (double.IsNaN(x) || double.IsInfinity(x)) throw raiseError("ValueError", "cannot turn " + showFloat(x) + " into an int", line);
                return toInt(x);
            case string s:
                {
                    string t = strip(s).Replace("_", "");
                    if (System.Text.RegularExpressions.Regex.IsMatch(t, "^[+-]?[0-9]+$") && !t.EndsWith("\n")
                        && long.TryParse(t, System.Globalization.NumberStyles.AllowLeadingSign, C, out long v))
                    {
                        return v;
                    }
                    throw raiseError("ValueError", "cannot turn " + quoted(s) + " into an int", line);
                }
        }
        throw wrongType("int", "a number or a str", args[0], line);
    }

    public static object b_float(object[] args, int line)
    {
        arity("float", args, 1, 1, line);
        switch (args[0])
        {
            case long n: return (double)n;
            case double x: return x;
            case string s:
                {
                    double? x = parseFloat(strip(s));
                    if (x == null) throw raiseError("ValueError", "cannot turn " + quoted(s) + " into a float", line);
                    return x.Value;
                }
        }
        throw wrongType("float", "a number or a str", args[0], line);
    }

    public static object b_str(object[] args, int line)
    {
        arity("str", args, 1, 1, line);
        return show(args[0]);
    }

    public static object b_abs(object[] args, int line)
    {
        arity("abs", args, 1, 1, line);
        switch (args[0])
        {
            case long n:
                if (n == long.MinValue) throw overflow(line);
                return System.Math.Abs(n);
            case double x: return System.Math.Abs(x);
            case Quantity q: return new Quantity(System.Math.Abs(q.v), q.unit);
        }
        throw wrongType("abs", "a number", args[0], line);
    }

    public static object b_min(object[] args, int line) => pick("min", args, line, true);

    public static object b_max(object[] args, int line) => pick("max", args, line, false);

    public static object b_sum(object[] args, int line)
    {
        var values = numbers("sum", args, line, out string unit);
        if (unit != null)
        {
            throw fatal("`sum` works on plain numbers, and these have a unit. Add quantities with `+`, which keeps the unit: start from `var total = 0 V` and add each in a loop", line);
        }
        long total = 0;
        bool allInts = true;
        double floatTotal = 0;
        foreach (var item in values)
        {
            floatTotal += number(item);
            if (allInts && item is long n) total = addInt(total, n, line);
            else allInts = false;
        }
        return allInts ? (object)total : (object)floatTotal;
    }

    public static object b_char(object[] args, int line)
    {
        arity("char", args, 1, 1, line);
        long code = wholeArg("char", args[0], line);
        if (code <= 0 || code > 0x10FFFF || (code >= 0xD800 && code <= 0xDFFF))
        {
            throw raiseError("ValueError", code + " is not the code of a character", line);
        }
        return fromCode(code);
    }

    public static object b_allocate(object[] args, int line) => throw fatal("manual memory is only for a program compiled to machine code", line);

    public static object b_free(object[] args, int line) => b_allocate(args, line);

    static double oneNumber(string name, object[] args, int line)
    {
        arity(name, args, 1, 1, line);
        if (!isNumber(args[0])) throw wrongType(name, "a number", args[0], line);
        return number(args[0]);
    }

    static double angle(string name, object[] args, int line)
    {
        arity(name, args, 1, 1, line);
        if (args[0] is Quantity q)
        {
            var s = convert(q.unit, "rad");
            if (s == null) throw fatal("`" + name + "` needs a number or an angle, such as `30 deg`, not a quantity in " + q.unit, line);
            return apply(s, q.v);
        }
        if (!isNumber(args[0])) throw wrongType(name, "a number or an angle, such as `30 deg`", args[0], line);
        return number(args[0]);
    }

    static double ratio(string name, object[] args, int line)
    {
        double x = oneNumber(name, args, line);
        if (x < -1 || x > 1) throw raiseError("ValueError", "`" + name + "` needs a number from -1 to 1, and this is " + showFloat(x), line);
        return x;
    }

    static double aboveZero(string name, object[] args, int line)
    {
        double x = oneNumber(name, args, line);
        if (x <= 0) throw raiseError("ValueError", "cannot take the logarithm of " + showFloat(x), line);
        return x;
    }

    // A whole float as an int; infinity and NaN stay as they are.
    static object wholeOf(double x) => double.IsNaN(x) || double.IsInfinity(x) ? (object)x : (object)toInt(x);

    public static object b_math_sqrt(object[] args, int line)
    {
        double x = oneNumber("sqrt", args, line);
        if (x < 0) throw raiseError("ValueError", "cannot take the square root of " + showFloat(x), line);
        return System.Math.Sqrt(x);
    }

    public static object b_math_floor(object[] args, int line) => wholeOf(System.Math.Floor(oneNumber("floor", args, line)));

    public static object b_math_ceil(object[] args, int line) => wholeOf(System.Math.Ceiling(oneNumber("ceil", args, line)));

    // A half goes away from zero, as Rust's `round` does.
    public static object b_math_round(object[] args, int line) =>
        toInt(System.Math.Round(oneNumber("round", args, line), System.MidpointRounding.AwayFromZero));

    public static object b_math_sin(object[] args, int line) => System.Math.Sin(angle("sin", args, line));

    public static object b_math_cos(object[] args, int line) => System.Math.Cos(angle("cos", args, line));

    public static object b_math_tan(object[] args, int line) => System.Math.Tan(angle("tan", args, line));

    public static object b_math_asin(object[] args, int line) => new Quantity(System.Math.Asin(ratio("asin", args, line)), "rad");

    public static object b_math_acos(object[] args, int line) => new Quantity(System.Math.Acos(ratio("acos", args, line)), "rad");

    public static object b_math_atan(object[] args, int line) => new Quantity(System.Math.Atan(oneNumber("atan", args, line)), "rad");

    public static object b_math_atan2(object[] args, int line)
    {
        arity("atan2", args, 2, 2, line);
        if (!isNumber(args[0])) throw wrongType("atan2", "two numbers", args[0], line);
        if (!isNumber(args[1])) throw wrongType("atan2", "two numbers", args[1], line);
        return new Quantity(System.Math.Atan2(number(args[0]), number(args[1])), "rad");
    }

    public static object b_math_exp(object[] args, int line) => System.Math.Exp(oneNumber("exp", args, line));

    public static object b_math_log(object[] args, int line) => System.Math.Log(aboveZero("log", args, line));

    public static object b_math_log2(object[] args, int line) => System.Math.Log2(aboveZero("log2", args, line));

    public static object b_math_log10(object[] args, int line) => System.Math.Log10(aboveZero("log10", args, line));

    public static object b_time_sleep(object[] args, int line)
    {
        arity("sleep", args, 1, 1, line);
        if (!(args[0] is Quantity q)) throw wrongType("sleep", "a time with a unit, such as `500 ms`", args[0], line);
        var s = convert(q.unit, "s");
        if (s == null) throw fatal("`sleep` needs a time, such as `500 ms`, not a quantity in " + q.unit, line);
        double seconds = apply(s, q.v);
        if (seconds > 0)
        {
            flush();
            System.Threading.Thread.Sleep(System.TimeSpan.FromSeconds(System.Math.Min(seconds, 1e7)));
        }
        return null;
    }

    public static object b_time_now(object[] args, int line)
    {
        arity("now", args, 0, 0, line);
        return new Quantity(STARTED.Elapsed.TotalMilliseconds, "ms");
    }

    public static object b_files_read(object[] args, int line)
    {
        string path = paths("read", args, 1, line)[0];
        string where = at(path);
        if (where == "nothing") throw fileError("there is no file `" + path + "`", line);
        if (where == "other") throw fileError("cannot read `" + path + "`", line);
        byte[] data;
        try { data = System.IO.File.ReadAllBytes(path); }
        catch (System.Exception) { throw fileError("cannot read `" + path + "`", line); }
        string s;
        try { s = new System.Text.UTF8Encoding(false, true).GetString(data); }
        catch (System.ArgumentException) { s = null; }
        if (s == null || s.IndexOf('\0') >= 0) throw fileError("`" + path + "` does not hold text", line);
        return s;
    }

    static object put(string name, object[] args, bool append, int line)
    {
        string[] p = paths(name, args, 2, line);
        try
        {
            using var f = new System.IO.FileStream(p[0], append ? System.IO.FileMode.Append : System.IO.FileMode.Create);
            var bytes = new System.Text.UTF8Encoding(false).GetBytes(p[1]);
            f.Write(bytes, 0, bytes.Length);
        }
        catch (System.Exception) { throw fileError("cannot write `" + p[0] + "`", line); }
        return null;
    }

    public static object b_files_write(object[] args, int line) => put("write", args, false, line);

    public static object b_files_append(object[] args, int line) => put("append", args, true, line);

    public static object b_files_exists(object[] args, int line) => at(paths("exists", args, 1, line)[0]) == "file";

    public static object b_files_remove(object[] args, int line)
    {
        string path = paths("remove", args, 1, line)[0];
        string where = at(path);
        if (where == "nothing") throw fileError("there is no file `" + path + "`", line);
        if (where == "file")
        {
            try { System.IO.File.Delete(path); return null; }
            catch (System.Exception) { }
        }
        throw fileError("cannot remove `" + path + "`", line);
    }

    static string[] arguments = new string[0];

    public static object b_program_arguments(object[] args, int line)
    {
        arity("arguments", args, 0, 0, line);
        return new ZList(new System.Collections.Generic.List<object>(arguments));
    }

    public static object b_json_parse(object[] args, int line)
    {
        arity("parse", args, 1, 1, line);
        string s = text("parse", args[0], line);
        object v = jsonParse(s);
        if (v == NOT_JSON)
        {
            int a = 0;
            while (a < s.Length && char.IsWhiteSpace(s[a])) a++;
            string start = s.Substring(a);
            string shown = textLength(start) > 30 ? sub(start, 0, 30) + "..." : start;
            throw raiseError("ValueError", "this text is not JSON: `" + shown + "`", line);
        }
        return v;
    }

    public static object b_json_write(object[] args, int line)
    {
        arity("write", args, 1, 1, line);
        return jsonWrite(args[0], line);
    }

    static BuiltinBody sized(string name, bool wrap)
    {
        var w = WIDTHS[name];
        string article = w.signed ? "an" : "a";
        if (!wrap)
        {
            return (args, line) =>
            {
                arity(name, args, 1, 1, line);
                long n = args[0] is long x ? x : throw wrongType(name, "a whole number", args[0], line);
                if (n < w.lo || n > w.hi) throw raiseError("OverflowError", n + " does not fit in " + article + " " + name + " (" + w.lo + " to " + w.hi + ")", line);
                return n;
            };
        }
        return (args, line) =>
        {
            arity(name + ".wrap", args, 1, 1, line);
            long n = args[0] is long x ? x : throw wrongType(name + ".wrap", "a whole number", args[0], line);
            long low = n & ((1L << w.bits) - 1);
            return w.signed && low > w.hi ? low - (1L << w.bits) : low;
        };
    }

    static readonly string[] ERROR_KINDS = { "Error", "ValueError", "IndexError", "KeyError", "ZeroDivisionError", "OverflowError", "FileError" };

    public static object b_Error(object[] args, int line) => newError("Error", args, line);
    public static object b_ValueError(object[] args, int line) => newError("ValueError", args, line);
    public static object b_IndexError(object[] args, int line) => newError("IndexError", args, line);
    public static object b_KeyError(object[] args, int line) => newError("KeyError", args, line);
    public static object b_ZeroDivisionError(object[] args, int line) => newError("ZeroDivisionError", args, line);
    public static object b_OverflowError(object[] args, int line) => newError("OverflowError", args, line);
    public static object b_FileError(object[] args, int line) => newError("FileError", args, line);

    static readonly System.Collections.Generic.Dictionary<string, BuiltinBody> BUILTINS = MakeBuiltins();

    static System.Collections.Generic.Dictionary<string, BuiltinBody> MakeBuiltins()
    {
        var b = new System.Collections.Generic.Dictionary<string, BuiltinBody>
        {
            ["print"] = b_print, ["input"] = b_input, ["len"] = b_len, ["range"] = b_range, ["int"] = b_int,
            ["float"] = b_float, ["str"] = b_str, ["abs"] = b_abs, ["min"] = b_min, ["max"] = b_max, ["sum"] = b_sum,
            ["char"] = b_char, ["allocate"] = b_allocate, ["free"] = b_free,
            ["math.sqrt"] = b_math_sqrt, ["math.floor"] = b_math_floor, ["math.ceil"] = b_math_ceil,
            ["math.round"] = b_math_round, ["math.sin"] = b_math_sin, ["math.cos"] = b_math_cos, ["math.tan"] = b_math_tan,
            ["math.asin"] = b_math_asin, ["math.acos"] = b_math_acos, ["math.atan"] = b_math_atan, ["math.atan2"] = b_math_atan2,
            ["math.exp"] = b_math_exp, ["math.log"] = b_math_log, ["math.log2"] = b_math_log2, ["math.log10"] = b_math_log10,
            ["time.sleep"] = b_time_sleep, ["time.now"] = b_time_now,
            ["files.read"] = b_files_read, ["files.write"] = b_files_write, ["files.append"] = b_files_append,
            ["files.exists"] = b_files_exists, ["files.remove"] = b_files_remove,
            ["program.arguments"] = b_program_arguments, ["json.parse"] = b_json_parse, ["json.write"] = b_json_write,
        };
        foreach (var name in WIDTHS.Keys)
        {
            b[name] = sized(name, false);
            b[name + ".wrap"] = sized(name, true);
        }
        foreach (var kind in ERROR_KINDS) b[kind] = (args, line) => newError(kind, args, line);
        return b;
    }

    public static object call_builtin(string name, object[] args, int line)
    {
        if (!BUILTINS.TryGetValue(name, out var f)) throw fatal("there is no `" + name + "` in a program built for C#", line);
        return f(args, line);
    }

    static readonly System.Collections.Generic.Dictionary<string, object> BUILTIN_VALUES = new();

    // Something that comes with Zeno, as a value: `print`, `math.pi`.
    public static object builtin(string name, int line)
    {
        if (name == "math.pi") return System.Math.PI;
        if (name == "math.e") return System.Math.E;
        if (System.Array.IndexOf(ERROR_KINDS, name) >= 0) return new ErrorType(name);
        if (BUILTIN_VALUES.TryGetValue(name, out var known)) return known;
        int dot = name.IndexOf('.');
        if (!BUILTINS.TryGetValue(name, out var body))
        {
            if (dot >= 0) throw fatal("module `" + name.Substring(0, dot) + "` has no `" + name.Substring(dot + 1) + "`", line);
            throw fatal("`" + name + "` is not defined", line);
        }
        var v = new Func(dot >= 0 ? name.Substring(dot + 1) : name, new string[0], new bool[0], null, name, body);
        BUILTIN_VALUES[name] = v;
        return v;
    }

    public static object module(string name) => new Module(name);

    // ------------------------------------------------------- The program

    // Run the program, and say what stopped it if something did.
    public static void start(string[] files, string[] args, System.Func<object> main)
    {
        arguments = args;
        int status = 0;
        var thread = new System.Threading.Thread(() =>
        {
            try
            {
                main();
                flush();
            }
            catch (Raise e)
            {
                flush();
                System.Console.Error.Write("error: uncaught " + shownName(e.value.kind) + ": " + show(e.value) + "\n" + where(files, e.line) + "\n");
                status = 1;
            }
            catch (Fatal e)
            {
                flush();
                System.Console.Error.Write("error: " + e.text + "\n" + where(files, e.line) + "\n");
                status = 1;
            }
        }, 256 * 1024 * 1024);
        thread.Start();
        thread.Join();
        System.Console.Error.Flush();
        System.Environment.Exit(status);
    }

    static string where(string[] files, int line)
    {
        int index = line / LINES_A_FILE;
        string file = index < files.Length ? files[index] : files[0];
        return " --> " + file + ":" + (line % LINES_A_FILE);
    }
}
