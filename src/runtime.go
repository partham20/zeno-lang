// The Zeno runtime for Go.
//
// A program built with `zeno build --target go` is this runtime followed
// by the program, in one file of `package main`. It holds what every Zeno
// value is, and what each operator and built-in does with it: what ops.rs
// and builtins.rs do for `zeno run`, step for step and message for
// message, so that the program prints what `zeno run` prints.
// runtime.js and runtime.py are the same for JavaScript and Python.
//
// How a Zeno value is held (every one is a ZV, which is Go's `any`):
//
//	int        int64, with every step checked for overflow
//	float      float64
//	str        string, in UTF-8; an index counts characters, not bytes
//	bool       bool
//	none       nil
//	list       *ZList
//	dict       *ZDict, which keeps the order the keys were put in
//	quantity   *ZQuantity: a number and its unit
//	function   *ZFunc
//	object     *ZInstance of a *ZClass
//	enum case  *ZCase of a *ZEnum
//	error      *ZError
//
// Every name here starts with Z, which no name of the program does.

package main

import (
	"bufio"
	"math"
	"math/bits"
	"os"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"
)

type ZV = any

const zLinesAFile = 1000000

type ZList struct{ items []ZV }

type ZDict struct {
	keys []ZV
	vals map[ZV]ZV
}

type ZQuantity struct {
	v    float64
	unit string
}

type ZFunc struct {
	name     string
	params   []string
	defaults []bool
	fn       func([]ZV) ZV
	// A function that comes with Zeno: its full name, and its body.
	builtin string
	bfn     func([]ZV, int) ZV
}

type ZClass struct {
	name     string
	fields   []string
	defaults []ZV
	methods  map[string]*ZFunc
}

type ZInstance struct {
	cls    *ZClass
	fields []ZV
}

type ZCaseDef struct {
	name   string
	fields []string
}

type ZEnum struct {
	name  string
	cases []ZCaseDef
}

type ZCase struct {
	def    *ZEnum
	index  int
	fields []ZV
}

type ZConstructor struct {
	def   *ZEnum
	index int
}

type ZErrorType struct{ kind string }

type ZError struct {
	kind, message string
	object        ZV
}

type ZModule struct{ name string }

// A method of a str, a list or a dict, read but not yet called.
type ZMethod struct {
	receiver ZV
	name     string
}

// A method of a class, read from an object.
type ZBound struct {
	object ZV
	fn     *ZFunc
}

// Several values from one function, as `return a, b` gives them, until
// `let a, b = ...` takes them apart (decision 57).
type ZSeveral struct{ values []ZV }

type zMissing struct{ _ byte }

// What a call gives a parameter it leaves out.
var ZMISSING = &zMissing{}

// An error the program may catch, on its way to a `try`.
type ZRaise struct {
	value *ZError
	line  int
}

// A mistake in the program, which nothing catches.
type ZFatal struct {
	message string
	line    int
}

func Zfatal(message string, line int) { panic(&ZFatal{message, line}) }

func zRaise(kind, message string, line int) {
	panic(&ZRaise{&ZError{kind, message, nil}, line})
}

func zOverflow(line int) { zRaise("OverflowError", "the result is too big for an int", line) }
func zByZero(line int)   { zRaise("ZeroDivisionError", "division by zero", line) }

// The number inside an int or a float, as a float.
func zNumber(v ZV) (float64, bool) {
	switch x := v.(type) {
	case int64:
		return float64(x), true
	case float64:
		return x, true
	case float32:
		return float64(x), true
	case uint64:
		return float64(x), true
	}
	return 0, false
}

// ---- u64 (decision 42): a Go uint64.

const zU64TooBig = "the result is too big for a u64"

// A u64, or an int of 0 or more, as a uint64.
func zUnsigned(v ZV) (uint64, bool) {
	switch x := v.(type) {
	case uint64:
		return x, true
	case int64:
		if x >= 0 {
			return uint64(x), true
		}
	}
	return 0, false
}

func zIsU64(a, b ZV) bool {
	_, x := a.(uint64)
	_, y := b.(uint64)
	return x || y
}

func zU64ShiftCount(n ZV, line int) uint {
	switch c := n.(type) {
	case uint64:
		if c >= 64 {
			zRaise("ValueError", "cannot shift by "+strconv.FormatUint(c, 10)+". A u64 has 64 bits, so a shift is 0 to 63", line)
		}
		return uint(c)
	case int64:
		if c < 0 || c >= 64 {
			zRaise("ValueError", "cannot shift by "+strconv.FormatInt(c, 10)+". A u64 has 64 bits, so a shift is 0 to 63", line)
		}
		return uint(c)
	}
	return 0
}

// `a op b` where one side is a u64 (decision 42).
func zU64Op(op string, a, b ZV, line int) ZV {
	if x, ok := a.(uint64); ok && (op == "<<" || op == ">>") {
		switch b.(type) {
		case int64, uint64:
			n := zU64ShiftCount(b, line)
			if op == ">>" {
				return x >> n
			}
			if (x<<n)>>n != x {
				zRaise("OverflowError", zU64TooBig, line)
			}
			return x << n
		}
	}
	x, ok1 := zUnsigned(a)
	y, ok2 := zUnsigned(b)
	if !ok1 || !ok2 {
		Zfatal("`"+op+"` between "+ZtypeName(a)+" and "+ZtypeName(b)+" would have to choose between a u64 and something else. Write `u64( )` or `int( )` around one of them", line)
	}
	switch op {
	case "+":
		r, carry := bits.Add64(x, y, 0)
		if carry != 0 {
			zRaise("OverflowError", zU64TooBig, line)
		}
		return r
	case "-":
		if y > x {
			zRaise("OverflowError", "the result is below 0, and a u64 never is", line)
		}
		return x - y
	case "*":
		hi, lo := bits.Mul64(x, y)
		if hi != 0 {
			zRaise("OverflowError", zU64TooBig, line)
		}
		return lo
	case "/":
		if y == 0 {
			zByZero(line)
		}
		return float64(x) / float64(y)
	case "//", "%":
		if y == 0 {
			zByZero(line)
		}
		if op == "//" {
			return x / y
		}
		return x % y
	case "**":
		if y == 0 {
			return uint64(1)
		}
		if x < 2 {
			return x
		}
		r := uint64(1)
		for i := uint64(0); i < y; i++ {
			hi, lo := bits.Mul64(r, x)
			if hi != 0 {
				zRaise("OverflowError", zU64TooBig, line)
			}
			r = lo
		}
		return r
	case "&":
		return x & y
	case "|":
		return x | y
	}
	return x ^ y
}

func Zb_u64(args []ZV, line int) ZV {
	zArity("u64", args, 1, 1, line)
	switch v := args[0].(type) {
	case uint64:
		return v
	case int64:
		if v < 0 {
			zRaise("OverflowError", strconv.FormatInt(v, 10)+" does not fit in a u64 (0 to 18446744073709551615)", line)
		}
		return uint64(v)
	}
	zWrongType("u64", "a whole number", args[0], line)
	return nil
}

// `u64.wrap(n)`: the 64 bits of an int read with no sign (42.6).
func Zb_u64__of(args []ZV, line int) ZV {
	zArity("u64.wrap", args, 1, 1, line)
	switch v := args[0].(type) {
	case uint64:
		return v
	case int64:
		return uint64(v)
	}
	zWrongType("u64.wrap", "a whole number", args[0], line)
	return nil
}

// `u64.wrap(a + b)`: the answer cut to 64 bits (42.6).
func zU64Wrapping(op string, args []ZV, line int) ZV {
	zArity("u64.wrap", args, 2, 2, line)
	x, ok1 := zUnsigned(args[0])
	if op == "<<" && ok1 {
		switch args[1].(type) {
		case int64, uint64:
			return x << zU64ShiftCount(args[1], line)
		}
	}
	y, ok2 := zUnsigned(args[1])
	if !ok1 {
		zWrongType("u64.wrap", "u64s", args[0], line)
	}
	if !ok2 {
		zWrongType("u64.wrap", "u64s", args[1], line)
	}
	switch op {
	case "+":
		return x + y
	case "-":
		return x - y
	}
	return x * y
}

func Zb_u64__add(args []ZV, line int) ZV { return zU64Wrapping("+", args, line) }
func Zb_u64__sub(args []ZV, line int) ZV { return zU64Wrapping("-", args, line) }
func Zb_u64__mul(args []ZV, line int) ZV { return zU64Wrapping("*", args, line) }
func Zb_u64__shl(args []ZV, line int) ZV { return zU64Wrapping("<<", args, line) }

// The nearest f32 to a float, as a float (decision 41).
func zFround(x float64) float64 { return float64(float32(x)) }

// An f32, or an int as a float and then the nearest f32 (41.2).
func zSingle(v ZV) (float64, bool) {
	switch x := v.(type) {
	case float32:
		return float64(x), true
	case int64:
		return zFround(float64(x)), true
	}
	return 0, false
}

// The float with the fewest digits that reads back as this f32.
func zF32Shown(f float32) float64 {
	x := float64(f)
	if x != x || x == 0 || math.IsInf(x, 0) {
		return x
	}
	for digits := 1; digits <= 9; digits++ {
		d, _ := strconv.ParseFloat(strconv.FormatFloat(x, 'e', digits-1, 64), 64)
		if zFround(d) == x {
			return d
		}
	}
	return x
}

func zIsF32(a, b ZV) bool {
	_, x := a.(float32)
	_, y := b.(float32)
	return x || y
}

func zF32Op(op string, a, b ZV, line int) ZV {
	x, ok1 := zSingle(a)
	y, ok2 := zSingle(b)
	if !ok1 || !ok2 {
		_, fa := a.(float64)
		_, fb := b.(float64)
		if fa || fb {
			Zfatal("`"+op+"` between "+ZtypeName(a)+" and "+ZtypeName(b)+" would have to choose between 32 bits and 64. Write `f32( )` around the float to work in 32, or `float( )` around the f32 to work in 64", line)
		}
		zMismatch(op, a, b, line)
	}
	var r float64
	switch op {
	case "+":
		r = x + y
	case "-":
		r = x - y
	case "*":
		r = x * y
	case "/":
		r = Zdiv(x, y, line).(float64)
	case "//":
		r = Zfloordiv(x, y, line).(float64)
	case "%":
		r = Zmod(x, y, line).(float64)
	case "**":
		r = Zpow(x, y, line).(float64)
	}
	return float32(r)
}

func ZtypeName(v ZV) string {
	switch x := v.(type) {
	case int64:
		return "an int"
	case float64:
		return "a float"
	case float32:
		return "an f32"
	case uint64:
		return "a u64"
	case string:
		return "a str"
	case bool:
		return "a bool"
	case nil:
		return "none"
	case *ZQuantity:
		return "a quantity in " + x.unit
	case *ZList:
		return "a list"
	case *ZDict:
		return "a dict"
	case *ZFunc, *ZMethod, *ZBound:
		return "a function"
	case *ZClass:
		return "a class"
	case *ZInstance:
		return "a " + x.cls.name
	case *ZEnum:
		return "an enum"
	case *ZConstructor:
		return "an enum case"
	case *ZCase:
		return "a " + x.def.name
	case *ZErrorType:
		return "an error type"
	case *ZError:
		return "a " + x.kind
	case *ZModule:
		return "a module"
	case *ZSeveral:
		return strconv.Itoa(len(x.values)) + " values"
	}
	return "a value"
}

// `Point`, not `shapes.Point`; `Stack`, not `Stack[int]`.
func zShownName(name string) string {
	if i := strings.IndexByte(name, '['); i >= 0 {
		name = name[:i]
	}
	if i := strings.LastIndexByte(name, '.'); i >= 0 {
		name = name[i+1:]
	}
	return name
}

// ------------------------------------------------------------- Text

// The characters of a str. The last one asked for is kept, so that a
// loop over the indexes of a str does not take it apart each time.
var zLastText string
var zLastRunes []rune
var zLastASCII bool

func zRunes(s string) ([]rune, bool) {
	if s == zLastText && (zLastRunes != nil || zLastASCII) {
		return zLastRunes, zLastASCII
	}
	ascii := true
	for i := 0; i < len(s); i++ {
		if s[i] >= 0x80 {
			ascii = false
			break
		}
	}
	zLastText, zLastASCII = s, ascii
	if ascii {
		zLastRunes = nil
	} else {
		zLastRunes = []rune(s)
	}
	return zLastRunes, ascii
}

func zLen(s string) int {
	runes, ascii := zRunes(s)
	if ascii {
		return len(s)
	}
	return len(runes)
}

// Characters from..to of a str.
func zSub(s string, from, to int) string {
	runes, ascii := zRunes(s)
	if ascii {
		return s[from:to]
	}
	return string(runes[from:to])
}

// ------------------------------------------------------------- Showing

// A float as Rust's `{:?}` shows it: the shortest digits that read back
// as the same number, `.0` on a whole one, and an exponent for one below
// 1e-4 or from 1e16 up.
func ZshowFloat(x float64) string {
	if math.IsNaN(x) {
		return "NaN"
	}
	if math.IsInf(x, 1) {
		return "inf"
	}
	if math.IsInf(x, -1) {
		return "-inf"
	}
	if x == 0 {
		if math.Signbit(x) {
			return "-0.0"
		}
		return "0.0"
	}
	s := strconv.FormatFloat(x, 'e', -1, 64)
	sign := ""
	if s[0] == '-' {
		sign, s = "-", s[1:]
	}
	at := strings.IndexByte(s, 'e')
	exp, _ := strconv.Atoi(s[at+1:])
	digits := strings.Replace(s[:at], ".", "", 1)
	if exp < -4 || exp >= 16 {
		if len(digits) > 1 {
			return sign + digits[:1] + "." + digits[1:] + "e" + strconv.Itoa(exp)
		}
		return sign + digits + "e" + strconv.Itoa(exp)
	}
	if exp < 0 {
		return sign + "0." + strings.Repeat("0", -exp-1) + digits
	}
	if len(digits) <= exp+1 {
		return sign + digits + strings.Repeat("0", exp+1-len(digits)) + ".0"
	}
	return sign + digits[:exp+1] + "." + digits[exp+1:]
}

// A float with a fixed number of decimals, as Rust's `{:.2}` writes it:
// from its exact value, a half rounding to the even digit.
func zFixed(x float64, places int) string {
	if math.IsNaN(x) {
		return "NaN"
	}
	if math.IsInf(x, 0) {
		if x > 0 {
			return "inf"
		}
		return "-inf"
	}
	return strconv.FormatFloat(x, 'f', places, 64)
}

// How Rust's `{:?}` shows a str: in quotes, with what cannot be seen escaped.
func zQuoted(s string) string {
	var b strings.Builder
	b.WriteByte('"')
	for _, c := range s {
		switch {
		case c == '"':
			b.WriteString("\\\"")
		case c == '\\':
			b.WriteString("\\\\")
		case c == '\n':
			b.WriteString("\\n")
		case c == '\r':
			b.WriteString("\\r")
		case c == '\t':
			b.WriteString("\\t")
		case c == 0:
			b.WriteString("\\0")
		case c < 0x20 || (c >= 0x7f && c < 0xa0):
			b.WriteString("\\u{" + strconv.FormatInt(int64(c), 16) + "}")
		default:
			b.WriteRune(c)
		}
	}
	b.WriteByte('"')
	return b.String()
}

