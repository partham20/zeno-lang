// The Zeno runtime for PHP.
//
// A program built with `zeno build --target php` is its own code after
// this: what every Zeno value is, and what each operator and built-in
// does with it. It does what ops.rs and builtins.rs do for `zeno run`,
// step for step and message for message, so that the program prints
// what `zeno run` prints. It needs PHP 8.0 or newer and no extension.
//
// How a Zeno value is held:
//
//   int        a PHP int: 64 bits, as in Zeno. PHP turns a sum that
//              does not fit into a float, which is how a step sees it
//   float      a PHP float
//   f32        a `ZF32`: the float it is exactly (decision 41)
//   u64        a `ZU64`: the PHP int with the same 64 bits (decision 42)
//   str        a PHP string, in UTF-8
//   bool       true, false
//   none       null
//   list       a `ZList`: a PHP array in an object, because a PHP array
//              is copied when it is handed on and a Zeno list is shared
//   dict       a `ZDict`, for the same reason. PHP makes the key "1" the
//              int 1, so a str key is kept with an `s` before it
//   quantity   a `ZQuantity`: a number and its unit
//   function   a `ZFn`: a closure, with its Zeno name and parameters
//   object     a `ZInstance` of a `ZClass`
//   enum case  a `ZCase` of a `ZEnum`
//   error      a `ZError`
//
// The steps the printed program calls are the static functions of `Z`.

final class ZList { public $v; function __construct(array $v) { $this->v = $v; } }
final class ZDict { public $map = []; }
final class ZF32 { public $v; function __construct(float $v) { $this->v = $v; } }
final class ZU64 { public $v; function __construct(int $v) { $this->v = $v; } }
final class ZQuantity { public $v; public $unit; function __construct(float $v, string $unit) { $this->v = $v; $this->unit = $unit; } }
final class ZFn {
    public $name; public $params; public $defaults; public $fn; public $builtin;
    function __construct(string $name, array $params, array $defaults, ?Closure $fn, ?string $builtin = null) {
        $this->name = $name; $this->params = $params; $this->defaults = $defaults; $this->fn = $fn; $this->builtin = $builtin;
    }
}
final class ZClass {
    public $name; public $fields; public $defaults; public $methods;
    function __construct(string $name, array $fields, array $defaults, array $methods) {
        $this->name = $name; $this->fields = $fields; $this->defaults = $defaults; $this->methods = $methods;
    }
}
final class ZInstance {
    public $cls; public $fields;
    function __construct(ZClass $cls, array $fields) { $this->cls = $cls; $this->fields = $fields; }
    function __destruct() { Z::letGo($this->fields); }
}
final class ZEnum { public $name; public $cases; function __construct(string $name, array $cases) { $this->name = $name; $this->cases = $cases; } }
final class ZCase {
    public $def; public $index; public $fields;
    function __construct(ZEnum $def, int $index, array $fields) { $this->def = $def; $this->index = $index; $this->fields = $fields; }
    function __destruct() { Z::letGo($this->fields); }
}
final class ZConstructor { public $def; public $index; function __construct(ZEnum $def, int $index) { $this->def = $def; $this->index = $index; } }
final class ZErrorType { public $kind; function __construct(string $kind) { $this->kind = $kind; } }
final class ZError { public $kind; public $message; public $object; function __construct(string $kind, string $message, $object) { $this->kind = $kind; $this->message = $message; $this->object = $object; } }
final class ZModule { public $name; function __construct(string $name) { $this->name = $name; } }
// A method of a str, a list or a dict, read but not yet called: "abc".upper
final class ZMethod { public $receiver; public $name; function __construct($receiver, string $name) { $this->receiver = $receiver; $this->name = $name; } }
// A method of a class, read from an object: p.distance_to
final class ZBound { public $object; public $fn; function __construct(ZInstance $object, ZFn $fn) { $this->object = $object; $this->fn = $fn; } }
// What a call gives a parameter it leaves out.
final class ZMissing {}

// An error the program may catch, on its way to a `try`.
final class ZRaise extends Exception { public $value; public $at; function __construct(ZError $value, int $at) { parent::__construct($value->message); $this->value = $value; $this->at = $at; } }
// A mistake in the program, which nothing catches.
final class ZFatal extends Exception { public $text; public $at; function __construct(string $text, int $at) { parent::__construct($text); $this->text = $text; $this->at = $at; } }
// Text that is not JSON, on its way out of the reader.
final class ZNotJson extends Exception {}

final class Z {
    public static $MISSING;
    const LINES_A_FILE = 1000000;

    static function fatal(string $message, int $line) { throw new ZFatal($message, $line); }
    static function raise_(string $kind, string $message, int $line) { throw new ZRaise(new ZError($kind, $message, null), $line); }
    static function overflow(int $line) { self::raise_("OverflowError", "the result is too big for an int", $line); }
    static function byZero(int $line) { self::raise_("ZeroDivisionError", "division by zero", $line); }

    // The number inside an int or a float, as a float.
    static function number($v): ?float {
        if (is_float($v)) return $v;
        if (is_int($v)) return (float)$v;
        if ($v instanceof ZF32) return $v->v;
        if ($v instanceof ZU64) return self::uFloat($v->v);
        return null;
    }
    // ---- u64 (decision 42). A PHP int has 64 bits and a sign, and a sum
    // past its largest becomes a float, so a u64 is the int with the same
    // bits and its arithmetic is done in pieces that cannot spill over.
    const U64_TOO_BIG = "the result is too big for a u64";
    // A u64, or an int of 0 or more, as its bits; else null.
    static function unsigned($v): ?int {
        if ($v instanceof ZU64) return $v->v;
        if (is_int($v) && $v >= 0) return $v;
        return null;
    }
    // `a < b` with no sign: the top bit turned over makes the signed
    // order the unsigned one.
    static function uLess(int $a, int $b): bool { return ($a ^ PHP_INT_MIN) < ($b ^ PHP_INT_MIN); }
    // The bits moved down, with zeros coming in at the top.
    static function uDown(int $a, int $n): int { return $n === 0 ? $a : (($a >> $n) & (PHP_INT_MAX >> ($n - 1))); }
    // `a + b` cut to 64 bits, and whether it was cut.
    static function uAdd(int $a, int $b): array {
        $lo = ($a & 0xFFFFFFFF) + ($b & 0xFFFFFFFF);
        $hi = (($a >> 32) & 0xFFFFFFFF) + (($b >> 32) & 0xFFFFFFFF) + ($lo >> 32);
        return [(($hi & 0xFFFFFFFF) << 32) | ($lo & 0xFFFFFFFF), ($hi >> 32) !== 0];
    }
    // `a * b` cut to 64 bits, and whether it was cut: four pieces of 16
    // bits each side, as on paper.
    static function uMul(int $a, int $b): array {
        $x = []; $y = [];
        for ($i = 0; $i < 4; $i++) { $x[] = ($a >> (16 * $i)) & 0xFFFF; $y[] = ($b >> (16 * $i)) & 0xFFFF; }
        $out = array_fill(0, 8, 0);
        for ($i = 0; $i < 4; $i++) {
            $carry = 0;
            for ($j = 0; $j < 4; $j++) {
                $t = $out[$i + $j] + $x[$i] * $y[$j] + $carry;
                $out[$i + $j] = $t & 0xFFFF;
                $carry = $t >> 16;
            }
            $out[$i + 4] += $carry;
        }
        $r = $out[0] | ($out[1] << 16) | ($out[2] << 32) | ($out[3] << 48);
        return [$r, ($out[4] | $out[5] | $out[6] | $out[7]) !== 0];
    }
    // `a // b` and `a % b` with no sign; `b` is not 0.
    static function uDivMod(int $a, int $b): array {
        if ($b < 0) return self::uLess($a, $b) ? [0, $a] : [1, self::uAdd($a, self::uAdd(~$b, 1)[0])[0]];
        if ($a >= 0) return [intdiv($a, $b), $a % $b];
        $q = intdiv(self::uDown($a, 1), $b) << 1;
        $r = self::uAdd($a, self::uAdd(~self::uMul($q, $b)[0], 1)[0])[0];
        if (!self::uLess($r, $b)) { $q += 1; $r = self::uAdd($r, self::uAdd(~$b, 1)[0])[0]; }
        return [$q, $r];
    }
    static function uText(int $a): string {
        if ($a >= 0) return (string)$a;
        [$q, $r] = self::uDivMod($a, 10);
        return $q . $r;
    }
    // The nearest float, as Rust's `as f64` makes it.
    static function uFloat(int $a): float {
        if ($a >= 0) return (float)$a;
        return (float)(self::uDown($a, 1) | ($a & 1)) * 2.0;
    }
    static function u64ShiftCount($n, int $line): int {
        if ($n instanceof ZU64) {
            if ($n->v < 0 || $n->v >= 64) self::raise_("ValueError", "cannot shift by " . self::uText($n->v) . ". A u64 has 64 bits, so a shift is 0 to 63", $line);
            return $n->v;
        }
        if ($n < 0 || $n >= 64) self::raise_("ValueError", "cannot shift by " . $n . ". A u64 has 64 bits, so a shift is 0 to 63", $line);
        return $n;
    }
    // `a op b` where one side is a u64 (decision 42).
    static function u64Op(string $op, $a, $b, int $line) {
        if (($op === "<<" || $op === ">>") && $a instanceof ZU64 && (is_int($b) || $b instanceof ZU64)) {
            $n = self::u64ShiftCount($b, $line);
            if ($op === ">>") return new ZU64(self::uDown($a->v, $n));
            $r = $a->v << $n;
            if (self::uDown($r, $n) !== $a->v) self::raise_("OverflowError", self::U64_TOO_BIG, $line);
            return new ZU64($r);
        }
        $x = self::unsigned($a); $y = self::unsigned($b);
        if ($x === null || $y === null) self::fatal("`" . $op . "` between " . self::typeName($a) . " and " . self::typeName($b) . " would have to choose between a u64 and something else. Write `u64( )` or `int( )` around one of them", $line);
        switch ($op) {
            case "+":
                [$r, $cut] = self::uAdd($x, $y);
                if ($cut) self::raise_("OverflowError", self::U64_TOO_BIG, $line);
                return new ZU64($r);
            case "-":
                if (self::uLess($x, $y)) self::raise_("OverflowError", "the result is below 0, and a u64 never is", $line);
                return new ZU64(self::uAdd($x, self::uAdd(~$y, 1)[0])[0]);
            case "*":
                [$r, $cut] = self::uMul($x, $y);
                if ($cut) self::raise_("OverflowError", self::U64_TOO_BIG, $line);
                return new ZU64($r);
            case "/":
                if ($y === 0) self::byZero($line);
                return self::uFloat($x) / self::uFloat($y);
            case "//":
            case "%":
                if ($y === 0) self::byZero($line);
                return new ZU64(self::uDivMod($x, $y)[$op === "//" ? 0 : 1]);
            case "**":
                if ($y === 0) return new ZU64(1);
                if ($x === 0 || $x === 1) return new ZU64($x);
                if ($y < 0 || $y > 64) self::raise_("OverflowError", self::U64_TOO_BIG, $line);
                $r = 1;
                for ($i = 0; $i < $y; $i++) {
                    [$r, $cut] = self::uMul($r, $x);
                    if ($cut) self::raise_("OverflowError", self::U64_TOO_BIG, $line);
                }
                return new ZU64($r);
            case "&": return new ZU64($x & $y);
            case "|": return new ZU64($x | $y);
            default: return new ZU64($x ^ $y);
        }
    }
    // `u64.wrap(a + b)`: the answer cut to 64 bits (42.6).
    static function u64Wrapping(string $op, array $args, int $line): ZU64 {
        self::arity("u64.wrap", $args, 2, 2, $line);
        [$a, $b] = $args;
        if ($op === "<<" && self::unsigned($a) !== null && (is_int($b) || $b instanceof ZU64)) return new ZU64(self::unsigned($a) << self::u64ShiftCount($b, $line));
        $x = self::unsigned($a); $y = self::unsigned($b);
        if ($x === null || $y === null) self::wrongType("u64.wrap", "u64s", $x === null ? $a : $b, $line);
        if ($op === "+") return new ZU64(self::uAdd($x, $y)[0]);
        if ($op === "-") return new ZU64(self::uAdd($x, self::uAdd(~$y, 1)[0])[0]);
        return new ZU64(self::uMul($x, $y)[0]);
    }
    static function b_u64(array $args, int $line) {
        self::arity("u64", $args, 1, 1, $line);
        $v = $args[0];
        if ($v instanceof ZU64) return $v;
        if (is_int($v)) {
            if ($v < 0) self::raise_("OverflowError", $v . " does not fit in a u64 (0 to 18446744073709551615)", $line);
            return new ZU64($v);
        }
        self::wrongType("u64", "a whole number", $v, $line);
    }
    // `u64.wrap(n)`: the 64 bits of an int read with no sign (42.6).
    static function b_u64__of(array $args, int $line) {
        self::arity("u64.wrap", $args, 1, 1, $line);
        $v = $args[0];
        if ($v instanceof ZU64) return $v;
        if (is_int($v)) return new ZU64($v);
        self::wrongType("u64.wrap", "a whole number", $v, $line);
    }
    static function b_u64__add(array $args, int $line) { return self::u64Wrapping("+", $args, $line); }
    static function b_u64__sub(array $args, int $line) { return self::u64Wrapping("-", $args, $line); }
    static function b_u64__mul(array $args, int $line) { return self::u64Wrapping("*", $args, $line); }
    static function b_u64__shl(array $args, int $line) { return self::u64Wrapping("<<", $args, $line); }

