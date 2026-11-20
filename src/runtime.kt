// The Zeno runtime for Kotlin.
//
// A program built with `zeno build --target kotlin` is one `.kt` file: the
// program, in `main`, and this runtime after it, in the object `Z`. It
// holds what every Zeno value is, and what each operator and built-in
// does with it: what ops.rs and builtins.rs do for `zeno run`, step for
// step and message for message, so that the program prints what
// `zeno run` prints. It is runtime.java in Kotlin, for the same machine.
//
// How a Zeno value is held (every one is an Any?):
//
//   int        Long, with every step checked for overflow
//   float      Double
//   str        String; an index counts characters, not UTF-16 units
//   bool       Boolean
//   none       null
//   list       ArrayList<Any?>
//   dict       LinkedHashMap<Any?, Any?>, which keeps the order the keys were put in
//   quantity   ZQuantity: a number and its unit
//   function   ZFunc
//   object     ZInstance of a ZClass
//   enum case  ZCase of a ZEnum
//   error      ZError
//
// Sine, logarithm and the rest are Java's Math, as in runtime.java.
//
// Every name here begins with `Z`, and no name of a Zeno program does.

class ZQuantity(val v: Double, val unit: String)

class ZFunc(
    val name: String,
    val params: Array<String>,
    val defaults: BooleanArray,
    val body: ((Array<Any?>) -> Any?)?,
    // A function that comes with Zeno: its full name, and its body.
    val builtin: String?,
    val bbody: ((Array<Any?>, Int) -> Any?)?,
)

class ZClass(val name: String, val fields: Array<String>, val defaults: Array<Any?>, val methods: HashMap<String, ZFunc>)

class ZInstance(val cls: ZClass, val fields: Array<Any?>)

class ZEnum(val name: String, val cases: Array<String>, val fields: Array<Array<String>>)

class ZCase(val def: ZEnum, val index: Int, val fields: Array<Any?>)

class ZConstructor(val def: ZEnum, val index: Int)

class ZErrorType(val kind: String)

class ZError(val kind: String, val message: String, val obj: Any?)

class ZModule(val name: String)

// A method of a str, a list or a dict, read but not yet called.
class ZMethod(val receiver: Any?, val name: String)

// A method of a class, read from an object.
class ZBound(val obj: Any?, val fn: ZFunc)

// Several values from one function, as `return a, b` gives them, until
// `let a, b = ...` takes them apart (decision 57).
class ZSeveral(val values: Array<Any?>)

// An error the program may catch, on its way to a `try`.
class ZRaise(val value: ZError, val line: Int) : RuntimeException(null, null, false, false)

// A mistake in the program, which nothing catches.
class ZFatal(val text: String, val line: Int) : RuntimeException(null, null, false, false)

class ZNotJson : RuntimeException(null, null, false, false)

class ZUnitInfo(val kind: String, val size: Double)

class ZWidth(val lo: Long, val hi: Long, val bits: Int, val signed: Boolean)

// Reads JSON, a character at a time.
class ZJson(s: String) {
    val cs: IntArray = s.codePoints().toArray()
    var at = 0

    fun spaces() {
        while (at < cs.size && (cs[at] == 0x20 || cs[at] == 0x09 || cs[at] == 0x0a || cs[at] == 0x0d)) at++
    }

    fun word(w: String): Boolean {
        if (at + w.length > cs.size) return false
        for (i in 0 until w.length) if (cs[at + i] != w[i].code) return false
        at += w.length
        return true
    }

    fun fourHex(): Int {
        if (at + 4 > cs.size) throw ZNotJson()
        var n = 0
        for (i in 0 until 4) {
            val d = Character.digit(cs[at + i], 16)
            if (d < 0 || cs[at + i] > 'f'.code) throw ZNotJson()
            n = n * 16 + d
        }
        at += 4
        return n
    }

    fun text(): String {
        if (at >= cs.size || cs[at] != '"'.code) throw ZNotJson()
        at++
        val b = StringBuilder()
        while (true) {
            if (at >= cs.size) throw ZNotJson()
            val c = cs[at++]
            if (c == '"'.code) return b.toString()
            if (c != '\\'.code) {
                b.appendCodePoint(c)
                continue
            }
            if (at >= cs.size) throw ZNotJson()
            val e = cs[at++]
            when (e) {
                'n'.code -> b.append('\n')
                't'.code -> b.append('\t')
                'r'.code -> b.append('\r')
                'b'.code -> b.append('\b')
                'f'.code -> b.append('\u000c')
                'u'.code -> {
                    var code = fourHex()
                    if (code >= 0xD800 && code < 0xDC00 && word("\\u")) {
                        val low = fourHex()
                        if (low < 0xDC00 || low > 0xDFFF) throw ZNotJson()
                        code = 0x10000 + ((code - 0xD800) shl 10) + (low - 0xDC00)
                    }
                    if ((code >= 0xD800 && code <= 0xDFFF) || code > 0x10FFFF) throw ZNotJson()
                    b.appendCodePoint(code)
                }
                else -> b.appendCodePoint(e)
            }
        }
    }

    fun value(): Any? {
        spaces()
        if (at >= cs.size) throw ZNotJson()
        val c = cs[at]
        if (c == 'n'.code) {
            if (word("null")) return null
            throw ZNotJson()
        }
        if (c == 't'.code) {
            if (word("true")) return true
            throw ZNotJson()
        }
        if (c == 'f'.code) {
            if (word("false")) return false
            throw ZNotJson()
        }
        if (c == '"'.code) return text()
        if (c == '['.code) {
            at++
            val out = ArrayList<Any?>()
            while (true) {
                spaces()
                if (word("]")) return out
                if (!out.isEmpty() && !word(",")) throw ZNotJson()
                out.add(value())
            }
        }
        if (c == '{'.code) {
            at++
            val d = LinkedHashMap<Any?, Any?>()
            var count = 0
            while (true) {
                spaces()
                if (word("}")) return d
                if (count > 0 && !word(",")) throw ZNotJson()
                spaces()
                val name = text()
                spaces()
                if (!word(":")) throw ZNotJson()
                val v = value()
                count++
                if (v != null) d[name] = v
            }
        }
        val start = at
        while (at < cs.size && "0123456789+-.eE".indexOf(cs[at].toChar()) >= 0 && cs[at] < 128) at++
        val x = Z.parseFloat(String(cs, start, at - start)) ?: throw ZNotJson()
        if (!x.isInfinite() && !x.isNaN() && x == Math.floor(x) && Math.abs(x) < 9.0e15) return x.toLong()
        return x
    }
}

object Z {
    const val LINES_A_FILE = 1000000
    const val MIN_INT = Long.MIN_VALUE
    const val NAN = Double.NaN
    const val INF = Double.POSITIVE_INFINITY

    // What a call gives a parameter it leaves out.
    val MISSING = Any()

    fun fatal(message: String, line: Int): Nothing = throw ZFatal(message, line)

    fun raiseError(kind: String, message: String, line: Int): Nothing = throw ZRaise(ZError(kind, message, null), line)

    fun overflow(line: Int): Nothing = raiseError("OverflowError", "the result is too big for an int", line)

    fun byZero(line: Int): Nothing = raiseError("ZeroDivisionError", "division by zero", line)

    fun isNumber(v: Any?): Boolean = v is Long || v is Double || v is Float || v is ULong

    fun number(v: Any?): Double = if (v is Long) v.toDouble() else if (v is Float) v.toDouble() else if (v is ULong) v.toDouble() else v as Double

    // ---- u64 (decision 42): a Kotlin `ULong`.
    val U64_TOO_BIG = "the result is too big for a u64"

    // A u64, or an int of 0 or more, as a ULong; null for anything else.
    fun unsigned(v: Any?): ULong? = if (v is ULong) v else if (v is Long && v >= 0) v.toULong() else null

    fun isU64(a: Any?, b: Any?): Boolean = a is ULong || b is ULong

    fun u64ShiftCount(n: Any?, line: Int): Int {
        if (n is ULong) {
            if (n >= 64uL) raiseError("ValueError", "cannot shift by " + n + ". A u64 has 64 bits, so a shift is 0 to 63", line)
            return n.toInt()
        }
        val c = n as Long
        if (c < 0 || c >= 64) raiseError("ValueError", "cannot shift by " + c + ". A u64 has 64 bits, so a shift is 0 to 63", line)
        return c.toInt()
    }

    // `a op b` where one side is a u64 (decision 42).
    fun u64Op(op: String, a: Any?, b: Any?, line: Int): Any? {
        if ((op == "<<" || op == ">>") && a is ULong && (b is Long || b is ULong)) {
            val n = u64ShiftCount(b, line)
            if (op == ">>") return a shr n
            val r = a shl n
            if ((r shr n) != a) raiseError("OverflowError", U64_TOO_BIG, line)
            return r
        }
        val x = unsigned(a)
        val y = unsigned(b)
        if (x == null || y == null) {
            fatal("`" + op + "` between " + typeName(a) + " and " + typeName(b) + " would have to choose between a u64 and something else. Write `u64( )` or `int( )` around one of them", line)
        }
        when (op) {
            "+" -> {
                val r = x + y
                if (r < x) raiseError("OverflowError", U64_TOO_BIG, line)
                return r
            }
            "-" -> {
                if (x < y) raiseError("OverflowError", "the result is below 0, and a u64 never is", line)
                return x - y
            }
            "*" -> {
                val r = x * y
                if (x != 0uL && r / x != y) raiseError("OverflowError", U64_TOO_BIG, line)
                return r
            }
            "/" -> {
                if (y == 0uL) byZero(line)
                return x.toDouble() / y.toDouble()
            }
            "//" -> {
                if (y == 0uL) byZero(line)
                return x / y
            }
            "%" -> {
                if (y == 0uL) byZero(line)
                return x % y
            }
            "**" -> {
                if (y == 0uL) return 1uL
                if (x < 2uL) return x
                var r: Any? = 1uL
                var i = 0uL
                while (i < y) {
                    r = u64Op("*", r, x, line)
                    i++
                }
                return r
            }
            "&" -> return x and y
            "|" -> return x or y
        }
        return x xor y
    }

    // `u64.wrap(a + b)`: the answer cut to 64 bits (42.6).
    fun u64Wrapping(op: String, args: Array<Any?>, line: Int): Any? {
        arity("u64.wrap", args, 2, 2, line)
        val x = unsigned(args[0])
        val y = unsigned(args[1])
        if (op == "<<" && x != null && (args[1] is Long || args[1] is ULong)) return x shl u64ShiftCount(args[1], line)
        if (x == null || y == null) wrongType("u64.wrap", "u64s", if (x == null) args[0] else args[1], line)
        return if (op == "+") x + y else if (op == "-") x - y else x * y
    }

    fun b_u64(args: Array<Any?>, line: Int): Any? {
        arity("u64", args, 1, 1, line)
        val v = args[0]
        if (v is ULong) return v
        if (v is Long) {
            if (v < 0) raiseError("OverflowError", v.toString() + " does not fit in a u64 (0 to 18446744073709551615)", line)
            return v.toULong()
        }
        wrongType("u64", "a whole number", v, line)
    }

    // `u64.wrap(n)`: the 64 bits of an int read with no sign (42.6).
    fun b_u64__of(args: Array<Any?>, line: Int): Any? {
        arity("u64.wrap", args, 1, 1, line)
        val v = args[0]
        if (v is ULong) return v
        if (v is Long) return v.toULong()
        wrongType("u64.wrap", "a whole number", v, line)
    }

    fun b_u64__add(args: Array<Any?>, line: Int): Any? = u64Wrapping("+", args, line)

    fun b_u64__sub(args: Array<Any?>, line: Int): Any? = u64Wrapping("-", args, line)

    fun b_u64__mul(args: Array<Any?>, line: Int): Any? = u64Wrapping("*", args, line)

    fun b_u64__shl(args: Array<Any?>, line: Int): Any? = u64Wrapping("<<", args, line)

    // An f32 is a `Float` (decision 41).
    fun fround(x: Double): Double = x.toFloat().toDouble()

    // An f32, or an int as a float and then the nearest f32 (41.2); null for anything else.
    fun single(v: Any?): Double? = if (v is Float) v.toDouble() else if (v is Long) fround(v.toDouble()) else null

    // The float with the fewest digits that reads back as this f32.
    fun f32Shown(f: Float): Double {
        val x = f.toDouble()
        if (x.isNaN() || x == 0.0 || x.isInfinite()) return x
        val exact = java.math.BigDecimal(x)
        for (digits in 1..9) {
            val d = exact.round(java.math.MathContext(digits, java.math.RoundingMode.HALF_EVEN)).toDouble()
            if (fround(d) == x) return d
        }
        return x
    }

    fun isF32(a: Any?, b: Any?): Boolean = a is Float || b is Float

    fun f32Op(op: String, a: Any?, b: Any?, line: Int): Any? {
        val x = single(a)
        val y = single(b)
        if (x == null || y == null) {
            if (a is Double || b is Double) {
                fatal("`" + op + "` between " + typeName(a) + " and " + typeName(b) + " would have to choose between 32 bits and 64. Write `f32( )` around the float to work in 32, or `float( )` around the f32 to work in 64", line)
            }
            mismatch(op, a, b, line)
        }
        val r: Double = when (op) {
            "+" -> x + y
            "-" -> x - y
            "*" -> x * y
            "/" -> div(x, y, line) as Double
            "//" -> floordiv(x, y, line) as Double
            "%" -> mod(x, y, line) as Double
            else -> pow(x, y, line) as Double
        }
        return r.toFloat()
    }