// How `print`, `str()` and f-strings show a value.
func Zshow(v ZV) string {
	switch x := v.(type) {
	case string:
		return x
	case int64:
		return strconv.FormatInt(x, 10)
	case float64:
		return ZshowFloat(x)
	case float32:
		return ZshowFloat(zF32Shown(x))
	case uint64:
		return strconv.FormatUint(x, 10)
	case bool:
		if x {
			return "true"
		}
		return "false"
	case nil:
		return "none"
	case *ZList:
		parts := make([]string, len(x.items))
		for i, item := range x.items {
			parts[i] = zRepr(item)
		}
		return "[" + strings.Join(parts, ", ") + "]"
	case *ZDict:
		if len(x.keys) == 0 {
			return "[:]"
		}
		parts := make([]string, len(x.keys))
		for i, k := range x.keys {
			parts[i] = zRepr(k) + ": " + zRepr(x.vals[k])
		}
		return "[" + strings.Join(parts, ", ") + "]"
	case *ZQuantity:
		if !math.IsNaN(x.v) && !math.IsInf(x.v, 0) && x.v == math.Trunc(x.v) && math.Abs(x.v) < 1e15 {
			return strconv.FormatInt(int64(x.v), 10) + " " + x.unit
		}
		return ZshowFloat(x.v) + " " + x.unit
	case *ZInstance:
		parts := make([]string, len(x.cls.fields))
		for i, name := range x.cls.fields {
			parts[i] = name + "=" + zRepr(x.fields[i])
		}
		return zShownName(x.cls.name) + "(" + strings.Join(parts, ", ") + ")"
	case *ZCase:
		name := zShownName(x.def.cases[x.index].name)
		if len(x.fields) == 0 {
			return name
		}
		parts := make([]string, len(x.fields))
		for i, f := range x.fields {
			parts[i] = zRepr(f)
		}
		return name + "(" + strings.Join(parts, ", ") + ")"
	case *ZError:
		return x.message
	case *ZMethod:
		return "<function " + x.name + ">"
	case *ZBound:
		return "<function " + x.fn.name + ">"
	case *ZClass:
		return "<class " + x.name + ">"
	case *ZEnum:
		return "<enum " + x.name + ">"
	case *ZConstructor:
		return "<case " + x.def.cases[x.index].name + ">"
	case *ZErrorType:
		return "<error " + x.kind + ">"
	case *ZModule:
		return "<module " + x.name + ">"
	case *ZSeveral:
		parts := make([]string, len(x.values))
		for i, item := range x.values {
			parts[i] = Zshow(item)
		}
		return "<" + strconv.Itoa(len(x.values)) + " values: " + strings.Join(parts, ", ") + ">"
	case *ZFunc:
		if x.name != "" {
			return "<function " + x.name + ">"
		}
		return "<function>"
	}
	return "?"
}

func zRepr(v ZV) string {
	if s, ok := v.(string); ok {
		return zQuoted(s)
	}
	return Zshow(v)
}

// --------------------------------------------------------------- Units

type zUnit struct {
	kind string
	size float64
}

var zUnits = map[string]zUnit{
	"ns": {"time", 1}, "us": {"time", 1e3}, "ms": {"time", 1e6}, "s": {"time", 1e9},
	"min": {"time", 60e9}, "h": {"time", 3600e9},
	"Hz": {"frequency", 1}, "kHz": {"frequency", 1e3}, "MHz": {"frequency", 1e6},
	"uV": {"voltage", 1}, "mV": {"voltage", 1e3}, "V": {"voltage", 1e6}, "kV": {"voltage", 1e9},
	"uA": {"current", 1}, "mA": {"current", 1e3}, "A": {"current", 1e6},
	"ohm": {"resistance", 1}, "kohm": {"resistance", 1e3}, "Mohm": {"resistance", 1e6},
	"mW": {"power", 1}, "W": {"power", 1e3}, "kW": {"power", 1e6},
	"J": {"energy", 1}, "Wh": {"energy", 3600}, "kWh": {"energy", 3600e3},
	"degC": {"temperature in Celsius", 1}, "K": {"temperature in kelvin", 1},
	"mm": {"length", 1}, "cm": {"length", 10}, "m": {"length", 1e3}, "km": {"length", 1e6},
	"mg": {"mass", 1}, "g": {"mass", 1e3}, "kg": {"mass", 1e6},
	"deg": {"angle in degrees", 1}, "rad": {"angle in radians", 1}, "rpm": {"rotation", 1},
}

type zScale struct{ times, over float64 }

func zConvert(from, to string) (zScale, bool) {
	if from == "deg" && to == "rad" {
		return zScale{math.Pi / 180, 1}, true
	}
	if from == "rad" && to == "deg" {
		return zScale{180 / math.Pi, 1}, true
	}
	f, ok1 := zUnits[from]
	t, ok2 := zUnits[to]
	if !ok1 || !ok2 || f.kind != t.kind {
		return zScale{}, false
	}
	if f.size >= t.size {
		return zScale{f.size / t.size, 1}, true
	}
	return zScale{1, t.size / f.size}, true
}

func zApply(s zScale, x float64) float64 { return x * s.times / s.over }

// The unit two quantities meet in: the finer one.
func zCommon(a, b string) (string, zScale, zScale, bool) {
	ua, ok1 := zUnits[a]
	ub, ok2 := zUnits[b]
	if !ok1 || !ok2 || ua.kind != ub.kind {
		return "", zScale{}, zScale{}, false
	}
	unit := b
	if ua.size <= ub.size {
		unit = a
	}
	sa, _ := zConvert(a, unit)
	sb, _ := zConvert(b, unit)
	return unit, sa, sb, true
}

// What `*` and `/` make of two quantities (decision 43): the operator,
// what each side measures ("" for a plain number), and what the answer
// measures ("" for a plain number). The answer is in the unit of its
// kind with no prefix.
var zMade = map[string]string{
	"* voltage current": "power", "* current voltage": "power",
	"* current resistance": "voltage", "* resistance current": "voltage",
	"* power time": "energy", "* time power": "energy",
	"* time frequency": "", "* frequency time": "",
	"/ power voltage": "current", "/ power current": "voltage",
	"/ voltage current": "resistance", "/ voltage resistance": "current",
	"/ energy time": "power", "/ energy power": "time",
	"/  time": "frequency", "/  frequency": "time",
}

var zPlain = map[string]string{"time": "s", "frequency": "Hz", "voltage": "V", "current": "A", "resistance": "ohm", "power": "W", "energy": "J"}

// `a op b` for two quantities, or a number over a quantity.
func zMadeOf(op string, a ZV, b *ZQuantity, line int) ZV {
	left := ""
	var x float64
	ok := true
	if q, isQ := a.(*ZQuantity); isQ {
		left = zUnits[q.unit].kind
		scale, _ := zConvert(q.unit, zPlain[left])
		x = zApply(scale, q.v)
	} else {
		x, ok = zNumber(a)
	}
	right := zUnits[b.unit].kind
	kind, known := zMade[op+" "+left+" "+right]
	if !known || !ok {
		zMismatch(op, a, b, line)
	}
	if op == "/" && b.v == 0 {
		zByZero(line)
	}
	scale, _ := zConvert(b.unit, zPlain[right])
	y := zApply(scale, b.v)
	r := x * y
	if op == "/" {
		r = x / y
	}
	if kind == "" {
		return r
	}
	return &ZQuantity{r, zPlain[kind]}
}

func Zunit(v ZV, name string, line int) ZV {
	if _, ok := zUnits[name]; !ok {
		Zfatal("`"+name+"` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC", line)
	}
	x, ok := zNumber(v)
	if !ok {
		Zfatal("a unit can only follow a number", line)
	}
	return &ZQuantity{x, name}
}

// ------------------------------------------------------------ Equality

func zEqual(a, b ZV) bool {
	if zIsU64(a, b) {
		x, ok1 := zUnsigned(a)
		y, ok2 := zUnsigned(b)
		return ok1 && ok2 && x == y
	}
	if zIsF32(a, b) {
		x, ok1 := zSingle(a)
		y, ok2 := zSingle(b)
		if !ok1 || !ok2 {
			x, ok1 = zNumber(a)
			y, ok2 = zNumber(b)
		}
		return ok1 && ok2 && x == y
	}
	switch x := a.(type) {
	case int64:
		switch y := b.(type) {
		case int64:
			return x == y
		case float64:
			return float64(x) == y
		}
		return false
	case float64:
		switch y := b.(type) {
		case int64:
			return x == float64(y)
		case float64:
			return x == y
		}
		return false
	case string:
		y, ok := b.(string)
		return ok && x == y
	case bool:
		y, ok := b.(bool)
		return ok && x == y
	case nil:
		return b == nil
	case *ZQuantity:
		y, ok := b.(*ZQuantity)
		if !ok {
			return false
		}
		_, sa, sb, ok := zCommon(x.unit, y.unit)
		return ok && zApply(sa, x.v) == zApply(sb, y.v)
	case *ZList:
		y, ok := b.(*ZList)
		if !ok || len(x.items) != len(y.items) {
			return false
		}
		for i := range x.items {
			if !zEqual(x.items[i], y.items[i]) {
				return false
			}
		}
		return true
	case *ZDict:
		y, ok := b.(*ZDict)
		if !ok || len(x.keys) != len(y.keys) {
			return false
		}
		for _, k := range x.keys {
			other, has := y.vals[k]
			if !has || !zEqual(x.vals[k], other) {
				return false
			}
		}
		return true
	case *ZInstance:
		y, ok := b.(*ZInstance)
		if !ok || x.cls != y.cls {
			return false
		}
		for i := range x.fields {
			if !zEqual(x.fields[i], y.fields[i]) {
				return false
			}
		}
		return true
	case *ZCase:
		y, ok := b.(*ZCase)
		if !ok || x.def != y.def || x.index != y.index || len(x.fields) != len(y.fields) {
			return false
		}
		for i := range x.fields {
			if !zEqual(x.fields[i], y.fields[i]) {
				return false
			}
		}
		return true
	case *ZError:
		y, ok := b.(*ZError)
		return ok && x.kind == y.kind && x.message == y.message
	case *ZConstructor:
		y, ok := b.(*ZConstructor)
		return ok && x.def == y.def && x.index == y.index
	}
	return a == b
}

func Zeq(a, b ZV) bool { return zEqual(a, b) }
func Zne(a, b ZV) bool { return !zEqual(a, b) }

// ----------------------------------------------------------- Operators

func zMismatch(op string, a, b ZV, line int) {
	Zfatal("`"+op+"` does not work between "+ZtypeName(a)+" and "+ZtypeName(b), line)
}

func zAddInt(a, b int64, line int) int64 {
	r := a + b
	if (a >= 0) == (b >= 0) && (r >= 0) != (a >= 0) {
		zOverflow(line)
	}
	return r
}

func zSubInt(a, b int64, line int) int64 {
	r := a - b
	if (a >= 0) != (b >= 0) && (r >= 0) != (a >= 0) {
		zOverflow(line)
	}
	return r
}

func zMulInt(a, b int64, line int) int64 {
	if a == 0 || b == 0 {
		return 0
	}
	r := a * b
	if r/b != a || (a == -1 && b == math.MinInt64) || (b == -1 && a == math.MinInt64) {
		zOverflow(line)
	}
	return r
}

func Zadd(a, b ZV, line int) ZV {
	if zIsU64(a, b) {
		return zU64Op("+", a, b, line)
	}
	if zIsF32(a, b) {
		return zF32Op("+", a, b, line)
	}
	switch x := a.(type) {
	case int64:
		if y, ok := b.(int64); ok {
			return zAddInt(x, y, line)
		}
	case string:
		if y, ok := b.(string); ok {
			return x + y
		}
	case *ZList:
		if y, ok := b.(*ZList); ok {
			items := make([]ZV, 0, len(x.items)+len(y.items))
			items = append(items, x.items...)
			return &ZList{append(items, y.items...)}
		}
	case *ZQuantity:
		if y, ok := b.(*ZQuantity); ok {
			unit, sa, sb, ok := zCommon(x.unit, y.unit)
			if !ok {
				zMismatch("+", a, b, line)
			}
			return &ZQuantity{zApply(sa, x.v) + zApply(sb, y.v), unit}
		}
	}
	x, ok1 := zNumber(a)
	y, ok2 := zNumber(b)
	if !ok1 || !ok2 {
		zMismatch("+", a, b, line)
	}
	return x + y
}

func Zsub(a, b ZV, line int) ZV {
	if zIsU64(a, b) {
		return zU64Op("-", a, b, line)
	}
	if zIsF32(a, b) {
		return zF32Op("-", a, b, line)
	}
	switch x := a.(type) {
	case int64:
		if y, ok := b.(int64); ok {
			return zSubInt(x, y, line)
		}
	case *ZQuantity:
		if y, ok := b.(*ZQuantity); ok {
			unit, sa, sb, ok := zCommon(x.unit, y.unit)
			if !ok {
				zMismatch("-", a, b, line)
			}
			return &ZQuantity{zApply(sa, x.v) - zApply(sb, y.v), unit}
		}
	}
	x, ok1 := zNumber(a)
	y, ok2 := zNumber(b)
	if !ok1 || !ok2 {
		zMismatch("-", a, b, line)
	}
	return x - y
}

func Zmul(a, b ZV, line int) ZV {
	if zIsU64(a, b) {
		return zU64Op("*", a, b, line)
	}
	if zIsF32(a, b) {
		return zF32Op("*", a, b, line)
	}
	if list, ok := a.(*ZList); ok {
		if times, ok := b.(int64); ok {
			if times < 0 {
				zRaise("ValueError", "`*` repeats a list 0 or more times, and this is "+strconv.FormatInt(times, 10), line)
			}
			items := make([]ZV, 0, int64(len(list.items))*times)
			for t := int64(0); t < times; t++ {
				items = append(items, list.items...)
			}
			return &ZList{items}
		}
	}
	if x, ok := a.(int64); ok {
		if y, ok := b.(int64); ok {
			return zMulInt(x, y, line)
		}
	}
	if q, isQ := b.(*ZQuantity); isQ {
		if _, both := a.(*ZQuantity); both {
			return zMadeOf("*", a, q, line)
		}
	}
	q, isQ := a.(*ZQuantity)
	other := b
	if !isQ {
		q, isQ = b.(*ZQuantity)
		other = a
	}
	if isQ {
		y, ok := zNumber(other)
		if !ok {
			zMismatch("*", a, b, line)
		}
		return &ZQuantity{q.v * y, q.unit}
	}
	x, ok1 := zNumber(a)
	y, ok2 := zNumber(b)
	if !ok1 || !ok2 {
		zMismatch("*", a, b, line)
	}
	return x * y
}

func Zdiv(a, b ZV, line int) ZV {
	if zIsU64(a, b) {
		return zU64Op("/", a, b, line)
	}
	if zIsF32(a, b) {
		return zF32Op("/", a, b, line)
	}
	if q, ok := a.(*ZQuantity); ok {
		switch y := b.(type) {
		case *ZQuantity:
			if y.v == 0 {
				zByZero(line)
			}
			_, sa, sb, ok := zCommon(q.unit, y.unit)
			if !ok {
				return zMadeOf("/", a, y, line)
			}
			return zApply(sa, q.v) / zApply(sb, y.v)
		case float64:
			if y == 0 {
				zByZero(line)
			}
		case int64:
			if y == 0 {
				zByZero(line)
			}
		}
		y, ok := zNumber(b)
		if !ok {
			zMismatch("/", a, b, line)
		}
		return &ZQuantity{q.v / y, q.unit}
	}
	if q, ok := b.(*ZQuantity); ok {
		return zMadeOf("/", a, q, line)
	}
	x, ok1 := zNumber(a)
	y, ok2 := zNumber(b)
	if !ok1 || !ok2 {
		zMismatch("/", a, b, line)
	}
	if y == 0 {
		zByZero(line)
	}
	return x / y
}