    // The nearest f32 to a float, as a float.
    static function fround(float $x): float { return unpack("g", pack("g", $x))[1]; }
    // An f32, or an int as a float and then the nearest f32 (41.2).
    static function single($v): ?float {
        if ($v instanceof ZF32) return $v->v;
        if (is_int($v)) return self::fround((float)$v);
        return null;
    }
    // The float with the fewest digits that reads back as this f32.
    static function f32Shown(float $x): float {
        if (is_nan($x) || $x == 0 || !is_finite($x)) return $x;
        for ($digits = 1; $digits <= 9; $digits++) {
            $d = (float)sprintf("%." . ($digits - 1) . "e", $x);
            if (self::fround($d) === $x) return $d;
        }
        return $x;
    }
    static function f32Op(string $op, $a, $b, int $line) {
        $x = self::single($a); $y = self::single($b);
        if ($x === null || $y === null) {
            if (is_float($a) || is_float($b)) self::fatal("`" . $op . "` between " . self::typeName($a) . " and " . self::typeName($b) . " would have to choose between 32 bits and 64. Write `f32( )` around the float to work in 32, or `float( )` around the f32 to work in 64", $line);
            self::mismatch($op, $a, $b, $line);
        }
        if (($op === "/" || $op === "//" || $op === "%") && $y == 0) self::byZero($line);
        switch ($op) {
            case "+": $r = $x + $y; break;
            case "-": $r = $x - $y; break;
            case "*": $r = $x * $y; break;
            case "/": $r = $x / $y; break;
            case "//": $r = floor($x / $y); break;
            case "%": $r = $x - $y * floor($x / $y); break;
            default: $r = self::pow($x, $y, $line);
        }
        return new ZF32(self::fround($r));
    }
    static function float($x) { return $x; }

    static function typeName($v): string {
        if (is_int($v)) return "an int";
        if (is_float($v)) return "a float";
        if ($v instanceof ZF32) return "an f32";
        if ($v instanceof ZU64) return "a u64";
        if (is_string($v)) return "a str";
        if (is_bool($v)) return "a bool";
        if ($v === null) return "none";
        if ($v instanceof ZQuantity) return "a quantity in " . $v->unit;
        if ($v instanceof ZList) return "a list";
        if ($v instanceof ZDict) return "a dict";
        if ($v instanceof ZFn || $v instanceof ZMethod || $v instanceof ZBound) return "a function";
        if ($v instanceof ZClass) return "a class";
        if ($v instanceof ZInstance) return "a " . $v->cls->name;
        if ($v instanceof ZEnum) return "an enum";
        if ($v instanceof ZConstructor) return "an enum case";
        if ($v instanceof ZCase) return "a " . $v->def->name;
        if ($v instanceof ZErrorType) return "an error type";
        if ($v instanceof ZError) return "a " . $v->kind;
        if ($v instanceof ZModule) return "a module";
        return "a value";
    }

    // `Point`, not `shapes.Point`; `Stack`, not `Stack[int]`.
    static function shownName(string $name): string {
        $name = explode("[", $name)[0];
        $dot = strrpos($name, ".");
        return $dot === false ? $name : substr($name, $dot + 1);
    }

    // When the last holder of an object lets go, PHP lets go of what the
    // object held, and so on down: for a chain of 200,000 objects that is
    // 200,000 steps inside each other, more than PHP has room for. So an
    // object hands what it held to this list, and one loop lets go of it
    // all, however long the chain.
    private static $held = [];
    private static $lettingGo = false;
    static function letGo(array &$values) {
        self::$held[] = $values;
        $values = [];
        if (self::$lettingGo) return;
        self::$lettingGo = true;
        while (self::$held) array_pop(self::$held);
        self::$lettingGo = false;
    }

    // ------------------------------------------------------------- Showing

    static function isNegative(float $x): bool { return $x < 0 || ($x == 0 && (ord(pack("E", $x)) & 0x80) !== 0); }

    // A float as Rust's `{:?}` shows it: the shortest digits that read back
    // as the same number, with `.0` on a whole one, and an exponent for one
    // below 1e-4 or from 1e16 up.
    static function showFloat(float $x): string {
        if (is_nan($x)) return "NaN";
        if ($x === INF) return "inf";
        if ($x === -INF) return "-inf";
        if ($x == 0) return self::isNegative($x) ? "-0.0" : "0.0";
        // PHP writes the shortest digits too, in a form of its own:
        // 0.30000000000000004, 1.0E+25. Take the digits and the power of
        // ten of the first one out of it.
        $text = var_export($x, true);
        $sign = "";
        if ($text[0] === "-") { $sign = "-"; $text = substr($text, 1); }
        $exp = 0;
        $e = strpos($text, "E");
        if ($e !== false) { $exp = (int)substr($text, $e + 1); $text = substr($text, 0, $e); }
        $dot = strpos($text, ".");
        if ($dot === false) $dot = strlen($text); else $text = substr($text, 0, $dot) . substr($text, $dot + 1);
        $lead = strspn($text, "0");
        $digits = rtrim(substr($text, $lead), "0");
        $exp += $dot - $lead - 1;
        $count = strlen($digits);
        if ($exp < -4 || $exp >= 16) {
            return $sign . ($count > 1 ? $digits[0] . "." . substr($digits, 1) : $digits) . "e" . $exp;
        }
        if ($exp < 0) return $sign . "0." . str_repeat("0", -$exp - 1) . $digits;
        if ($count <= $exp + 1) return $sign . $digits . str_repeat("0", $exp + 1 - $count) . ".0";
        return $sign . substr($digits, 0, $exp + 1) . "." . substr($digits, $exp + 1);
    }

    // The digits of a whole number held as a list of pieces of nine
    // digits, the lowest first, times a small number.
    private static function times(array $pieces, int $by): array {
        $carry = 0;
        foreach ($pieces as $i => $piece) {
            $all = $piece * $by + $carry;
            $pieces[$i] = $all % 1000000000;
            $carry = intdiv($all, 1000000000);
        }
        while ($carry > 0) { $pieces[] = $carry % 1000000000; $carry = intdiv($carry, 1000000000); }
        return $pieces;
    }

    // A float with a fixed number of decimals, as Rust's `{:.2}` writes it:
    // from the exact value of the float, a half rounding to the even digit.
    static function fixed(float $x, int $places): string {
        if (is_nan($x)) return "NaN";
        if (is_infinite($x)) return $x > 0 ? "inf" : "-inf";
        $negative = self::isNegative($x);
        $x = abs($x);
        // x = mantissa * 2**exponent, exactly.
        $bits = unpack("J", pack("E", $x))[1];
        $exponent = ($bits >> 52) & 0x7ff;
        $mantissa = $bits & 0xfffffffffffff;
        if ($exponent === 0) $exponent = 1; else $mantissa |= 1 << 52;
        $exponent -= 1075;
        // Every float has an exact decimal text: the mantissa times 2
        // to the exponent, or, below one, times 5 to it with the point
        // moved that many places.
        $pieces = [$mantissa % 1000000000, intdiv($mantissa, 1000000000)];
        $after = 0;
        if ($exponent >= 0) {
            for ($n = $exponent; $n > 0; $n -= 20) $pieces = self::times($pieces, 1 << min($n, 20));
        } else {
            $after = -$exponent;
            for ($n = $after; $n > 0; $n -= 8) $pieces = self::times($pieces, 5 ** min($n, 8));
        }
        $digits = "";
        for ($i = count($pieces) - 1; $i >= 0; $i--) $digits .= str_pad((string)$pieces[$i], 9, "0", STR_PAD_LEFT);
        $digits = ltrim($digits, "0");
        if (strlen($digits) <= $after) $digits = str_repeat("0", $after - strlen($digits) + 1) . $digits;
        $whole = substr($digits, 0, strlen($digits) - $after);
        $part = substr($digits, strlen($digits) - $after);
        if (strlen($part) <= $places) {
            $part = str_pad($part, $places, "0");
        } else {
            $rest = substr($part, $places);
            $part = substr($part, 0, $places);
            $half = "5" . str_repeat("0", strlen($rest) - 1);
            $last = $part === "" ? $whole[strlen($whole) - 1] : $part[strlen($part) - 1];
            $order = strcmp($rest, $half);
            if ($order > 0 || ($order === 0 && ((int)$last) % 2 === 1)) {
                // One more in the last place kept.
                $all = $whole . $part;
                $i = strlen($all) - 1;
                while ($i >= 0 && $all[$i] === "9") { $all[$i] = "0"; $i--; }
                if ($i >= 0) $all[$i] = (string)((int)$all[$i] + 1); else $all = "1" . $all;
                $whole = substr($all, 0, strlen($all) - $places);
                $part = $places > 0 ? substr($all, strlen($all) - $places) : "";
            }
        }
        return ($negative ? "-" : "") . $whole . ($places > 0 ? "." . $part : "");
    }

    // A str in quotes, with what cannot be seen escaped.
    static function quoted(string $s): string {
        return '"' . preg_replace_callback('/["\\\\\x00-\x1f\x7f]|\xc2[\x80-\x9f]/', static function ($m) {
            switch ($m[0]) {
                case '"': return '\\"';
                case "\\": return "\\\\";
                case "\n": return "\\n";
                case "\r": return "\\r";
                case "\t": return "\\t";
                case "\0": return "\\0";
            }
            return "\\u{" . dechex(ord($m[0][strlen($m[0]) - 1])) . "}";
        }, $s) . '"';
    }

    // How `print`, `str()` and f-strings show a value.
    static function show($v): string {
        if (is_string($v)) return $v;
        if (is_int($v)) return (string)$v;
        if (is_float($v)) return self::showFloat($v);
        if ($v instanceof ZF32) return self::showFloat(self::f32Shown($v->v));
        if ($v instanceof ZU64) return self::uText($v->v);
        if (is_bool($v)) return $v ? "true" : "false";
        if ($v === null) return "none";
        if ($v instanceof ZQuantity) {
            if (is_finite($v->v) && floor($v->v) === $v->v && abs($v->v) < 1e15) return (string)(int)$v->v . " " . $v->unit;
            return self::showFloat($v->v) . " " . $v->unit;
        }
        if ($v instanceof ZList) {
            $parts = [];
            foreach ($v->v as $item) $parts[] = self::repr($item);
            return "[" . implode(", ", $parts) . "]";
        }
        if ($v instanceof ZDict) {
            if (count($v->map) === 0) return "[:]";
            $parts = [];
            foreach ($v->map as $key => $value) $parts[] = self::repr(is_int($key) ? $key : substr($key, 1)) . ": " . self::repr($value);
            return "[" . implode(", ", $parts) . "]";
        }
        if ($v instanceof ZFn) return $v->name !== "" ? "<function " . $v->name . ">" : "<function>";
        if ($v instanceof ZMethod) return "<function " . $v->name . ">";
        if ($v instanceof ZBound) return "<function " . $v->fn->name . ">";
        if ($v instanceof ZClass) return "<class " . $v->name . ">";
        if ($v instanceof ZInstance) {
            $parts = [];
            foreach ($v->cls->fields as $i => $name) $parts[] = $name . "=" . self::repr($v->fields[$i]);
            return self::shownName($v->cls->name) . "(" . implode(", ", $parts) . ")";
        }
        if ($v instanceof ZEnum) return "<enum " . $v->name . ">";
        if ($v instanceof ZConstructor) return "<case " . $v->def->cases[$v->index][0] . ">";
        if ($v instanceof ZCase) {
            $name = self::shownName($v->def->cases[$v->index][0]);
            if (count($v->fields) === 0) return $name;
            $parts = [];
            foreach ($v->fields as $item) $parts[] = self::repr($item);
            return $name . "(" . implode(", ", $parts) . ")";
        }
        if ($v instanceof ZErrorType) return "<error " . $v->kind . ">";
        if ($v instanceof ZError) return $v->message;
        if ($v instanceof ZModule) return "<module " . $v->name . ">";
        return "<value>";
    }

    static function repr($v): string { return is_string($v) ? self::quoted($v) : self::show($v); }

    // --------------------------------------------------------------- Units

    const UNITS = [
        "ns" => ["time", 1.0], "us" => ["time", 1e3], "ms" => ["time", 1e6], "s" => ["time", 1e9], "min" => ["time", 60e9], "h" => ["time", 3600e9],
        "Hz" => ["frequency", 1.0], "kHz" => ["frequency", 1e3], "MHz" => ["frequency", 1e6],
        "uV" => ["voltage", 1.0], "mV" => ["voltage", 1e3], "V" => ["voltage", 1e6], "kV" => ["voltage", 1e9],
        "uA" => ["current", 1.0], "mA" => ["current", 1e3], "A" => ["current", 1e6],
        "ohm" => ["resistance", 1.0], "kohm" => ["resistance", 1e3], "Mohm" => ["resistance", 1e6],
        "mW" => ["power", 1.0], "W" => ["power", 1e3], "kW" => ["power", 1e6],
        "J" => ["energy", 1.0], "Wh" => ["energy", 3600.0], "kWh" => ["energy", 3600e3],
        "degC" => ["temperature in Celsius", 1.0], "K" => ["temperature in kelvin", 1.0],
        "mm" => ["length", 1.0], "cm" => ["length", 10.0], "m" => ["length", 1e3], "km" => ["length", 1e6],
        "mg" => ["mass", 1.0], "g" => ["mass", 1e3], "kg" => ["mass", 1e6],
        "deg" => ["angle in degrees", 1.0], "rad" => ["angle in radians", 1.0], "rpm" => ["rotation", 1.0],
    ];
    static function isUnit(string $name): bool { return isset(self::UNITS[$name]); }

    // How to turn a number in one unit into another: [times, over], or null.
    static function convert(string $from, string $to): ?array {
        if ($from === "deg" && $to === "rad") return [M_PI / 180, 1.0];
        if ($from === "rad" && $to === "deg") return [180 / M_PI, 1.0];
        if (!self::isUnit($from) || !self::isUnit($to)) return null;
        [$fromKind, $fromSize] = self::UNITS[$from];
        [$toKind, $toSize] = self::UNITS[$to];
        if ($fromKind !== $toKind) return null;
        return $fromSize >= $toSize ? [$fromSize / $toSize, 1.0] : [1.0, $toSize / $fromSize];
    }
    static function apply(array $scale, float $x): float { return ($x * $scale[0]) / $scale[1]; }