    fun typeName(v: Any?): String {
        if (v is Long) return "an int"
        if (v is Double) return "a float"
        if (v is Float) return "an f32"
        if (v is ULong) return "a u64"
        if (v is String) return "a str"
        if (v is Boolean) return "a bool"
        if (v == null) return "none"
        if (v is ZQuantity) return "a quantity in " + v.unit
        if (v is ArrayList<*>) return "a list"
        if (v is LinkedHashMap<*, *>) return "a dict"
        if (v is ZFunc || v is ZMethod || v is ZBound) return "a function"
        if (v is ZClass) return "a class"
        if (v is ZInstance) return "a " + v.cls.name
        if (v is ZEnum) return "an enum"
        if (v is ZConstructor) return "an enum case"
        if (v is ZCase) return "a " + v.def.name
        if (v is ZErrorType) return "an error type"
        if (v is ZError) return "a " + v.kind
        if (v is ZModule) return "a module"
        if (v is ZSeveral) return v.values.size.toString() + " values"
        return "a value"
    }

    // `Point`, not `shapes.Point`; `Stack`, not `Stack[int]`.
    fun shownName(full: String): String {
        var name = full
        val bracket = name.indexOf('[')
        if (bracket >= 0) name = name.substring(0, bracket)
        val dot = name.lastIndexOf('.')
        return if (dot < 0) name else name.substring(dot + 1)
    }

    @Suppress("UNCHECKED_CAST")
    fun asList(v: Any?): ArrayList<Any?> = v as ArrayList<Any?>

    @Suppress("UNCHECKED_CAST")
    fun asDict(v: Any?): LinkedHashMap<Any?, Any?> = v as LinkedHashMap<Any?, Any?>

    // ---------------------------------------------------------------- Text

    // A str's characters are code points; a String counts UTF-16 units.
    // Most text has no character outside the first 65536, and then the
    // two agree; the last str asked about is remembered.
    var lastText: String? = null
    var lastPlain = true

    fun plain(s: String): Boolean {
        if (s === lastText) return lastPlain
        var p = true
        for (i in 0 until s.length) {
            if (Character.isSurrogate(s[i])) {
                p = false
                break
            }
        }
        lastText = s
        lastPlain = p
        return p
    }

    fun textLength(s: String): Int = if (plain(s)) s.length else s.codePointCount(0, s.length)

    fun part(s: String, from: Int, to: Int): String {
        if (plain(s)) return s.substring(from, to)
        val a = s.offsetByCodePoints(0, from)
        val b = s.offsetByCodePoints(a, to - from)
        return s.substring(a, b)
    }

    // ------------------------------------------------------------- Showing

    // A float as Rust's `{:?}` shows it: the shortest digits that read
    // back as the same number, `.0` on a whole one, and an exponent for one
    // below 1e-4 or from 1e16 up.
    fun showFloat(x: Double): String {
        if (x.isNaN()) return "NaN"
        if (x == Double.POSITIVE_INFINITY) return "inf"
        if (x == Double.NEGATIVE_INFINITY) return "-inf"
        if (x == 0.0) return if (1 / x < 0) "-0.0" else "0.0"
        // Java writes two digits at least, so the smallest floats there are
        // come out as 4.9E-324 where one digit, 5e-324, names the float.
        if (Math.abs(x) < 1e-320) {
            val one = java.math.BigDecimal(Math.abs(x)).round(java.math.MathContext(1, java.math.RoundingMode.HALF_EVEN))
            if (one.toDouble() == Math.abs(x)) return (if (x < 0) "-" else "") + one.unscaledValue() + "e" + (-one.scale())
        }
        var s = java.lang.Double.toString(x)
        var sign = ""
        if (s[0] == '-') {
            sign = "-"
            s = s.substring(1)
        }
        val e = s.indexOf('E')
        val mantissa = if (e < 0) s else s.substring(0, e)
        var exp = if (e < 0) 0 else Integer.parseInt(s.substring(e + 1))
        val point = mantissa.indexOf('.')
        val whole = mantissa.substring(0, point)
        val fraction = mantissa.substring(point + 1)
        var digits = whole + fraction
        exp += whole.length - 1
        var lead = 0
        while (lead < digits.length - 1 && digits[lead] == '0') {
            lead++
            exp--
        }
        digits = digits.substring(lead)
        var end = digits.length
        while (end > 1 && digits[end - 1] == '0') end--
        digits = digits.substring(0, end)
        if (exp < -4 || exp >= 16) {
            return sign + digits[0] + (if (digits.length > 1) "." + digits.substring(1) else "") + "e" + exp
        }
        if (exp < 0) return sign + "0." + "0".repeat(-exp - 1) + digits
        if (digits.length <= exp + 1) return sign + digits + "0".repeat(exp + 1 - digits.length) + ".0"
        return sign + digits.substring(0, exp + 1) + "." + digits.substring(exp + 1)
    }

    // A float with a fixed number of decimals, as Rust's `{:.2}` writes it:
    // from its exact value, a half rounding to the even digit.
    fun fixed(x: Double, places: Int): String {
        if (x.isNaN()) return "NaN"
        if (x.isInfinite()) return if (x > 0) "inf" else "-inf"
        var s = java.math.BigDecimal(x).setScale(places, java.math.RoundingMode.HALF_EVEN).toPlainString()
        if ((x < 0 || (x == 0.0 && 1 / x < 0)) && !s.startsWith("-")) s = "-" + s
        return s
    }

    // Each character of a str, as its number.
    inline fun eachCode(s: String, step: (Int) -> Unit) {
        var i = 0
        while (i < s.length) {
            val c = s.codePointAt(i)
            step(c)
            i += Character.charCount(c)
        }
    }

    // How Rust's `{:?}` shows a str: in quotes, with what cannot be seen escaped.
    fun quoted(s: String): String {
        val b = StringBuilder("\"")
        eachCode(s) { c ->
            if (c == '"'.code) b.append("\\\"")
            else if (c == '\\'.code) b.append("\\\\")
            else if (c == '\n'.code) b.append("\\n")
            else if (c == '\r'.code) b.append("\\r")
            else if (c == '\t'.code) b.append("\\t")
            else if (c == 0) b.append("\\0")
            else if (c < 0x20 || (c >= 0x7f && c < 0xa0)) b.append("\\u{").append(Integer.toHexString(c)).append("}")
            else b.appendCodePoint(c)
        }
        return b.append('"').toString()
    }

    // How `print`, `str()` and f-strings show a value.
    fun show(v: Any?): String {
        if (v is String) return v
        if (v is Long) return v.toString()
        if (v is Double) return showFloat(v)
        if (v is Float) return showFloat(f32Shown(v))
        if (v is ULong) return v.toString()
        if (v is Boolean) return if (v) "true" else "false"
        if (v == null) return "none"
        if (v is ArrayList<*>) {
            val b = StringBuilder("[")
            for (i in 0 until v.size) {
                if (i > 0) b.append(", ")
                b.append(repr(v[i]))
            }
            return b.append("]").toString()
        }
        if (v is LinkedHashMap<*, *>) {
            if (v.isEmpty()) return "[:]"
            val b = StringBuilder("[")
            var first = true
            for (entry in v.entries) {
                if (!first) b.append(", ")
                first = false
                b.append(repr(entry.key)).append(": ").append(repr(entry.value))
            }
            return b.append("]").toString()
        }
        if (v is ZQuantity) {
            if (!v.v.isNaN() && !v.v.isInfinite() && v.v == Math.floor(v.v) && Math.abs(v.v) < 1e15) {
                return v.v.toLong().toString() + " " + v.unit
            }
            return showFloat(v.v) + " " + v.unit
        }
        if (v is ZInstance) {
            val b = StringBuilder(shownName(v.cls.name)).append("(")
            for (i in 0 until v.cls.fields.size) {
                if (i > 0) b.append(", ")
                b.append(v.cls.fields[i]).append("=").append(repr(v.fields[i]))
            }
            return b.append(")").toString()
        }
        if (v is ZCase) {
            val name = shownName(v.def.cases[v.index])
            if (v.fields.isEmpty()) return name
            val b = StringBuilder(name).append("(")
            for (i in 0 until v.fields.size) {
                if (i > 0) b.append(", ")
                b.append(repr(v.fields[i]))
            }
            return b.append(")").toString()
        }
        if (v is ZError) return v.message
        if (v is ZMethod) return "<function " + v.name + ">"
        if (v is ZBound) return "<function " + v.fn.name + ">"
        if (v is ZClass) return "<class " + v.name + ">"
        if (v is ZEnum) return "<enum " + v.name + ">"
        if (v is ZConstructor) return "<case " + v.def.cases[v.index] + ">"
        if (v is ZErrorType) return "<error " + v.kind + ">"
        if (v is ZModule) return "<module " + v.name + ">"
        if (v is ZFunc) return if (v.name.isEmpty()) "<function>" else "<function " + v.name + ">"
        if (v is ZSeveral) return "<" + v.values.size + " values: " + v.values.joinToString(", ") { show(it) } + ">"
        return "?"
    }

    fun repr(v: Any?): String = if (v is String) quoted(v) else show(v)

    // --------------------------------------------------------------- Units

    val UNITS = HashMap<String, ZUnitInfo>()

    fun units(kind: String, vararg pairs: Any) {
        var i = 0
        while (i < pairs.size) {
            UNITS[pairs[i] as String] = ZUnitInfo(kind, (pairs[i + 1] as Number).toDouble())
            i += 2
        }
    }

    init {
        units("time", "ns", 1, "us", 1e3, "ms", 1e6, "s", 1e9, "min", 60e9, "h", 3600e9)
        units("frequency", "Hz", 1, "kHz", 1e3, "MHz", 1e6)
        units("voltage", "uV", 1, "mV", 1e3, "V", 1e6, "kV", 1e9)
        units("current", "uA", 1, "mA", 1e3, "A", 1e6)
        units("resistance", "ohm", 1, "kohm", 1e3, "Mohm", 1e6)
        units("power", "mW", 1, "W", 1e3, "kW", 1e6)
        units("energy", "J", 1, "Wh", 3600, "kWh", 3600e3)
        units("temperature in Celsius", "degC", 1)
        units("temperature in kelvin", "K", 1)
        units("length", "mm", 1, "cm", 10, "m", 1e3, "km", 1e6)
        units("mass", "mg", 1, "g", 1e3, "kg", 1e6)
        units("angle in degrees", "deg", 1)
        units("angle in radians", "rad", 1)
        units("rotation", "rpm", 1)
    }

    // How to turn a number in one unit into another: {times, over}, or null.
    fun convert(from: String, to: String): DoubleArray? {
        if (from == "deg" && to == "rad") return doubleArrayOf(Math.PI / 180, 1.0)
        if (from == "rad" && to == "deg") return doubleArrayOf(180 / Math.PI, 1.0)
        val f = UNITS[from]
        val t = UNITS[to]
        if (f == null || t == null || f.kind != t.kind) return null
        return if (f.size >= t.size) doubleArrayOf(f.size / t.size, 1.0) else doubleArrayOf(1.0, t.size / f.size)
    }

    fun apply(scale: DoubleArray, x: Double): Double = x * scale[0] / scale[1]

    // The unit two quantities meet in, the finer one, or null.
    fun common(a: String, b: String): String? {
        val ua = UNITS[a]
        val ub = UNITS[b]
        if (ua == null || ub == null || ua.kind != ub.kind) return null
        return if (ua.size <= ub.size) a else b
    }

    // A quantity's number, in the unit `u`, which its own unit can be turned into.
    fun inUnit(q: ZQuantity, u: String): Double = apply(convert(q.unit, u)!!, q.v)

    // What `*` and `/` make of two quantities (decision 43): the operator,
    // what each side measures ("" for a plain number), and what the answer
    // measures ("" for a plain number). The answer is in the unit of its
    // kind with no prefix.
    val MADE = mapOf(
        "* voltage current" to "power", "* current voltage" to "power",
        "* current resistance" to "voltage", "* resistance current" to "voltage",
        "* power time" to "energy", "* time power" to "energy",
        "* time frequency" to "", "* frequency time" to "",
        "/ power voltage" to "current", "/ power current" to "voltage",
        "/ voltage current" to "resistance", "/ voltage resistance" to "current",
        "/ energy time" to "power", "/ energy power" to "time",
        "/  time" to "frequency", "/  frequency" to "time",
    )

    val PLAIN = mapOf("time" to "s", "frequency" to "Hz", "voltage" to "V", "current" to "A", "resistance" to "ohm", "power" to "W", "energy" to "J")

    // `a op b` for two quantities, or a number over a quantity.
    fun made(op: String, a: Any?, b: ZQuantity, line: Int): Any? {
        val left = if (a is ZQuantity) UNITS[a.unit]!!.kind else ""
        val right = UNITS[b.unit]!!.kind
        val kind = MADE[op + " " + left + " " + right]
        if (kind == null || (left.isEmpty() && !isNumber(a))) mismatch(op, a, b, line)
        val x = if (a is ZQuantity) inUnit(a, PLAIN[left]!!) else number(a)
        if (op == "/" && b.v == 0.0) byZero(line)
        val y = inUnit(b, PLAIN[right]!!)
        val r = if (op == "*") x * y else x / y
        if (kind.isEmpty()) return r
        return ZQuantity(r, PLAIN[kind]!!)
    }