func Zfloordiv(a, b ZV, line int) ZV {
	if zIsU64(a, b) {
		return zU64Op("//", a, b, line)
	}
	if zIsF32(a, b) {
		return zF32Op("//", a, b, line)
	}
	if x, ok := a.(int64); ok {
		if y, ok := b.(int64); ok {
			if y == 0 {
				zByZero(line)
			}
			if x == math.MinInt64 && y == -1 {
				zOverflow(line)
			}
			q := x / y
			if x%y != 0 && (x < 0) != (y < 0) {
				q--
			}
			return q
		}
	}
	x, ok1 := zNumber(a)
	y, ok2 := zNumber(b)
	if !ok1 || !ok2 {
		zMismatch("//", a, b, line)
	}
	if y == 0 {
		zByZero(line)
	}
	return math.Floor(x / y)
}

func Zmod(a, b ZV, line int) ZV {
	if zIsU64(a, b) {
		return zU64Op("%", a, b, line)
	}
	if zIsF32(a, b) {
		return zF32Op("%", a, b, line)
	}
	if x, ok := a.(int64); ok {
		if y, ok := b.(int64); ok {
			if y == 0 {
				zByZero(line)
			}
			if y == -1 {
				return int64(0)
			}
			r := x % y
			if r != 0 && (r < 0) != (y < 0) {
				r += y
			}
			return r
		}
	}
	x, ok1 := zNumber(a)
	y, ok2 := zNumber(b)
	if !ok1 || !ok2 {
		zMismatch("%", a, b, line)
	}
	if y == 0 {
		zByZero(line)
	}
	q := x / y
	// float64(...) rounds the product before it is taken away. Without
	// it Go may do both in one step where the processor can (ARM64),
	// and `7.0 % 0.1` would be -3.885780586188048e-16, not 0.0.
	if math.IsNaN(q) || math.IsInf(q, 0) {
		return x - float64(y*q)
	}
	return x - float64(y*math.Floor(q))
}

func Zpow(a, b ZV, line int) ZV {
	if zIsU64(a, b) {
		return zU64Op("**", a, b, line)
	}
	if zIsF32(a, b) {
		return zF32Op("**", a, b, line)
	}
	if x, ok := a.(int64); ok {
		if n, ok := b.(int64); ok {
			if n < 0 {
				zRaise("ValueError", "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`", line)
			}
			if n > 4294967295 {
				zOverflow(line)
			}
			if x == 0 || x == 1 {
				if n == 0 {
					return int64(1)
				}
				return x
			}
			if x == -1 {
				if n%2 == 0 {
					return int64(1)
				}
				return int64(-1)
			}
			if n >= 64 {
				zOverflow(line)
			}
			r := int64(1)
			for i := int64(0); i < n; i++ {
				r = zMulInt(r, x, line)
			}
			return r
		}
	}
	x, ok1 := zNumber(a)
	y, ok2 := zNumber(b)
	if !ok1 || !ok2 {
		zMismatch("**", a, b, line)
	}
	if x == 1 || y == 0 {
		return 1.0
	}
	return math.Pow(x, y)
}

func zBitsMismatch(op string, a, b ZV, line int) {
	hint := ""
	_, ba := a.(bool)
	_, bb := b.(bool)
	if ba && bb {
		if op == "&" {
			hint = ". For two bools Zeno spells this `and`"
		}
		if op == "|" {
			hint = ". For two bools Zeno spells this `or`"
		}
	}
	Zfatal("`"+op+"` works on the bits of whole numbers, and these are "+ZtypeName(a)+" and "+ZtypeName(b)+hint, line)
}

func zInts(op string, a, b ZV, line int) (int64, int64) {
	x, ok1 := a.(int64)
	y, ok2 := b.(int64)
	if !ok1 || !ok2 {
		zBitsMismatch(op, a, b, line)
	}
	return x, y
}

func Zbitand(a, b ZV, line int) ZV {
	if zIsU64(a, b) {
		return zU64Op("&", a, b, line)
	}
	x, y := zInts("&", a, b, line)
	return x & y
}

func Zbitor(a, b ZV, line int) ZV {
	if zIsU64(a, b) {
		return zU64Op("|", a, b, line)
	}
	x, y := zInts("|", a, b, line)
	return x | y
}

func Zbitxor(a, b ZV, line int) ZV {
	if zIsU64(a, b) {
		return zU64Op("^", a, b, line)
	}
	x, y := zInts("^", a, b, line)
	return x ^ y
}

func zShift(n int64, line int) {
	if n < 0 || n >= 64 {
		zRaise("ValueError", "cannot shift by "+strconv.FormatInt(n, 10)+". An int has 64 bits, so a shift is 0 to 63", line)
	}
}

func Zshl(a, b ZV, line int) ZV {
	if zIsU64(a, b) {
		return zU64Op("<<", a, b, line)
	}
	x, n := zInts("<<", a, b, line)
	zShift(n, line)
	s := x << uint(n)
	if s>>uint(n) != x {
		zOverflow(line)
	}
	return s
}

func Zshr(a, b ZV, line int) ZV {
	if zIsU64(a, b) {
		return zU64Op(">>", a, b, line)
	}
	x, n := zInts(">>", a, b, line)
	zShift(n, line)
	return x >> uint(n)
}

func Zneg(v ZV, line int) ZV {
	switch x := v.(type) {
	case int64:
		if x == math.MinInt64 {
			zOverflow(line)
		}
		return -x
	case float64:
		return -x
	case float32:
		return -x
	case uint64:
		Zfatal("`-` does not work on a u64, which is never below 0. Make it an int first with `int( )`", line)
	case *ZQuantity:
		return &ZQuantity{-x.v, x.unit}
	}
	Zfatal("`-` does not work on "+ZtypeName(v), line)
	return nil
}

func Zbitnot(v ZV, line int) ZV {
	if x, ok := v.(uint64); ok {
		return ^x
	}
	x, ok := v.(int64)
	if !ok {
		Zfatal("`~` works on the bits of a whole number, and this is "+ZtypeName(v), line)
	}
	return ^x
}

// -1, 0 or 1, or 2 when the two cannot be ordered (a NaN).
func zOrder(op string, a, b ZV, line int) int {
	if zIsU64(a, b) {
		x, ok1 := zUnsigned(a)
		y, ok2 := zUnsigned(b)
		if !ok1 || !ok2 {
			zU64Op(op, a, b, line)
		}
		if x < y {
			return -1
		}
		if x > y {
			return 1
		}
		return 0
	}
	if zIsF32(a, b) {
		x, ok1 := zSingle(a)
		y, ok2 := zSingle(b)
		if !ok1 || !ok2 {
			zF32Op(op, a, b, line)
		}
		if x < y {
			return -1
		}
		if x > y {
			return 1
		}
		if x == y {
			return 0
		}
		return 2
	}
	switch x := a.(type) {
	case int64:
		if y, ok := b.(int64); ok {
			if x < y {
				return -1
			}
			if x > y {
				return 1
			}
			return 0
		}
	case string:
		if y, ok := b.(string); ok {
			return strings.Compare(x, y)
		}
	}
	var x, y float64
	qa, ok1 := a.(*ZQuantity)
	qb, ok2 := b.(*ZQuantity)
	if ok1 && ok2 {
		_, sa, sb, ok := zCommon(qa.unit, qb.unit)
		if !ok {
			zMismatch(op, a, b, line)
		}
		x, y = zApply(sa, qa.v), zApply(sb, qb.v)
	} else {
		var ok1, ok2 bool
		x, ok1 = zNumber(a)
		y, ok2 = zNumber(b)
		if !ok1 || !ok2 {
			zMismatch(op, a, b, line)
		}
	}
	if x < y {
		return -1
	}
	if x > y {
		return 1
	}
	if x == y {
		return 0
	}
	return 2
}

func Zlt(a, b ZV, line int) bool { return zOrder("<", a, b, line) == -1 }
func Zle(a, b ZV, line int) bool { o := zOrder("<=", a, b, line); return o == -1 || o == 0 }
func Zgt(a, b ZV, line int) bool { return zOrder(">", a, b, line) == 1 }
func Zge(a, b ZV, line int) bool { o := zOrder(">=", a, b, line); return o == 1 || o == 0 }

func zContains(op string, item, container ZV, line int) bool {
	switch c := container.(type) {
	case *ZList:
		for _, other := range c.items {
			if zEqual(item, other) {
				return true
			}
		}
		return false
	case *ZDict:
		switch item.(type) {
		case int64, string:
		default:
			zMismatch(op, item, container, line)
		}
		_, has := c.vals[item]
		return has
	case string:
		if s, ok := item.(string); ok {
			return strings.Contains(c, s)
		}
	}
	zMismatch(op, item, container, line)
	return false
}

func Zin(a, b ZV, line int) bool    { return zContains("in", a, b, line) }
func Znot_in(a, b ZV, line int) bool { return !zContains("not in", a, b, line) }

func Ztruth(v ZV, line int) bool {
	if b, ok := v.(bool); ok {
		return b
	}
	Zfatal("a condition must be true or false, but this is "+ZtypeName(v), line)
	return false
}

// ------------------------------------------------------ Lists and text

func zKey(v ZV, line int) ZV {
	switch v.(type) {
	case int64, string:
		return v
	}
	Zfatal("a key of a dict is a str or an int, but this is "+ZtypeName(v), line)
	return nil
}

func zPosition(index ZV, length int, what string, line int) int {
	i, ok := index.(int64)
	if !ok {
		Zfatal("an index must be an int, but this is "+ZtypeName(index), line)
	}
	if i < 0 || i >= int64(length) {
		zRaise("IndexError", "index "+strconv.FormatInt(i, 10)+" is out of range for a "+what+" of length "+strconv.Itoa(length), line)
	}
	return int(i)
}

func (d *ZDict) set(k, v ZV) {
	if _, has := d.vals[k]; !has {
		d.keys = append(d.keys, k)
	}
	d.vals[k] = v
}

func (d *ZDict) remove(k ZV) {
	delete(d.vals, k)
	for i, key := range d.keys {
		if key == k {
			d.keys = append(d.keys[:i:i], d.keys[i+1:]...)
			return
		}
	}
}

func zNewDict() *ZDict { return &ZDict{nil, map[ZV]ZV{}} }

func Zindex(obj, i ZV, line int) ZV {
	switch o := obj.(type) {
	case *ZList:
		return o.items[zPosition(i, len(o.items), "list", line)]
	case string:
		at := zPosition(i, zLen(o), "str", line)
		return zSub(o, at, at+1)
	case *ZDict:
		k := zKey(i, line)
		v, has := o.vals[k]
		if !has {
			zRaise("KeyError", "this dict has no key "+zRepr(i), line)
		}
		return v
	}
	Zfatal("cannot take an item from "+ZtypeName(obj), line)
	return nil
}

func Zset_index(obj, i, value ZV, line int) {
	switch o := obj.(type) {
	case *ZList:
		o.items[zPosition(i, len(o.items), "list", line)] = value
		return
	case *ZDict:
		o.set(zKey(i, line), value)
		return
	}
	Zfatal("cannot set an item of "+ZtypeName(obj), line)
}

func zChars(s string) []ZV {
	out := make([]ZV, 0, len(s))
	for _, c := range s {
		out = append(out, string(c))
	}
	return out
}

func Zitems(v ZV, line int) []ZV {
	switch x := v.(type) {
	case *ZList:
		return append([]ZV(nil), x.items...)
	case *ZDict:
		return append([]ZV(nil), x.keys...)
	case string:
		return zChars(x)
	}
	Zfatal("cannot loop over "+ZtypeName(v), line)
	return nil
}

func Zpairs(v ZV, line int) [][2]ZV {
	if d, ok := v.(*ZDict); ok {
		out := make([][2]ZV, len(d.keys))
		for i, k := range d.keys {
			out[i] = [2]ZV{k, d.vals[k]}
		}
		return out
	}
	items := Zitems(v, line)
	out := make([][2]ZV, len(items))
	for i, item := range items {
		out[i] = [2]ZV{int64(i), item}
	}
	return out
}

func Zlist(values []ZV) ZV { return &ZList{values} }

func Zdict(flat []ZV, line int) ZV {
	d := zNewDict()
	for i := 0; i < len(flat); i += 2 {
		d.set(zKey(flat[i], line), flat[i+1])
	}
	return d
}

func Zpush(list, value ZV) ZV {
	l := list.(*ZList)
	l.items = append(l.items, value)
	return nil
}

func Zconcat(pieces []ZV) ZV {
	var b strings.Builder
	for _, p := range pieces {
		b.WriteString(p.(string))
	}
	return b.String()
}

// ------------------------------------------------- Objects and fields

func Zfn(name string, params []string, defaults []bool, fn func([]ZV) ZV) *ZFunc {
	return &ZFunc{name: name, params: params, defaults: defaults, fn: fn}
}

func Zclass(name string, fields []string, defaults []ZV, methods map[string]*ZFunc) ZV {
	return &ZClass{name, fields, defaults, methods}
}

func Zenum(name string, cases []ZCaseDef) ZV { return &ZEnum{name, cases} }

func Zcase(def ZV, i int) ZV {
	e := def.(*ZEnum)
	if len(e.cases[i].fields) == 0 {
		return &ZCase{e, i, nil}
	}
	return &ZConstructor{e, i}
}

func Zis_case(value, ctor ZV, count int, line int) bool {
	c, ok := ctor.(*ZConstructor)
	if !ok {
		Zfatal("this is not an enum case that carries values", line)
	}
	v, ok := value.(*ZCase)
	if !ok || v.def != c.def || v.index != c.index {
		return false
	}
	if len(v.fields) != count {
		Zfatal("`"+c.def.cases[c.index].name+"` carries "+strconv.Itoa(len(v.fields))+" values, but this pattern has "+strconv.Itoa(count), line)
	}
	return true
}

func Zcase_field(value ZV, i int) ZV { return value.(*ZCase).fields[i] }

func Zseveral(values []ZV) ZV { return &ZSeveral{values} }

// What place `at` of the `count` on the left of `=` is given.
func Zreceived(value ZV, count, at int, line int) ZV {
	s, ok := value.(*ZSeveral)
	if !ok {
		Zfatal("there are "+strconv.Itoa(count)+" places on the left of `=`, and the right gives one value, "+ZtypeName(value), line)
	}
	if len(s.values) != count {
		Zfatal("this gives "+strconv.Itoa(len(s.values))+" values, and there are "+strconv.Itoa(count)+" places on the left of `=`", line)
	}
	return s.values[at]
}

func zIndexOf(names []string, name string) int {
	for i, n := range names {
		if n == name {
			return i
		}
	}
	return -1
}