    // The unit two quantities meet in: the finer one. [unit, scaleA, scaleB] or null.
    static function common(string $a, string $b): ?array {
        if (!self::isUnit($a) || !self::isUnit($b)) return null;
        if (self::UNITS[$a][0] !== self::UNITS[$b][0]) return null;
        $unit = self::UNITS[$a][1] <= self::UNITS[$b][1] ? $a : $b;
        return [$unit, self::convert($a, $unit), self::convert($b, $unit)];
    }

    // What `*` and `/` make of two quantities (decision 43): the operator,
    // what each side measures ("" for a plain number), and what the answer
    // measures ("" for a plain number). The answer is in the unit of its
    // kind with no prefix.
    const MADE = [
        "* voltage current" => "power", "* current voltage" => "power",
        "* current resistance" => "voltage", "* resistance current" => "voltage",
        "* power time" => "energy", "* time power" => "energy",
        "* time frequency" => "", "* frequency time" => "",
        "/ power voltage" => "current", "/ power current" => "voltage",
        "/ voltage current" => "resistance", "/ voltage resistance" => "current",
        "/ energy time" => "power", "/ energy power" => "time",
        "/  time" => "frequency", "/  frequency" => "time",
    ];

    const PLAIN = ["time" => "s", "frequency" => "Hz", "voltage" => "V", "current" => "A", "resistance" => "ohm", "power" => "W", "energy" => "J"];

    // `a op b` for two quantities, or a number over a quantity.
    static function made(string $op, $a, $b, int $line) {
        $left = $a instanceof ZQuantity ? self::UNITS[$a->unit][0] : "";
        $right = self::UNITS[$b->unit][0];
        $key = $op . " " . $left . " " . $right;
        $x = $left === "" ? self::number($a) : self::apply(self::convert($a->unit, self::PLAIN[$left]), $a->v);
        if (!array_key_exists($key, self::MADE) || $x === null) self::mismatch($op, $a, $b, $line);
        if ($op === "/" && $b->v == 0) self::byZero($line);
        $y = self::apply(self::convert($b->unit, self::PLAIN[$right]), $b->v);
        $r = $op === "*" ? $x * $y : $x / $y;
        $kind = self::MADE[$key];
        return $kind === "" ? (float)$r : new ZQuantity($r, self::PLAIN[$kind]);
    }

    static function unit($v, string $name, int $line) {
        if (!self::isUnit($name)) self::fatal("`" . $name . "` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC", $line);
        $x = self::number($v);
        if ($x === null) self::fatal("a unit can only follow a number", $line);
        return new ZQuantity($x, $name);
    }

    // ------------------------------------------------------------ Equality

    static function eq($a, $b): bool {
        if ($a instanceof ZU64 || $b instanceof ZU64) {
            $x = self::unsigned($a);
            return $x !== null && $x === self::unsigned($b);
        }
        if ($a instanceof ZF32 || $b instanceof ZF32) {
            $x = self::single($a); $y = self::single($b);
            if ($x !== null && $y !== null) return $x === $y;
            $x = self::number($a); $y = self::number($b);
            return $x !== null && $y !== null && $x === $y;
        }
        if (is_int($a)) return is_int($b) ? $a === $b : (is_float($b) && (float)$a === $b);
        if (is_float($a)) return is_float($b) ? $a === $b : (is_int($b) && $a === (float)$b);
        if (!is_object($a) || !is_object($b)) return $a === $b;
        if ($a === $b) return true;
        if ($a instanceof ZQuantity && $b instanceof ZQuantity) {
            $c = self::common($a->unit, $b->unit);
            return $c !== null && self::apply($c[1], $a->v) === self::apply($c[2], $b->v);
        }
        if ($a instanceof ZList && $b instanceof ZList) {
            if (count($a->v) !== count($b->v)) return false;
            foreach ($a->v as $i => $item) if (!self::eq($item, $b->v[$i])) return false;
            return true;
        }
        if ($a instanceof ZDict && $b instanceof ZDict) {
            if (count($a->map) !== count($b->map)) return false;
            foreach ($a->map as $key => $value) {
                if (!array_key_exists($key, $b->map) || !self::eq($value, $b->map[$key])) return false;
            }
            return true;
        }
        if ($a instanceof ZInstance && $b instanceof ZInstance) {
            if ($a->cls !== $b->cls) return false;
            foreach ($a->fields as $i => $item) if (!self::eq($item, $b->fields[$i])) return false;
            return true;
        }
        if ($a instanceof ZCase && $b instanceof ZCase) {
            if ($a->def !== $b->def || $a->index !== $b->index) return false;
            foreach ($a->fields as $i => $item) if (!self::eq($item, $b->fields[$i])) return false;
            return true;
        }
        if ($a instanceof ZError && $b instanceof ZError) return $a->kind === $b->kind && $a->message === $b->message;
        if ($a instanceof ZConstructor && $b instanceof ZConstructor) return $a->def === $b->def && $a->index === $b->index;
        return false;
    }
    static function ne($a, $b): bool { return !self::eq($a, $b); }

    // ----------------------------------------------------------- Operators

    static function mismatch(string $op, $a, $b, int $line) {
        self::fatal("`" . $op . "` does not work between " . self::typeName($a) . " and " . self::typeName($b), $line);
    }

    static function add($a, $b, int $line) {
        if ($a instanceof ZU64 || $b instanceof ZU64) return self::u64Op("+", $a, $b, $line);
        if ($a instanceof ZF32 || $b instanceof ZF32) return self::f32Op("+", $a, $b, $line);
        if (is_int($a) && is_int($b)) {
            // A sum too big for an int is a float in PHP.
            $r = $a + $b;
            if (!is_int($r)) self::overflow($line);
            return $r;
        }
        if (is_string($a) && is_string($b)) return $a . $b;
        if ($a instanceof ZList && $b instanceof ZList) return new ZList(array_merge($a->v, $b->v));
        if ($a instanceof ZQuantity && $b instanceof ZQuantity) {
            $c = self::common($a->unit, $b->unit);
            if ($c === null) self::mismatch("+", $a, $b, $line);
            return new ZQuantity(self::apply($c[1], $a->v) + self::apply($c[2], $b->v), $c[0]);
        }
        $x = self::number($a); $y = self::number($b);
        if ($x === null || $y === null) self::mismatch("+", $a, $b, $line);
        return $x + $y;
    }

    static function sub($a, $b, int $line) {
        if ($a instanceof ZU64 || $b instanceof ZU64) return self::u64Op("-", $a, $b, $line);
        if ($a instanceof ZF32 || $b instanceof ZF32) return self::f32Op("-", $a, $b, $line);
        if (is_int($a) && is_int($b)) {
            $r = $a - $b;
            if (!is_int($r)) self::overflow($line);
            return $r;
        }
        if ($a instanceof ZQuantity && $b instanceof ZQuantity) {
            $c = self::common($a->unit, $b->unit);
            if ($c === null) self::mismatch("-", $a, $b, $line);
            return new ZQuantity(self::apply($c[1], $a->v) - self::apply($c[2], $b->v), $c[0]);
        }
        $x = self::number($a); $y = self::number($b);
        if ($x === null || $y === null) self::mismatch("-", $a, $b, $line);
        return $x - $y;
    }

    static function mul($a, $b, int $line) {
        if ($a instanceof ZU64 || $b instanceof ZU64) return self::u64Op("*", $a, $b, $line);
        if ($a instanceof ZF32 || $b instanceof ZF32) return self::f32Op("*", $a, $b, $line);
        if ($a instanceof ZList && is_int($b)) {
            if ($b < 0) self::raise_("ValueError", "`*` repeats a list 0 or more times, and this is " . $b, $line);
            $items = [];
            for ($t = 0; $t < $b; $t++) foreach ($a->v as $item) $items[] = $item;
            return new ZList($items);
        }
        if (is_int($a) && is_int($b)) {
            $r = $a * $b;
            if (!is_int($r)) self::overflow($line);
            return $r;
        }
        if ($a instanceof ZQuantity && $b instanceof ZQuantity) return self::made("*", $a, $b, $line);
        if ($a instanceof ZQuantity || $b instanceof ZQuantity) {
            [$q, $other] = $a instanceof ZQuantity ? [$a, $b] : [$b, $a];
            $y = self::number($other);
            if ($y === null) self::mismatch("*", $a, $b, $line);
            return new ZQuantity($q->v * $y, $q->unit);
        }
        $x = self::number($a); $y = self::number($b);
        if ($x === null || $y === null) self::mismatch("*", $a, $b, $line);
        return $x * $y;
    }

    static function div($a, $b, int $line) {
        if ($a instanceof ZU64 || $b instanceof ZU64) return self::u64Op("/", $a, $b, $line);
        if ($a instanceof ZF32 || $b instanceof ZF32) return self::f32Op("/", $a, $b, $line);
        if ($a instanceof ZQuantity) {
            if ($b === 0 || $b === 0.0 || ($b instanceof ZQuantity && $b->v == 0)) self::byZero($line);
            if ($b instanceof ZQuantity) {
                $c = self::common($a->unit, $b->unit);
                if ($c === null) return self::made("/", $a, $b, $line);
                return self::apply($c[1], $a->v) / self::apply($c[2], $b->v);
            }
            $y = self::number($b);
            if ($y === null) self::mismatch("/", $a, $b, $line);
            return new ZQuantity($a->v / $y, $a->unit);
        }
        if ($b instanceof ZQuantity) return self::made("/", $a, $b, $line);
        $x = self::number($a); $y = self::number($b);
        if ($x === null || $y === null) self::mismatch("/", $a, $b, $line);
        if ($y == 0) self::byZero($line);
        return $x / $y;
    }

    static function floordiv($a, $b, int $line) {
        if ($a instanceof ZU64 || $b instanceof ZU64) return self::u64Op("//", $a, $b, $line);
        if ($a instanceof ZF32 || $b instanceof ZF32) return self::f32Op("//", $a, $b, $line);
        if (is_int($a) && is_int($b)) {
            if ($b === 0) self::byZero($line);
            if ($b === -1) {
                $r = -$a;
                if (!is_int($r)) self::overflow($line);
                return $r;
            }
            $q = intdiv($a, $b);
            if ($a % $b !== 0 && ($a < 0) !== ($b < 0)) $q -= 1;
            return $q;
        }
        $x = self::number($a); $y = self::number($b);
        if ($x === null || $y === null) self::mismatch("//", $a, $b, $line);
        if ($y == 0) self::byZero($line);
        return floor($x / $y);
    }

    static function mod($a, $b, int $line) {
        if ($a instanceof ZU64 || $b instanceof ZU64) return self::u64Op("%", $a, $b, $line);
        if ($a instanceof ZF32 || $b instanceof ZF32) return self::f32Op("%", $a, $b, $line);
        if (is_int($a) && is_int($b)) {
            if ($b === 0) self::byZero($line);
            if ($b === -1) return 0;
            $r = $a % $b;
            return $r !== 0 && ($r < 0) !== ($b < 0) ? $r + $b : $r;
        }
        $x = self::number($a); $y = self::number($b);
        if ($x === null || $y === null) self::mismatch("%", $a, $b, $line);
        if ($y == 0) self::byZero($line);
        return $x - $y * floor($x / $y);
    }

    static function pow($a, $b, int $line) {
        if ($a instanceof ZU64 || $b instanceof ZU64) return self::u64Op("**", $a, $b, $line);
        if ($a instanceof ZF32 || $b instanceof ZF32) return self::f32Op("**", $a, $b, $line);
        if (is_int($a) && is_int($b)) {
            if ($b < 0) self::raise_("ValueError", "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`", $line);
            // As Rust's `checked_pow` with a u32 power.
            if ($b > 4294967295) self::overflow($line);
            if ($a === 0) return $b === 0 ? 1 : 0;
            if ($a === 1) return 1;
            if ($a === -1) return $b % 2 === 0 ? 1 : -1;
            if ($b >= 64) self::overflow($line);
            $r = 1;
            for ($i = 0; $i < $b; $i++) {
                $r *= $a;
                if (!is_int($r)) self::overflow($line);
            }
            return $r;
        }
        $x = self::number($a); $y = self::number($b);
        if ($x === null || $y === null) self::mismatch("**", $a, $b, $line);
        if ($x === 1.0 || ($x === -1.0 && is_infinite($y))) return 1.0;
        // Zero to a power below zero is infinite, as the processor has it.
        // PHP is on its way to refusing it.
        if ($x == 0 && $y < 0) {
            $odd = is_finite($y) && floor($y) === $y && fmod($y, 2.0) != 0;
            return $odd && self::isNegative($x) ? -INF : INF;
        }
        return pow($x, $y);
    }

    static function bitsMismatch(string $op, $a, $b, int $line) {
        $hint = "";
        if (is_bool($a) && is_bool($b)) {
            if ($op === "&") $hint = ". For two bools Zeno spells this `and`";
            if ($op === "|") $hint = ". For two bools Zeno spells this `or`";
        }
        self::fatal("`" . $op . "` works on the bits of whole numbers, and these are " . self::typeName($a) . " and " . self::typeName($b) . $hint, $line);
    }

    static function bitand($a, $b, int $line) { if ($a instanceof ZU64 || $b instanceof ZU64) return self::u64Op("&", $a, $b, $line); if (!is_int($a) || !is_int($b)) self::bitsMismatch("&", $a, $b, $line); return $a & $b; }
    static function bitor($a, $b, int $line) { if ($a instanceof ZU64 || $b instanceof ZU64) return self::u64Op("|", $a, $b, $line); if (!is_int($a) || !is_int($b)) self::bitsMismatch("|", $a, $b, $line); return $a | $b; }
    static function bitxor($a, $b, int $line) { if ($a instanceof ZU64 || $b instanceof ZU64) return self::u64Op("^", $a, $b, $line); if (!is_int($a) || !is_int($b)) self::bitsMismatch("^", $a, $b, $line); return $a ^ $b; }
    static function shift(string $op, $a, $n, int $line) {
        if ($a instanceof ZU64 || $n instanceof ZU64) return self::u64Op($op, $a, $n, $line);
        if (!is_int($a) || !is_int($n)) self::bitsMismatch($op, $a, $n, $line);
        if ($n < 0 || $n >= 64) self::raise_("ValueError", "cannot shift by " . $n . ". An int has 64 bits, so a shift is 0 to 63", $line);
        if ($op === ">>") return $a >> $n;
        $shifted = $a << $n;
        if (($shifted >> $n) !== $a) self::overflow($line);
        return $shifted;
    }
    static function shl($a, $n, int $line) { return self::shift("<<", $a, $n, $line); }
    static function shr($a, $n, int $line) { return self::shift(">>", $a, $n, $line); }

