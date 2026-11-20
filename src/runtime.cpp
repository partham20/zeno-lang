// The Zeno runtime for C++.
//
// A program built with `zeno build --target cpp` is one `.cpp` file: this
// runtime, the namespace `Z`, then the program, in `main`. It holds what
// every Zeno value is, and what each operator and built-in does with it:
// what ops.rs and builtins.rs do for `zeno run`, step for step and message
// for message, so that the program prints what `zeno run` prints.
// runtime.cs is the same for C#, and the two are close.
//
// Every Zeno value is a `Z::V`: a small struct that holds an int, a float,
// a bool or none itself, and anything else by a shared pointer:
//
//   int        int64_t, with every step checked for overflow
//   float      double
//   str        Z::Str, UTF-8; an index counts characters, not bytes
//   list       Z::List
//   dict       Z::Dict, which keeps the order the keys were put in
//   quantity   Z::Quantity: a number and its unit
//   function   Z::Func
//   object     Z::Instance of a Z::Class
//   enum case  Z::Case of a Z::Enum
//   error      Z::Error
//
// Values are freed when the last pointer to them goes. A cycle (a list
// that holds itself) is never freed, which only matters to a program
// that makes many.

#include <algorithm>
#include <array>
#include <cerrno>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <dirent.h>
#include <functional>
#include <memory>
#include <pthread.h>
#include <sys/stat.h>
#include <string>
#include <thread>
#include <unordered_map>
#include <utility>
#include <vector>

// `a * b + c` is two steps: multiply and round, then add and round. Some
// processors (ARM64, and x86 where the compiler is told it may) have one
// instruction that does both and rounds once, and a C++ compiler is
// allowed to pick it without being asked. `zeno run` always takes two
// steps, so this program must too: `7.0 % 0.1` is 0.0 in two steps and
// -3.885780586188048e-16 in one. These lines say so to each family of
// compiler, as runtime.c does for C.
#if defined(__clang__)
#pragma STDC FP_CONTRACT OFF
#elif defined(__GNUC__)
#pragma GCC optimize("fp-contract=off")
#elif defined(_MSC_VER)
#pragma fp_contract(off)
#endif