func Zfield(obj ZV, name string, line int) ZV {
	switch o := obj.(type) {
	case *ZInstance:
		if at := zIndexOf(o.cls.fields, name); at >= 0 {
			return o.fields[at]
		}
		if m, ok := o.cls.methods[name]; ok {
			return &ZBound{o, m}
		}
		Zfatal("`"+o.cls.name+"` has no field or method `"+name+"`", line)
	case *ZCase:
		if at := zIndexOf(o.def.cases[o.index].fields, name); at >= 0 {
			return o.fields[at]
		}
		Zfatal("`"+o.def.cases[o.index].name+"` has no field `"+name+"`", line)
	case *ZEnum:
		for at, c := range o.cases {
			if c.name == name {
				return Zcase(o, at)
			}
		}
		Zfatal("enum `"+o.name+"` has no case `"+name+"`", line)
	case *ZModule:
		return Zbuiltin(o.name+"."+name, line)
	case *ZQuantity:
		if _, ok := zUnits[name]; ok {
			s, ok := zConvert(o.unit, name)
			if !ok {
				Zfatal("a quantity in "+o.unit+" cannot be turned into "+name+". They measure different things", line)
			}
			return &ZQuantity{zApply(s, o.v), name}
		}
	case *ZError:
		if name == "message" {
			return o.message
		}
		if name == "kind" {
			return o.kind
		}
	case *ZFunc:
		if _, sized := zWidths[o.builtin]; sized && name == "wrap" {
			return Zbuiltin(o.builtin+".wrap", line)
		}
	}
	if zHasMethod(obj, name) {
		return &ZMethod{obj, name}
	}
	Zfatal(ZtypeName(obj)+" has no field or method `"+name+"`", line)
	return nil
}

func Zset_field(obj ZV, name string, value ZV, line int) {
	if o, ok := obj.(*ZInstance); ok {
		at := zIndexOf(o.cls.fields, name)
		if at < 0 {
			Zfatal("`"+o.cls.name+"` has no field `"+name+"`", line)
		}
		o.fields[at] = value
		return
	}
	Zfatal("cannot set a field on "+ZtypeName(obj), line)
}

// ------------------------------------------------------------- Calls

func zCounted(n int, word string) string {
	if n == 1 {
		return "1 " + word
	}
	return strconv.Itoa(n) + " " + word + "s"
}

func zWere(n int) string {
	if n == 1 {
		return "1 was"
	}
	return strconv.Itoa(n) + " were"
}

func zBind(what string, names []string, positional []ZV, namedNames []string, named []ZV, line int) []ZV {
	if len(positional) > len(names) {
		Zfatal("`"+what+"` takes "+zCounted(len(names), "argument")+", but "+zWere(len(positional))+" given", line)
	}
	slots := make([]ZV, len(names))
	copy(slots, positional)
	for i := len(positional); i < len(names); i++ {
		slots[i] = ZMISSING
	}
	for i, name := range namedNames {
		at := zIndexOf(names, name)
		if at < 0 {
			Zfatal("`"+what+"` has no argument named `"+name+"`", line)
		}
		if slots[at] != ZMISSING {
			Zfatal("`"+what+"` was given `"+name+"` twice", line)
		}
		slots[at] = named[i]
	}
	return slots
}

// How many calls are nested now. `zeno run` stops at 1000.
var zDepth = 0

const zMaxDepth = 1000

func zEnter(f *ZFunc, line int) {
	if zDepth >= zMaxDepth {
		name := f.name
		if name == "" {
			name = "function"
		}
		Zfatal("more than 1000 calls are nested. Is `"+name+"` calling itself forever?", line)
	}
	zDepth++
}

// A call of a function of the program known where it is written, with
// its arguments lined up already.
func Zdirect(f ZV, line int, args ...ZV) ZV {
	fn := f.(*ZFunc)
	zEnter(fn, line)
	r := fn.fn(args)
	zDepth--
	return r
}

func zCallFunction(f *ZFunc, positional []ZV, namedNames []string, named []ZV, line int) ZV {
	name := f.name
	if name == "" {
		name = "function"
	}
	zEnter(f, line)
	slots := zBind(name, f.params, positional, namedNames, named, line)
	for i, slot := range slots {
		if slot == ZMISSING && !f.defaults[i] {
			Zfatal("`"+name+"` needs a value for `"+f.params[i]+"`", line)
		}
	}
	r := f.fn(slots)
	zDepth--
	return r
}

func zNoNamed(what string, namedNames []string, line int) {
	if len(namedNames) > 0 {
		Zfatal("`"+what+"` does not take a named argument `"+namedNames[0]+"`", line)
	}
}

func Zcall_named(callee ZV, positional []ZV, namedNames []string, named []ZV, line int) ZV {
	switch c := callee.(type) {
	case *ZBound:
		return zCallFunction(c.fn, append([]ZV{c.object}, positional...), namedNames, named, line)
	case *ZMethod:
		zNoNamed(c.name, namedNames, line)
		return zCallMethodOf(c.receiver, c.name, positional, line)
	case *ZClass:
		slots := zBind(c.name, c.fields, positional, namedNames, named, line)
		for i, slot := range slots {
			if slot == ZMISSING {
				if c.defaults[i] == nil {
					Zfatal("`"+c.name+"` needs a value for `"+c.fields[i]+"`", line)
				}
				slots[i] = c.defaults[i].(*ZFunc).fn(nil)
			}
		}
		return &ZInstance{c, slots}
	case *ZConstructor:
		def := c.def.cases[c.index]
		slots := zBind(def.name, def.fields, positional, namedNames, named, line)
		for i, slot := range slots {
			if slot == ZMISSING {
				Zfatal("`"+def.name+"` needs a value for `"+def.fields[i]+"`", line)
			}
		}
		return &ZCase{c.def, c.index, slots}
	case *ZErrorType:
		zNoNamed(c.kind, namedNames, line)
		return zNewError(c.kind, positional, line)
	case *ZFunc:
		if c.bfn != nil {
			zNoNamed(c.name, namedNames, line)
			return c.bfn(positional, line)
		}
		return zCallFunction(c, positional, namedNames, named, line)
	}
	Zfatal(ZtypeName(callee)+" cannot be called", line)
	return nil
}

func Zcall(callee ZV, args []ZV, line int) ZV { return Zcall_named(callee, args, nil, nil, line) }

func zNewError(kind string, args []ZV, line int) ZV {
	if len(args) > 1 {
		Zfatal("`"+kind+"` takes one message", line)
	}
	message := ""
	if len(args) == 1 {
		message = Zshow(args[0])
	}
	return &ZError{kind, message, nil}
}

func Zcall_method_named(obj ZV, name string, positional []ZV, namedNames []string, named []ZV, line int) ZV {
	switch o := obj.(type) {
	case *ZInstance:
		if zIndexOf(o.cls.fields, name) < 0 {
			if m, ok := o.cls.methods[name]; ok {
				return zCallFunction(m, append([]ZV{o}, positional...), namedNames, named, line)
			}
		}
	case *ZModule:
		zNoNamed(name, namedNames, line)
		return Zcall_builtin(o.name+"."+name, positional, line)
	case string, *ZList, *ZDict:
		zNoNamed(name, namedNames, line)
		return zCallMethodOf(obj, name, positional, line)
	}
	return Zcall_named(Zfield(obj, name, line), positional, namedNames, named, line)
}

func Zcall_method(obj ZV, name string, args []ZV, line int) ZV {
	return Zcall_method_named(obj, name, args, nil, nil, line)
}

// ------------------------------------------------------------- Errors

func Zraise(value ZV, line int) {
	switch v := value.(type) {
	case *ZError:
		panic(&ZRaise{v, line})
	case *ZInstance:
		panic(&ZRaise{&ZError{v.cls.name, Zshow(v), v}, line})
	}
	Zfatal("only an error can be raised, and this is "+ZtypeName(value), line)
}

func Zcatches(e *ZRaise, name string) bool { return name == "Error" || name == e.value.kind }

func Zcaught(e *ZRaise, name string) ZV {
	if name != "" && name != "Error" && e.value.object != nil {
		return e.value.object
	}
	return e.value
}

func Zno_match(v ZV, line int) { Zfatal("no `case` matches "+zRepr(v), line) }

func Zundefined(name string, line int) ZV {
	Zfatal("`"+name+"` is not defined", line)
	return nil
}

// A `try`: the body, then the handler if the body raised an error the
// program may catch. Each says how it ended: 0 at its end, 1 `break`,
// 2 `continue`, 3 `return` with the value.
func Ztry(body func() (int, ZV), handler func(*ZRaise) (int, ZV)) (int, ZV) {
	depth := zDepth
	var caught *ZRaise
	ctl, ret := func() (ctl int, ret ZV) {
		defer func() {
			if r := recover(); r != nil {
				if e, ok := r.(*ZRaise); ok {
					caught = e
					return
				}
				panic(r)
			}
		}()
		return body()
	}()
	if caught != nil {
		zDepth = depth
		return handler(caught)
	}
	return ctl, ret
}

// ------------------------------------------------- Built-in functions

func zArity(name string, args []ZV, lo, hi int, line int) {
	if len(args) >= lo && len(args) <= hi {
		return
	}
	wanted := strconv.Itoa(lo) + " to " + strconv.Itoa(hi) + " arguments"
	if lo == hi {
		wanted = zCounted(lo, "argument")
	}
	Zfatal("`"+name+"` takes "+wanted+", but "+zWere(len(args))+" given", line)
}

func zWrongType(name, wanted string, got ZV, line int) {
	Zfatal("`"+name+"` needs "+wanted+", but this is "+ZtypeName(got), line)
}

func zText(name string, v ZV, line int) string {
	s, ok := v.(string)
	if !ok {
		zWrongType(name, "a str", v, line)
	}
	return s
}

func zWhole(name string, v ZV, line int) int64 {
	n, ok := v.(int64)
	if !ok {
		zWrongType(name, "a whole number", v, line)
	}
	return n
}

const zBlanks = " \t\n\r\f"

func zStrip(s string) string { return strings.Trim(s, zBlanks) }

var zOut = bufio.NewWriterSize(os.Stdout, 65536)

func zFlush() { zOut.Flush() }

type zWidth struct {
	lo, hi int64
	bits   uint
	signed bool
}

var zWidths = map[string]zWidth{
	"u8": {0, 255, 8, false}, "u16": {0, 65535, 16, false}, "u32": {0, 4294967295, 32, false},
	"i8": {-128, 127, 8, true}, "i16": {-32768, 32767, 16, true}, "i32": {-2147483648, 2147483647, 32, true},
}

func zAmount(v ZV) float64 {
	if q, ok := v.(*ZQuantity); ok {
		return q.v
	}
	x, _ := zNumber(v)
	return x
}

func zNumbers(name string, args []ZV, line int) ([]ZV, string) {
	values := args
	if len(args) == 1 {
		if l, ok := args[0].(*ZList); ok {
			values = l.items
		}
	}
	unit := ""
	for place, item := range values {
		if q, ok := item.(*ZQuantity); ok {
			if unit != "" && unit != q.unit {
				Zfatal("`"+name+"` needs quantities in one unit, but these are in "+unit+" and in "+q.unit+". Turn one into the other first, as in `x."+unit+"`", line)
			}
			if unit == "" && place > 0 {
				Zfatal("`"+name+"` needs all plain numbers or all quantities, but these are both", line)
			}
			unit = q.unit
		} else if _, ok := zNumber(item); !ok {
			zWrongType(name, "numbers", item, line)
		} else if unit != "" {
			Zfatal("`"+name+"` needs all plain numbers or all quantities, but these are both", line)
		}
	}
	return values, unit
}

func zPick(name string, args []ZV, line int, better func(a, b float64) bool) ZV {
	values, _ := zNumbers(name, args, line)
	if len(values) == 0 {
		zRaise("ValueError", "`"+name+"` of nothing", line)
	}
	single, others := false, false
	for _, item := range values {
		switch item.(type) {
		case float32:
			single = true
		case int64:
		default:
			others = true
		}
	}
	if single && !others {
		widened := make([]ZV, len(values))
		for i, item := range values {
			x, _ := zSingle(item)
			widened[i] = float32(x)
		}
		values = widened
	}
	best := values[0]
	for _, item := range values[1:] {
		x, ok1 := item.(uint64)
		y, ok2 := best.(uint64)
		if ok1 && ok2 {
			if (name == "min" && x < y) || (name != "min" && x > y) {
				best = item
			}
			continue
		}
		if better(zAmount(item), zAmount(best)) {
			best = item
		}
	}
	return best
}

// A float as Rust's `parse::<f64>` reads it, or false.
func zParseFloat(s string) (float64, bool) {
	body := s
	if body != "" && (body[0] == '+' || body[0] == '-') {
		body = body[1:]
	}
	switch strings.ToLower(body) {
	case "inf", "infinity":
		if s[0] == '-' {
			return math.Inf(-1), true
		}
		return math.Inf(1), true
	case "nan":
		// With a sign too, which strconv refuses.
		return math.NaN(), true
	}
	digits, point, i := 0, false, 0
	for ; i < len(body); i++ {
		c := body[i]
		if c >= '0' && c <= '9' {
			digits++
		} else if c == '.' && !point {
			point = true
		} else {
			break
		}
	}
	if digits == 0 {
		return 0, false
	}
	if i < len(body) {
		if body[i] != 'e' && body[i] != 'E' {
			return 0, false
		}
		i++
		if i < len(body) && (body[i] == '+' || body[i] == '-') {
			i++
		}
		if i == len(body) {
			return 0, false
		}
		for ; i < len(body); i++ {
			if body[i] < '0' || body[i] > '9' {
				return 0, false
			}
		}
	}
	x, err := strconv.ParseFloat(s, 64)
	if err != nil {
		if ne, ok := err.(*strconv.NumError); ok && ne.Err == strconv.ErrRange {
			return x, true
		}
		return 0, false
	}
	return x, true
}

// Heapsort, step for step as the Rust code sorts, so that a function
// given to `sort` is called in the same order.
func zHeapsort(n int, before func(i, j int) bool, swap func(i, j int)) {
	sift := func(root, end int) {
		for {
			child := 2*root + 1
			if child >= end {
				return
			}
			if child+1 < end && before(child, child+1) {
				child++
			}
			if !before(root, child) {
				return
			}
			swap(root, child)
			root = child
		}
	}
	for root := n/2 - 1; root >= 0; root-- {
		sift(root, n)
	}
	for end := n - 1; end > 0; end-- {
		swap(0, end)
		sift(0, end)
	}
}

func zSortList(l *ZList, line int) {
	s := append([]ZV(nil), l.items...)
	zHeapsort(len(s), func(i, j int) bool { return Zlt(s[i], s[j], line) }, func(i, j int) { s[i], s[j] = s[j], s[i] })
	l.items = s
}

func zSortBy(l *ZList, function ZV, line int) {
	var keys []ZV
	for _, item := range append([]ZV(nil), l.items...) {
		k := Zcall(function, []ZV{item}, line)
		switch k.(type) {
		case int64, float64, float32, uint64, string:
		default:
			Zfatal("the function given to `sort` must return a number or text to order by, but it returned "+ZtypeName(k), line)
		}
		keys = append(keys, k)
	}
	s := append([]ZV(nil), l.items...)
	if len(s) != len(keys) {
		zRaise("ValueError", "the function given to `sort` changed how many items the list has", line)
	}
	places := make([]int, len(s))
	for i := range places {
		places[i] = i
	}
	before := func(a, b int) bool {
		if Zlt(keys[a], keys[b], line) {
			return true
		}
		if Zlt(keys[b], keys[a], line) {
			return false
		}
		return places[a] < places[b]
	}
	swap := func(i, j int) {
		s[i], s[j] = s[j], s[i]
		keys[i], keys[j] = keys[j], keys[i]
		places[i], places[j] = places[j], places[i]
	}
	zHeapsort(len(s), before, swap)
	l.items = s
}