    static function neg($v, int $line) {
        if (is_int($v)) {
            $r = -$v;
            if (!is_int($r)) self::overflow($line);
            return $r;
        }
        if (is_float($v)) return -$v;
        if ($v instanceof ZF32) return new ZF32(-$v->v);
        if ($v instanceof ZU64) self::fatal("`-` does not work on a u64, which is never below 0. Make it an int first with `int( )`", $line);
        if ($v instanceof ZQuantity) return new ZQuantity(-$v->v, $v->unit);
        self::fatal("`-` does not work on " . self::typeName($v), $line);
    }
    static function bitnot($v, int $line) {
        if ($v instanceof ZU64) return new ZU64(~$v->v);
        if (!is_int($v)) self::fatal("`~` works on the bits of a whole number, and this is " . self::typeName($v), $line);
        return ~$v;
    }

    // `<` and the others: -1, 0, 1, or null for a NaN.
    static function order(string $op, $a, $b, int $line): ?int {
        if ($a instanceof ZU64 || $b instanceof ZU64) {
            $x = self::unsigned($a); $y = self::unsigned($b);
            if ($x === null || $y === null) self::u64Op($op, $a, $b, $line);
            return $x === $y ? 0 : (self::uLess($x, $y) ? -1 : 1);
        }
        if ($a instanceof ZF32 || $b instanceof ZF32) {
            $x = self::single($a); $y = self::single($b);
            if ($x === null || $y === null) self::f32Op($op, $a, $b, $line);
            return $x < $y ? -1 : ($x > $y ? 1 : ($x === $y ? 0 : null));
        }
        if (is_int($a) && is_int($b)) return $a <=> $b;
        if (is_string($a) && is_string($b)) return strcmp($a, $b) <=> 0;
        if ($a instanceof ZQuantity && $b instanceof ZQuantity) {
            $c = self::common($a->unit, $b->unit);
            if ($c === null) self::mismatch($op, $a, $b, $line);
            $x = self::apply($c[1], $a->v); $y = self::apply($c[2], $b->v);
        } else {
            $x = self::number($a); $y = self::number($b);
            if ($x === null || $y === null) self::mismatch($op, $a, $b, $line);
        }
        return $x < $y ? -1 : ($x > $y ? 1 : ($x === $y ? 0 : null));
    }
    static function lt($a, $b, int $line): bool { $o = self::order("<", $a, $b, $line); return $o !== null && $o < 0; }
    static function le($a, $b, int $line): bool { $o = self::order("<=", $a, $b, $line); return $o !== null && $o <= 0; }
    static function gt($a, $b, int $line): bool { $o = self::order(">", $a, $b, $line); return $o !== null && $o > 0; }
    static function ge($a, $b, int $line): bool { $o = self::order(">=", $a, $b, $line); return $o !== null && $o >= 0; }

    static function contains(string $op, $item, $container, int $line): bool {
        if ($container instanceof ZList) {
            foreach ($container->v as $other) if (self::eq($item, $other)) return true;
            return false;
        }
        if ($container instanceof ZDict) {
            if (!is_int($item) && !is_string($item)) self::mismatch($op, $item, $container, $line);
            return array_key_exists(is_int($item) ? $item : "s" . $item, $container->map);
        }
        if (is_string($item) && is_string($container)) return $item === "" || strpos($container, $item) !== false;
        self::mismatch($op, $item, $container, $line);
    }
    static function in($a, $b, int $line): bool { return self::contains("in", $a, $b, $line); }
    static function not_in($a, $b, int $line): bool { return !self::contains("not in", $a, $b, $line); }

    // A condition must be a real bool.
    static function truth($v, int $line): bool {
        if ($v === true || $v === false) return $v;
        self::fatal("a condition must be true or false, but this is " . self::typeName($v), $line);
    }

    // ------------------------------------------------------ Lists and text

    // A str is UTF-8: a character is one to four bytes. Text that is all
    // ASCII, which most is, is taken a byte at a time.
    static function isAscii(string $s): bool { return preg_match('/[\x80-\xff]/', $s) === 0; }
    static function chars(string $s): array {
        if ($s === "") return [];
        return self::isAscii($s) ? str_split($s) : preg_split('//u', $s, -1, PREG_SPLIT_NO_EMPTY);
    }
    static function strLen(string $s): int { return strlen($s) - preg_match_all('/[\x80-\xbf]/', $s); }

    // The key of a dict as the PHP array under it has it.
    static function key($v, int $line) {
        if (is_int($v)) return $v;
        if (is_string($v)) return "s" . $v;
        self::fatal("a key of a dict is a str or an int, but this is " . self::typeName($v), $line);
    }

    static function position($index, int $length, string $what, int $line): int {
        if (!is_int($index)) self::fatal("an index must be an int, but this is " . self::typeName($index), $line);
        if ($index < 0 || $index >= $length) self::raise_("IndexError", "index " . $index . " is out of range for a " . $what . " of length " . $length, $line);
        return $index;
    }

    static function index($object, $i, int $line) {
        if ($object instanceof ZList) return $object->v[self::position($i, count($object->v), "list", $line)];
        if (is_string($object)) {
            if (self::isAscii($object)) return $object[self::position($i, strlen($object), "str", $line)];
            $cs = self::chars($object);
            return $cs[self::position($i, count($cs), "str", $line)];
        }
        if ($object instanceof ZDict) {
            $k = self::key($i, $line);
            if (!array_key_exists($k, $object->map)) self::raise_("KeyError", "this dict has no key " . self::repr($i), $line);
            return $object->map[$k];
        }
        self::fatal("cannot take an item from " . self::typeName($object), $line);
    }

    static function setIndex($object, $i, $value, int $line) {
        if ($object instanceof ZList) { $object->v[self::position($i, count($object->v), "list", $line)] = $value; return; }
        if ($object instanceof ZDict) { $object->map[self::key($i, $line)] = $value; return; }
        self::fatal("cannot set an item of " . self::typeName($object), $line);
    }

    static function keys(ZDict $d): array {
        $keys = [];
        foreach ($d->map as $key => $_) $keys[] = is_int($key) ? $key : substr($key, 1);
        return $keys;
    }

    // What a `for` loop steps through: a copy, so that the loop is not upset
    // by a change to what it is walking.
    static function items($v, int $line): array {
        if ($v instanceof ZList) return $v->v;
        if ($v instanceof ZDict) return self::keys($v);
        if (is_string($v)) return self::chars($v);
        self::fatal("cannot loop over " . self::typeName($v), $line);
    }
    // The same for a loop with two names: [key or position, value] each round.
    static function pairs($v, int $line): array {
        $pairs = [];
        if ($v instanceof ZDict) {
            foreach ($v->map as $key => $value) $pairs[] = [is_int($key) ? $key : substr($key, 1), $value];
            return $pairs;
        }
        foreach (self::items($v, $line) as $at => $item) $pairs[] = [$at, $item];
        return $pairs;
    }

    static function list(array $values) { return new ZList($values); }
    static function dict(array $flat, int $line) {
        $d = new ZDict();
        for ($i = 0; $i < count($flat); $i += 2) $d->map[self::key($flat[$i], $line)] = $flat[$i + 1];
        return $d;
    }
    static function push(ZList $list, $value) { $list->v[] = $value; return null; }
    static function concat(array $pieces): string { return implode("", $pieces); }

    // ------------------------------------------------- Objects and fields

    static function fn(string $name, array $params, array $defaults, Closure $fn) { return new ZFn($name, $params, $defaults, $fn); }
    static function class_(string $name, array $fields, array $defaults, array $methods) { return new ZClass($name, $fields, $defaults, $methods); }
    static function enum(string $name, array $cases) { return new ZEnum($name, $cases); }

    // A case with no values is a value; one with values needs calling.
    static function case(ZEnum $def, int $i) {
        return count($def->cases[$i][1]) === 0 ? new ZCase($def, $i, []) : new ZConstructor($def, $i);
    }

    static function isCase($value, $ctor, int $count, int $line): bool {
        if (!($ctor instanceof ZConstructor)) self::fatal("this is not an enum case that carries values", $line);
        if (!($value instanceof ZCase) || $value->def !== $ctor->def || $value->index !== $ctor->index) return false;
        if (count($value->fields) !== $count) self::fatal("`" . $ctor->def->cases[$ctor->index][0] . "` carries " . count($value->fields) . " values, but this pattern has " . $count, $line);
        return true;
    }
    static function caseField(ZCase $value, int $i) { return $value->fields[$i]; }

    static function field($object, string $name, int $line) {
        if ($object instanceof ZInstance) {
            $at = array_search($name, $object->cls->fields, true);
            if ($at !== false) return $object->fields[$at];
            if (isset($object->cls->methods[$name])) return new ZBound($object, $object->cls->methods[$name]);
            self::fatal("`" . $object->cls->name . "` has no field or method `" . $name . "`", $line);
        }
        if ($object instanceof ZCase) {
            $at = array_search($name, $object->def->cases[$object->index][1], true);
            if ($at !== false) return $object->fields[$at];
            self::fatal("`" . $object->def->cases[$object->index][0] . "` has no field `" . $name . "`", $line);
        }
        if ($object instanceof ZEnum) {
            foreach ($object->cases as $at => $case) if ($case[0] === $name) return self::case($object, $at);
            self::fatal("enum `" . $object->name . "` has no case `" . $name . "`", $line);
        }
        if ($object instanceof ZModule) return self::builtin($object->name . "." . $name, $line);
        if ($object instanceof ZQuantity && self::isUnit($name)) {
            $scale = self::convert($object->unit, $name);
            if ($scale === null) self::fatal("a quantity in " . $object->unit . " cannot be turned into " . $name . ". They measure different things", $line);
            return new ZQuantity(self::apply($scale, $object->v), $name);
        }
        if ($object instanceof ZError && $name === "message") return $object->message;
        if ($object instanceof ZError && $name === "kind") return $object->kind;
        if ($object instanceof ZFn && $object->builtin !== null && $name === "wrap" && isset(self::WIDTHS[$object->builtin])) return self::builtin($object->builtin . ".wrap", $line);
        if (self::hasMethod($object, $name)) return new ZMethod($object, $name);
        self::fatal(self::typeName($object) . " has no field or method `" . $name . "`", $line);
    }

    static function setField($object, string $name, $value, int $line) {
        if ($object instanceof ZInstance) {
            $at = array_search($name, $object->cls->fields, true);
            if ($at === false) self::fatal("`" . $object->cls->name . "` has no field `" . $name . "`", $line);
            $object->fields[$at] = $value;
            return;
        }
        self::fatal("cannot set a field on " . self::typeName($object), $line);
    }

    // --------------------------------------------------------------- Calls

    static function counted(int $n, string $word): string { return $n . " " . $word . ($n === 1 ? "" : "s"); }
    static function were(int $n): string { return $n === 1 ? "1 was" : $n . " were"; }

    // Line the arguments up with the parameters: one slot each, MISSING
    // where none was given.
    static function bind(string $what, array $names, array $positional, array $namedNames, array $named, int $line): array {
        if (count($positional) > count($names)) self::fatal("`" . $what . "` takes " . self::counted(count($names), "argument") . ", but " . self::were(count($positional)) . " given", $line);
        $slots = $positional;
        while (count($slots) < count($names)) $slots[] = self::$MISSING;
        foreach ($namedNames as $i => $name) {
            $at = array_search($name, $names, true);
            if ($at === false) self::fatal("`" . $what . "` has no argument named `" . $name . "`", $line);
            if ($slots[$at] !== self::$MISSING) self::fatal("`" . $what . "` was given `" . $name . "` twice", $line);
            $slots[$at] = $named[$i];
        }
        return $slots;
    }

    // How many calls are nested now. `zeno run` stops at 1000, so a
    // program stops at the same call here.
    private static $depth = 0;
    const MAX_DEPTH = 1000;

    static function enter(ZFn $fn, int $line) {
        if (self::$depth >= self::MAX_DEPTH) self::fatal("more than 1000 calls are nested. Is `" . ($fn->name !== "" ? $fn->name : "function") . "` calling itself forever?", $line);
        self::$depth++;
    }

    // A call of a function of the program known where it is written, with
    // its arguments lined up already.
    static function direct(ZFn $fn, int $line, ...$args) {
        self::enter($fn, $line);
        try {
            return ($fn->fn)(...$args);
        } finally {
            self::$depth--;
        }
    }

    static function callFunction(ZFn $fn, array $positional, array $namedNames, array $named, int $line) {
        $name = $fn->name !== "" ? $fn->name : "function";
        self::enter($fn, $line);
        try {
            $slots = self::bind($name, $fn->params, $positional, $namedNames, $named, $line);
            foreach ($slots as $i => $slot) {
                if ($slot === self::$MISSING && !$fn->defaults[$i]) self::fatal("`" . $name . "` needs a value for `" . $fn->params[$i] . "`", $line);
            }
            return ($fn->fn)(...$slots);
        } finally {
            self::$depth--;
        }
    }

    static function noNamed(string $what, array $namedNames, int $line) {
        if (count($namedNames) > 0) self::fatal("`" . $what . "` does not take a named argument `" . $namedNames[0] . "`", $line);
    }