    fun unit(v: Any?, name: String, line: Int): Any? {
        if (!UNITS.containsKey(name)) {
            fatal("`" + name + "` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC", line)
        }
        if (!isNumber(v)) fatal("a unit can only follow a number", line)
        return ZQuantity(number(v), name)
    }

    // ------------------------------------------------------------ Equality

    fun equal(a: Any?, b: Any?): Boolean {
        if (a is Long && b is Long) return a == b
        if (isU64(a, b)) {
            val x = unsigned(a)
            return x != null && x == unsigned(b)
        }
        if (isF32(a, b)) {
            val x = single(a)
            val y = single(b)
            if (x != null && y != null) return x == y
            return isNumber(a) && isNumber(b) && number(a) == number(b)
        }
        if (isNumber(a) && isNumber(b)) return number(a) == number(b)
        if (a is String && b is String) return a == b
        if (a is Boolean && b is Boolean) return a == b
        if (a == null || b == null) return a === b
        if (a is ZQuantity && b is ZQuantity) {
            val u = common(a.unit, b.unit)
            return u != null && inUnit(a, u) == inUnit(b, u)
        }
        if (a is ArrayList<*> && b is ArrayList<*>) {
            if (a.size != b.size) return false
            for (i in 0 until a.size) if (!equal(a[i], b[i])) return false
            return true
        }
        if (a is LinkedHashMap<*, *> && b is LinkedHashMap<*, *>) {
            if (a.size != b.size) return false
            for (entry in a.entries) {
                if (!b.containsKey(entry.key) || !equal(entry.value, b[entry.key])) return false
            }
            return true
        }
        if (a is ZInstance && b is ZInstance) {
            if (a.cls !== b.cls) return false
            for (i in 0 until a.fields.size) if (!equal(a.fields[i], b.fields[i])) return false
            return true
        }
        if (a is ZCase && b is ZCase) {
            if (a.def !== b.def || a.index != b.index || a.fields.size != b.fields.size) return false
            for (i in 0 until a.fields.size) if (!equal(a.fields[i], b.fields[i])) return false
            return true
        }
        if (a is ZError && b is ZError) return a.kind == b.kind && a.message == b.message
        if (a is ZConstructor && b is ZConstructor) return a.def === b.def && a.index == b.index
        return a === b
    }

    fun eq(a: Any?, b: Any?): Boolean = equal(a, b)

    fun ne(a: Any?, b: Any?): Boolean = !equal(a, b)

    // ----------------------------------------------------------- Operators

    fun mismatch(op: String, a: Any?, b: Any?, line: Int): Nothing =
        fatal("`" + op + "` does not work between " + typeName(a) + " and " + typeName(b), line)

    fun addInt(a: Long, b: Long, line: Int): Long {
        val r = a + b
        if (((a xor r) and (b xor r)) < 0) overflow(line)
        return r
    }

    fun mulInt(a: Long, b: Long, line: Int): Long {
        val hi = Math.multiplyHigh(a, b)
        val lo = a * b
        if ((hi != 0L || lo < 0) && (hi != -1L || lo >= 0)) overflow(line)
        return lo
    }

    fun add(a: Any?, b: Any?, line: Int): Any? {
        if (isU64(a, b)) return u64Op("+", a, b, line)
        if (isF32(a, b)) return f32Op("+", a, b, line)
        if (a is Long && b is Long) return addInt(a, b, line)
        if (a is String && b is String) return a + b
        if (a is ArrayList<*> && b is ArrayList<*>) {
            val items = ArrayList<Any?>(a.size + b.size)
            items.addAll(a)
            items.addAll(b)
            return items
        }
        if (a is ZQuantity && b is ZQuantity) {
            val u = common(a.unit, b.unit) ?: mismatch("+", a, b, line)
            return ZQuantity(inUnit(a, u) + inUnit(b, u), u)
        }
        if (!isNumber(a) || !isNumber(b)) mismatch("+", a, b, line)
        return number(a) + number(b)
    }

    fun sub(a: Any?, b: Any?, line: Int): Any? {
        if (isU64(a, b)) return u64Op("-", a, b, line)
        if (isF32(a, b)) return f32Op("-", a, b, line)
        if (a is Long && b is Long) {
            val r = a - b
            if (((a xor b) and (a xor r)) < 0) overflow(line)
            return r
        }
        if (a is ZQuantity && b is ZQuantity) {
            val u = common(a.unit, b.unit) ?: mismatch("-", a, b, line)
            return ZQuantity(inUnit(a, u) - inUnit(b, u), u)
        }
        if (!isNumber(a) || !isNumber(b)) mismatch("-", a, b, line)
        return number(a) - number(b)
    }

    fun mul(a: Any?, b: Any?, line: Int): Any? {
        if (isU64(a, b)) return u64Op("*", a, b, line)
        if (isF32(a, b)) return f32Op("*", a, b, line)
        if (a is ArrayList<*> && b is Long) {
            if (b < 0L) raiseError("ValueError", "`*` repeats a list 0 or more times, and this is " + b, line)
            val items = ArrayList<Any?>()
            for (t in 0 until b) items.addAll(a)
            return items
        }
        if (a is Long && b is Long) return mulInt(a, b, line)
        if (a is ZQuantity && b is ZQuantity) return made("*", a, b, line)
        if (a is ZQuantity || b is ZQuantity) {
            val q = (if (a is ZQuantity) a else b) as ZQuantity
            val other = if (a is ZQuantity) b else a
            if (!isNumber(other)) mismatch("*", a, b, line)
            return ZQuantity(q.v * number(other), q.unit)
        }
        if (!isNumber(a) || !isNumber(b)) mismatch("*", a, b, line)
        return number(a) * number(b)
    }

    fun div(a: Any?, b: Any?, line: Int): Any? {
        if (isU64(a, b)) return u64Op("/", a, b, line)
        if (isF32(a, b)) return f32Op("/", a, b, line)
        if (a is ZQuantity) {
            if (b is ZQuantity && b.v == 0.0) byZero(line)
            if (b is Double && b == 0.0) byZero(line)
            if (b is Long && b == 0L) byZero(line)
            if (b is ZQuantity) {
                val u = common(a.unit, b.unit) ?: return made("/", a, b, line)
                return inUnit(a, u) / inUnit(b, u)
            }
            if (!isNumber(b)) mismatch("/", a, b, line)
            return ZQuantity(a.v / number(b), a.unit)
        }
        if (b is ZQuantity) return made("/", a, b, line)
        if (!isNumber(a) || !isNumber(b)) mismatch("/", a, b, line)
        if (number(b) == 0.0) byZero(line)
        return number(a) / number(b)
    }

    fun floordiv(a: Any?, b: Any?, line: Int): Any? {
        if (isU64(a, b)) return u64Op("//", a, b, line)
        if (isF32(a, b)) return f32Op("//", a, b, line)
        if (a is Long && b is Long) {
            if (b == 0L) byZero(line)
            if (a == Long.MIN_VALUE && b == -1L) overflow(line)
            return Math.floorDiv(a, b)
        }
        if (!isNumber(a) || !isNumber(b)) mismatch("//", a, b, line)
        if (number(b) == 0.0) byZero(line)
        return Math.floor(number(a) / number(b))
    }

    fun mod(a: Any?, b: Any?, line: Int): Any? {
        if (isU64(a, b)) return u64Op("%", a, b, line)
        if (isF32(a, b)) return f32Op("%", a, b, line)
        if (a is Long && b is Long) {
            if (b == 0L) byZero(line)
            return Math.floorMod(a, b)
        }
        if (!isNumber(a) || !isNumber(b)) mismatch("%", a, b, line)
        val x = number(a)
        val y = number(b)
        if (y == 0.0) byZero(line)
        val q = x / y
        if (q.isNaN() || q.isInfinite()) return x - y * q
        return x - y * Math.floor(q)
    }

    fun pow(a: Any?, b: Any?, line: Int): Any? {
        if (isU64(a, b)) return u64Op("**", a, b, line)
        if (isF32(a, b)) return f32Op("**", a, b, line)
        if (a is Long && b is Long) {
            if (b < 0) raiseError("ValueError", "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`", line)
            if (b > 4294967295L) overflow(line)
            if (a == 0L || a == 1L) return if (b == 0L) 1L else a
            if (a == -1L) return if (b % 2 == 0L) 1L else -1L
            if (b >= 64) overflow(line)
            var r = 1L
            var i = 0L
            while (i < b) {
                r = mulInt(r, a, line)
                i++
            }
            return r
        }
        if (!isNumber(a) || !isNumber(b)) mismatch("**", a, b, line)
        val x = number(a)
        val y = number(b)
        if (x == 1.0 || y == 0.0) return 1.0
        return Math.pow(x, y)
    }

    fun bitsMismatch(op: String, a: Any?, b: Any?, line: Int): Nothing {
        var hint = ""
        if (a is Boolean && b is Boolean) {
            if (op == "&") hint = ". For two bools Zeno spells this `and`"
            if (op == "|") hint = ". For two bools Zeno spells this `or`"
        }
        fatal("`" + op + "` works on the bits of whole numbers, and these are " + typeName(a) + " and " + typeName(b) + hint, line)
    }

    fun whole(op: String, v: Any?, a: Any?, b: Any?, line: Int): Long {
        if (v !is Long) bitsMismatch(op, a, b, line)
        return v
    }

    fun bitand(a: Any?, b: Any?, line: Int): Any? = if (isU64(a, b)) u64Op("&", a, b, line) else whole("&", a, a, b, line) and whole("&", b, a, b, line)

    fun bitor(a: Any?, b: Any?, line: Int): Any? = if (isU64(a, b)) u64Op("|", a, b, line) else whole("|", a, a, b, line) or whole("|", b, a, b, line)

    fun bitxor(a: Any?, b: Any?, line: Int): Any? = if (isU64(a, b)) u64Op("^", a, b, line) else whole("^", a, a, b, line) xor whole("^", b, a, b, line)

    fun shift(n: Long, line: Int) {
        if (n < 0 || n >= 64) raiseError("ValueError", "cannot shift by " + n + ". An int has 64 bits, so a shift is 0 to 63", line)
    }

    fun shl(a: Any?, b: Any?, line: Int): Any? {
        if (isU64(a, b)) return u64Op("<<", a, b, line)
        val x = whole("<<", a, a, b, line)
        val n = whole("<<", b, a, b, line)
        shift(n, line)
        val s = x shl n.toInt()
        if ((s shr n.toInt()) != x) overflow(line)
        return s
    }

    fun shr(a: Any?, b: Any?, line: Int): Any? {
        if (isU64(a, b)) return u64Op(">>", a, b, line)
        val x = whole(">>", a, a, b, line)
        val n = whole(">>", b, a, b, line)
        shift(n, line)
        return x shr n.toInt()
    }

    fun neg(v: Any?, line: Int): Any? {
        if (v is Long) {
            if (v == Long.MIN_VALUE) overflow(line)
            return -v
        }
        if (v is Double) return -v
        if (v is Float) return -v
        if (v is ULong) fatal("`-` does not work on a u64, which is never below 0. Make it an int first with `int( )`", line)
        if (v is ZQuantity) return ZQuantity(-v.v, v.unit)
        fatal("`-` does not work on " + typeName(v), line)
    }

    fun bitnot(v: Any?, line: Int): Any? {
        if (v is ULong) return v.inv()
        if (v !is Long) fatal("`~` works on the bits of a whole number, and this is " + typeName(v), line)
        return v.inv()
    }

    // -1, 0 or 1, or 2 when the two cannot be ordered (a NaN).
    fun order(op: String, a: Any?, b: Any?, line: Int): Int {
        if (isU64(a, b)) {
            val x = unsigned(a)
            val y = unsigned(b)
            if (x == null || y == null) u64Op(op, a, b, line)
            return if (x!! < y!!) -1 else if (x > y) 1 else 0
        }
        if (isF32(a, b)) {
            val x = single(a)
            val y = single(b)
            if (x == null || y == null) f32Op(op, a, b, line)
            return if (x!! < y!!) -1 else if (x > y) 1 else if (x == y) 0 else 2
        }
        if (a is Long && b is Long) return if (a < b) -1 else if (a > b) 1 else 0
        if (a is String && b is String) {
            // By code points, as Rust compares, not by UTF-16 units.
            var i = 0
            var j = 0
            while (i < a.length && j < b.length) {
                val c = a.codePointAt(i)
                val d = b.codePointAt(j)
                if (c != d) return if (c < d) -1 else 1
                i += Character.charCount(c)
                j += Character.charCount(d)
            }
            return if (i < a.length) 1 else if (j < b.length) -1 else 0
        }
        val x: Double
        val y: Double
        if (a is ZQuantity && b is ZQuantity) {
            val u = common(a.unit, b.unit) ?: mismatch(op, a, b, line)
            x = inUnit(a, u)
            y = inUnit(b, u)
        } else {
            if (!isNumber(a) || !isNumber(b)) mismatch(op, a, b, line)
            x = number(a)
            y = number(b)
        }
        if (x < y) return -1
        if (x > y) return 1
        if (x == y) return 0
        return 2
    }

    fun lt(a: Any?, b: Any?, line: Int): Boolean = order("<", a, b, line) == -1

    fun le(a: Any?, b: Any?, line: Int): Boolean {
        val o = order("<=", a, b, line)
        return o == -1 || o == 0
    }

    fun gt(a: Any?, b: Any?, line: Int): Boolean = order(">", a, b, line) == 1