func zSlice(args []ZV, length int, what string, line int) (int, int) {
	from := zWhole("slice", args[0], line)
	to := int64(length)
	if len(args) > 1 {
		to = zWhole("slice", args[1], line)
	}
	if from < 0 || to < from || to > int64(length) {
		zRaise("IndexError", "slice "+strconv.FormatInt(from, 10)+" to "+strconv.FormatInt(to, 10)+" is out of range for a "+what+" of length "+strconv.Itoa(length), line)
	}
	return int(from), int(to)
}

var zStrMethods = []string{"upper", "lower", "strip", "split", "lines", "replace", "starts_with", "ends_with", "find", "slice", "code"}
var zListMethods = []string{"append", "pop", "map", "filter", "reduce", "insert", "reverse", "sort", "clear", "find", "slice", "join"}
var zDictMethods = []string{"get", "pop", "keys", "values", "clear"}

func zHasMethod(v ZV, name string) bool {
	switch v.(type) {
	case string:
		return zIndexOf(zStrMethods, name) >= 0
	case *ZList:
		return zIndexOf(zListMethods, name) >= 0
	case *ZDict:
		return zIndexOf(zDictMethods, name) >= 0
	}
	return false
}

func zStrings(parts []string) ZV {
	out := make([]ZV, len(parts))
	for i, p := range parts {
		out[i] = p
	}
	return &ZList{out}
}

func zCallMethodOf(receiver ZV, name string, args []ZV, line int) ZV {
	switch r := receiver.(type) {
	case string:
		s := r
		switch name {
		case "upper", "lower":
			zArity(name, args, 0, 0, line)
			b := []byte(s)
			for i, c := range b {
				if name == "upper" && c >= 'a' && c <= 'z' {
					b[i] = c - 32
				} else if name == "lower" && c >= 'A' && c <= 'Z' {
					b[i] = c + 32
				}
			}
			return string(b)
		case "strip":
			zArity(name, args, 0, 0, line)
			return zStrip(s)
		case "split":
			zArity(name, args, 0, 1, line)
			if len(args) == 0 {
				return zStrings(strings.FieldsFunc(s, func(c rune) bool { return strings.ContainsRune(zBlanks, c) }))
			}
			sep := zText(name, args[0], line)
			if sep == "" {
				zRaise("ValueError", "cannot split on an empty str", line)
			}
			return zStrings(strings.Split(s, sep))
		case "lines":
			zArity(name, args, 0, 0, line)
			if s == "" {
				return &ZList{nil}
			}
			// A "\r" goes with the "\n" just after it, and with nothing else.
			parts := strings.Split(s, "\n")
			last := parts[len(parts)-1]
			parts = parts[:len(parts)-1]
			for i, p := range parts {
				parts[i] = strings.TrimSuffix(p, "\r")
			}
			if last != "" {
				parts = append(parts, last)
			}
			return zStrings(parts)
		case "replace":
			zArity(name, args, 2, 2, line)
			old, new := zText(name, args[0], line), zText(name, args[1], line)
			if old == "" {
				// As Rust and Python do: the new text between every character.
				var b strings.Builder
				b.WriteString(new)
				for _, c := range s {
					b.WriteRune(c)
					b.WriteString(new)
				}
				return b.String()
			}
			return strings.ReplaceAll(s, old, new)
		case "starts_with":
			zArity(name, args, 1, 1, line)
			return strings.HasPrefix(s, zText(name, args[0], line))
		case "ends_with":
			zArity(name, args, 1, 1, line)
			return strings.HasSuffix(s, zText(name, args[0], line))
		case "find":
			zArity(name, args, 1, 1, line)
			at := strings.Index(s, zText(name, args[0], line))
			if at < 0 {
				return nil
			}
			return int64(utf8.RuneCountInString(s[:at]))
		case "slice":
			zArity(name, args, 1, 2, line)
			from, to := zSlice(args, zLen(s), "str", line)
			return zSub(s, from, to)
		case "code":
			zArity(name, args, 0, 0, line)
			if n := zLen(s); n != 1 {
				zRaise("ValueError", "`code` needs a str of exactly one character, and this one has "+strconv.Itoa(n), line)
			}
			c, _ := utf8.DecodeRuneInString(s)
			return int64(c)
		}
		Zfatal("a str has no method `"+name+"`", line)
	case *ZDict:
		d := r
		switch name {
		case "get":
			zArity(name, args, 1, 2, line)
			k := zKey(args[0], line)
			if v, has := d.vals[k]; has {
				return v
			}
			if len(args) > 1 {
				return args[1]
			}
			return nil
		case "pop":
			zArity(name, args, 1, 1, line)
			k := zKey(args[0], line)
			v, has := d.vals[k]
			if !has {
				zRaise("KeyError", "this dict has no key "+zRepr(args[0]), line)
			}
			d.remove(k)
			return v
		case "keys":
			zArity(name, args, 0, 0, line)
			return &ZList{append([]ZV(nil), d.keys...)}
		case "values":
			zArity(name, args, 0, 0, line)
			out := make([]ZV, len(d.keys))
			for i, k := range d.keys {
				out[i] = d.vals[k]
			}
			return &ZList{out}
		case "clear":
			zArity(name, args, 0, 0, line)
			d.keys = nil
			d.vals = map[ZV]ZV{}
			return nil
		}
		Zfatal("a dict has no method `"+name+"`", line)
	case *ZList:
		l := r
		switch name {
		case "append":
			zArity(name, args, 1, 1, line)
			l.items = append(l.items, args[0])
			return nil
		case "pop":
			zArity(name, args, 0, 1, line)
			if len(args) > 0 {
				at := zPosition(args[0], len(l.items), "list", line)
				v := l.items[at]
				l.items = append(l.items[:at], l.items[at+1:]...)
				return v
			}
			if len(l.items) == 0 {
				zRaise("IndexError", "cannot pop from an empty list", line)
			}
			v := l.items[len(l.items)-1]
			l.items = l.items[:len(l.items)-1]
			return v
		case "insert":
			zArity(name, args, 2, 2, line)
			at := zWhole(name, args[0], line)
			if at < 0 || at > int64(len(l.items)) {
				zRaise("IndexError", "cannot insert at "+strconv.FormatInt(at, 10)+" in a list of length "+strconv.Itoa(len(l.items)), line)
			}
			l.items = append(l.items, nil)
			copy(l.items[at+1:], l.items[at:])
			l.items[at] = args[1]
			return nil
		case "reverse":
			zArity(name, args, 0, 0, line)
			for i, j := 0, len(l.items)-1; i < j; i, j = i+1, j-1 {
				l.items[i], l.items[j] = l.items[j], l.items[i]
			}
			return nil
		case "clear":
			zArity(name, args, 0, 0, line)
			l.items = nil
			return nil
		case "sort":
			if len(args) == 1 {
				zSortBy(l, args[0], line)
				return nil
			}
			zArity(name, args, 0, 1, line)
			zSortList(l, line)
			return nil
		case "find":
			zArity(name, args, 1, 1, line)
			for at, item := range l.items {
				if zEqual(item, args[0]) {
					return int64(at)
				}
			}
			return nil
		case "slice":
			zArity(name, args, 1, 2, line)
			from, to := zSlice(args, len(l.items), "list", line)
			return &ZList{append([]ZV(nil), l.items[from:to]...)}
		case "join":
			zArity(name, args, 1, 1, line)
			sep := zText(name, args[0], line)
			parts := make([]string, len(l.items))
			for i, item := range l.items {
				s, ok := item.(string)
				if !ok {
					Zfatal("`join` puts text together, and this list holds "+ZtypeName(item)+". Make each item text first, with `str( )`", line)
				}
				parts[i] = s
			}
			return strings.Join(parts, sep)
		case "map":
			zArity(name, args, 1, 1, line)
			items := append([]ZV(nil), l.items...)
			out := make([]ZV, len(items))
			for i, item := range items {
				out[i] = Zcall(args[0], []ZV{item}, line)
			}
			return &ZList{out}
		case "filter":
			zArity(name, args, 1, 1, line)
			var kept []ZV
			for _, item := range append([]ZV(nil), l.items...) {
				keep := Zcall(args[0], []ZV{item}, line)
				b, ok := keep.(bool)
				if !ok {
					Zfatal("the function given to `filter` must return true or false, but it returned "+ZtypeName(keep), line)
				}
				if b {
					kept = append(kept, item)
				}
			}
			return &ZList{kept}
		case "reduce":
			zArity(name, args, 2, 2, line)
			total := args[1]
			for _, item := range append([]ZV(nil), l.items...) {
				total = Zcall(args[0], []ZV{total, item}, line)
			}
			return total
		}
		Zfatal("a list has no method `"+name+"`", line)
	}
	Zfatal(ZtypeName(receiver)+" has no method `"+name+"`", line)
	return nil
}

// `{x:>8.2f}`: [<>^]? 0? width? (.precision)? kind?
func Zformat(v ZV, spec string, line int) ZV {
	if spec == "" {
		return Zshow(v)
	}
	bad := func() { Zfatal("`"+spec+"` is not a format Zeno understands", line) }
	rest := spec
	align := byte(0)
	if rest != "" && (rest[0] == '<' || rest[0] == '>' || rest[0] == '^') {
		align, rest = rest[0], rest[1:]
	}
	zero := false
	if rest != "" && rest[0] == '0' {
		zero, rest = true, rest[1:]
	}
	i := 0
	for i < len(rest) && rest[i] >= '0' && rest[i] <= '9' {
		i++
	}
	width := 0
	if i > 0 {
		width, _ = strconv.Atoi(rest[:i])
	}
	rest = rest[i:]
	precision := -1
	if rest != "" && rest[0] == '.' {
		rest = rest[1:]
		i = 0
		for i < len(rest) && rest[i] >= '0' && rest[i] <= '9' {
			i++
		}
		precision = 0
		if i > 0 {
			precision, _ = strconv.Atoi(rest[:i])
		}
		rest = rest[i:]
	}
	kind := ""
	if rest != "" {
		c, size := utf8.DecodeRuneInString(rest)
		if size != len(rest) || (c >= '0' && c <= '9') {
			bad()
		}
		kind = rest
	}
	n, isInt := v.(int64)
	u, isU64 := v.(uint64)
	_, isFloat := v.(float64)
	q, isQ := v.(*ZQuantity)
	places := precision
	if places < 0 {
		places = 6
	}
	var body string
	switch {
	case (kind == "f" || (kind == "" && precision >= 0)) && isQ:
		body = zFixed(q.v, places) + " " + q.unit
	case kind == "f" || (kind == "" && precision >= 0 && isFloat):
		x, ok := zNumber(v)
		if !ok {
			zWrongType(spec, "a number", v, line)
		}
		body = zFixed(x, places)
	case (kind == "" && precision < 0) || (kind == "d" && precision < 0 && (isInt || isU64)):
		body = Zshow(v)
	case (kind == "x" || kind == "X" || kind == "b") && precision < 0 && isU64:
		base := 16
		if kind == "b" {
			base = 2
		}
		body = strconv.FormatUint(u, base)
		if kind == "X" {
			body = strings.ToUpper(body)
		}
	case (kind == "x" || kind == "X" || kind == "b") && precision < 0 && isInt:
		base := 16
		if kind == "b" {
			base = 2
		}
		mag := uint64(n)
		sign := ""
		if n < 0 {
			mag, sign = uint64(-n), "-"
		}
		digits := strconv.FormatUint(mag, base)
		if kind == "X" {
			digits = strings.ToUpper(digits)
		}
		body = sign + digits
	case (kind == "d" || kind == "x" || kind == "X" || kind == "b") && precision < 0:
		zWrongType(spec, "an int", v, line)
	default:
		bad()
	}
	padding := width - utf8.RuneCountInString(body)
	if padding < 0 {
		padding = 0
	}
	_, isNumber := zNumber(v)
	isNumber = isNumber || isQ
	if zero && isNumber {
		if strings.HasPrefix(body, "-") {
			return "-" + strings.Repeat("0", padding) + body[1:]
		}
		return strings.Repeat("0", padding) + body
	}
	if align == 0 {
		align = '<'
		if isNumber {
			align = '>'
		}
	}
	switch align {
	case '<':
		return body + strings.Repeat(" ", padding)
	case '>':
		return strings.Repeat(" ", padding) + body
	}
	left := padding / 2
	return strings.Repeat(" ", left) + body + strings.Repeat(" ", padding-left)
}

// ------------------------------------------------------------ JSON

type zJSON struct {
	cs []rune
	at int
}

type zNotJSON struct{}

func (j *zJSON) fail() { panic(zNotJSON{}) }

func (j *zJSON) spaces() {
	for j.at < len(j.cs) && strings.ContainsRune(" \t\n\r", j.cs[j.at]) {
		j.at++
	}
}

func (j *zJSON) word(w string) bool {
	r := []rune(w)
	if j.at+len(r) > len(j.cs) || string(j.cs[j.at:j.at+len(r)]) != w {
		return false
	}
	j.at += len(r)
	return true
}

func (j *zJSON) fourHex() int {
	if j.at+4 > len(j.cs) {
		j.fail()
	}
	n, err := strconv.ParseUint(string(j.cs[j.at:j.at+4]), 16, 32)
	if err != nil || strings.ContainsAny(string(j.cs[j.at:j.at+4]), "+-") {
		j.fail()
	}
	j.at += 4
	return int(n)
}

func (j *zJSON) text() string {
	if j.at >= len(j.cs) || j.cs[j.at] != '"' {
		j.fail()
	}
	j.at++
	var b strings.Builder
	for {
		if j.at >= len(j.cs) {
			j.fail()
		}
		c := j.cs[j.at]
		j.at++
		if c == '"' {
			return b.String()
		}
		if c != '\\' {
			b.WriteRune(c)
			continue
		}
		if j.at >= len(j.cs) {
			j.fail()
		}
		e := j.cs[j.at]
		j.at++
		switch e {
		case 'n':
			b.WriteByte('\n')
		case 't':
			b.WriteByte('\t')
		case 'r':
			b.WriteByte('\r')
		case 'b':
			b.WriteByte('\b')
		case 'f':
			b.WriteByte('\f')
		case 'u':
			code := j.fourHex()
			if code >= 0xD800 && code < 0xDC00 && j.word("\\u") {
				low := j.fourHex()
				if low < 0xDC00 || low > 0xDFFF {
					j.fail()
				}
				code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00)
			}
			if (code >= 0xD800 && code <= 0xDFFF) || code > 0x10FFFF {
				j.fail()
			}
			b.WriteRune(rune(code))
		default:
			b.WriteRune(e)
		}
	}
}