    static function callNamed($callee, array $positional, array $namedNames, array $named, int $line) {
        if ($callee instanceof ZFn) {
            if ($callee->builtin !== null) { self::noNamed($callee->name, $namedNames, $line); return self::callBuiltin($callee->builtin, $positional, $line); }
            return self::callFunction($callee, $positional, $namedNames, $named, $line);
        }
        if ($callee instanceof ZBound) return self::callFunction($callee->fn, array_merge([$callee->object], $positional), $namedNames, $named, $line);
        if ($callee instanceof ZMethod) { self::noNamed($callee->name, $namedNames, $line); return self::callMethodOf($callee->receiver, $callee->name, $positional, $line); }
        if ($callee instanceof ZClass) {
            $slots = self::bind($callee->name, $callee->fields, $positional, $namedNames, $named, $line);
            foreach ($slots as $i => $slot) {
                if ($slot !== self::$MISSING) continue;
                if ($callee->defaults[$i] === null) self::fatal("`" . $callee->name . "` needs a value for `" . $callee->fields[$i] . "`", $line);
                $slots[$i] = ($callee->defaults[$i]->fn)();
            }
            return new ZInstance($callee, $slots);
        }
        if ($callee instanceof ZConstructor) {
            [$name, $fields] = $callee->def->cases[$callee->index];
            $slots = self::bind($name, $fields, $positional, $namedNames, $named, $line);
            foreach ($slots as $i => $slot) if ($slot === self::$MISSING) self::fatal("`" . $name . "` needs a value for `" . $fields[$i] . "`", $line);
            return new ZCase($callee->def, $callee->index, $slots);
        }
        if ($callee instanceof ZErrorType) { self::noNamed($callee->kind, $namedNames, $line); return self::newError($callee->kind, $positional, $line); }
        self::fatal(self::typeName($callee) . " cannot be called", $line);
    }

    static function call($callee, array $args, int $line) { return self::callNamed($callee, $args, [], [], $line); }

    static function newError(string $kind, array $args, int $line) {
        if (count($args) > 1) self::fatal("`" . $kind . "` takes one message", $line);
        return new ZError($kind, count($args) === 0 ? "" : self::show($args[0]), null);
    }

    // `object.name(args)`
    static function callMethodNamed($object, string $name, array $positional, array $namedNames, array $named, int $line) {
        if ($object instanceof ZInstance) {
            if (!in_array($name, $object->cls->fields, true) && isset($object->cls->methods[$name])) {
                return self::callFunction($object->cls->methods[$name], array_merge([$object], $positional), $namedNames, $named, $line);
            }
        }
        if ($object instanceof ZModule) { self::noNamed($name, $namedNames, $line); return self::callBuiltin($object->name . "." . $name, $positional, $line); }
        if (is_string($object) || $object instanceof ZList || $object instanceof ZDict) {
            self::noNamed($name, $namedNames, $line);
            return self::callMethodOf($object, $name, $positional, $line);
        }
        return self::callNamed(self::field($object, $name, $line), $positional, $namedNames, $named, $line);
    }
    static function callMethod($object, string $name, array $args, int $line) { return self::callMethodNamed($object, $name, $args, [], [], $line); }

    // -------------------------------------------------------------- Errors

    static function raise($value, int $line) {
        if ($value instanceof ZError) throw new ZRaise($value, $line);
        if ($value instanceof ZInstance) throw new ZRaise(new ZError($value->cls->name, self::show($value), $value), $line);
        self::fatal("only an error can be raised, and this is " . self::typeName($value), $line);
    }

    // `except name` catches this error?
    static function catches(ZRaise $e, string $name): bool {
        return $name === "Error" || $name === $e->value->kind;
    }

    // What `except name as e` calls `e`.
    static function caught(ZRaise $e, string $name) {
        if ($name !== "" && $name !== "Error") return $e->value->object !== null ? $e->value->object : $e->value;
        return $e->value;
    }

    static function noMatch($v, int $line) { self::fatal("no `case` matches " . self::repr($v), $line); }
    static function undefined(string $name, int $line) { self::fatal("`" . $name . "` is not defined", $line); }

    // ------------------------------------------------- Built-in functions

    static function arity(string $name, array $args, int $min, int $max, int $line) {
        if (count($args) >= $min && count($args) <= $max) return;
        $wanted = $min === $max ? self::counted($min, "argument") : $min . " to " . $max . " arguments";
        self::fatal("`" . $name . "` takes " . $wanted . ", but " . self::were(count($args)) . " given", $line);
    }
    static function wrongType(string $name, string $wanted, $got, int $line) { self::fatal("`" . $name . "` needs " . $wanted . ", but this is " . self::typeName($got), $line); }
    static function text(string $name, $v, int $line): string { if (!is_string($v)) self::wrongType($name, "a str", $v, $line); return $v; }
    static function whole(string $name, $v, int $line): int { if (!is_int($v)) self::wrongType($name, "a whole number", $v, $line); return $v; }
    static function strip(string $s): string { return trim($s, " \t\n\r\f"); }

    // What is printed goes out in large pieces, which is far faster than a
    // line at a time.
    private static $output = "";
    static function flush() {
        if (self::$output === "") return;
        $out = self::$output;
        self::$output = "";
        // Nobody is reading any more (`| head`): stop quietly.
        if (@fwrite(STDOUT, $out) === false) exit(0);
    }
    static function write(string $s) {
        self::$output .= $s;
        if (strlen(self::$output) > 8192) self::flush();
    }

    // What was typed may be anything. Bytes that are not UTF-8 become
    // U+FFFD, one for each run of bytes that cannot begin a character.
    static function lossy(string $s): string {
        if (preg_match('//u', $s) === 1) return $s;
        $out = "";
        $n = strlen($s);
        $i = 0;
        while ($i < $n) {
            $b = ord($s[$i]);
            if ($b < 0x80) { $out .= $s[$i]; $i++; continue; }
            // How many bytes follow, and what the second one may be.
            $more = 0; $low = 0x80; $high = 0xbf;
            if ($b >= 0xc2 && $b <= 0xdf) $more = 1;
            else if ($b === 0xe0) { $more = 2; $low = 0xa0; }
            else if ($b === 0xed) { $more = 2; $high = 0x9f; }
            else if ($b >= 0xe1 && $b <= 0xef) $more = 2;
            else if ($b === 0xf0) { $more = 3; $low = 0x90; }
            else if ($b === 0xf4) { $more = 3; $high = 0x8f; }
            else if ($b >= 0xf1 && $b <= 0xf3) $more = 3;
            $got = 1;
            while ($got <= $more && $i + $got < $n) {
                $c = ord($s[$i + $got]);
                if ($c < $low || $c > $high) break;
                $low = 0x80; $high = 0xbf;
                $got++;
            }
            if ($more > 0 && $got === $more + 1) $out .= substr($s, $i, $got); else $out .= "\u{fffd}";
            $i += $got;
        }
        return $out;
    }

    const WIDTHS = [
        "u8" => [0, 255, 8, false], "u16" => [0, 65535, 16, false], "u32" => [0, 4294967295, 32, false],
        "i8" => [-128, 127, 8, true], "i16" => [-32768, 32767, 16, true], "i32" => [-2147483648, 2147483647, 32, true],
    ];

    static function amount($v): ?float { return $v instanceof ZQuantity ? $v->v : self::number($v); }

    static function numbers(string $name, array $args, int $line): array {
        $values = count($args) === 1 && $args[0] instanceof ZList ? $args[0]->v : $args;
        $u = null;
        foreach ($values as $place => $item) {
            if ($item instanceof ZQuantity) {
                if ($u !== null && $u !== $item->unit) self::fatal("`" . $name . "` needs quantities in one unit, but these are in " . $u . " and in " . $item->unit . ". Turn one into the other first, as in `x." . $u . "`", $line);
                if ($u === null && $place > 0) self::fatal("`" . $name . "` needs all plain numbers or all quantities, but these are both", $line);
                $u = $item->unit;
            } else if (self::number($item) === null) self::wrongType($name, "numbers", $item, $line);
            else if ($u !== null) self::fatal("`" . $name . "` needs all plain numbers or all quantities, but these are both", $line);
        }
        return [$values, $u];
    }

    static function pick(string $name, array $args, int $line, bool $smaller) {
        [$values] = self::numbers($name, $args, $line);
        if (count($values) === 0) self::raise_("ValueError", "`" . $name . "` of nothing", $line);
        $singles = false;
        foreach ($values as $item) $singles = $singles || $item instanceof ZF32;
        foreach ($values as $item) $singles = $singles && ($item instanceof ZF32 || is_int($item));
        if ($singles) $values = array_map(fn($item) => new ZF32(self::single($item)), $values);
        $best = $values[0];
        foreach ($values as $item) {
            if ($item instanceof ZU64 && $best instanceof ZU64) {
                if ($smaller ? self::uLess($item->v, $best->v) : self::uLess($best->v, $item->v)) $best = $item;
                continue;
            }
            $a = self::amount($item); $b = self::amount($best);
            if ($smaller ? $a < $b : $a > $b) $best = $item;
        }
        return $best;
    }

    static function parseFloatText(string $s): ?float {
        if (preg_match('/^[+-]?(\d+\.?\d*([eE][+-]?\d+)?|\.\d+([eE][+-]?\d+)?|inf|infinity|nan)$/Di', $s) !== 1) return null;
        $lower = strtolower($s);
        if ($lower[0] === "+") $lower = substr($lower, 1);
        if ($lower === "inf" || $lower === "infinity") return INF;
        if ($lower === "-inf" || $lower === "-infinity") return -INF;
        if ($lower === "nan" || $lower === "-nan") return NAN;
        return (float)$s;
    }

    // Heapsort, step for step as builtins.rs and runtime.c do it, so that
    // values that are neither before the other end up where they do there.
    static function heapsort(int $n, Closure $before, Closure $swap) {
        $sift = static function (int $root, int $end) use ($before, $swap) {
            for (;;) {
                $child = 2 * $root + 1;
                if ($child >= $end) return;
                if ($child + 1 < $end && $before($child, $child + 1)) $child += 1;
                if (!$before($root, $child)) return;
                $swap($root, $child);
                $root = $child;
            }
        };
        for ($root = intdiv($n, 2) - 1; $root >= 0; $root--) $sift($root, $n);
        for ($end = $n - 1; $end >= 1; $end--) { $swap(0, $end); $sift(0, $end); }
    }

    static function sortList(ZList $items, int $line) {
        $sorted = $items->v;
        self::heapsort(count($sorted),
            static function ($i, $j) use (&$sorted, $line) { return self::lt($sorted[$i], $sorted[$j], $line); },
            static function ($i, $j) use (&$sorted) { $t = $sorted[$i]; $sorted[$i] = $sorted[$j]; $sorted[$j] = $t; });
        foreach ($sorted as $i => $item) $items->v[$i] = $item;
    }

    static function sortBy(ZList $items, $fn, int $line) {
        $keys = [];
        foreach ($items->v as $item) {
            $k = self::call($fn, [$item], $line);
            if (!is_int($k) && !is_float($k) && !($k instanceof ZF32) && !($k instanceof ZU64) && !is_string($k)) self::fatal("the function given to `sort` must return a number or text to order by, but it returned " . self::typeName($k), $line);
            $keys[] = $k;
        }
        $sorted = $items->v;
        if (count($sorted) !== count($keys)) self::raise_("ValueError", "the function given to `sort` changed how many items the list has", $line);
        $places = array_keys($sorted);
        self::heapsort(count($sorted),
            static function ($a, $b) use (&$keys, &$places, $line) {
                return self::lt($keys[$a], $keys[$b], $line) ? true : (self::lt($keys[$b], $keys[$a], $line) ? false : $places[$a] < $places[$b]);
            },
            static function ($i, $j) use (&$sorted, &$keys, &$places) {
                $t = $sorted[$i]; $sorted[$i] = $sorted[$j]; $sorted[$j] = $t;
                $t = $keys[$i]; $keys[$i] = $keys[$j]; $keys[$j] = $t;
                $t = $places[$i]; $places[$i] = $places[$j]; $places[$j] = $t;
            });
        $items->v = $sorted;
    }

    static function slice(array $args, int $length, string $what, int $line): array {
        $from = self::whole("slice", $args[0], $line);
        $to = count($args) > 1 ? self::whole("slice", $args[1], $line) : $length;
        if ($from < 0 || $to < $from || $to > $length) self::raise_("IndexError", "slice " . $from . " to " . $to . " is out of range for a " . $what . " of length " . $length, $line);
        return [$from, $to];
    }

    const STR_METHODS = ["upper", "lower", "strip", "split", "lines", "replace", "starts_with", "ends_with", "find", "slice", "code"];
    const LIST_METHODS = ["append", "pop", "map", "filter", "reduce", "insert", "reverse", "sort", "clear", "find", "slice", "join"];
    const DICT_METHODS = ["get", "pop", "keys", "values", "clear"];
    static function hasMethod($v, string $name): bool {
        if (is_string($v)) return in_array($name, self::STR_METHODS, true);
        if ($v instanceof ZList) return in_array($name, self::LIST_METHODS, true);
        if ($v instanceof ZDict) return in_array($name, self::DICT_METHODS, true);
        return false;
    }

    // The code of the one character of a str.
    static function codeOf(string $c): int {
        $b = ord($c[0]);
        if ($b < 0x80) return $b;
        if ($b < 0xe0) return (($b & 0x1f) << 6) | (ord($c[1]) & 0x3f);
        if ($b < 0xf0) return (($b & 0x0f) << 12) | ((ord($c[1]) & 0x3f) << 6) | (ord($c[2]) & 0x3f);
        return (($b & 0x07) << 18) | ((ord($c[1]) & 0x3f) << 12) | ((ord($c[2]) & 0x3f) << 6) | (ord($c[3]) & 0x3f);
    }
    // The character with a code, in UTF-8.
    static function charOf(int $code): string {
        if ($code < 0x80) return chr($code);
        if ($code < 0x800) return chr(0xc0 | ($code >> 6)) . chr(0x80 | ($code & 0x3f));
        if ($code < 0x10000) return chr(0xe0 | ($code >> 12)) . chr(0x80 | (($code >> 6) & 0x3f)) . chr(0x80 | ($code & 0x3f));
        return chr(0xf0 | ($code >> 18)) . chr(0x80 | (($code >> 12) & 0x3f)) . chr(0x80 | (($code >> 6) & 0x3f)) . chr(0x80 | ($code & 0x3f));
    }