    fun ge(a: Any?, b: Any?, line: Int): Boolean {
        val o = order(">=", a, b, line)
        return o == 1 || o == 0
    }

    fun contains(op: String, item: Any?, container: Any?, line: Int): Boolean {
        if (container is ArrayList<*>) {
            for (other in container) if (equal(item, other)) return true
            return false
        }
        if (container is LinkedHashMap<*, *>) {
            if (item !is Long && item !is String) mismatch(op, item, container, line)
            return container.containsKey(item)
        }
        if (container is String && item is String) return container.contains(item)
        mismatch(op, item, container, line)
    }

    fun is_in(a: Any?, b: Any?, line: Int): Boolean = contains("in", a, b, line)

    fun not_in(a: Any?, b: Any?, line: Int): Boolean = !contains("not in", a, b, line)

    fun truth(v: Any?, line: Int): Boolean {
        if (v is Boolean) return v
        fatal("a condition must be true or false, but this is " + typeName(v), line)
    }

    // ------------------------------------------------------ Lists and text

    fun key(v: Any?, line: Int): Any? {
        if (v is Long || v is String) return v
        fatal("a key of a dict is a str or an int, but this is " + typeName(v), line)
    }

    fun position(index: Any?, length: Int, what: String, line: Int): Int {
        if (index !is Long) fatal("an index must be an int, but this is " + typeName(index), line)
        if (index < 0 || index >= length) {
            raiseError("IndexError", "index " + index + " is out of range for a " + what + " of length " + length, line)
        }
        return index.toInt()
    }

    fun index(obj: Any?, i: Any?, line: Int): Any? {
        if (obj is ArrayList<*>) return obj[position(i, obj.size, "list", line)]
        if (obj is String) {
            val at = position(i, textLength(obj), "str", line)
            return part(obj, at, at + 1)
        }
        if (obj is LinkedHashMap<*, *>) {
            val k = key(i, line)
            val v = obj[k]
            if (v == null && !obj.containsKey(k)) raiseError("KeyError", "this dict has no key " + repr(i), line)
            return v
        }
        fatal("cannot take an item from " + typeName(obj), line)
    }

    fun set_index(obj: Any?, i: Any?, value: Any?, line: Int) {
        if (obj is ArrayList<*>) {
            asList(obj)[position(i, obj.size, "list", line)] = value
            return
        }
        if (obj is LinkedHashMap<*, *>) {
            asDict(obj)[key(i, line)] = value
            return
        }
        fatal("cannot set an item of " + typeName(obj), line)
    }

    fun chars(s: String): ArrayList<Any?> {
        val out = ArrayList<Any?>(s.length)
        eachCode(s) { c -> out.add(String(Character.toChars(c))) }
        return out
    }

    fun items(v: Any?, line: Int): ArrayList<Any?> {
        if (v is ArrayList<*>) return ArrayList<Any?>(v)
        if (v is LinkedHashMap<*, *>) return ArrayList<Any?>(v.keys)
        if (v is String) return chars(v)
        fatal("cannot loop over " + typeName(v), line)
    }

    fun pairs(v: Any?, line: Int): ArrayList<Array<Any?>> {
        val out = ArrayList<Array<Any?>>()
        if (v is LinkedHashMap<*, *>) {
            for (entry in v.entries) out.add(arrayOf<Any?>(entry.key, entry.value))
            return out
        }
        val all = items(v, line)
        for (i in 0 until all.size) out.add(arrayOf<Any?>(i.toLong(), all[i]))
        return out
    }

    fun list(values: Array<Any?>): Any? {
        val out = ArrayList<Any?>(values.size)
        for (value in values) out.add(value)
        return out
    }

    fun dict(flat: Array<Any?>, line: Int): Any? {
        val d = LinkedHashMap<Any?, Any?>()
        var i = 0
        while (i < flat.size) {
            d[key(flat[i], line)] = flat[i + 1]
            i += 2
        }
        return d
    }

    fun push(list: Any?, value: Any?): Any? {
        asList(list).add(value)
        return null
    }

    fun concat(pieces: Array<Any?>): Any? {
        val b = StringBuilder()
        for (p in pieces) b.append(p as String)
        return b.toString()
    }

    // ------------------------------------------------- Objects and fields

    fun fn(name: String, params: Array<String>, defaults: BooleanArray, body: (Array<Any?>) -> Any?): ZFunc =
        ZFunc(name, params, defaults, body, null, null)

    fun methods(vararg pairs: Any?): HashMap<String, ZFunc> {
        val out = HashMap<String, ZFunc>()
        var i = 0
        while (i < pairs.size) {
            out[pairs[i] as String] = pairs[i + 1] as ZFunc
            i += 2
        }
        return out
    }

    fun cls(name: String, fields: Array<String>, defaults: Array<Any?>, methods: HashMap<String, ZFunc>): Any? =
        ZClass(name, fields, defaults, methods)

    fun enumOf(name: String, cases: Array<String>, fields: Array<Array<String>>): Any? = ZEnum(name, cases, fields)

    // A case with no values is a value; one with values needs calling.
    fun case_of(def: Any?, i: Int): Any? {
        val e = def as ZEnum
        return if (e.fields[i].isEmpty()) ZCase(e, i, NO_VALUES) else ZConstructor(e, i)
    }

    fun is_case(value: Any?, ctor: Any?, count: Int, line: Int): Boolean {
        if (ctor !is ZConstructor) fatal("this is not an enum case that carries values", line)
        if (value !is ZCase || value.def !== ctor.def || value.index != ctor.index) return false
        if (value.fields.size != count) {
            fatal("`" + ctor.def.cases[ctor.index] + "` carries " + value.fields.size + " values, but this pattern has " + count, line)
        }
        return true
    }

    fun case_field(value: Any?, i: Int): Any? = (value as ZCase).fields[i]

    fun several(values: Array<Any?>): Any? = ZSeveral(values)

    // What place `at` of the `count` on the left of `=` is given.
    fun received(value: Any?, count: Int, at: Int, line: Int): Any? {
        if (value !is ZSeveral) fatal("there are " + count + " places on the left of `=`, and the right gives one value, " + typeName(value), line)
        if (value.values.size != count) {
            fatal("this gives " + value.values.size + " values, and there are " + count + " places on the left of `=`", line)
        }
        return value.values[at]
    }

    fun field(obj: Any?, name: String, line: Int): Any? {
        if (obj is ZInstance) {
            val at = obj.cls.fields.indexOf(name)
            if (at >= 0) return obj.fields[at]
            val m = obj.cls.methods[name]
            if (m != null) return ZBound(obj, m)
            fatal("`" + obj.cls.name + "` has no field or method `" + name + "`", line)
        }
        if (obj is ZCase) {
            val at = obj.def.fields[obj.index].indexOf(name)
            if (at >= 0) return obj.fields[at]
            fatal("`" + obj.def.cases[obj.index] + "` has no field `" + name + "`", line)
        }
        if (obj is ZEnum) {
            val at = obj.cases.indexOf(name)
            if (at >= 0) return case_of(obj, at)
            fatal("enum `" + obj.name + "` has no case `" + name + "`", line)
        }
        if (obj is ZModule) return builtin(obj.name + "." + name, line)
        if (obj is ZQuantity && UNITS.containsKey(name)) {
            val s = convert(obj.unit, name)
                ?: fatal("a quantity in " + obj.unit + " cannot be turned into " + name + ". They measure different things", line)
            return ZQuantity(apply(s, obj.v), name)
        }
        if (obj is ZError && name == "message") return obj.message
        if (obj is ZError && name == "kind") return obj.kind
        if (obj is ZFunc && obj.builtin != null && WIDTHS.containsKey(obj.builtin) && name == "wrap") {
            return builtin(obj.builtin + ".wrap", line)
        }
        if (hasMethod(obj, name)) return ZMethod(obj, name)
        fatal(typeName(obj) + " has no field or method `" + name + "`", line)
    }

    fun set_field(obj: Any?, name: String, value: Any?, line: Int) {
        if (obj is ZInstance) {
            val at = obj.cls.fields.indexOf(name)
            if (at < 0) fatal("`" + obj.cls.name + "` has no field `" + name + "`", line)
            obj.fields[at] = value
            return
        }
        fatal("cannot set a field on " + typeName(obj), line)
    }

    // --------------------------------------------------------------- Calls

    fun counted(n: Int, word: String): String = if (n == 1) "1 " + word else n.toString() + " " + word + "s"

    fun were(n: Int): String = if (n == 1) "1 was" else n.toString() + " were"

    fun bind(what: String, names: Array<String>, positional: Array<Any?>, namedNames: Array<String>, named: Array<Any?>, line: Int): Array<Any?> {
        if (positional.size > names.size) {
            fatal("`" + what + "` takes " + counted(names.size, "argument") + ", but " + were(positional.size) + " given", line)
        }
        val slots = arrayOfNulls<Any?>(names.size)
        for (i in 0 until names.size) slots[i] = if (i < positional.size) positional[i] else MISSING
        for (i in 0 until namedNames.size) {
            val at = names.indexOf(namedNames[i])
            if (at < 0) fatal("`" + what + "` has no argument named `" + namedNames[i] + "`", line)
            if (slots[at] !== MISSING) fatal("`" + what + "` was given `" + namedNames[i] + "` twice", line)
            slots[at] = named[i]
        }
        return slots
    }

    // How many calls are nested now. `zeno run` stops at 1000.
    var depth = 0

    fun enter(f: ZFunc, line: Int) {
        if (depth >= 1000) {
            val name = if (f.name.isEmpty()) "function" else f.name
            fatal("more than 1000 calls are nested. Is `" + name + "` calling itself forever?", line)
        }
        depth++
    }

    // A call of a function of the program known where it is written, with
    // its arguments lined up already.
    fun direct(f: Any?, line: Int, args: Array<Any?>): Any? {
        val fn = f as ZFunc
        enter(fn, line)
        try {
            return fn.body!!(args)
        } finally {
            depth--
        }
    }

    val NO_NAMES = arrayOf<String>()
    val NO_VALUES = arrayOf<Any?>()

    fun callFunction(f: ZFunc, positional: Array<Any?>, namedNames: Array<String>, named: Array<Any?>, line: Int): Any? {
        val name = if (f.name.isEmpty()) "function" else f.name
        enter(f, line)
        try {
            val slots = bind(name, f.params, positional, namedNames, named, line)
            for (i in 0 until slots.size) {
                if (slots[i] === MISSING && !f.defaults[i]) fatal("`" + name + "` needs a value for `" + f.params[i] + "`", line)
            }
            return f.body!!(slots)
        } finally {
            depth--
        }
    }

    fun noNamed(what: String, namedNames: Array<String>, line: Int) {
        if (namedNames.isNotEmpty()) fatal("`" + what + "` does not take a named argument `" + namedNames[0] + "`", line)
    }

    fun prepend(first: Any?, rest: Array<Any?>): Array<Any?> {
        val out = arrayOfNulls<Any?>(rest.size + 1)
        out[0] = first
        System.arraycopy(rest, 0, out, 1, rest.size)
        return out
    }

    fun call_named(callee: Any?, positional: Array<Any?>, namedNames: Array<String>, named: Array<Any?>, line: Int): Any? {
        if (callee is ZBound) return callFunction(callee.fn, prepend(callee.obj, positional), namedNames, named, line)
        if (callee is ZMethod) {
            noNamed(callee.name, namedNames, line)
            return callMethodOf(callee.receiver, callee.name, positional, line)
        }
        if (callee is ZClass) {
            val slots = bind(callee.name, callee.fields, positional, namedNames, named, line)
            for (i in 0 until slots.size) {
                if (slots[i] === MISSING) {
                    val byDefault = callee.defaults[i] ?: fatal("`" + callee.name + "` needs a value for `" + callee.fields[i] + "`", line)
                    slots[i] = (byDefault as ZFunc).body!!(NO_VALUES)
                }
            }
            return ZInstance(callee, slots)
        }
        if (callee is ZConstructor) {
            val name = callee.def.cases[callee.index]
            val fields = callee.def.fields[callee.index]
            val slots = bind(name, fields, positional, namedNames, named, line)
            for (i in 0 until slots.size) {
                if (slots[i] === MISSING) fatal("`" + name + "` needs a value for `" + fields[i] + "`", line)
            }
            return ZCase(callee.def, callee.index, slots)
        }
        if (callee is ZErrorType) {
            noNamed(callee.kind, namedNames, line)
            return newError(callee.kind, positional, line)
        }
        if (callee is ZFunc) {
            val bbody = callee.bbody
            if (bbody != null) {
                noNamed(callee.name, namedNames, line)
                return bbody(positional, line)
            }
            return callFunction(callee, positional, namedNames, named, line)
        }
        fatal(typeName(callee) + " cannot be called", line)
    }

    fun call(callee: Any?, args: Array<Any?>, line: Int): Any? = call_named(callee, args, NO_NAMES, NO_VALUES, line)

    fun newError(kind: String, args: Array<Any?>, line: Int): Any? {
        if (args.size > 1) fatal("`" + kind + "` takes one message", line)
        return ZError(kind, if (args.size == 1) show(args[0]) else "", null)
    }