func (j *zJSON) value() ZV {
	j.spaces()
	if j.at >= len(j.cs) {
		j.fail()
	}
	switch j.cs[j.at] {
	case 'n':
		if j.word("null") {
			return nil
		}
		j.fail()
	case 't':
		if j.word("true") {
			return true
		}
		j.fail()
	case 'f':
		if j.word("false") {
			return false
		}
		j.fail()
	case '"':
		return j.text()
	case '[':
		j.at++
		var out []ZV
		for {
			j.spaces()
			if j.word("]") {
				return &ZList{out}
			}
			if len(out) > 0 && !j.word(",") {
				j.fail()
			}
			out = append(out, j.value())
		}
	case '{':
		j.at++
		d := zNewDict()
		count := 0
		for {
			j.spaces()
			if j.word("}") {
				return d
			}
			if count > 0 && !j.word(",") {
				j.fail()
			}
			j.spaces()
			name := j.text()
			j.spaces()
			if !j.word(":") {
				j.fail()
			}
			v := j.value()
			count++
			if v != nil {
				d.set(name, v)
			}
		}
	}
	start := j.at
	for j.at < len(j.cs) && strings.ContainsRune("0123456789+-.eE", j.cs[j.at]) {
		j.at++
	}
	x, ok := zParseFloat(string(j.cs[start:j.at]))
	if !ok {
		j.fail()
	}
	if !math.IsInf(x, 0) && x == math.Trunc(x) && math.Abs(x) < 9.0e15 {
		return int64(x)
	}
	return x
}

func zJSONParse(s string) (result ZV, ok bool) {
	defer func() {
		if r := recover(); r != nil {
			if _, is := r.(zNotJSON); !is {
				panic(r)
			}
			result, ok = nil, false
		}
	}()
	j := &zJSON{[]rune(s), 0}
	v := j.value()
	j.spaces()
	if j.at != len(j.cs) {
		return nil, false
	}
	return v, true
}

func zJSONText(s string) string {
	var b strings.Builder
	b.WriteByte('"')
	for _, c := range s {
		switch {
		case c == '"':
			b.WriteString("\\\"")
		case c == '\\':
			b.WriteString("\\\\")
		case c == '\n':
			b.WriteString("\\n")
		case c == '\r':
			b.WriteString("\\r")
		case c == '\t':
			b.WriteString("\\t")
		case c < 0x20:
			h := strconv.FormatInt(int64(c), 16)
			b.WriteString("\\u" + strings.Repeat("0", 4-len(h)) + h)
		default:
			b.WriteRune(c)
		}
	}
	b.WriteByte('"')
	return b.String()
}

func zJSONWrite(v ZV, line int) string {
	switch x := v.(type) {
	case nil:
		return "null"
	case bool:
		if x {
			return "true"
		}
		return "false"
	case int64:
		return strconv.FormatInt(x, 10)
	case float64:
		if math.IsNaN(x) || math.IsInf(x, 0) {
			zRaise("ValueError", "JSON has no way to write "+ZshowFloat(x), line)
		}
		return ZshowFloat(x)
	case string:
		return zJSONText(x)
	case *ZList:
		parts := make([]string, len(x.items))
		for i, item := range x.items {
			parts[i] = zJSONWrite(item, line)
		}
		return "[" + strings.Join(parts, ",") + "]"
	case *ZDict:
		parts := make([]string, len(x.keys))
		for i, k := range x.keys {
			parts[i] = zJSONText(Zshow(k)) + ":" + zJSONWrite(x.vals[k], line)
		}
		return "{" + strings.Join(parts, ",") + "}"
	case *ZInstance:
		parts := make([]string, len(x.cls.fields))
		for i, name := range x.cls.fields {
			parts[i] = zJSONText(name) + ":" + zJSONWrite(x.fields[i], line)
		}
		return "{" + strings.Join(parts, ",") + "}"
	case *ZQuantity:
		zRaise("ValueError", "JSON has no units, and this is a quantity in "+x.unit+". Write its number, and put the unit in the name: `[\""+x.unit+"\": ...]`", line)
	}
	zRaise("ValueError", "JSON has objects, lists, text, numbers, bools and none, and this is "+ZtypeName(v), line)
	return ""
}

// ----------------------------------------------------------- Files

// "file", "nothing" or "other": what is at a path.
func zAt(path string) string {
	f, err := os.Open(path)
	if err != nil {
		if os.IsNotExist(err) {
			return "nothing"
		}
		return "other"
	}
	defer f.Close()
	one := make([]byte, 1)
	if _, err := f.Read(one); err != nil && err.Error() != "EOF" {
		return "other"
	}
	return "file"
}

func zPaths(name string, args []ZV, count int, line int) []string {
	zArity(name, args, count, count, line)
	out := make([]string, len(args))
	for i, a := range args {
		out[i] = zText(name, a, line)
	}
	return out
}

func zFileError(message string, line int) { zRaise("FileError", message, line) }

var zStarted = time.Now()

// ------------------------------------------------- The built-in table

func Zb_print(args []ZV, line int) ZV {
	for i, a := range args {
		if i > 0 {
			zOut.WriteByte(' ')
		}
		zOut.WriteString(Zshow(a))
	}
	zOut.WriteByte('\n')
	return nil
}

var zIn = bufio.NewReader(os.Stdin)

// Text from outside that may not be UTF-8. Each run of bytes that could
// have begun a character and did not finish one becomes one U+FFFD, and
// so does each byte that could not begin one: the way Rust, Python and
// the browsers read such text.
func zLossy(s string) string {
	if utf8.ValidString(s) {
		return s
	}
	var out strings.Builder
	for i := 0; i < len(s); {
		c := s[i]
		if c < 0x80 {
			out.WriteByte(c)
			i++
			continue
		}
		// How many bytes follow a first byte like this one, and what
		// the second of them may be.
		need, lo, hi := 0, byte(0x80), byte(0xBF)
		switch {
		case c >= 0xC2 && c <= 0xDF:
			need = 1
		case c == 0xE0:
			need, lo = 2, 0xA0
		case c == 0xED:
			need, hi = 2, 0x9F
		case c >= 0xE1 && c <= 0xEF:
			need = 2
		case c == 0xF0:
			need, lo = 3, 0x90
		case c == 0xF4:
			need, hi = 3, 0x8F
		case c >= 0xF1 && c <= 0xF3:
			need = 3
		}
		j, whole := i+1, need > 0
		for k := 0; whole && k < need; k++ {
			if j < len(s) && s[j] >= lo && s[j] <= hi {
				j++
			} else {
				whole = false
			}
			lo, hi = 0x80, 0xBF
		}
		if whole {
			out.WriteString(s[i:j])
		} else {
			out.WriteString("\uFFFD")
		}
		i = j
	}
	return out.String()
}

func Zb_input(args []ZV, line int) ZV {
	zArity("input", args, 0, 1, line)
	if len(args) > 0 {
		zOut.WriteString(zText("input", args[0], line))
	}
	zFlush()
	s, err := zIn.ReadString('\n')
	if err != nil && s == "" {
		return nil
	}
	s = zLossy(s)
	if strings.HasSuffix(s, "\n") {
		s = strings.TrimSuffix(strings.TrimSuffix(s, "\n"), "\r")
	}
	return strings.ReplaceAll(s, "\x00", "")
}

func Zb_len(args []ZV, line int) ZV {
	zArity("len", args, 1, 1, line)
	switch v := args[0].(type) {
	case string:
		return int64(zLen(v))
	case *ZList:
		return int64(len(v.items))
	case *ZDict:
		return int64(len(v.keys))
	}
	zWrongType("len", "a str, a list or a dict", args[0], line)
	return nil
}

func Zb_range(args []ZV, line int) ZV {
	zArity("range", args, 1, 3, line)
	n := make([]int64, len(args))
	for i, a := range args {
		x, ok := a.(int64)
		if !ok {
			zWrongType("range", "ints", a, line)
		}
		n[i] = x
	}
	start, stop, step := int64(0), int64(0), int64(1)
	switch len(n) {
	case 1:
		stop = n[0]
	case 2:
		start, stop = n[0], n[1]
	default:
		start, stop, step = n[0], n[1], n[2]
	}
	if step == 0 {
		zRaise("ValueError", "the step of a range cannot be 0", line)
	}
	var out []ZV
	for i := start; (step > 0 && i < stop) || (step < 0 && i > stop); {
		out = append(out, i)
		next := i + step
		if (step > 0 && next < i) || (step < 0 && next > i) {
			break
		}
		i = next
	}
	return &ZList{out}
}

func zToInt(x float64) int64 {
	if math.IsNaN(x) {
		return 0
	}
	if x >= 9.223372036854775807e18 {
		return math.MaxInt64
	}
	if x <= -9.223372036854775808e18 {
		return math.MinInt64
	}
	return int64(x)
}

func Zb_int(args []ZV, line int) ZV {
	zArity("int", args, 1, 1, line)
	switch v := args[0].(type) {
	case int64:
		return v
	case uint64:
		if v > math.MaxInt64 {
			zRaise("OverflowError", strconv.FormatUint(v, 10)+" does not fit in an int (-9223372036854775808 to 9223372036854775807)", line)
		}
		return int64(v)
	case float64, float32:
		x, _ := zNumber(v)
		if math.IsNaN(x) || math.IsInf(x, 0) {
			zRaise("ValueError", "cannot turn "+ZshowFloat(x)+" into an int", line)
		}
		return zToInt(x)
	case string:
		s := strings.ReplaceAll(zStrip(v), "_", "")
		digits := strings.TrimLeft(s, "+-")
		ok := len(s)-len(digits) <= 1 && digits != ""
		for i := 0; ok && i < len(digits); i++ {
			ok = digits[i] >= '0' && digits[i] <= '9'
		}
		if ok {
			if n, err := strconv.ParseInt(s, 10, 64); err == nil {
				return n
			}
		}
		zRaise("ValueError", "cannot turn "+zQuoted(v)+" into an int", line)
	}
	zWrongType("int", "a number or a str", args[0], line)
	return nil
}

func Zb_float(args []ZV, line int) ZV {
	zArity("float", args, 1, 1, line)
	switch v := args[0].(type) {
	case int64:
		return float64(v)
	case float64:
		return v
	case float32:
		return float64(v)
	case uint64:
		return float64(v)
	case string:
		x, ok := zParseFloat(zStrip(v))
		if !ok {
			zRaise("ValueError", "cannot turn "+zQuoted(v)+" into a float", line)
		}
		return x
	}
	zWrongType("float", "a number or a str", args[0], line)
	return nil
}

func Zb_f32(args []ZV, line int) ZV {
	zArity("f32", args, 1, 1, line)
	switch v := args[0].(type) {
	case float32:
		return v
	case int64:
		x, _ := zSingle(v)
		return float32(x)
	case float64:
		r := zFround(v)
		if math.IsInf(r, 0) && !math.IsInf(v, 0) && v == v {
			zRaise("OverflowError", ZshowFloat(v)+" does not fit in an f32, whose largest is about 3.4e38", line)
		}
		return float32(r)
	}
	zWrongType("f32", "a number", args[0], line)
	return nil
}

func Zb_str(args []ZV, line int) ZV {
	zArity("str", args, 1, 1, line)
	return Zshow(args[0])
}

func Zb_abs(args []ZV, line int) ZV {
	zArity("abs", args, 1, 1, line)
	switch v := args[0].(type) {
	case int64:
		if v == math.MinInt64 {
			zOverflow(line)
		}
		if v < 0 {
			return -v
		}
		return v
	case float64:
		return math.Abs(v)
	case float32:
		return float32(math.Abs(float64(v)))
	case uint64:
		return v
	case *ZQuantity:
		return &ZQuantity{math.Abs(v.v), v.unit}
	}
	zWrongType("abs", "a number", args[0], line)
	return nil
}

func Zb_min(args []ZV, line int) ZV {
	return zPick("min", args, line, func(a, b float64) bool { return a < b })
}

func Zb_max(args []ZV, line int) ZV {
	return zPick("max", args, line, func(a, b float64) bool { return a > b })
}

func Zb_sum(args []ZV, line int) ZV {
	values, unit := zNumbers("sum", args, line)
	if unit != "" {
		Zfatal("`sum` works on plain numbers, and these have a unit. Add quantities with `+`, which keeps the unit: start from `var total = 0 V` and add each in a loop", line)
	}
	single, others := false, false
	for _, item := range values {
		switch item.(type) {
		case float32:
			single = true
		case int64:
		default:
			others = true
		}
	}
	if single && !others {
		r := 0.0
		for _, item := range values {
			x, _ := zSingle(item)
			r = zFround(r + x)
		}
		return float32(r)
	}
	u64s := false
	for _, item := range values {
		_, isU64 := item.(uint64)
		u64s = u64s || isU64
	}
	if u64s {
		total := uint64(0)
		for _, item := range values {
			x, _ := zUnsigned(item)
			r, carry := bits.Add64(total, x, 0)
			if carry != 0 {
				zRaise("OverflowError", zU64TooBig, line)
			}
			total = r
		}
		return total
	}
	total, allInts, floatTotal := int64(0), true, 0.0
	for _, item := range values {
		x, _ := zNumber(item)
		floatTotal += x
		if n, ok := item.(int64); ok && allInts {
			total = zAddInt(total, n, line)
		} else {
			allInts = false
		}
	}
	if allInts {
		return total
	}
	return floatTotal
}

func Zb_char(args []ZV, line int) ZV {
	zArity("char", args, 1, 1, line)
	code := zWhole("char", args[0], line)
	if code <= 0 || code > 0x10FFFF || (code >= 0xD800 && code <= 0xDFFF) {
		zRaise("ValueError", strconv.FormatInt(code, 10)+" is not the code of a character", line)
	}
	return string(rune(code))
}

func Zb_allocate(args []ZV, line int) ZV {
	Zfatal("manual memory is only for a program compiled to machine code", line)
	return nil
}

var Zb_free = Zb_allocate

func zOneNumber(name string, args []ZV, line int) float64 {
	zArity(name, args, 1, 1, line)
	x, ok := zNumber(args[0])
	if !ok {
		zWrongType(name, "a number", args[0], line)
	}
	return x
}

func zAngle(name string, args []ZV, line int) float64 {
	zArity(name, args, 1, 1, line)
	if q, ok := args[0].(*ZQuantity); ok {
		s, ok := zConvert(q.unit, "rad")
		if !ok {
			Zfatal("`"+name+"` needs a number or an angle, such as `30 deg`, not a quantity in "+q.unit, line)
		}
		return zApply(s, q.v)
	}
	x, ok := zNumber(args[0])
	if !ok {
		zWrongType(name, "a number or an angle, such as `30 deg`", args[0], line)
	}
	return x
}

func zRatio(name string, args []ZV, line int) float64 {
	x := zOneNumber(name, args, line)
	if x < -1 || x > 1 {
		zRaise("ValueError", "`"+name+"` needs a number from -1 to 1, and this is "+ZshowFloat(x), line)
	}
	return x
}

func zAboveZero(name string, args []ZV, line int) float64 {
	x := zOneNumber(name, args, line)
	if x <= 0 {
		zRaise("ValueError", "cannot take the logarithm of "+ZshowFloat(x), line)
	}
	return x
}

// A whole float as an int; infinity and NaN stay as they are.
// What `floor` and `ceil` give is an int: one too big for an int is the
// biggest int there is, and what is not a number is 0, as in `zeno run`.
func zWholeOf(x float64) ZV {
	return zToInt(x)
}

func Zb_math_sqrt(args []ZV, line int) ZV {
	x := zOneNumber("sqrt", args, line)
	if x < 0 {
		zRaise("ValueError", "cannot take the square root of "+ZshowFloat(x), line)
	}
	return math.Sqrt(x)
}