    static function callMethodOf($receiver, string $name, array $args, int $line) {
        if (is_string($receiver)) {
            $s = $receiver;
            switch ($name) {
                case "upper": self::arity($name, $args, 0, 0, $line); return strtr($s, "abcdefghijklmnopqrstuvwxyz", "ABCDEFGHIJKLMNOPQRSTUVWXYZ");
                case "lower": self::arity($name, $args, 0, 0, $line); return strtr($s, "ABCDEFGHIJKLMNOPQRSTUVWXYZ", "abcdefghijklmnopqrstuvwxyz");
                case "strip": self::arity($name, $args, 0, 0, $line); return self::strip($s);
                case "split": {
                    self::arity($name, $args, 0, 1, $line);
                    if (count($args) === 0) return new ZList(preg_split('/[ \t\n\r\f]+/', $s, -1, PREG_SPLIT_NO_EMPTY));
                    $sep = self::text($name, $args[0], $line);
                    if ($sep === "") self::raise_("ValueError", "cannot split on an empty str", $line);
                    return new ZList(explode($sep, $s));
                }
                case "lines": {
                    self::arity($name, $args, 0, 0, $line);
                    // A "\r" goes with the "\n" just after it, and with nothing else.
                    if ($s === "") return new ZList([]);
                    $parts = explode("\n", $s);
                    $last = array_pop($parts);
                    foreach ($parts as $i => $p) if ($p !== "" && $p[strlen($p) - 1] === "\r") $parts[$i] = substr($p, 0, -1);
                    if ($last !== "") $parts[] = $last;
                    return new ZList($parts);
                }
                case "replace": {
                    self::arity($name, $args, 2, 2, $line);
                    $old = self::text($name, $args[0], $line); $now = self::text($name, $args[1], $line);
                    // The new text between every character, and at both ends.
                    if ($old === "") return $now . implode($now, self::chars($s)) . ($s === "" ? "" : $now);
                    return str_replace($old, $now, $s);
                }
                case "starts_with": {
                    self::arity($name, $args, 1, 1, $line);
                    $start = self::text($name, $args[0], $line);
                    return strncmp($s, $start, strlen($start)) === 0;
                }
                case "ends_with": {
                    self::arity($name, $args, 1, 1, $line);
                    $end = self::text($name, $args[0], $line);
                    return $end === "" || (strlen($end) <= strlen($s) && substr($s, -strlen($end)) === $end);
                }
                case "find": {
                    self::arity($name, $args, 1, 1, $line);
                    $what = self::text($name, $args[0], $line);
                    $at = $what === "" ? 0 : strpos($s, $what);
                    return $at === false ? null : self::strLen(substr($s, 0, $at));
                }
                case "slice": {
                    self::arity($name, $args, 1, 2, $line);
                    if (self::isAscii($s)) {
                        [$from, $to] = self::slice($args, strlen($s), "str", $line);
                        return substr($s, $from, $to - $from);
                    }
                    $cs = self::chars($s);
                    [$from, $to] = self::slice($args, count($cs), "str", $line);
                    return implode("", array_slice($cs, $from, $to - $from));
                }
                case "code": {
                    self::arity($name, $args, 0, 0, $line);
                    $count = self::strLen($s);
                    if ($count !== 1) self::raise_("ValueError", "`code` needs a str of exactly one character, and this one has " . $count, $line);
                    return self::codeOf($s);
                }
            }
            self::fatal("a str has no method `" . $name . "`", $line);
        }
        if ($receiver instanceof ZDict) {
            switch ($name) {
                case "get": {
                    self::arity($name, $args, 1, 2, $line);
                    $k = self::key($args[0], $line);
                    if (array_key_exists($k, $receiver->map)) return $receiver->map[$k];
                    return count($args) > 1 ? $args[1] : null;
                }
                case "pop": {
                    self::arity($name, $args, 1, 1, $line);
                    $k = self::key($args[0], $line);
                    if (!array_key_exists($k, $receiver->map)) self::raise_("KeyError", "this dict has no key " . self::repr($args[0]), $line);
                    $value = $receiver->map[$k];
                    unset($receiver->map[$k]);
                    return $value;
                }
                case "keys": self::arity($name, $args, 0, 0, $line); return new ZList(self::keys($receiver));
                case "values": self::arity($name, $args, 0, 0, $line); return new ZList(array_values($receiver->map));
                case "clear": self::arity($name, $args, 0, 0, $line); $receiver->map = []; return null;
            }
            self::fatal("a dict has no method `" . $name . "`", $line);
        }
        if ($receiver instanceof ZList) {
            $xs = $receiver;
            switch ($name) {
                case "append": self::arity($name, $args, 1, 1, $line); $xs->v[] = $args[0]; return null;
                case "pop": {
                    self::arity($name, $args, 0, 1, $line);
                    if (count($args) === 1) {
                        $at = self::position($args[0], count($xs->v), "list", $line);
                        return array_splice($xs->v, $at, 1)[0];
                    }
                    if (count($xs->v) === 0) self::raise_("IndexError", "cannot pop from an empty list", $line);
                    return array_pop($xs->v);
                }
                case "insert": {
                    self::arity($name, $args, 2, 2, $line);
                    $at = self::whole($name, $args[0], $line);
                    if ($at < 0 || $at > count($xs->v)) self::raise_("IndexError", "cannot insert at " . $at . " in a list of length " . count($xs->v), $line);
                    array_splice($xs->v, $at, 0, [$args[1]]);
                    return null;
                }
                case "reverse": self::arity($name, $args, 0, 0, $line); $xs->v = array_reverse($xs->v); return null;
                case "clear": self::arity($name, $args, 0, 0, $line); $xs->v = []; return null;
                case "sort":
                    if (count($args) === 1) { self::sortBy($xs, $args[0], $line); return null; }
                    self::arity($name, $args, 0, 1, $line);
                    self::sortList($xs, $line);
                    return null;
                case "find": {
                    self::arity($name, $args, 1, 1, $line);
                    foreach ($xs->v as $at => $item) if (self::eq($item, $args[0])) return $at;
                    return null;
                }
                case "slice": {
                    self::arity($name, $args, 1, 2, $line);
                    [$from, $to] = self::slice($args, count($xs->v), "list", $line);
                    return new ZList(array_slice($xs->v, $from, $to - $from));
                }
                case "join": {
                    self::arity($name, $args, 1, 1, $line);
                    $sep = self::text($name, $args[0], $line);
                    foreach ($xs->v as $item) {
                        if (!is_string($item)) self::fatal("`join` puts text together, and this list holds " . self::typeName($item) . ". Make each item text first, with `str( )`", $line);
                    }
                    return implode($sep, $xs->v);
                }
                case "map": {
                    self::arity($name, $args, 1, 1, $line);
                    $out = [];
                    foreach ($xs->v as $item) $out[] = self::call($args[0], [$item], $line);
                    return new ZList($out);
                }
                case "filter": {
                    self::arity($name, $args, 1, 1, $line);
                    $kept = [];
                    foreach ($xs->v as $item) {
                        $keep = self::call($args[0], [$item], $line);
                        if ($keep === true) $kept[] = $item;
                        else if ($keep !== false) self::fatal("the function given to `filter` must return true or false, but it returned " . self::typeName($keep), $line);
                    }
                    return new ZList($kept);
                }
                case "reduce": {
                    self::arity($name, $args, 2, 2, $line);
                    $total = $args[1];
                    foreach ($xs->v as $item) $total = self::call($args[0], [$total, $item], $line);
                    return $total;
                }
            }
            self::fatal("a list has no method `" . $name . "`", $line);
        }
        self::fatal(self::typeName($receiver) . " has no method `" . $name . "`", $line);
    }

    // The format after the colon of `{value:format}`.
    static function format($v, string $spec, int $line): string {
        if ($spec === "") return self::show($v);
        $bad = static function () use ($spec, $line) { self::fatal("`" . $spec . "` is not a format Zeno understands", $line); };
        if (preg_match('/^([<>^])?(0)?(\d*)(?:\.(\d*))?(.)?$/Du', $spec, $m, PREG_UNMATCHED_AS_NULL) !== 1) $bad();
        $m = array_pad($m, 6, null);
        $align = $m[1]; $zero = $m[2] !== null; $width = $m[3] !== null && $m[3] !== "" ? (int)$m[3] : 0;
        $precision = $m[4] !== null ? (int)$m[4] : null;
        $kind = $m[5];
        if ($kind !== null && strlen($kind) === 1 && $kind >= "0" && $kind <= "9") $bad();
        if (($kind === "f" || ($kind === null && $precision !== null)) && $v instanceof ZQuantity) {
            $body = self::fixed($v->v, $precision === null ? 6 : $precision) . " " . $v->unit;
        } else if ($kind === "f" || ($kind === null && $precision !== null && is_float($v))) {
            $x = self::number($v);
            if ($x === null) self::wrongType($spec, "a number", $v, $line);
            $body = self::fixed($x, $precision === null ? 6 : $precision);
        } else if (($kind === null && $precision === null) || ($kind === "d" && $precision === null && (is_int($v) || $v instanceof ZU64))) {
            $body = self::show($v);
        } else if (($kind === "x" || $kind === "X" || $kind === "b") && $precision === null && $v instanceof ZU64) {
            // PHP writes the bits of an int with no sign.
            $body = $kind === "b" ? decbin($v->v) : dechex($v->v);
            if ($kind === "X") $body = strtoupper($body);
        } else if (($kind === "x" || $kind === "X" || $kind === "b") && $precision === null && is_int($v)) {
            // The digits of the size of the number, with `-` before them.
            // The smallest int has no size that fits, so its digits are
            // read from its bits.
            if ($v === PHP_INT_MIN) $digits = $kind === "b" ? "1" . str_repeat("0", 63) : "8000000000000000";
            else $digits = $kind === "b" ? decbin(abs($v)) : dechex(abs($v));
            if ($kind === "X") $digits = strtoupper($digits);
            $body = ($v < 0 ? "-" : "") . $digits;
        } else if (($kind === "d" || $kind === "x" || $kind === "X" || $kind === "b") && $precision === null) {
            self::wrongType($spec, "an int", $v, $line);
        } else {
            $bad();
        }
        $padding = max(0, $width - self::strLen($body));
        $isNumber = is_int($v) || is_float($v) || $v instanceof ZQuantity || $v instanceof ZU64;
        if ($zero && $isNumber) {
            $negative = $body !== "" && $body[0] === "-";
            return ($negative ? "-" : "") . str_repeat("0", $padding) . ($negative ? substr($body, 1) : $body);
        }
        switch ($align !== null ? $align : ($isNumber ? ">" : "<")) {
            case "<": return $body . str_repeat(" ", $padding);
            case ">": return str_repeat(" ", $padding) . $body;
        }
        $left = intdiv($padding, 2);
        return str_repeat(" ", $left) . $body . str_repeat(" ", $padding - $left);
    }

    // ---------------------------------------------------------------- JSON

    // The reader walks the bytes of the text: every mark of JSON is one
    // byte, and the bytes of a longer character pass through as they are.
    private static $json = "";
    private static $jsonAt = 0;

    private static function jsonSpaces() {
        self::$jsonAt += strspn(self::$json, " \t\n\r", self::$jsonAt);
    }
    private static function jsonWord(string $w): bool {
        if (substr_compare(self::$json, $w, self::$jsonAt, strlen($w)) !== 0) return false;
        self::$jsonAt += strlen($w);
        return true;
    }
    private static function jsonHex(): int {
        $digits = (string)substr(self::$json, self::$jsonAt, 4);
        self::$jsonAt += 4;
        if (preg_match('/^[0-9a-fA-F]{4}$/D', $digits) !== 1) throw new ZNotJson();
        return hexdec($digits);
    }
    private static function jsonTextValue(): string {
        $s = self::$json;
        $n = strlen($s);
        if (self::$jsonAt >= $n || $s[self::$jsonAt] !== '"') throw new ZNotJson();
        self::$jsonAt++;
        $out = "";
        for (;;) {
            // All that is plain, in one piece.
            $plain = strcspn($s, "\"\\", self::$jsonAt);
            $out .= substr($s, self::$jsonAt, $plain);
            self::$jsonAt += $plain;
            if (self::$jsonAt >= $n) throw new ZNotJson();
            $c = $s[self::$jsonAt++];
            if ($c === '"') return $out;
            if (self::$jsonAt >= $n) throw new ZNotJson();
            $e = $s[self::$jsonAt++];
            if ($e === "n") $out .= "\n";
            else if ($e === "t") $out .= "\t";
            else if ($e === "r") $out .= "\r";
            else if ($e === "b") $out .= "\x08";
            else if ($e === "f") $out .= "\f";
            else if ($e === "u") {
                $code = self::jsonHex();
                if ($code >= 0xd800 && $code < 0xdc00 && self::$jsonAt + 2 <= $n && self::jsonWord("\\u")) {
                    $low = self::jsonHex();
                    if ($low < 0xdc00 || $low > 0xdfff) throw new ZNotJson();
                    $code = 0x10000 + (($code - 0xd800) << 10) + ($low - 0xdc00);
                }
                if ($code >= 0xd800 && $code <= 0xdfff) throw new ZNotJson();
                $out .= self::charOf($code);
            } else $out .= $e;
        }
    }
    private static function jsonValue() {
        self::jsonSpaces();
        $s = self::$json;
        $n = strlen($s);
        if (self::$jsonAt >= $n) throw new ZNotJson();
        $c = $s[self::$jsonAt];
        if ($c === "n") { if (self::jsonWord("null")) return null; throw new ZNotJson(); }
        if ($c === "t") { if (self::jsonWord("true")) return true; throw new ZNotJson(); }
        if ($c === "f") { if (self::jsonWord("false")) return false; throw new ZNotJson(); }
        if ($c === '"') return self::jsonTextValue();
        if ($c === "[") {
            self::$jsonAt++;
            $items = [];
            for (;;) {
                self::jsonSpaces();
                if (self::$jsonAt < $n && self::jsonWord("]")) return new ZList($items);
                if (count($items) > 0 && !(self::$jsonAt < $n && self::jsonWord(","))) throw new ZNotJson();
                $items[] = self::jsonValue();
            }
        }
        if ($c === "{") {
            self::$jsonAt++;
            $d = new ZDict();
            $count = 0;
            for (;;) {
                self::jsonSpaces();
                if (self::$jsonAt < $n && self::jsonWord("}")) return $d;
                if ($count > 0 && !(self::$jsonAt < $n && self::jsonWord(","))) throw new ZNotJson();
                self::jsonSpaces();
                $name = self::jsonTextValue();
                self::jsonSpaces();
                if (!(self::$jsonAt < $n && self::jsonWord(":"))) throw new ZNotJson();
                $v = self::jsonValue();
                $count++;
                // A member that is null is left out: a dict holds no none.
                if ($v === null) continue;
                $d->map["s" . $name] = $v;
            }
        }
        $length = strspn($s, "0123456789+-.eE", self::$jsonAt);
        $x = self::parseFloatText(substr($s, self::$jsonAt, $length));
        self::$jsonAt += $length;
        if ($x === null) throw new ZNotJson();
        if (is_finite($x) && floor($x) === $x && abs($x) < 9.0e15) return (int)$x;
        return $x;
    }

