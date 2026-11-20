# The Zeno runtime for Ruby.
#
# A program built with `zeno build --target ruby` is its own code after
# this: what every Zeno value is, and what each operator and built-in
# does with it. It does what ops.rs and builtins.rs do for `zeno run`,
# step for step and message for message, so that the program prints
# what `zeno run` prints. It needs Ruby 3.0 or newer and no gem.
#
# How a Zeno value is held:
#
#   int        a Ruby Integer. Ruby's own never overflows, so each step
#              looks whether the result still fits in 64 bits
#   float      a Ruby Float
#   f32        a `ZF32`: the float it is exactly (decision 41)
#   u64        a `ZU64`: an Integer from 0 to 2**64 - 1 (decision 42)
#   str        a Ruby String, in UTF-8
#   bool       true, false
#   none       nil
#   list       a Ruby Array
#   dict       a Ruby Hash: it keeps the order its keys were added in
#   quantity   a `ZQuantity`: a number and its unit
#   function   a `ZFn`: a lambda, with its Zeno name and parameters
#   object     a `ZInstance` of a `ZClass`
#   enum case  a `ZCase` of a `ZEnum`
#   error      a `ZError`
#
# The steps the printed program calls are the methods of `Z`.

require "rbconfig"

class ZF32
  attr_reader :v

  def initialize(v)
    @v = v
  end
end

class ZU64
  attr_reader :v

  def initialize(v)
    @v = v
  end
end

class ZQuantity
  attr_reader :v, :unit

  def initialize(v, unit)
    @v = v
    @unit = unit
  end
end

class ZFn
  attr_reader :name, :params, :defaults, :fn, :builtin

  def initialize(name, params, defaults, fn, builtin = nil)
    @name = name
    @params = params
    @defaults = defaults
    @fn = fn
    @builtin = builtin
  end
end

# `funcs` is each method by its name.
class ZClass
  attr_reader :name, :fields, :defaults, :funcs

  def initialize(name, fields, defaults, funcs)
    @name = name
    @fields = fields
    @defaults = defaults
    @funcs = funcs
  end
end

class ZInstance
  attr_reader :cls, :fields

  def initialize(cls, fields)
    @cls = cls
    @fields = fields
  end
end

# Each case is [name, [the names of what it carries]].
class ZEnum
  attr_reader :name, :cases

  def initialize(name, cases)
    @name = name
    @cases = cases
  end
end

class ZCase
  attr_reader :enum, :index, :fields

  def initialize(enum, index, fields)
    @enum = enum
    @index = index
    @fields = fields
  end
end

class ZConstructor
  attr_reader :enum, :index

  def initialize(enum, index)
    @enum = enum
    @index = index
  end
end

class ZErrorType
  attr_reader :kind

  def initialize(kind)
    @kind = kind
  end
end

class ZError
  attr_reader :kind, :message, :object

  def initialize(kind, message, object)
    @kind = kind
    @message = message
    @object = object
  end
end

class ZModule
  attr_reader :name

  def initialize(name)
    @name = name
  end
end

# A method of a str, a list or a dict, read but not yet called: "abc".upper
class ZMethod
  attr_reader :receiver, :name

  def initialize(receiver, name)
    @receiver = receiver
    @name = name
  end
end

# A method of a class, read from an object: p.distance_to
class ZBound
  attr_reader :object, :fn

  def initialize(object, fn)
    @object = object
    @fn = fn
  end
end

# Several values from one function, as `return a, b` gives them, until
# `let a, b = ...` takes them apart (decision 57).
class ZSeveral
  attr_reader :values

  def initialize(values)
    @values = values
  end
end

# An error the program may catch, on its way to a `try`. Ruby would note
# every call it passes on its way; it is told there is nothing to note.
class ZRaise < Exception
  attr_reader :value, :at

  def initialize(value, at)
    super(value.message)
    @value = value
    @at = at
  end

  def backtrace
    []
  end
end

# A mistake in the program, which nothing catches.
class ZFatal < Exception
  attr_reader :text, :at

  def initialize(text, at)
    super(text)
    @text = text
    @at = at
  end

  def backtrace
    []
  end
end

# Text that is not JSON, on its way out of the reader.
class ZNotJson < Exception
  def backtrace
    []
  end
end