namespace Z {

using std::string;
using namespace std::string_literals;

const int LINES_A_FILE = 1000000;

// -------------------------------------------------------------- Values

enum class K { STR, LIST, DICT, QUANTITY, FUNC, CLASS, INSTANCE, ENUM, CASE, CTOR, ERRTYPE, ERROR, MODULE, METHOD, BOUND, SEVERAL };

struct Obj {
    K k;
    explicit Obj(K k) : k(k) {}
    virtual ~Obj() {}
};

struct V {
    // An f32 is an F32, whose `f` is the float it is exactly (decision 41).
    // A u64 is a U64, whose `i` holds its 64 bits (decision 42).
    enum T : uint8_t { NONE, INT, FLOAT, BOOL, OBJ, MISSING, F32, U64 } t = NONE;
    union {
        int64_t i;
        double f;
        bool b;
    };
    std::shared_ptr<Obj> p;
    V() : i(0) {}
    bool is(K k) const { return t == OBJ && p->k == k; }
};

using Args = std::vector<V>;
using Names = std::vector<string>;
using Flags = std::vector<bool>;
using Cell = std::shared_ptr<V>;

inline V I(int64_t n) { V v; v.t = V::INT; v.i = n; return v; }
inline V F(double x) { V v; v.t = V::FLOAT; v.f = x; return v; }
inline V F32V(double x) { V v; v.t = V::F32; v.f = (double)(float)x; return v; }
inline V U64V(uint64_t u) { V v; v.t = V::U64; v.i = (int64_t)u; return v; }
inline V B(bool b) { V v; v.t = V::BOOL; v.b = b; return v; }
inline V O(std::shared_ptr<Obj> p) { V v; v.t = V::OBJ; v.p = std::move(p); return v; }
const V NONE_V;
inline V make_missing() { V v; v.t = V::MISSING; return v; }
// What a call gives a parameter it leaves out.
const V MISSING = make_missing();

inline Cell cell() { return std::make_shared<V>(); }
inline Cell cell(const V& v) { return std::make_shared<V>(v); }

struct Str : Obj {
    string s;
    int plain = -1; // 1 if every character is one byte, 0 if not, -1 not yet known
    explicit Str(string s) : Obj(K::STR), s(std::move(s)) {}
};

inline V S(string s) { return O(std::make_shared<Str>(std::move(s))); }

struct List : Obj {
    Args items;
    explicit List(Args items) : Obj(K::LIST), items(std::move(items)) {}
};

// A key of a dict: an int or a str.
struct Key {
    bool isInt;
    int64_t i;
    string s;
    bool operator==(const Key& o) const { return isInt == o.isInt && (isInt ? i == o.i : s == o.s); }
};

struct KeyHash {
    size_t operator()(const Key& k) const { return k.isInt ? std::hash<int64_t>()(k.i) : std::hash<string>()(k.s); }
};

// A dict: its keys in the order they were put in, and their values.
struct Dict : Obj {
    std::vector<V> keys;
    std::unordered_map<Key, V, KeyHash> map;
    Dict() : Obj(K::DICT) {}
};

struct Quantity : Obj {
    double v;
    string unit;
    Quantity(double v, string unit) : Obj(K::QUANTITY), v(v), unit(std::move(unit)) {}
};

inline V Q(double v, const string& unit) { return O(std::make_shared<Quantity>(v, unit)); }

using Body = std::function<V(const Args&)>;
using BuiltinBody = std::function<V(const Args&, int)>;

struct Func : Obj {
    string name;
    Names parameters;
    Flags defaults;
    Body body;
    // A function that comes with Zeno: its full name, and its body.
    string builtin;
    BuiltinBody bbody;
    Func(string name, Names parameters, Flags defaults, Body body, string builtin, BuiltinBody bbody)
        : Obj(K::FUNC), name(std::move(name)), parameters(std::move(parameters)), defaults(std::move(defaults)),
          body(std::move(body)), builtin(std::move(builtin)), bbody(std::move(bbody)) {}
};

struct Class : Obj {
    string name;
    Names fields;
    Args defaults;
    std::vector<std::pair<string, std::shared_ptr<Func>>> methods;
    Class(string name, Names fields, Args defaults) : Obj(K::CLASS), name(std::move(name)), fields(std::move(fields)), defaults(std::move(defaults)) {}
    std::shared_ptr<Func> method(const string& n) const {
        for (auto& m : methods)
            if (m.first == n) return m.second;
        return nullptr;
    }
};

struct Instance : Obj {
    std::shared_ptr<Class> cls;
    Args fields;
    Instance(std::shared_ptr<Class> cls, Args fields) : Obj(K::INSTANCE), cls(std::move(cls)), fields(std::move(fields)) {}
};

struct Enum : Obj {
    string name;
    Names cases;
    std::vector<Names> fields;
    Enum(string name, Names cases, std::vector<Names> fields) : Obj(K::ENUM), name(std::move(name)), cases(std::move(cases)), fields(std::move(fields)) {}
};

struct Case : Obj {
    std::shared_ptr<Enum> def;
    int index;
    Args fields;
    Case(std::shared_ptr<Enum> def, int index, Args fields) : Obj(K::CASE), def(std::move(def)), index(index), fields(std::move(fields)) {}
};

struct Ctor : Obj {
    std::shared_ptr<Enum> def;
    int index;
    Ctor(std::shared_ptr<Enum> def, int index) : Obj(K::CTOR), def(std::move(def)), index(index) {}
};

struct ErrorType : Obj {
    string kind;
    explicit ErrorType(string kind) : Obj(K::ERRTYPE), kind(std::move(kind)) {}
};

struct Error : Obj {
    string kind, message;
    V obj; // the object raised, if it was one of a class
    Error(string kind, string message, V obj) : Obj(K::ERROR), kind(std::move(kind)), message(std::move(message)), obj(std::move(obj)) {}
};

struct Module : Obj {
    string name;
    explicit Module(string name) : Obj(K::MODULE), name(std::move(name)) {}
};

// A method of a str, a list or a dict, read but not yet called.
struct Method : Obj {
    V receiver;
    string name;
    Method(V receiver, string name) : Obj(K::METHOD), receiver(std::move(receiver)), name(std::move(name)) {}
};

// A method of a class, read from an object.
struct Bound : Obj {
    V obj;
    std::shared_ptr<Func> fn;
    Bound(V obj, std::shared_ptr<Func> fn) : Obj(K::BOUND), obj(std::move(obj)), fn(std::move(fn)) {}
};

// Several values from one function, as `return a, b` gives them, until
// `let a, b = ...` takes them apart (decision 57).
struct Several : Obj {
    Args values;
    explicit Several(Args values) : Obj(K::SEVERAL), values(std::move(values)) {}
};

template <class T>
inline T& as(const V& v) { return *static_cast<T*>(v.p.get()); }

template <class T>
inline std::shared_ptr<T> ptr(const V& v) { return std::static_pointer_cast<T>(v.p); }

inline const string& str(const V& v) { return as<Str>(v).s; }

// An error the program may catch, on its way to a `try`.
struct Raise {
    std::shared_ptr<Error> value;
    int line;
};

// A mistake in the program, which nothing catches.
struct Fatal {
    string text;
    int line;
};

[[noreturn]] inline void fatal(const string& message, int line) { throw Fatal{message, line}; }

[[noreturn]] inline void raiseError(const string& kind, const string& message, int line) {
    throw Raise{std::make_shared<Error>(kind, message, V()), line};
}

[[noreturn]] inline void overflow(int line) { raiseError("OverflowError", "the result is too big for an int", line); }

[[noreturn]] inline void byZero(int line) { raiseError("ZeroDivisionError", "division by zero", line); }

inline bool isNumber(const V& v) { return v.t == V::INT || v.t == V::FLOAT || v.t == V::F32 || v.t == V::U64; }

inline double number(const V& v) { return v.t == V::INT ? (double)v.i : v.t == V::U64 ? (double)(uint64_t)v.i : v.f; }

// The nearest f32 to a float, as a float.
inline double fround(double x) { return (double)(float)x; }

// An f32, or an int as a float and then the nearest f32 (41.2).
inline bool single(const V& v, double& x) {
    if (v.t == V::F32) { x = v.f; return true; }
    if (v.t == V::INT) { x = fround((double)v.i); return true; }
    return false;
}

inline bool isF32(const V& a, const V& b) { return a.t == V::F32 || b.t == V::F32; }

inline bool isU64(const V& a, const V& b) { return a.t == V::U64 || b.t == V::U64; }

// A u64, or an int of 0 or more, as a uint64_t (decision 42).
inline bool unsignedOf(const V& v, uint64_t& x) {
    if (v.t == V::U64 || (v.t == V::INT && v.i >= 0)) { x = (uint64_t)v.i; return true; }
    return false;
}

inline unsigned u64ShiftCount(const V& n, int line) {
    if (n.t == V::U64 ? (uint64_t)n.i >= 64 : (n.i < 0 || n.i >= 64)) {
        string count = n.t == V::U64 ? std::to_string((uint64_t)n.i) : std::to_string(n.i);
        raiseError("ValueError", "cannot shift by " + count + ". A u64 has 64 bits, so a shift is 0 to 63", line);
    }
    return (unsigned)n.i;
}

// True when some value is an f32 and every one is an f32 or an int.
inline bool allSingle(const Args& values) {
    bool any = false;
    for (auto& v : values) {
        if (v.t == V::F32) any = true;
        else if (v.t != V::INT) return false;
    }
    return any;
}

// The float with the fewest digits that reads back as this f32.
inline double f32Shown(double x) {
    if (std::isnan(x) || x == 0 || std::isinf(x)) return x;
    for (int digits = 1; digits <= 9; digits++) {
        char buffer[64];
        snprintf(buffer, sizeof buffer, "%.*e", digits - 1, x);
        double d = strtod(buffer, nullptr);
        if (fround(d) == x) return d;
    }
    return x;
}

inline string num(int64_t n) { return std::to_string(n); }

string typeName(const V& v) {
    switch (v.t) {
    case V::INT: return "an int";
    case V::FLOAT: return "a float";
    case V::F32: return "an f32";
    case V::U64: return "a u64";
    case V::BOOL: return "a bool";
    case V::NONE: return "none";
    case V::MISSING: return "a value";
    case V::OBJ: break;
    }
    switch (v.p->k) {
    case K::STR: return "a str";
    case K::QUANTITY: return "a quantity in " + as<Quantity>(v).unit;
    case K::LIST: return "a list";
    case K::DICT: return "a dict";
    case K::FUNC: case K::METHOD: case K::BOUND: return "a function";
    case K::CLASS: return "a class";
    case K::INSTANCE: return "a " + as<Instance>(v).cls->name;
    case K::ENUM: return "an enum";
    case K::CTOR: return "an enum case";
    case K::CASE: return "a " + as<Case>(v).def->name;
    case K::ERRTYPE: return "an error type";
    case K::ERROR: return "a " + as<Error>(v).kind;
    case K::MODULE: return "a module";
    case K::SEVERAL: return num(as<Several>(v).values.size()) + " values";
    }
    return "a value";
}

// `Point`, not `shapes.Point`; `Stack`, not `Stack[int]`.
string shownName(string name) {
    size_t bracket = name.find('[');
    if (bracket != string::npos) name = name.substr(0, bracket);
    size_t dot = name.rfind('.');
    return dot == string::npos ? name : name.substr(dot + 1);
}

// ---------------------------------------------------------------- Text

// A str is UTF-8; its characters are code points. Most text is all one
// byte a character, and then a byte index is a character index; a Str
// remembers which it is.
inline bool plainText(const string& s) {
    for (unsigned char c : s)
        if (c >= 0x80) return false;
    return true;
}

inline bool plain(const V& v) {
    Str& s = as<Str>(v);
    if (s.plain < 0) s.plain = plainText(s.s) ? 1 : 0;
    return s.plain == 1;
}

inline size_t charLen(unsigned char c) { return c < 0x80 ? 1 : c < 0xE0 ? 2 : c < 0xF0 ? 3 : 4; }

std::vector<string> codePoints(const string& s) {
    std::vector<string> out;
    out.reserve(s.size());
    for (size_t i = 0; i < s.size();) {
        size_t n = std::min(charLen((unsigned char)s[i]), s.size() - i);
        out.push_back(s.substr(i, n));
        i += n;
    }
    return out;
}

int64_t textLength(const string& s) {
    int64_t n = 0;
    for (unsigned char c : s)
        if ((c & 0xC0) != 0x80) n++;
    return n;
}

int64_t textLength(const V& v) { return plain(v) ? (int64_t)str(v).size() : textLength(str(v)); }

// The byte where character `at` starts.
size_t byteAt(const string& s, int64_t at) {
    size_t i = 0;
    for (int64_t n = 0; n < at && i < s.size(); n++) i += charLen((unsigned char)s[i]);
    return std::min(i, s.size());
}

string sub(const V& v, int64_t from, int64_t to) {
    const string& s = str(v);
    if (plain(v)) return s.substr(from, to - from);
    size_t a = byteAt(s, from);
    size_t b = a + byteAt(s.substr(a), to - from);
    return s.substr(a, b - a);
}

int codePointAt(const string& s, size_t i = 0) {
    unsigned char c = s[i];
    if (c < 0x80) return c;
    int n = (int)charLen(c);
    int code = c & (n == 2 ? 0x1F : n == 3 ? 0x0F : 0x07);
    for (int k = 1; k < n && i + k < s.size(); k++) code = (code << 6) | (s[i + k] & 0x3F);
    return code;
}

string fromCode(int64_t code) {
    string out;
    if (code < 0x80) out += (char)code;
    else if (code < 0x800) { out += (char)(0xC0 | (code >> 6)); out += (char)(0x80 | (code & 0x3F)); }
    else if (code < 0x10000) { out += (char)(0xE0 | (code >> 12)); out += (char)(0x80 | ((code >> 6) & 0x3F)); out += (char)(0x80 | (code & 0x3F)); }
    else { out += (char)(0xF0 | (code >> 18)); out += (char)(0x80 | ((code >> 12) & 0x3F)); out += (char)(0x80 | ((code >> 6) & 0x3F)); out += (char)(0x80 | (code & 0x3F)); }
    return out;
}

string hex(int64_t n, int width = 0) {
    char buf[32];
    snprintf(buf, sizeof buf, "%0*llx", width, (unsigned long long)n);
    return buf;
}

string join(const std::vector<string>& parts, const string& sep) {
    string out;
    for (size_t i = 0; i < parts.size(); i++) {
        if (i > 0) out += sep;
        out += parts[i];
    }
    return out;
}

// ------------------------------------------------------------- Showing

// The digits of `text`, which is d.ddde-XX, with the last of them one
// higher. False when every digit is a 9: one digit fewer names that
// number, and was tried before.
static bool stepHigher(char* text) {
    char* at = strchr(text, 'e');
    while (at-- > text) {
        if (*at == '.') continue;
        if (*at != '9') {
            (*at)++;
            return true;
        }
        *at = '0';
    }
    return false;
}

// A float as Rust's `{:?}` shows it: the shortest digits that read back
// as the same number, `.0` on a whole one, and an exponent for one below
// 1e-4 or from 1e16 up.
string showFloat(double x) {
    if (std::isnan(x)) return "NaN";
    if (std::isinf(x)) return x > 0 ? "inf" : "-inf";
    if (x == 0) return std::signbit(x) ? "-0.0" : "0.0";
    string sign = x < 0 ? "-" : "";
    x = std::fabs(x);
    // The shortest digits that read back as x: try 1 to 17. At each
    // length the digits rounded from x are tried, and then those one
    // step higher. A power of two has the float below it half as far
    // away as the float above, so now and then only the higher digits
    // name it: 2^-44 is 5.684341886080802e-14, and rounded to 16 digits
    // it ends in 801 (decisions 24.21 and 53.2).
    char buf[64];
    for (int precision = 1; precision <= 17; precision++) {
        snprintf(buf, sizeof buf, "%.*e", precision - 1, x);
        if (strtod(buf, nullptr) == x) break;
        if (stepHigher(buf) && strtod(buf, nullptr) == x) break;
    }
    string r = buf;
    size_t e = r.find('e');
    string mantissa = r.substr(0, e);
    int exp = atoi(r.c_str() + e + 1);
    string digits;
    for (char c : mantissa)
        if (c != '.') digits += c;
    while (digits.size() > 1 && digits.back() == '0') digits.pop_back();
    if (exp < -4 || exp >= 16) {
        return sign + digits[0] + (digits.size() > 1 ? "." + digits.substr(1) : "") + "e" + std::to_string(exp);
    }
    if (exp < 0) return sign + "0." + string(-exp - 1, '0') + digits;
    if ((int)digits.size() <= exp + 1) return sign + digits + string(exp + 1 - digits.size(), '0') + ".0";
    return sign + digits.substr(0, exp + 1) + "." + digits.substr(exp + 1);
}

// A float with a fixed number of decimals, as Rust's `{:.2}` writes it:
// from its exact value, a half rounding to the even digit (as C's own
// printf does).
string fixedPlaces(double x, int places) {
    if (std::isnan(x)) return "NaN";
    if (std::isinf(x)) return x > 0 ? "inf" : "-inf";
    int n = snprintf(nullptr, 0, "%.*f", places, x);
    string out(n + 1, '\0');
    snprintf(&out[0], n + 1, "%.*f", places, x);
    out.resize(n);
    return out;
}

// How Rust's `{:?}` shows a str: in quotes, with what cannot be seen escaped.
string quoted(const string& s) {
    string b = "\"";
    for (const string& ch : codePoints(s)) {
        int c = codePointAt(ch);
        if (c == '"') b += "\\\"";
        else if (c == '\\') b += "\\\\";
        else if (c == '\n') b += "\\n";
        else if (c == '\r') b += "\\r";
        else if (c == '\t') b += "\\t";
        else if (c == 0) b += "\\0";
        else if (c < 0x20 || (c >= 0x7f && c < 0xa0)) b += "\\u{" + hex(c) + "}";
        else b += ch;
    }
    return b + "\"";
}

string repr(const V& v);

// How `print`, `str()` and f-strings show a value.
string show(const V& v) {
    switch (v.t) {
    case V::INT: return num(v.i);
    case V::FLOAT: return showFloat(v.f);
    case V::F32: return showFloat(f32Shown(v.f));
    case V::U64: return std::to_string((uint64_t)v.i);
    case V::BOOL: return v.b ? "true" : "false";
    case V::NONE: return "none";
    case V::MISSING: return "?";
    case V::OBJ: break;
    }
    switch (v.p->k) {
    case K::STR: return str(v);
    case K::LIST: {
        std::vector<string> parts;
        for (auto& item : as<List>(v).items) parts.push_back(repr(item));
        return "[" + join(parts, ", ") + "]";
    }
    case K::DICT: {
        Dict& d = as<Dict>(v);
        if (d.keys.empty()) return "[:]";
        std::vector<string> parts;
        for (auto& k : d.keys) {
            Key key = k.t == V::INT ? Key{true, k.i, ""} : Key{false, 0, str(k)};
            parts.push_back(repr(k) + ": " + repr(d.map[key]));
        }
        return "[" + join(parts, ", ") + "]";
    }
    case K::QUANTITY: {
        Quantity& q = as<Quantity>(v);
        if (!std::isnan(q.v) && !std::isinf(q.v) && q.v == std::floor(q.v) && std::fabs(q.v) < 1e15) {
            return num((int64_t)q.v) + " " + q.unit;
        }
        return showFloat(q.v) + " " + q.unit;
    }
    case K::INSTANCE: {
        Instance& o = as<Instance>(v);
        std::vector<string> parts;
        for (size_t i = 0; i < o.cls->fields.size(); i++) parts.push_back(o.cls->fields[i] + "=" + repr(o.fields[i]));
        return shownName(o.cls->name) + "(" + join(parts, ", ") + ")";
    }
    case K::CASE: {
        Case& c = as<Case>(v);
        string name = shownName(c.def->cases[c.index]);
        if (c.fields.empty()) return name;
        std::vector<string> parts;
        for (auto& f : c.fields) parts.push_back(repr(f));
        return name + "(" + join(parts, ", ") + ")";
    }
    case K::ERROR: return as<Error>(v).message;
    case K::METHOD: return "<function " + as<Method>(v).name + ">";
    case K::BOUND: return "<function " + as<Bound>(v).fn->name + ">";
    case K::CLASS: return "<class " + as<Class>(v).name + ">";
    case K::ENUM: return "<enum " + as<Enum>(v).name + ">";
    case K::CTOR: return "<case " + as<Ctor>(v).def->cases[as<Ctor>(v).index] + ">";
    case K::ERRTYPE: return "<error " + as<ErrorType>(v).kind + ">";
    case K::MODULE: return "<module " + as<Module>(v).name + ">";
    case K::SEVERAL: {
        std::vector<string> parts;
        for (auto& item : as<Several>(v).values) parts.push_back(show(item));
        return "<" + num(parts.size()) + " values: " + join(parts, ", ") + ">";
    }
    case K::FUNC: {
        Func& f = as<Func>(v);
        return f.name.empty() ? "<function>" : "<function " + f.name + ">";
    }
    }
    return "?";
}

string repr(const V& v) { return v.is(K::STR) ? quoted(str(v)) : show(v); }

// --------------------------------------------------------------- Units

struct Unit {
    string kind;
    double size;
};

const std::unordered_map<string, Unit>& units() {
    static const std::unordered_map<string, Unit> u = [] {
        std::unordered_map<string, Unit> u;
        auto add = [&](const string& kind, std::vector<std::pair<string, double>> pairs) {
            for (auto& p : pairs) u[p.first] = Unit{kind, p.second};
        };
        add("time", {{"ns", 1}, {"us", 1e3}, {"ms", 1e6}, {"s", 1e9}, {"min", 60e9}, {"h", 3600e9}});
        add("frequency", {{"Hz", 1}, {"kHz", 1e3}, {"MHz", 1e6}});
        add("voltage", {{"uV", 1}, {"mV", 1e3}, {"V", 1e6}, {"kV", 1e9}});
        add("current", {{"uA", 1}, {"mA", 1e3}, {"A", 1e6}});
        add("resistance", {{"ohm", 1}, {"kohm", 1e3}, {"Mohm", 1e6}});
        add("power", {{"mW", 1}, {"W", 1e3}, {"kW", 1e6}});
        add("energy", {{"J", 1}, {"Wh", 3600}, {"kWh", 3600e3}});
        add("temperature in Celsius", {{"degC", 1}});
        add("temperature in kelvin", {{"K", 1}});
        add("length", {{"mm", 1}, {"cm", 10}, {"m", 1e3}, {"km", 1e6}});
        add("mass", {{"mg", 1}, {"g", 1e3}, {"kg", 1e6}});
        add("angle in degrees", {{"deg", 1}});
        add("angle in radians", {{"rad", 1}});
        add("rotation", {{"rpm", 1}});
        return u;
    }();
    return u;
}

struct Scale {
    bool ok;
    double times, over;
};

// How to turn a number in one unit into another.
Scale convert(const string& from, const string& to) {
    if (from == "deg" && to == "rad") return {true, M_PI / 180, 1};
    if (from == "rad" && to == "deg") return {true, 180 / M_PI, 1};
    auto f = units().find(from), t = units().find(to);
    if (f == units().end() || t == units().end() || f->second.kind != t->second.kind) return {false, 0, 0};
    return f->second.size >= t->second.size ? Scale{true, f->second.size / t->second.size, 1} : Scale{true, 1, t->second.size / f->second.size};
}

inline double apply(const Scale& scale, double x) { return x * scale.times / scale.over; }

// The unit two quantities meet in, the finer one, or "".
string common(const string& a, const string& b) {
    auto ua = units().find(a), ub = units().find(b);
    if (ua == units().end() || ub == units().end() || ua->second.kind != ub->second.kind) return "";
    return ua->second.size <= ub->second.size ? a : b;
}

V unit(const V& v, const string& name, int line) {
    if (!units().count(name)) {
        fatal("`" + name + "` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC", line);
    }
    if (!isNumber(v)) fatal("a unit can only follow a number", line);
    return Q(number(v), name);
}

V unit(const V& v, const V& name, int line) { return unit(v, str(name), line); }

// ------------------------------------------------------------- Equality

Key keyOf(const V& k) { return k.t == V::INT ? Key{true, k.i, ""} : Key{false, 0, str(k)}; }

bool equal(const V& a, const V& b) {
    if (a.t == V::INT && b.t == V::INT) return a.i == b.i;
    if (isU64(a, b)) {
        uint64_t x, y;
        return unsignedOf(a, x) && unsignedOf(b, y) && x == y;
    }
    if (isF32(a, b)) {
        double x, y;
        if (single(a, x) && single(b, y)) return x == y;
        return isNumber(a) && isNumber(b) && number(a) == number(b);
    }
    if (isNumber(a) && isNumber(b)) return number(a) == number(b);
    if (a.t == V::BOOL && b.t == V::BOOL) return a.b == b.b;
    if (a.t != V::OBJ || b.t != V::OBJ) return a.t == b.t && (a.t == V::NONE || a.t == V::MISSING);
    if (a.p == b.p) return true;
    if (a.p->k != b.p->k) return false;
    switch (a.p->k) {
    case K::STR: return str(a) == str(b);
    case K::QUANTITY: {
        Quantity &qa = as<Quantity>(a), &qb = as<Quantity>(b);
        string u = common(qa.unit, qb.unit);
        return !u.empty() && apply(convert(qa.unit, u), qa.v) == apply(convert(qb.unit, u), qb.v);
    }
    case K::LIST: {
        Args &la = as<List>(a).items, &lb = as<List>(b).items;
        if (la.size() != lb.size()) return false;
        for (size_t i = 0; i < la.size(); i++)
            if (!equal(la[i], lb[i])) return false;
        return true;
    }
    case K::DICT: {
        Dict &da = as<Dict>(a), &db = as<Dict>(b);
        if (da.keys.size() != db.keys.size()) return false;
        for (auto& k : da.keys) {
            auto other = db.map.find(keyOf(k));
            if (other == db.map.end() || !equal(da.map[keyOf(k)], other->second)) return false;
        }
        return true;
    }
    case K::INSTANCE: {
        Instance &ia = as<Instance>(a), &ib = as<Instance>(b);
        if (ia.cls != ib.cls) return false;
        for (size_t i = 0; i < ia.fields.size(); i++)
            if (!equal(ia.fields[i], ib.fields[i])) return false;
        return true;
    }
    case K::CASE: {
        Case &ca = as<Case>(a), &cb = as<Case>(b);
        if (ca.def != cb.def || ca.index != cb.index || ca.fields.size() != cb.fields.size()) return false;
        for (size_t i = 0; i < ca.fields.size(); i++)
            if (!equal(ca.fields[i], cb.fields[i])) return false;
        return true;
    }
    case K::ERROR: return as<Error>(a).kind == as<Error>(b).kind && as<Error>(a).message == as<Error>(b).message;
    case K::CTOR: return as<Ctor>(a).def == as<Ctor>(b).def && as<Ctor>(a).index == as<Ctor>(b).index;
    default: return false;
    }
}

bool eq(const V& a, const V& b) { return equal(a, b); }

bool ne(const V& a, const V& b) { return !equal(a, b); }

// ------------------------------------------------------------ Operators

[[noreturn]] void mismatch(const string& op, const V& a, const V& b, int line) {
    fatal("`" + op + "` does not work between " + typeName(a) + " and " + typeName(b), line);
}

inline int64_t addInt(int64_t a, int64_t b, int line) {
    int64_t r;
    if (__builtin_add_overflow(a, b, &r)) overflow(line);
    return r;
}

inline int64_t mulInt(int64_t a, int64_t b, int line) {
    int64_t r;
    if (__builtin_mul_overflow(a, b, &r)) overflow(line);
    return r;
}

V div(const V& a, const V& b, int line);
V floordiv(const V& a, const V& b, int line);
V mod(const V& a, const V& b, int line);
V pow(const V& a, const V& b, int line);

// ---- u64 (decision 42): the bits of a uint64_t.

const char* const tooBigU64 = "the result is too big for a u64";

V u64Op(const string& op, const V& a, const V& b, int line) {
    if (a.t == V::U64 && (op == "<<" || op == ">>") && (b.t == V::INT || b.t == V::U64)) {
        uint64_t x = (uint64_t)a.i;
        unsigned n = u64ShiftCount(b, line);
        if (op == ">>") return U64V(x >> n);
        if (((x << n) >> n) != x) raiseError("OverflowError", tooBigU64, line);
        return U64V(x << n);
    }
    uint64_t x, y;
    if (!unsignedOf(a, x) || !unsignedOf(b, y)) {
        fatal("`" + op + "` between " + typeName(a) + " and " + typeName(b) + " would have to choose between a u64 and something else. Write `u64( )` or `int( )` around one of them", line);
    }
    uint64_t r;
    if (op == "+") {
        if (__builtin_add_overflow(x, y, &r)) raiseError("OverflowError", tooBigU64, line);
        return U64V(r);
    }
    if (op == "-") {
        if (y > x) raiseError("OverflowError", "the result is below 0, and a u64 never is", line);
        return U64V(x - y);
    }
    if (op == "*") {
        if (__builtin_mul_overflow(x, y, &r)) raiseError("OverflowError", tooBigU64, line);
        return U64V(r);
    }
    if (op == "/") {
        if (y == 0) byZero(line);
        return F((double)x / (double)y);
    }
    if (op == "//" || op == "%") {
        if (y == 0) byZero(line);
        return U64V(op == "//" ? x / y : x % y);
    }
    if (op == "**") {
        if (y == 0) return U64V(1);
        if (x < 2) return U64V(x);
        r = 1;
        for (uint64_t i = 0; i < y; i++)
            if (__builtin_mul_overflow(r, x, &r)) raiseError("OverflowError", tooBigU64, line);
        return U64V(r);
    }
    if (op == "&") return U64V(x & y);
    if (op == "|") return U64V(x | y);
    return U64V(x ^ y);
}

V f32Op(const string& op, const V& a, const V& b, int line) {
    double x, y;
    if (!single(a, x) || !single(b, y)) {
        if (a.t == V::FLOAT || b.t == V::FLOAT) {
            fatal("`" + op + "` between " + typeName(a) + " and " + typeName(b) + " would have to choose between 32 bits and 64. Write `f32( )` around the float to work in 32, or `float( )` around the f32 to work in 64", line);
        }
        mismatch(op, a, b, line);
    }
    double r;
    if (op == "+") r = x + y;
    else if (op == "-") r = x - y;
    else if (op == "*") r = x * y;
    else if (op == "/") r = div(F(x), F(y), line).f;
    else if (op == "//") r = floordiv(F(x), F(y), line).f;
    else if (op == "%") r = mod(F(x), F(y), line).f;
    else r = pow(F(x), F(y), line).f;
    return F32V(r);
}

// What `*` and `/` make of two quantities (decision 43): the operator,
// what each side measures ("" for a plain number), and what the answer
// measures ("" for a plain number). The answer is in the unit of its kind
// with no prefix.
const std::unordered_map<string, string>& madeUnits() {
    static const std::unordered_map<string, string> m = {
        {"* voltage current", "power"}, {"* current voltage", "power"},
        {"* current resistance", "voltage"}, {"* resistance current", "voltage"},
        {"* power time", "energy"}, {"* time power", "energy"},
        {"* time frequency", ""}, {"* frequency time", ""},
        {"/ power voltage", "current"}, {"/ power current", "voltage"},
        {"/ voltage current", "resistance"}, {"/ voltage resistance", "current"},
        {"/ energy time", "power"}, {"/ energy power", "time"},
        {"/  time", "frequency"}, {"/  frequency", "time"},
    };
    return m;
}

const std::unordered_map<string, string>& plainUnits() {
    static const std::unordered_map<string, string> m = {
        {"time", "s"}, {"frequency", "Hz"}, {"voltage", "V"}, {"current", "A"}, {"resistance", "ohm"}, {"power", "W"}, {"energy", "J"},
    };
    return m;
}

// `a op b` for two quantities, or a number over a quantity.
V made(const string& op, const V& a, const V& b, int line) {
    const Quantity& qb = as<Quantity>(b);
    bool quantity = a.is(K::QUANTITY);
    string left = quantity ? units().at(as<Quantity>(a).unit).kind : "";
    string right = units().at(qb.unit).kind;
    auto found = madeUnits().find(op + " " + left + " " + right);
    if (found == madeUnits().end() || (!quantity && !isNumber(a))) mismatch(op, a, b, line);
    double x = quantity ? apply(convert(as<Quantity>(a).unit, plainUnits().at(left)), as<Quantity>(a).v) : number(a);
    if (op == "/" && qb.v == 0) byZero(line);
    double y = apply(convert(qb.unit, plainUnits().at(right)), qb.v);
    double r = op == "*" ? x * y : x / y;
    if (found->second.empty()) return F(r);
    return Q(r, plainUnits().at(found->second));
}

V add(const V& a, const V& b, int line) {
    if (isU64(a, b)) return u64Op("+", a, b, line);
    if (isF32(a, b)) return f32Op("+", a, b, line);
    if (a.t == V::INT && b.t == V::INT) return I(addInt(a.i, b.i, line));
    if (a.is(K::STR) && b.is(K::STR)) return S(str(a) + str(b));
    if (a.is(K::LIST) && b.is(K::LIST)) {
        Args items = as<List>(a).items;
        items.insert(items.end(), as<List>(b).items.begin(), as<List>(b).items.end());
        return O(std::make_shared<List>(std::move(items)));
    }
    if (a.is(K::QUANTITY) && b.is(K::QUANTITY)) {
        Quantity &qa = as<Quantity>(a), &qb = as<Quantity>(b);
        string u = common(qa.unit, qb.unit);
        if (u.empty()) mismatch("+", a, b, line);
        return Q(apply(convert(qa.unit, u), qa.v) + apply(convert(qb.unit, u), qb.v), u);
    }
    if (!isNumber(a) || !isNumber(b)) mismatch("+", a, b, line);
    return F(number(a) + number(b));
}

V sub(const V& a, const V& b, int line) {
    if (isU64(a, b)) return u64Op("-", a, b, line);
    if (isF32(a, b)) return f32Op("-", a, b, line);
    if (a.t == V::INT && b.t == V::INT) {
        int64_t r;
        if (__builtin_sub_overflow(a.i, b.i, &r)) overflow(line);
        return I(r);
    }
    if (a.is(K::QUANTITY) && b.is(K::QUANTITY)) {
        Quantity &qa = as<Quantity>(a), &qb = as<Quantity>(b);
        string u = common(qa.unit, qb.unit);
        if (u.empty()) mismatch("-", a, b, line);
        return Q(apply(convert(qa.unit, u), qa.v) - apply(convert(qb.unit, u), qb.v), u);
    }
    if (!isNumber(a) || !isNumber(b)) mismatch("-", a, b, line);
    return F(number(a) - number(b));
}

V mul(const V& a, const V& b, int line) {
    if (isU64(a, b)) return u64Op("*", a, b, line);
    if (isF32(a, b)) return f32Op("*", a, b, line);
    if (a.is(K::LIST) && b.t == V::INT) {
        if (b.i < 0) raiseError("ValueError", "`*` repeats a list 0 or more times, and this is " + std::to_string(b.i), line);
        Args items;
        for (int64_t t = 0; t < b.i; t++) items.insert(items.end(), as<List>(a).items.begin(), as<List>(a).items.end());
        return O(std::make_shared<List>(std::move(items)));
    }
    if (a.t == V::INT && b.t == V::INT) return I(mulInt(a.i, b.i, line));
    if (a.is(K::QUANTITY) && b.is(K::QUANTITY)) return made("*", a, b, line);
    if (a.is(K::QUANTITY) || b.is(K::QUANTITY)) {
        const V& qv = a.is(K::QUANTITY) ? a : b;
        const V& other = a.is(K::QUANTITY) ? b : a;
        if (!isNumber(other)) mismatch("*", a, b, line);
        return Q(as<Quantity>(qv).v * number(other), as<Quantity>(qv).unit);
    }
    if (!isNumber(a) || !isNumber(b)) mismatch("*", a, b, line);
    return F(number(a) * number(b));
}

V div(const V& a, const V& b, int line) {
    if (isU64(a, b)) return u64Op("/", a, b, line);
    if (isF32(a, b)) return f32Op("/", a, b, line);
    if (a.is(K::QUANTITY)) {
        Quantity& q = as<Quantity>(a);
        if (b.is(K::QUANTITY) && as<Quantity>(b).v == 0) byZero(line);
        if (b.t == V::FLOAT && b.f == 0) byZero(line);
        if (b.t == V::INT && b.i == 0) byZero(line);
        if (b.is(K::QUANTITY)) {
            Quantity& y = as<Quantity>(b);
            string u = common(q.unit, y.unit);
            if (u.empty()) return made("/", a, b, line);
            return F(apply(convert(q.unit, u), q.v) / apply(convert(y.unit, u), y.v));
        }
        if (!isNumber(b)) mismatch("/", a, b, line);
        return Q(q.v / number(b), q.unit);
    }
    if (b.is(K::QUANTITY)) return made("/", a, b, line);
    if (!isNumber(a) || !isNumber(b)) mismatch("/", a, b, line);
    if (number(b) == 0) byZero(line);
    return F(number(a) / number(b));
}

V floordiv(const V& a, const V& b, int line) {
    if (isU64(a, b)) return u64Op("//", a, b, line);
    if (isF32(a, b)) return f32Op("//", a, b, line);
    if (a.t == V::INT && b.t == V::INT) {
        int64_t x = a.i, y = b.i;
        if (y == 0) byZero(line);
        if (x == INT64_MIN && y == -1) overflow(line);
        int64_t q = x / y;
        if (x % y != 0 && (x < 0) != (y < 0)) q--;
        return I(q);
    }
    if (!isNumber(a) || !isNumber(b)) mismatch("//", a, b, line);
    if (number(b) == 0) byZero(line);
    return F(std::floor(number(a) / number(b)));
}

V mod(const V& a, const V& b, int line) {
    if (isU64(a, b)) return u64Op("%", a, b, line);
    if (isF32(a, b)) return f32Op("%", a, b, line);
    if (a.t == V::INT && b.t == V::INT) {
        int64_t x = a.i, y = b.i;
        if (y == 0) byZero(line);
        if (y == -1) return I(0);
        int64_t r = x % y;
        if (r != 0 && (r < 0) != (y < 0)) r += y;
        return I(r);
    }
    if (!isNumber(a) || !isNumber(b)) mismatch("%", a, b, line);
    double fx = number(a), fy = number(b);
    if (fy == 0) byZero(line);
    double fq = fx / fy;
    if (std::isnan(fq) || std::isinf(fq)) return F(fx - fy * fq);
    return F(fx - fy * std::floor(fq));
}

V pow(const V& a, const V& b, int line) {
    if (isU64(a, b)) return u64Op("**", a, b, line);
    if (isF32(a, b)) return f32Op("**", a, b, line);
    if (a.t == V::INT && b.t == V::INT) {
        int64_t x = a.i, n = b.i;
        if (n < 0) raiseError("ValueError", "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`", line);
        if (n > 4294967295LL) overflow(line);
        if (x == 0 || x == 1) return I(n == 0 ? 1 : x);
        if (x == -1) return I(n % 2 == 0 ? 1 : -1);
        if (n >= 64) overflow(line);
        int64_t r = 1;
        for (int64_t i = 0; i < n; i++) r = mulInt(r, x, line);
        return I(r);
    }
    if (!isNumber(a) || !isNumber(b)) mismatch("**", a, b, line);
    double fx = number(a), fy = number(b);
    if (fx == 1 || fy == 0) return F(1.0);
    return F(std::pow(fx, fy));
}

[[noreturn]] void bitsMismatch(const string& op, const V& a, const V& b, int line) {
    string hint;
    if (a.t == V::BOOL && b.t == V::BOOL) {
        if (op == "&") hint = ". For two bools Zeno spells this `and`";
        if (op == "|") hint = ". For two bools Zeno spells this `or`";
    }
    fatal("`" + op + "` works on the bits of whole numbers, and these are " + typeName(a) + " and " + typeName(b) + hint, line);
}

inline int64_t whole(const string& op, const V& v, const V& a, const V& b, int line) {
    if (v.t == V::INT) return v.i;
    bitsMismatch(op, a, b, line);
}

V bitand_(const V& a, const V& b, int line) {
    if (isU64(a, b)) return u64Op("&", a, b, line);
    return I(whole("&", a, a, b, line) & whole("&", b, a, b, line));
}

V bitor_(const V& a, const V& b, int line) {
    if (isU64(a, b)) return u64Op("|", a, b, line);
    return I(whole("|", a, a, b, line) | whole("|", b, a, b, line));
}

V bitxor_(const V& a, const V& b, int line) {
    if (isU64(a, b)) return u64Op("^", a, b, line);
    return I(whole("^", a, a, b, line) ^ whole("^", b, a, b, line));
}

void shift(int64_t n, int line) {
    if (n < 0 || n >= 64) raiseError("ValueError", "cannot shift by " + num(n) + ". An int has 64 bits, so a shift is 0 to 63", line);
}

V shl(const V& a, const V& b, int line) {
    if (isU64(a, b)) return u64Op("<<", a, b, line);
    int64_t x = whole("<<", a, a, b, line), n = whole("<<", b, a, b, line);
    shift(n, line);
    int64_t s = (int64_t)((uint64_t)x << n);
    if ((s >> n) != x) overflow(line);
    return I(s);
}

V shr(const V& a, const V& b, int line) {
    if (isU64(a, b)) return u64Op(">>", a, b, line);
    int64_t x = whole(">>", a, a, b, line), n = whole(">>", b, a, b, line);
    shift(n, line);
    return I(x >> n);
}

V neg(const V& v, int line) {
    if (v.t == V::INT) {
        if (v.i == INT64_MIN) overflow(line);
        return I(-v.i);
    }
    if (v.t == V::FLOAT) return F(-v.f);
    if (v.t == V::F32) return F32V(-v.f);
    if (v.t == V::U64) fatal("`-` does not work on a u64, which is never below 0. Make it an int first with `int( )`", line);
    if (v.is(K::QUANTITY)) return Q(-as<Quantity>(v).v, as<Quantity>(v).unit);
    fatal("`-` does not work on " + typeName(v), line);
}

V bitnot(const V& v, int line) {
    if (v.t == V::INT) return I(~v.i);
    if (v.t == V::U64) return U64V(~(uint64_t)v.i);
    fatal("`~` works on the bits of a whole number, and this is " + typeName(v), line);
}

// -1, 0 or 1, or 2 when the two cannot be ordered (a NaN).
int order(const string& op, const V& a, const V& b, int line) {
    if (isU64(a, b)) {
        uint64_t x, y;
        if (!unsignedOf(a, x) || !unsignedOf(b, y)) u64Op(op, a, b, line);
        return x < y ? -1 : x > y ? 1 : 0;
    }
    if (isF32(a, b)) {
        double x, y;
        if (!single(a, x) || !single(b, y)) f32Op(op, a, b, line);
        return x < y ? -1 : x > y ? 1 : x == y ? 0 : 2;
    }
    if (a.t == V::INT && b.t == V::INT) return a.i < b.i ? -1 : a.i > b.i ? 1 : 0;
    if (a.is(K::STR) && b.is(K::STR)) {
        // UTF-8 bytes are in the order of the code points, as Rust compares.
        int c = str(a).compare(str(b));
        return c < 0 ? -1 : c > 0 ? 1 : 0;
    }
    double x, y;
    if (a.is(K::QUANTITY) && b.is(K::QUANTITY)) {
        Quantity &qa = as<Quantity>(a), &qb = as<Quantity>(b);
        string u = common(qa.unit, qb.unit);
        if (u.empty()) mismatch(op, a, b, line);
        x = apply(convert(qa.unit, u), qa.v);
        y = apply(convert(qb.unit, u), qb.v);
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

bool lt(const V& a, const V& b, int line) { return order("<", a, b, line) == -1; }

bool le(const V& a, const V& b, int line) { int o = order("<=", a, b, line); return o == -1 || o == 0; }

bool gt(const V& a, const V& b, int line) { return order(">", a, b, line) == 1; }

bool ge(const V& a, const V& b, int line) { int o = order(">=", a, b, line); return o == 1 || o == 0; }

bool contains(const string& op, const V& item, const V& container, int line) {
    if (container.is(K::LIST)) {
        for (auto& other : as<List>(container).items)
            if (equal(item, other)) return true;
        return false;
    }
    if (container.is(K::DICT)) {
        if (item.t != V::INT && !item.is(K::STR)) mismatch(op, item, container, line);
        return as<Dict>(container).map.count(keyOf(item)) > 0;
    }
    if (container.is(K::STR) && item.is(K::STR)) return str(container).find(str(item)) != string::npos;
    mismatch(op, item, container, line);
}

bool in(const V& a, const V& b, int line) { return contains("in", a, b, line); }

bool not_in(const V& a, const V& b, int line) { return !contains("not in", a, b, line); }

bool truth(const V& v, int line) {
    if (v.t == V::BOOL) return v.b;
    fatal("a condition must be true or false, but this is " + typeName(v), line);
}

// ------------------------------------------------------- Lists and text

inline V newList(Args items) { return O(std::make_shared<List>(std::move(items))); }

const V& key(const V& v, int line) {
    if (v.t == V::INT || v.is(K::STR)) return v;
    fatal("a key of a dict is a str or an int, but this is " + typeName(v), line);
}

void dictSet(Dict& d, const V& k, const V& v) {
    Key kk = keyOf(k);
    auto at = d.map.find(kk);
    if (at == d.map.end()) {
        d.keys.push_back(k);
        d.map.emplace(std::move(kk), v);
    } else {
        at->second = v;
    }
}

void dictRemove(Dict& d, const V& k) {
    Key kk = keyOf(k);
    d.map.erase(kk);
    for (size_t i = 0; i < d.keys.size(); i++) {
        if (keyOf(d.keys[i]) == kk) {
            d.keys.erase(d.keys.begin() + i);
            return;
        }
    }
}

size_t position(const V& index, int64_t length, const string& what, int line) {
    if (index.t != V::INT) fatal("an index must be an int, but this is " + typeName(index), line);
    int64_t i = index.i;
    if (i < 0 || i >= length) raiseError("IndexError", "index " + num(i) + " is out of range for a " + what + " of length " + num(length), line);
    return (size_t)i;
}

V index(const V& obj, const V& i, int line) {
    if (obj.is(K::LIST)) {
        Args& items = as<List>(obj).items;
        return items[position(i, items.size(), "list", line)];
    }
    if (obj.is(K::STR)) {
        int64_t at = position(i, textLength(obj), "str", line);
        return S(sub(obj, at, at + 1));
    }
    if (obj.is(K::DICT)) {
        Dict& d = as<Dict>(obj);
        auto at = d.map.find(keyOf(key(i, line)));
        if (at == d.map.end()) raiseError("KeyError", "this dict has no key " + repr(i), line);
        return at->second;
    }
    fatal("cannot take an item from " + typeName(obj), line);
}

void set_index(const V& obj, const V& i, const V& value, int line) {
    if (obj.is(K::LIST)) {
        Args& items = as<List>(obj).items;
        items[position(i, items.size(), "list", line)] = value;
        return;
    }
    if (obj.is(K::DICT)) {
        dictSet(as<Dict>(obj), key(i, line), value);
        return;
    }
    fatal("cannot set an item of " + typeName(obj), line);
}

Args items(const V& v, int line) {
    if (v.is(K::LIST)) return as<List>(v).items;
    if (v.is(K::DICT)) return as<Dict>(v).keys;
    if (v.is(K::STR)) {
        Args out;
        for (auto& c : codePoints(str(v))) out.push_back(S(c));
        return out;
    }
    fatal("cannot loop over " + typeName(v), line);
}

std::vector<std::array<V, 2>> pairs(const V& v, int line) {
    std::vector<std::array<V, 2>> out;
    if (v.is(K::DICT)) {
        Dict& d = as<Dict>(v);
        for (auto& k : d.keys) out.push_back({k, d.map[keyOf(k)]});
        return out;
    }
    Args all = items(v, line);
    for (size_t i = 0; i < all.size(); i++) out.push_back({I((int64_t)i), all[i]});
    return out;
}

V list(const Args& values) { return newList(values); }

V dict(const Args& flat, int line) {
    auto d = std::make_shared<Dict>();
    for (size_t i = 0; i + 1 < flat.size(); i += 2) dictSet(*d, key(flat[i], line), flat[i + 1]);
    return O(d);
}

V push(const V& list, const V& value) {
    as<List>(list).items.push_back(value);
    return V();
}

V concat(const Args& pieces) {
    string b;
    for (auto& p : pieces) b += str(p);
    return S(std::move(b));
}

// --------------------------------------------------- Objects and fields

V fn(const string& name, Names parameters, Flags defaults, Body body) {
    return O(std::make_shared<Func>(name, std::move(parameters), std::move(defaults), std::move(body), "", nullptr));
}

V cls(const string& name, Names fields, Args defaults, const Args& methods) {
    auto c = std::make_shared<Class>(name, std::move(fields), std::move(defaults));
    for (size_t i = 0; i + 1 < methods.size(); i += 2) c->methods.push_back({str(methods[i]), ptr<Func>(methods[i + 1])});
    return O(c);
}

V enumOf(const string& name, Names cases, std::vector<Names> fields) {
    return O(std::make_shared<Enum>(name, std::move(cases), std::move(fields)));
}

V case_(const V& def, int i) {
    auto e = ptr<Enum>(def);
    if (e->fields[i].empty()) return O(std::make_shared<Case>(e, i, Args{}));
    return O(std::make_shared<Ctor>(e, i));
}

bool is_case(const V& value, const V& ctor, int count, int line) {
    if (!ctor.is(K::CTOR)) fatal("this is not an enum case that carries values", line);
    Ctor& c = as<Ctor>(ctor);
    if (!value.is(K::CASE) || as<Case>(value).def != c.def || as<Case>(value).index != c.index) return false;
    Case& v = as<Case>(value);
    if ((int)v.fields.size() != count) {
        fatal("`" + c.def->cases[c.index] + "` carries " + num(v.fields.size()) + " values, but this pattern has " + num(count), line);
    }
    return true;
}

V case_field(const V& value, int i) { return as<Case>(value).fields[i]; }

V several(const Args& values) { return O(std::make_shared<Several>(values)); }

// What place `at` of the `count` on the left of `=` is given.
V received(const V& value, int count, int at, int line) {
    if (!value.is(K::SEVERAL)) fatal("there are " + num(count) + " places on the left of `=`, and the right gives one value, " + typeName(value), line);
    Several& s = as<Several>(value);
    if ((int)s.values.size() != count) fatal("this gives " + num(s.values.size()) + " values, and there are " + num(count) + " places on the left of `=`", line);
    return s.values[at];
}

int indexOf(const Names& names, const string& name) {
    for (size_t i = 0; i < names.size(); i++)
        if (names[i] == name) return (int)i;
    return -1;
}

struct Width {
    int64_t lo, hi;
    int bits;
    bool isSigned;
};

const std::vector<std::pair<string, Width>> WIDTHS = {
    {"u8", {0, 255, 8, false}},
    {"u16", {0, 65535, 16, false}},
    {"u32", {0, 4294967295LL, 32, false}},
    {"i8", {-128, 127, 8, true}},
    {"i16", {-32768, 32767, 16, true}},
    {"i32", {-2147483648LL, 2147483647LL, 32, true}},
};

const Width* width(const string& name) {
    for (auto& w : WIDTHS)
        if (w.first == name) return &w.second;
    return nullptr;
}

V builtin(const string& name, int line);
bool hasMethod(const V& v, const string& name);

V field(const V& obj, const string& name, int line) {
    if (obj.is(K::INSTANCE)) {
        Instance& o = as<Instance>(obj);
        int at = indexOf(o.cls->fields, name);
        if (at >= 0) return o.fields[at];
        if (auto m = o.cls->method(name)) return O(std::make_shared<Bound>(obj, m));
        fatal("`" + o.cls->name + "` has no field or method `" + name + "`", line);
    }
    if (obj.is(K::CASE)) {
        Case& c = as<Case>(obj);
        int at = indexOf(c.def->fields[c.index], name);
        if (at >= 0) return c.fields[at];
        fatal("`" + c.def->cases[c.index] + "` has no field `" + name + "`", line);
    }
    if (obj.is(K::ENUM)) {
        Enum& e = as<Enum>(obj);
        int at = indexOf(e.cases, name);
        if (at >= 0) return case_(obj, at);
        fatal("enum `" + e.name + "` has no case `" + name + "`", line);
    }
    if (obj.is(K::MODULE)) return builtin(as<Module>(obj).name + "." + name, line);
    if (obj.is(K::QUANTITY) && units().count(name)) {
        Quantity& q = as<Quantity>(obj);
        Scale s = convert(q.unit, name);
        if (!s.ok) fatal("a quantity in " + q.unit + " cannot be turned into " + name + ". They measure different things", line);
        return Q(apply(s, q.v), name);
    }
    if (obj.is(K::ERROR) && name == "message") return S(as<Error>(obj).message);
    if (obj.is(K::ERROR) && name == "kind") return S(as<Error>(obj).kind);
    if (obj.is(K::FUNC) && !as<Func>(obj).builtin.empty() && width(as<Func>(obj).builtin) && name == "wrap") {
        return builtin(as<Func>(obj).builtin + ".wrap", line);
    }
    if (hasMethod(obj, name)) return O(std::make_shared<Method>(obj, name));
    fatal(typeName(obj) + " has no field or method `" + name + "`", line);
}

V field(const V& obj, const V& name, int line) { return field(obj, str(name), line); }

void set_field(const V& obj, const string& name, const V& value, int line) {
    if (obj.is(K::INSTANCE)) {
        Instance& o = as<Instance>(obj);
        int at = indexOf(o.cls->fields, name);
        if (at < 0) fatal("`" + o.cls->name + "` has no field `" + name + "`", line);
        o.fields[at] = value;
        return;
    }
    fatal("cannot set a field on " + typeName(obj), line);
}

// ---------------------------------------------------------------- Calls

string counted(int64_t n, const string& word) { return n == 1 ? "1 " + word : num(n) + " " + word + "s"; }

string were(int64_t n) { return n == 1 ? "1 was" : num(n) + " were"; }

Args bindArgs(const string& what, const Names& names, const Args& positional, const Names& namedNames, const Args& named, int line) {
    if (positional.size() > names.size()) {
        fatal("`" + what + "` takes " + counted(names.size(), "argument") + ", but " + were(positional.size()) + " given", line);
    }
    Args slots(names.size());
    for (size_t i = 0; i < names.size(); i++) slots[i] = i < positional.size() ? positional[i] : MISSING;
    for (size_t i = 0; i < namedNames.size(); i++) {
        int at = indexOf(names, namedNames[i]);
        if (at < 0) fatal("`" + what + "` has no argument named `" + namedNames[i] + "`", line);
        if (slots[at].t != V::MISSING) fatal("`" + what + "` was given `" + namedNames[i] + "` twice", line);
        slots[at] = named[i];
    }
    return slots;
}

// How many calls are nested now. `zeno run` stops at 1000.
int depth = 0;

struct Enter {
    Enter(const Func& f, int line) {
        if (depth >= 1000) {
            string name = f.name.empty() ? "function" : f.name;
            fatal("more than 1000 calls are nested. Is `" + name + "` calling itself forever?", line);
        }
        depth++;
    }
    ~Enter() { depth--; }
};

// A call of a function of the program known where it is written, with
// its arguments lined up already.
V direct(const V& f, int line, const Args& args) {
    Func& fn = as<Func>(f);
    Enter enter(fn, line);
    return fn.body(args);
}

const Names NO_NAMES;
const Args NO_VALUES;

V callFunction(Func& f, const Args& positional, const Names& namedNames, const Args& named, int line) {
    string name = f.name.empty() ? "function" : f.name;
    Enter enter(f, line);
    Args slots = bindArgs(name, f.parameters, positional, namedNames, named, line);
    for (size_t i = 0; i < slots.size(); i++) {
        if (slots[i].t == V::MISSING && !f.defaults[i]) fatal("`" + name + "` needs a value for `" + f.parameters[i] + "`", line);
    }
    return f.body(slots);
}

void noNamed(const string& what, const Names& namedNames, int line) {
    if (!namedNames.empty()) fatal("`" + what + "` does not take a named argument `" + namedNames[0] + "`", line);
}

Args prepend(const V& first, const Args& rest) {
    Args out;
    out.reserve(rest.size() + 1);
    out.push_back(first);
    out.insert(out.end(), rest.begin(), rest.end());
    return out;
}

V callMethodOf(const V& receiver, const string& name, const Args& args, int line);
V newError(const string& kind, const Args& args, int line);
V call_builtin(const string& name, const Args& args, int line);

V call_named(const V& callee, const Args& positional, const Names& namedNames, const Args& named, int line) {
    if (callee.t == V::OBJ) {
        switch (callee.p->k) {
        case K::BOUND: {
            Bound& b = as<Bound>(callee);
            return callFunction(*b.fn, prepend(b.obj, positional), namedNames, named, line);
        }
        case K::METHOD: {
            Method& m = as<Method>(callee);
            noNamed(m.name, namedNames, line);
            return callMethodOf(m.receiver, m.name, positional, line);
        }
        case K::CLASS: {
            auto c = ptr<Class>(callee);
            Args slots = bindArgs(c->name, c->fields, positional, namedNames, named, line);
            for (size_t i = 0; i < slots.size(); i++) {
                if (slots[i].t == V::MISSING) {
                    if (c->defaults[i].t == V::NONE) fatal("`" + c->name + "` needs a value for `" + c->fields[i] + "`", line);
                    slots[i] = as<Func>(c->defaults[i]).body(NO_VALUES);
                }
            }
            return O(std::make_shared<Instance>(c, std::move(slots)));
        }
        case K::CTOR: {
            Ctor& k = as<Ctor>(callee);
            const string& name = k.def->cases[k.index];
            const Names& fields = k.def->fields[k.index];
            Args slots = bindArgs(name, fields, positional, namedNames, named, line);
            for (size_t i = 0; i < slots.size(); i++) {
                if (slots[i].t == V::MISSING) fatal("`" + name + "` needs a value for `" + fields[i] + "`", line);
            }
            return O(std::make_shared<Case>(k.def, k.index, std::move(slots)));
        }
        case K::ERRTYPE: {
            ErrorType& e = as<ErrorType>(callee);
            noNamed(e.kind, namedNames, line);
            return newError(e.kind, positional, line);
        }
        case K::FUNC: {
            Func& f = as<Func>(callee);
            if (f.bbody) {
                noNamed(f.name, namedNames, line);
                return f.bbody(positional, line);
            }
            return callFunction(f, positional, namedNames, named, line);
        }
        default: break;
        }
    }
    fatal(typeName(callee) + " cannot be called", line);
}

V call(const V& callee, const Args& args, int line) { return call_named(callee, args, NO_NAMES, NO_VALUES, line); }

V newError(const string& kind, const Args& args, int line) {
    if (args.size() > 1) fatal("`" + kind + "` takes one message", line);
    return O(std::make_shared<Error>(kind, args.size() == 1 ? show(args[0]) : "", V()));
}

V call_method_named(const V& obj, const string& name, const Args& positional, const Names& namedNames, const Args& named, int line) {
    if (obj.is(K::INSTANCE)) {
        Instance& o = as<Instance>(obj);
        if (indexOf(o.cls->fields, name) < 0) {
            if (auto m = o.cls->method(name)) return callFunction(*m, prepend(obj, positional), namedNames, named, line);
        }
    }
    if (obj.is(K::MODULE)) {
        noNamed(name, namedNames, line);
        return call_builtin(as<Module>(obj).name + "." + name, positional, line);
    }
    if (obj.is(K::STR) || obj.is(K::LIST) || obj.is(K::DICT)) {
        noNamed(name, namedNames, line);
        return callMethodOf(obj, name, positional, line);
    }
    return call_named(field(obj, name, line), positional, namedNames, named, line);
}

V call_method(const V& obj, const string& name, const Args& args, int line) {
    return call_method_named(obj, name, args, NO_NAMES, NO_VALUES, line);
}

// --------------------------------------------------------------- Errors

void raise(const V& value, int line) {
    if (value.is(K::ERROR)) throw Raise{ptr<Error>(value), line};
    if (value.is(K::INSTANCE)) throw Raise{std::make_shared<Error>(as<Instance>(value).cls->name, show(value), value), line};
    fatal("only an error can be raised, and this is " + typeName(value), line);
}

bool catches(const Raise& e, const string& name) { return name == "Error" || name == e.value->kind; }

bool catches(const Raise& e, const V& name) { return catches(e, str(name)); }

V caught(const Raise& e, const string& name) {
    if (!name.empty() && name != "Error" && e.value->obj.t != V::NONE) return e.value->obj;
    return O(e.value);
}

V caught(const Raise& e, const V& name) { return caught(e, str(name)); }

void no_match(const V& v, int line) { fatal("no `case` matches " + repr(v), line); }

V undefined(const string& name, int line) { fatal("`" + name + "` is not defined", line); }

V undefined(const V& name, int line) { return undefined(str(name), line); }

// --------------------------------------------------- Built-in functions

void arity(const string& name, const Args& args, size_t lo, size_t hi, int line) {
    if (args.size() >= lo && args.size() <= hi) return;
    string wanted = lo == hi ? counted(lo, "argument") : num(lo) + " to " + num(hi) + " arguments";
    fatal("`" + name + "` takes " + wanted + ", but " + were(args.size()) + " given", line);
}

[[noreturn]] void wrongType(const string& name, const string& wanted, const V& got, int line) {
    fatal("`" + name + "` needs " + wanted + ", but this is " + typeName(got), line);
}

const string& text(const string& name, const V& v, int line) {
    if (v.is(K::STR)) return str(v);
    wrongType(name, "a str", v, line);
}

int64_t wholeArg(const string& name, const V& v, int line) {
    if (v.t == V::INT) return v.i;
    wrongType(name, "a whole number", v, line);
}

inline bool blank(char c) { return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\f'; }

string strip(const string& s) {
    size_t a = 0, b = s.size();
    while (a < b && blank(s[a])) a++;
    while (b > a && blank(s[b - 1])) b--;
    return s.substr(a, b - a);
}

// What the program prints waits here, and goes out in big pieces.
string OUT;

void flush() {
    if (!OUT.empty()) {
        fwrite(OUT.data(), 1, OUT.size(), stdout);
        OUT.clear();
    }
    fflush(stdout);
}

inline void write(const string& s) {
    OUT += s;
    if (OUT.size() > 65536) flush();
}

inline double amount(const V& v) { return v.is(K::QUANTITY) ? as<Quantity>(v).v : number(v); }

Args numbers(const string& name, const Args& args, int line, string& unit) {
    Args values = args.size() == 1 && args[0].is(K::LIST) ? as<List>(args[0]).items : args;
    unit = "";
    for (size_t place = 0; place < values.size(); place++) {
        const V& item = values[place];
        if (item.is(K::QUANTITY)) {
            const string& u = as<Quantity>(item).unit;
            if (!unit.empty() && unit != u) {
                fatal("`" + name + "` needs quantities in one unit, but these are in " + unit + " and in " + u + ". Turn one into the other first, as in `x." + unit + "`", line);
            }
            if (unit.empty() && place > 0) fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line);
            unit = u;
        } else if (!isNumber(item)) {
            wrongType(name, "numbers", item, line);
        } else if (!unit.empty()) {
            fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line);
        }
    }
    return values;
}

V pick(const string& name, const Args& args, int line, bool smaller) {
    string unit;
    Args values = numbers(name, args, line, unit);
    if (values.empty()) raiseError("ValueError", "`" + name + "` of nothing", line);
    if (allSingle(values)) {
        for (auto& v : values) {
            double x;
            single(v, x);
            v = F32V(x);
        }
    }
    V best = values[0];
    for (size_t i = 1; i < values.size(); i++) {
        const V& item = values[i];
        if (item.t == V::U64 && best.t == V::U64) {
            uint64_t x = (uint64_t)item.i, y = (uint64_t)best.i;
            if (smaller ? x < y : x > y) best = item;
            continue;
        }
        if (smaller ? amount(item) < amount(best) : amount(item) > amount(best)) best = item;
    }
    return best;
}

inline bool digit(char c) { return c >= '0' && c <= '9'; }

string lowerAscii(string s) {
    for (char& c : s)
        if (c >= 'A' && c <= 'Z') c = (char)(c + 32);
    return s;
}

// A float as Rust's `parse::<f64>` reads it.
bool parseFloat(const string& s, double& out) {
    size_t i = 0, n = s.size();
    bool negative = false;
    if (i < n && (s[i] == '+' || s[i] == '-')) negative = s[i++] == '-';
    string body = lowerAscii(s.substr(i));
    if (body == "inf" || body == "infinity") { out = negative ? -INFINITY : INFINITY; return true; }
    if (body == "nan") { out = NAN; return true; }
    size_t start = i;
    size_t whole = 0, fraction = 0;
    while (i < n && digit(s[i])) { i++; whole++; }
    if (i < n && s[i] == '.') {
        i++;
        while (i < n && digit(s[i])) { i++; fraction++; }
    }
    if (whole == 0 && fraction == 0) return false;
    if (i < n && (s[i] == 'e' || s[i] == 'E')) {
        i++;
        if (i < n && (s[i] == '+' || s[i] == '-')) i++;
        size_t exp = 0;
        while (i < n && digit(s[i])) { i++; exp++; }
        if (exp == 0) return false;
    }
    if (i != n) return false;
    (void)start;
    out = strtod(s.c_str(), nullptr);
    return true;
}

// Heapsort, step for step as the Rust code sorts, so that a function
// given to `sort` is called in the same order.
void heapsort(int n, const std::function<bool(int, int)>& before, const std::function<void(int, int)>& swap) {
    auto sift = [&](int root, int end) {
        while (true) {
            int child = 2 * root + 1;
            if (child >= end) return;
            if (child + 1 < end && before(child, child + 1)) child++;
            if (!before(root, child)) return;
            swap(root, child);
            root = child;
        }
    };
    for (int root = n / 2 - 1; root >= 0; root--) sift(root, n);
    for (int end = n - 1; end > 0; end--) {
        swap(0, end);
        sift(0, end);
    }
}

void sortList(List& l, int line) {
    Args s = l.items;
    heapsort((int)s.size(), [&](int i, int j) { return lt(s[i], s[j], line); }, [&](int i, int j) { std::swap(s[i], s[j]); });
    l.items = s;
}

void sortBy(List& l, const V& function, int line) {
    Args keys;
    Args copy = l.items;
    for (auto& item : copy) {
        V k = call(function, {item}, line);
        if (k.t != V::INT && k.t != V::FLOAT && k.t != V::F32 && k.t != V::U64 && !k.is(K::STR)) {
            fatal("the function given to `sort` must return a number or text to order by, but it returned " + typeName(k), line);
        }
        keys.push_back(k);
    }
    Args s = l.items;
    if (s.size() != keys.size()) raiseError("ValueError", "the function given to `sort` changed how many items the list has", line);
    std::vector<int> places;
    for (size_t i = 0; i < s.size(); i++) places.push_back((int)i);
    heapsort(
        (int)s.size(),
        [&](int a, int b) {
            if (lt(keys[a], keys[b], line)) return true;
            if (lt(keys[b], keys[a], line)) return false;
            return places[a] < places[b];
        },
        [&](int i, int j) {
            std::swap(s[i], s[j]);
            std::swap(keys[i], keys[j]);
            std::swap(places[i], places[j]);
        });
    l.items = s;
}

std::pair<int64_t, int64_t> slice(const Args& args, int64_t length, const string& what, int line) {
    int64_t from = wholeArg("slice", args[0], line);
    int64_t to = args.size() > 1 ? wholeArg("slice", args[1], line) : length;
    if (from < 0 || to < from || to > length) {
        raiseError("IndexError", "slice " + num(from) + " to " + num(to) + " is out of range for a " + what + " of length " + num(length), line);
    }
    return {from, to};
}

const Names STR_METHODS = {"upper", "lower", "strip", "split", "lines", "replace", "starts_with", "ends_with", "find", "slice", "code"};
const Names LIST_METHODS = {"append", "pop", "map", "filter", "reduce", "insert", "reverse", "sort", "clear", "find", "slice", "join"};
const Names DICT_METHODS = {"get", "pop", "keys", "values", "clear"};

bool hasMethod(const V& v, const string& name) {
    if (v.is(K::STR)) return indexOf(STR_METHODS, name) >= 0;
    if (v.is(K::LIST)) return indexOf(LIST_METHODS, name) >= 0;
    if (v.is(K::DICT)) return indexOf(DICT_METHODS, name) >= 0;
    return false;
}

V strings(const std::vector<string>& parts) {
    Args items;
    for (auto& p : parts) items.push_back(S(p));
    return newList(std::move(items));
}

std::vector<string> splitOn(const string& s, const string& sep) {
    std::vector<string> out;
    size_t start = 0;
    while (true) {
        size_t at = s.find(sep, start);
        if (at == string::npos) {
            out.push_back(s.substr(start));
            return out;
        }
        out.push_back(s.substr(start, at - start));
        start = at + sep.size();
    }
}

string replaceAll(const string& s, const string& old, const string& now) {
    string out;
    size_t start = 0;
    while (true) {
        size_t at = s.find(old, start);
        if (at == string::npos) {
            out += s.substr(start);
            return out;
        }
        out += s.substr(start, at - start);
        out += now;
        start = at + old.size();
    }
}

V callMethodOf(const V& receiver, const string& name, const Args& args, int line) {
    if (receiver.is(K::STR)) {
        const string& s = str(receiver);
        if (name == "upper" || name == "lower") {
            arity(name, args, 0, 0, line);
            string cs = s;
            for (char& c : cs) {
                if (name == "upper" && c >= 'a' && c <= 'z') c = (char)(c - 32);
                else if (name == "lower" && c >= 'A' && c <= 'Z') c = (char)(c + 32);
            }
            return S(cs);
        }
        if (name == "strip") {
            arity(name, args, 0, 0, line);
            return S(strip(s));
        }
        if (name == "split") {
            arity(name, args, 0, 1, line);
            if (args.empty()) {
                std::vector<string> words;
                size_t i = 0;
                while (i < s.size()) {
                    while (i < s.size() && blank(s[i])) i++;
                    size_t start = i;
                    while (i < s.size() && !blank(s[i])) i++;
                    if (i > start) words.push_back(s.substr(start, i - start));
                }
                return strings(words);
            }
            const string& sep = text(name, args[0], line);
            if (sep.empty()) raiseError("ValueError", "cannot split on an empty str", line);
            return strings(splitOn(s, sep));
        }
        if (name == "lines") {
            arity(name, args, 0, 0, line);
            if (s.empty()) return newList({});
            std::vector<string> parts = splitOn(s, "\n");
            // A "\r" goes with the "\n" just after it, and with nothing else.
            string last = parts.back();
            parts.pop_back();
            for (auto& p : parts)
                if (!p.empty() && p.back() == '\r') p.pop_back();
            if (!last.empty()) parts.push_back(last);
            return strings(parts);
        }
        if (name == "replace") {
            arity(name, args, 2, 2, line);
            const string &old = text(name, args[0], line), &now = text(name, args[1], line);
            if (old.empty()) {
                string b = now;
                for (auto& c : codePoints(s)) b += c + now;
                return S(b);
            }
            return S(replaceAll(s, old, now));
        }
        if (name == "starts_with") {
            arity(name, args, 1, 1, line);
            const string& p = text(name, args[0], line);
            return B(s.compare(0, p.size(), p) == 0 && s.size() >= p.size());
        }
        if (name == "ends_with") {
            arity(name, args, 1, 1, line);
            const string& p = text(name, args[0], line);
            return B(s.size() >= p.size() && s.compare(s.size() - p.size(), p.size(), p) == 0);
        }
        if (name == "find") {
            arity(name, args, 1, 1, line);
            size_t at = s.find(text(name, args[0], line));
            if (at == string::npos) return V();
            return I(textLength(s.substr(0, at)));
        }
        if (name == "slice") {
            arity(name, args, 1, 2, line);
            auto [from, to] = slice(args, textLength(receiver), "str", line);
            return S(sub(receiver, from, to));
        }
        if (name == "code") {
            arity(name, args, 0, 0, line);
            int64_t n = textLength(receiver);
            if (n != 1) raiseError("ValueError", "`code` needs a str of exactly one character, and this one has " + num(n), line);
            return I(codePointAt(s));
        }
        fatal("a str has no method `" + name + "`", line);
    }
    if (receiver.is(K::DICT)) {
        Dict& d = as<Dict>(receiver);
        if (name == "get") {
            arity(name, args, 1, 2, line);
            auto at = d.map.find(keyOf(key(args[0], line)));
            if (at != d.map.end()) return at->second;
            return args.size() > 1 ? args[1] : V();
        }
        if (name == "pop") {
            arity(name, args, 1, 1, line);
            const V& k = key(args[0], line);
            auto at = d.map.find(keyOf(k));
            if (at == d.map.end()) raiseError("KeyError", "this dict has no key " + repr(args[0]), line);
            V v = at->second;
            dictRemove(d, k);
            return v;
        }
        if (name == "keys") {
            arity(name, args, 0, 0, line);
            return newList(d.keys);
        }
        if (name == "values") {
            arity(name, args, 0, 0, line);
            Args values;
            for (auto& k : d.keys) values.push_back(d.map[keyOf(k)]);
            return newList(std::move(values));
        }
        if (name == "clear") {
            arity(name, args, 0, 0, line);
            d.keys.clear();
            d.map.clear();
            return V();
        }
        fatal("a dict has no method `" + name + "`", line);
    }
    if (receiver.is(K::LIST)) {
        List& l = as<List>(receiver);
        if (name == "append") {
            arity(name, args, 1, 1, line);
            l.items.push_back(args[0]);
            return V();
        }
        if (name == "pop") {
            arity(name, args, 0, 1, line);
            if (!args.empty()) {
                size_t at = position(args[0], l.items.size(), "list", line);
                V v = l.items[at];
                l.items.erase(l.items.begin() + at);
                return v;
            }
            if (l.items.empty()) raiseError("IndexError", "cannot pop from an empty list", line);
            V last = l.items.back();
            l.items.pop_back();
            return last;
        }
        if (name == "insert") {
            arity(name, args, 2, 2, line);
            int64_t at = wholeArg(name, args[0], line);
            if (at < 0 || at > (int64_t)l.items.size()) {
                raiseError("IndexError", "cannot insert at " + num(at) + " in a list of length " + num(l.items.size()), line);
            }
            l.items.insert(l.items.begin() + at, args[1]);
            return V();
        }
        if (name == "reverse") {
            arity(name, args, 0, 0, line);
            std::reverse(l.items.begin(), l.items.end());
            return V();
        }
        if (name == "clear") {
            arity(name, args, 0, 0, line);
            l.items.clear();
            return V();
        }
        if (name == "sort") {
            if (args.size() == 1) {
                sortBy(l, args[0], line);
                return V();
            }
            arity(name, args, 0, 1, line);
            sortList(l, line);
            return V();
        }
        if (name == "find") {
            arity(name, args, 1, 1, line);
            for (size_t at = 0; at < l.items.size(); at++)
                if (equal(l.items[at], args[0])) return I((int64_t)at);
            return V();
        }
        if (name == "slice") {
            arity(name, args, 1, 2, line);
            auto [from, to] = slice(args, l.items.size(), "list", line);
            return newList(Args(l.items.begin() + from, l.items.begin() + to));
        }
        if (name == "join") {
            arity(name, args, 1, 1, line);
            const string& sep = text(name, args[0], line);
            std::vector<string> parts;
            for (auto& item : l.items) {
                if (!item.is(K::STR)) {
                    fatal("`join` puts text together, and this list holds " + typeName(item) + ". Make each item text first, with `str( )`", line);
                }
                parts.push_back(str(item));
            }
            return S(join(parts, sep));
        }
        if (name == "map") {
            arity(name, args, 1, 1, line);
            Args out;
            Args copy = l.items;
            for (auto& item : copy) out.push_back(call(args[0], {item}, line));
            return newList(std::move(out));
        }
        if (name == "filter") {
            arity(name, args, 1, 1, line);
            Args kept;
            Args copy = l.items;
            for (auto& item : copy) {
                V keep = call(args[0], {item}, line);
                if (keep.t != V::BOOL) {
                    fatal("the function given to `filter` must return true or false, but it returned " + typeName(keep), line);
                }
                if (keep.b) kept.push_back(item);
            }
            return newList(std::move(kept));
        }
        if (name == "reduce") {
            arity(name, args, 2, 2, line);
            V total = args[1];
            Args copy = l.items;
            for (auto& item : copy) total = call(args[0], {total, item}, line);
            return total;
        }
        fatal("a list has no method `" + name + "`", line);
    }
    fatal(typeName(receiver) + " has no method `" + name + "`", line);
}

// `{x:>8.2f}`: [<>^]? 0? width? (.precision)? kind?
V format(const V& v, const string& spec, int line) {
    if (spec.empty()) return S(show(v));
    size_t i = 0, n = spec.size();
    char align = 0;
    if (i < n && (spec[i] == '<' || spec[i] == '>' || spec[i] == '^')) align = spec[i++];
    bool zero = false;
    if (i < n && spec[i] == '0') { zero = true; i++; }
    string widthText;
    while (i < n && digit(spec[i])) widthText += spec[i++];
    bool hasPrecision = false;
    string precisionText;
    if (i < n && spec[i] == '.') {
        hasPrecision = true;
        i++;
        while (i < n && digit(spec[i])) precisionText += spec[i++];
    }
    string kind;
    if (i < n) {
        size_t len = std::min(charLen((unsigned char)spec[i]), n - i);
        kind = spec.substr(i, len);
        i += len;
    }
    if (i != n || (kind.size() == 1 && digit(kind[0]))) fatal("`" + spec + "` is not a format Zeno understands", line);
    int width = widthText.empty() ? 0 : atoi(widthText.c_str());
    int places = hasPrecision ? (precisionText.empty() ? 0 : atoi(precisionText.c_str())) : 6;
    string body;
    if ((kind == "f" || (kind.empty() && hasPrecision)) && v.is(K::QUANTITY)) {
        body = fixedPlaces(as<Quantity>(v).v, places) + " " + as<Quantity>(v).unit;
    } else if (kind == "f" || (kind.empty() && hasPrecision && v.t == V::FLOAT)) {
        if (!isNumber(v)) wrongType(spec, "a number", v, line);
        body = fixedPlaces(number(v), places);
    } else if ((kind.empty() && !hasPrecision) || (kind == "d" && !hasPrecision && (v.t == V::INT || v.t == V::U64))) {
        body = show(v);
    } else if ((kind == "x" || kind == "X" || kind == "b") && !hasPrecision && v.t == V::U64) {
        uint64_t mag = (uint64_t)v.i;
        if (kind == "b") {
            if (mag == 0) body = "0";
            while (mag > 0) { body.insert(body.begin(), (char)('0' + (mag & 1))); mag >>= 1; }
        } else {
            char buf[32];
            snprintf(buf, sizeof buf, kind == "X" ? "%llX" : "%llx", (unsigned long long)mag);
            body = buf;
        }
    } else if ((kind == "x" || kind == "X" || kind == "b") && !hasPrecision && v.t == V::INT) {
        int64_t x = v.i;
        uint64_t mag = x < 0 ? (uint64_t)(-(x + 1)) + 1 : (uint64_t)x;
        string digits;
        if (kind == "b") {
            if (mag == 0) digits = "0";
            while (mag > 0) { digits.insert(digits.begin(), (char)('0' + (mag & 1))); mag >>= 1; }
        } else {
            char buf[32];
            snprintf(buf, sizeof buf, kind == "X" ? "%llX" : "%llx", (unsigned long long)mag);
            digits = buf;
        }
        body = (x < 0 ? "-" : "") + digits;
    } else if ((kind == "d" || kind == "x" || kind == "X" || kind == "b") && !hasPrecision) {
        wrongType(spec, "an int", v, line);
    } else {
        fatal("`" + spec + "` is not a format Zeno understands", line);
    }
    int64_t padding = std::max<int64_t>(0, width - textLength(body));
    bool isNum = isNumber(v) || v.is(K::QUANTITY);
    if (zero && isNum) {
        if (!body.empty() && body[0] == '-') return S("-" + string(padding, '0') + body.substr(1));
        return S(string(padding, '0') + body);
    }
    char a = align ? align : (isNum ? '>' : '<');
    if (a == '<') return S(body + string(padding, ' '));
    if (a == '>') return S(string(padding, ' ') + body);
    int64_t left = padding / 2;
    return S(string(left, ' ') + body + string(padding - left, ' '));
}

V format(const V& v, const V& spec, int line) { return format(v, str(spec), line); }

// ----------------------------------------------------------------- JSON

struct NotJson {};

struct Json {
    std::vector<int> cs;
    size_t at = 0;

    explicit Json(const string& s) {
        for (auto& c : codePoints(s)) cs.push_back(codePointAt(c));
    }

    void spaces() {
        while (at < cs.size() && (cs[at] == ' ' || cs[at] == '\t' || cs[at] == '\n' || cs[at] == '\r')) at++;
    }

    bool word(const string& w) {
        if (at + w.size() > cs.size()) return false;
        for (size_t i = 0; i < w.size(); i++)
            if (cs[at + i] != (unsigned char)w[i]) return false;
        at += w.size();
        return true;
    }

    int fourHex() {
        if (at + 4 > cs.size()) throw NotJson();
        int n = 0;
        for (int i = 0; i < 4; i++) {
            int c = cs[at + i];
            int d = c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : c >= 'A' && c <= 'F' ? c - 'A' + 10 : -1;
            if (d < 0) throw NotJson();
            n = n * 16 + d;
        }
        at += 4;
        return n;
    }

    string text() {
        if (at >= cs.size() || cs[at] != '"') throw NotJson();
        at++;
        string b;
        while (true) {
            if (at >= cs.size()) throw NotJson();
            int c = cs[at++];
            if (c == '"') return b;
            if (c != '\\') { b += fromCode(c); continue; }
            if (at >= cs.size()) throw NotJson();
            int e = cs[at++];
            switch (e) {
            case 'n': b += '\n'; break;
            case 't': b += '\t'; break;
            case 'r': b += '\r'; break;
            case 'b': b += '\b'; break;
            case 'f': b += '\f'; break;
            case 'u': {
                int code = fourHex();
                if (code >= 0xD800 && code < 0xDC00 && word("\\u")) {
                    int low = fourHex();
                    if (low < 0xDC00 || low > 0xDFFF) throw NotJson();
                    code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                }
                if ((code >= 0xD800 && code <= 0xDFFF) || code > 0x10FFFF) throw NotJson();
                b += fromCode(code);
                break;
            }
            default: b += fromCode(e); break;
            }
        }
    }

    V value() {
        spaces();
        if (at >= cs.size()) throw NotJson();
        int c = cs[at];
        if (c == 'n') { if (word("null")) return V(); throw NotJson(); }
        if (c == 't') { if (word("true")) return B(true); throw NotJson(); }
        if (c == 'f') { if (word("false")) return B(false); throw NotJson(); }
        if (c == '"') return S(text());
        if (c == '[') {
            at++;
            Args out;
            while (true) {
                spaces();
                if (word("]")) return newList(std::move(out));
                if (!out.empty() && !word(",")) throw NotJson();
                out.push_back(value());
            }
        }
        if (c == '{') {
            at++;
            auto d = std::make_shared<Dict>();
            int count = 0;
            while (true) {
                spaces();
                if (word("}")) return O(d);
                if (count > 0 && !word(",")) throw NotJson();
                spaces();
                string name = text();
                spaces();
                if (!word(":")) throw NotJson();
                V v = value();
                count++;
                if (v.t != V::NONE) dictSet(*d, S(name), v);
            }
        }
        size_t start = at;
        while (at < cs.size() && cs[at] < 128 && strchr("0123456789+-.eE", cs[at]) && cs[at] != 0) at++;
        string chars;
        for (size_t i = start; i < at; i++) chars += (char)cs[i];
        double f;
        if (!parseFloat(chars, f)) throw NotJson();
        if (!std::isinf(f) && !std::isnan(f) && f == std::floor(f) && std::fabs(f) < 9.0e15) return I((int64_t)f);
        return F(f);
    }
};

bool jsonParse(const string& s, V& out) {
    try {
        Json j(s);
        out = j.value();
        j.spaces();
        return j.at == j.cs.size();
    } catch (NotJson&) {
        return false;
    }
}

string jsonText(const string& s) {
    string b = "\"";
    for (auto& ch : codePoints(s)) {
        int c = codePointAt(ch);
        if (c == '"') b += "\\\"";
        else if (c == '\\') b += "\\\\";
        else if (c == '\n') b += "\\n";
        else if (c == '\r') b += "\\r";
        else if (c == '\t') b += "\\t";
        else if (c < 0x20) b += "\\u" + hex(c, 4);
        else b += ch;
    }
    return b + "\"";
}

string jsonWrite(const V& v, int line) {
    switch (v.t) {
    case V::NONE: return "null";
    case V::BOOL: return v.b ? "true" : "false";
    case V::INT: return num(v.i);
    case V::FLOAT:
        if (std::isnan(v.f) || std::isinf(v.f)) raiseError("ValueError", "JSON has no way to write " + showFloat(v.f), line);
        return showFloat(v.f);
    default: break;
    }
    if (v.is(K::STR)) return jsonText(str(v));
    if (v.is(K::LIST)) {
        std::vector<string> parts;
        for (auto& item : as<List>(v).items) parts.push_back(jsonWrite(item, line));
        return "[" + join(parts, ",") + "]";
    }
    if (v.is(K::DICT)) {
        Dict& d = as<Dict>(v);
        std::vector<string> parts;
        for (auto& k : d.keys) parts.push_back(jsonText(show(k)) + ":" + jsonWrite(d.map[keyOf(k)], line));
        return "{" + join(parts, ",") + "}";
    }
    if (v.is(K::INSTANCE)) {
        Instance& o = as<Instance>(v);
        std::vector<string> parts;
        for (size_t i = 0; i < o.cls->fields.size(); i++) parts.push_back(jsonText(o.cls->fields[i]) + ":" + jsonWrite(o.fields[i], line));
        return "{" + join(parts, ",") + "}";
    }
    if (v.is(K::QUANTITY)) {
        const string& u = as<Quantity>(v).unit;
        raiseError("ValueError", "JSON has no units, and this is a quantity in " + u + ". Write its number, and put the unit in the name: `[\"" + u + "\": ...]`", line);
    }
    raiseError("ValueError", "JSON has objects, lists, text, numbers, bools and none, and this is " + typeName(v), line);
}

// ---------------------------------------------------------------- Files

// "file", "nothing" or "other": what is at a path.
string at(const string& path) {
    FILE* f = fopen(path.c_str(), "rb");
    // A path through a file (`notes.txt/x`) is not "nothing": something
    // is in the way, as `zeno run` says.
    if (!f) return errno == ENOENT ? "nothing" : "other";
    fgetc(f);
    bool bad = ferror(f);
    fclose(f);
    return bad ? "other" : "file";
}

std::vector<string> paths(const string& name, const Args& args, size_t count, int line) {
    arity(name, args, count, count, line);
    std::vector<string> out;
    for (auto& a : args) out.push_back(text(name, a, line));
    return out;
}

[[noreturn]] void fileError(const string& message, int line) { raiseError("FileError", message, line); }

bool validUtf8(const string& s) {
    size_t i = 0, n = s.size();
    while (i < n) {
        unsigned char c = s[i];
        if (c < 0x80) { i++; continue; }
        size_t len;
        int code;
        if (c >= 0xC2 && c <= 0xDF) { len = 2; code = c & 0x1F; }
        else if (c >= 0xE0 && c <= 0xEF) { len = 3; code = c & 0x0F; }
        else if (c >= 0xF0 && c <= 0xF4) { len = 4; code = c & 0x07; }
        else return false;
        if (i + len > n) return false;
        for (size_t k = 1; k < len; k++) {
            unsigned char d = s[i + k];
            if ((d & 0xC0) != 0x80) return false;
            code = (code << 6) | (d & 0x3F);
        }
        if ((len == 3 && code < 0x800) || (len == 4 && (code < 0x10000 || code > 0x10FFFF)) || (code >= 0xD800 && code <= 0xDFFF)) return false;
        i += len;
    }
    return true;
}

const auto STARTED = std::chrono::steady_clock::now();

// --------------------------------------------------- The built-in table

V b_print(const Args& args, int line) {
    string line_;
    for (size_t i = 0; i < args.size(); i++) {
        if (i > 0) line_ += ' ';
        line_ += show(args[i]);
    }
    line_ += '\n';
    write(line_);
    return V();
}

// Text from outside that may not be UTF-8. Each run of bytes that could
// have begun a character and did not finish one becomes one U+FFFD, and
// so does each byte that could not begin one: the way Rust, Python and
// the browsers read such text.
string lossy(const string& s) {
    string out;
    for (size_t i = 0; i < s.size();) {
        unsigned char c = s[i];
        if (c < 0x80) { out += (char)c; i++; continue; }
        int need = 0;
        unsigned char lo = 0x80, hi = 0xBF;
        if (c >= 0xC2 && c <= 0xDF) need = 1;
        else if (c == 0xE0) { need = 2; lo = 0xA0; }
        else if (c == 0xED) { need = 2; hi = 0x9F; }
        else if (c >= 0xE1 && c <= 0xEF) need = 2;
        else if (c == 0xF0) { need = 3; lo = 0x90; }
        else if (c == 0xF4) { need = 3; hi = 0x8F; }
        else if (c >= 0xF1 && c <= 0xF3) need = 3;
        size_t j = i + 1;
        bool whole = need > 0;
        for (int k = 0; whole && k < need; k++) {
            if (j < s.size() && (unsigned char)s[j] >= lo && (unsigned char)s[j] <= hi) j++;
            else whole = false;
            lo = 0x80;
            hi = 0xBF;
        }
        if (whole) out.append(s, i, j - i);
        else out += "\xEF\xBF\xBD";
        i = j;
    }
    return out;
}

V b_input(const Args& args, int line) {
    arity("input", args, 0, 1, line);
    if (!args.empty()) write(text("input", args[0], line));
    flush();
    string bytes;
    bool any = false, ended = false;
    int c;
    while ((c = getchar()) != EOF) {
        any = true;
        if (c == '\n') { ended = true; break; }
        bytes += (char)c;
    }
    if (!any) return V();
    if (ended && !bytes.empty() && bytes.back() == '\r') bytes.pop_back();
    if (!validUtf8(bytes)) bytes = lossy(bytes);
    bytes.erase(std::remove(bytes.begin(), bytes.end(), '\0'), bytes.end());
    return S(bytes);
}

V b_len(const Args& args, int line) {
    arity("len", args, 1, 1, line);
    const V& v = args[0];
    if (v.is(K::STR)) return I(textLength(v));
    if (v.is(K::LIST)) return I((int64_t)as<List>(v).items.size());
    if (v.is(K::DICT)) return I((int64_t)as<Dict>(v).keys.size());
    wrongType("len", "a str, a list or a dict", v, line);
}

V b_range(const Args& args, int line) {
    arity("range", args, 1, 3, line);
    std::vector<int64_t> n;
    for (auto& a : args) {
        if (a.t != V::INT) wrongType("range", "ints", a, line);
        n.push_back(a.i);
    }
    int64_t start = 0, stop, step = 1;
    if (n.size() == 1) stop = n[0];
    else if (n.size() == 2) { start = n[0]; stop = n[1]; }
    else { start = n[0]; stop = n[1]; step = n[2]; }
    if (step == 0) raiseError("ValueError", "the step of a range cannot be 0", line);
    Args out;
    for (int64_t i = start; step > 0 ? i < stop : i > stop;) {
        out.push_back(I(i));
        int64_t next;
        if (__builtin_add_overflow(i, step, &next)) break;
        i = next;
    }
    return newList(std::move(out));
}

V b_u64(const Args& args, int line) {
    arity("u64", args, 1, 1, line);
    const V& v = args[0];
    if (v.t == V::U64) return v;
    if (v.t == V::INT) {
        if (v.i < 0) raiseError("OverflowError", num(v.i) + " does not fit in a u64 (0 to 18446744073709551615)", line);
        return U64V((uint64_t)v.i);
    }
    wrongType("u64", "a whole number", v, line);
}

// `u64.wrap(n)`: the 64 bits of an int read with no sign (42.6).
V b_u64__of(const Args& args, int line) {
    arity("u64.wrap", args, 1, 1, line);
    const V& v = args[0];
    if (v.t == V::U64 || v.t == V::INT) return U64V((uint64_t)v.i);
    wrongType("u64.wrap", "a whole number", v, line);
}

// `u64.wrap(a + b)`: the answer cut to 64 bits (42.6).
V u64Wrapping(const string& op, const Args& args, int line) {
    arity("u64.wrap", args, 2, 2, line);
    uint64_t x, y;
    bool ok1 = unsignedOf(args[0], x);
    if (op == "<<" && ok1 && (args[1].t == V::INT || args[1].t == V::U64)) return U64V(x << u64ShiftCount(args[1], line));
    bool ok2 = unsignedOf(args[1], y);
    if (!ok1) wrongType("u64.wrap", "u64s", args[0], line);
    if (!ok2) wrongType("u64.wrap", "u64s", args[1], line);
    if (op == "+") return U64V(x + y);
    if (op == "-") return U64V(x - y);
    return U64V(x * y);
}

V b_u64__add(const Args& args, int line) { return u64Wrapping("+", args, line); }
V b_u64__sub(const Args& args, int line) { return u64Wrapping("-", args, line); }
V b_u64__mul(const Args& args, int line) { return u64Wrapping("*", args, line); }
V b_u64__shl(const Args& args, int line) { return u64Wrapping("<<", args, line); }

// A float to an int, as Rust's `as i64` does: NaN is 0, and the ends hold.
int64_t toInt(double x) {
    if (std::isnan(x)) return 0;
    if (x >= 9.223372036854775807e18) return INT64_MAX;
    if (x <= -9.223372036854775808e18) return INT64_MIN;
    return (int64_t)x;
}

V b_int(const Args& args, int line) {
    arity("int", args, 1, 1, line);
    const V& v = args[0];
    if (v.t == V::INT) return v;
    if (v.t == V::U64) {
        if (v.i < 0) raiseError("OverflowError", std::to_string((uint64_t)v.i) + " does not fit in an int (-9223372036854775808 to 9223372036854775807)", line);
        return I(v.i);
    }
    if (v.t == V::FLOAT || v.t == V::F32) {
        if (std::isnan(v.f) || std::isinf(v.f)) raiseError("ValueError", "cannot turn " + showFloat(v.f) + " into an int", line);
        return I(toInt(v.f));
    }
    if (v.is(K::STR)) {
        string t;
        for (char c : strip(str(v)))
            if (c != '_') t += c;
        size_t i = (!t.empty() && (t[0] == '+' || t[0] == '-')) ? 1 : 0;
        bool ok = i < t.size();
        for (size_t k = i; k < t.size(); k++)
            if (!digit(t[k])) ok = false;
        if (ok) {
            errno = 0;
            long long n = strtoll(t.c_str(), nullptr, 10);
            if (errno == 0) return I(n);
        }
        raiseError("ValueError", "cannot turn " + quoted(str(v)) + " into an int", line);
    }
    wrongType("int", "a number or a str", v, line);
}

V b_float(const Args& args, int line) {
    arity("float", args, 1, 1, line);
    const V& v = args[0];
    if (v.t == V::INT) return F((double)v.i);
    if (v.t == V::FLOAT) return v;
    if (v.t == V::F32) return F(v.f);
    if (v.t == V::U64) return F((double)(uint64_t)v.i);
    if (v.is(K::STR)) {
        double x;
        if (!parseFloat(strip(str(v)), x)) raiseError("ValueError", "cannot turn " + quoted(str(v)) + " into a float", line);
        return F(x);
    }
    wrongType("float", "a number or a str", v, line);
}

V b_f32(const Args& args, int line) {
    arity("f32", args, 1, 1, line);
    const V& v = args[0];
    if (v.t == V::F32) return v;
    if (v.t == V::INT) return F32V(fround((double)v.i));
    if (v.t == V::FLOAT) {
        double r = fround(v.f);
        if (std::isinf(r) && !std::isinf(v.f) && !std::isnan(v.f)) {
            raiseError("OverflowError", showFloat(v.f) + " does not fit in an f32, whose largest is about 3.4e38", line);
        }
        return F32V(r);
    }
    wrongType("f32", "a number", v, line);
}

V b_str(const Args& args, int line) {
    arity("str", args, 1, 1, line);
    return S(show(args[0]));
}

V b_abs(const Args& args, int line) {
    arity("abs", args, 1, 1, line);
    const V& v = args[0];
    if (v.t == V::INT) {
        if (v.i == INT64_MIN) overflow(line);
        return I(v.i < 0 ? -v.i : v.i);
    }
    if (v.t == V::FLOAT) return F(std::fabs(v.f));
    if (v.t == V::F32) return F32V(std::fabs(v.f));
    if (v.t == V::U64) return v;
    if (v.is(K::QUANTITY)) return Q(std::fabs(as<Quantity>(v).v), as<Quantity>(v).unit);
    wrongType("abs", "a number", v, line);
}

V b_min(const Args& args, int line) { return pick("min", args, line, true); }

V b_max(const Args& args, int line) { return pick("max", args, line, false); }

V b_sum(const Args& args, int line) {
    string unit;
    Args values = numbers("sum", args, line, unit);
    if (!unit.empty()) {
        fatal("`sum` works on plain numbers, and these have a unit. Add quantities with `+`, which keeps the unit: start from `var total = 0 V` and add each in a loop", line);
    }
    bool u64s = false;
    for (auto& item : values) u64s = u64s || item.t == V::U64;
    if (u64s) {
        uint64_t total = 0;
        for (auto& item : values) {
            uint64_t x = 0;
            unsignedOf(item, x);
            if (__builtin_add_overflow(total, x, &total)) raiseError("OverflowError", tooBigU64, line);
        }
        return U64V(total);
    }
    if (allSingle(values)) {
        double single32 = 0;
        for (auto& item : values) {
            double x;
            single(item, x);
            single32 = fround(single32 + x);
        }
        return F32V(single32);
    }
    int64_t total = 0;
    bool allInts = true;
    double floatTotal = 0;
    for (auto& item : values) {
        floatTotal += number(item);
        if (allInts && item.t == V::INT) total = addInt(total, item.i, line);
        else allInts = false;
    }
    return allInts ? I(total) : F(floatTotal);
}

V b_char(const Args& args, int line) {
    arity("char", args, 1, 1, line);
    int64_t code = wholeArg("char", args[0], line);
    if (code <= 0 || code > 0x10FFFF || (code >= 0xD800 && code <= 0xDFFF)) {
        raiseError("ValueError", num(code) + " is not the code of a character", line);
    }
    return S(fromCode(code));
}

V b_allocate(const Args&, int line) { fatal("manual memory is only for a program compiled to machine code", line); }

V b_free(const Args& args, int line) { return b_allocate(args, line); }

double oneNumber(const string& name, const Args& args, int line) {
    arity(name, args, 1, 1, line);
    if (!isNumber(args[0])) wrongType(name, "a number", args[0], line);
    return number(args[0]);
}

double angle(const string& name, const Args& args, int line) {
    arity(name, args, 1, 1, line);
    if (args[0].is(K::QUANTITY)) {
        Quantity& q = as<Quantity>(args[0]);
        Scale s = convert(q.unit, "rad");
        if (!s.ok) fatal("`" + name + "` needs a number or an angle, such as `30 deg`, not a quantity in " + q.unit, line);
        return apply(s, q.v);
    }
    if (!isNumber(args[0])) wrongType(name, "a number or an angle, such as `30 deg`", args[0], line);
    return number(args[0]);
}

double ratio(const string& name, const Args& args, int line) {
    double x = oneNumber(name, args, line);
    if (x < -1 || x > 1) raiseError("ValueError", "`" + name + "` needs a number from -1 to 1, and this is " + showFloat(x), line);
    return x;
}

double aboveZero(const string& name, const Args& args, int line) {
    double x = oneNumber(name, args, line);
    if (x <= 0) raiseError("ValueError", "cannot take the logarithm of " + showFloat(x), line);
    return x;
}

// A whole float as an int; infinity and NaN stay as they are.
// What `floor` and `ceil` give is an int: one too big for an int is the
// biggest int there is, and what is not a number is 0, as in `zeno run`.
V wholeOf(double x) { return I(toInt(x)); }

V b_math_sqrt(const Args& args, int line) {
    double x = oneNumber("sqrt", args, line);
    if (x < 0) raiseError("ValueError", "cannot take the square root of " + showFloat(x), line);
    return F(std::sqrt(x));
}

V b_math_floor(const Args& args, int line) { return wholeOf(std::floor(oneNumber("floor", args, line))); }

V b_math_ceil(const Args& args, int line) { return wholeOf(std::ceil(oneNumber("ceil", args, line))); }

// A half goes away from zero, as Rust's `round` does.
V b_math_round(const Args& args, int line) { return I(toInt(std::round(oneNumber("round", args, line)))); }

V b_math_sin(const Args& args, int line) { return F(std::sin(angle("sin", args, line))); }

V b_math_cos(const Args& args, int line) { return F(std::cos(angle("cos", args, line))); }

V b_math_tan(const Args& args, int line) { return F(std::tan(angle("tan", args, line))); }

V b_math_asin(const Args& args, int line) { return Q(std::asin(ratio("asin", args, line)), "rad"); }

V b_math_acos(const Args& args, int line) { return Q(std::acos(ratio("acos", args, line)), "rad"); }

V b_math_atan(const Args& args, int line) { return Q(std::atan(oneNumber("atan", args, line)), "rad"); }

V b_math_atan2(const Args& args, int line) {
    arity("atan2", args, 2, 2, line);
    if (!isNumber(args[0])) wrongType("atan2", "two numbers", args[0], line);
    if (!isNumber(args[1])) wrongType("atan2", "two numbers", args[1], line);
    return Q(std::atan2(number(args[0]), number(args[1])), "rad");
}

V b_math_exp(const Args& args, int line) { return F(std::exp(oneNumber("exp", args, line))); }

V b_math_log(const Args& args, int line) { return F(std::log(aboveZero("log", args, line))); }

V b_math_log2(const Args& args, int line) { return F(std::log2(aboveZero("log2", args, line))); }

V b_math_log10(const Args& args, int line) { return F(std::log10(aboveZero("log10", args, line))); }

V b_time_sleep(const Args& args, int line) {
    arity("sleep", args, 1, 1, line);
    if (!args[0].is(K::QUANTITY)) wrongType("sleep", "a time with a unit, such as `500 ms`", args[0], line);
    Quantity& q = as<Quantity>(args[0]);
    Scale s = convert(q.unit, "s");
    if (!s.ok) fatal("`sleep` needs a time, such as `500 ms`, not a quantity in " + q.unit, line);
    double seconds = apply(s, q.v);
    if (seconds > 0) {
        flush();
        std::this_thread::sleep_for(std::chrono::duration<double>(std::min(seconds, 1e7)));
    }
    return V();
}

V b_time_now(const Args& args, int line) {
    arity("now", args, 0, 0, line);
    return Q(std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - STARTED).count(), "ms");
}

V b_files_read(const Args& args, int line) {
    string path = paths("read", args, 1, line)[0];
    string where = at(path);
    if (where == "nothing") fileError("there is no file `" + path + "`", line);
    if (where == "other") fileError("cannot read `" + path + "`", line);
    FILE* f = fopen(path.c_str(), "rb");
    if (!f) fileError("cannot read `" + path + "`", line);
    string data;
    char buf[65536];
    size_t got;
    while ((got = fread(buf, 1, sizeof buf, f)) > 0) data.append(buf, got);
    bool bad = ferror(f);
    fclose(f);
    if (bad) fileError("cannot read `" + path + "`", line);
    if (!validUtf8(data) || data.find('\0') != string::npos) fileError("`" + path + "` does not hold text", line);
    return S(data);
}

V put(const string& name, const Args& args, bool append, int line) {
    std::vector<string> p = paths(name, args, 2, line);
    FILE* f = fopen(p[0].c_str(), append ? "ab" : "wb");
    if (!f) fileError("cannot write `" + p[0] + "`", line);
    bool ok = fwrite(p[1].data(), 1, p[1].size(), f) == p[1].size();
    ok = fclose(f) == 0 && ok;
    if (!ok) fileError("cannot write `" + p[0] + "`", line);
    return V();
}

V b_files_write(const Args& args, int line) { return put("write", args, false, line); }

V b_files_append(const Args& args, int line) { return put("append", args, true, line); }

V b_files_read_bytes(const Args& args, int line) {
    string path = paths("read_bytes", args, 1, line)[0];
    string where = at(path);
    if (where == "nothing") fileError("there is no file `" + path + "`", line);
    if (where == "other") fileError("cannot read `" + path + "`", line);
    FILE* f = fopen(path.c_str(), "rb");
    if (!f) fileError("cannot read `" + path + "`", line);
    Args items;
    unsigned char buf[65536];
    size_t got;
    while ((got = fread(buf, 1, sizeof buf, f)) > 0) {
        for (size_t i = 0; i < got; i++) items.push_back(I(buf[i]));
    }
    bool bad = ferror(f);
    fclose(f);
    if (bad) fileError("cannot read `" + path + "`", line);
    return list(items);
}

V b_files_write_bytes(const Args& args, int line) {
    arity("write_bytes", args, 2, 2, line);
    string path = text("write_bytes", args[0], line);
    if (!args[1].is(K::LIST)) wrongType("write_bytes", "a list[u8]", args[1], line);
    const Args& items = as<List>(args[1]).items;
    string data;
    data.reserve(items.size());
    for (const V& item : items) {
        if (item.t != V::INT) wrongType("write_bytes", "a list[u8]", item, line);
        if (item.i < 0 || item.i > 255) raiseError("ValueError", std::to_string(item.i) + " is not a byte: a byte is a number from 0 to 255", line);
        data.push_back((char)(unsigned char)item.i);
    }
    FILE* f = fopen(path.c_str(), "wb");
    if (!f) fileError("cannot write `" + path + "`", line);
    bool ok = fwrite(data.data(), 1, data.size(), f) == data.size();
    ok = fclose(f) == 0 && ok;
    if (!ok) fileError("cannot write `" + path + "`", line);
    return V();
}

V b_files_exists(const Args& args, int line) { return B(at(paths("exists", args, 1, line)[0]) == "file"); }

V b_files_remove(const Args& args, int line) {
    string path = paths("remove", args, 1, line)[0];
    string where = at(path);
    if (where == "nothing") fileError("there is no file `" + path + "`", line);
    if (where == "file" && std::remove(path.c_str()) == 0) return V();
    fileError("cannot remove `" + path + "`", line);
}

bool isFolder(const string& path) {
    struct stat info;
    return !path.empty() && stat(path.c_str(), &info) == 0 && S_ISDIR(info.st_mode);
}

// `files.list(folder)`: the names in it, sorted by their bytes, which for
// UTF-8 is the order of the numbers of their characters.
V b_files_list(const Args& args, int line) {
    string folder = paths("list", args, 1, line)[0];
    DIR* d = folder.empty() ? nullptr : opendir(folder.c_str());
    if (!d) {
        if (folder.empty() || errno == ENOENT) fileError("there is no folder `" + folder + "`", line);
        fileError("cannot list the folder `" + folder + "`", line);
    }
    std::vector<string> names;
    while (struct dirent* entry = readdir(d)) {
        string name = entry->d_name;
        if (name == "." || name == "..") continue;
        if (!validUtf8(name)) {
            closedir(d);
            fileError("the folder `" + folder + "` holds a name that is not text", line);
        }
        names.push_back(name);
    }
    closedir(d);
    std::sort(names.begin(), names.end());
    Args items;
    for (auto& name : names) items.push_back(S(name));
    return list(items);
}

// `files.make_folder(path)`: it worked if there is a folder there at the end.
V b_files_make_folder(const Args& args, int line) {
    string path = paths("make_folder", args, 1, line)[0];
    for (size_t i = 1; i <= path.size(); i++) {
        if (i == path.size() || path[i] == '/') mkdir(path.substr(0, i).c_str(), 0777);
    }
    if (!isFolder(path)) fileError("cannot make the folder `" + path + "`", line);
    return V();
}

// What a `files.Reader` and a `files.Writer` are made of (decision
// 59.6). A file's number is its place in `openFiles`, plus one; a closed
// file leaves its place empty. Files are read as bytes.
struct OpenFile {
    FILE* f;
    bool reading;
};
std::vector<OpenFile> openFiles;

OpenFile& handle(const string& name, const Args& args, int line) {
    if (args[0].t != V::INT) wrongType(name, "an int", args[0], line);
    int64_t number = args[0].i;
    if (number < 1 || number > (int64_t)openFiles.size() || !openFiles[number - 1].f) {
        fatal("there is no open file numbered " + num(number), line);
    }
    return openFiles[number - 1];
}

V b_files_handle_open(const Args& args, int line) {
    arity("handle_open", args, 2, 2, line);
    string path = text("handle_open", args[0], line);
    int64_t how = args[1].t == V::INT ? args[1].i : 0;
    OpenFile open{nullptr, how == 0};
    if (how == 0) {
        string where = at(path);
        if (where == "nothing") fileError("there is no file `" + path + "`", line);
        if (where == "other") fileError("cannot read `" + path + "`", line);
        open.f = fopen(path.c_str(), "rb");
        if (!open.f) fileError("cannot read `" + path + "`", line);
    } else {
        open.f = path.empty() ? nullptr : fopen(path.c_str(), how == 2 ? "ab" : "wb");
        if (!open.f) fileError("cannot write `" + path + "`", line);
        // Nothing is held back, so that a full disk is an error at the
        // write that met it.
        setvbuf(open.f, nullptr, _IONBF, 0);
    }
    for (size_t place = 0; place < openFiles.size(); place++) {
        if (!openFiles[place].f) {
            openFiles[place] = open;
            return I((int64_t)place + 1);
        }
    }
    openFiles.push_back(open);
    return I((int64_t)openFiles.size());
}

// The next line without its line end (a `\r` before the `\n` goes with
// it), or none at the end.
V b_files_handle_line(const Args& args, int line) {
    arity("handle_line", args, 2, 2, line);
    string path = text("handle_line", args[1], line);
    OpenFile& open = handle("handle_line", args, line);
    if (!open.reading) fileError("cannot read `" + path + "`", line);
    // The end is where the file ends now: a stream remembers that it met
    // the end, and would give nothing written there since (59.6).
    clearerr(open.f);
    string data;
    bool any = false, ended = false;
    int c;
    while ((c = getc(open.f)) != EOF) {
        any = true;
        if (c == '\n') {
            ended = true;
            break;
        }
        data.push_back((char)c);
    }
    if (ferror(open.f)) fileError("cannot read `" + path + "`", line);
    if (!any) return V();
    if (ended && !data.empty() && data.back() == '\r') data.pop_back();
    if (!validUtf8(data) || data.find('\0') != string::npos) fileError("`" + path + "` does not hold text", line);
    return S(data);
}

// Up to `count` bytes, fewer at the end, none after it.
V b_files_handle_bytes(const Args& args, int line) {
    arity("handle_bytes", args, 3, 3, line);
    string path = text("handle_bytes", args[1], line);
    int64_t count = args[2].t == V::INT && args[2].i > 0 ? args[2].i : 0;
    OpenFile& open = handle("handle_bytes", args, line);
    if (!open.reading) fileError("cannot read `" + path + "`", line);
    clearerr(open.f);
    Args items;
    unsigned char buf[65536];
    while ((int64_t)items.size() < count) {
        size_t want = (size_t)std::min<int64_t>(count - (int64_t)items.size(), (int64_t)sizeof buf);
        size_t got = fread(buf, 1, want, open.f);
        for (size_t i = 0; i < got; i++) items.push_back(I(buf[i]));
        if (got < want) break;
    }
    if (ferror(open.f)) fileError("cannot read `" + path + "`", line);
    return list(items);
}

V handlePut(const string& name, const Args& args, const string& data, const string& path, int line) {
    OpenFile& open = handle(name, args, line);
    if (open.reading) fileError("cannot write `" + path + "`", line);
    bool ok = fwrite(data.data(), 1, data.size(), open.f) == data.size();
    ok = fflush(open.f) == 0 && ok;
    if (!ok) fileError("cannot write `" + path + "`", line);
    return V();
}

V b_files_handle_write(const Args& args, int line) {
    arity("handle_write", args, 3, 3, line);
    string path = text("handle_write", args[1], line);
    const string& s = text("handle_write", args[2], line);
    return handlePut("handle_write", args, s, path, line);
}

V b_files_handle_write_bytes(const Args& args, int line) {
    arity("handle_write_bytes", args, 3, 3, line);
    string path = text("handle_write_bytes", args[1], line);
    if (!args[2].is(K::LIST)) wrongType("handle_write_bytes", "a list[u8]", args[2], line);
    const Args& items = as<List>(args[2]).items;
    string data;
    data.reserve(items.size());
    for (const V& item : items) {
        if (item.t != V::INT) wrongType("handle_write_bytes", "a list[u8]", item, line);
        if (item.i < 0 || item.i > 255) raiseError("ValueError", std::to_string(item.i) + " is not a byte: a byte is a number from 0 to 255", line);
        data.push_back((char)(unsigned char)item.i);
    }
    return handlePut("handle_write_bytes", args, data, path, line);
}

// The file is closed, and its number free.
V b_files_handle_close(const Args& args, int line) {
    arity("handle_close", args, 1, 1, line);
    OpenFile& open = handle("handle_close", args, line);
    FILE* f = open.f;
    open.f = nullptr;
    fclose(f);
    return V();
}

std::vector<string> arguments;

// `program.exit(code)`: what was printed goes out, and the program ends
// with `code`. `_Exit` ends it at once: the program runs on a thread of
// its own, and nothing is to be torn down under it.
V b_program_exit(const Args& args, int line) {
    arity("exit", args, 1, 1, line);
    int64_t code = wholeArg("exit", args[0], line);
    if (code < 0 || code > 255) raiseError("ValueError", "a program ends with a number from 0 to 255, not " + num(code), line);
    flush();
    fflush(stderr);
    std::_Exit((int)code);
}

// `program.environment(name)`: the text, or none. A name with `=` or a
// zero byte in it is in no environment (and C would read only the part
// before the zero).
V b_program_environment(const Args& args, int line) {
    arity("environment", args, 1, 1, line);
    const string& name = text("environment", args[0], line);
    if (name.empty() || name.find_first_of(string("=\0", 2)) != string::npos) return V();
    const char* value = getenv(name.c_str());
    if (value == nullptr) return V();
    string got = value;
    return S(validUtf8(got) ? got : lossy(got));
}

// `program.warn(text)`: one line on the error stream, after what was printed.
V b_program_warn(const Args& args, int line) {
    arity("warn", args, 1, 1, line);
    string said = text("warn", args[0], line) + "\n";
    flush();
    fwrite(said.data(), 1, said.size(), stderr);
    fflush(stderr);
    return V();
}

V b_program_arguments(const Args& args, int line) {
    arity("arguments", args, 0, 0, line);
    return strings(arguments);
}

V b_json_parse(const Args& args, int line) {
    arity("parse", args, 1, 1, line);
    V s = S(text("parse", args[0], line));
    V v;
    if (!jsonParse(str(s), v)) {
        const string& t = str(s);
        size_t a = 0;
        while (a < t.size() && (t[a] == ' ' || t[a] == '\t' || t[a] == '\n' || t[a] == '\r')) a++;
        V start = S(t.substr(a));
        string shown = textLength(start) > 30 ? sub(start, 0, 30) + "..." : str(start);
        raiseError("ValueError", "this text is not JSON: `" + shown + "`", line);
    }
    return v;
}

V b_json_write(const Args& args, int line) {
    arity("write", args, 1, 1, line);
    return S(jsonWrite(args[0], line));
}

BuiltinBody sized(const string& name, bool wrap) {
    Width w = *width(name);
    string article = w.isSigned ? "an" : "a";
    if (!wrap) {
        return [=](const Args& args, int line) -> V {
            arity(name, args, 1, 1, line);
            if (args[0].t != V::INT) wrongType(name, "a whole number", args[0], line);
            int64_t n = args[0].i;
            if (n < w.lo || n > w.hi) raiseError("OverflowError", num(n) + " does not fit in " + article + " " + name + " (" + num(w.lo) + " to " + num(w.hi) + ")", line);
            return I(n);
        };
    }
    return [=](const Args& args, int line) -> V {
        arity(name + ".wrap", args, 1, 1, line);
        if (args[0].t != V::INT) wrongType(name + ".wrap", "a whole number", args[0], line);
        int64_t low = args[0].i & ((1LL << w.bits) - 1);
        return I(w.isSigned && low > w.hi ? low - (1LL << w.bits) : low);
    };
}

const Names ERROR_KINDS = {"Error", "ValueError", "IndexError", "KeyError", "ZeroDivisionError", "OverflowError", "FileError"};

V b_Error(const Args& args, int line) { return newError("Error", args, line); }
V b_ValueError(const Args& args, int line) { return newError("ValueError", args, line); }
V b_IndexError(const Args& args, int line) { return newError("IndexError", args, line); }
V b_KeyError(const Args& args, int line) { return newError("KeyError", args, line); }
V b_ZeroDivisionError(const Args& args, int line) { return newError("ZeroDivisionError", args, line); }
V b_OverflowError(const Args& args, int line) { return newError("OverflowError", args, line); }
V b_FileError(const Args& args, int line) { return newError("FileError", args, line); }

const std::unordered_map<string, BuiltinBody>& builtins() {
    static const std::unordered_map<string, BuiltinBody> b = [] {
        std::unordered_map<string, BuiltinBody> b = {
            {"print", b_print}, {"input", b_input}, {"len", b_len}, {"range", b_range}, {"int", b_int},
            {"float", b_float}, {"f32", b_f32}, {"u64", b_u64}, {"u64..of", b_u64__of}, {"u64..add", b_u64__add}, {"u64..sub", b_u64__sub}, {"u64..mul", b_u64__mul}, {"u64..shl", b_u64__shl}, {"str", b_str}, {"abs", b_abs}, {"min", b_min}, {"max", b_max}, {"sum", b_sum},
            {"char", b_char}, {"allocate", b_allocate}, {"free", b_free},
            {"math.sqrt", b_math_sqrt}, {"math.floor", b_math_floor}, {"math.ceil", b_math_ceil},
            {"math.round", b_math_round}, {"math.sin", b_math_sin}, {"math.cos", b_math_cos}, {"math.tan", b_math_tan},
            {"math.asin", b_math_asin}, {"math.acos", b_math_acos}, {"math.atan", b_math_atan}, {"math.atan2", b_math_atan2},
            {"math.exp", b_math_exp}, {"math.log", b_math_log}, {"math.log2", b_math_log2}, {"math.log10", b_math_log10},
            {"time.sleep", b_time_sleep}, {"time.now", b_time_now},
            {"files.read", b_files_read}, {"files.write", b_files_write}, {"files.read_bytes", b_files_read_bytes}, {"files.write_bytes", b_files_write_bytes}, {"files.append", b_files_append},
            {"files.exists", b_files_exists}, {"files.remove", b_files_remove}, {"files.list", b_files_list}, {"files.make_folder", b_files_make_folder},
            {"files.handle_open", b_files_handle_open}, {"files.handle_line", b_files_handle_line}, {"files.handle_bytes", b_files_handle_bytes},
            {"files.handle_write", b_files_handle_write}, {"files.handle_write_bytes", b_files_handle_write_bytes}, {"files.handle_close", b_files_handle_close},
            {"program.arguments", b_program_arguments}, {"program.exit", b_program_exit}, {"program.environment", b_program_environment}, {"program.warn", b_program_warn}, {"json.parse", b_json_parse}, {"json.write", b_json_write},
        };
        for (auto& w : WIDTHS) {
            b[w.first] = sized(w.first, false);
            b[w.first + ".wrap"] = sized(w.first, true);
        }
        for (auto& kind : ERROR_KINDS) {
            b[kind] = [kind](const Args& args, int line) { return newError(kind, args, line); };
        }
        return b;
    }();
    return b;
}

V call_builtin(const string& name, const Args& args, int line) {
    auto f = builtins().find(name);
    if (f == builtins().end()) fatal("there is no `" + name + "` in a program built for C++", line);
    return f->second(args, line);
}

// Something that comes with Zeno, as a value: `print`, `math.pi`.
V builtin(const string& name, int line) {
    static std::unordered_map<string, V> known;
    if (name == "math.pi") return F(M_PI);
    if (name == "math.e") return F(M_E);
    if (indexOf(ERROR_KINDS, name) >= 0) return O(std::make_shared<ErrorType>(name));
    auto k = known.find(name);
    if (k != known.end()) return k->second;
    size_t dot = name.find('.');
    auto body = builtins().find(name);
    if (body == builtins().end()) {
        if (dot != string::npos) fatal("module `" + name.substr(0, dot) + "` has no `" + name.substr(dot + 1) + "`", line);
        fatal("`" + name + "` is not defined", line);
    }
    V v = O(std::make_shared<Func>(dot != string::npos ? name.substr(dot + 1) : name, Names{}, Flags{}, nullptr, name, body->second));
    known[name] = v;
    return v;
}

V module(const string& name) { return O(std::make_shared<Module>(name)); }

// ---------------------------------------------------------- The program

string where(const std::vector<string>& files, int line) {
    size_t index = line / LINES_A_FILE;
    const string& file = index < files.size() ? files[index] : files[0];
    return " --> " + file + ":" + std::to_string(line % LINES_A_FILE);
}

struct Run {
    std::vector<string> files;
    std::function<V()> main;
    int status = 0;
};

void* runMain(void* p) {
    Run& run = *static_cast<Run*>(p);
    try {
        run.main();
        flush();
    } catch (Raise& e) {
        flush();
        string message = "error: uncaught " + shownName(e.value->kind) + ": " + e.value->message + "\n" + where(run.files, e.line) + "\n";
        fputs(message.c_str(), stderr);
        run.status = 1;
    } catch (Fatal& e) {
        flush();
        string message = "error: " + e.text + "\n" + where(run.files, e.line) + "\n";
        fputs(message.c_str(), stderr);
        run.status = 1;
    }
    return nullptr;
}

// Run the program, and say what stopped it if something did. It runs on
// a thread with a big stack, so that 1000 nested calls always fit.
int start(std::vector<string> files, int argc, char** argv, std::function<V()> main) {
    for (int i = 1; i < argc; i++) arguments.push_back(argv[i]);
    Run run{std::move(files), std::move(main)};
    pthread_attr_t attr;
    pthread_attr_init(&attr);
    pthread_attr_setstacksize(&attr, (size_t)512 * 1024 * 1024);
    pthread_t thread;
    if (pthread_create(&thread, &attr, runMain, &run) != 0) runMain(&run);
    else pthread_join(thread, nullptr);
    fflush(stderr);
    return run.status;
}

} // namespace Z

using Z::V;
using namespace std::string_literals;