    // The value a JSON text holds, or MISSING if the text is not JSON.
    static function jsonParse(string $s) {
        self::$json = $s;
        self::$jsonAt = 0;
        try {
            $v = self::jsonValue();
            self::jsonSpaces();
            return self::$jsonAt === strlen($s) ? $v : self::$MISSING;
        } catch (ZNotJson $e) {
            return self::$MISSING;
        } finally {
            self::$json = "";
        }
    }

    static function jsonText(string $s): string {
        return '"' . preg_replace_callback('/["\\\\\x00-\x1f]/', static function ($m) {
            switch ($m[0]) {
                case '"': return '\\"';
                case "\\": return "\\\\";
                case "\n": return "\\n";
                case "\r": return "\\r";
                case "\t": return "\\t";
            }
            return sprintf("\\u%04x", ord($m[0]));
        }, $s) . '"';
    }

    static function jsonWrite($v, int $line): string {
        if ($v === null) return "null";
        if ($v === true) return "true";
        if ($v === false) return "false";
        if (is_int($v)) return (string)$v;
        if (is_float($v)) {
            if (!is_finite($v)) self::raise_("ValueError", "JSON has no way to write " . (is_nan($v) ? "NaN" : ($v > 0 ? "inf" : "-inf")), $line);
            return self::showFloat($v);
        }
        if (is_string($v)) return self::jsonText($v);
        if ($v instanceof ZList) {
            $parts = [];
            foreach ($v->v as $item) $parts[] = self::jsonWrite($item, $line);
            return "[" . implode(",", $parts) . "]";
        }
        if ($v instanceof ZDict) {
            $parts = [];
            foreach ($v->map as $k => $value) $parts[] = self::jsonText(is_int($k) ? (string)$k : substr($k, 1)) . ":" . self::jsonWrite($value, $line);
            return "{" . implode(",", $parts) . "}";
        }
        if ($v instanceof ZInstance) {
            $parts = [];
            foreach ($v->cls->fields as $i => $name) $parts[] = self::jsonText($name) . ":" . self::jsonWrite($v->fields[$i], $line);
            return "{" . implode(",", $parts) . "}";
        }
        if ($v instanceof ZQuantity) self::raise_("ValueError", "JSON has no units, and this is a quantity in " . $v->unit . '. Write its number, and put the unit in the name: `["' . $v->unit . '": ...]`', $line);
        self::raise_("ValueError", "JSON has objects, lists, text, numbers, bools and none, and this is " . self::typeName($v), $line);
    }

    // --------------------------------------------------------------- Files

    // What is at a path: "file", "nothing", or "other" (a folder, or
    // something that cannot be read).
    static function at(string $path): string {
        if ($path === "") return "nothing";
        if (strpos($path, "\0") !== false) return "other";
        clearstatcache();
        $file = @fopen($path, "rb");
        if ($file === false) {
            if (@file_exists($path)) return "other";
            // Nothing by that name. If a name on the way to it is there
            // and is not a folder, the path cannot be followed at all.
            for ($parent = dirname($path); $parent !== dirname($parent) && !@file_exists($parent); $parent = dirname($parent));
            return @is_dir($parent) ? "nothing" : "other";
        }
        $byte = @fread($file, 1);
        fclose($file);
        return $byte === false ? "other" : "file";
    }
    static function paths(string $name, array $args, int $count, int $line): array {
        self::arity($name, $args, $count, $count, $line);
        foreach ($args as $a) self::text($name, $a, $line);
        return $args;
    }
    static function fileError(string $message, int $line) { self::raise_("FileError", $message, $line); }

    // ------------------------------------------------- The built-in table
    //
    // Each is `b_` and its name, with `_` for the `.` of a module:
    // Z::b_print, Z::b_math_sqrt.

    static function b_print(array $args, int $line) {
        $parts = [];
        foreach ($args as $a) $parts[] = self::show($a);
        self::write(implode(" ", $parts) . "\n");
        return null;
    }
    static function b_input(array $args, int $line) {
        self::arity("input", $args, 0, 1, $line);
        self::write(count($args) > 0 ? self::text("input", $args[0], $line) : "");
        self::flush();
        $typed = fgets(STDIN);
        if ($typed === false) return null;
        $s = self::lossy($typed);
        // The line end is not part of the line: "\n", or "\r\n". A "\r"
        // at the end of input that no "\n" follows stays.
        if ($s !== "" && $s[strlen($s) - 1] === "\n") {
            $s = substr($s, 0, -1);
            if ($s !== "" && $s[strlen($s) - 1] === "\r") $s = substr($s, 0, -1);
        }
        return str_replace("\0", "", $s);
    }
    static function b_len(array $args, int $line) {
        self::arity("len", $args, 1, 1, $line);
        $v = $args[0];
        if (is_string($v)) return self::strLen($v);
        if ($v instanceof ZList) return count($v->v);
        if ($v instanceof ZDict) return count($v->map);
        self::wrongType("len", "a str, a list or a dict", $v, $line);
    }
    static function b_range(array $args, int $line) {
        self::arity("range", $args, 1, 3, $line);
        foreach ($args as $a) if (!is_int($a)) self::wrongType("range", "ints", $a, $line);
        [$start, $stop, $step] = count($args) === 1 ? [0, $args[0], 1] : (count($args) === 2 ? [$args[0], $args[1], 1] : $args);
        if ($step === 0) self::raise_("ValueError", "the step of a range cannot be 0", $line);
        $out = [];
        // A step past the last int there is ends the range.
        for ($i = $start; is_int($i) && ($step > 0 ? $i < $stop : $i > $stop); $i += $step) $out[] = $i;
        return new ZList($out);
    }
    static function b_int(array $args, int $line) {
        self::arity("int", $args, 1, 1, $line);
        $v = $args[0];
        if (is_int($v)) return $v;
        if ($v instanceof ZU64) {
            if ($v->v < 0) self::raise_("OverflowError", self::uText($v->v) . " does not fit in an int (-9223372036854775808 to 9223372036854775807)", $line);
            return $v->v;
        }
        if ($v instanceof ZF32) $v = $v->v;
        if (is_float($v)) {
            if (!is_finite($v)) self::raise_("ValueError", "cannot turn " . (is_nan($v) ? "NaN" : ($v > 0 ? "inf" : "-inf")) . " into an int", $line);
            return self::toInt($v < 0 ? ceil($v) : floor($v));
        }
        if (is_string($v)) {
            $s = str_replace("_", "", self::strip($v));
            if (preg_match('/^([+-]?)0*(\d+)$/D', $s, $m) === 1) {
                // Nineteen digits may or may not fit: PHP says, by giving a
                // float for a number too big to be an int.
                $n = strlen($m[2]) <= 19 ? (($m[1] === "-" ? "-" : "") . $m[2]) + 0 : 0.0;
                if (is_int($n)) return $n;
                if ($m[1] === "-" && $m[2] === "9223372036854775808") return PHP_INT_MIN;
            }
            self::raise_("ValueError", "cannot turn " . self::quoted($v) . " into an int", $line);
        }
        self::wrongType("int", "a number or a str", $v, $line);
    }
    static function b_float(array $args, int $line) {
        self::arity("float", $args, 1, 1, $line);
        $v = $args[0];
        if (is_int($v)) return (float)$v;
        if (is_float($v)) return $v;
        if ($v instanceof ZF32) return $v->v;
        if ($v instanceof ZU64) return self::uFloat($v->v);
        if (is_string($v)) {
            $x = self::parseFloatText(self::strip($v));
            if ($x === null) self::raise_("ValueError", "cannot turn " . self::quoted($v) . " into a float", $line);
            return $x;
        }
        self::wrongType("float", "a number or a str", $v, $line);
    }
    static function b_f32(array $args, int $line) {
        self::arity("f32", $args, 1, 1, $line);
        $v = $args[0];
        if ($v instanceof ZF32) return $v;
        if (is_int($v)) return new ZF32(self::single($v));
        if (is_float($v)) {
            $r = self::fround($v);
            if (!is_finite($r) && is_finite($v)) self::raise_("OverflowError", self::showFloat($v) . " does not fit in an f32, whose largest is about 3.4e38", $line);
            return new ZF32($r);
        }
        self::wrongType("f32", "a number", $v, $line);
    }
    static function b_str(array $args, int $line) { self::arity("str", $args, 1, 1, $line); return self::show($args[0]); }
    static function b_abs(array $args, int $line) {
        self::arity("abs", $args, 1, 1, $line);
        $v = $args[0];
        if (is_int($v)) return $v < 0 ? self::neg($v, $line) : $v;
        if (is_float($v)) return abs($v);
        if ($v instanceof ZF32) return new ZF32(abs($v->v));
        if ($v instanceof ZU64) return $v;
        if ($v instanceof ZQuantity) return new ZQuantity(abs($v->v), $v->unit);
        self::wrongType("abs", "a number", $v, $line);
    }
    static function b_min(array $args, int $line) { return self::pick("min", $args, $line, true); }
    static function b_max(array $args, int $line) { return self::pick("max", $args, $line, false); }
    static function b_sum(array $args, int $line) {
        [$values, $u] = self::numbers("sum", $args, $line);
        if ($u !== null) self::fatal("`sum` works on plain numbers, and these have a unit. Add quantities with `+`, which keeps the unit: start from `var total = 0 V` and add each in a loop", $line);
        $singles = false;
        foreach ($values as $item) $singles = $singles || $item instanceof ZF32;
        foreach ($values as $item) $singles = $singles && ($item instanceof ZF32 || is_int($item));
        if ($singles) {
            $single = 0.0;
            foreach ($values as $item) $single = self::fround($single + self::single($item));
            return new ZF32($single);
        }
        $u64s = false;
        foreach ($values as $item) $u64s = $u64s || $item instanceof ZU64;
        if ($u64s) {
            $total = 0;
            foreach ($values as $item) {
                [$total, $cut] = self::uAdd($total, self::unsigned($item) ?? 0);
                if ($cut) self::raise_("OverflowError", self::U64_TOO_BIG, $line);
            }
            return new ZU64($total);
        }
        $total = 0; $allInts = true; $floatTotal = 0.0;
        foreach ($values as $item) {
            $floatTotal += self::number($item);
            if ($allInts && is_int($item)) $total = self::add($total, $item, $line);
            else $allInts = false;
        }
        return $allInts ? $total : $floatTotal;
    }
    static function b_char(array $args, int $line) {
        self::arity("char", $args, 1, 1, $line);
        $code = self::whole("char", $args[0], $line);
        if ($code <= 0 || $code > 0x10ffff || ($code >= 0xd800 && $code <= 0xdfff)) self::raise_("ValueError", $code . " is not the code of a character", $line);
        return self::charOf($code);
    }
    static function b_allocate(array $args, int $line) { self::fatal("manual memory is only for a program compiled to machine code", $line); }
    static function b_free(array $args, int $line) { self::fatal("manual memory is only for a program compiled to machine code", $line); }

    static function b_math_sqrt(array $args, int $line) {
        $x = self::oneNumber("sqrt", $args, $line);
        if ($x < 0) self::raise_("ValueError", "cannot take the square root of " . self::showFloat($x), $line);
        return sqrt($x);
    }
    static function b_math_floor(array $args, int $line) { return self::toInt(floor(self::oneNumber("floor", $args, $line))); }
    static function b_math_ceil(array $args, int $line) { return self::toInt(ceil(self::oneNumber("ceil", $args, $line))); }
    static function b_math_round(array $args, int $line) {
        // A half goes away from zero. Worked out from the float itself:
        // adding 0.5 first would round 0.49999999999999994 up.
        $x = self::oneNumber("round", $args, $line);
        $size = abs($x);
        $whole = floor($size);
        if ($size - $whole >= 0.5) $whole += 1;
        return self::toInt($x < 0 ? -$whole : $whole);
    }
    static function b_math_sin(array $args, int $line) { return sin(self::angle("sin", $args, $line)); }
    static function b_math_cos(array $args, int $line) { return cos(self::angle("cos", $args, $line)); }
    static function b_math_tan(array $args, int $line) { return tan(self::angle("tan", $args, $line)); }
    static function b_math_asin(array $args, int $line) { return new ZQuantity(asin(self::ratio("asin", $args, $line)), "rad"); }
    static function b_math_acos(array $args, int $line) { return new ZQuantity(acos(self::ratio("acos", $args, $line)), "rad"); }
    static function b_math_atan(array $args, int $line) { return new ZQuantity(atan(self::oneNumber("atan", $args, $line)), "rad"); }
    static function b_math_atan2(array $args, int $line) {
        self::arity("atan2", $args, 2, 2, $line);
        $y = self::number($args[0]); $x = self::number($args[1]);
        if ($y === null) self::wrongType("atan2", "two numbers", $args[0], $line);
        if ($x === null) self::wrongType("atan2", "two numbers", $args[1], $line);
        return new ZQuantity(atan2($y, $x), "rad");
    }
    static function b_math_exp(array $args, int $line) { return exp(self::oneNumber("exp", $args, $line)); }
    static function b_math_log(array $args, int $line) { return log(self::aboveZero("log", $args, $line)); }
    static function b_math_log2(array $args, int $line) { return log(self::aboveZero("log2", $args, $line), 2); }
    static function b_math_log10(array $args, int $line) { return log10(self::aboveZero("log10", $args, $line)); }