    fun call_method_named(obj: Any?, name: String, positional: Array<Any?>, namedNames: Array<String>, named: Array<Any?>, line: Int): Any? {
        if (obj is ZInstance && obj.cls.fields.indexOf(name) < 0) {
            val m = obj.cls.methods[name]
            if (m != null) return callFunction(m, prepend(obj, positional), namedNames, named, line)
        }
        if (obj is ZModule) {
            noNamed(name, namedNames, line)
            return call_builtin(obj.name + "." + name, positional, line)
        }
        if (obj is String || obj is ArrayList<*> || obj is LinkedHashMap<*, *>) {
            noNamed(name, namedNames, line)
            return callMethodOf(obj, name, positional, line)
        }
        return call_named(field(obj, name, line), positional, namedNames, named, line)
    }

    fun call_method(obj: Any?, name: String, args: Array<Any?>, line: Int): Any? =
        call_method_named(obj, name, args, NO_NAMES, NO_VALUES, line)

    // -------------------------------------------------------------- Errors

    fun raise(value: Any?, line: Int) {
        if (value is ZError) throw ZRaise(value, line)
        if (value is ZInstance) throw ZRaise(ZError(value.cls.name, show(value), value), line)
        fatal("only an error can be raised, and this is " + typeName(value), line)
    }

    fun catches(e: ZRaise, name: String): Boolean = name == "Error" || name == e.value.kind

    fun caught(e: ZRaise, name: String): Any? {
        if (name.isNotEmpty() && name != "Error" && e.value.obj != null) return e.value.obj
        return e.value
    }

    fun no_match(v: Any?, line: Int) {
        fatal("no `case` matches " + repr(v), line)
    }

    fun undefined(name: String, line: Int): Any? = fatal("`" + name + "` is not defined", line)

    // An expression whose value is not wanted.
    fun ignore(v: Any?) {}

    // -------------------------------------------------- Built-in functions

    fun arity(name: String, args: Array<Any?>, lo: Int, hi: Int, line: Int) {
        if (args.size >= lo && args.size <= hi) return
        val wanted = if (lo == hi) counted(lo, "argument") else lo.toString() + " to " + hi + " arguments"
        fatal("`" + name + "` takes " + wanted + ", but " + were(args.size) + " given", line)
    }

    fun wrongType(name: String, wanted: String, got: Any?, line: Int): Nothing =
        fatal("`" + name + "` needs " + wanted + ", but this is " + typeName(got), line)

    fun text(name: String, v: Any?, line: Int): String {
        if (v !is String) wrongType(name, "a str", v, line)
        return v
    }

    fun wholeArg(name: String, v: Any?, line: Int): Long {
        if (v !is Long) wrongType(name, "a whole number", v, line)
        return v
    }

    fun blank(c: Char): Boolean = c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\u000c'

    fun strip(s: String): String {
        var a = 0
        var b = s.length
        while (a < b && blank(s[a])) a++
        while (b > a && blank(s[b - 1])) b--
        return s.substring(a, b)
    }

    val OUT = java.io.PrintStream(java.io.BufferedOutputStream(java.io.FileOutputStream(java.io.FileDescriptor.out), 65536), false, "UTF-8")

    fun flush() {
        OUT.flush()
    }

    val WIDTHS = HashMap<String, ZWidth>()

    init {
        WIDTHS["u8"] = ZWidth(0, 255, 8, false)
        WIDTHS["u16"] = ZWidth(0, 65535, 16, false)
        WIDTHS["u32"] = ZWidth(0, 4294967295L, 32, false)
        WIDTHS["i8"] = ZWidth(-128, 127, 8, true)
        WIDTHS["i16"] = ZWidth(-32768, 32767, 16, true)
        WIDTHS["i32"] = ZWidth(-2147483648L, 2147483647L, 32, true)
    }

    fun amount(v: Any?): Double = if (v is ZQuantity) v.v else number(v)

    // The numbers `min`, `max` and `sum` are given, and the unit they
    // are in, if they are quantities.
    var unitOfNumbers: String? = null