func Zb_math_floor(args []ZV, line int) ZV { return zWholeOf(math.Floor(zOneNumber("floor", args, line))) }
func Zb_math_ceil(args []ZV, line int) ZV  { return zWholeOf(math.Ceil(zOneNumber("ceil", args, line))) }

// A half goes away from zero, as Rust's `round` does.
func Zb_math_round(args []ZV, line int) ZV { return zToInt(math.Round(zOneNumber("round", args, line))) }
func Zb_math_sin(args []ZV, line int) ZV   { return zSin(zAngle("sin", args, line)) }
func Zb_math_cos(args []ZV, line int) ZV   { return zCos(zAngle("cos", args, line)) }
func Zb_math_tan(args []ZV, line int) ZV   { return zTan(zAngle("tan", args, line)) }
func Zb_math_asin(args []ZV, line int) ZV {
	return &ZQuantity{math.Asin(zRatio("asin", args, line)), "rad"}
}
func Zb_math_acos(args []ZV, line int) ZV {
	return &ZQuantity{math.Acos(zRatio("acos", args, line)), "rad"}
}
func Zb_math_atan(args []ZV, line int) ZV {
	return &ZQuantity{math.Atan(zOneNumber("atan", args, line)), "rad"}
}
func Zb_math_atan2(args []ZV, line int) ZV {
	zArity("atan2", args, 2, 2, line)
	y, ok1 := zNumber(args[0])
	if !ok1 {
		zWrongType("atan2", "two numbers", args[0], line)
	}
	x, ok2 := zNumber(args[1])
	if !ok2 {
		zWrongType("atan2", "two numbers", args[1], line)
	}
	return &ZQuantity{math.Atan2(y, x), "rad"}
}
func Zb_math_exp(args []ZV, line int) ZV   { return math.Exp(zOneNumber("exp", args, line)) }
func Zb_math_log(args []ZV, line int) ZV   { return math.Log(zAboveZero("log", args, line)) }
func Zb_math_log2(args []ZV, line int) ZV  { return math.Log2(zAboveZero("log2", args, line)) }
func Zb_math_log10(args []ZV, line int) ZV { return math.Log10(zAboveZero("log10", args, line)) }

func Zb_time_sleep(args []ZV, line int) ZV {
	zArity("sleep", args, 1, 1, line)
	q, ok := args[0].(*ZQuantity)
	if !ok {
		zWrongType("sleep", "a time with a unit, such as `500 ms`", args[0], line)
	}
	s, ok := zConvert(q.unit, "s")
	if !ok {
		Zfatal("`sleep` needs a time, such as `500 ms`, not a quantity in "+q.unit, line)
	}
	seconds := zApply(s, q.v)
	if seconds > 0 {
		zFlush()
		time.Sleep(time.Duration(math.Min(seconds, 1e9) * 1e9))
	}
	return nil
}

func Zb_time_now(args []ZV, line int) ZV {
	zArity("now", args, 0, 0, line)
	return &ZQuantity{float64(time.Since(zStarted).Nanoseconds()) / 1e6, "ms"}
}

func Zb_files_read(args []ZV, line int) ZV {
	path := zPaths("read", args, 1, line)[0]
	switch zAt(path) {
	case "nothing":
		zFileError("there is no file `"+path+"`", line)
	case "other":
		zFileError("cannot read `"+path+"`", line)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		zFileError("cannot read `"+path+"`", line)
	}
	if !utf8.Valid(data) || strings.ContainsRune(string(data), 0) {
		zFileError("`"+path+"` does not hold text", line)
	}
	return string(data)
}

func zPut(name string, args []ZV, appending bool, line int) ZV {
	p := zPaths(name, args, 2, line)
	flags := os.O_WRONLY | os.O_CREATE | os.O_TRUNC
	if appending {
		flags = os.O_WRONLY | os.O_CREATE | os.O_APPEND
	}
	f, err := os.OpenFile(p[0], flags, 0o644)
	if err == nil {
		_, err = f.WriteString(p[1])
		if cerr := f.Close(); err == nil {
			err = cerr
		}
	}
	if err != nil {
		zFileError("cannot write `"+p[0]+"`", line)
	}
	return nil
}

func Zb_files_read_bytes(args []ZV, line int) ZV {
	path := zPaths("read_bytes", args, 1, line)[0]
	switch zAt(path) {
	case "nothing":
		zFileError("there is no file `"+path+"`", line)
	case "other":
		zFileError("cannot read `"+path+"`", line)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		zFileError("cannot read `"+path+"`", line)
	}
	out := make([]ZV, len(data))
	for i, b := range data {
		out[i] = int64(b)
	}
	return &ZList{out}
}

func Zb_files_write_bytes(args []ZV, line int) ZV {
	zArity("write_bytes", args, 2, 2, line)
	path := zText("write_bytes", args[0], line)
	items, ok := args[1].(*ZList)
	if !ok {
		zWrongType("write_bytes", "a list[u8]", args[1], line)
	}
	data := make([]byte, len(items.items))
	for i, item := range items.items {
		n, ok := item.(int64)
		if !ok {
			zWrongType("write_bytes", "a list[u8]", item, line)
		}
		if n < 0 || n > 255 {
			zRaise("ValueError", strconv.FormatInt(n, 10)+" is not a byte: a byte is a number from 0 to 255", line)
		}
		data[i] = byte(n)
	}
	if err := os.WriteFile(path, data, 0o644); err != nil {
		zFileError("cannot write `"+path+"`", line)
	}
	return nil
}

func Zb_files_write(args []ZV, line int) ZV  { return zPut("write", args, false, line) }
func Zb_files_append(args []ZV, line int) ZV { return zPut("append", args, true, line) }
func Zb_files_exists(args []ZV, line int) ZV {
	return zAt(zPaths("exists", args, 1, line)[0]) == "file"
}

func Zb_files_remove(args []ZV, line int) ZV {
	path := zPaths("remove", args, 1, line)[0]
	switch zAt(path) {
	case "nothing":
		zFileError("there is no file `"+path+"`", line)
	case "file":
		if os.Remove(path) == nil {
			return nil
		}
	}
	zFileError("cannot remove `"+path+"`", line)
	return nil
}

// `files.list(folder)`: the names in it, sorted by their bytes, which for
// UTF-8 is the order of the numbers of their characters. `os.ReadDir`
// gives them in that order already.
func Zb_files_list(args []ZV, line int) ZV {
	folder := zPaths("list", args, 1, line)[0]
	entries, err := os.ReadDir(folder)
	if err != nil {
		if os.IsNotExist(err) {
			zFileError("there is no folder `"+folder+"`", line)
		}
		zFileError("cannot list the folder `"+folder+"`", line)
	}
	out := make([]ZV, 0, len(entries))
	for _, entry := range entries {
		name := entry.Name()
		if !utf8.ValidString(name) {
			zFileError("the folder `"+folder+"` holds a name that is not text", line)
		}
		out = append(out, name)
	}
	return &ZList{out}
}

// `files.make_folder(path)`: it worked if there is a folder there at the end.
func Zb_files_make_folder(args []ZV, line int) ZV {
	path := zPaths("make_folder", args, 1, line)[0]
	if path != "" {
		_ = os.MkdirAll(path, 0o777)
	}
	if info, err := os.Stat(path); path == "" || err != nil || !info.IsDir() {
		zFileError("cannot make the folder `"+path+"`", line)
	}
	return nil
}

// What a `files.Reader` and a `files.Writer` are made of (decision
// 59.6). A file's number is its place in `zOpenFiles`, plus one; a closed
// file leaves its place empty. Files are read as bytes.
type zOpenFile struct {
	file   *os.File
	reader *bufio.Reader
}

var zOpenFiles []*zOpenFile

func zHandle(name string, args []ZV, line int) *zOpenFile {
	number, ok := args[0].(int64)
	if !ok {
		zWrongType(name, "an int", args[0], line)
	}
	if number < 1 || number > int64(len(zOpenFiles)) || zOpenFiles[number-1] == nil {
		Zfatal("there is no open file numbered "+strconv.FormatInt(number, 10), line)
	}
	return zOpenFiles[number-1]
}

func Zb_files_handle_open(args []ZV, line int) ZV {
	zArity("handle_open", args, 2, 2, line)
	path := zText("handle_open", args[0], line)
	how, _ := args[1].(int64)
	open := &zOpenFile{}
	if how == 0 {
		switch zAt(path) {
		case "nothing":
			zFileError("there is no file `"+path+"`", line)
		case "other":
			zFileError("cannot read `"+path+"`", line)
		}
		f, err := os.Open(path)
		if err != nil {
			zFileError("cannot read `"+path+"`", line)
		}
		open.file, open.reader = f, bufio.NewReader(f)
	} else {
		flags := os.O_WRONLY | os.O_CREATE | os.O_TRUNC
		if how == 2 {
			flags = os.O_WRONLY | os.O_CREATE | os.O_APPEND
		}
		f, err := os.OpenFile(path, flags, 0o644)
		if err != nil {
			zFileError("cannot write `"+path+"`", line)
		}
		open.file = f
	}
	for place, there := range zOpenFiles {
		if there == nil {
			zOpenFiles[place] = open
			return int64(place + 1)
		}
	}
	zOpenFiles = append(zOpenFiles, open)
	return int64(len(zOpenFiles))
}

// The next line without its line end (a `\r` before the `\n` goes with
// it), or none at the end.
func Zb_files_handle_line(args []ZV, line int) ZV {
	zArity("handle_line", args, 2, 2, line)
	path := zText("handle_line", args[1], line)
	open := zHandle("handle_line", args, line)
	if open.reader == nil {
		zFileError("cannot read `"+path+"`", line)
	}
	data, err := open.reader.ReadBytes('\n')
	if err != nil && len(data) == 0 {
		if err.Error() == "EOF" {
			return nil
		}
		zFileError("cannot read `"+path+"`", line)
	}
	if n := len(data); n > 0 && data[n-1] == '\n' {
		data = data[:n-1]
		if n := len(data); n > 0 && data[n-1] == '\r' {
			data = data[:n-1]
		}
	}
	if !utf8.Valid(data) || strings.ContainsRune(string(data), 0) {
		zFileError("`"+path+"` does not hold text", line)
	}
	return string(data)
}

// Up to `count` bytes, fewer at the end, none after it.
func Zb_files_handle_bytes(args []ZV, line int) ZV {
	zArity("handle_bytes", args, 3, 3, line)
	path := zText("handle_bytes", args[1], line)
	count, _ := args[2].(int64)
	if count < 0 {
		count = 0
	}
	open := zHandle("handle_bytes", args, line)
	if open.reader == nil {
		zFileError("cannot read `"+path+"`", line)
	}
	data := make([]byte, count)
	got := 0
	for int64(got) < count {
		n, err := open.reader.Read(data[got:])
		got += n
		if err != nil {
			if err.Error() == "EOF" {
				break
			}
			zFileError("cannot read `"+path+"`", line)
		}
	}
	out := make([]ZV, got)
	for i, b := range data[:got] {
		out[i] = int64(b)
	}
	return &ZList{out}
}

// Every byte is handed to the system before this gives back (an
// `*os.File` holds nothing back), so that a full disk is an error at the
// write that met it.
func zHandlePut(name string, args []ZV, data []byte, path string, line int) ZV {
	open := zHandle(name, args, line)
	if open.reader != nil {
		zFileError("cannot write `"+path+"`", line)
	}
	if _, err := open.file.Write(data); err != nil {
		zFileError("cannot write `"+path+"`", line)
	}
	return nil
}

func Zb_files_handle_write(args []ZV, line int) ZV {
	zArity("handle_write", args, 3, 3, line)
	path := zText("handle_write", args[1], line)
	text := zText("handle_write", args[2], line)
	return zHandlePut("handle_write", args, []byte(text), path, line)
}

func Zb_files_handle_write_bytes(args []ZV, line int) ZV {
	zArity("handle_write_bytes", args, 3, 3, line)
	path := zText("handle_write_bytes", args[1], line)
	items, ok := args[2].(*ZList)
	if !ok {
		zWrongType("handle_write_bytes", "a list[u8]", args[2], line)
	}
	data := make([]byte, len(items.items))
	for i, item := range items.items {
		n, ok := item.(int64)
		if !ok {
			zWrongType("handle_write_bytes", "a list[u8]", item, line)
		}
		if n < 0 || n > 255 {
			zRaise("ValueError", strconv.FormatInt(n, 10)+" is not a byte: a byte is a number from 0 to 255", line)
		}
		data[i] = byte(n)
	}
	return zHandlePut("handle_write_bytes", args, data, path, line)
}

// The file is closed, and its number free.
func Zb_files_handle_close(args []ZV, line int) ZV {
	zArity("handle_close", args, 1, 1, line)
	open := zHandle("handle_close", args, line)
	zOpenFiles[args[0].(int64)-1] = nil
	_ = open.file.Close()
	return nil
}

// `program.exit(code)`: what was printed goes out, and the program ends
// with `code`.
func Zb_program_exit(args []ZV, line int) ZV {
	zArity("exit", args, 1, 1, line)
	code := zWhole("exit", args[0], line)
	if code < 0 || code > 255 {
		zRaise("ValueError", "a program ends with a number from 0 to 255, not "+strconv.FormatInt(code, 10), line)
	}
	zFlush()
	os.Exit(int(code))
	return nil
}

// `program.environment(name)`: the text, or none. A name with `=` or a
// zero byte in it is in no environment.
func Zb_program_environment(args []ZV, line int) ZV {
	zArity("environment", args, 1, 1, line)
	name := zText("environment", args[0], line)
	if name == "" || strings.ContainsAny(name, "=\x00") {
		return nil
	}
	value, found := os.LookupEnv(name)
	if !found {
		return nil
	}
	return zLossy(value)
}

// `program.warn(text)`: one line on the error stream, after what was printed.
func Zb_program_warn(args []ZV, line int) ZV {
	zArity("warn", args, 1, 1, line)
	said := zText("warn", args[0], line)
	zFlush()
	os.Stderr.WriteString(said + "\n")
	return nil
}

func Zb_program_arguments(args []ZV, line int) ZV {
	zArity("arguments", args, 0, 0, line)
	out := make([]ZV, 0, len(os.Args))
	for _, a := range os.Args[1:] {
		out = append(out, a)
	}
	return &ZList{out}
}

func Zb_json_parse(args []ZV, line int) ZV {
	zArity("parse", args, 1, 1, line)
	s := zText("parse", args[0], line)
	v, ok := zJSONParse(s)
	if !ok {
		start := []rune(strings.TrimLeft(s, " \t\n\r"))
		shown := string(start)
		if len(start) > 30 {
			shown = string(start[:30]) + "..."
		}
		zRaise("ValueError", "this text is not JSON: `"+shown+"`", line)
	}
	return v
}

func Zb_json_write(args []ZV, line int) ZV {
	zArity("write", args, 1, 1, line)
	return zJSONWrite(args[0], line)
}