    static function b_time_sleep(array $args, int $line) {
        self::arity("sleep", $args, 1, 1, $line);
        $v = $args[0];
        if (!($v instanceof ZQuantity)) self::wrongType("sleep", "a time with a unit, such as `500 ms`", $v, $line);
        $scale = self::convert($v->unit, "s");
        if ($scale === null) self::fatal("`sleep` needs a time, such as `500 ms`, not a quantity in " . $v->unit, $line);
        $seconds = self::apply($scale, $v->v);
        if ($seconds > 0) {
            self::flush();
            $seconds = min($seconds, 1e9);
            $whole = (int)floor($seconds);
            time_nanosleep($whole, (int)(($seconds - $whole) * 1e9));
        }
        return null;
    }
    private static $started = 0;
    static function b_time_now(array $args, int $line) { self::arity("now", $args, 0, 0, $line); return new ZQuantity((hrtime(true) - self::$started) / 1e6, "ms"); }

    static function b_files_read(array $args, int $line) {
        [$path] = self::paths("read", $args, 1, $line);
        $where = self::at($path);
        if ($where === "nothing") self::fileError("there is no file `" . $path . "`", $line);
        if ($where === "other") self::fileError("cannot read `" . $path . "`", $line);
        $s = @file_get_contents($path);
        if ($s === false) self::fileError("cannot read `" . $path . "`", $line);
        if (preg_match('//u', $s) !== 1 || strpos($s, "\0") !== false) self::fileError("`" . $path . "` does not hold text", $line);
        return $s;
    }
    private static function put(string $path, string $s, int $flags, int $line) {
        try {
            $ok = @file_put_contents($path, $s, $flags) !== false;
        } catch (ValueError $e) {
            $ok = false;
        }
        if (!$ok) self::fileError("cannot write `" . $path . "`", $line);
        return null;
    }
    static function b_files_write(array $args, int $line) { [$path, $s] = self::paths("write", $args, 2, $line); return self::put($path, $s, 0, $line); }
    static function b_files_read_bytes(array $args, int $line) {
        [$path] = self::paths("read_bytes", $args, 1, $line);
        $where = self::at($path);
        if ($where === "nothing") self::fileError("there is no file `" . $path . "`", $line);
        if ($where === "other") self::fileError("cannot read `" . $path . "`", $line);
        $s = @file_get_contents($path);
        if ($s === false) self::fileError("cannot read `" . $path . "`", $line);
        return new ZList($s === "" ? [] : array_values(unpack("C*", $s)));
    }
    static function b_files_write_bytes(array $args, int $line) {
        self::arity("write_bytes", $args, 2, 2, $line);
        $path = self::text("write_bytes", $args[0], $line);
        $items = $args[1];
        if (!($items instanceof ZList)) self::wrongType("write_bytes", "a list[u8]", $items, $line);
        foreach ($items->v as $n) {
            if (!is_int($n)) self::wrongType("write_bytes", "a list[u8]", $n, $line);
            if ($n < 0 || $n > 255) self::raise_("ValueError", $n . " is not a byte: a byte is a number from 0 to 255", $line);
        }
        return self::put($path, pack("C*", ...$items->v), 0, $line);
    }
    static function b_files_append(array $args, int $line) { [$path, $s] = self::paths("append", $args, 2, $line); return self::put($path, $s, FILE_APPEND, $line); }
    static function b_files_exists(array $args, int $line) { [$path] = self::paths("exists", $args, 1, $line); return self::at($path) === "file"; }
    static function b_files_remove(array $args, int $line) {
        [$path] = self::paths("remove", $args, 1, $line);
        $where = self::at($path);
        if ($where === "nothing") self::fileError("there is no file `" . $path . "`", $line);
        if ($where === "file" && @unlink($path)) return null;
        self::fileError("cannot remove `" . $path . "`", $line);
    }

    // `program.exit(code)`: what was printed goes out, and the program
    // ends with `code`.
    static function b_program_exit(array $args, int $line) {
        self::arity("exit", $args, 1, 1, $line);
        $code = self::whole("exit", $args[0], $line);
        if ($code < 0 || $code > 255) self::raise_("ValueError", "a program ends with a number from 0 to 255, not " . $code, $line);
        self::flush();
        exit($code);
    }
    // `program.environment(name)`: the text, or none. A name with `=` or
    // a zero byte in it is in no environment.
    static function b_program_environment(array $args, int $line) {
        self::arity("environment", $args, 1, 1, $line);
        $name = self::text("environment", $args[0], $line);
        if ($name === "" || strpbrk($name, "=\0") !== false) return null;
        $value = getenv($name);
        return $value === false ? null : self::lossy($value);
    }
    // `program.warn(text)`: one line on the error stream, after what was printed.
    static function b_program_warn(array $args, int $line) {
        self::arity("warn", $args, 1, 1, $line);
        $said = self::text("warn", $args[0], $line);
        self::flush();
        fwrite(STDERR, $said . "\n");
        return null;
    }
    static function b_program_arguments(array $args, int $line) {
        self::arity("arguments", $args, 0, 0, $line);
        $words = [];
        foreach (array_slice($_SERVER["argv"] ?? [], 1) as $word) $words[] = self::lossy((string)$word);
        return new ZList($words);
    }

    static function b_json_parse(array $args, int $line) {
        self::arity("parse", $args, 1, 1, $line);
        $s = self::text("parse", $args[0], $line);
        $v = self::jsonParse($s);
        if ($v === self::$MISSING) {
            $rest = ltrim($s, " \t\n\r");
            preg_match('/^.{0,30}/su', $rest, $m);
            self::raise_("ValueError", "this text is not JSON: `" . $m[0] . (strlen($m[0]) < strlen($rest) ? "..." : "") . "`", $line);
        }
        return $v;
    }
    static function b_json_write(array $args, int $line) { self::arity("write", $args, 1, 1, $line); return self::jsonWrite($args[0], $line); }

    // A float that is a whole number, as an int: the nearest int there
    // is for one too big, and 0 for a NaN.
    static function toInt(float $x): int {
        if (is_nan($x)) return 0;
        if ($x >= 9223372036854775807) return PHP_INT_MAX;
        if ($x <= -9223372036854775808) return PHP_INT_MIN;
        return (int)$x;
    }
    static function oneNumber(string $name, array $args, int $line): float {
        self::arity($name, $args, 1, 1, $line);
        $x = self::number($args[0]);
        if ($x === null) self::wrongType($name, "a number", $args[0], $line);
        return $x;
    }
    static function angle(string $name, array $args, int $line): float {
        self::arity($name, $args, 1, 1, $line);
        $v = $args[0];
        if ($v instanceof ZQuantity) {
            $scale = self::convert($v->unit, "rad");
            if ($scale === null) self::fatal("`" . $name . "` needs a number or an angle, such as `30 deg`, not a quantity in " . $v->unit, $line);
            return self::apply($scale, $v->v);
        }
        $x = self::number($v);
        if ($x === null) self::wrongType($name, "a number or an angle, such as `30 deg`", $v, $line);
        return $x;
    }
    static function ratio(string $name, array $args, int $line): float {
        $x = self::oneNumber($name, $args, $line);
        if ($x < -1 || $x > 1) self::raise_("ValueError", "`" . $name . "` needs a number from -1 to 1, and this is " . self::showFloat($x), $line);
        return $x;
    }
    static function aboveZero(string $name, array $args, int $line): float {
        $x = self::oneNumber($name, $args, $line);
        if ($x <= 0) self::raise_("ValueError", "cannot take the logarithm of " . self::showFloat($x), $line);
        return $x;
    }

    // The sized whole numbers: `u8(n)` checks, `u8.wrap(n)` keeps the low bits.
    static function sized(string $name, array $args, int $line) {
        [$lo, $hi, $bits, $signed] = self::WIDTHS[$name];
        self::arity($name, $args, 1, 1, $line);
        $n = $args[0];
        if (!is_int($n)) self::wrongType($name, "a whole number", $n, $line);
        if ($n < $lo || $n > $hi) self::raise_("OverflowError", $n . " does not fit in " . ($signed ? "an" : "a") . " " . $name . " (" . $lo . " to " . $hi . ")", $line);
        return $n;
    }
    static function wrapped(string $name, array $args, int $line) {
        [$lo, $hi, $bits, $signed] = self::WIDTHS[$name];
        self::arity($name . ".wrap", $args, 1, 1, $line);
        $n = $args[0];
        if (!is_int($n)) self::wrongType($name . ".wrap", "a whole number", $n, $line);
        $low = $n & ((1 << $bits) - 1);
        return $signed && $low >= (1 << ($bits - 1)) ? $low - (1 << $bits) : $low;
    }
    static function b_u8(array $args, int $line) { return self::sized("u8", $args, $line); }
    static function b_u16(array $args, int $line) { return self::sized("u16", $args, $line); }
    static function b_u32(array $args, int $line) { return self::sized("u32", $args, $line); }
    static function b_i8(array $args, int $line) { return self::sized("i8", $args, $line); }
    static function b_i16(array $args, int $line) { return self::sized("i16", $args, $line); }
    static function b_i32(array $args, int $line) { return self::sized("i32", $args, $line); }
    static function b_u8_wrap(array $args, int $line) { return self::wrapped("u8", $args, $line); }
    static function b_u16_wrap(array $args, int $line) { return self::wrapped("u16", $args, $line); }
    static function b_u32_wrap(array $args, int $line) { return self::wrapped("u32", $args, $line); }
    static function b_i8_wrap(array $args, int $line) { return self::wrapped("i8", $args, $line); }
    static function b_i16_wrap(array $args, int $line) { return self::wrapped("i16", $args, $line); }
    static function b_i32_wrap(array $args, int $line) { return self::wrapped("i32", $args, $line); }

    const ERROR_KINDS = ["Error", "ValueError", "IndexError", "KeyError", "ZeroDivisionError", "OverflowError", "FileError"];
    static function b_Error(array $args, int $line) { return self::newError("Error", $args, $line); }
    static function b_ValueError(array $args, int $line) { return self::newError("ValueError", $args, $line); }
    static function b_IndexError(array $args, int $line) { return self::newError("IndexError", $args, $line); }
    static function b_KeyError(array $args, int $line) { return self::newError("KeyError", $args, $line); }
    static function b_ZeroDivisionError(array $args, int $line) { return self::newError("ZeroDivisionError", $args, $line); }
    static function b_OverflowError(array $args, int $line) { return self::newError("OverflowError", $args, $line); }
    static function b_FileError(array $args, int $line) { return self::newError("FileError", $args, $line); }

    // The function of the table that a name stands for, or null.
    private static $table = null;
    private static function tabled(string $name): ?string {
        if (preg_match('/^[A-Za-z0-9]+(\.[A-Za-z0-9]+)?$/D', $name) !== 1) return null;
        // PHP takes `B_PRINT` for `b_print`; Zeno does not.
        if (self::$table === null) self::$table = array_flip(get_class_methods(self::class));
        $method = "b_" . str_replace(".", "_", $name);
        return isset(self::$table[$method]) ? $method : null;
    }

    static function callBuiltin(string $name, array $args, int $line) {
        $method = self::tabled($name);
        if ($method === null) self::fatal("there is no `" . $name . "` in a program built for PHP", $line);
        return self::$method($args, $line);
    }

    // Something that comes with Zeno, as a value.
    private static $builtinValues = [];
    static function builtin(string $name, int $line = 0) {
        if ($name === "math.pi") return M_PI;
        if ($name === "math.e") return M_E;
        if (in_array($name, self::ERROR_KINDS, true)) return new ZErrorType($name);
        if (isset(self::$builtinValues[$name])) return self::$builtinValues[$name];
        if (self::tabled($name) === null) {
            $dot = strpos($name, ".");
            if ($dot !== false && $dot > 0) self::fatal("module `" . substr($name, 0, $dot) . "` has no `" . substr($name, $dot + 1) . "`", $line);
            self::fatal("`" . $name . "` is not defined", $line);
        }
        $dot = strpos($name, ".");
        $fn = new ZFn($dot === false ? $name : substr($name, $dot + 1), [], [], null, $name);
        self::$builtinValues[$name] = $fn;
        return $fn;
    }
    static function module(string $name) { return new ZModule($name); }

    // --------------------------------------------------------- The program

    // Run the program, and say what stopped it if something did.
    static function start(array $files, Closure $main) {
        // What PHP is set to do must not change what the program does:
        // no limit on memory, floats written with every digit, and a
        // warning from PHP is a fault in this file, not text to print.
        ini_set("memory_limit", "-1");
        ini_set("serialize_precision", "-1");
        error_reporting(E_ALL & ~E_DEPRECATED & ~E_USER_DEPRECATED);
        set_error_handler(static function (int $kind, string $message, string $file, int $line) {
            if ((error_reporting() & $kind) === 0) return false;
            throw new ErrorException($message, 0, $kind, $file, $line);
        });
        self::$started = hrtime(true);
        $where = static function (int $line) use ($files) {
            $file = $files[intdiv($line, self::LINES_A_FILE)] ?? $files[0];
            return " --> " . $file . ":" . ($line % self::LINES_A_FILE);
        };
        try {
            $main();
            self::flush();
            return;
        } catch (ZRaise $e) {
            $message = "error: uncaught " . self::shownName($e->value->kind) . ": " . self::show($e->value) . "\n" . $where($e->at);
        } catch (ZFatal $e) {
            $message = "error: " . $e->text . "\n" . $where($e->at);
        }
        self::flush();
        fwrite(STDERR, $message . "\n");
        exit(1);
    }
}

Z::$MISSING = new ZMissing();
