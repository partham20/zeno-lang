-- The Zeno runtime for Lua.
--
-- A program built with `zeno build --target lua` is its own code after
-- this: what every Zeno value is, and what each operator and built-in
-- does with it. It does what ops.rs and builtins.rs do for `zeno run`,
-- step for step and message for message, so that the program prints
-- what `zeno run` prints. It needs Lua 5.4 and nothing else.
--
-- How a Zeno value is held:
--
--   int        a Lua integer: 64 bits, as in Zeno. Lua's own sums wrap
--              around, so each step looks for that and raises Zeno's error
--   float      a Lua float
--   f32        a table with the metatable `F32`: the float it is exactly (decision 41)
--   u64        a table with the metatable `U64`: the Lua integer with the same 64 bits (decision 42)
--   str        a Lua string, in UTF-8
--   bool       true, false
--   none       `Z.none`, a value of its own. Lua's `nil` cannot be kept
--              in a list, and a Zeno list can hold none
--   list       a table of the items, from 1, marked as a list
--   dict       a table with the keys in the order they were added
--              (`keys`) and the value of each (`map`)
--   quantity   a number and its unit
--   function   a Lua function, with its Zeno name and parameters
--   object     its class and its fields
--   enum case  its enum, which case, and what it carries
--   error      its kind and its message
--
-- Which of these a table is, is said by its metatable: `List`, `Dict`
-- and the others below are used for nothing else.
--
-- The steps the printed program calls are the functions of `Z`.

local Z = {}

-- The words the program was started with.
local ARGV = {...}

local List, Dict, Quantity, Fn, Class, Instance, Enum, Case, Constructor = {}, {}, {}, {}, {}, {}, {}, {}, {}
local F32 = {}
local U64 = {}
local ErrorType, Error, Module, Method, Bound = {}, {}, {}, {}, {}
-- Several values from one function, as `return a, b` gives them, until
-- `let a, b = ...` takes them apart (decision 57).
local Several = {}
-- An error the program may catch, on its way to a `try`, and a mistake
-- in the program, which nothing catches.
local Raise, Fatal = {}, {}
-- Text that is not JSON, on its way out of the reader.
local NOT_JSON = {}

local NONE = setmetatable({}, {__tostring = function() return "none" end})
-- What a call gives a parameter it leaves out.
local MISSING = {}
Z.none, Z.MISSING = NONE, MISSING

local LINES_A_FILE = 1000000
local MAXINT, MININT, INF = math.maxinteger, math.mininteger, math.huge
local mtype, getmt, setmt = math.type, getmetatable, setmetatable
local fmt, rep, concat = string.format, string.rep, table.concat

local function fatal(message, line) error(setmt({text = message, at = line}, Fatal), 0) end
local function raise_(kind, message, line)
  error(setmt({value = setmt({kind = kind, message = message, object = false}, Error), at = line}, Raise), 0)
end
local function overflow(line) raise_("OverflowError", "the result is too big for an int", line) end
local function by_zero(line) raise_("ZeroDivisionError", "division by zero", line) end
Z.fatal = fatal

-- The number inside an int or a float, as a float.
local function is(v, kind) return type(v) == "table" and getmt(v) == kind end
local function number(v)
  local kind = mtype(v)
  if kind == "float" then return v end
  if kind == "integer" then return v + 0.0 end
  if is(v, F32) then return v.v end
  if is(v, U64) then return v.v >= 0 and v.v + 0.0 or ((v.v >> 1) | (v.v & 1)) * 2.0 end
  return nil
end

local function is_int(v) return mtype(v) == "integer" end
local function is_float(v) return mtype(v) == "float" end
-- The nearest f32 to a float, as a float.
local function fround(x) return (string.unpack("<f", string.pack("<f", x))) end
local function f32(x) return setmt({v = x}, F32) end
-- An f32, or an int as a float and then the nearest f32 (41.2).
local function single(v)
  if is(v, F32) then return v.v end
  if mtype(v) == "integer" then return fround(v + 0.0) end
  return nil
end
-- The float with the fewest digits that reads back as this f32.
local function f32_shown(x)
  if x ~= x or x == 0 or x == math.huge or x == -math.huge then return x end
  for digits = 1, 9 do
    local d = tonumber(string.format("%." .. (digits - 1) .. "e", x))
    if fround(d) == x then return d end
  end
  return x
end
-- ---- u64 (decision 42). A Lua integer has 64 bits and wraps round, so
-- a u64 is the integer with the same bits; `math.ult` orders them with
-- no sign, and `>>` fills with zeros.
local U64_TOO_BIG = "the result is too big for a u64"
local function u64(bits) return setmt({v = bits}, U64) end
-- A u64, or an int of 0 or more, as its bits; else nil.
local function unsigned(v)
  if is(v, U64) then return v.v end
  if mtype(v) == "integer" and v >= 0 then return v end
  return nil