module Z
  # What a call gives a parameter it leaves out.
  MISSING = Object.new.freeze
  LINES_A_FILE = 1_000_000
  MAX_INT = 9223372036854775807
  MIN_INT = -9223372036854775808
  UTF8 = Encoding::UTF_8

  @depth = 0
  @output = String.new(capacity: 16384)
  @builtin_values = {}
  @started = 0.0

  class << self
    def fatal(message, line)
      raise ZFatal.new(message, line)
    end

    def raise_(kind, message, line)
      raise ZRaise.new(ZError.new(kind, message, nil), line)
    end

    def overflow(line)
      raise_("OverflowError", "the result is too big for an int", line)
    end

    def by_zero(line)
      raise_("ZeroDivisionError", "division by zero", line)
    end

    # An int that still fits in 64 bits, or Zeno's error.
    def fits(r, line)
      overflow(line) if r > MAX_INT || r < MIN_INT
      r
    end

    # The number inside an int or a float, as a float.
    def number(v)
      return v if v.is_a?(Float)
      return v.to_f if v.is_a?(Integer)
      return v.v if v.is_a?(ZF32)
      return v.v.to_f if v.is_a?(ZU64)
      nil
    end

    U64_MAX = (1 << 64) - 1
    U64_TOO_BIG = "the result is too big for a u64"

    # A u64, or an int of 0 or more, as an Integer; else nil.
    def unsigned(v)
      return v.v if v.is_a?(ZU64)
      return v if v.is_a?(Integer) && v >= 0
      nil
    end

    def u64_shift_count(n, line)
      n = n.v if n.is_a?(ZU64)
      raise_("ValueError", "cannot shift by " + n.to_s + ". A u64 has 64 bits, so a shift is 0 to 63", line) if n < 0 || n >= 64
      n
    end

    # `a op b` where one side is a u64 (decision 42).
    def u64_op(op, a, b, line)
      if (op == "<<" || op == ">>") && a.is_a?(ZU64) && (b.is_a?(Integer) || b.is_a?(ZU64))
        n = u64_shift_count(b, line)
        return ZU64.new(a.v >> n) if op == ">>"
        raise_("OverflowError", U64_TOO_BIG, line) if (a.v << n) > U64_MAX
        return ZU64.new(a.v << n)
      end
      x = unsigned(a)
      y = unsigned(b)
      if x.nil? || y.nil?
        fatal("`" + op + "` between " + type_name(a) + " and " + type_name(b) + " would have to choose between a u64 and something else. Write `u64( )` or `int( )` around one of them", line)
      end
      r = case op
          when "+" then x + y
          when "-"
            raise_("OverflowError", "the result is below 0, and a u64 never is", line) if y > x
            x - y
          when "*" then x * y
          when "/"
            by_zero(line) if y.zero?
            return x.to_f / y.to_f
          when "//", "%"
            by_zero(line) if y.zero?
            op == "//" ? x / y : x % y
          when "**"
            return ZU64.new(1) if y.zero?
            return ZU64.new(x) if x < 2
            raise_("OverflowError", U64_TOO_BIG, line) if y > 64
            x**y
          when "&" then x & y
          when "|" then x | y
          else x ^ y
          end
      raise_("OverflowError", U64_TOO_BIG, line) if r > U64_MAX
      ZU64.new(r)
    end

    # The nearest f32 to a float, as a float.
    def fround(x)
      [x].pack("e").unpack1("e")
    end

    # An f32, or an int as a float and then the nearest f32 (41.2).
    def single(v)
      return v.v if v.is_a?(ZF32)
      return fround(v.to_f) if v.is_a?(Integer)
      nil
    end

    # The float with the fewest digits that reads back as this f32.
    def f32_shown(x)
      return x if x.nan? || x == 0 || !x.finite?
      (1..9).each do |digits|
        d = Float(format("%.#{digits - 1}e", x))
        return d if fround(d) == x
      end
      x
    end

    def f32_op(op, a, b, line)
      x = single(a)
      y = single(b)
      if x.nil? || y.nil?
        if a.is_a?(Float) || b.is_a?(Float)
          fatal("`" + op + "` between " + type_name(a) + " and " + type_name(b) + " would have to choose between 32 bits and 64. Write `f32( )` around the float to work in 32, or `float( )` around the f32 to work in 64", line)
        end
        mismatch(op, a, b, line)
      end
      by_zero(line) if ["/", "//", "%"].include?(op) && y == 0
      r = case op
          when "+" then x + y
          when "-" then x - y
          when "*" then x * y
          when "/" then x / y
          when "//" then floor(x / y)
          when "%" then x - y * floor(x / y)
          else pow(x, y, line)
          end
      ZF32.new(fround(r))
    end

    def negative?(x)
      x < 0 || (x == 0 && 1 / x < 0)
    end

    # The whole number at or below a float, as a float.
    def floor(x)
      x.finite? && x != 0 ? x.floor.to_f : x
    end

    def ceil(x)
      return x unless x.finite? && x != 0
      up = x.ceil.to_f
      up == 0 && x < 0 ? -0.0 : up
    end

    def type_name(v)
      case v
      when Integer then "an int"
      when Float then "a float"
      when ZF32 then "an f32"
      when ZU64 then "a u64"
      when String then "a str"
      when true, false then "a bool"
      when nil then "none"
      when ZQuantity then "a quantity in " + v.unit
      when Array then "a list"
      when Hash then "a dict"
      when ZFn, ZMethod, ZBound then "a function"
      when ZClass then "a class"
      when ZInstance then "a " + v.cls.name
      when ZEnum then "an enum"
      when ZConstructor then "an enum case"
      when ZCase then "a " + v.enum.name
      when ZErrorType then "an error type"
      when ZError then "a " + v.kind
      when ZModule then "a module"
      when ZSeveral then v.values.length.to_s + " values"
      else "a value"
      end
    end

    # `Point`, not `shapes.Point`; `Stack`, not `Stack[int]`.
    def shown_name(name)
      name = name[0, name.index("[")] if name.include?("[")
      dot = name.rindex(".")
      dot ? name[(dot + 1)..] : name
    end

    # ----------------------------------------------------------- Showing

    # A float as Rust's `{:?}` shows it: the shortest digits that read
    # back as the same number, with `.0` on a whole one, and an exponent
    # for one below 1e-4 or from 1e16 up.
    def show_float(x)
      return "NaN" if x.nan?
      return x > 0 ? "inf" : "-inf" if x.infinite?
      return negative?(x) ? "-0.0" : "0.0" if x == 0
      # Ruby writes the shortest digits too, in a form of its own:
      # 0.30000000000000004, 1.0e+25. Take the digits and the power of
      # ten of the first one out of it.
      text = x.abs.to_s
      sign = x < 0 ? "-" : ""
      exp = 0
      e = text.index("e")
      if e
        exp = text[(e + 1)..].to_i
        text = text[0, e]
      end
      dot = text.index(".")
      if dot
        text = text[0, dot] + text[(dot + 1)..]
      else
        dot = text.length
      end
      lead = text[/\A0*/].length
      digits = text[lead..].sub(/0+\z/, "")
      exp += dot - lead - 1
      count = digits.length
      if exp < -4 || exp >= 16
        return sign + (count > 1 ? digits[0] + "." + digits[1..] : digits) + "e" + exp.to_s
      end
      return sign + "0." + "0" * (-exp - 1) + digits if exp < 0
      return sign + digits + "0" * (exp + 1 - count) + ".0" if count <= exp + 1
      sign + digits[0, exp + 1] + "." + digits[(exp + 1)..]
    end

    # A float with a fixed number of decimals, as Rust's `{:.2}` writes
    # it: from the exact value of the float, a half rounding to the even
    # digit. Ruby has the exact value as a fraction.
    def fixed(x, places)
      return "NaN" if x.nan?
      return x > 0 ? "inf" : "-inf" if x.infinite?
      digits = (x.abs.to_r * (10**places)).round(half: :even).to_s.rjust(places + 1, "0")
      whole = digits[0, digits.length - places]
      (negative?(x) ? "-" : "") + whole + (places > 0 ? "." + digits[digits.length - places, places] : "")
    end

    # A str in quotes, with what cannot be seen escaped.
    def quoted(s)
      escaped = s.gsub(/["\\\u0000-\u001f\u007f-\u009f]/) do |c|
        case c
        when '"' then '\\"'
        when "\\" then "\\\\"
        when "\n" then "\\n"
        when "\r" then "\\r"
        when "\t" then "\\t"
        when "\0" then "\\0"
        else "\\u{" + c.ord.to_s(16) + "}"
        end
      end
      '"' + escaped + '"'
    end

    # How `print`, `str()` and f-strings show a value.
    def show(v)
      case v
      when String then v
      when Integer then v.to_s
      when Float then show_float(v)
      when ZF32 then show_float(f32_shown(v.v))
      when ZU64 then v.v.to_s
      when true then "true"
      when false then "false"
      when nil then "none"
      when ZQuantity
        x = v.v
        return x.to_i.to_s + " " + v.unit if x.finite? && x.floor == x && x.abs < 1e15
        show_float(x) + " " + v.unit
      when Array then "[" + v.map { |item| repr(item) }.join(", ") + "]"
      when Hash
        return "[:]" if v.empty?
        "[" + v.map { |key, value| repr(key) + ": " + repr(value) }.join(", ") + "]"
      when ZFn then v.name != "" ? "<function " + v.name + ">" : "<function>"
      when ZMethod then "<function " + v.name + ">"
      when ZBound then "<function " + v.fn.name + ">"
      when ZClass then "<class " + v.name + ">"
      when ZInstance
        parts = v.cls.fields.each_with_index.map { |name, i| name + "=" + repr(v.fields[i]) }
        shown_name(v.cls.name) + "(" + parts.join(", ") + ")"
      when ZEnum then "<enum " + v.name + ">"
      when ZConstructor then "<case " + v.enum.cases[v.index][0] + ">"
      when ZCase
        name = shown_name(v.enum.cases[v.index][0])
        return name if v.fields.empty?
        name + "(" + v.fields.map { |item| repr(item) }.join(", ") + ")"
      when ZErrorType then "<error " + v.kind + ">"
      when ZError then v.message
      when ZModule then "<module " + v.name + ">"
      when ZSeveral then "<" + v.values.length.to_s + " values: " + v.values.map { |item| show(item) }.join(", ") + ">"
      else "<value>"
      end
    end

    def repr(v)
      v.is_a?(String) ? quoted(v) : show(v)
    end

    # ------------------------------------------------------------- Units

    UNITS = {
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
    }.freeze

    def unit?(name)
      UNITS.key?(name)
    end

    # How to turn a number in one unit into another: [times, over], or nil.
    def convert(from, to)
      return [Math::PI / 180, 1.0] if from == "deg" && to == "rad"
      return [180 / Math::PI, 1.0] if from == "rad" && to == "deg"
      return nil unless unit?(from) && unit?(to)
      from_kind, from_size = UNITS[from]
      to_kind, to_size = UNITS[to]
      return nil if from_kind != to_kind
      from_size >= to_size ? [from_size / to_size, 1.0] : [1.0, to_size / from_size]
    end

    def apply(scale, x)
      (x * scale[0]) / scale[1]
    end

    # The unit two quantities meet in: the finer one. [unit, scale_a, scale_b] or nil.
    def common(a, b)
      return nil unless unit?(a) && unit?(b)
      return nil if UNITS[a][0] != UNITS[b][0]
      unit = UNITS[a][1] <= UNITS[b][1] ? a : b
      [unit, convert(a, unit), convert(b, unit)]
    end

    # What `*` and `/` make of two quantities (decision 43): the operator,
    # what each side measures ("" for a plain number), and what the answer
    # measures ("" for a plain number). The answer is in the unit of its
    # kind with no prefix.
    MADE = {
      ["*", "voltage", "current"] => "power", ["*", "current", "voltage"] => "power",
      ["*", "current", "resistance"] => "voltage", ["*", "resistance", "current"] => "voltage",
      ["*", "power", "time"] => "energy", ["*", "time", "power"] => "energy",
      ["*", "time", "frequency"] => "", ["*", "frequency", "time"] => "",
      ["/", "power", "voltage"] => "current", ["/", "power", "current"] => "voltage",
      ["/", "voltage", "current"] => "resistance", ["/", "voltage", "resistance"] => "current",
      ["/", "energy", "time"] => "power", ["/", "energy", "power"] => "time",
      ["/", "", "time"] => "frequency", ["/", "", "frequency"] => "time",
    }.freeze

    PLAIN = { "time" => "s", "frequency" => "Hz", "voltage" => "V", "current" => "A", "resistance" => "ohm", "power" => "W", "energy" => "J" }.freeze

    # `a op b` for two quantities, or a number over a quantity.
    def made(op, a, b, line)
      left = a.is_a?(ZQuantity) ? UNITS[a.unit][0] : ""
      right = UNITS[b.unit][0]
      kind = MADE[[op, left, right]]
      x = left.empty? ? number(a) : apply(convert(a.unit, PLAIN[left]), a.v)
      mismatch(op, a, b, line) if kind.nil? || x.nil?
      by_zero(line) if op == "/" && b.v == 0
      y = apply(convert(b.unit, PLAIN[right]), b.v)
      r = op == "*" ? x * y : x / y
      kind.empty? ? r.to_f : ZQuantity.new(r, PLAIN[kind])
    end

    def unit(v, name, line)
      fatal("`" + name + "` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC", line) unless unit?(name)
      x = number(v)
      fatal("a unit can only follow a number", line) if x.nil?
      ZQuantity.new(x, name)
    end

    # ---------------------------------------------------------- Equality

    def all_eq(a, b)
      return false if a.length != b.length
      i = 0
      while i < a.length
        return false unless eq(a[i], b[i])
        i += 1
      end
      true
    end

    def eq(a, b)
      if a.is_a?(ZU64) || b.is_a?(ZU64)
        x = unsigned(a)
        return !x.nil? && x == unsigned(b)
      end
      if a.is_a?(ZF32) || b.is_a?(ZF32)
        x = single(a)
        y = single(b)
        return x == y unless x.nil? || y.nil?
        x = number(a)
        y = number(b)
        return !x.nil? && !y.nil? && x == y
      end
      case a
      when Integer
        return a == b if b.is_a?(Integer)
        b.is_a?(Float) && a.to_f == b
      when Float
        return a == b if b.is_a?(Float)
        b.is_a?(Integer) && a == b.to_f
      when String then b.is_a?(String) && a == b
      when true, false, nil then a.equal?(b)
      when ZQuantity
        return false unless b.is_a?(ZQuantity)
        c = common(a.unit, b.unit)
        !c.nil? && apply(c[1], a.v) == apply(c[2], b.v)
      when Array then b.is_a?(Array) && (a.equal?(b) || all_eq(a, b))
      when Hash
        return false unless b.is_a?(Hash)
        return true if a.equal?(b)
        return false if a.length != b.length
        a.each { |key, value| return false unless b.key?(key) && eq(value, b[key]) }
        true
      when ZInstance
        return false unless b.is_a?(ZInstance)
        a.equal?(b) || (a.cls.equal?(b.cls) && all_eq(a.fields, b.fields))
      when ZCase
        return false unless b.is_a?(ZCase)
        a.equal?(b) || (a.enum.equal?(b.enum) && a.index == b.index && all_eq(a.fields, b.fields))
      when ZError then a.equal?(b) || (b.is_a?(ZError) && a.kind == b.kind && a.message == b.message)
      when ZConstructor then a.equal?(b) || (b.is_a?(ZConstructor) && a.enum.equal?(b.enum) && a.index == b.index)
      else a.equal?(b)
      end
    end

    def ne(a, b)
      !eq(a, b)
    end

    # --------------------------------------------------------- Operators

    def mismatch(op, a, b, line)
      fatal("`" + op + "` does not work between " + type_name(a) + " and " + type_name(b), line)
    end

    # Both quantities as numbers in the unit they meet in, and that unit.
    def met(op, a, b, line)
      c = common(a.unit, b.unit)
      mismatch(op, a, b, line) if c.nil?
      [apply(c[1], a.v), apply(c[2], b.v), c[0]]
    end

    def add(a, b, line)
      return u64_op("+", a, b, line) if a.is_a?(ZU64) || b.is_a?(ZU64)
      return f32_op("+", a, b, line) if a.is_a?(ZF32) || b.is_a?(ZF32)
      return fits(a + b, line) if a.is_a?(Integer) && b.is_a?(Integer)
      return a + b if a.is_a?(String) && b.is_a?(String)
      return a + b if a.is_a?(Array) && b.is_a?(Array)
      if a.is_a?(ZQuantity) && b.is_a?(ZQuantity)
        x, y, unit = met("+", a, b, line)
        return ZQuantity.new(x + y, unit)
      end
      x = number(a)
      y = number(b)
      mismatch("+", a, b, line) if x.nil? || y.nil?
      x + y
    end

    def sub(a, b, line)
      return u64_op("-", a, b, line) if a.is_a?(ZU64) || b.is_a?(ZU64)
      return f32_op("-", a, b, line) if a.is_a?(ZF32) || b.is_a?(ZF32)
      return fits(a - b, line) if a.is_a?(Integer) && b.is_a?(Integer)
      if a.is_a?(ZQuantity) && b.is_a?(ZQuantity)
        x, y, unit = met("-", a, b, line)
        return ZQuantity.new(x - y, unit)
      end
      x = number(a)
      y = number(b)
      mismatch("-", a, b, line) if x.nil? || y.nil?
      x - y
    end

    def mul(a, b, line)
      return u64_op("*", a, b, line) if a.is_a?(ZU64) || b.is_a?(ZU64)
      return f32_op("*", a, b, line) if a.is_a?(ZF32) || b.is_a?(ZF32)
      if a.is_a?(Array) && b.is_a?(Integer)
        raise_("ValueError", "`*` repeats a list 0 or more times, and this is #{b}", line) if b < 0
        return a * b
      end
      return fits(a * b, line) if a.is_a?(Integer) && b.is_a?(Integer)
      return made("*", a, b, line) if a.is_a?(ZQuantity) && b.is_a?(ZQuantity)
      if a.is_a?(ZQuantity) || b.is_a?(ZQuantity)
        q, other = a.is_a?(ZQuantity) ? [a, b] : [b, a]
        y = number(other)
        mismatch("*", a, b, line) if y.nil?
        return ZQuantity.new(q.v * y, q.unit)
      end
      x = number(a)
      y = number(b)
      mismatch("*", a, b, line) if x.nil? || y.nil?
      x * y
    end

    def div(a, b, line)
      return u64_op("/", a, b, line) if a.is_a?(ZU64) || b.is_a?(ZU64)
      return f32_op("/", a, b, line) if a.is_a?(ZF32) || b.is_a?(ZF32)
      if a.is_a?(ZQuantity)
        by_zero(line) if ((b.is_a?(Integer) || b.is_a?(Float)) && b == 0) || (b.is_a?(ZQuantity) && b.v == 0)
        if b.is_a?(ZQuantity)
          return made("/", a, b, line) if common(a.unit, b.unit).nil?
          x, y, = met("/", a, b, line)
          return x / y
        end
        y = number(b)
        mismatch("/", a, b, line) if y.nil?
        return ZQuantity.new(a.v / y, a.unit)
      end
      return made("/", a, b, line) if b.is_a?(ZQuantity)
      x = number(a)
      y = number(b)
      mismatch("/", a, b, line) if x.nil? || y.nil?
      by_zero(line) if y == 0
      x / y
    end

    def floordiv(a, b, line)
      return u64_op("//", a, b, line) if a.is_a?(ZU64) || b.is_a?(ZU64)
      return f32_op("//", a, b, line) if a.is_a?(ZF32) || b.is_a?(ZF32)
      if a.is_a?(Integer) && b.is_a?(Integer)
        by_zero(line) if b == 0
        return fits(a.div(b), line)
      end
      x = number(a)
      y = number(b)
      mismatch("//", a, b, line) if x.nil? || y.nil?
      by_zero(line) if y == 0
      floor(x / y)
    end

    def mod(a, b, line)
      return u64_op("%", a, b, line) if a.is_a?(ZU64) || b.is_a?(ZU64)
      return f32_op("%", a, b, line) if a.is_a?(ZF32) || b.is_a?(ZF32)
      if a.is_a?(Integer) && b.is_a?(Integer)
        by_zero(line) if b == 0
        return a % b
      end
      x = number(a)
      y = number(b)
      mismatch("%", a, b, line) if x.nil? || y.nil?
      by_zero(line) if y == 0
      x - y * floor(x / y)
    end

    def pow(a, b, line)
      return u64_op("**", a, b, line) if a.is_a?(ZU64) || b.is_a?(ZU64)
      return f32_op("**", a, b, line) if a.is_a?(ZF32) || b.is_a?(ZF32)
      if a.is_a?(Integer) && b.is_a?(Integer)
        raise_("ValueError", "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`", line) if b < 0
        # As Rust's `checked_pow` with a u32 power.
        overflow(line) if b > 4294967295
        return b == 0 ? 1 : 0 if a == 0
        return 1 if a == 1
        return b.even? ? 1 : -1 if a == -1
        overflow(line) if b >= 64
        return fits(a**b, line)
      end
      x = number(a)
      y = number(b)
      mismatch("**", a, b, line) if x.nil? || y.nil?
      # Ruby gives a complex number for a root of a number below zero.
      # The processor gives NaN, or what the root of an infinity is.
      if x < 0 && y.finite? && y % 1 != 0
        return Float::NAN if x.finite?
        return y > 0 ? Float::INFINITY : 0.0
      end
      x**y
    end

    def bits_mismatch(op, a, b, line)
      hint = ""
      if (a == true || a == false) && (b == true || b == false)
        hint = ". For two bools Zeno spells this `and`" if op == "&"
        hint = ". For two bools Zeno spells this `or`" if op == "|"
      end
      fatal("`" + op + "` works on the bits of whole numbers, and these are " + type_name(a) + " and " + type_name(b) + hint, line)
    end

    def bitand(a, b, line)
      return u64_op("&", a, b, line) if a.is_a?(ZU64) || b.is_a?(ZU64)
      bits_mismatch("&", a, b, line) unless a.is_a?(Integer) && b.is_a?(Integer)
      a & b
    end

    def bitor(a, b, line)
      return u64_op("|", a, b, line) if a.is_a?(ZU64) || b.is_a?(ZU64)
      bits_mismatch("|", a, b, line) unless a.is_a?(Integer) && b.is_a?(Integer)
      a | b
    end

    def bitxor(a, b, line)
      return u64_op("^", a, b, line) if a.is_a?(ZU64) || b.is_a?(ZU64)
      bits_mismatch("^", a, b, line) unless a.is_a?(Integer) && b.is_a?(Integer)
      a ^ b
    end

    def shift(op, a, n, line)
      return u64_op(op, a, n, line) if a.is_a?(ZU64) || n.is_a?(ZU64)
      bits_mismatch(op, a, n, line) unless a.is_a?(Integer) && n.is_a?(Integer)
      raise_("ValueError", "cannot shift by " + n.to_s + ". An int has 64 bits, so a shift is 0 to 63", line) if n < 0 || n >= 64
      op == ">>" ? a >> n : fits(a << n, line)
    end

    def shl(a, n, line)
      shift("<<", a, n, line)
    end

    def shr(a, n, line)
      shift(">>", a, n, line)
    end

    def neg(v, line)
      return fits(-v, line) if v.is_a?(Integer)
      return -v if v.is_a?(Float)
      return ZF32.new(-v.v) if v.is_a?(ZF32)
      fatal("`-` does not work on a u64, which is never below 0. Make it an int first with `int( )`", line) if v.is_a?(ZU64)
      return ZQuantity.new(-v.v, v.unit) if v.is_a?(ZQuantity)
      fatal("`-` does not work on " + type_name(v), line)
    end

    def bitnot(v, line)
      return ZU64.new(U64_MAX ^ v.v) if v.is_a?(ZU64)
      fatal("`~` works on the bits of a whole number, and this is " + type_name(v), line) unless v.is_a?(Integer)
      ~v
    end

    # `<` and the others: -1, 0, 1, or nil for a NaN.
    def order(op, a, b, line)
      if a.is_a?(ZU64) || b.is_a?(ZU64)
        x = unsigned(a)
        y = unsigned(b)
        u64_op(op, a, b, line) if x.nil? || y.nil?
        return x <=> y
      end
      if a.is_a?(ZF32) || b.is_a?(ZF32)
        x = single(a)
        y = single(b)
        f32_op(op, a, b, line) if x.nil? || y.nil?
        return x < y ? -1 : (x > y ? 1 : (x == y ? 0 : nil))
      end
      return a <=> b if a.is_a?(Integer) && b.is_a?(Integer)
      return a <=> b if a.is_a?(String) && b.is_a?(String)
      if a.is_a?(ZQuantity) && b.is_a?(ZQuantity)
        x, y, = met(op, a, b, line)
      else
        x = number(a)
        y = number(b)
        mismatch(op, a, b, line) if x.nil? || y.nil?
      end
      x < y ? -1 : (x > y ? 1 : (x == y ? 0 : nil))
    end

    def lt(a, b, line)
      o = order("<", a, b, line)
      !o.nil? && o < 0
    end

    def le(a, b, line)
      o = order("<=", a, b, line)
      !o.nil? && o <= 0
    end

    def gt(a, b, line)
      o = order(">", a, b, line)
      !o.nil? && o > 0
    end

    def ge(a, b, line)
      o = order(">=", a, b, line)
      !o.nil? && o >= 0
    end

    def contains(op, item, container, line)
      if container.is_a?(Array)
        i = 0
        while i < container.length
          return true if eq(item, container[i])
          i += 1
        end
        return false
      end
      if container.is_a?(Hash)
        mismatch(op, item, container, line) unless item.is_a?(Integer) || item.is_a?(String)
        return container.key?(item)
      end
      return container.include?(item) if item.is_a?(String) && container.is_a?(String)
      mismatch(op, item, container, line)
    end

    def is_in(a, b, line)
      contains("in", a, b, line)
    end

    def not_in(a, b, line)
      !contains("not in", a, b, line)
    end

    # A condition must be a real bool.
    def truth(v, line)
      return v if v == true || v == false
      fatal("a condition must be true or false, but this is " + type_name(v), line)
    end

    # ---------------------------------------------------- Lists and text

    # The key of a dict: a str or an int.
    def key(v, line)
      return v if v.is_a?(Integer) || v.is_a?(String)
      fatal("a key of a dict is a str or an int, but this is " + type_name(v), line)
    end

    def position(index, length, what, line)
      fatal("an index must be an int, but this is " + type_name(index), line) unless index.is_a?(Integer)
      raise_("IndexError", "index " + index.to_s + " is out of range for a " + what + " of length " + length.to_s, line) if index < 0 || index >= length
      index
    end

    def index(object, i, line)
      case object
      when Array then object[position(i, object.length, "list", line)]
      when String then object[position(i, object.length, "str", line)]
      when Hash
        k = key(i, line)
        raise_("KeyError", "this dict has no key " + repr(i), line) unless object.key?(k)
        object[k]
      else fatal("cannot take an item from " + type_name(object), line)
      end
    end

    def set_index(object, i, value, line)
      case object
      when Array then object[position(i, object.length, "list", line)] = value
      when Hash then object[key(i, line)] = value
      else fatal("cannot set an item of " + type_name(object), line)
      end
      nil
    end

    # What a `for` loop steps through: a copy, so that the loop is not
    # upset by a change to what it is walking.
    def items(v, line)
      case v
      when Array then v.dup
      when Hash then v.keys
      when String then v.chars
      else fatal("cannot loop over " + type_name(v), line)
      end
    end

    # `Z.each(v, line) do |x|`: the body of a `for`, once for each item.
    # A block called from here is a step of Ruby's own and takes no room
    # on the processor's stack, which Ruby's `each` did before Ruby 3.4.
    def each(v, line)
      list = items(v, line)
      i = 0
      while i < list.length
        yield list[i]
        i += 1
      end
      nil
    end

    # The same for a loop with two names: the key or the position, and the value.
    def each_pair(v, line)
      if v.is_a?(Hash)
        list = v.to_a
        i = 0
        while i < list.length
          yield list[i][0], list[i][1]
          i += 1
        end
        return nil
      end
      list = items(v, line)
      i = 0
      while i < list.length
        yield i, list[i]
        i += 1
      end
      nil
    end

    # `Z.loop do`: the body of a `while`, until it breaks. Each round
    # has variables of its own, which Ruby's `while` does not give.
    def loop
      yield while true
    end

    def list(values)
      values
    end

    def dict(flat, line)
      d = {}
      i = 0
      while i < flat.length
        d[key(flat[i], line)] = flat[i + 1]
        i += 2
      end
      d
    end

    def push(list, value)
      list << value
      nil
    end

    def concat(pieces)
      pieces.join
    end

    # ----------------------------------------------- Objects and fields

    def fn(name, params, defaults, fn)
      ZFn.new(name, params, defaults, fn)
    end

    def klass(name, fields, defaults, funcs)
      ZClass.new(name, fields, defaults, funcs)
    end

    def enum(name, cases)
      ZEnum.new(name, cases)
    end

    # A case with no values is a value; one with values needs calling.
    def case_of(enum, i)
      enum.cases[i][1].empty? ? ZCase.new(enum, i, []) : ZConstructor.new(enum, i)
    end

    def is_case(value, ctor, count, line)
      fatal("this is not an enum case that carries values", line) unless ctor.is_a?(ZConstructor)
      return false unless value.is_a?(ZCase) && value.enum.equal?(ctor.enum) && value.index == ctor.index
      if value.fields.length != count
        fatal("`" + ctor.enum.cases[ctor.index][0] + "` carries " + value.fields.length.to_s + " values, but this pattern has " + count.to_s, line)
      end
      true
    end

    def case_field(value, i)
      value.fields[i]
    end

    def several(values)
      ZSeveral.new(values)
    end

    # What place `at` of the `count` on the left of `=` is given.
    def received(value, count, at, line)
      fatal("there are " + count.to_s + " places on the left of `=`, and the right gives one value, " + type_name(value), line) unless value.is_a?(ZSeveral)
      if value.values.length != count
        fatal("this gives " + value.values.length.to_s + " values, and there are " + count.to_s + " places on the left of `=`", line)
      end
      value.values[at]
    end

    STR_METHODS = %w[upper lower strip split lines replace starts_with ends_with find slice code].freeze
    LIST_METHODS = %w[append pop map filter reduce insert reverse sort clear find slice join].freeze
    DICT_METHODS = %w[get pop keys values clear].freeze

    def method?(v, name)
      case v
      when String then STR_METHODS.include?(name)
      when Array then LIST_METHODS.include?(name)
      when Hash then DICT_METHODS.include?(name)
      else false
      end
    end

    WIDTHS = {
      "u8" => [0, 255, 8, false], "u16" => [0, 65535, 16, false], "u32" => [0, 4294967295, 32, false],
      "i8" => [-128, 127, 8, true], "i16" => [-32768, 32767, 16, true], "i32" => [-2147483648, 2147483647, 32, true],
    }.freeze

    def field(object, name, line)
      case object
      when ZInstance
        at = object.cls.fields.index(name)
        return object.fields[at] if at
        method = object.cls.funcs[name]
        return ZBound.new(object, method) if method
        fatal("`" + object.cls.name + "` has no field or method `" + name + "`", line)
      when ZCase
        this = object.enum.cases[object.index]
        at = this[1].index(name)
        return object.fields[at] if at
        fatal("`" + this[0] + "` has no field `" + name + "`", line)
      when ZEnum
        at = object.cases.index { |this| this[0] == name }
        return case_of(object, at) if at
        fatal("enum `" + object.name + "` has no case `" + name + "`", line)
      when ZModule then return builtin(object.name + "." + name, line)
      when ZQuantity
        if unit?(name)
          scale = convert(object.unit, name)
          fatal("a quantity in " + object.unit + " cannot be turned into " + name + ". They measure different things", line) if scale.nil?
          return ZQuantity.new(apply(scale, object.v), name)
        end
      when ZError
        return object.message if name == "message"
        return object.kind if name == "kind"
      when ZFn
        return builtin(object.builtin + ".wrap", line) if object.builtin && name == "wrap" && WIDTHS.key?(object.builtin)
      end
      return ZMethod.new(object, name) if method?(object, name)
      fatal(type_name(object) + " has no field or method `" + name + "`", line)
    end

    def set_field(object, name, value, line)
      fatal("cannot set a field on " + type_name(object), line) unless object.is_a?(ZInstance)
      at = object.cls.fields.index(name)
      fatal("`" + object.cls.name + "` has no field `" + name + "`", line) if at.nil?
      object.fields[at] = value
      nil
    end

    # ------------------------------------------------------------- Calls

    def counted(n, word)
      n.to_s + " " + word + (n == 1 ? "" : "s")
    end

    def were(n)
      n == 1 ? "1 was" : n.to_s + " were"
    end

    # Line the arguments up with the parameters: one slot each, MISSING
    # where none was given.
    def bind(what, names, positional, named_names, named, line)
      if positional.length > names.length
        fatal("`" + what + "` takes " + counted(names.length, "argument") + ", but " + were(positional.length) + " given", line)
      end
      slots = positional.dup
      slots << MISSING while slots.length < names.length
      i = 0
      while i < named_names.length
        name = named_names[i]
        at = names.index(name)
        fatal("`" + what + "` has no argument named `" + name + "`", line) if at.nil?
        fatal("`" + what + "` was given `" + name + "` twice", line) unless slots[at].equal?(MISSING)
        slots[at] = named[i]
        i += 1
      end
      slots
    end

    # How many calls are nested now. `zeno run` stops at 1000, so a
    # program stops at the same call here.
    MAX_DEPTH = 1000

    def enter(fn, line)
      if @depth >= MAX_DEPTH
        fatal("more than 1000 calls are nested. Is `" + (fn.name != "" ? fn.name : "function") + "` calling itself forever?", line)
      end
      @depth += 1
    end

    # A call of a function of the program known where it is written, with
    # its arguments lined up already.
    def direct(fn, line, *args)
      enter(fn, line)
      begin
        fn.fn.call(*args)
      ensure
        @depth -= 1
      end
    end

    def call_function(fn, positional, named_names, named, line)
      name = fn.name != "" ? fn.name : "function"
      enter(fn, line)
      begin
        slots = bind(name, fn.params, positional, named_names, named, line)
        i = 0
        while i < slots.length
          fatal("`" + name + "` needs a value for `" + fn.params[i] + "`", line) if slots[i].equal?(MISSING) && !fn.defaults[i]
          i += 1
        end
        fn.fn.call(*slots)
      ensure
        @depth -= 1
      end
    end

    def no_named(what, named_names, line)
      fatal("`" + what + "` does not take a named argument `" + named_names[0] + "`", line) unless named_names.empty?
    end

    NONE_NAMED = [].freeze

    def call_named(callee, positional, named_names, named, line)
      case callee
      when ZFn
        if callee.builtin
          no_named(callee.name, named_names, line)
          return call_builtin(callee.builtin, positional, line)
        end
        call_function(callee, positional, named_names, named, line)
      when ZBound then call_function(callee.fn, [callee.object] + positional, named_names, named, line)
      when ZMethod
        no_named(callee.name, named_names, line)
        call_method_of(callee.receiver, callee.name, positional, line)
      when ZClass
        slots = bind(callee.name, callee.fields, positional, named_names, named, line)
        i = 0
        while i < slots.length
          if slots[i].equal?(MISSING)
            default = callee.defaults[i]
            fatal("`" + callee.name + "` needs a value for `" + callee.fields[i] + "`", line) if default.nil?
            slots[i] = default.fn.call
          end
          i += 1
        end
        ZInstance.new(callee, slots)
      when ZConstructor
        name, fields = callee.enum.cases[callee.index]
        slots = bind(name, fields, positional, named_names, named, line)
        slots.each_with_index do |slot, at|
          fatal("`" + name + "` needs a value for `" + fields[at] + "`", line) if slot.equal?(MISSING)
        end
        ZCase.new(callee.enum, callee.index, slots)
      when ZErrorType
        no_named(callee.kind, named_names, line)
        new_error(callee.kind, positional, line)
      else fatal(type_name(callee) + " cannot be called", line)
      end
    end

    def call(callee, args, line)
      call_named(callee, args, NONE_NAMED, NONE_NAMED, line)
    end

    def new_error(kind, args, line)
      fatal("`" + kind + "` takes one message", line) if args.length > 1
      ZError.new(kind, args.empty? ? "" : show(args[0]), nil)
    end

    # `object.name(args)`
    def call_method_named(object, name, positional, named_names, named, line)
      case object
      when ZInstance
        method = object.cls.funcs[name]
        if method && !object.cls.fields.include?(name)
          return call_function(method, [object] + positional, named_names, named, line)
        end
      when ZModule
        no_named(name, named_names, line)
        return call_builtin(object.name + "." + name, positional, line)
      when String, Array, Hash
        no_named(name, named_names, line)
        return call_method_of(object, name, positional, line)
      end
      call_named(field(object, name, line), positional, named_names, named, line)
    end

    def call_method(object, name, args, line)
      call_method_named(object, name, args, NONE_NAMED, NONE_NAMED, line)
    end

    # ------------------------------------------------------------ Errors

    def raise_error(value, line)
      raise ZRaise.new(value, line) if value.is_a?(ZError)
      raise ZRaise.new(ZError.new(value.cls.name, show(value), value), line) if value.is_a?(ZInstance)
      fatal("only an error can be raised, and this is " + type_name(value), line)
    end

    # `except name` catches this error?
    def catches(e, name)
      name == "Error" || name == e.value.kind
    end

    # What `except name as e` calls `e`.
    def caught(e, name)
      return e.value.object if name != "" && name != "Error" && !e.value.object.nil?
      e.value
    end

    def no_match(v, line)
      fatal("no `case` matches " + repr(v), line)
    end

    def undefined(name, line)
      fatal("`" + name + "` is not defined", line)
    end

    # ----------------------------------------------- Built-in functions

    def arity(name, args, min, max, line)
      return if args.length >= min && args.length <= max
      wanted = min == max ? counted(min, "argument") : min.to_s + " to " + max.to_s + " arguments"
      fatal("`" + name + "` takes " + wanted + ", but " + were(args.length) + " given", line)
    end

    def wrong_type(name, wanted, got, line)
      fatal("`" + name + "` needs " + wanted + ", but this is " + type_name(got), line)
    end

    def text(name, v, line)
      wrong_type(name, "a str", v, line) unless v.is_a?(String)
      v
    end

    def whole(name, v, line)
      wrong_type(name, "a whole number", v, line) unless v.is_a?(Integer)
      v
    end

    def strip(s)
      s.sub(/\A[ \t\n\r\f]+/, "").sub(/[ \t\n\r\f]+\z/, "")
    end

    # What is printed goes out in large pieces, which is far faster than
    # a line at a time.
    def flush
      return if @output.empty?
      begin
        STDOUT.write(@output)
        STDOUT.flush
      rescue Errno::EPIPE, IOError
        # Nobody is reading any more (`| head`): stop quietly.
        exit!(0)
      end
      @output.clear
    end

    def write(s)
      @output << s
      flush if @output.bytesize > 8192
    end

    # What was typed may be anything. Bytes that are not UTF-8 become
    # U+FFFD, one for each run of bytes that cannot begin a character.
    def lossy(typed)
      s = typed.dup.force_encoding(UTF8)
      return s if s.valid_encoding?
      out = String.new(encoding: UTF8)
      n = s.bytesize
      i = 0
      while i < n
        b = s.getbyte(i)
        if b < 0x80
          out << b
          i += 1
          next
        end
        # How many bytes follow, and what the second one may be.
        more = 0
        low = 0x80
        high = 0xbf
        if b >= 0xc2 && b <= 0xdf then more = 1
        elsif b == 0xe0 then more = 2; low = 0xa0
        elsif b == 0xed then more = 2; high = 0x9f
        elsif b >= 0xe1 && b <= 0xef then more = 2
        elsif b == 0xf0 then more = 3; low = 0x90
        elsif b == 0xf4 then more = 3; high = 0x8f
        elsif b >= 0xf1 && b <= 0xf3 then more = 3
        end
        got = 1
        while got <= more && i + got < n
          c = s.getbyte(i + got)
          break if c < low || c > high
          low = 0x80
          high = 0xbf
          got += 1
        end
        out << (more > 0 && got == more + 1 ? s.byteslice(i, got) : "\u{fffd}")
        i += got
      end
      out
    end

    def amount(v)
      v.is_a?(ZQuantity) ? v.v : number(v)
    end

    # The numbers `min`, `max` and `sum` are given, and the unit they share.
    def numbers(name, args, line)
      values = args.length == 1 && args[0].is_a?(Array) ? args[0] : args
      unit = nil
      values.each_with_index do |item, place|
        if item.is_a?(ZQuantity)
          if !unit.nil? && unit != item.unit
            fatal("`" + name + "` needs quantities in one unit, but these are in " + unit + " and in " + item.unit + ". Turn one into the other first, as in `x." + unit + "`", line)
          end
          fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line) if unit.nil? && place > 0
          unit = item.unit
        elsif number(item).nil?
          wrong_type(name, "numbers", item, line)
        elsif !unit.nil?
          fatal("`" + name + "` needs all plain numbers or all quantities, but these are both", line)
        end
      end
      [values, unit]
    end

    def pick(name, args, line, smaller)
      values, = numbers(name, args, line)
      raise_("ValueError", "`" + name + "` of nothing", line) if values.empty?
      if values.any? { |item| item.is_a?(ZF32) } && values.all? { |item| item.is_a?(ZF32) || item.is_a?(Integer) }
        values = values.map { |item| ZF32.new(single(item)) }
      end
      best = values[0]
      values.each do |item|
        if item.is_a?(ZU64) && best.is_a?(ZU64)
          best = item if smaller ? item.v < best.v : item.v > best.v
          next
        end
        a = amount(item)
        b = amount(best)
        best = item if smaller ? a < b : a > b
      end
      best
    end

    # The float a text spells, or nil.
    def parse_float(s)
      return nil unless s.match?(/\A[+-]?(\d+\.?\d*([eE][+-]?\d+)?|\.\d+([eE][+-]?\d+)?|inf|infinity|nan)\z/i)
      lower = s.downcase.delete_prefix("+")
      return Float::INFINITY if lower == "inf" || lower == "infinity"
      return -Float::INFINITY if lower == "-inf" || lower == "-infinity"
      return Float::NAN if lower == "nan" || lower == "-nan"
      # Ruby wants a digit on both sides of the point.
      lower.sub(/\A(-?)\./, '\10.').sub(/\.(?=e|\z)/, ".0").to_f
    end

    # Heapsort, step for step as builtins.rs and runtime.c do it, so that
    # values that are neither before the other end up where they do there.
    def heapsort(n, before, swap)
      sift = lambda do |root, last|
        while true
          child = 2 * root + 1
          return if child >= last
          child += 1 if child + 1 < last && before.call(child, child + 1)
          return unless before.call(root, child)
          swap.call(root, child)
          root = child
        end
      end
      root = n / 2 - 1
      while root >= 0
        sift.call(root, n)
        root -= 1
      end
      last = n - 1
      while last >= 1
        swap.call(0, last)
        sift.call(0, last)
        last -= 1
      end
    end

    def sort_list(items, line)
      sorted = items.dup
      heapsort(sorted.length,
               ->(i, j) { lt(sorted[i], sorted[j], line) },
               ->(i, j) { sorted[i], sorted[j] = sorted[j], sorted[i] })
      sorted.each_with_index { |item, i| items[i] = item }
    end

    def sort_by(items, fn, line)
      # The function may change the list: it is given the items the list
      # had when the sort began.
      keys = []
      given = items.dup
      i = 0
      while i < given.length
        k = call(fn, [given[i]], line)
        unless k.is_a?(Integer) || k.is_a?(Float) || k.is_a?(ZF32) || k.is_a?(ZU64) || k.is_a?(String)
          fatal("the function given to `sort` must return a number or text to order by, but it returned " + type_name(k), line)
        end
        keys << k
        i += 1
      end
      sorted = items.dup
      raise_("ValueError", "the function given to `sort` changed how many items the list has", line) if sorted.length != keys.length
      places = (0...sorted.length).to_a
      heapsort(sorted.length,
               lambda { |a, b|
                 lt(keys[a], keys[b], line) ? true : (lt(keys[b], keys[a], line) ? false : places[a] < places[b])
               },
               lambda { |a, b|
                 sorted[a], sorted[b] = sorted[b], sorted[a]
                 keys[a], keys[b] = keys[b], keys[a]
                 places[a], places[b] = places[b], places[a]
               })
      items.replace(sorted)
    end

    def slice(args, length, what, line)
      from = whole("slice", args[0], line)
      to = args.length > 1 ? whole("slice", args[1], line) : length
      if from < 0 || to < from || to > length
        raise_("IndexError", "slice " + from.to_s + " to " + to.to_s + " is out of range for a " + what + " of length " + length.to_s, line)
      end
      [from, to]
    end

    # The pieces of a str between each place a separator is. The search
    # goes over the bytes, which is the same and does not count characters.
    def split(s, sep)
      bytes = s.b
      mark = sep.b
      out = []
      from = 0
      while (at = bytes.index(mark, from))
        out << bytes[from...at].force_encoding(UTF8)
        from = at + mark.bytesize
      end
      out << bytes[from..].force_encoding(UTF8)
      out
    end

    def str_method(s, name, args, line)
      case name
      when "upper"
        arity(name, args, 0, 0, line)
        s.tr("a-z", "A-Z")
      when "lower"
        arity(name, args, 0, 0, line)
        s.tr("A-Z", "a-z")
      when "strip"
        arity(name, args, 0, 0, line)
        strip(s)
      when "split"
        arity(name, args, 0, 1, line)
        return s.scan(/[^ \t\n\r\f]+/) if args.empty?
        sep = text(name, args[0], line)
        raise_("ValueError", "cannot split on an empty str", line) if sep.empty?
        split(s, sep)
      when "lines"
        arity(name, args, 0, 0, line)
        # A "\r" goes with the "\n" just after it, and with nothing else.
        return [] if s.empty?
        parts = split(s, "\n")
        last = parts.pop
        parts = parts.map { |part| part.end_with?("\r") ? part[0, part.length - 1] : part }
        parts << last unless last.empty?
        parts
      when "replace"
        arity(name, args, 2, 2, line)
        old = text(name, args[0], line)
        now = text(name, args[1], line)
        # The new text between every character, and at both ends.
        return now + s.chars.join(now) + (s.empty? ? "" : now) if old.empty?
        split(s, old).join(now)
      when "starts_with"
        arity(name, args, 1, 1, line)
        s.start_with?(text(name, args[0], line))
      when "ends_with"
        arity(name, args, 1, 1, line)
        s.end_with?(text(name, args[0], line))
      when "find"
        arity(name, args, 1, 1, line)
        at = s.b.index(text(name, args[0], line).b)
        at.nil? ? nil : s.byteslice(0, at).length
      when "slice"
        arity(name, args, 1, 2, line)
        from, to = slice(args, s.length, "str", line)
        s[from, to - from]
      when "code"
        arity(name, args, 0, 0, line)
        raise_("ValueError", "`code` needs a str of exactly one character, and this one has " + s.length.to_s, line) if s.length != 1
        s.ord
      else fatal("a str has no method `" + name + "`", line)
      end
    end

    def dict_method(d, name, args, line)
      case name
      when "get"
        arity(name, args, 1, 2, line)
        k = key(args[0], line)
        return d[k] if d.key?(k)
        args.length > 1 ? args[1] : nil
      when "pop"
        arity(name, args, 1, 1, line)
        k = key(args[0], line)
        raise_("KeyError", "this dict has no key " + repr(args[0]), line) unless d.key?(k)
        d.delete(k)
      when "keys"
        arity(name, args, 0, 0, line)
        d.keys
      when "values"
        arity(name, args, 0, 0, line)
        d.values
      when "clear"
        arity(name, args, 0, 0, line)
        d.clear
        nil
      else fatal("a dict has no method `" + name + "`", line)
      end
    end

    def list_method(xs, name, args, line)
      case name
      when "append"
        arity(name, args, 1, 1, line)
        xs << args[0]
        nil
      when "pop"
        arity(name, args, 0, 1, line)
        return xs.delete_at(position(args[0], xs.length, "list", line)) if args.length == 1
        raise_("IndexError", "cannot pop from an empty list", line) if xs.empty?
        xs.pop
      when "insert"
        arity(name, args, 2, 2, line)
        at = whole(name, args[0], line)
        raise_("IndexError", "cannot insert at " + at.to_s + " in a list of length " + xs.length.to_s, line) if at < 0 || at > xs.length
        xs.insert(at, args[1])
        nil
      when "reverse"
        arity(name, args, 0, 0, line)
        xs.reverse!
        nil
      when "clear"
        arity(name, args, 0, 0, line)
        xs.clear
        nil
      when "sort"
        if args.length == 1
          sort_by(xs, args[0], line)
          return nil
        end
        arity(name, args, 0, 1, line)
        sort_list(xs, line)
        nil
      when "find"
        arity(name, args, 1, 1, line)
        at = 0
        while at < xs.length
          return at if eq(xs[at], args[0])
          at += 1
        end
        nil
      when "slice"
        arity(name, args, 1, 2, line)
        from, to = slice(args, xs.length, "list", line)
        xs[from, to - from]
      when "join"
        arity(name, args, 1, 1, line)
        sep = text(name, args[0], line)
        xs.each do |item|
          unless item.is_a?(String)
            fatal("`join` puts text together, and this list holds " + type_name(item) + ". Make each item text first, with `str( )`", line)
          end
        end
        xs.join(sep)
      when "map"
        arity(name, args, 1, 1, line)
        items = xs.dup
        out = []
        i = 0
        while i < items.length
          out << call(args[0], [items[i]], line)
          i += 1
        end
        out
      when "filter"
        arity(name, args, 1, 1, line)
        items = xs.dup
        kept = []
        i = 0
        while i < items.length
          keep = call(args[0], [items[i]], line)
          if keep == true
            kept << items[i]
          elsif keep != false
            fatal("the function given to `filter` must return true or false, but it returned " + type_name(keep), line)
          end
          i += 1
        end
        kept
      when "reduce"
        arity(name, args, 2, 2, line)
        items = xs.dup
        total = args[1]
        i = 0
        while i < items.length
          total = call(args[0], [total, items[i]], line)
          i += 1
        end
        total
      else fatal("a list has no method `" + name + "`", line)
      end
    end

    def call_method_of(receiver, name, args, line)
      case receiver
      when String then str_method(receiver, name, args, line)
      when Hash then dict_method(receiver, name, args, line)
      when Array then list_method(receiver, name, args, line)
      else fatal(type_name(receiver) + " has no method `" + name + "`", line)
      end
    end

    # The format after the colon of `{value:format}`.
    def formatted(v, spec, line)
      return show(v) if spec.empty?
      m = /\A([<>^])?(0)?(\d*)(?:\.(\d*))?(.)?\z/.match(spec)
      bad = "`" + spec + "` is not a format Zeno understands"
      fatal(bad, line) if m.nil?
      align = m[1]
      zero = !m[2].nil?
      width = m[3].empty? ? 0 : m[3].to_i
      precision = m[4]&.to_i
      kind = m[5]
      fatal(bad, line) if kind && kind >= "0" && kind <= "9"
      body =
        if (kind == "f" || (kind.nil? && precision)) && v.is_a?(ZQuantity)
          fixed(v.v, precision || 6) + " " + v.unit
        elsif kind == "f" || (kind.nil? && precision && v.is_a?(Float))
          x = number(v)
          wrong_type(spec, "a number", v, line) if x.nil?
          fixed(x, precision || 6)
        elsif (kind.nil? && precision.nil?) || (kind == "d" && precision.nil? && (v.is_a?(Integer) || v.is_a?(ZU64)))
          show(v)
        elsif %w[x X b].include?(kind) && precision.nil? && v.is_a?(ZU64)
          digits = v.v.to_s(kind == "b" ? 2 : 16)
          kind == "X" ? digits.upcase : digits
        elsif %w[x X b].include?(kind) && precision.nil? && v.is_a?(Integer)
          # The digits of the size of the number, with `-` before them.
          digits = v.abs.to_s(kind == "b" ? 2 : 16)
          digits = digits.upcase if kind == "X"
          (v < 0 ? "-" : "") + digits
        elsif %w[d x X b].include?(kind) && precision.nil?
          wrong_type(spec, "an int", v, line)
        else
          fatal(bad, line)
        end
      padding = [0, width - body.length].max
      is_number = v.is_a?(Integer) || v.is_a?(Float) || v.is_a?(ZQuantity) || v.is_a?(ZU64)
      if zero && is_number
        negative = body.start_with?("-")
        return (negative ? "-" : "") + "0" * padding + (negative ? body[1..] : body)
      end
      case align || (is_number ? ">" : "<")
      when "<" then body + " " * padding
      when ">" then " " * padding + body
      else
        left = padding / 2
        " " * left + body + " " * (padding - left)
      end
    end

    # -------------------------------------------------------------- JSON

    # The reader walks the bytes of the text: every mark of JSON is one
    # byte, and the bytes of a longer character pass through as they are.
    def json_spaces
      while (c = @json.getbyte(@json_at)) && (c == 32 || c == 9 || c == 10 || c == 13)
        @json_at += 1
      end
    end

    def json_word(word)
      return false unless @json.byteslice(@json_at, word.bytesize) == word
      @json_at += word.bytesize
      true
    end

    def json_hex
      digits = @json.byteslice(@json_at, 4) || ""
      @json_at += 4
      raise ZNotJson unless digits.match?(/\A[0-9a-fA-F]{4}\z/)
      digits.to_i(16)
    end

    JSON_ESCAPES = { "n" => "\n", "t" => "\t", "r" => "\r", "b" => "\b", "f" => "\f" }.freeze

    def json_text
      raise ZNotJson unless @json.getbyte(@json_at) == 34
      @json_at += 1
      out = String.new
      while true
        # All that is plain, in one piece.
        stop = @json.index(/["\\]/n, @json_at)
        raise ZNotJson if stop.nil?
        out << @json.byteslice(@json_at, stop - @json_at)
        @json_at = stop + 1
        return out.force_encoding(UTF8) if @json.getbyte(stop) == 34
        e = @json.byteslice(@json_at, 1)
        raise ZNotJson if e.nil? || e.empty?
        @json_at += 1
        if e == "u"
          code = json_hex
          if code >= 0xd800 && code < 0xdc00 && json_word("\\u")
            low = json_hex
            raise ZNotJson if low < 0xdc00 || low > 0xdfff
            code = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00)
          end
          raise ZNotJson if code >= 0xd800 && code <= 0xdfff
          out << [code].pack("U").b
        else
          out << (JSON_ESCAPES[e] || e)
        end
      end
    end

    def json_value
      json_spaces
      c = @json.getbyte(@json_at)
      raise ZNotJson if c.nil?
      case c
      when 110 # n
        return nil if json_word("null")
        raise ZNotJson
      when 116 # t
        return true if json_word("true")
        raise ZNotJson
      when 102 # f
        return false if json_word("false")
        raise ZNotJson
      when 34 then return json_text
      when 91 # [
        @json_at += 1
        items = []
        while true
          json_spaces
          return items if json_word("]")
          raise ZNotJson if !items.empty? && !json_word(",")
          items << json_value
        end
      when 123 # {
        @json_at += 1
        d = {}
        count = 0
        while true
          json_spaces
          return d if json_word("}")
          raise ZNotJson if count > 0 && !json_word(",")
          json_spaces
          name = json_text
          json_spaces
          raise ZNotJson unless json_word(":")
          v = json_value
          count += 1
          # A member that is null is left out: a dict holds no none.
          d[name] = v unless v.nil?
        end
      end
      digits = /\G[0-9+\-.eE]*/n.match(@json, @json_at)[0]
      x = parse_float(digits)
      raise ZNotJson if x.nil?
      @json_at += digits.bytesize
      return x.to_i if x.finite? && x.floor == x && x.abs < 9.0e15
      x
    end

    # The value a JSON text holds, or MISSING if the text is not JSON.
    def json_parse(s)
      @json = s.b
      @json_at = 0
      begin
        v = json_value
        json_spaces
        @json_at == @json.bytesize ? v : MISSING
      rescue ZNotJson
        MISSING
      ensure
        @json = nil
      end
    end

    def json_quoted(s)
      escaped = s.gsub(/["\\\u0000-\u001f]/) do |c|
        case c
        when '"' then '\\"'
        when "\\" then "\\\\"
        when "\n" then "\\n"
        when "\r" then "\\r"
        when "\t" then "\\t"
        else format("\\u%04x", c.ord)
        end
      end
      '"' + escaped + '"'
    end

    def json_write(v, line)
      case v
      when nil then "null"
      when true then "true"
      when false then "false"
      when Integer then v.to_s
      when Float
        raise_("ValueError", "JSON has no way to write " + show_float(v), line) unless v.finite?
        show_float(v)
      when String then json_quoted(v)
      when Array then "[" + v.map { |item| json_write(item, line) }.join(",") + "]"
      when Hash then "{" + v.map { |k, value| json_quoted(k.to_s) + ":" + json_write(value, line) }.join(",") + "}"
      when ZInstance
        parts = v.cls.fields.each_with_index.map { |name, i| json_quoted(name) + ":" + json_write(v.fields[i], line) }
        "{" + parts.join(",") + "}"
      when ZQuantity
        raise_("ValueError", "JSON has no units, and this is a quantity in " + v.unit + '. Write its number, and put the unit in the name: `["' + v.unit + '": ...]`', line)
      else raise_("ValueError", "JSON has objects, lists, text, numbers, bools and none, and this is " + type_name(v), line)
      end
    end

    # ------------------------------------------------------------- Files

    # What is at a path: "file", "nothing", or "other" (a folder, or
    # something that cannot be read).
    def at_path(path)
      return "nothing" if path.empty?
      return "other" if path.include?("\0")
      begin
        file = File.open(path, "rb")
      rescue SystemCallError
        return "other" if File.exist?(path)
        # Nothing by that name. If a name on the way to it is there and
        # is not a folder, the path cannot be followed at all.
        parent = File.dirname(path)
        parent = File.dirname(parent) while parent != File.dirname(parent) && !File.exist?(parent)
        return File.directory?(parent) ? "nothing" : "other"
      end
      begin
        file.read(1)
        "file"
      rescue SystemCallError
        "other"
      ensure
        file.close
      end
    end

    def paths(name, args, count, line)
      arity(name, args, count, count, line)
      args.each { |a| text(name, a, line) }
      args
    end

    def file_error(message, line)
      raise_("FileError", message, line)
    end

    def put(path, s, mode, line)
      begin
        File.open(path, mode) { |file| file.write(s) }
      rescue SystemCallError, ArgumentError, IOError
        file_error("cannot write `" + path + "`", line)
      end
      nil
    end

    # ----------------------------------------------- The built-in table
    #
    # Each is `b_` and its name, with `_` for the `.` of a module:
    # Z.b_print, Z.b_math_sqrt.

    def b_print(args, line)
      write(args.map { |a| show(a) }.join(" ") + "\n")
      nil
    end

    def b_input(args, line)
      arity("input", args, 0, 1, line)
      write(args.empty? ? "" : text("input", args[0], line))
      flush
      typed = STDIN.gets
      return nil if typed.nil?
      s = lossy(typed)
      # The line end is not part of the line: "\n", or "\r\n". A "\r" at
      # the end of input that no "\n" follows stays.
      if s.end_with?("\n")
        s = s[0, s.length - 1]
        s = s[0, s.length - 1] if s.end_with?("\r")
      end
      s.delete("\0")
    end

    def b_len(args, line)
      arity("len", args, 1, 1, line)
      v = args[0]
      return v.length if v.is_a?(String) || v.is_a?(Array) || v.is_a?(Hash)
      wrong_type("len", "a str, a list or a dict", v, line)
    end

    def b_range(args, line)
      arity("range", args, 1, 3, line)
      args.each { |a| wrong_type("range", "ints", a, line) unless a.is_a?(Integer) }
      start, stop, step = args.length == 1 ? [0, args[0], 1] : (args.length == 2 ? [args[0], args[1], 1] : args)
      raise_("ValueError", "the step of a range cannot be 0", line) if step == 0
      out = []
      i = start
      # A step past the last int there is ends the range.
      while i <= MAX_INT && i >= MIN_INT && (step > 0 ? i < stop : i > stop)
        out << i
        i += step
      end
      out
    end

    def b_int(args, line)
      arity("int", args, 1, 1, line)
      v = args[0]
      return v if v.is_a?(Integer)
      if v.is_a?(ZU64)
        raise_("OverflowError", v.v.to_s + " does not fit in an int (-9223372036854775808 to 9223372036854775807)", line) if v.v > MAX_INT
        return v.v
      end
      v = v.v if v.is_a?(ZF32)
      if v.is_a?(Float)
        raise_("ValueError", "cannot turn " + show_float(v) + " into an int", line) unless v.finite?
        return to_int(v.truncate.to_f)
      end
      if v.is_a?(String)
        m = /\A([+-]?)0*(\d+)\z/.match(strip(v).delete("_"))
        if m
          n = m[1] == "-" ? -m[2].to_i : m[2].to_i
          return n if n <= MAX_INT && n >= MIN_INT
        end
        raise_("ValueError", "cannot turn " + quoted(v) + " into an int", line)
      end
      wrong_type("int", "a number or a str", v, line)
    end

    def b_float(args, line)
      arity("float", args, 1, 1, line)
      v = args[0]
      return v.to_f if v.is_a?(Integer)
      return v if v.is_a?(Float)
      return v.v if v.is_a?(ZF32)
      return v.v.to_f if v.is_a?(ZU64)
      if v.is_a?(String)
        x = parse_float(strip(v))
        raise_("ValueError", "cannot turn " + quoted(v) + " into a float", line) if x.nil?
        return x
      end
      wrong_type("float", "a number or a str", v, line)
    end

    def b_f32(args, line)
      arity("f32", args, 1, 1, line)
      v = args[0]
      return v if v.is_a?(ZF32)
      return ZF32.new(single(v)) if v.is_a?(Integer)
      if v.is_a?(Float)
        r = fround(v)
        if !r.finite? && v.finite?
          raise_("OverflowError", show_float(v) + " does not fit in an f32, whose largest is about 3.4e38", line)
        end
        return ZF32.new(r)
      end
      wrong_type("f32", "a number", v, line)
    end

    def b_u64(args, line)
      arity("u64", args, 1, 1, line)
      v = args[0]
      return v if v.is_a?(ZU64)
      if v.is_a?(Integer)
        raise_("OverflowError", v.to_s + " does not fit in a u64 (0 to 18446744073709551615)", line) if v < 0
        return ZU64.new(v)
      end
      wrong_type("u64", "a whole number", v, line)
    end

    # `u64.wrap(n)`: the 64 bits of an int read with no sign (42.6).
    def b_u64__of(args, line)
      arity("u64.wrap", args, 1, 1, line)
      v = args[0]
      return v if v.is_a?(ZU64)
      return ZU64.new(v & U64_MAX) if v.is_a?(Integer)
      wrong_type("u64.wrap", "a whole number", v, line)
    end

    # `u64.wrap(a + b)`: the answer cut to 64 bits (42.6).
    def u64_wrapping(op, args, line)
      arity("u64.wrap", args, 2, 2, line)
      a, b = args
      if op == "<<" && !unsigned(a).nil? && (b.is_a?(Integer) || b.is_a?(ZU64))
        return ZU64.new((unsigned(a) << u64_shift_count(b, line)) & U64_MAX)
      end
      x = unsigned(a)
      y = unsigned(b)
      wrong_type("u64.wrap", "u64s", x.nil? ? a : b, line) if x.nil? || y.nil?
      r = case op
          when "+" then x + y
          when "-" then x - y
          else x * y
          end
      ZU64.new(r & U64_MAX)
    end

    def b_u64__add(args, line)
      u64_wrapping("+", args, line)
    end

    def b_u64__sub(args, line)
      u64_wrapping("-", args, line)
    end

    def b_u64__mul(args, line)
      u64_wrapping("*", args, line)
    end

    def b_u64__shl(args, line)
      u64_wrapping("<<", args, line)
    end

    def b_str(args, line)
      arity("str", args, 1, 1, line)
      show(args[0])
    end

    def b_abs(args, line)
      arity("abs", args, 1, 1, line)
      v = args[0]
      return v < 0 ? neg(v, line) : v if v.is_a?(Integer)
      return v.abs if v.is_a?(Float)
      return ZF32.new(v.v.abs) if v.is_a?(ZF32)
      return v if v.is_a?(ZU64)
      return ZQuantity.new(v.v.abs, v.unit) if v.is_a?(ZQuantity)
      wrong_type("abs", "a number", v, line)
    end

    def b_min(args, line)
      pick("min", args, line, true)
    end

    def b_max(args, line)
      pick("max", args, line, false)
    end

    def b_sum(args, line)
      values, unit = numbers("sum", args, line)
      unless unit.nil?
        fatal("`sum` works on plain numbers, and these have a unit. Add quantities with `+`, which keeps the unit: start from `var total = 0 V` and add each in a loop", line)
      end
      if values.any? { |item| item.is_a?(ZF32) } && values.all? { |item| item.is_a?(ZF32) || item.is_a?(Integer) }
        single32 = 0.0
        values.each { |item| single32 = fround(single32 + single(item)) }
        return ZF32.new(single32)
      end
      if values.any? { |item| item.is_a?(ZU64) }
        total = 0
        values.each do |item|
          total += unsigned(item) || 0
          raise_("OverflowError", U64_TOO_BIG, line) if total > U64_MAX
        end
        return ZU64.new(total)
      end
      total = 0
      all_ints = true
      float_total = 0.0
      values.each do |item|
        float_total += number(item)
        if all_ints && item.is_a?(Integer)
          total = add(total, item, line)
        else
          all_ints = false
        end
      end
      all_ints ? total : float_total
    end

    def b_char(args, line)
      arity("char", args, 1, 1, line)
      code = whole("char", args[0], line)
      raise_("ValueError", code.to_s + " is not the code of a character", line) if code <= 0 || code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff)
      [code].pack("U")
    end

    def b_allocate(args, line)
      fatal("manual memory is only for a program compiled to machine code", line)
    end

    def b_free(args, line)
      fatal("manual memory is only for a program compiled to machine code", line)
    end

    # A float that is a whole number, as an int: the nearest int there is
    # for one too big, and 0 for a NaN.
    def to_int(x)
      return 0 if x.nan?
      return MAX_INT if x >= 9223372036854775807.0
      return MIN_INT if x <= -9223372036854775808.0
      x.to_i
    end

    def one_number(name, args, line)
      arity(name, args, 1, 1, line)
      x = number(args[0])
      wrong_type(name, "a number", args[0], line) if x.nil?
      x
    end

    def angle(name, args, line)
      arity(name, args, 1, 1, line)
      v = args[0]
      if v.is_a?(ZQuantity)
        scale = convert(v.unit, "rad")
        fatal("`" + name + "` needs a number or an angle, such as `30 deg`, not a quantity in " + v.unit, line) if scale.nil?
        return apply(scale, v.v)
      end
      x = number(v)
      wrong_type(name, "a number or an angle, such as `30 deg`", v, line) if x.nil?
      x
    end

    def ratio(name, args, line)
      x = one_number(name, args, line)
      raise_("ValueError", "`" + name + "` needs a number from -1 to 1, and this is " + show_float(x), line) if x < -1 || x > 1
      x
    end

    def above_zero(name, args, line)
      x = one_number(name, args, line)
      raise_("ValueError", "cannot take the logarithm of " + show_float(x), line) if x <= 0
      x
    end

    def b_math_sqrt(args, line)
      x = one_number("sqrt", args, line)
      raise_("ValueError", "cannot take the square root of " + show_float(x), line) if x < 0
      # Ruby's own gives 0.0 for -0.0.
      x == 0 ? x : Math.sqrt(x)
    end

    def b_math_floor(args, line)
      to_int(floor(one_number("floor", args, line)))
    end

    def b_math_ceil(args, line)
      to_int(ceil(one_number("ceil", args, line)))
    end

    def b_math_round(args, line)
      # A half goes away from zero. Worked out from the float itself:
      # adding 0.5 first would round 0.49999999999999994 up.
      x = one_number("round", args, line)
      size = x.abs
      nearest = floor(size)
      nearest += 1 if size - nearest >= 0.5
      to_int(x < 0 ? -nearest : nearest)
    end

    def b_math_sin(args, line)
      Math.sin(angle("sin", args, line))
    end

    def b_math_cos(args, line)
      Math.cos(angle("cos", args, line))
    end

    def b_math_tan(args, line)
      Math.tan(angle("tan", args, line))
    end

    def b_math_asin(args, line)
      ZQuantity.new(Math.asin(ratio("asin", args, line)), "rad")
    end

    def b_math_acos(args, line)
      ZQuantity.new(Math.acos(ratio("acos", args, line)), "rad")
    end

    def b_math_atan(args, line)
      ZQuantity.new(Math.atan(one_number("atan", args, line)), "rad")
    end

    def b_math_atan2(args, line)
      arity("atan2", args, 2, 2, line)
      y = number(args[0])
      x = number(args[1])
      wrong_type("atan2", "two numbers", args[0], line) if y.nil?
      wrong_type("atan2", "two numbers", args[1], line) if x.nil?
      ZQuantity.new(Math.atan2(y, x), "rad")
    end

    def b_math_exp(args, line)
      Math.exp(one_number("exp", args, line))
    end

    def b_math_log(args, line)
      Math.log(above_zero("log", args, line))
    end

    def b_math_log2(args, line)
      Math.log2(above_zero("log2", args, line))
    end

    def b_math_log10(args, line)
      Math.log10(above_zero("log10", args, line))
    end

    def clock
      Process.clock_gettime(Process::CLOCK_MONOTONIC)
    end

    def b_time_sleep(args, line)
      arity("sleep", args, 1, 1, line)
      v = args[0]
      wrong_type("sleep", "a time with a unit, such as `500 ms`", v, line) unless v.is_a?(ZQuantity)
      scale = convert(v.unit, "s")
      fatal("`sleep` needs a time, such as `500 ms`, not a quantity in " + v.unit, line) if scale.nil?
      seconds = apply(scale, v.v)
      if seconds > 0
        flush
        sleep([seconds, 1e9].min)
      end
      nil
    end

    def b_time_now(args, line)
      arity("now", args, 0, 0, line)
      ZQuantity.new((clock - @started) * 1000, "ms")
    end

    def b_files_read(args, line)
      path, = paths("read", args, 1, line)
      where = at_path(path)
      file_error("there is no file `" + path + "`", line) if where == "nothing"
      file_error("cannot read `" + path + "`", line) if where == "other"
      begin
        s = File.binread(path).force_encoding(UTF8)
      rescue SystemCallError, IOError
        file_error("cannot read `" + path + "`", line)
      end
      file_error("`" + path + "` does not hold text", line) if !s.valid_encoding? || s.include?("\0")
      s
    end

    def b_files_write(args, line)
      path, s = paths("write", args, 2, line)
      put(path, s, "wb", line)
    end

    def b_files_read_bytes(args, line)
      path, = paths("read_bytes", args, 1, line)
      where = at_path(path)
      file_error("there is no file `" + path + "`", line) if where == "nothing"
      file_error("cannot read `" + path + "`", line) if where == "other"
      begin
        File.binread(path).bytes
      rescue SystemCallError, IOError
        file_error("cannot read `" + path + "`", line)
      end
    end

    def b_files_write_bytes(args, line)
      arity("write_bytes", args, 2, 2, line)
      path = text("write_bytes", args[0], line)
      items = args[1]
      wrong_type("write_bytes", "a list[u8]", items, line) unless items.is_a?(Array)
      items.each do |n|
        wrong_type("write_bytes", "a list[u8]", n, line) unless n.is_a?(Integer)
        raise_("ValueError", n.to_s + " is not a byte: a byte is a number from 0 to 255", line) if n < 0 || n > 255
      end
      begin
        File.binwrite(path, items.pack("C*"))
      rescue SystemCallError, IOError
        file_error("cannot write `" + path + "`", line)
      end
      nil
    end

    def b_files_append(args, line)
      path, s = paths("append", args, 2, line)
      put(path, s, "ab", line)
    end

    def b_files_exists(args, line)
      path, = paths("exists", args, 1, line)
      at_path(path) == "file"
    end

    def b_files_remove(args, line)
      path, = paths("remove", args, 1, line)
      where = at_path(path)
      file_error("there is no file `" + path + "`", line) if where == "nothing"
      if where == "file"
        begin
          File.delete(path)
          return nil
        rescue SystemCallError
          # Said below.
        end
      end
      file_error("cannot remove `" + path + "`", line)
    end

    # `files.list(folder)`: the names in it, sorted by their bytes, which
    # for UTF-8 is the order of the numbers of their characters.
    def b_files_list(args, line)
      folder, = paths("list", args, 1, line)
      begin
        found = Dir.children(folder)
      rescue Errno::ENOENT
        file_error("there is no folder `" + folder + "`", line)
      rescue SystemCallError, IOError, ArgumentError
        file_error("cannot list the folder `" + folder + "`", line)
      end
      names = found.map do |name|
        name = name.dup.force_encoding(UTF8)
        file_error("the folder `" + folder + "` holds a name that is not text", line) unless name.valid_encoding?
        name
      end
      names.sort
    end

    # `files.make_folder(path)`: it worked if there is a folder there at the end.
    def b_files_make_folder(args, line)
      path, = paths("make_folder", args, 1, line)
      begin
        require "fileutils"
        FileUtils.mkdir_p(path) unless path.empty?
      rescue SystemCallError, IOError, ArgumentError
        # Said below.
      end
      file_error("cannot make the folder `" + path + "`", line) unless !path.empty? && File.directory?(path)
      nil
    end

    # `program.exit(code)`: what was printed goes out, and the program
    # ends with `code`. `exit!` is not an exception a `rescue` could stop.
    def b_program_exit(args, line)
      arity("exit", args, 1, 1, line)
      code = whole("exit", args[0], line)
      raise_("ValueError", "a program ends with a number from 0 to 255, not " + code.to_s, line) if code < 0 || code > 255
      flush
      STDERR.flush
      exit!(code)
    end

    # `program.environment(name)`: the text, or none. A name with `=` or
    # a zero byte in it is in no environment.
    def b_program_environment(args, line)
      arity("environment", args, 1, 1, line)
      name = text("environment", args[0], line)
      return nil if name.empty? || name.include?("=") || name.include?("\0")
      value = ENV[name]
      value.nil? ? nil : lossy(value)
    end

    # `program.warn(text)`: one line on the error stream, after what was printed.
    def b_program_warn(args, line)
      arity("warn", args, 1, 1, line)
      said = text("warn", args[0], line)
      flush
      STDERR.write(said + "\n")
      STDERR.flush
      nil
    end

    def b_program_arguments(args, line)
      arity("arguments", args, 0, 0, line)
      ARGV.map { |word| lossy(word) }
    end

    def b_json_parse(args, line)
      arity("parse", args, 1, 1, line)
      s = text("parse", args[0], line)
      v = json_parse(s)
      if v.equal?(MISSING)
        rest = s.sub(/\A[ \t\n\r]+/, "")
        raise_("ValueError", "this text is not JSON: `" + rest[0, 30] + (rest.length > 30 ? "..." : "") + "`", line)
      end
      v
    end

    def b_json_write(args, line)
      arity("write", args, 1, 1, line)
      json_write(args[0], line)
    end

    # The sized whole numbers: `u8(n)` checks, `u8.wrap(n)` keeps the low bits.
    def sized(name, args, line)
      lo, hi, _, signed = WIDTHS[name]
      arity(name, args, 1, 1, line)
      n = args[0]
      wrong_type(name, "a whole number", n, line) unless n.is_a?(Integer)
      if n < lo || n > hi
        raise_("OverflowError", n.to_s + " does not fit in " + (signed ? "an" : "a") + " " + name + " (" + lo.to_s + " to " + hi.to_s + ")", line)
      end
      n
    end

    def wrapped(name, args, line)
      _, _, bits, signed = WIDTHS[name]
      arity(name + ".wrap", args, 1, 1, line)
      n = args[0]
      wrong_type(name + ".wrap", "a whole number", n, line) unless n.is_a?(Integer)
      low = n & ((1 << bits) - 1)
      signed && low >= (1 << (bits - 1)) ? low - (1 << bits) : low
    end

    WIDTHS.each_key do |name|
      define_method("b_" + name) { |args, line| sized(name, args, line) }
      define_method("b_" + name + "_wrap") { |args, line| wrapped(name, args, line) }
    end

    ERROR_KINDS = %w[Error ValueError IndexError KeyError ZeroDivisionError OverflowError FileError].freeze
    ERROR_KINDS.each do |kind|
      define_method("b_" + kind) { |args, line| new_error(kind, args, line) }
    end

    # The method of the table that a name stands for, or nil.
    def tabled(name)
      return nil unless name.match?(/\A[A-Za-z0-9]+(\.[A-Za-z0-9]+)?\z/)
      method = "b_" + name.tr(".", "_")
      respond_to?(method) ? method : nil
    end

    def call_builtin(name, args, line)
      method = tabled(name)
      fatal("there is no `" + name + "` in a program built for Ruby", line) if method.nil?
      public_send(method, args, line)
    end

    # Something that comes with Zeno, as a value.
    def builtin(name, line = 0)
      return Math::PI if name == "math.pi"
      return Math::E if name == "math.e"
      return ZErrorType.new(name) if ERROR_KINDS.include?(name)
      found = @builtin_values[name]
      return found if found
      dot = name.index(".")
      if tabled(name).nil?
        fatal("module `" + name[0, dot] + "` has no `" + name[(dot + 1)..] + "`", line) if dot && dot > 0
        fatal("`" + name + "` is not defined", line)
      end
      @builtin_values[name] = ZFn.new(dot ? name[(dot + 1)..] : name, [], [], nil, name)
    end

    def module_of(name)
      ZModule.new(name)
    end

    # ------------------------------------------------------- The program

    # Ruby gives a program one megabyte for the calls that are inside one
    # another, which 999 Zeno calls can pass. How much is decided before
    # Ruby starts, so the program starts itself again, asking for more.
    def with_room_for_calls
      return if ENV["RUBY_THREAD_VM_STACK_SIZE"] || !File.file?($PROGRAM_NAME)
      ENV["RUBY_THREAD_VM_STACK_SIZE"] = (64 * 1024 * 1024).to_s
      begin
        exec(RbConfig.ruby, $PROGRAM_NAME, *ARGV)
      rescue SystemCallError, NotImplementedError
        # It runs with the room it has.
      end
    end

    # Run the program, and say what stopped it if something did.
    def start(files, main)
      with_room_for_calls
      STDOUT.binmode
      STDIN.binmode
      STDERR.binmode
      @started = clock
      where = lambda do |line|
        " --> " + (files[line / LINES_A_FILE] || files[0]) + ":" + (line % LINES_A_FILE).to_s
      end
      begin
        main.call
        flush
        return
      rescue ZRaise => e
        message = "error: uncaught " + shown_name(e.value.kind) + ": " + show(e.value) + "\n" + where.call(e.at)
      rescue ZFatal => e
        message = "error: " + e.text + "\n" + where.call(e.at)
      end
      flush
      STDERR.write(message + "\n")
      exit(1)
    end
  end
end