func zSized(name string, w zWidth) (func([]ZV, int) ZV, func([]ZV, int) ZV) {
	article := "a"
	if w.signed {
		article = "an"
	}
	fit := func(args []ZV, line int) ZV {
		zArity(name, args, 1, 1, line)
		n, ok := args[0].(int64)
		if !ok {
			zWrongType(name, "a whole number", args[0], line)
		}
		if n < w.lo || n > w.hi {
			zRaise("OverflowError", strconv.FormatInt(n, 10)+" does not fit in "+article+" "+name+" ("+strconv.FormatInt(w.lo, 10)+" to "+strconv.FormatInt(w.hi, 10)+")", line)
		}
		return n
	}
	wrap := func(args []ZV, line int) ZV {
		zArity(name+".wrap", args, 1, 1, line)
		n, ok := args[0].(int64)
		if !ok {
			zWrongType(name+".wrap", "a whole number", args[0], line)
		}
		low := n & (int64(1)<<w.bits - 1)
		if w.signed && low > w.hi {
			return low - int64(1)<<w.bits
		}
		return low
	}
	return fit, wrap
}

var Zb_u8, Zb_u8_wrap = zSized("u8", zWidths["u8"])
var Zb_u16, Zb_u16_wrap = zSized("u16", zWidths["u16"])
var Zb_u32, Zb_u32_wrap = zSized("u32", zWidths["u32"])
var Zb_i8, Zb_i8_wrap = zSized("i8", zWidths["i8"])
var Zb_i16, Zb_i16_wrap = zSized("i16", zWidths["i16"])
var Zb_i32, Zb_i32_wrap = zSized("i32", zWidths["i32"])

func zErrorMaker(kind string) func([]ZV, int) ZV {
	return func(args []ZV, line int) ZV { return zNewError(kind, args, line) }
}

var Zb_Error = zErrorMaker("Error")
var Zb_ValueError = zErrorMaker("ValueError")
var Zb_IndexError = zErrorMaker("IndexError")
var Zb_KeyError = zErrorMaker("KeyError")
var Zb_ZeroDivisionError = zErrorMaker("ZeroDivisionError")
var Zb_OverflowError = zErrorMaker("OverflowError")
var Zb_FileError = zErrorMaker("FileError")

var zErrorKinds = []string{"Error", "ValueError", "IndexError", "KeyError", "ZeroDivisionError", "OverflowError", "FileError"}

var zBuiltins map[string]func([]ZV, int) ZV

func init() {
	zBuiltins = map[string]func([]ZV, int) ZV{
		"print": Zb_print, "input": Zb_input, "len": Zb_len, "range": Zb_range, "int": Zb_int,
		"float": Zb_float, "f32": Zb_f32, "u64": Zb_u64, "u64..of": Zb_u64__of, "u64..add": Zb_u64__add, "u64..sub": Zb_u64__sub, "u64..mul": Zb_u64__mul, "u64..shl": Zb_u64__shl, "str": Zb_str, "abs": Zb_abs, "min": Zb_min, "max": Zb_max, "sum": Zb_sum,
		"char": Zb_char, "allocate": Zb_allocate, "free": Zb_free,
		"math.sqrt": Zb_math_sqrt, "math.floor": Zb_math_floor, "math.ceil": Zb_math_ceil,
		"math.round": Zb_math_round, "math.sin": Zb_math_sin, "math.cos": Zb_math_cos,
		"math.tan": Zb_math_tan, "math.asin": Zb_math_asin, "math.acos": Zb_math_acos,
		"math.atan": Zb_math_atan, "math.atan2": Zb_math_atan2, "math.exp": Zb_math_exp,
		"math.log": Zb_math_log, "math.log2": Zb_math_log2, "math.log10": Zb_math_log10,
		"time.sleep": Zb_time_sleep, "time.now": Zb_time_now,
		"files.read": Zb_files_read, "files.write": Zb_files_write, "files.read_bytes": Zb_files_read_bytes, "files.write_bytes": Zb_files_write_bytes, "files.append": Zb_files_append,
		"files.exists": Zb_files_exists, "files.remove": Zb_files_remove, "files.list": Zb_files_list, "files.make_folder": Zb_files_make_folder,
		"files.handle_open": Zb_files_handle_open, "files.handle_line": Zb_files_handle_line, "files.handle_bytes": Zb_files_handle_bytes,
		"files.handle_write": Zb_files_handle_write, "files.handle_write_bytes": Zb_files_handle_write_bytes, "files.handle_close": Zb_files_handle_close,
		"program.arguments": Zb_program_arguments, "program.exit": Zb_program_exit, "program.environment": Zb_program_environment, "program.warn": Zb_program_warn, "json.parse": Zb_json_parse, "json.write": Zb_json_write,
		"u8": Zb_u8, "u8.wrap": Zb_u8_wrap, "u16": Zb_u16, "u16.wrap": Zb_u16_wrap,
		"u32": Zb_u32, "u32.wrap": Zb_u32_wrap, "i8": Zb_i8, "i8.wrap": Zb_i8_wrap,
		"i16": Zb_i16, "i16.wrap": Zb_i16_wrap, "i32": Zb_i32, "i32.wrap": Zb_i32_wrap,
		"Error": Zb_Error, "ValueError": Zb_ValueError, "IndexError": Zb_IndexError, "KeyError": Zb_KeyError,
		"ZeroDivisionError": Zb_ZeroDivisionError, "OverflowError": Zb_OverflowError, "FileError": Zb_FileError,
	}
}

func Zcall_builtin(name string, args []ZV, line int) ZV {
	f, ok := zBuiltins[name]
	if !ok {
		Zfatal("there is no `"+name+"` in a program built for Go", line)
	}
	return f(args, line)
}

var zBuiltinValues = map[string]ZV{}

// Something that comes with Zeno, as a value: `print`, `math.pi`.
func Zbuiltin(name string, line int) ZV {
	switch name {
	case "math.pi":
		return math.Pi
	case "math.e":
		return math.E
	}
	if zIndexOf(zErrorKinds, name) >= 0 {
		return &ZErrorType{name}
	}
	if v, ok := zBuiltinValues[name]; ok {
		return v
	}
	body, ok := zBuiltins[name]
	if !ok {
		if at := strings.IndexByte(name, '.'); at >= 0 {
			Zfatal("module `"+name[:at]+"` has no `"+name[at+1:]+"`", line)
		}
		Zfatal("`"+name+"` is not defined", line)
	}
	short := name
	if at := strings.IndexByte(name, '.'); at >= 0 {
		short = name[at+1:]
	}
	v := &ZFunc{name: short, builtin: name, bfn: body}
	zBuiltinValues[name] = v
	return v
}

func Zmodule(name string) ZV { return &ZModule{name} }

// ------------------------------------------------------ Sine and cosine

// Go's own sine, cosine and tangent can differ in the last digit from
// those of the C library that `zeno run` uses. These are fdlibm's, as
// JavaScript and Java (StrictMath) have them, which agree with it.

func zHigh(x float64) int32 { return int32(math.Float64bits(x) >> 32) }

func zLowZero(x float64) float64 { return math.Float64frombits(math.Float64bits(x) &^ 0xffffffff) }

func zKernelSin(x, y float64, iy int) float64 {
	const (
		s1 = -1.66666666666666324348e-01
		s2 = 8.33333333332248946124e-03
		s3 = -1.98412698298579493134e-04
		s4 = 2.75573137070700676789e-06
		s5 = -2.50507602534068634195e-08
		s6 = 1.58969099521155010221e-10
	)
	ix := zHigh(x) & 0x7fffffff
	if ix < 0x3e400000 && int(x) == 0 {
		return x
	}
	z := x * x
	v := z * x
	r := s2 + z*(s3+z*(s4+z*(s5+z*s6)))
	if iy == 0 {
		return x + v*(s1+z*r)
	}
	return x - ((z*(0.5*y-v*r) - y) - v*s1)
}

func zKernelCos(x, y float64) float64 {
	const (
		c1 = 4.16666666666666019037e-02
		c2 = -1.38888888888741095749e-03
		c3 = 2.48015872894767294178e-05
		c4 = -2.75573143513906633035e-07
		c5 = 2.08757232129817482790e-09
		c6 = -1.13596475577881948265e-11
	)
	ix := zHigh(x) & 0x7fffffff
	if ix < 0x3e400000 && int(x) == 0 {
		return 1
	}
	z := x * x
	r := z * (c1 + z*(c2+z*(c3+z*(c4+z*(c5+z*c6)))))
	if ix < 0x3FD33333 {
		return 1 - (0.5*z - (z*r - x*y))
	}
	qx := 0.28125
	if ix <= 0x3fe90000 {
		qx = math.Float64frombits(uint64(uint32(ix-0x00200000)) << 32)
	}
	hz := 0.5*z - qx
	a := 1 - qx
	return a - (hz - (z*r - x*y))
}

var zT = [13]float64{
	3.33333333333334091986e-01, 1.33333333333201242699e-01, 5.39682539762260521377e-02,
	2.18694882948595424599e-02, 8.86323982359930005737e-03, 3.59207910759131235356e-03,
	1.45620945432529025516e-03, 5.88041240820264096874e-04, 2.46463134818469906812e-04,
	7.81794442939557092300e-05, 7.14072491382608190305e-05, -1.85586374855275456654e-05,
	2.59073051863633712884e-05,
}

func zKernelTan(x, y float64, iy int) float64 {
	const (
		pio4   = 7.85398163397448278999e-01
		pio4lo = 3.06161699786838301793e-17
	)
	hx := zHigh(x)
	ix := hx & 0x7fffffff
	if ix < 0x3e300000 && int(x) == 0 {
		low := uint32(math.Float64bits(x))
		if (uint32(ix)|low)|uint32(iy+1) == 0 {
			return 1 / math.Abs(x)
		}
		if iy == 1 {
			return x
		}
		w := x + y
		z := zLowZero(w)
		v := y - (z - x)
		a := -1 / w
		t := zLowZero(a)
		s := 1 + t*z
		return t + a*(s+t*v)
	}
	if ix >= 0x3FE59428 {
		if hx < 0 {
			x, y = -x, -y
		}
		z := pio4 - x
		w := pio4lo - y
		x = z + w
		y = 0
	}
	z := x * x
	w := z * z
	r := zT[1] + w*(zT[3]+w*(zT[5]+w*(zT[7]+w*(zT[9]+w*zT[11]))))
	v := z * (zT[2] + w*(zT[4]+w*(zT[6]+w*(zT[8]+w*(zT[10]+w*zT[12])))))
	s := z * x
	r = y + z*(s*(r+v)+y)
	r += zT[0] * s
	w = x + r
	if ix >= 0x3FE59428 {
		v = float64(iy)
		return float64(1-((hx>>30)&2)) * (v - 2.0*(x-(w*w/(w+v)-r)))
	}
	if iy == 1 {
		return w
	}
	z = zLowZero(w)
	v = r - (z - x)
	a := -1 / w
	t := zLowZero(a)
	s = 1 + t*z
	return t + a*(s+t*v)
}

// x less a whole number n of half turns (pi/2), as y0 + y1; false when x
// is too big for this way.
func zRemPio2(x float64) (int, float64, float64, bool) {
	const (
		invpio2 = 6.36619772367581382433e-01
		pio2_1  = 1.57079632673412561417e+00
		pio2_1t = 6.07710050650619224932e-11
		pio2_2  = 6.07710050630396597660e-11
		pio2_2t = 2.02226624879595063154e-21
		pio2_3  = 2.02226624871116645580e-21
		pio2_3t = 8.47842766036889956997e-32
	)
	hx := zHigh(x)
	ix := hx & 0x7fffffff
	if ix < 0x4002d97c {
		if hx > 0 {
			z := x - pio2_1
			if ix != 0x3ff921fb {
				y0 := z - pio2_1t
				return 1, y0, (z - y0) - pio2_1t, true
			}
			z -= pio2_2
			y0 := z - pio2_2t
			return 1, y0, (z - y0) - pio2_2t, true
		}
		z := x + pio2_1
		if ix != 0x3ff921fb {
			y0 := z + pio2_1t
			return -1, y0, (z - y0) + pio2_1t, true
		}
		z += pio2_2
		y0 := z + pio2_2t
		return -1, y0, (z - y0) + pio2_2t, true
	}
	if ix > 0x413921fb {
		return 0, 0, 0, false
	}
	t := math.Abs(x)
	n := int(t*invpio2 + 0.5)
	fn := float64(n)
	r := t - fn*pio2_1
	w := fn * pio2_1t
	j := ix >> 20
	y0 := r - w
	i := j - ((zHigh(y0) >> 20) & 0x7ff)
	if i > 16 {
		t = r
		w = fn * pio2_2
		r = t - w
		w = fn*pio2_2t - ((t - r) - w)
		y0 = r - w
		i = j - ((zHigh(y0) >> 20) & 0x7ff)
		if i > 49 {
			t = r
			w = fn * pio2_3
			r = t - w
			w = fn*pio2_3t - ((t - r) - w)
			y0 = r - w
		}
	}
	y1 := (r - y0) - w
	if hx < 0 {
		return -n, -y0, -y1, true
	}
	return n, y0, y1, true
}

func zSin(x float64) float64 {
	ix := zHigh(x) & 0x7fffffff
	if ix <= 0x3fe921fb {
		return zKernelSin(x, 0, 0)
	}
	if ix >= 0x7ff00000 {
		return math.NaN()
	}
	n, y0, y1, ok := zRemPio2(x)
	if !ok {
		return math.Sin(x)
	}
	switch n & 3 {
	case 0:
		return zKernelSin(y0, y1, 1)
	case 1:
		return zKernelCos(y0, y1)
	case 2:
		return -zKernelSin(y0, y1, 1)
	}
	return -zKernelCos(y0, y1)
}

func zCos(x float64) float64 {
	ix := zHigh(x) & 0x7fffffff
	if ix <= 0x3fe921fb {
		return zKernelCos(x, 0)
	}
	if ix >= 0x7ff00000 {
		return math.NaN()
	}
	n, y0, y1, ok := zRemPio2(x)
	if !ok {
		return math.Cos(x)
	}
	switch n & 3 {
	case 0:
		return zKernelCos(y0, y1)
	case 1:
		return -zKernelSin(y0, y1, 1)
	case 2:
		return -zKernelCos(y0, y1)
	}
	return zKernelSin(y0, y1, 1)
}

func zTan(x float64) float64 {
	ix := zHigh(x) & 0x7fffffff
	if ix <= 0x3fe921fb {
		return zKernelTan(x, 0, 1)
	}
	if ix >= 0x7ff00000 {
		return math.NaN()
	}
	n, y0, y1, ok := zRemPio2(x)
	if !ok {
		return math.Tan(x)
	}
	return zKernelTan(y0, y1, 1-((n&1)<<1))
}

// ------------------------------------------------------- The program

// Run the program, and say what stopped it if something did.
func Zstart(files []string, main func() ZV) {
	where := func(line int) string {
		index := line / zLinesAFile
		file := files[0]
		if index < len(files) {
			file = files[index]
		}
		return " --> " + file + ":" + strconv.Itoa(line%zLinesAFile)
	}
	status := 0
	func() {
		defer func() {
			if r := recover(); r != nil {
				zFlush()
				switch e := r.(type) {
				case *ZRaise:
					os.Stderr.WriteString("error: uncaught " + zShownName(e.value.kind) + ": " + Zshow(e.value) + "\n" + where(e.line) + "\n")
				case *ZFatal:
					os.Stderr.WriteString("error: " + e.message + "\n" + where(e.line) + "\n")
				default:
					panic(r)
				}
				status = 1
			}
		}()
		main()
	}()
	zFlush()
	os.Exit(status)
}