end
-- `a // b` and `a % b` with no sign; `b` is not 0.
local function udivmod(a, b)
  if b < 0 then
    if math.ult(a, b) then return 0, a end
    return 1, a - b
  end
  if a >= 0 then return a // b, a % b end
  local q = ((a >> 1) // b) << 1
  local r = a - q * b
  if not math.ult(r, b) then q, r = q + 1, r - b end
  return q, r
end
local function u64_text(a)
  if a >= 0 then return fmt("%d", a) end
  local q, r = udivmod(a, 10)
  return fmt("%d", q) .. r
end
local function u64_shift_count(n, line)
  if is(n, U64) then
    if n.v < 0 or n.v >= 64 then raise_("ValueError", "cannot shift by " .. u64_text(n.v) .. ". A u64 has 64 bits, so a shift is 0 to 63", line) end
    return n.v
  end
  if n < 0 or n >= 64 then raise_("ValueError", "cannot shift by " .. n .. ". A u64 has 64 bits, so a shift is 0 to 63", line) end
  return n
end
local function is_negative(x) return x < 0 or (x == 0 and 1 / x < 0) end
local function is_finite(x) return x == x and x ~= INF and x ~= -INF end
-- The whole number at or below a float, as a float.
local function floor(x) return x // 1 end
local function ceil(x) return -((-x) // 1) end

local function type_name(v)
  local kind = type(v)
  if kind == "number" then return mtype(v) == "integer" and "an int" or "a float" end
  if is(v, F32) then return "an f32" end
  if is(v, U64) then return "a u64" end
  if kind == "string" then return "a str" end
  if kind == "boolean" then return "a bool" end
  if v == NONE then return "none" end
  local mt = getmt(v)
  if mt == Quantity then return "a quantity in " .. v.unit end
  if mt == List then return "a list" end
  if mt == Dict then return "a dict" end
  if mt == Fn or mt == Method or mt == Bound then return "a function" end
  if mt == Class then return "a class" end
  if mt == Instance then return "a " .. v.cls.name end
  if mt == Enum then return "an enum" end
  if mt == Constructor then return "an enum case" end
  if mt == Case then return "a " .. v.def.name end
  if mt == ErrorType then return "an error type" end
  if mt == Error then return "a " .. v.kind end
  if mt == Module then return "a module" end
  if mt == Several then return #v.values .. " values" end
  return "a value"
end

-- `Point`, not `shapes.Point`; `Stack`, not `Stack[int]`.
local function shown_name(name)
  return name:match("^[^%[]*"):match("[^.]*$")
end

-- A copy of the items of a table, from 1.
local function copied(items) return table.move(items, 1, #items, 1, {}) end

-- ------------------------------------------------------------- Showing

-- `text`, which is d.ddde-XX, with the last of its digits one higher.
-- Nothing when every digit is a 9: one digit fewer names that number,
-- and was tried before.
local function step_higher(text)
  local mantissa, power = text:match("^(.-)(e.*)$")
  for at = #mantissa, 1, -1 do
    local digit = mantissa:sub(at, at)
    if digit >= "0" and digit <= "8" then
      local after = mantissa:sub(at + 1):gsub("9", "0")
      return mantissa:sub(1, at - 1) .. string.char(digit:byte() + 1) .. after .. power
    end
  end
  return nil
end

-- A float as Rust's `{:?}` shows it: the shortest digits that read back
-- as the same number, with `.0` on a whole one, and an exponent for one
-- below 1e-4 or from 1e16 up.
local function show_float(x)
  if x ~= x then return "NaN" end
  if x == INF then return "inf" end
  if x == -INF then return "-inf" end
  if x == 0 then return is_negative(x) and "-0.0" or "0.0" end
  -- Fifteen digits are enough for most floats, and seventeen for all.
  -- The first count that reads back as the same float has the shortest
  -- digits, once the zeros at its end are taken off. The smallest floats
  -- have fewer digits than fifteen, so for them the count starts at one.
  -- At each count the digits rounded from x are tried, and then those
  -- one step higher. A power of two has the float below it half as far
  -- away as the float above, so now and then only the higher digits
  -- name it: 2^-44 is 5.684341886080802e-14, and rounded to 16 digits
  -- it ends in 801 (decisions 24.21 and 53.2).
  local text
  for places = (x < 2.2250738585072014e-308 and x > -2.2250738585072014e-308) and 0 or 14, 16 do
    text = fmt("%." .. places .. "e", x)
    if tonumber(text) == x then break end
    local higher = step_higher(text)
    if higher and tonumber(higher) == x then
      text = higher
      break
    end
  end
  local sign, lead, rest, exp = text:match("^(-?)(%d)%.?(%d*)e([-+]%d+)$")
  local digits = (lead .. rest):gsub("0+$", "")
  exp = math.tointeger(tonumber(exp))
  local count = #digits
  if exp < -4 or exp >= 16 then
    return sign .. (count > 1 and (digits:sub(1, 1) .. "." .. digits:sub(2)) or digits) .. "e" .. exp
  end
  if exp < 0 then return sign .. "0." .. rep("0", -exp - 1) .. digits end
  if count <= exp + 1 then return sign .. digits .. rep("0", exp + 1 - count) .. ".0" end
  return sign .. digits:sub(1, exp + 1) .. "." .. digits:sub(exp + 2)
end

-- The digits of a whole number held as a list of pieces of nine digits,
-- the lowest first, times a small number.
local function times(pieces, by)
  local carry = 0
  for i = 1, #pieces do
    local all = pieces[i] * by + carry
    pieces[i] = all % 1000000000
    carry = all // 1000000000
  end
  while carry > 0 do
    pieces[#pieces + 1] = carry % 1000000000
    carry = carry // 1000000000
  end
  return pieces
end

local POWERS_OF_5 = {5, 25, 125, 625, 3125, 15625, 78125, 390625}

-- A float with a fixed number of decimals, as Rust's `{:.2}` writes it:
-- from the exact value of the float, a half rounding to the even digit.
local function fixed(x, places)
  if x ~= x then return "NaN" end
  if x == INF then return "inf" end
  if x == -INF then return "-inf" end
  local negative = is_negative(x)
  if negative then x = -x end
  -- x = mantissa * 2^exponent, exactly.
  local bits = string.unpack(">i8", string.pack(">d", x))
  local exponent = (bits >> 52) & 0x7ff
  local mantissa = bits & 0xfffffffffffff
  if exponent == 0 then exponent = 1 else mantissa = mantissa | (1 << 52) end
  exponent = exponent - 1075
  -- Every float has an exact decimal text: the mantissa times 2 to the
  -- exponent, or, below one, times 5 to it with the point moved that
  -- many places.
  local pieces = {mantissa % 1000000000, mantissa // 1000000000}
  local after = 0
  if exponent >= 0 then
    local n = exponent
    while n > 0 do
      pieces = times(pieces, 1 << math.min(n, 20))
      n = n - 20
    end
  else
    after = -exponent
    local n = after
    while n > 0 do
      pieces = times(pieces, POWERS_OF_5[math.min(n, 8)])
      n = n - 8
    end
  end
  local parts = {}
  for i = #pieces, 1, -1 do parts[#parts + 1] = fmt("%09d", pieces[i]) end
  local digits = concat(parts):gsub("^0+", "")
  if #digits <= after then digits = rep("0", after - #digits + 1) .. digits end
  local whole = digits:sub(1, #digits - after)
  local part = digits:sub(#digits - after + 1)
  if #part <= places then
    part = part .. rep("0", places - #part)
  else
    local rest = part:sub(places + 1)
    part = part:sub(1, places)
    local half = "5" .. rep("0", #rest - 1)
    local last = part == "" and whole:sub(-1) or part:sub(-1)
    if rest > half or (rest == half and tonumber(last) % 2 == 1) then
      -- One more in the last place kept.
      local all = whole .. part
      local head, nines = all:match("^(.-)(9*)$")
      if head == "" then
        all = "1" .. rep("0", #nines)
      else
        all = head:sub(1, -2) .. string.char(head:byte(-1) + 1) .. rep("0", #nines)
      end
      whole = all:sub(1, #all - places)
      part = places > 0 and all:sub(#all - places + 1) or ""
    end
  end
  return (negative and "-" or "") .. whole .. (places > 0 and ("." .. part) or "")
end

local ESCAPES = {['"'] = '\\"', ["\\"] = "\\\\", ["\n"] = "\\n", ["\r"] = "\\r", ["\t"] = "\\t", ["\0"] = "\\0"}
local function escaped(c) return ESCAPES[c] or fmt("\\u{%x}", c:byte(-1)) end

-- A str in quotes, with what cannot be seen escaped.
local function quoted(s)
  s = s:gsub('[%c"\\]', escaped)
  s = s:gsub("\xc2[\x80-\x9f]", escaped)
  return '"' .. s .. '"'
end

local show

local function repr(v)
  if type(v) == "string" then return quoted(v) end
  return show(v)
end

-- How `print`, `str()` and f-strings show a value.
function show(v)
  local kind = type(v)
  if kind == "string" then return v end
  if kind == "number" then
    if mtype(v) == "integer" then return fmt("%d", v) end
    return show_float(v)
  end
  if kind == "boolean" then return v and "true" or "false" end
  if v == NONE then return "none" end
  local mt = getmt(v)
  if mt == F32 then return show_float(f32_shown(v.v)) end
  if mt == U64 then return u64_text(v.v) end
  if mt == Quantity then
    local x = v.v
    if is_finite(x) and floor(x) == x and math.abs(x) < 1e15 then return fmt("%d", math.tointeger(x)) .. " " .. v.unit end
    return show_float(x) .. " " .. v.unit
  end
  if mt == List then
    local parts = {}
    for i = 1, #v do parts[i] = repr(v[i]) end
    return "[" .. concat(parts, ", ") .. "]"
  end
  if mt == Dict then
    if #v.keys == 0 then return "[:]" end
    local parts = {}
    for i, key in ipairs(v.keys) do parts[i] = repr(key) .. ": " .. repr(v.map[key]) end
    return "[" .. concat(parts, ", ") .. "]"
  end
  if mt == Fn then return v.name ~= "" and ("<function " .. v.name .. ">") or "<function>" end
  if mt == Method then return "<function " .. v.name .. ">" end
  if mt == Bound then return "<function " .. v.fn.name .. ">" end
  if mt == Class then return "<class " .. v.name .. ">" end
  if mt == Instance then
    local parts = {}
    for i, name in ipairs(v.cls.fields) do parts[i] = name .. "=" .. repr(v.fields[i]) end
    return shown_name(v.cls.name) .. "(" .. concat(parts, ", ") .. ")"
  end
  if mt == Enum then return "<enum " .. v.name .. ">" end
  if mt == Constructor then return "<case " .. v.def.cases[v.index + 1][1] .. ">" end
  if mt == Case then
    local name = shown_name(v.def.cases[v.index + 1][1])
    if #v.fields == 0 then return name end
    local parts = {}
    for i = 1, #v.fields do parts[i] = repr(v.fields[i]) end
    return name .. "(" .. concat(parts, ", ") .. ")"
  end
  if mt == ErrorType then return "<error " .. v.kind .. ">" end
  if mt == Error then return v.message end
  if mt == Module then return "<module " .. v.name .. ">" end
  if mt == Several then
    local parts = {}
    for i = 1, #v.values do parts[i] = show(v.values[i]) end
    return "<" .. #v.values .. " values: " .. concat(parts, ", ") .. ">"
  end
  return "<value>"
end

-- --------------------------------------------------------------- Units

local UNITS = {
  ns = {"time", 1.0}, us = {"time", 1e3}, ms = {"time", 1e6}, s = {"time", 1e9}, min = {"time", 60e9}, h = {"time", 3600e9},
  Hz = {"frequency", 1.0}, kHz = {"frequency", 1e3}, MHz = {"frequency", 1e6},
  uV = {"voltage", 1.0}, mV = {"voltage", 1e3}, V = {"voltage", 1e6}, kV = {"voltage", 1e9},
  uA = {"current", 1.0}, mA = {"current", 1e3}, A = {"current", 1e6},
  ohm = {"resistance", 1.0}, kohm = {"resistance", 1e3}, Mohm = {"resistance", 1e6},
  mW = {"power", 1.0}, W = {"power", 1e3}, kW = {"power", 1e6},
  J = {"energy", 1.0}, Wh = {"energy", 3600.0}, kWh = {"energy", 3600e3},
  degC = {"temperature in Celsius", 1.0}, K = {"temperature in kelvin", 1.0},
  mm = {"length", 1.0}, cm = {"length", 10.0}, m = {"length", 1e3}, km = {"length", 1e6},
  mg = {"mass", 1.0}, g = {"mass", 1e3}, kg = {"mass", 1e6},
  deg = {"angle in degrees", 1.0}, rad = {"angle in radians", 1.0}, rpm = {"rotation", 1.0},
}

-- How to turn a number in one unit into another: {times, over}, or nil.
local function convert(from, to)
  if from == "deg" and to == "rad" then return {math.pi / 180, 1.0} end
  if from == "rad" and to == "deg" then return {180 / math.pi, 1.0} end
  local a, b = UNITS[from], UNITS[to]
  if not a or not b or a[1] ~= b[1] then return nil end
  if a[2] >= b[2] then return {a[2] / b[2], 1.0} end
  return {1.0, b[2] / a[2]}
end
local function apply(scale, x) return (x * scale[1]) / scale[2] end

-- The unit two quantities meet in: the finer one, and how each gets there.
local function common(a, b)
  local ua, ub = UNITS[a], UNITS[b]
  if not ua or not ub or ua[1] ~= ub[1] then return nil end
  local unit = ua[2] <= ub[2] and a or b
  return unit, convert(a, unit), convert(b, unit)
end

local function quantity(x, unit) return setmt({v = x, unit = unit}, Quantity) end

function Z.unit(v, name, line)
  if not UNITS[name] then fatal("`" .. name .. "` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC", line) end
  local x = number(v)
  if x == nil then fatal("a unit can only follow a number", line) end
  return quantity(x, name)
end

-- ------------------------------------------------------------ Equality

local function eq(a, b)
  if is(a, U64) or is(b, U64) then
    local x = unsigned(a)
    return x ~= nil and x == unsigned(b)
  end
  if is(a, F32) or is(b, F32) then
    local x, y = single(a), single(b)
    if x ~= nil and y ~= nil then return x == y end
    x, y = number(a), number(b)
    return x ~= nil and y ~= nil and x == y
  end
  local ka, kb = type(a), type(b)
  if ka == "number" then
    if kb ~= "number" then return false end
    -- An int beside a float is looked at as a float.
    if mtype(a) == mtype(b) then return a == b end
    return a + 0.0 == b + 0.0
  end
  if ka ~= "table" or kb ~= "table" then return a == b end
  if a == b then return true end
  local mt = getmt(a)
  if mt ~= getmt(b) then return false end
  if mt == Quantity then
    local unit, to_a, to_b = common(a.unit, b.unit)
    return unit ~= nil and apply(to_a, a.v) == apply(to_b, b.v)
  end
  if mt == List then
    if #a ~= #b then return false end
    for i = 1, #a do
      if not eq(a[i], b[i]) then return false end
    end
    return true
  end
  if mt == Dict then
    if #a.keys ~= #b.keys then return false end
    for _, key in ipairs(a.keys) do
      local other = b.map[key]
      if other == nil or not eq(a.map[key], other) then return false end
    end
    return true
  end
  if mt == Instance or mt == Case then
    if mt == Instance and a.cls ~= b.cls then return false end
    if mt == Case and (a.def ~= b.def or a.index ~= b.index) then return false end
    for i = 1, #a.fields do
      if not eq(a.fields[i], b.fields[i]) then return false end
    end
    return true
  end
  if mt == Error then return a.kind == b.kind and a.message == b.message end
  if mt == Constructor then return a.def == b.def and a.index == b.index end
  return false
end
Z.eq = eq
function Z.ne(a, b) return not eq(a, b) end

-- ----------------------------------------------------------- Operators

local function mismatch(op, a, b, line)
  fatal("`" .. op .. "` does not work between " .. type_name(a) .. " and " .. type_name(b), line)
end

local function f32_op(op, a, b, line)
  local x, y = single(a), single(b)
  if x == nil or y == nil then
    if is_float(a) or is_float(b) then
      fatal("`" .. op .. "` between " .. type_name(a) .. " and " .. type_name(b) .. " would have to choose between 32 bits and 64. Write `f32( )` around the float to work in 32, or `float( )` around the f32 to work in 64", line)
    end
    mismatch(op, a, b, line)
  end
  if (op == "/" or op == "//" or op == "%") and y == 0 then by_zero(line) end
  local r
  if op == "+" then r = x + y
  elseif op == "-" then r = x - y
  elseif op == "*" then r = x * y
  elseif op == "/" then r = x / y
  elseif op == "//" then r = floor(x / y)
  elseif op == "%" then r = x - y * floor(x / y)
  else r = Z.pow(x, y, line) end
  return f32(fround(r))
end

-- Both quantities as numbers in the unit they meet in.
local function met(op, a, b, line)
  local unit, to_a, to_b = common(a.unit, b.unit)
  if unit == nil then mismatch(op, a, b, line) end
  return apply(to_a, a.v), apply(to_b, b.v), unit
end

-- What `*` and `/` make of two quantities (decision 43): the operator,
-- what each side measures ("" for a plain number), and what the answer
-- measures ("" for a plain number). The answer is in the unit of its
-- kind with no prefix.
local MADE = {
  ["* voltage current"] = "power", ["* current voltage"] = "power",
  ["* current resistance"] = "voltage", ["* resistance current"] = "voltage",
  ["* power time"] = "energy", ["* time power"] = "energy",
  ["* time frequency"] = "", ["* frequency time"] = "",
  ["/ power voltage"] = "current", ["/ power current"] = "voltage",
  ["/ voltage current"] = "resistance", ["/ voltage resistance"] = "current",
  ["/ energy time"] = "power", ["/ energy power"] = "time",
  ["/  time"] = "frequency", ["/  frequency"] = "time",
}

local PLAIN = {time = "s", frequency = "Hz", voltage = "V", current = "A", resistance = "ohm", power = "W", energy = "J"}

-- `a op b` for two quantities, or a number over a quantity.
local function made(op, a, b, line)
  local left = is(a, Quantity) and UNITS[a.unit][1] or ""
  local right = UNITS[b.unit][1]
  local kind = MADE[op .. " " .. left .. " " .. right]
  local x
  if left == "" then x = number(a) else x = apply(convert(a.unit, PLAIN[left]), a.v) end
  if kind == nil or x == nil then mismatch(op, a, b, line) end
  if op == "/" and b.v == 0 then by_zero(line) end
  local y = apply(convert(b.unit, PLAIN[right]), b.v)
  local r
  if op == "*" then r = x * y else r = x / y end
  if kind == "" then return r + 0.0 end
  return quantity(r, PLAIN[kind])
end

-- `a op b` where one side is a u64 (decision 42).
local function u64_op(op, a, b, line)
  if (op == "<<" or op == ">>") and is(a, U64) and (is_int(b) or is(b, U64)) then
    local n = u64_shift_count(b, line)
    if op == ">>" then return u64(a.v >> n) end
    local r = a.v << n
    if r >> n ~= a.v then raise_("OverflowError", U64_TOO_BIG, line) end
    return u64(r)
  end
  local x, y = unsigned(a), unsigned(b)
  if x == nil or y == nil then
    fatal("`" .. op .. "` between " .. type_name(a) .. " and " .. type_name(b) .. " would have to choose between a u64 and something else. Write `u64( )` or `int( )` around one of them", line)
  end
  if op == "+" then
    local r = x + y
    if math.ult(r, x) then raise_("OverflowError", U64_TOO_BIG, line) end
    return u64(r)
  elseif op == "-" then
    if math.ult(x, y) then raise_("OverflowError", "the result is below 0, and a u64 never is", line) end
    return u64(x - y)
  elseif op == "*" then
    local r = x * y
    if x ~= 0 and (udivmod(r, x)) ~= y then raise_("OverflowError", U64_TOO_BIG, line) end
    return u64(r)
  elseif op == "/" then
    if y == 0 then by_zero(line) end
    return number(u64(x)) / number(u64(y))
  elseif op == "//" or op == "%" then
    if y == 0 then by_zero(line) end
    local q, r = udivmod(x, y)
    return u64(op == "//" and q or r)
  elseif op == "**" then
    if y == 0 then return u64(1) end
    if x == 0 or x == 1 then return u64(x) end
    if y < 0 or y > 64 then raise_("OverflowError", U64_TOO_BIG, line) end
    local r = u64(1)
    for _ = 1, y do r = u64_op("*", r, u64(x), line) end
    return r
  elseif op == "&" then return u64(x & y)
  elseif op == "|" then return u64(x | y)
  end
  return u64(x ~ y)
end

function Z.add(a, b, line)
  if is(a, U64) or is(b, U64) then return u64_op("+", a, b, line) end
  if is(a, F32) or is(b, F32) then return f32_op("+", a, b, line) end
  local ka, kb = mtype(a), mtype(b)
  if ka == "integer" and kb == "integer" then
    -- A sum that does not fit comes back with the wrong sign.
    local r = a + b
    if ((a ~ r) & (b ~ r)) < 0 then overflow(line) end
    return r
  end
  if ka and kb then return a + b end
  if type(a) == "string" and type(b) == "string" then return a .. b end
  if is(a, List) and is(b, List) then
    local out = copied(a)
    table.move(b, 1, #b, #out + 1, out)
    return setmt(out, List)
  end
  if is(a, Quantity) and is(b, Quantity) then
    local x, y, unit = met("+", a, b, line)
    return quantity(x + y, unit)
  end
  mismatch("+", a, b, line)
end

function Z.sub(a, b, line)
  if is(a, U64) or is(b, U64) then return u64_op("-", a, b, line) end
  if is(a, F32) or is(b, F32) then return f32_op("-", a, b, line) end
  local ka, kb = mtype(a), mtype(b)
  if ka == "integer" and kb == "integer" then
    local r = a - b
    if ((a ~ b) & (a ~ r)) < 0 then overflow(line) end
    return r
  end
  if ka and kb then return a - b end
  if is(a, Quantity) and is(b, Quantity) then
    local x, y, unit = met("-", a, b, line)
    return quantity(x - y, unit)
  end
  mismatch("-", a, b, line)
end

-- The product of two ints, or nil if it does not fit.
local function product(a, b)
  local r = a * b
  if a ~= 0 and (r // a ~= b or (a == -1 and b == MININT)) then return nil end
  return r
end

function Z.mul(a, b, line)
  if is(a, U64) or is(b, U64) then return u64_op("*", a, b, line) end
  if is(a, F32) or is(b, F32) then return f32_op("*", a, b, line) end
  local ka, kb = mtype(a), mtype(b)
  if is(a, List) and kb == "integer" then
    if b < 0 then raise_("ValueError", "`*` repeats a list 0 or more times, and this is " .. tostring(b), line) end
    local out = {}
    for _ = 1, b do table.move(a, 1, #a, #out + 1, out) end
    return setmt(out, List)
  end
  if ka == "integer" and kb == "integer" then
    local r = product(a, b)
    if r == nil then overflow(line) end
    return r
  end
  if ka and kb then return a * b end
  if is(a, Quantity) and is(b, Quantity) then return made("*", a, b, line) end
  if is(a, Quantity) or is(b, Quantity) then
    local q, other = a, b
    if not is(a, Quantity) then q, other = b, a end
    local y = number(other)
    if y == nil then mismatch("*", a, b, line) end
    return quantity(q.v * y, q.unit)
  end
  mismatch("*", a, b, line)
end

function Z.div(a, b, line)
  if is(a, U64) or is(b, U64) then return u64_op("/", a, b, line) end
  if is(a, F32) or is(b, F32) then return f32_op("/", a, b, line) end
  if is(a, Quantity) then
    if (mtype(b) and b == 0) or (is(b, Quantity) and b.v == 0) then by_zero(line) end
    if is(b, Quantity) then
      if common(a.unit, b.unit) == nil then return made("/", a, b, line) end
      local x, y = met("/", a, b, line)
      return x / y
    end
    local y = number(b)
    if y == nil then mismatch("/", a, b, line) end
    return quantity(a.v / y, a.unit)
  end
  if is(b, Quantity) then return made("/", a, b, line) end
  local x, y = number(a), number(b)
  if x == nil or y == nil then mismatch("/", a, b, line) end
  if y == 0 then by_zero(line) end
  return x / y
end

function Z.floordiv(a, b, line)
  if is(a, U64) or is(b, U64) then return u64_op("//", a, b, line) end
  if is(a, F32) or is(b, F32) then return f32_op("//", a, b, line) end
  local ka, kb = mtype(a), mtype(b)
  if ka == "integer" and kb == "integer" then
    if b == 0 then by_zero(line) end
    if b == -1 then
      if a == MININT then overflow(line) end
      return -a
    end
    return a // b
  end
  if not ka or not kb then mismatch("//", a, b, line) end
  if b == 0 then by_zero(line) end
  return floor((a + 0.0) / (b + 0.0))
end

function Z.mod(a, b, line)
  if is(a, U64) or is(b, U64) then return u64_op("%", a, b, line) end
  if is(a, F32) or is(b, F32) then return f32_op("%", a, b, line) end
  local ka, kb = mtype(a), mtype(b)
  if ka == "integer" and kb == "integer" then
    if b == 0 then by_zero(line) end
    if b == -1 then return 0 end
    return a % b
  end
  if not ka or not kb then mismatch("%", a, b, line) end
  if b == 0 then by_zero(line) end
  local x, y = a + 0.0, b + 0.0
  return x - y * floor(x / y)
end

function Z.pow(a, b, line)
  if is(a, U64) or is(b, U64) then return u64_op("**", a, b, line) end
  if is(a, F32) or is(b, F32) then return f32_op("**", a, b, line) end
  local ka, kb = mtype(a), mtype(b)
  if ka == "integer" and kb == "integer" then
    if b < 0 then raise_("ValueError", "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`", line) end
    -- As Rust's `checked_pow` with a u32 power.
    if b > 4294967295 then overflow(line) end
    if a == 0 then return b == 0 and 1 or 0 end
    if a == 1 then return 1 end
    if a == -1 then return b % 2 == 0 and 1 or -1 end
    if b >= 64 then overflow(line) end
    local r = 1
    for _ = 1, b do
      r = product(r, a)
      if r == nil then overflow(line) end
    end
    return r
  end
  if not ka or not kb then mismatch("**", a, b, line) end
  local x, y = a + 0.0, b + 0.0
  if x == 1.0 or (x == -1.0 and (y == INF or y == -INF)) then return 1.0 end
  return x ^ y
end

local function bits_mismatch(op, a, b, line)
  local hint = ""
  if type(a) == "boolean" and type(b) == "boolean" then
    if op == "&" then hint = ". For two bools Zeno spells this `and`" end
    if op == "|" then hint = ". For two bools Zeno spells this `or`" end
  end
  fatal("`" .. op .. "` works on the bits of whole numbers, and these are " .. type_name(a) .. " and " .. type_name(b) .. hint, line)
end

function Z.bitand(a, b, line)
  if is(a, U64) or is(b, U64) then return u64_op("&", a, b, line) end
  if not is_int(a) or not is_int(b) then bits_mismatch("&", a, b, line) end
  return a & b
end
function Z.bitor(a, b, line)
  if is(a, U64) or is(b, U64) then return u64_op("|", a, b, line) end
  if not is_int(a) or not is_int(b) then bits_mismatch("|", a, b, line) end
  return a | b
end
function Z.bitxor(a, b, line)
  if is(a, U64) or is(b, U64) then return u64_op("^", a, b, line) end
  if not is_int(a) or not is_int(b) then bits_mismatch("^", a, b, line) end
  return a ~ b
end

-- `>>` that keeps the sign: Lua's own fills with zeros.
local function shifted_right(a, n)
  if n == 63 then return a < 0 and -1 or 0 end
  return a // (1 << n)
end
local function shift(op, a, n, line)
  if is(a, U64) or is(n, U64) then return u64_op(op, a, n, line) end
  if not is_int(a) or not is_int(n) then bits_mismatch(op, a, n, line) end
  if n < 0 or n >= 64 then raise_("ValueError", "cannot shift by " .. n .. ". An int has 64 bits, so a shift is 0 to 63", line) end
  if op == ">>" then return shifted_right(a, n) end
  local shifted = a << n
  if shifted_right(shifted, n) ~= a then overflow(line) end
  return shifted
end
function Z.shl(a, n, line) return shift("<<", a, n, line) end
function Z.shr(a, n, line) return shift(">>", a, n, line) end

function Z.neg(v, line)
  local kind = mtype(v)
  if kind == "integer" then
    if v == MININT then overflow(line) end
    return -v
  end
  if kind == "float" then return -v end
  if is(v, F32) then return f32(-v.v) end
  if is(v, U64) then fatal("`-` does not work on a u64, which is never below 0. Make it an int first with `int( )`", line) end
  if is(v, Quantity) then return quantity(-v.v, v.unit) end
  fatal("`-` does not work on " .. type_name(v), line)
end
function Z.bitnot(v, line)
  if is(v, U64) then return u64(~v.v) end
  if not is_int(v) then fatal("`~` works on the bits of a whole number, and this is " .. type_name(v), line) end
  return ~v
end

-- `<` and the others: -1, 0, 1, or nil for a NaN.
local function order(op, a, b, line)
  if is(a, U64) or is(b, U64) then
    local x, y = unsigned(a), unsigned(b)
    if x == nil or y == nil then u64_op(op, a, b, line) end
    if x == y then return 0 end
    return math.ult(x, y) and -1 or 1
  end
  if is(a, F32) or is(b, F32) then
    local x, y = single(a), single(b)
    if x == nil or y == nil then f32_op(op, a, b, line) end
    if x < y then return -1 end
    if x > y then return 1 end
    if x == y then return 0 end
    return nil
  end
  local ka, kb = mtype(a), mtype(b)
  if ka == "integer" and kb == "integer" then return a < b and -1 or (a > b and 1 or 0) end
  local x, y
  if ka and kb then
    x, y = a + 0.0, b + 0.0
  elseif type(a) == "string" and type(b) == "string" then
    return a < b and -1 or (a > b and 1 or 0)
  elseif is(a, Quantity) and is(b, Quantity) then
    x, y = met(op, a, b, line)
  else
    mismatch(op, a, b, line)
  end
  if x < y then return -1 end
  if x > y then return 1 end
  if x == y then return 0 end
  return nil
end
local function lt(a, b, line)
  local o = order("<", a, b, line)
  return o ~= nil and o < 0
end
Z.lt = lt
function Z.le(a, b, line)
  local o = order("<=", a, b, line)
  return o ~= nil and o <= 0
end
function Z.gt(a, b, line)
  local o = order(">", a, b, line)
  return o ~= nil and o > 0
end
function Z.ge(a, b, line)
  local o = order(">=", a, b, line)
  return o ~= nil and o >= 0
end

local function contains(op, item, container, line)
  if is(container, List) then
    for i = 1, #container do
      if eq(item, container[i]) then return true end
    end
    return false
  end
  if is(container, Dict) then
    if not is_int(item) and type(item) ~= "string" then mismatch(op, item, container, line) end
    return container.map[item] ~= nil
  end
  if type(item) == "string" and type(container) == "string" then return container:find(item, 1, true) ~= nil end
  mismatch(op, item, container, line)
end
function Z.is_in(a, b, line) return contains("in", a, b, line) end
function Z.not_in(a, b, line) return not contains("not in", a, b, line) end

-- A condition must be a real bool.
function Z.truth(v, line)
  if v == true or v == false then return v end
  fatal("a condition must be true or false, but this is " .. type_name(v), line)
end

-- ------------------------------------------------------ Lists and text

-- A str is UTF-8: a character is one to four bytes. Text that is all
-- ASCII, which most is, is taken a byte at a time.
local function is_ascii(s) return not s:find("[\x80-\xff]") end
local function chars(s)
  local out, n = {}, 0
  if is_ascii(s) then
    for i = 1, #s do out[i] = s:sub(i, i) end
    return out
  end
  for c in s:gmatch(utf8.charpattern) do
    n = n + 1
    out[n] = c
  end
  return out
end
local function str_len(s)
  if is_ascii(s) then return #s end
  local _, more = s:gsub("[\x80-\xbf]", "")
  return #s - more
end
-- The part of a str from one character to before another, counted from 0.
local function str_part(s, from, to)
  if is_ascii(s) then return s:sub(from + 1, to) end
  return s:sub(utf8.offset(s, from + 1), utf8.offset(s, to + 1) - 1)
end

-- The key of a dict: a str or an int.
local function key(v, line)
  if is_int(v) or type(v) == "string" then return v end
  fatal("a key of a dict is a str or an int, but this is " .. type_name(v), line)
end

local function position(index, length, what, line)
  if not is_int(index) then fatal("an index must be an int, but this is " .. type_name(index), line) end
  if index < 0 or index >= length then raise_("IndexError", "index " .. index .. " is out of range for a " .. what .. " of length " .. length, line) end
  return index
end

function Z.index(object, i, line)
  if type(object) == "string" then
    local at = position(i, str_len(object), "str", line)
    return str_part(object, at, at + 1)
  end
  local mt = getmt(object)
  if mt == List then return object[position(i, #object, "list", line) + 1] end
  if mt == Dict then
    local value = object.map[key(i, line)]
    if value == nil then raise_("KeyError", "this dict has no key " .. repr(i), line) end
    return value
  end
  fatal("cannot take an item from " .. type_name(object), line)
end

local function dict_set(d, k, value)
  if d.map[k] == nil then d.keys[#d.keys + 1] = k end
  d.map[k] = value
end

function Z.set_index(object, i, value, line)
  if is(object, List) then
    object[position(i, #object, "list", line) + 1] = value
  elseif is(object, Dict) then
    dict_set(object, key(i, line), value)
  else
    fatal("cannot set an item of " .. type_name(object), line)
  end
end

-- What a `for` loop steps through: a copy, so that the loop is not upset
-- by a change to what it is walking.
local function items_of(v, line)
  if type(v) == "string" then return chars(v) end
  if is(v, List) then return copied(v) end
  if is(v, Dict) then return copied(v.keys) end
  fatal("cannot loop over " .. type_name(v), line)
end

-- `for x in Z.items(v, line) do`: each item in turn.
function Z.items(v, line)
  local items, at = items_of(v, line), 0
  return function()
    at = at + 1
    return items[at]
  end
end

-- The same for a loop with two names: the key or the position, and the value.
function Z.pairs(v, line)
  local at = 0
  if is(v, Dict) then
    local keys, values = copied(v.keys), {}
    for i, k in ipairs(keys) do values[i] = v.map[k] end
    return function()
      at = at + 1
      return keys[at], values[at]
    end
  end
  local items = items_of(v, line)
  return function()
    at = at + 1
    local item = items[at]
    if item == nil then return nil end
    return at - 1, item
  end
end

local function list(values) return setmt(values, List) end
local function new_dict() return setmt({keys = {}, map = {}}, Dict) end
Z.list = list
function Z.dict(flat, line)
  local d = new_dict()
  for i = 1, #flat, 2 do dict_set(d, key(flat[i], line), flat[i + 1]) end
  return d
end
function Z.push(items, value)
  items[#items + 1] = value
  return NONE
end
function Z.concat(pieces) return concat(pieces) end

-- ------------------------------------------------- Objects and fields

function Z.fn(name, params, defaults, fn)
  return setmt({name = name, params = params, defaults = defaults, fn = fn, builtin = false}, Fn)
end

-- `methods` is each method by its name. `at` says where each field is.
function Z.class(name, fields, defaults, methods)
  local at = {}
  for i, field in ipairs(fields) do at[field] = i end
  return setmt({name = name, fields = fields, defaults = defaults, methods = methods, at = at}, Class)
end

-- Each case is {name, {the names of what it carries}}.
function Z.enum(name, cases) return setmt({name = name, cases = cases}, Enum) end

-- A case with no values is a value; one with values needs calling.
local function case(def, i)
  if #def.cases[i + 1][2] == 0 then return setmt({def = def, index = i, fields = {}}, Case) end
  return setmt({def = def, index = i}, Constructor)
end
Z.case = case

function Z.is_case(value, ctor, count, line)
  if not is(ctor, Constructor) then fatal("this is not an enum case that carries values", line) end
  if not is(value, Case) or value.def ~= ctor.def or value.index ~= ctor.index then return false end
  if #value.fields ~= count then
    fatal("`" .. ctor.def.cases[ctor.index + 1][1] .. "` carries " .. #value.fields .. " values, but this pattern has " .. count, line)
  end
  return true
end
function Z.case_field(value, i) return value.fields[i + 1] end
function Z.several(values) return setmt({values = values}, Several) end
-- What place `at` of the `count` on the left of `=` is given.
function Z.received(value, count, at, line)
  if not is(value, Several) then fatal("there are " .. count .. " places on the left of `=`, and the right gives one value, " .. type_name(value), line) end
  if #value.values ~= count then fatal("this gives " .. #value.values .. " values, and there are " .. count .. " places on the left of `=`", line) end
  return value.values[at + 1]
end

local function place_of(names, name)
  for i, other in ipairs(names) do
    if other == name then return i end
  end
  return nil
end

local WIDTHS = {
  u8 = {0, 255, 8, false}, u16 = {0, 65535, 16, false}, u32 = {0, 4294967295, 32, false},
  i8 = {-128, 127, 8, true}, i16 = {-32768, 32767, 16, true}, i32 = {-2147483648, 2147483647, 32, true},
}

local STR_METHODS = {upper = true, lower = true, strip = true, split = true, lines = true, replace = true, starts_with = true, ends_with = true, find = true, slice = true, code = true}
local LIST_METHODS = {append = true, pop = true, map = true, filter = true, reduce = true, insert = true, reverse = true, sort = true, clear = true, find = true, slice = true, join = true}
local DICT_METHODS = {get = true, pop = true, keys = true, values = true, clear = true}
local function has_method(v, name)
  if type(v) == "string" then return STR_METHODS[name] == true end
  if is(v, List) then return LIST_METHODS[name] == true end
  if is(v, Dict) then return DICT_METHODS[name] == true end
  return false
end

local builtin

local function field(object, name, line)
  local mt = type(object) == "table" and getmt(object) or nil
  if mt == Instance then
    local at = object.cls.at[name]
    if at then return object.fields[at] end
    local method = object.cls.methods[name]
    if method then return setmt({object = object, fn = method}, Bound) end
    fatal("`" .. object.cls.name .. "` has no field or method `" .. name .. "`", line)
  end
  if mt == Case then
    local this = object.def.cases[object.index + 1]
    local at = place_of(this[2], name)
    if at then return object.fields[at] end
    fatal("`" .. this[1] .. "` has no field `" .. name .. "`", line)
  end
  if mt == Enum then
    for at, this in ipairs(object.cases) do
      if this[1] == name then return case(object, at - 1) end
    end
    fatal("enum `" .. object.name .. "` has no case `" .. name .. "`", line)
  end
  if mt == Module then return builtin(object.name .. "." .. name, line) end
  if mt == Quantity and UNITS[name] then
    local scale = convert(object.unit, name)
    if scale == nil then fatal("a quantity in " .. object.unit .. " cannot be turned into " .. name .. ". They measure different things", line) end
    return quantity(apply(scale, object.v), name)
  end
  if mt == Error and name == "message" then return object.message end
  if mt == Error and name == "kind" then return object.kind end
  if mt == Fn and object.builtin and name == "wrap" and WIDTHS[object.builtin] then return builtin(object.builtin .. ".wrap", line) end
  if has_method(object, name) then return setmt({receiver = object, name = name}, Method) end
  fatal(type_name(object) .. " has no field or method `" .. name .. "`", line)
end
Z.field = field

function Z.set_field(object, name, value, line)
  if is(object, Instance) then
    local at = object.cls.at[name]
    if not at then fatal("`" .. object.cls.name .. "` has no field `" .. name .. "`", line) end
    object.fields[at] = value
    return
  end
  fatal("cannot set a field on " .. type_name(object), line)
end

-- --------------------------------------------------------------- Calls

local function counted(n, word) return n .. " " .. word .. (n == 1 and "" or "s") end
local function were(n) return n == 1 and "1 was" or (n .. " were") end

-- Line the arguments up with the parameters: one slot each, MISSING
-- where none was given.
local function bind(what, names, positional, named_names, named, line)
  if #positional > #names then
    fatal("`" .. what .. "` takes " .. counted(#names, "argument") .. ", but " .. were(#positional) .. " given", line)
  end
  local slots = copied(positional)
  for i = #slots + 1, #names do slots[i] = MISSING end
  for i, name in ipairs(named_names) do
    local at = place_of(names, name)
    if not at then fatal("`" .. what .. "` has no argument named `" .. name .. "`", line) end
    if slots[at] ~= MISSING then fatal("`" .. what .. "` was given `" .. name .. "` twice", line) end
    slots[at] = named[i]
  end
  return slots
end

-- How many calls are nested now. `zeno run` stops at 1000, so a program
-- stops at the same call here. A call that an error leaves is taken off
-- the count where the error is caught (`Z.try`).
local depth = 0
local MAX_DEPTH = 1000

local function enter(fn, line)
  if depth >= MAX_DEPTH then
    fatal("more than 1000 calls are nested. Is `" .. (fn.name ~= "" and fn.name or "function") .. "` calling itself forever?", line)
  end
  depth = depth + 1
end

-- A call of a function of the program known where it is written, with
-- its arguments lined up already.
function Z.direct(fn, line, ...)
  enter(fn, line)
  local result = fn.fn(...)
  depth = depth - 1
  return result
end

local function call_function(fn, positional, named_names, named, line)
  local name = fn.name ~= "" and fn.name or "function"
  enter(fn, line)
  local slots = bind(name, fn.params, positional, named_names, named, line)
  for i = 1, #slots do
    if slots[i] == MISSING and not fn.defaults[i] then fatal("`" .. name .. "` needs a value for `" .. fn.params[i] .. "`", line) end
  end
  local result = fn.fn(table.unpack(slots, 1, #fn.params))
  depth = depth - 1
  return result
end

local function no_named(what, named_names, line)
  if #named_names > 0 then fatal("`" .. what .. "` does not take a named argument `" .. named_names[1] .. "`", line) end
end

local function new_error(kind, args, line)
  if #args > 1 then fatal("`" .. kind .. "` takes one message", line) end
  return setmt({kind = kind, message = #args == 0 and "" or show(args[1]), object = false}, Error)
end

-- The receiver, then the arguments.
local function with_first(first, rest)
  local out = {first}
  table.move(rest, 1, #rest, 2, out)
  return out
end

local call_builtin, call_method_of

local function call_named(callee, positional, named_names, named, line)
  local mt = type(callee) == "table" and getmt(callee) or nil
  if mt == Fn then
    if callee.builtin then
      no_named(callee.name, named_names, line)
      return call_builtin(callee.builtin, positional, line)
    end
    return call_function(callee, positional, named_names, named, line)
  end
  if mt == Bound then return call_function(callee.fn, with_first(callee.object, positional), named_names, named, line) end
  if mt == Method then
    no_named(callee.name, named_names, line)
    return call_method_of(callee.receiver, callee.name, positional, line)
  end
  if mt == Class then
    local slots = bind(callee.name, callee.fields, positional, named_names, named, line)
    for i = 1, #slots do
      if slots[i] == MISSING then
        local default = callee.defaults[i]
        if not default then fatal("`" .. callee.name .. "` needs a value for `" .. callee.fields[i] .. "`", line) end
        slots[i] = default.fn()
      end
    end
    return setmt({cls = callee, fields = slots}, Instance)
  end
  if mt == Constructor then
    local this = callee.def.cases[callee.index + 1]
    local slots = bind(this[1], this[2], positional, named_names, named, line)
    for i = 1, #slots do
      if slots[i] == MISSING then fatal("`" .. this[1] .. "` needs a value for `" .. this[2][i] .. "`", line) end
    end
    return setmt({def = callee.def, index = callee.index, fields = slots}, Case)
  end
  if mt == ErrorType then
    no_named(callee.kind, named_names, line)
    return new_error(callee.kind, positional, line)
  end
  fatal(type_name(callee) .. " cannot be called", line)
end
Z.call_named = call_named

local NO_NAMES = {}
local function call(callee, args, line) return call_named(callee, args, NO_NAMES, NO_NAMES, line) end
Z.call = call

-- `object.name(args)`
local function call_method_named(object, name, positional, named_names, named, line)
  local mt = type(object) == "table" and getmt(object) or nil
  if mt == Instance then
    local method = object.cls.methods[name]
    if method and not object.cls.at[name] then
      return call_function(method, with_first(object, positional), named_names, named, line)
    end
  end
  if mt == Module then
    no_named(name, named_names, line)
    return call_builtin(object.name .. "." .. name, positional, line)
  end
  if type(object) == "string" or mt == List or mt == Dict then
    no_named(name, named_names, line)
    return call_method_of(object, name, positional, line)
  end
  return call_named(field(object, name, line), positional, named_names, named, line)
end
Z.call_method_named = call_method_named
function Z.call_method(object, name, args, line) return call_method_named(object, name, args, NO_NAMES, NO_NAMES, line) end

-- -------------------------------------------------------------- Errors

function Z.raise(value, line)
  if is(value, Error) then error(setmt({value = value, at = line}, Raise), 0) end
  if is(value, Instance) then
    local wrapped = setmt({kind = value.cls.name, message = show(value), object = value}, Error)
    error(setmt({value = wrapped, at = line}, Raise), 0)
  end
  fatal("only an error can be raised, and this is " .. type_name(value), line)
end

-- Run the body of a `try`. True and what the body gave back, or false
-- and what it raised.
--
-- Lua's own way to catch an error, `pcall`, can be inside itself about
-- 190 times and no more, and a Zeno program may be 999 calls deep with a
-- `try` in each. So the body is not called from here. It is handed to
-- `Z.start`, which runs it as a coroutine of its own while this one
-- waits, and hands back how it ended: however many `try`s are inside
-- one another, Lua sees one coroutine running at a time.
function Z.try(body)
  local before = depth
  local ok, how, value = coroutine.yield(body)
  if not ok then depth = before end
  return ok, how, value
end

-- Raise again what a `try` caught.
function Z.reraise(e) error(e, 0) end

-- `except name` catches this error?
function Z.catches(e, name) return name == "Error" or name == e.value.kind end

-- What `except name as e` calls `e`.
function Z.caught(e, name)
  if name ~= "" and name ~= "Error" and e.value.object then return e.value.object end
  return e.value
end

function Z.no_match(v, line) fatal("no `case` matches " .. repr(v), line) end
function Z.undefined(name, line) fatal("`" .. name .. "` is not defined", line) end

-- ------------------------------------------------- Built-in functions

local function arity(name, args, min, max, line)
  if #args >= min and #args <= max then return end
  local wanted = min == max and counted(min, "argument") or (min .. " to " .. max .. " arguments")
  fatal("`" .. name .. "` takes " .. wanted .. ", but " .. were(#args) .. " given", line)
end
local function wrong_type(name, wanted, got, line)
  fatal("`" .. name .. "` needs " .. wanted .. ", but this is " .. type_name(got), line)
end
local function text(name, v, line)
  if type(v) ~= "string" then wrong_type(name, "a str", v, line) end
  return v
end
local function whole(name, v, line)
  if not is_int(v) then wrong_type(name, "a whole number", v, line) end
  return v
end
local function strip(s) return s:match("^[ \t\n\r\f]*(.-)[ \t\n\r\f]*$") end

-- What is printed goes out in large pieces, which is far faster than a
-- line at a time.
io.stdout:setvbuf("full", 65536)
local function write(s)
  -- Nobody is reading any more (`| head`): stop quietly.
  if not io.stdout:write(s) then os.exit(0) end
end
local function flush() io.stdout:flush() end

-- What was typed may be anything. Bytes that are not UTF-8 become
-- U+FFFD, one for each run of bytes that cannot begin a character.
local function lossy(s)
  if utf8.len(s) then return s end
  local out, n, i = {}, #s, 1
  while i <= n do
    local b = s:byte(i)
    if b < 0x80 then
      out[#out + 1] = s:sub(i, i)
      i = i + 1
    else
      -- How many bytes follow, and what the second one may be.
      local more, low, high = 0, 0x80, 0xbf
      if b >= 0xc2 and b <= 0xdf then more = 1
      elseif b == 0xe0 then more, low = 2, 0xa0
      elseif b == 0xed then more, high = 2, 0x9f
      elseif b >= 0xe1 and b <= 0xef then more = 2
      elseif b == 0xf0 then more, low = 3, 0x90
      elseif b == 0xf4 then more, high = 3, 0x8f
      elseif b >= 0xf1 and b <= 0xf3 then more = 3
      end
      local got = 1
      while got <= more and i + got <= n do
        local c = s:byte(i + got)
        if c < low or c > high then break end
        low, high = 0x80, 0xbf
        got = got + 1
      end
      if more > 0 and got == more + 1 then out[#out + 1] = s:sub(i, i + got - 1) else out[#out + 1] = "\u{fffd}" end
      i = i + got
    end
  end
  return concat(out)
end

local function amount(v)
  if is(v, Quantity) then return v.v end
  return number(v)
end

-- The numbers `min`, `max` and `sum` are given, and the unit they share.
local function numbers(name, args, line)
  local values = args
  if #args == 1 and is(args[1], List) then values = args[1] end
  local unit = nil
  for place = 1, #values do
    local item = values[place]
    if is(item, Quantity) then
      if unit ~= nil and unit ~= item.unit then
        fatal("`" .. name .. "` needs quantities in one unit, but these are in " .. unit .. " and in " .. item.unit .. ". Turn one into the other first, as in `x." .. unit .. "`", line)
      end
      if unit == nil and place > 1 then fatal("`" .. name .. "` needs all plain numbers or all quantities, but these are both", line) end
      unit = item.unit
    elseif number(item) == nil then
      wrong_type(name, "numbers", item, line)
    elseif unit ~= nil then
      fatal("`" .. name .. "` needs all plain numbers or all quantities, but these are both", line)
    end
  end
  return values, unit
end

local function pick(name, args, line, smaller)
  local values = numbers(name, args, line)
  if #values == 0 then raise_("ValueError", "`" .. name .. "` of nothing", line) end
  local singles, all = false, true
  for i = 1, #values do
    singles = singles or is(values[i], F32)
    all = all and (is(values[i], F32) or is_int(values[i]))
  end
  if singles and all then
    local made = {}
    for i = 1, #values do made[i] = f32(single(values[i])) end
    values = made
  end
  local best = values[1]
  for i = 1, #values do
    if is(values[i], U64) and is(best, U64) then
      if (smaller and math.ult(values[i].v, best.v)) or (not smaller and math.ult(best.v, values[i].v)) then best = values[i] end
    else
      local a, b = amount(values[i]), amount(best)
      if (smaller and a < b) or (not smaller and a > b) then best = values[i] end
    end
  end
  return best
end

-- The float a text spells, or nil.
local function parse_float(s)
  local body = s:gsub("^[+-]", "")
  local lower = body:lower()
  if lower == "inf" or lower == "infinity" then return s:sub(1, 1) == "-" and -INF or INF end
  if lower == "nan" then return 0 / 0 end
  local digits = body:match("^(.-)[eE][+-]?%d+$") or body
  if not (digits:match("^%d+%.?%d*$") or digits:match("^%.%d+$")) then return nil end
  -- The size first and the sign after, so that "-0" is -0.0.
  local x = tonumber(body) + 0.0
  if s:sub(1, 1) == "-" then return -x end
  return x
end

-- Heapsort, step for step as builtins.rs and runtime.c do it, so that
-- values that are neither before the other end up where they do there.
-- Places are counted from 0.
local function heapsort(n, before, swap)
  local function sift(root, last)
    while true do
      local child = 2 * root + 1
      if child >= last then return end
      if child + 1 < last and before(child, child + 1) then child = child + 1 end
      if not before(root, child) then return end
      swap(root, child)
      root = child
    end
  end
  for root = n // 2 - 1, 0, -1 do sift(root, n) end
  for last = n - 1, 1, -1 do
    swap(0, last)
    sift(0, last)
  end
end

local function sort_list(items, line)
  local sorted = copied(items)
  heapsort(#sorted,
    function(i, j) return lt(sorted[i + 1], sorted[j + 1], line) end,
    function(i, j) sorted[i + 1], sorted[j + 1] = sorted[j + 1], sorted[i + 1] end)
  for i = 1, #sorted do items[i] = sorted[i] end
end

local function sort_by(items, fn, line)
  -- The function may change the list: it is given the items the list
  -- had when the sort began.
  local keys, given = {}, copied(items)
  for i = 1, #given do
    local k = call(fn, {given[i]}, line)
    if not mtype(k) and not is(k, F32) and not is(k, U64) and type(k) ~= "string" then
      fatal("the function given to `sort` must return a number or text to order by, but it returned " .. type_name(k), line)
    end
    keys[i] = k
  end
  local sorted = copied(items)
  if #sorted ~= #keys then raise_("ValueError", "the function given to `sort` changed how many items the list has", line) end
  local places = {}
  for i = 1, #sorted do places[i] = i end
  heapsort(#sorted,
    function(a, b)
      if lt(keys[a + 1], keys[b + 1], line) then return true end
      if lt(keys[b + 1], keys[a + 1], line) then return false end
      return places[a + 1] < places[b + 1]
    end,
    function(i, j)
      i, j = i + 1, j + 1
      sorted[i], sorted[j] = sorted[j], sorted[i]
      keys[i], keys[j] = keys[j], keys[i]
      places[i], places[j] = places[j], places[i]
    end)
  for i = #items, 1, -1 do items[i] = nil end
  for i = 1, #sorted do items[i] = sorted[i] end
end

local function slice(args, length, what, line)
  local from = whole("slice", args[1], line)
  local to = length
  if #args > 1 then to = whole("slice", args[2], line) end
  if from < 0 or to < from or to > length then
    raise_("IndexError", "slice " .. from .. " to " .. to .. " is out of range for a " .. what .. " of length " .. length, line)
  end
  return from, to
end

-- The pieces of a str between each place a separator is.
local function split(s, sep)
  local out, from = {}, 1
  while true do
    local first, last = s:find(sep, from, true)
    if not first then
      out[#out + 1] = s:sub(from)
      return out
    end
    out[#out + 1] = s:sub(from, first - 1)
    from = last + 1
  end
end

local STR, LIST, DICT = {}, {}, {}

function STR.upper(s, args, line) arity("upper", args, 0, 0, line) return s:upper() end
function STR.lower(s, args, line) arity("lower", args, 0, 0, line) return s:lower() end
function STR.strip(s, args, line) arity("strip", args, 0, 0, line) return strip(s) end
function STR.split(s, args, line)
  arity("split", args, 0, 1, line)
  if #args == 0 then
    local out = {}
    for word in s:gmatch("[^ \t\n\r\f]+") do out[#out + 1] = word end
    return list(out)
  end
  local sep = text("split", args[1], line)
  if sep == "" then raise_("ValueError", "cannot split on an empty str", line) end
  return list(split(s, sep))
end
function STR.lines(s, args, line)
  arity("lines", args, 0, 0, line)
  -- A "\r" goes with the "\n" just after it, and with nothing else.
  if s == "" then return list({}) end
  local parts = split(s, "\n")
  local last = table.remove(parts)
  for i, part in ipairs(parts) do
    if part:sub(-1) == "\r" then parts[i] = part:sub(1, -2) end
  end
  if last ~= "" then parts[#parts + 1] = last end
  return list(parts)
end
function STR.replace(s, args, line)
  arity("replace", args, 2, 2, line)
  local old, now = text("replace", args[1], line), text("replace", args[2], line)
  -- The new text between every character, and at both ends.
  if old == "" then return now .. concat(chars(s), now) .. (s == "" and "" or now) end
  return concat(split(s, old), now)
end
function STR.starts_with(s, args, line)
  arity("starts_with", args, 1, 1, line)
  local start = text("starts_with", args[1], line)
  return s:sub(1, #start) == start
end
function STR.ends_with(s, args, line)
  arity("ends_with", args, 1, 1, line)
  local ending = text("ends_with", args[1], line)
  return ending == "" or (#ending <= #s and s:sub(-#ending) == ending)
end
function STR.find(s, args, line)
  arity("find", args, 1, 1, line)
  local what = text("find", args[1], line)
  local at = s:find(what, 1, true)
  if not at then return NONE end
  return str_len(s:sub(1, at - 1))
end
function STR.slice(s, args, line)
  arity("slice", args, 1, 2, line)
  local from, to = slice(args, str_len(s), "str", line)
  return str_part(s, from, to)
end
function STR.code(s, args, line)
  arity("code", args, 0, 0, line)
  local count = str_len(s)
  if count ~= 1 then raise_("ValueError", "`code` needs a str of exactly one character, and this one has " .. count, line) end
  return utf8.codepoint(s)
end

function DICT.get(d, args, line)
  arity("get", args, 1, 2, line)
  local value = d.map[key(args[1], line)]
  if value ~= nil then return value end
  if #args > 1 then return args[2] end
  return NONE
end
function DICT.pop(d, args, line)
  arity("pop", args, 1, 1, line)
  local k = key(args[1], line)
  local value = d.map[k]
  if value == nil then raise_("KeyError", "this dict has no key " .. repr(args[1]), line) end
  d.map[k] = nil
  table.remove(d.keys, place_of(d.keys, k))
  return value
end
function DICT.keys(d, args, line) arity("keys", args, 0, 0, line) return list(copied(d.keys)) end
function DICT.values(d, args, line)
  arity("values", args, 0, 0, line)
  local out = {}
  for i, k in ipairs(d.keys) do out[i] = d.map[k] end
  return list(out)
end
function DICT.clear(d, args, line)
  arity("clear", args, 0, 0, line)
  d.keys, d.map = {}, {}
  return NONE
end

local function emptied(xs)
  for i = #xs, 1, -1 do xs[i] = nil end
end

function LIST.append(xs, args, line)
  arity("append", args, 1, 1, line)
  xs[#xs + 1] = args[1]
  return NONE
end
function LIST.pop(xs, args, line)
  arity("pop", args, 0, 1, line)
  if #args == 1 then return table.remove(xs, position(args[1], #xs, "list", line) + 1) end
  if #xs == 0 then raise_("IndexError", "cannot pop from an empty list", line) end
  return table.remove(xs)
end
function LIST.insert(xs, args, line)
  arity("insert", args, 2, 2, line)
  local at = whole("insert", args[1], line)
  if at < 0 or at > #xs then raise_("IndexError", "cannot insert at " .. at .. " in a list of length " .. #xs, line) end
  table.insert(xs, at + 1, args[2])
  return NONE
end
function LIST.reverse(xs, args, line)
  arity("reverse", args, 0, 0, line)
  local i, j = 1, #xs
  while i < j do
    xs[i], xs[j] = xs[j], xs[i]
    i, j = i + 1, j - 1
  end
  return NONE
end
function LIST.clear(xs, args, line)
  arity("clear", args, 0, 0, line)
  emptied(xs)
  return NONE
end
function LIST.sort(xs, args, line)
  if #args == 1 then
    sort_by(xs, args[1], line)
    return NONE
  end
  arity("sort", args, 0, 1, line)
  sort_list(xs, line)
  return NONE
end
function LIST.find(xs, args, line)
  arity("find", args, 1, 1, line)
  for at = 1, #xs do
    if eq(xs[at], args[1]) then return at - 1 end
  end
  return NONE
end
function LIST.slice(xs, args, line)
  arity("slice", args, 1, 2, line)
  local from, to = slice(args, #xs, "list", line)
  return list(table.move(xs, from + 1, to, 1, {}))
end
function LIST.join(xs, args, line)
  arity("join", args, 1, 1, line)
  local sep = text("join", args[1], line)
  for i = 1, #xs do
    if type(xs[i]) ~= "string" then
      fatal("`join` puts text together, and this list holds " .. type_name(xs[i]) .. ". Make each item text first, with `str( )`", line)
    end
  end
  return concat(xs, sep)
end
function LIST.map(xs, args, line)
  arity("map", args, 1, 1, line)
  local items, out = copied(xs), {}
  for i = 1, #items do out[i] = call(args[1], {items[i]}, line) end
  return list(out)
end
function LIST.filter(xs, args, line)
  arity("filter", args, 1, 1, line)
  local items, kept = copied(xs), {}
  for i = 1, #items do
    local keep = call(args[1], {items[i]}, line)
    if keep == true then
      kept[#kept + 1] = items[i]
    elseif keep ~= false then
      fatal("the function given to `filter` must return true or false, but it returned " .. type_name(keep), line)
    end
  end
  return list(kept)
end
function LIST.reduce(xs, args, line)
  arity("reduce", args, 2, 2, line)
  local items, total = copied(xs), args[2]
  for i = 1, #items do total = call(args[1], {total, items[i]}, line) end
  return total
end

function call_method_of(receiver, name, args, line)
  local methods, what
  if type(receiver) == "string" then
    methods, what = STR, "a str"
  elseif is(receiver, Dict) then
    methods, what = DICT, "a dict"
  elseif is(receiver, List) then
    methods, what = LIST, "a list"
  else
    fatal(type_name(receiver) .. " has no method `" .. name .. "`", line)
  end
  local method = methods[name]
  if not method then fatal(what .. " has no method `" .. name .. "`", line) end
  return method(receiver, args, line)
end

-- The format after the colon of `{value:format}`.
function Z.format(v, spec, line)
  if spec == "" then return show(v) end
  local function bad() fatal("`" .. spec .. "` is not a format Zeno understands", line) end
  -- An alignment, a zero, a width, a point and a precision, a kind:
  -- each may be left out.
  local at = 1
  local align = spec:match("^[<>^]")
  if align then at = 2 end
  local zero = spec:sub(at, at) == "0"
  if zero then at = at + 1 end
  local width = spec:match("^%d*", at)
  at = at + #width
  width = width == "" and 0 or math.tointeger(tonumber(width)) or 0
  local precision = nil
  if spec:sub(at, at) == "." then
    local digits = spec:match("^%d*", at + 1)
    at = at + 1 + #digits
    precision = digits == "" and 0 or math.tointeger(tonumber(digits)) or 0
  end
  local kind = spec:sub(at)
  if kind == "" then
    kind = nil
  elseif str_len(kind) ~= 1 or not utf8.len(kind) or kind:match("^%d$") or kind == "\n" then
    bad()
  end
  local body
  local whole_kind = kind == "d" or kind == "x" or kind == "X" or kind == "b"
  if (kind == "f" or (kind == nil and precision ~= nil)) and is(v, Quantity) then
    body = fixed(v.v, precision or 6) .. " " .. v.unit
  elseif kind == "f" or (kind == nil and precision ~= nil and is_float(v)) then
    local x = number(v)
    if x == nil then wrong_type(spec, "a number", v, line) end
    body = fixed(x, precision or 6)
  elseif (kind == nil and precision == nil) or (kind == "d" and precision == nil and (is_int(v) or is(v, U64))) then
    body = show(v)
  elseif whole_kind and kind ~= "d" and precision == nil and is(v, U64) then
    -- Lua writes the bits of an integer with no sign.
    if kind == "b" then
      local bits, size = {}, v.v
      repeat
        bits[#bits + 1] = (size & 1 == 1) and "1" or "0"
        size = size >> 1
      until size == 0
      body = concat(bits):reverse()
    else
      body = fmt(kind == "X" and "%X" or "%x", v.v)
    end
  elseif whole_kind and kind ~= "d" and precision == nil and is_int(v) then
    -- The digits of the size of the number, with `-` before them. The
    -- smallest int has no size that fits, so its digits are read from
    -- its bits, which Lua does for every int.
    local size = v < 0 and -v or v
    local digits
    if kind == "b" then
      local bits = {}
      repeat
        bits[#bits + 1] = (size & 1 == 1) and "1" or "0"
        size = size >> 1
      until size == 0
      digits = concat(bits):reverse()
    else
      digits = fmt(kind == "X" and "%X" or "%x", size)
    end
    body = (v < 0 and "-" or "") .. digits
  elseif whole_kind and precision == nil then
    wrong_type(spec, "an int", v, line)
  else
    bad()
  end
  local padding = math.max(0, width - str_len(body))
  local is_number = mtype(v) ~= nil or is(v, Quantity) or is(v, U64)
  if zero and is_number then
    local negative = body:sub(1, 1) == "-"
    return (negative and "-" or "") .. rep("0", padding) .. (negative and body:sub(2) or body)
  end
  local side = align or (is_number and ">" or "<")
  if side == "<" then return body .. rep(" ", padding) end
  if side == ">" then return rep(" ", padding) .. body end
  local left = padding // 2
  return rep(" ", left) .. body .. rep(" ", padding - left)
end

-- ---------------------------------------------------------------- JSON

-- The reader walks the bytes of the text: every mark of JSON is one
-- byte, and the bytes of a longer character pass through as they are.
-- It gives back the value and the place after it.
local json_value

local function json_spaces(s, at)
  return s:find("[^ \t\n\r]", at) or #s + 1
end
local function json_hex(s, at)
  local digits = s:sub(at, at + 3)
  if not digits:match("^%x%x%x%x$") then error(NOT_JSON, 0) end
  return tonumber(digits, 16), at + 4
end
local JSON_ESCAPES = {n = "\n", t = "\t", r = "\r", b = "\b", f = "\f"}
local function json_text(s, at)
  if s:sub(at, at) ~= '"' then error(NOT_JSON, 0) end
  at = at + 1
  local out = {}
  while true do
    -- All that is plain, in one piece.
    local stop = s:find('["\\]', at)
    if not stop then error(NOT_JSON, 0) end
    out[#out + 1] = s:sub(at, stop - 1)
    at = stop + 1
    if s:sub(stop, stop) == '"' then return concat(out), at end
    if at > #s then error(NOT_JSON, 0) end
    local e = s:sub(at, at)
    at = at + 1
    if e == "u" then
      local code
      code, at = json_hex(s, at)
      if code >= 0xd800 and code < 0xdc00 and s:sub(at, at + 1) == "\\u" then
        local low
        low, at = json_hex(s, at + 2)
        if low < 0xdc00 or low > 0xdfff then error(NOT_JSON, 0) end
        code = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00)
      end
      if code >= 0xd800 and code <= 0xdfff then error(NOT_JSON, 0) end
      out[#out + 1] = utf8.char(code)
    else
      out[#out + 1] = JSON_ESCAPES[e] or e
    end
  end
end

function json_value(s, at)
  at = json_spaces(s, at)
  if at > #s then error(NOT_JSON, 0) end
  local c = s:sub(at, at)
  if c == "n" then
    if s:sub(at, at + 3) == "null" then return NONE, at + 4 end
    error(NOT_JSON, 0)
  end
  if c == "t" then
    if s:sub(at, at + 3) == "true" then return true, at + 4 end
    error(NOT_JSON, 0)
  end
  if c == "f" then
    if s:sub(at, at + 4) == "false" then return false, at + 5 end
    error(NOT_JSON, 0)
  end
  if c == '"' then return json_text(s, at) end
  if c == "[" then
    at = at + 1
    local items = {}
    while true do
      at = json_spaces(s, at)
      if s:sub(at, at) == "]" then return list(items), at + 1 end
      if #items > 0 then
        if s:sub(at, at) ~= "," then error(NOT_JSON, 0) end
        at = at + 1
      end
      items[#items + 1], at = json_value(s, at)
    end
  end
  if c == "{" then
    at = at + 1
    local d, count = new_dict(), 0
    while true do
      at = json_spaces(s, at)
      if s:sub(at, at) == "}" then return d, at + 1 end
      if count > 0 then
        if s:sub(at, at) ~= "," then error(NOT_JSON, 0) end
        at = at + 1
      end
      at = json_spaces(s, at)
      local name, v
      name, at = json_text(s, at)
      at = json_spaces(s, at)
      if s:sub(at, at) ~= ":" then error(NOT_JSON, 0) end
      v, at = json_value(s, at + 1)
      count = count + 1
      -- A member that is null is left out: a dict holds no none.
      if v ~= NONE then dict_set(d, name, v) end
    end
  end
  local digits = s:match("^[0-9+%-.eE]*", at)
  local x = parse_float(digits)
  if x == nil then error(NOT_JSON, 0) end
  at = at + #digits
  if is_finite(x) and floor(x) == x and math.abs(x) < 9.0e15 then return math.tointeger(x), at end
  return x, at
end

-- The value a JSON text holds, or MISSING if the text is not JSON.
local function json_parse(s)
  local ok, v, at = pcall(json_value, s, 1)
  if not ok then
    if v ~= NOT_JSON then error(v, 0) end
    return MISSING
  end
  if json_spaces(s, at) <= #s then return MISSING end
  return v
end

local JSON_OUT = {['"'] = '\\"', ["\\"] = "\\\\", ["\n"] = "\\n", ["\r"] = "\\r", ["\t"] = "\\t"}
local function json_quoted(s)
  s = s:gsub('[%z\1-\31"\\]', function(c) return JSON_OUT[c] or fmt("\\u%04x", c:byte()) end)
  return '"' .. s .. '"'
end

local function json_write(v, line)
  if v == NONE then return "null" end
  if v == true then return "true" end
  if v == false then return "false" end
  local kind = mtype(v)
  if kind == "integer" then return fmt("%d", v) end
  if kind == "float" then
    if not is_finite(v) then raise_("ValueError", "JSON has no way to write " .. show_float(v), line) end
    return show_float(v)
  end
  if type(v) == "string" then return json_quoted(v) end
  local mt = getmt(v)
  local parts = {}
  if mt == List then
    for i = 1, #v do parts[i] = json_write(v[i], line) end
    return "[" .. concat(parts, ",") .. "]"
  end
  if mt == Dict then
    for i, k in ipairs(v.keys) do parts[i] = json_quoted(show(k)) .. ":" .. json_write(v.map[k], line) end
    return "{" .. concat(parts, ",") .. "}"
  end
  if mt == Instance then
    for i, name in ipairs(v.cls.fields) do parts[i] = json_quoted(name) .. ":" .. json_write(v.fields[i], line) end
    return "{" .. concat(parts, ",") .. "}"
  end
  if mt == Quantity then
    raise_("ValueError", "JSON has no units, and this is a quantity in " .. v.unit .. '. Write its number, and put the unit in the name: `["' .. v.unit .. '": ...]`', line)
  end
  raise_("ValueError", "JSON has objects, lists, text, numbers, bools and none, and this is " .. type_name(v), line)
end

-- --------------------------------------------------------------- Files

-- What is at a path: "file", "nothing", or "other" (a folder, or
-- something that cannot be read).
local function at_path(path)
  if path == "" then return "nothing" end
  if path:find("\0", 1, true) then return "other" end
  local file, _, code = io.open(path, "rb")
  if not file then
    -- 2 is "there is no such file": every other reason is something
    -- that is there and cannot be read, or a path that cannot be followed.
    return code == 2 and "nothing" or "other"
  end
  local byte, problem = file:read(1)
  file:close()
  if byte == nil and problem ~= nil then return "other" end
  return "file"
end
local function paths(name, args, count, line)
  arity(name, args, count, count, line)
  for i = 1, #args do text(name, args[i], line) end
  return args[1], args[2]
end
local function file_error(message, line) raise_("FileError", message, line) end

local function put(path, s, mode, line)
  local file = not path:find("\0", 1, true) and path ~= "" and io.open(path, mode)
  if not file then file_error("cannot write `" .. path .. "`", line) end
  local ok = file:write(s)
  if not file:close() or not ok then file_error("cannot write `" .. path .. "`", line) end
  return NONE
end

-- ------------------------------------------------- The built-in table
--
-- Each is `b_` and its name, with `_` for the `.` of a module:
-- Z.b_print, Z.b_math_sqrt.

function Z.b_print(args, line)
  local parts = {}
  for i = 1, #args do parts[i] = show(args[i]) end
  write(concat(parts, " ") .. "\n")
  return NONE
end
function Z.b_input(args, line)
  arity("input", args, 0, 1, line)
  write(#args > 0 and text("input", args[1], line) or "")
  flush()
  local typed = io.stdin:read("L")
  if typed == nil then return NONE end
  local s = lossy(typed)
  -- The line end is not part of the line: "\n", or "\r\n". A "\r" at
  -- the end of input that no "\n" follows stays.
  if s:sub(-1) == "\n" then
    s = s:sub(1, -2)
    if s:sub(-1) == "\r" then s = s:sub(1, -2) end
  end
  return (s:gsub("\0", ""))
end
function Z.b_len(args, line)
  arity("len", args, 1, 1, line)
  local v = args[1]
  if type(v) == "string" then return str_len(v) end
  if is(v, List) then return #v end
  if is(v, Dict) then return #v.keys end
  wrong_type("len", "a str, a list or a dict", v, line)
end
function Z.b_range(args, line)
  arity("range", args, 1, 3, line)
  for i = 1, #args do
    if not is_int(args[i]) then wrong_type("range", "ints", args[i], line) end
  end
  local start, stop, step = 0, args[1], 1
  if #args >= 2 then start, stop = args[1], args[2] end
  if #args == 3 then step = args[3] end
  if step == 0 then raise_("ValueError", "the step of a range cannot be 0", line) end
  local out, n, i = {}, 0, start
  while (step > 0 and i < stop) or (step < 0 and i > stop) do
    n = n + 1
    out[n] = i
    -- A step past the last int there is ends the range.
    if (step > 0 and i > MAXINT - step) or (step < 0 and i < MININT - step) then break end
    i = i + step
  end
  return list(out)
end

-- A float that is a whole number, as an int: the nearest int there is
-- for one too big, and 0 for a NaN.
local function to_int(x)
  if x ~= x then return 0 end
  if x >= 9223372036854775808.0 then return MAXINT end
  if x <= -9223372036854775808.0 then return MININT end
  return math.tointeger(x)
end

function Z.b_int(args, line)
  arity("int", args, 1, 1, line)
  local v = args[1]
  if is(v, U64) then
    if v.v < 0 then raise_("OverflowError", u64_text(v.v) .. " does not fit in an int (-9223372036854775808 to 9223372036854775807)", line) end
    return v.v
  end
  if is(v, F32) then v = v.v end
  local kind = mtype(v)
  if kind == "integer" then return v end
  if kind == "float" then
    if not is_finite(v) then raise_("ValueError", "cannot turn " .. show_float(v) .. " into an int", line) end
    return to_int(v < 0 and ceil(v) or floor(v))
  end
  if type(v) == "string" then
    local s = strip(v):gsub("_", "")
    local sign, digits = s:match("^([+-]?)0*(%d+)$")
    if digits then
      -- Nineteen digits may or may not fit: Lua says, by giving a float
      -- for a number too big to be an int.
      local n = #digits <= 19 and tonumber(digits) or nil
      if mtype(n) == "integer" then return sign == "-" and -n or n end
      if sign == "-" and digits == "9223372036854775808" then return MININT end
    end
    raise_("ValueError", "cannot turn " .. quoted(v) .. " into an int", line)
  end
  wrong_type("int", "a number or a str", v, line)
end
function Z.b_float(args, line)
  arity("float", args, 1, 1, line)
  local v = args[1]
  local kind = mtype(v)
  if kind == "integer" then return v + 0.0 end
  if kind == "float" then return v end
  if is(v, F32) then return v.v end
  if is(v, U64) then return number(v) end
  if type(v) == "string" then
    local x = parse_float(strip(v))
    if x == nil then raise_("ValueError", "cannot turn " .. quoted(v) .. " into a float", line) end
    return x
  end
  wrong_type("float", "a number or a str", v, line)
end
function Z.b_f32(args, line)
  arity("f32", args, 1, 1, line)
  local v = args[1]
  if is(v, F32) then return v end
  if is_int(v) then return f32(single(v)) end
  if is_float(v) then
    local r = fround(v)
    if not is_finite(r) and is_finite(v) then
      raise_("OverflowError", show_float(v) .. " does not fit in an f32, whose largest is about 3.4e38", line)
    end
    return f32(r)
  end
  wrong_type("f32", "a number", v, line)
end
function Z.b_u64(args, line)
  arity("u64", args, 1, 1, line)
  local v = args[1]
  if is(v, U64) then return v end
  if is_int(v) then
    if v < 0 then raise_("OverflowError", v .. " does not fit in a u64 (0 to 18446744073709551615)", line) end
    return u64(v)
  end
  wrong_type("u64", "a whole number", v, line)
end
-- `u64.wrap(n)`: the 64 bits of an int read with no sign (42.6).
function Z.b_u64__of(args, line)
  arity("u64.wrap", args, 1, 1, line)
  local v = args[1]
  if is(v, U64) then return v end
  if is_int(v) then return u64(v) end
  wrong_type("u64.wrap", "a whole number", v, line)
end
-- `u64.wrap(a + b)`: the answer cut to 64 bits (42.6).
local function u64_wrapping(op, args, line)
  arity("u64.wrap", args, 2, 2, line)
  local a, b = args[1], args[2]
  if op == "<<" and unsigned(a) ~= nil and (is_int(b) or is(b, U64)) then return u64(unsigned(a) << u64_shift_count(b, line)) end
  local x, y = unsigned(a), unsigned(b)
  if x == nil or y == nil then wrong_type("u64.wrap", "u64s", x == nil and a or b, line) end
  if op == "+" then return u64(x + y) end
  if op == "-" then return u64(x - y) end
  return u64(x * y)
end
function Z.b_u64__add(args, line) return u64_wrapping("+", args, line) end
function Z.b_u64__sub(args, line) return u64_wrapping("-", args, line) end
function Z.b_u64__mul(args, line) return u64_wrapping("*", args, line) end
function Z.b_u64__shl(args, line) return u64_wrapping("<<", args, line) end
function Z.b_str(args, line)
  arity("str", args, 1, 1, line)
  return show(args[1])
end
function Z.b_abs(args, line)
  arity("abs", args, 1, 1, line)
  local v = args[1]
  local kind = mtype(v)
  if kind == "integer" then return v < 0 and Z.neg(v, line) or v end
  if kind == "float" then return math.abs(v) end
  if is(v, F32) then return f32(math.abs(v.v)) end
  if is(v, U64) then return v end
  if is(v, Quantity) then return quantity(math.abs(v.v), v.unit) end
  wrong_type("abs", "a number", v, line)
end
function Z.b_min(args, line) return pick("min", args, line, true) end
function Z.b_max(args, line) return pick("max", args, line, false) end
function Z.b_sum(args, line)
  local values, unit = numbers("sum", args, line)
  if unit ~= nil then
    fatal("`sum` works on plain numbers, and these have a unit. Add quantities with `+`, which keeps the unit: start from `var total = 0 V` and add each in a loop", line)
  end
  local singles, all = false, true
  for i = 1, #values do
    singles = singles or is(values[i], F32)
    all = all and (is(values[i], F32) or is_int(values[i]))
  end
  if singles and all then
    local single32 = 0.0
    for i = 1, #values do single32 = fround(single32 + single(values[i])) end
    return f32(single32)
  end
  local u64s = false
  for i = 1, #values do u64s = u64s or is(values[i], U64) end
  if u64s then
    local total = 0
    for i = 1, #values do
      local r = total + (unsigned(values[i]) or 0)
      if math.ult(r, total) then raise_("OverflowError", U64_TOO_BIG, line) end
      total = r
    end
    return u64(total)
  end
  local total, all_ints, float_total = 0, true, 0.0
  for i = 1, #values do
    local item = values[i]
    float_total = float_total + number(item)
    if all_ints and is_int(item) then total = Z.add(total, item, line) else all_ints = false end
  end
  if all_ints then return total end
  return float_total
end
function Z.b_char(args, line)
  arity("char", args, 1, 1, line)
  local code = whole("char", args[1], line)
  if code <= 0 or code > 0x10ffff or (code >= 0xd800 and code <= 0xdfff) then raise_("ValueError", code .. " is not the code of a character", line) end
  return utf8.char(code)
end
function Z.b_allocate(args, line) fatal("manual memory is only for a program compiled to machine code", line) end
function Z.b_free(args, line) fatal("manual memory is only for a program compiled to machine code", line) end

local function one_number(name, args, line)
  arity(name, args, 1, 1, line)
  local x = number(args[1])
  if x == nil then wrong_type(name, "a number", args[1], line) end
  return x
end
local function angle(name, args, line)
  arity(name, args, 1, 1, line)
  local v = args[1]
  if is(v, Quantity) then
    local scale = convert(v.unit, "rad")
    if scale == nil then fatal("`" .. name .. "` needs a number or an angle, such as `30 deg`, not a quantity in " .. v.unit, line) end
    return apply(scale, v.v)
  end
  local x = number(v)
  if x == nil then wrong_type(name, "a number or an angle, such as `30 deg`", v, line) end
  return x
end
local function ratio(name, args, line)
  local x = one_number(name, args, line)
  if x < -1 or x > 1 then raise_("ValueError", "`" .. name .. "` needs a number from -1 to 1, and this is " .. show_float(x), line) end
  return x
end
local function above_zero(name, args, line)
  local x = one_number(name, args, line)
  if x <= 0 then raise_("ValueError", "cannot take the logarithm of " .. show_float(x), line) end
  return x
end

function Z.b_math_sqrt(args, line)
  local x = one_number("sqrt", args, line)
  if x < 0 then raise_("ValueError", "cannot take the square root of " .. show_float(x), line) end
  return math.sqrt(x)
end
function Z.b_math_floor(args, line) return to_int(floor(one_number("floor", args, line))) end
function Z.b_math_ceil(args, line) return to_int(ceil(one_number("ceil", args, line))) end
function Z.b_math_round(args, line)
  -- A half goes away from zero. Worked out from the float itself:
  -- adding 0.5 first would round 0.49999999999999994 up.
  local x = one_number("round", args, line)
  local size = math.abs(x)
  local nearest = floor(size)
  if size - nearest >= 0.5 then nearest = nearest + 1 end
  return to_int(x < 0 and -nearest or nearest)
end
function Z.b_math_sin(args, line) return math.sin(angle("sin", args, line)) end
function Z.b_math_cos(args, line) return math.cos(angle("cos", args, line)) end
function Z.b_math_tan(args, line) return math.tan(angle("tan", args, line)) end
function Z.b_math_asin(args, line) return quantity(math.asin(ratio("asin", args, line)), "rad") end
function Z.b_math_acos(args, line) return quantity(math.acos(ratio("acos", args, line)), "rad") end
function Z.b_math_atan(args, line) return quantity(math.atan(one_number("atan", args, line)), "rad") end
function Z.b_math_atan2(args, line)
  arity("atan2", args, 2, 2, line)
  local y, x = number(args[1]), number(args[2])
  if y == nil then wrong_type("atan2", "two numbers", args[1], line) end
  if x == nil then wrong_type("atan2", "two numbers", args[2], line) end
  return quantity(math.atan(y, x), "rad")
end
function Z.b_math_exp(args, line) return math.exp(one_number("exp", args, line)) end
function Z.b_math_log(args, line) return math.log(above_zero("log", args, line)) end
function Z.b_math_log2(args, line) return math.log(above_zero("log2", args, line), 2) end
function Z.b_math_log10(args, line) return math.log(above_zero("log10", args, line), 10) end

-- Lua itself has a clock of whole seconds, the time the processor has
-- spent, and no way to wait. Where LuaSocket is installed, its clock
-- and its `sleep` are used. Where it is not, `now` counts processor
-- time and `sleep` waits by counting it.
local has_socket, socket = pcall(require, "socket")
local clock = has_socket and socket.gettime or os.clock
local started = clock()

function Z.b_time_sleep(args, line)
  arity("sleep", args, 1, 1, line)
  local v = args[1]
  if not is(v, Quantity) then wrong_type("sleep", "a time with a unit, such as `500 ms`", v, line) end
  local scale = convert(v.unit, "s")
  if scale == nil then fatal("`sleep` needs a time, such as `500 ms`, not a quantity in " .. v.unit, line) end
  local seconds = apply(scale, v.v)
  if seconds > 0 then
    flush()
    seconds = math.min(seconds, 1e9)
    if has_socket then
      socket.sleep(seconds)
    else
      local until_ = clock() + seconds
      while clock() < until_ do end
    end
  end
  return NONE
end
function Z.b_time_now(args, line)
  arity("now", args, 0, 0, line)
  return quantity((clock() - started) * 1000, "ms")
end

function Z.b_files_read(args, line)
  local path = paths("read", args, 1, line)
  local where = at_path(path)
  if where == "nothing" then file_error("there is no file `" .. path .. "`", line) end
  if where == "other" then file_error("cannot read `" .. path .. "`", line) end
  local file = io.open(path, "rb")
  local s = file and file:read("a")
  if file then file:close() end
  if not s then file_error("cannot read `" .. path .. "`", line) end
  if not utf8.len(s) or s:find("\0", 1, true) then file_error("`" .. path .. "` does not hold text", line) end
  return s
end
function Z.b_files_write(args, line)
  local path, s = paths("write", args, 2, line)
  return put(path, s, "wb", line)
end
function Z.b_files_read_bytes(args, line)
  local path = paths("read_bytes", args, 1, line)
  local where = at_path(path)
  if where == "nothing" then file_error("there is no file `" .. path .. "`", line) end
  if where == "other" then file_error("cannot read `" .. path .. "`", line) end
  local file = io.open(path, "rb")
  local s = file and file:read("a")
  if file then file:close() end
  if not s then file_error("cannot read `" .. path .. "`", line) end
  local bytes = {}
  for i = 1, #s do bytes[i] = s:byte(i) end
  return list(bytes)
end
function Z.b_files_write_bytes(args, line)
  arity("write_bytes", args, 2, 2, line)
  local path = text("write_bytes", args[1], line)
  local items = args[2]
  if not is(items, List) then wrong_type("write_bytes", "a list[u8]", items, line) end
  local pieces = {}
  for i = 1, #items do
    local n = items[i]
    if mtype(n) ~= "integer" then wrong_type("write_bytes", "a list[u8]", n, line) end
    if n < 0 or n > 255 then raise_("ValueError", tostring(n) .. " is not a byte: a byte is a number from 0 to 255", line) end
    pieces[i] = string.char(n)
  end
  return put(path, table.concat(pieces), "wb", line)
end
function Z.b_files_append(args, line)
  local path, s = paths("append", args, 2, line)
  return put(path, s, "ab", line)
end
function Z.b_files_exists(args, line) return at_path((paths("exists", args, 1, line))) == "file" end
function Z.b_files_remove(args, line)
  local path = paths("remove", args, 1, line)
  local where = at_path(path)
  if where == "nothing" then file_error("there is no file `" .. path .. "`", line) end
  if where == "file" and os.remove(path) then return NONE end
  file_error("cannot remove `" .. path .. "`", line)
end

-- Lua itself cannot see into a folder. Where LuaFileSystem is installed,
-- it is used. Where it is not, on a system with a POSIX shell, `find` and
-- `mkdir -p` are.
local has_lfs, lfs = pcall(require, "lfs")
local posix = package.config:sub(1, 1) == "/"
local function shell_quoted(path)
  if path:sub(1, 1) == "-" then path = "./" .. path end
  return "'" .. path:gsub("'", "'\\''") .. "'"
end
local function is_folder(path)
  if path == "" or path:find("\0", 1, true) then return false end
  if has_lfs then return lfs.attributes(path, "mode") == "directory" end
  if not posix then return false end
  local file = io.open(path .. "/.", "rb")
  if file then file:close() end
  return file ~= nil
end
-- Sorted by their bytes, which for UTF-8 is the order of the numbers of
-- their characters, whatever the locale says.
local function by_bytes(a, b)
  for i = 1, math.min(#a, #b) do
    local x, y = a:byte(i), b:byte(i)
    if x ~= y then return x < y end
  end
  return #a < #b
end

-- `files.list(folder)`: the names in it, sorted.
function Z.b_files_list(args, line)
  local folder = paths("list", args, 1, line)
  if not is_folder(folder) then
    if at_path(folder) == "nothing" then file_error("there is no folder `" .. folder .. "`", line) end
    file_error("cannot list the folder `" .. folder .. "`", line)
  end
  local names = {}
  local ok = false
  if has_lfs then
    ok = pcall(function()
      for name in lfs.dir(folder) do
        if name ~= "." and name ~= ".." then names[#names + 1] = name end
      end
    end)
  elseif posix then
    local out = io.popen("find " .. shell_quoted(folder) .. " -mindepth 1 -maxdepth 1 -print0 2>/dev/null", "r")
    if out then
      local found = out:read("a") or ""
      ok = out:close()
      for entry in found:gmatch("([^%z]+)%z") do
        names[#names + 1] = entry:match("([^/]+)$")
      end
    end
  end
  if not ok then file_error("cannot list the folder `" .. folder .. "`", line) end
  for i = 1, #names do
    if not utf8.len(names[i]) then file_error("the folder `" .. folder .. "` holds a name that is not text", line) end
  end
  table.sort(names, by_bytes)
  return list(names)
end

-- `files.make_folder(path)`: it worked if there is a folder there at the end.
function Z.b_files_make_folder(args, line)
  local path = paths("make_folder", args, 1, line)
  if path ~= "" and not path:find("\0", 1, true) and not is_folder(path) then
    if has_lfs then
      for i = 1, #path do
        if i == #path or path:sub(i + 1, i + 1) == "/" then lfs.mkdir(path:sub(1, i)) end
      end
    elseif posix then
      os.execute("mkdir -p -- " .. shell_quoted(path) .. " 2>/dev/null")
    end
  end
  if not is_folder(path) then file_error("cannot make the folder `" .. path .. "`", line) end
  return NONE
end

-- What a `files.Reader` and a `files.Writer` are made of (decision
-- 59.6). A file's number is its place in `open_files`; a closed file
-- leaves its place `false`. Files are read as bytes. Its own `do` block,
-- so that its local names do not count toward the 200 of this file.
do
local open_files = {}
local function handle(name, args, line)
  local number = args[1]
  if mtype(number) ~= "integer" then wrong_type(name, "an int", number, line) end
  local open = number >= 1 and number <= #open_files and open_files[number] or nil
  if not open then fatal("there is no open file numbered " .. tostring(number), line) end
  return open
end

function Z.b_files_handle_open(args, line)
  arity("handle_open", args, 2, 2, line)
  local path = text("handle_open", args[1], line)
  local how = mtype(args[2]) == "integer" and args[2] or 0
  local file
  if how == 0 then
    local where = at_path(path)
    if where == "nothing" then file_error("there is no file `" .. path .. "`", line) end
    if where == "other" then file_error("cannot read `" .. path .. "`", line) end
    file = io.open(path, "rb")
    if not file then file_error("cannot read `" .. path .. "`", line) end
  else
    file = not path:find("\0", 1, true) and path ~= "" and io.open(path, how == 2 and "ab" or "wb")
    if not file then file_error("cannot write `" .. path .. "`", line) end
    -- Nothing is held back, so that a full disk is an error at the
    -- write that met it.
    file:setvbuf("no")
  end
  local open = {file = file, reading = how == 0}
  for place = 1, #open_files do
    if not open_files[place] then
      open_files[place] = open
      return place
    end
  end
  open_files[#open_files + 1] = open
  return #open_files
end

-- The next line without its line end (a `\r` before the `\n` goes with
-- it), or none at the end.
function Z.b_files_handle_line(args, line)
  arity("handle_line", args, 2, 2, line)
  local path = text("handle_line", args[2], line)
  local open = handle("handle_line", args, line)
  if not open.reading then file_error("cannot read `" .. path .. "`", line) end
  local ok, s = pcall(open.file.read, open.file, "L")
  if not ok then file_error("cannot read `" .. path .. "`", line) end
  if s == nil then return NONE end
  if s:sub(-1) == "\n" then
    s = s:sub(1, -2)
    if s:sub(-1) == "\r" then s = s:sub(1, -2) end
  end
  if not utf8.len(s) or s:find("\0", 1, true) then file_error("`" .. path .. "` does not hold text", line) end
  return s
end

-- Up to `count` bytes, fewer at the end, none after it.
function Z.b_files_handle_bytes(args, line)
  arity("handle_bytes", args, 3, 3, line)
  local path = text("handle_bytes", args[2], line)
  local count = mtype(args[3]) == "integer" and math.max(args[3], 0) or 0
  local open = handle("handle_bytes", args, line)
  if not open.reading then file_error("cannot read `" .. path .. "`", line) end
  local s = ""
  if count > 0 then
    local ok, got = pcall(open.file.read, open.file, count)
    if not ok then file_error("cannot read `" .. path .. "`", line) end
    s = got or ""
  end
  local bytes = {}
  for i = 1, #s do bytes[i] = s:byte(i) end
  return list(bytes)
end

local function handle_put(name, args, data, path, line)
  local open = handle(name, args, line)
  if open.reading then file_error("cannot write `" .. path .. "`", line) end
  local ok, done = pcall(open.file.write, open.file, data)
  if not ok or not done or not open.file:flush() then file_error("cannot write `" .. path .. "`", line) end
  return NONE
end

function Z.b_files_handle_write(args, line)
  arity("handle_write", args, 3, 3, line)
  local path = text("handle_write", args[2], line)
  local s = text("handle_write", args[3], line)
  return handle_put("handle_write", args, s, path, line)
end

function Z.b_files_handle_write_bytes(args, line)
  arity("handle_write_bytes", args, 3, 3, line)
  local path = text("handle_write_bytes", args[2], line)
  local items = args[3]
  if not is(items, List) then wrong_type("handle_write_bytes", "a list[u8]", items, line) end
  local pieces = {}
  for i = 1, #items do
    local n = items[i]
    if mtype(n) ~= "integer" then wrong_type("handle_write_bytes", "a list[u8]", n, line) end
    if n < 0 or n > 255 then raise_("ValueError", tostring(n) .. " is not a byte: a byte is a number from 0 to 255", line) end
    pieces[i] = string.char(n)
  end
  return handle_put("handle_write_bytes", args, table.concat(pieces), path, line)
end

-- The file is closed, and its number free.
function Z.b_files_handle_close(args, line)
  arity("handle_close", args, 1, 1, line)
  local open = handle("handle_close", args, line)
  open_files[args[1]] = false
  pcall(open.file.close, open.file)
  return NONE
end
end

-- `program.exit(code)`: what was printed goes out, and the program ends
-- with `code`.
function Z.b_program_exit(args, line)
  arity("exit", args, 1, 1, line)
  local code = whole("exit", args[1], line)
  if code < 0 or code > 255 then raise_("ValueError", "a program ends with a number from 0 to 255, not " .. code, line) end
  flush()
  io.stderr:flush()
  os.exit(code)
end
-- `program.environment(name)`: the text, or none. A name with `=` or a
-- zero byte in it is in no environment (and C would read only the part
-- before the zero).
function Z.b_program_environment(args, line)
  arity("environment", args, 1, 1, line)
  local name = text("environment", args[1], line)
  if name == "" or name:find("[=%z]") then return NONE end
  local value = os.getenv(name)
  if value == nil then return NONE end
  return lossy(value)
end
-- `program.warn(text)`: one line on the error stream, after what was printed.
function Z.b_program_warn(args, line)
  arity("warn", args, 1, 1, line)
  local said = text("warn", args[1], line)
  flush()
  io.stderr:write(said, "\n")
  return NONE
end
function Z.b_program_arguments(args, line)
  arity("arguments", args, 0, 0, line)
  local words = {}
  for i, word in ipairs(ARGV) do words[i] = lossy(word) end
  return list(words)
end

function Z.b_json_parse(args, line)
  arity("parse", args, 1, 1, line)
  local s = text("parse", args[1], line)
  local v = json_parse(s)
  if v == MISSING then
    local rest = s:match("^[ \t\n\r]*(.*)$")
    local shown = str_len(rest) > 30 and str_part(rest, 0, 30) or rest
    raise_("ValueError", "this text is not JSON: `" .. shown .. (#shown < #rest and "..." or "") .. "`", line)
  end
  return v
end
function Z.b_json_write(args, line)
  arity("write", args, 1, 1, line)
  return json_write(args[1], line)
end

-- The sized whole numbers: `u8(n)` checks, `u8.wrap(n)` keeps the low bits.
local function sized(name)
  local lo, hi, _, signed = table.unpack(WIDTHS[name])
  return function(args, line)
    arity(name, args, 1, 1, line)
    local n = args[1]
    if not is_int(n) then wrong_type(name, "a whole number", n, line) end
    if n < lo or n > hi then
      raise_("OverflowError", n .. " does not fit in " .. (signed and "an" or "a") .. " " .. name .. " (" .. lo .. " to " .. hi .. ")", line)
    end
    return n
  end
end
local function wrapped(name)
  local _, _, bits, signed = table.unpack(WIDTHS[name])
  return function(args, line)
    arity(name .. ".wrap", args, 1, 1, line)
    local n = args[1]
    if not is_int(n) then wrong_type(name .. ".wrap", "a whole number", n, line) end
    local low = n & ((1 << bits) - 1)
    if signed and low >= (1 << (bits - 1)) then return low - (1 << bits) end
    return low
  end
end
for name in pairs(WIDTHS) do
  Z["b_" .. name] = sized(name)
  Z["b_" .. name .. "_wrap"] = wrapped(name)
end

local ERROR_KINDS = {"Error", "ValueError", "IndexError", "KeyError", "ZeroDivisionError", "OverflowError", "FileError"}
local IS_ERROR_KIND = {}
for _, kind in ipairs(ERROR_KINDS) do
  IS_ERROR_KIND[kind] = true
  Z["b_" .. kind] = function(args, line) return new_error(kind, args, line) end
end

-- The function of the table that a name stands for, or nil.
local function tabled(name)
  if not (name:match("^%w+$") or name:match("^%w+%.%w+$")) then return nil end
  return Z["b_" .. name:gsub("%.", "_")]
end

function call_builtin(name, args, line)
  local found = tabled(name)
  if not found then fatal("there is no `" .. name .. "` in a program built for Lua", line) end
  return found(args, line)
end

-- Something that comes with Zeno, as a value.
local builtin_values = {}
function builtin(name, line)
  if name == "math.pi" then return math.pi end
  if name == "math.e" then return 2.718281828459045 end
  if IS_ERROR_KIND[name] then return setmt({kind = name}, ErrorType) end
  if builtin_values[name] then return builtin_values[name] end
  local module, member = name:match("^(.+)%.(.*)$")
  if not tabled(name) then
    if module then fatal("module `" .. module .. "` has no `" .. member .. "`", line or 0) end
    fatal("`" .. name .. "` is not defined", line or 0)
  end
  local fn = setmt({name = member or name, params = {}, defaults = {}, fn = false, builtin = name}, Fn)
  builtin_values[name] = fn
  return fn
end
Z.builtin = builtin
function Z.module(name) return setmt({name = name}, Module) end

-- --------------------------------------------------------- The program

-- Run the program, and say what stopped it if something did.
--
-- The program is a coroutine, and so is the body of each `try` that is
-- running (see `Z.try`): `running` holds them, the innermost last. The
-- innermost one runs until it ends, raises, or reaches a `try` of its own.
function Z.start(files, main)
  local function where(line)
    local file = files[line // LINES_A_FILE + 1] or files[1]
    return " --> " .. file .. ":" .. (line % LINES_A_FILE)
  end
  local running = {coroutine.create(main)}
  local ok, how, value = true, nil, nil
  local message
  while true do
    local routine = running[#running]
    local fine, first, second = coroutine.resume(routine, ok, how, value)
    if coroutine.status(routine) == "suspended" then
      -- It reached a `try`, and `first` is the body.
      running[#running + 1] = coroutine.create(first)
      ok, how, value = true, nil, nil
    elseif fine then
      -- It ended: the `try` it was the body of goes on.
      running[#running] = nil
      if #running == 0 then
        flush()
        return
      end
      ok, how, value = true, first, second
    elseif getmt(first) == Raise and #running > 1 then
      -- It raised: the `try` it was the body of catches it.
      running[#running] = nil
      ok, how, value = false, first, nil
    else
      if getmt(first) == Raise then
        message = "error: uncaught " .. shown_name(first.value.kind) .. ": " .. show(first.value) .. "\n" .. where(first.at)
      elseif getmt(first) == Fatal then
        message = "error: " .. first.text .. "\n" .. where(first.at)
      else
        -- A fault of Lua's own, or of this file, is shown with where it was.
        message = debug.traceback(routine, tostring(first))
      end
      break
    end
  end
  flush()
  io.stderr:write(message, "\n")
  os.exit(1)
end