    fun numbers(name: String, args: Array<Any?>, line: Int): List<Any?> {
        val first = if (args.size == 1) args[0] else null
        val values: List<Any?> = if (first is ArrayList<*>) first else args.asList()
        var unit: String? = null
        for (place in 0 until values.size) {
            val item = values[place]
            if (item is ZQuantity) {
                if (unit != null && unit != item.unit) {
                    fatal("`" + name + "` needs quantities in one unit, but these are in " + unit + " and in " + item.unit + ". Turn one into the other first, as in `x." + unit + "`", line)
                }
                if (unit == null && place > 0) fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line)
                unit = item.unit
            } else if (!isNumber(item)) {
                wrongType(name, "numbers", item, line)
            } else if (unit != null) {
                fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line)
            }
        }
        unitOfNumbers = unit
        return values
    }

    fun pick(name: String, args: Array<Any?>, line: Int, smaller: Boolean): Any? {
        var values = numbers(name, args, line)
        if (values.isEmpty()) raiseError("ValueError", "`" + name + "` of nothing", line)
        if (values.any { it is Float } && values.all { it is Float || it is Long }) {
            values = values.map { single(it)!!.toFloat() as Any? }
        }
        var best = values[0]
        for (i in 1 until values.size) {
            val item = values[i]
            val top = best
            if (item is ULong && top is ULong) {
                if (if (smaller) item < top else item > top) best = item
                continue
            }
            if (if (smaller) amount(item) < amount(best) else amount(item) > amount(best)) best = item
        }
        return best
    }

    val FLOAT: java.util.regex.Pattern = java.util.regex.Pattern.compile(
        "[+-]?([0-9]+\\.?[0-9]*([eE][+-]?[0-9]+)?|\\.[0-9]+([eE][+-]?[0-9]+)?|(?i:inf|infinity|nan))",
    )

    // A float as Rust's `parse::<f64>` reads it, or null.
    fun parseFloat(s: String): Double? {
        if (!FLOAT.matcher(s).matches()) return null
        val body = (if (s.startsWith("-") || s.startsWith("+")) s.substring(1) else s).lowercase()
        val negative = s.startsWith("-")
        if (body == "inf" || body == "infinity") return if (negative) Double.NEGATIVE_INFINITY else Double.POSITIVE_INFINITY
        if (body == "nan") return Double.NaN
        return java.lang.Double.parseDouble(s)
    }

    // Heapsort, step for step as the Rust code sorts, so that a function
    // given to `sort` is called in the same order.
    fun heapsort(n: Int, before: (Int, Int) -> Boolean, swap: (Int, Int) -> Unit) {
        var root = n / 2 - 1
        while (root >= 0) {
            sift(root, n, before, swap)
            root--
        }
        var end = n - 1
        while (end > 0) {
            swap(0, end)
            sift(0, end, before, swap)
            end--
        }
    }

    fun sift(from: Int, end: Int, before: (Int, Int) -> Boolean, swap: (Int, Int) -> Unit) {
        var root = from
        while (true) {
            var child = 2 * root + 1
            if (child >= end) return
            if (child + 1 < end && before(child, child + 1)) child++
            if (!before(root, child)) return
            swap(root, child)
            root = child
        }
    }

    fun sortList(l: ArrayList<Any?>, line: Int) {
        val s = ArrayList<Any?>(l)
        heapsort(s.size, { i, j -> lt(s[i], s[j], line) }, { i, j -> java.util.Collections.swap(s, i, j) })
        l.clear()
        l.addAll(s)
    }

    fun sortBy(l: ArrayList<Any?>, function: Any?, line: Int) {
        val keys = ArrayList<Any?>()
        for (item in ArrayList<Any?>(l)) {
            val k = call(function, arrayOf<Any?>(item), line)
            if (k !is Long && k !is Double && k !is Float && k !is ULong && k !is String) {
                fatal("the function given to `sort` must return a number or text to order by, but it returned " + typeName(k), line)
            }
            keys.add(k)
        }
        val s = ArrayList<Any?>(l)
        if (s.size != keys.size) raiseError("ValueError", "the function given to `sort` changed how many items the list has", line)
        val places = IntArray(s.size) { it }
        heapsort(s.size, { a, b ->
            if (lt(keys[a], keys[b], line)) true
            else if (lt(keys[b], keys[a], line)) false
            else places[a] < places[b]
        }, { i, j ->
            java.util.Collections.swap(s, i, j)
            java.util.Collections.swap(keys, i, j)
            val t = places[i]
            places[i] = places[j]
            places[j] = t
        })
        l.clear()
        l.addAll(s)
    }

    // {from, to} of a slice.
    fun slice(args: Array<Any?>, length: Int, what: String, line: Int): IntArray {
        val from = wholeArg("slice", args[0], line)
        val to = if (args.size > 1) wholeArg("slice", args[1], line) else length.toLong()
        if (from < 0 || to < from || to > length) {
            raiseError("IndexError", "slice " + from + " to " + to + " is out of range for a " + what + " of length " + length, line)
        }
        return intArrayOf(from.toInt(), to.toInt())
    }

    val STR_METHODS = listOf("upper", "lower", "strip", "split", "lines", "replace", "starts_with", "ends_with", "find", "slice", "code")
    val LIST_METHODS = listOf("append", "pop", "map", "filter", "reduce", "insert", "reverse", "sort", "clear", "find", "slice", "join")
    val DICT_METHODS = listOf("get", "pop", "keys", "values", "clear")

    fun hasMethod(v: Any?, name: String): Boolean {
        if (v is String) return STR_METHODS.contains(name)
        if (v is ArrayList<*>) return LIST_METHODS.contains(name)
        if (v is LinkedHashMap<*, *>) return DICT_METHODS.contains(name)
        return false
    }

    fun splitOn(s: String, sep: String): ArrayList<Any?> {
        val out = ArrayList<Any?>()
        var from = 0
        while (true) {
            val at = s.indexOf(sep, from)
            if (at < 0) break
            out.add(s.substring(from, at))
            from = at + sep.length
        }
        out.add(s.substring(from))
        return out
    }

    fun strMethod(s: String, name: String, args: Array<Any?>, line: Int): Any? {
        when (name) {
            "upper", "lower" -> {
                arity(name, args, 0, 0, line)
                val cs = s.toCharArray()
                for (i in 0 until cs.size) {
                    if (name == "upper" && cs[i] >= 'a' && cs[i] <= 'z') cs[i] = cs[i] - 32
                    else if (name == "lower" && cs[i] >= 'A' && cs[i] <= 'Z') cs[i] = cs[i] + 32
                }
                return String(cs)
            }
            "strip" -> {
                arity(name, args, 0, 0, line)
                return strip(s)
            }
            "split" -> {
                arity(name, args, 0, 1, line)
                if (args.isEmpty()) {
                    val out = ArrayList<Any?>()
                    var i = 0
                    while (i < s.length) {
                        while (i < s.length && blank(s[i])) i++
                        val start = i
                        while (i < s.length && !blank(s[i])) i++
                        if (i > start) out.add(s.substring(start, i))
                    }
                    return out
                }
                val sep = text(name, args[0], line)
                if (sep.isEmpty()) raiseError("ValueError", "cannot split on an empty str", line)
                return splitOn(s, sep)
            }
            "lines" -> {
                arity(name, args, 0, 0, line)
                if (s.isEmpty()) return ArrayList<Any?>()
                // A "\r" goes with the "\n" just after it, and with nothing else.
                val parts = splitOn(s, "\n")
                val last = parts.removeAt(parts.size - 1) as String
                for (i in 0 until parts.size) {
                    val p = parts[i] as String
                    if (p.endsWith("\r")) parts[i] = p.substring(0, p.length - 1)
                }
                if (last.isNotEmpty()) parts.add(last)
                return parts
            }
            "replace" -> {
                arity(name, args, 2, 2, line)
                val old = text(name, args[0], line)
                val now = text(name, args[1], line)
                if (old.isEmpty()) {
                    val b = StringBuilder(now)
                    eachCode(s) { c -> b.appendCodePoint(c).append(now) }
                    return b.toString()
                }
                return (s as java.lang.String).replace(old, now)
            }
            "starts_with" -> {
                arity(name, args, 1, 1, line)
                return s.startsWith(text(name, args[0], line))
            }
            "ends_with" -> {
                arity(name, args, 1, 1, line)
                return s.endsWith(text(name, args[0], line))
            }
            "find" -> {
                arity(name, args, 1, 1, line)
                val at = s.indexOf(text(name, args[0], line))
                return if (at < 0) null else s.codePointCount(0, at).toLong()
            }
            "slice" -> {
                arity(name, args, 1, 2, line)
                val r = slice(args, textLength(s), "str", line)
                return part(s, r[0], r[1])
            }
            "code" -> {
                arity(name, args, 0, 0, line)
                val n = textLength(s)
                if (n != 1) raiseError("ValueError", "`code` needs a str of exactly one character, and this one has " + n, line)
                return s.codePointAt(0).toLong()
            }
            else -> fatal("a str has no method `" + name + "`", line)
        }
    }

    fun dictMethod(d: LinkedHashMap<Any?, Any?>, name: String, args: Array<Any?>, line: Int): Any? {
        when (name) {
            "get" -> {
                arity(name, args, 1, 2, line)
                val k = key(args[0], line)
                if (d.containsKey(k)) return d[k]
                return if (args.size > 1) args[1] else null
            }
            "pop" -> {
                arity(name, args, 1, 1, line)
                val k = key(args[0], line)
                if (!d.containsKey(k)) raiseError("KeyError", "this dict has no key " + repr(args[0]), line)
                return d.remove(k)
            }
            "keys" -> {
                arity(name, args, 0, 0, line)
                return ArrayList<Any?>(d.keys)
            }
            "values" -> {
                arity(name, args, 0, 0, line)
                return ArrayList<Any?>(d.values)
            }
            "clear" -> {
                arity(name, args, 0, 0, line)
                d.clear()
                return null
            }
            else -> fatal("a dict has no method `" + name + "`", line)
        }
    }

    fun listMethod(l: ArrayList<Any?>, name: String, args: Array<Any?>, line: Int): Any? {
        when (name) {
            "append" -> {
                arity(name, args, 1, 1, line)
                l.add(args[0])
                return null
            }
            "pop" -> {
                arity(name, args, 0, 1, line)
                if (args.isNotEmpty()) return l.removeAt(position(args[0], l.size, "list", line))
                if (l.isEmpty()) raiseError("IndexError", "cannot pop from an empty list", line)
                return l.removeAt(l.size - 1)
            }
            "insert" -> {
                arity(name, args, 2, 2, line)
                val at = wholeArg(name, args[0], line)
                if (at < 0 || at > l.size) {
                    raiseError("IndexError", "cannot insert at " + at + " in a list of length " + l.size, line)
                }
                l.add(at.toInt(), args[1])
                return null
            }
            "reverse" -> {
                arity(name, args, 0, 0, line)
                java.util.Collections.reverse(l)
                return null
            }
            "clear" -> {
                arity(name, args, 0, 0, line)
                l.clear()
                return null
            }
            "sort" -> {
                if (args.size == 1) {
                    sortBy(l, args[0], line)
                    return null
                }
                arity(name, args, 0, 1, line)
                sortList(l, line)
                return null
            }
            "find" -> {
                arity(name, args, 1, 1, line)
                for (at in 0 until l.size) if (equal(l[at], args[0])) return at.toLong()
                return null
            }
            "slice" -> {
                arity(name, args, 1, 2, line)
                val r = slice(args, l.size, "list", line)
                return ArrayList<Any?>(l.subList(r[0], r[1]))
            }
            "join" -> {
                arity(name, args, 1, 1, line)
                val sep = text(name, args[0], line)
                val b = StringBuilder()
                for (i in 0 until l.size) {
                    val item = l[i]
                    if (item !is String) {
                        fatal("`join` puts text together, and this list holds " + typeName(item) + ". Make each item text first, with `str( )`", line)
                    }
                    if (i > 0) b.append(sep)
                    b.append(item)
                }
                return b.toString()
            }
            "map" -> {
                arity(name, args, 1, 1, line)
                val out = ArrayList<Any?>()
                for (item in ArrayList<Any?>(l)) out.add(call(args[0], arrayOf<Any?>(item), line))
                return out
            }
            "filter" -> {
                arity(name, args, 1, 1, line)
                val kept = ArrayList<Any?>()
                for (item in ArrayList<Any?>(l)) {
                    val keep = call(args[0], arrayOf<Any?>(item), line)
                    if (keep !is Boolean) {
                        fatal("the function given to `filter` must return true or false, but it returned " + typeName(keep), line)
                    }
                    if (keep) kept.add(item)
                }
                return kept
            }
            "reduce" -> {
                arity(name, args, 2, 2, line)
                var total = args[1]
                for (item in ArrayList<Any?>(l)) total = call(args[0], arrayOf<Any?>(total, item), line)
                return total
            }
            else -> fatal("a list has no method `" + name + "`", line)
        }
    }

    fun callMethodOf(receiver: Any?, name: String, args: Array<Any?>, line: Int): Any? {
        if (receiver is String) return strMethod(receiver, name, args, line)
        if (receiver is LinkedHashMap<*, *>) return dictMethod(asDict(receiver), name, args, line)
        if (receiver is ArrayList<*>) return listMethod(asList(receiver), name, args, line)
        fatal(typeName(receiver) + " has no method `" + name + "`", line)
    }

    val FORMAT: java.util.regex.Pattern = java.util.regex.Pattern.compile("([<>^])?(0)?(\\d*)(?:\\.(\\d*))?(.)?", java.util.regex.Pattern.DOTALL)

    // `{x:>8.2f}`: [<>^]? 0? width? (.precision)? kind?
    fun format(v: Any?, spec: String, line: Int): Any? {
        if (spec.isEmpty()) return show(v)
        val m = FORMAT.matcher(spec)
        if (!m.matches() || (m.group(5) != null && Character.isDigit(m.group(5)[0]))) {
            fatal("`" + spec + "` is not a format Zeno understands", line)
        }
        val align: String? = m.group(1)
        val zero = m.group(2) != null
        val width = if (m.group(3).isEmpty()) 0 else Integer.parseInt(m.group(3))
        val precision: Int? = if (m.group(4) == null) null else (if (m.group(4).isEmpty()) 0 else Integer.parseInt(m.group(4)))
        val kind: String? = m.group(5)
        val places = precision ?: 6
        val body: String
        if ((kind == "f" || (kind == null && precision != null)) && v is ZQuantity) {
            body = fixed(v.v, places) + " " + v.unit
        } else if (kind == "f" || (kind == null && precision != null && v is Double)) {
            if (!isNumber(v)) wrongType(spec, "a number", v, line)
            body = fixed(number(v), places)
        } else if ((kind == null && precision == null) || (kind == "d" && precision == null && (v is Long || v is ULong))) {
            body = show(v)
        } else if ((kind == "x" || kind == "X" || kind == "b") && precision == null && v is ULong) {
            val digits = v.toString(if (kind == "b") 2 else 16)
            body = if (kind == "X") digits.uppercase() else digits
        } else if ((kind == "x" || kind == "X" || kind == "b") && precision == null && v is Long) {
            var digits = java.lang.Long.toUnsignedString(if (v < 0) -v else v, if (kind == "b") 2 else 16)
            if (v == Long.MIN_VALUE) digits = if (kind == "b") "1" + "0".repeat(63) else "8000000000000000"
            if (kind == "X") digits = digits.uppercase()
            body = (if (v < 0) "-" else "") + digits
        } else if ((kind == "d" || kind == "x" || kind == "X" || kind == "b") && precision == null) {
            wrongType(spec, "an int", v, line)
        } else {
            fatal("`" + spec + "` is not a format Zeno understands", line)
        }
        val padding = Math.max(0, width - textLength(body))
        val numeric = isNumber(v) || v is ZQuantity
        if (zero && numeric) {
            if (body.startsWith("-")) return "-" + "0".repeat(padding) + body.substring(1)
            return "0".repeat(padding) + body
        }
        val a = align ?: (if (numeric) ">" else "<")
        if (a == "<") return body + " ".repeat(padding)
        if (a == ">") return " ".repeat(padding) + body
        val left = padding / 2
        return " ".repeat(left) + body + " ".repeat(padding - left)
    }

    // ---------------------------------------------------------------- JSON

    val NOT_JSON = Any()

    fun jsonParse(s: String): Any? {
        try {
            val j = ZJson(s)
            val v = j.value()
            j.spaces()
            return if (j.at == j.cs.size) v else NOT_JSON
        } catch (e: ZNotJson) {
            return NOT_JSON
        }
    }

    fun jsonText(s: String): String {
        val b = StringBuilder("\"")
        eachCode(s) { c ->
            if (c == '"'.code) b.append("\\\"")
            else if (c == '\\'.code) b.append("\\\\")
            else if (c == '\n'.code) b.append("\\n")
            else if (c == '\r'.code) b.append("\\r")
            else if (c == '\t'.code) b.append("\\t")
            else if (c < 0x20) b.append(String.format("\\u%04x", c))
            else b.appendCodePoint(c)
        }
        return b.append('"').toString()
    }

    fun jsonWrite(v: Any?, line: Int): String {
        if (v == null) return "null"
        if (v is Boolean) return if (v) "true" else "false"
        if (v is Long) return v.toString()
        if (v is Double) {
            if (v.isNaN() || v.isInfinite()) raiseError("ValueError", "JSON has no way to write " + showFloat(v), line)
            return showFloat(v)
        }
        if (v is String) return jsonText(v)
        if (v is ArrayList<*>) {
            val parts = ArrayList<String>()
            for (item in v) parts.add(jsonWrite(item, line))
            return "[" + parts.joinToString(",") + "]"
        }
        if (v is LinkedHashMap<*, *>) {
            val parts = ArrayList<String>()
            for (entry in v.entries) parts.add(jsonText(show(entry.key)) + ":" + jsonWrite(entry.value, line))
            return "{" + parts.joinToString(",") + "}"
        }
        if (v is ZInstance) {
            val parts = ArrayList<String>()
            for (i in 0 until v.cls.fields.size) parts.add(jsonText(v.cls.fields[i]) + ":" + jsonWrite(v.fields[i], line))
            return "{" + parts.joinToString(",") + "}"
        }
        if (v is ZQuantity) {
            raiseError("ValueError", "JSON has no units, and this is a quantity in " + v.unit + ". Write its number, and put the unit in the name: `[\"" + v.unit + "\": ...]`", line)
        }
        raiseError("ValueError", "JSON has objects, lists, text, numbers, bools and none, and this is " + typeName(v), line)
    }

    // --------------------------------------------------------------- Files

    // "file", "nothing" or "other": what is at a path.
    fun at(path: String): String {
        // Java takes a path with nothing in it for the folder it runs in.
        if (path.isEmpty()) return "nothing"
        try {
            val stream = java.io.FileInputStream(path)
            try {
                stream.read()
            } finally {
                stream.close()
            }
            return "file"
        } catch (e: java.io.FileNotFoundException) {
            if (java.io.File(path).exists()) return "other"
            // A path through a file (`notes.txt/x`) is not "nothing":
            // something is in the way, as `zeno run` says.
            var up = java.io.File(path).parentFile
            while (up != null) {
                if (up.exists()) return if (up.isDirectory) "nothing" else "other"
                up = up.parentFile
            }
            return "nothing"
        } catch (e: java.io.IOException) {
            return "other"
        }
    }

    fun paths(name: String, args: Array<Any?>, count: Int, line: Int): Array<String> {
        arity(name, args, count, count, line)
        return Array(args.size) { i -> text(name, args[i], line) }
    }

    fun fileError(message: String, line: Int): Nothing = raiseError("FileError", message, line)

    val STARTED = System.nanoTime()

    // ------------------------------------------------- The built-in table

    fun b_print(args: Array<Any?>, line: Int): Any? {
        for (i in 0 until args.size) {
            if (i > 0) OUT.print(' ')
            OUT.print(show(args[i]))
        }
        OUT.print('\n')
        return null
    }

    val IN: java.io.InputStream = java.io.BufferedInputStream(System.`in`)

    // Text from outside that may not be UTF-8. Each run of bytes that
    // could have begun a character and did not finish one becomes one
    // U+FFFD, and so does each byte that could not begin one: the way
    // Rust, Python and the browsers read such text. Java's own reader
    // counts some of them otherwise.
    fun lossy(b: ByteArray): String {
        val out = java.io.ByteArrayOutputStream()
        var i = 0
        while (i < b.size) {
            val c = b[i].toInt() and 0xFF
            if (c < 0x80) {
                out.write(c)
                i++
                continue
            }
            var need = 0
            var lo = 0x80
            var hi = 0xBF
            if (c >= 0xC2 && c <= 0xDF) need = 1
            else if (c == 0xE0) {
                need = 2
                lo = 0xA0
            } else if (c == 0xED) {
                need = 2
                hi = 0x9F
            } else if (c >= 0xE1 && c <= 0xEF) need = 2
            else if (c == 0xF0) {
                need = 3
                lo = 0x90
            } else if (c == 0xF4) {
                need = 3
                hi = 0x8F
            } else if (c >= 0xF1 && c <= 0xF3) need = 3
            var j = i + 1
            var whole = need > 0
            var k = 0
            while (whole && k < need) {
                if (j < b.size && (b[j].toInt() and 0xFF) >= lo && (b[j].toInt() and 0xFF) <= hi) j++
                else whole = false
                lo = 0x80
                hi = 0xBF
                k++
            }
            if (whole) out.write(b, i, j - i)
            else {
                out.write(0xEF)
                out.write(0xBF)
                out.write(0xBD)
            }
            i = j
        }
        return String(out.toByteArray(), java.nio.charset.StandardCharsets.UTF_8)
    }

    fun b_input(args: Array<Any?>, line: Int): Any? {
        arity("input", args, 0, 1, line)
        if (args.isNotEmpty()) OUT.print(text("input", args[0], line))
        flush()
        val bytes = java.io.ByteArrayOutputStream()
        var any = false
        var ended = false
        try {
            while (true) {
                val c = IN.read()
                if (c < 0) break
                any = true
                if (c == '\n'.code) {
                    ended = true
                    break
                }
                bytes.write(c)
            }
        } catch (e: java.io.IOException) {
            // As at the end of the input.
        }
        if (!any) return null
        var s = lossy(bytes.toByteArray())
        if (ended && s.endsWith("\r")) s = s.substring(0, s.length - 1)
        return (s as java.lang.String).replace("\u0000", "")
    }

    fun b_len(args: Array<Any?>, line: Int): Any? {
        arity("len", args, 1, 1, line)
        val v = args[0]
        if (v is String) return textLength(v).toLong()
        if (v is ArrayList<*>) return v.size.toLong()
        if (v is LinkedHashMap<*, *>) return v.size.toLong()
        wrongType("len", "a str, a list or a dict", v, line)
    }

    fun b_range(args: Array<Any?>, line: Int): Any? {
        arity("range", args, 1, 3, line)
        val n = LongArray(args.size)
        for (i in 0 until args.size) {
            val x = args[i]
            if (x !is Long) wrongType("range", "ints", x, line)
            n[i] = x
        }
        var start = 0L
        val stop: Long
        var step = 1L
        if (n.size == 1) stop = n[0]
        else if (n.size == 2) {
            start = n[0]
            stop = n[1]
        } else {
            start = n[0]
            stop = n[1]
            step = n[2]
        }
        if (step == 0L) raiseError("ValueError", "the step of a range cannot be 0", line)
        val out = ArrayList<Any?>()
        var i = start
        while (if (step > 0) i < stop else i > stop) {
            out.add(i)
            val next = i + step
            if (if (step > 0) next < i else next > i) break
            i = next
        }
        return out
    }

    // A float as an int: one too big for an int is the biggest int there
    // is, and what is not a number is 0, as in `zeno run`.
    fun toInt(x: Double): Long = if (x.isNaN()) 0L else x.toLong()

    val WHOLE: java.util.regex.Pattern = java.util.regex.Pattern.compile("[+-]?[0-9]+")

    fun b_int(args: Array<Any?>, line: Int): Any? {
        arity("int", args, 1, 1, line)
        val v = args[0]
        if (v is Long) return v
        if (v is ULong) {
            if (v > Long.MAX_VALUE.toULong()) raiseError("OverflowError", v.toString() + " does not fit in an int (-9223372036854775808 to 9223372036854775807)", line)
            return v.toLong()
        }
        if (v is Double || v is Float) {
            val x = number(v)
            if (x.isNaN() || x.isInfinite()) raiseError("ValueError", "cannot turn " + showFloat(x) + " into an int", line)
            return toInt(x)
        }
        if (v is String) {
            val t = (strip(v) as java.lang.String).replace("_", "")
            if (WHOLE.matcher(t).matches()) {
                try {
                    return java.lang.Long.parseLong(if (t.startsWith("+")) t.substring(1) else t)
                } catch (e: NumberFormatException) {
                    // Too big: as below.
                }
            }
            raiseError("ValueError", "cannot turn " + quoted(v) + " into an int", line)
        }
        wrongType("int", "a number or a str", v, line)
    }

    fun b_float(args: Array<Any?>, line: Int): Any? {
        arity("float", args, 1, 1, line)
        val v = args[0]
        if (v is Long) return v.toDouble()
        if (v is Double) return v
        if (v is Float) return v.toDouble()
        if (v is ULong) return v.toDouble()
        if (v is String) {
            return parseFloat(strip(v)) ?: raiseError("ValueError", "cannot turn " + quoted(v) + " into a float", line)
        }
        wrongType("float", "a number or a str", v, line)
    }

    fun b_f32(args: Array<Any?>, line: Int): Any? {
        arity("f32", args, 1, 1, line)
        val v = args[0]
        if (v is Float) return v
        if (v is Long) return single(v)!!.toFloat()
        if (v is Double) {
            val r = fround(v)
            if (r.isInfinite() && !v.isInfinite() && !v.isNaN()) {
                raiseError("OverflowError", showFloat(v) + " does not fit in an f32, whose largest is about 3.4e38", line)
            }
            return r.toFloat()
        }
        wrongType("f32", "a number", v, line)
    }

    fun b_str(args: Array<Any?>, line: Int): Any? {
        arity("str", args, 1, 1, line)
        return show(args[0])
    }

    fun b_abs(args: Array<Any?>, line: Int): Any? {
        arity("abs", args, 1, 1, line)
        val v = args[0]
        if (v is Long) {
            if (v == Long.MIN_VALUE) overflow(line)
            return Math.abs(v)
        }
        if (v is Double) return Math.abs(v)
        if (v is Float) return Math.abs(v)
        if (v is ULong) return v
        if (v is ZQuantity) return ZQuantity(Math.abs(v.v), v.unit)
        wrongType("abs", "a number", v, line)
    }

    fun b_min(args: Array<Any?>, line: Int): Any? = pick("min", args, line, true)

    fun b_max(args: Array<Any?>, line: Int): Any? = pick("max", args, line, false)

    fun b_sum(args: Array<Any?>, line: Int): Any? {
        val values = numbers("sum", args, line)
        if (unitOfNumbers != null) {
            fatal("`sum` works on plain numbers, and these have a unit. Add quantities with `+`, which keeps the unit: start from `var total = 0 V` and add each in a loop", line)
        }
        if (values.any { it is Float } && values.all { it is Float || it is Long }) {
            var single = 0.0
            for (item in values) single = fround(single + single(item)!!)
            return single.toFloat()
        }
        if (values.any { it is ULong }) {
            var sum = 0uL
            for (item in values) {
                val r = sum + (unsigned(item) ?: 0uL)
                if (r < sum) raiseError("OverflowError", U64_TOO_BIG, line)
                sum = r
            }
            return sum
        }
        var total = 0L
        var allInts = true
        var floatTotal = 0.0
        for (item in values) {
            floatTotal += number(item)
            if (allInts && item is Long) total = addInt(total, item, line)
            else allInts = false
        }
        return if (allInts) total else floatTotal
    }

    fun b_char(args: Array<Any?>, line: Int): Any? {
        arity("char", args, 1, 1, line)
        val code = wholeArg("char", args[0], line)
        if (code <= 0 || code > 0x10FFFF || (code >= 0xD800 && code <= 0xDFFF)) {
            raiseError("ValueError", code.toString() + " is not the code of a character", line)
        }
        return String(Character.toChars(code.toInt()))
    }

    fun b_allocate(args: Array<Any?>, line: Int): Any? = fatal("manual memory is only for a program compiled to machine code", line)

    fun b_free(args: Array<Any?>, line: Int): Any? = b_allocate(args, line)

    fun oneNumber(name: String, args: Array<Any?>, line: Int): Double {
        arity(name, args, 1, 1, line)
        if (!isNumber(args[0])) wrongType(name, "a number", args[0], line)
        return number(args[0])
    }

    fun angle(name: String, args: Array<Any?>, line: Int): Double {
        arity(name, args, 1, 1, line)
        val v = args[0]
        if (v is ZQuantity) {
            val s = convert(v.unit, "rad")
                ?: fatal("`" + name + "` needs a number or an angle, such as `30 deg`, not a quantity in " + v.unit, line)
            return apply(s, v.v)
        }
        if (!isNumber(v)) wrongType(name, "a number or an angle, such as `30 deg`", v, line)
        return number(v)
    }

    fun ratio(name: String, args: Array<Any?>, line: Int): Double {
        val x = oneNumber(name, args, line)
        if (x < -1 || x > 1) raiseError("ValueError", "`" + name + "` needs a number from -1 to 1, and this is " + showFloat(x), line)
        return x
    }

    fun aboveZero(name: String, args: Array<Any?>, line: Int): Double {
        val x = oneNumber(name, args, line)
        if (x <= 0) raiseError("ValueError", "cannot take the logarithm of " + showFloat(x), line)
        return x
    }

    fun b_math_sqrt(args: Array<Any?>, line: Int): Any? {
        val x = oneNumber("sqrt", args, line)
        if (x < 0) raiseError("ValueError", "cannot take the square root of " + showFloat(x), line)
        return Math.sqrt(x)
    }

    fun b_math_floor(args: Array<Any?>, line: Int): Any? = toInt(Math.floor(oneNumber("floor", args, line)))

    fun b_math_ceil(args: Array<Any?>, line: Int): Any? = toInt(Math.ceil(oneNumber("ceil", args, line)))

    // A half goes away from zero, as Rust's `round` does.
    fun b_math_round(args: Array<Any?>, line: Int): Any? {
        val x = oneNumber("round", args, line)
        if (x.isNaN()) return 0L
        var r = Math.floor(Math.abs(x))
        if (Math.abs(x) - r >= 0.5) r += 1
        return toInt(Math.copySign(r, x))
    }

    fun b_math_sin(args: Array<Any?>, line: Int): Any? = Math.sin(angle("sin", args, line))

    fun b_math_cos(args: Array<Any?>, line: Int): Any? = Math.cos(angle("cos", args, line))

    fun b_math_tan(args: Array<Any?>, line: Int): Any? = Math.tan(angle("tan", args, line))

    fun b_math_asin(args: Array<Any?>, line: Int): Any? = ZQuantity(Math.asin(ratio("asin", args, line)), "rad")

    fun b_math_acos(args: Array<Any?>, line: Int): Any? = ZQuantity(Math.acos(ratio("acos", args, line)), "rad")

    fun b_math_atan(args: Array<Any?>, line: Int): Any? = ZQuantity(Math.atan(oneNumber("atan", args, line)), "rad")

    fun b_math_atan2(args: Array<Any?>, line: Int): Any? {
        arity("atan2", args, 2, 2, line)
        if (!isNumber(args[0])) wrongType("atan2", "two numbers", args[0], line)
        if (!isNumber(args[1])) wrongType("atan2", "two numbers", args[1], line)
        return ZQuantity(Math.atan2(number(args[0]), number(args[1])), "rad")
    }

    fun b_math_exp(args: Array<Any?>, line: Int): Any? = Math.exp(oneNumber("exp", args, line))

    fun b_math_log(args: Array<Any?>, line: Int): Any? = Math.log(aboveZero("log", args, line))

    fun b_math_log2(args: Array<Any?>, line: Int): Any? {
        val x = aboveZero("log2", args, line)
        // Exact for the powers of two, as Rust's `log2` is.
        val e = Math.getExponent(x)
        if (x == Math.scalb(1.0, e)) return e.toDouble()
        return Math.log(x) / Math.log(2.0)
    }

    fun b_math_log10(args: Array<Any?>, line: Int): Any? = Math.log10(aboveZero("log10", args, line))

    fun b_time_sleep(args: Array<Any?>, line: Int): Any? {
        arity("sleep", args, 1, 1, line)
        val v = args[0]
        if (v !is ZQuantity) wrongType("sleep", "a time with a unit, such as `500 ms`", v, line)
        val s = convert(v.unit, "s") ?: fatal("`sleep` needs a time, such as `500 ms`, not a quantity in " + v.unit, line)
        val seconds = apply(s, v.v)
        if (seconds > 0) {
            flush()
            try {
                val nanos = (Math.min(seconds, 1e9) * 1e9).toLong()
                Thread.sleep(nanos / 1000000, (nanos % 1000000).toInt())
            } catch (e: InterruptedException) {
                // Woken early: go on.
            }
        }
        return null
    }

    fun b_time_now(args: Array<Any?>, line: Int): Any? {
        arity("now", args, 0, 0, line)
        return ZQuantity((System.nanoTime() - STARTED) / 1e6, "ms")
    }

    fun b_files_read(args: Array<Any?>, line: Int): Any? {
        val path = paths("read", args, 1, line)[0]
        val where = at(path)
        if (where == "nothing") fileError("there is no file `" + path + "`", line)
        if (where == "other") fileError("cannot read `" + path + "`", line)
        val data: ByteArray
        try {
            data = java.nio.file.Files.readAllBytes(java.io.File(path).toPath())
        } catch (e: java.io.IOException) {
            fileError("cannot read `" + path + "`", line)
        }
        val s: String?
        s = try {
            java.nio.charset.StandardCharsets.UTF_8.newDecoder().decode(java.nio.ByteBuffer.wrap(data)).toString()
        } catch (e: java.nio.charset.CharacterCodingException) {
            null
        }
        if (s == null || s.indexOf('\u0000') >= 0) fileError("`" + path + "` does not hold text", line)
        return s
    }

    fun put(name: String, args: Array<Any?>, append: Boolean, line: Int): Any? {
        val p = paths(name, args, 2, line)
        try {
            val out = java.io.FileOutputStream(p[0], append)
            try {
                out.write(p[1].toByteArray(java.nio.charset.StandardCharsets.UTF_8))
            } finally {
                out.close()
            }
        } catch (e: java.io.IOException) {
            fileError("cannot write `" + p[0] + "`", line)
        }
        return null
    }

    fun b_files_write(args: Array<Any?>, line: Int): Any? = put("write", args, false, line)

    fun b_files_read_bytes(args: Array<Any?>, line: Int): Any? {
        val path = paths("read_bytes", args, 1, line)[0]
        val where = at(path)
        if (where == "nothing") fileError("there is no file `" + path + "`", line)
        if (where == "other") fileError("cannot read `" + path + "`", line)
        val data: ByteArray
        try {
            data = java.nio.file.Files.readAllBytes(java.io.File(path).toPath())
        } catch (e: java.io.IOException) {
            fileError("cannot read `" + path + "`", line)
        }
        val items = ArrayList<Any?>(data.size)
        for (b in data) items.add((b.toInt() and 0xFF).toLong())
        return items
    }

    fun b_files_write_bytes(args: Array<Any?>, line: Int): Any? {
        arity("write_bytes", args, 2, 2, line)
        val path = text("write_bytes", args[0], line)
        val list = args[1] as? ArrayList<*> ?: wrongType("write_bytes", "a list[u8]", args[1], line)
        val data = ByteArray(list.size)
        for (i in data.indices) {
            val n = list[i] as? Long ?: wrongType("write_bytes", "a list[u8]", list[i], line)
            if (n < 0 || n > 255) raiseError("ValueError", n.toString() + " is not a byte: a byte is a number from 0 to 255", line)
            data[i] = n.toByte()
        }
        try {
            val out = java.io.FileOutputStream(path, false)
            try {
                out.write(data)
            } finally {
                out.close()
            }
        } catch (e: java.io.IOException) {
            fileError("cannot write `" + path + "`", line)
        }
        return null
    }

    fun b_files_append(args: Array<Any?>, line: Int): Any? = put("append", args, true, line)

    fun b_files_exists(args: Array<Any?>, line: Int): Any? = at(paths("exists", args, 1, line)[0]) == "file"

    fun b_files_remove(args: Array<Any?>, line: Int): Any? {
        val path = paths("remove", args, 1, line)[0]
        val where = at(path)
        if (where == "nothing") fileError("there is no file `" + path + "`", line)
        if (where == "file" && java.io.File(path).delete()) return null
        fileError("cannot remove `" + path + "`", line)
    }

    // `files.list(folder)`: the names in it, sorted by the numbers of their
    // characters, as `zeno run` sorts them. Kotlin's own order for text
    // goes by UTF-16, which puts some characters in another order. An
    // empty path is the folder the program runs in, to the JVM, and no
    // folder at all to Zeno.
    fun byCharacters(a: String, b: String): Int {
        var i = 0
        var j = 0
        while (i < a.length && j < b.length) {
            val x = a.codePointAt(i)
            val y = b.codePointAt(j)
            if (x != y) return x.compareTo(y)
            i += Character.charCount(x)
            j += Character.charCount(y)
        }
        return (a.length - i).compareTo(b.length - j)
    }

    fun b_files_list(args: Array<Any?>, line: Int): Any? {
        val folder = paths("list", args, 1, line)[0]
        if (folder.isEmpty()) fileError("there is no folder ``", line)
        val names = ArrayList<String>()
        try {
            java.nio.file.Files.list(java.io.File(folder).toPath()).use { found ->
                found.forEach { entry -> names.add(entry.fileName.toString()) }
            }
        } catch (e: java.nio.file.NoSuchFileException) {
            fileError("there is no folder `" + folder + "`", line)
        } catch (e: Exception) {
            fileError("cannot list the folder `" + folder + "`", line)
        }
        names.sortWith { a, b -> byCharacters(a, b) }
        return ArrayList<Any?>(names)
    }

    // `files.make_folder(path)`: it worked if there is a folder there at the end.
    fun b_files_make_folder(args: Array<Any?>, line: Int): Any? {
        val path = paths("make_folder", args, 1, line)[0]
        var folder = false
        if (path.isNotEmpty()) {
            try {
                java.nio.file.Files.createDirectories(java.io.File(path).toPath())
            } catch (e: Exception) {
                // Said below.
            }
            folder = java.io.File(path).isDirectory
        }
        if (!folder) fileError("cannot make the folder `" + path + "`", line)
        return null
    }

    var arguments = arrayOf<String>()

    // `program.exit(code)`: what was printed goes out, and the program
    // ends with `code`.
    fun b_program_exit(args: Array<Any?>, line: Int): Any? {
        arity("exit", args, 1, 1, line)
        val code = wholeArg("exit", args[0], line)
        if (code < 0 || code > 255) {
            raiseError("ValueError", "a program ends with a number from 0 to 255, not " + code.toString(), line)
        }
        flush()
        System.err.flush()
        System.exit(code.toInt())
        return null
    }

    // `program.environment(name)`: the text, or none. A name with `=` or
    // a zero byte in it is in no environment.
    fun b_program_environment(args: Array<Any?>, line: Int): Any? {
        arity("environment", args, 1, 1, line)
        val name = text("environment", args[0], line)
        if (name.isEmpty() || name.indexOf('=') >= 0 || name.indexOf('\u0000') >= 0) return null
        return System.getenv(name)
    }

    // `program.warn(text)`: one line on the error stream, after what was printed.
    fun b_program_warn(args: Array<Any?>, line: Int): Any? {
        arity("warn", args, 1, 1, line)
        val said = text("warn", args[0], line)
        flush()
        System.err.print(said + "\n")
        System.err.flush()
        return null
    }

    fun b_program_arguments(args: Array<Any?>, line: Int): Any? {
        arity("arguments", args, 0, 0, line)
        val out = ArrayList<Any?>()
        for (word in arguments) out.add(word)
        return out
    }

    fun b_json_parse(args: Array<Any?>, line: Int): Any? {
        arity("parse", args, 1, 1, line)
        val s = text("parse", args[0], line)
        val v = jsonParse(s)
        if (v === NOT_JSON) {
            var a = 0
            while (a < s.length && " \t\n\r".indexOf(s[a]) >= 0) a++
            val start = s.substring(a)
            val n = textLength(start)
            val shown = if (n > 30) part(start, 0, 30) + "..." else start
            raiseError("ValueError", "this text is not JSON: `" + shown + "`", line)
        }
        return v
    }

    fun b_json_write(args: Array<Any?>, line: Int): Any? {
        arity("write", args, 1, 1, line)
        return jsonWrite(args[0], line)
    }

    fun sized(name: String, wrap: Boolean, args: Array<Any?>, line: Int): Any? {
        val w = WIDTHS[name]!!
        if (!wrap) {
            arity(name, args, 1, 1, line)
            val n = args[0]
            if (n !is Long) wrongType(name, "a whole number", n, line)
            if (n < w.lo || n > w.hi) {
                raiseError("OverflowError", n.toString() + " does not fit in " + (if (w.signed) "an" else "a") + " " + name + " (" + w.lo + " to " + w.hi + ")", line)
            }
            return n
        }
        arity(name + ".wrap", args, 1, 1, line)
        val n = args[0]
        if (n !is Long) wrongType(name + ".wrap", "a whole number", n, line)
        val low = n and ((1L shl w.bits) - 1)
        return if (w.signed && low > w.hi) low - (1L shl w.bits) else low
    }

    fun b_u8(args: Array<Any?>, line: Int): Any? = sized("u8", false, args, line)
    fun b_u8_wrap(args: Array<Any?>, line: Int): Any? = sized("u8", true, args, line)
    fun b_u16(args: Array<Any?>, line: Int): Any? = sized("u16", false, args, line)
    fun b_u16_wrap(args: Array<Any?>, line: Int): Any? = sized("u16", true, args, line)
    fun b_u32(args: Array<Any?>, line: Int): Any? = sized("u32", false, args, line)
    fun b_u32_wrap(args: Array<Any?>, line: Int): Any? = sized("u32", true, args, line)
    fun b_i8(args: Array<Any?>, line: Int): Any? = sized("i8", false, args, line)
    fun b_i8_wrap(args: Array<Any?>, line: Int): Any? = sized("i8", true, args, line)
    fun b_i16(args: Array<Any?>, line: Int): Any? = sized("i16", false, args, line)
    fun b_i16_wrap(args: Array<Any?>, line: Int): Any? = sized("i16", true, args, line)
    fun b_i32(args: Array<Any?>, line: Int): Any? = sized("i32", false, args, line)
    fun b_i32_wrap(args: Array<Any?>, line: Int): Any? = sized("i32", true, args, line)

    fun b_Error(args: Array<Any?>, line: Int): Any? = newError("Error", args, line)
    fun b_ValueError(args: Array<Any?>, line: Int): Any? = newError("ValueError", args, line)
    fun b_IndexError(args: Array<Any?>, line: Int): Any? = newError("IndexError", args, line)
    fun b_KeyError(args: Array<Any?>, line: Int): Any? = newError("KeyError", args, line)
    fun b_ZeroDivisionError(args: Array<Any?>, line: Int): Any? = newError("ZeroDivisionError", args, line)
    fun b_OverflowError(args: Array<Any?>, line: Int): Any? = newError("OverflowError", args, line)
    fun b_FileError(args: Array<Any?>, line: Int): Any? = newError("FileError", args, line)

    val ERROR_KINDS = listOf("Error", "ValueError", "IndexError", "KeyError", "ZeroDivisionError", "OverflowError", "FileError")

    val BUILTINS = HashMap<String, (Array<Any?>, Int) -> Any?>()

    init {
        BUILTINS["print"] = ::b_print
        BUILTINS["input"] = ::b_input
        BUILTINS["len"] = ::b_len
        BUILTINS["range"] = ::b_range
        BUILTINS["int"] = ::b_int
        BUILTINS["float"] = ::b_float
        BUILTINS["f32"] = ::b_f32
        BUILTINS["u64"] = ::b_u64
        BUILTINS["u64..of"] = ::b_u64__of
        BUILTINS["u64..add"] = ::b_u64__add
        BUILTINS["u64..sub"] = ::b_u64__sub
        BUILTINS["u64..mul"] = ::b_u64__mul
        BUILTINS["u64..shl"] = ::b_u64__shl
        BUILTINS["str"] = ::b_str
        BUILTINS["abs"] = ::b_abs
        BUILTINS["min"] = ::b_min
        BUILTINS["max"] = ::b_max
        BUILTINS["sum"] = ::b_sum
        BUILTINS["char"] = ::b_char
        BUILTINS["allocate"] = ::b_allocate
        BUILTINS["free"] = ::b_free
        BUILTINS["math.sqrt"] = ::b_math_sqrt
        BUILTINS["math.floor"] = ::b_math_floor
        BUILTINS["math.ceil"] = ::b_math_ceil
        BUILTINS["math.round"] = ::b_math_round
        BUILTINS["math.sin"] = ::b_math_sin
        BUILTINS["math.cos"] = ::b_math_cos
        BUILTINS["math.tan"] = ::b_math_tan
        BUILTINS["math.asin"] = ::b_math_asin
        BUILTINS["math.acos"] = ::b_math_acos
        BUILTINS["math.atan"] = ::b_math_atan
        BUILTINS["math.atan2"] = ::b_math_atan2
        BUILTINS["math.exp"] = ::b_math_exp
        BUILTINS["math.log"] = ::b_math_log
        BUILTINS["math.log2"] = ::b_math_log2
        BUILTINS["math.log10"] = ::b_math_log10
        BUILTINS["time.sleep"] = ::b_time_sleep
        BUILTINS["time.now"] = ::b_time_now
        BUILTINS["files.read"] = ::b_files_read
        BUILTINS["files.write"] = ::b_files_write
        BUILTINS["files.read_bytes"] = ::b_files_read_bytes
        BUILTINS["files.write_bytes"] = ::b_files_write_bytes
        BUILTINS["files.append"] = ::b_files_append
        BUILTINS["files.exists"] = ::b_files_exists
        BUILTINS["files.remove"] = ::b_files_remove
        BUILTINS["files.list"] = ::b_files_list
        BUILTINS["files.make_folder"] = ::b_files_make_folder
        BUILTINS["program.arguments"] = ::b_program_arguments
        BUILTINS["program.exit"] = ::b_program_exit
        BUILTINS["program.environment"] = ::b_program_environment
        BUILTINS["program.warn"] = ::b_program_warn
        BUILTINS["json.parse"] = ::b_json_parse
        BUILTINS["json.write"] = ::b_json_write
        for (name in WIDTHS.keys) {
            BUILTINS[name] = { args, line -> sized(name, false, args, line) }
            BUILTINS[name + ".wrap"] = { args, line -> sized(name, true, args, line) }
        }
        for (kind in ERROR_KINDS) BUILTINS[kind] = { args, line -> newError(kind, args, line) }
    }

    fun call_builtin(name: String, args: Array<Any?>, line: Int): Any? {
        val f = BUILTINS[name] ?: fatal("there is no `" + name + "` in a program built for Kotlin", line)
        return f(args, line)
    }

    val BUILTIN_VALUES = HashMap<String, Any?>()

    // Something that comes with Zeno, as a value: `print`, `math.pi`.
    fun builtin(name: String, line: Int): Any? {
        if (name == "math.pi") return Math.PI
        if (name == "math.e") return Math.E
        if (ERROR_KINDS.contains(name)) return ZErrorType(name)
        val known = BUILTIN_VALUES[name]
        if (known != null) return known
        val body = BUILTINS[name]
        val dot = name.indexOf('.')
        if (body == null) {
            if (dot >= 0) fatal("module `" + name.substring(0, dot) + "` has no `" + name.substring(dot + 1) + "`", line)
            fatal("`" + name + "` is not defined", line)
        }
        val v = ZFunc(if (dot >= 0) name.substring(dot + 1) else name, NO_NAMES, BooleanArray(0), null, name, body)
        BUILTIN_VALUES[name] = v
        return v
    }

    fun module(name: String): Any? = ZModule(name)

    // --------------------------------------------------------- The program

    fun where(files: Array<String>, line: Int): String {
        val index = line / LINES_A_FILE
        val file = if (index < files.size) files[index] else files[0]
        return " --> " + file + ":" + (line % LINES_A_FILE)
    }

    // Run the program, and say what stopped it if something did. It runs
    // in a thread of its own, which has room for 1000 calls inside one
    // another.
    fun start(files: Array<String>, args: Array<String>, main: () -> Any?) {
        arguments = args
        var status = 0
        val thread = Thread(null, {
            try {
                main()
                flush()
            } catch (e: ZRaise) {
                flush()
                System.err.print("error: uncaught " + shownName(e.value.kind) + ": " + show(e.value) + "\n" + where(files, e.line) + "\n")
                status = 1
            } catch (e: ZFatal) {
                flush()
                System.err.print("error: " + e.text + "\n" + where(files, e.line) + "\n")
                status = 1
            }
        }, "zeno", 512L * 1024 * 1024)
        thread.start()
        try {
            thread.join()
        } catch (e: InterruptedException) {
            status = 1
        }
        System.err.flush()
        System.exit(status)
    }
}
