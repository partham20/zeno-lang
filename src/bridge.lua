-- The Lua end of `import lua` (docs/lua.md).
--
-- `zeno run` starts Lua with this file as its program. It reads what
-- Zeno asks for, one line at a time, does it, and answers with one
-- line. The lines are the ones bridge.py reads and writes:
--
--     none, a bool, an int, a float, a str     as JSON writes them
--     a float JSON cannot write                ["f", "nan"], "inf", "-inf"
--     a list                                   ["l", [item, ...]]
--     a dict                                   ["d", [[key, value], ...]]
--     anything else, which stays here          ["p", number]
--
-- Lua comes with no sockets. On Linux and macOS the lines go through
-- two named pipes that Zeno made, which Lua opens as files. On
-- Windows they go over a socket of this machine, and Lua needs
-- LuaSocket for that.
--
-- Written for Lua 5.1, 5.2, 5.3, 5.4 and LuaJIT.

local KEPT = {}
local COUNT = 0
local HOME = ""

local unpack = table.unpack or unpack
local floor = math.floor
local mtype = math.type
local huge = math.huge

-- Something this end has to say itself, as an error of the kind Zeno
-- knows it by.
local TOLD = {}
local function told(kind, text)
  error(setmetatable({ kind = kind, text = text }, TOLD), 0)
end

-- `thing.name` where Lua would write `thing:name`: a function and what
-- it will be called on.
local BOUND = {}
-- `lua.global(name)` of a name that is not there yet; and the values
-- Lua has that JSON has no word for.
local function bound(on, fn, name)
  return setmetatable({ on = on, fn = fn, name = name }, BOUND)
end

local function keep(thing)
  COUNT = COUNT + 1
  KEPT[COUNT] = thing
  return { "p", COUNT }
end

-- Is this string text: UTF-8 with nothing missing?
local function is_text(s)
  local at, n = 1, #s
  while at <= n do
    local c = s:byte(at)
    local more
    if c < 0x80 then more = 0
    elseif c >= 0xC2 and c <= 0xDF then more = 1
    elseif c >= 0xE0 and c <= 0xEF then more = 2
    elseif c >= 0xF0 and c <= 0xF4 then more = 3
    else return false end
    for i = 1, more do
      local d = s:byte(at + i)
      if not d or d < 0x80 or d > 0xBF then return false end
    end
    if more == 2 then
      local d = s:byte(at + 1)
      if (c == 0xE0 and d < 0xA0) or (c == 0xED and d > 0x9F) then return false end
    elseif more == 3 then
      local d = s:byte(at + 1)
      if (c == 0xF0 and d < 0x90) or (c == 0xF4 and d > 0x8F) then return false end
    end
    at = at + more + 1
  end
  return true
end

-- Is this number whole? In Lua 5.3 and later a number knows; before,
-- every number is a float, and a whole one is taken for an int.
local function is_int(x)
  if mtype then return mtype(x) == "integer" end
  return x == floor(x) and x >= -9007199254740992 and x <= 9007199254740992
end

-- The keys 1, 2, ... n and nothing else: a sequence, which is a list.
local function sequence_length(t)
  local n = 0
  for _ in pairs(t) do n = n + 1 end
  for i = 1, n do
    if rawget(t, i) == nil then return nil end
  end
  return n
end

-- A value of Lua, as it goes to Zeno.
local function out(value)
  local kind = type(value)
  if value == nil or kind == "boolean" then return value end
  if kind == "number" then
    if value ~= value then return { "f", "nan" } end
    if value == huge then return { "f", "inf" } end
    if value == -huge then return { "f", "-inf" } end
    if is_int(value) then return { "i", value } end
    return value
  end
  if kind == "string" then
    if is_text(value) then return value end
    return keep(value)
  end
  return keep(value)
end

local function described(thing)
  if getmetatable(thing) == BOUND then return "a method" end
  local kind = type(thing)
  if kind == "string" then return "bytes" end
  if kind == "userdata" then
    local ok, name = pcall(tostring, thing)
    name = ok and tostring(name):match("^([%w_%.]+)") or nil
    return name and name ~= "userdata" and "a " .. name or "a userdata"
  end
  return (kind:match("^[aeiou]") and "an " or "a ") .. kind
end

-- `lua.copy`: a table, as a list where its keys are 1, 2, ... n, else
-- as a dict, with everything in it.
local function copied(value, depth)
  depth = depth or 0
  if depth > 200 then
    told("ValueError", "this is nested more than 200 deep, or holds itself, and cannot be copied")
  end
  if type(value) ~= "table" or getmetatable(value) == BOUND then return out(value) end
  local n = sequence_length(value)
  if n and n > 0 then
    local items = {}
    for i = 1, n do items[i] = copied(value[i], depth + 1) end
    return { "l", items }
  end
  local pairs_ = {}
  for key, item in pairs(value) do
    local kind = type(key)
    if not ((kind == "string" and is_text(key)) or (kind == "number" and is_int(key))) then
      told("ValueError", "a dict in Zeno has keys that are str or int, and this one has the key " .. tostring(key))
    end
    pairs_[#pairs_ + 1] = { kind == "number" and { "i", key } or key, copied(item, depth + 1) }
  end
  if #pairs_ == 0 then return { "l", {} } end
  return { "d", pairs_ }
end

-- A value Zeno sent, as a value of Lua.
local function back(value)
  if type(value) ~= "table" then return value end
  local tag, inside = value[1], value[2]
  if tag == "p" then
    if KEPT[inside] == nil then told("Error", "this value was kept by a Lua that has stopped since") end
    return KEPT[inside]
  elseif tag == "i" then
    return inside
  elseif tag == "l" then
    local made = {}
    for i = 1, #inside do made[i] = back(inside[i]) end
    return made
  elseif tag == "d" then
    local made = {}
    for _, pair in ipairs(inside) do made[back(pair[1])] = back(pair[2]) end
    return made
  else
    if inside == "nan" then return 0 / 0 end
    if inside == "inf" then return huge end
    return -huge
  end
end

---------------------------------------------------------------------
-- JSON, as much as the lines need. Lua comes with none.

local escapes = { ['"'] = '\\"', ["\\"] = "\\\\", ["\b"] = "\\b", ["\f"] = "\\f", ["\n"] = "\\n", ["\r"] = "\\r", ["\t"] = "\\t" }

local function write_text(s)
  return '"' .. s:gsub('[%c"\\]', function (c)
    return escapes[c] or string.format("\\u%04x", c:byte())
  end) .. '"'
end

-- A number, as JSON writes it. An int is written with no point and a
-- float always with one, so that each arrives as what it was.
local function write_number(x, whole)
  if whole then
    if mtype then return string.format("%d", x) end
    return string.format("%.0f", x)
  end
  local text = string.format("%.17g", x)
  for digits = 15, 16 do
    local shorter = string.format("%." .. digits .. "g", x)
    if tonumber(shorter) == x then text = shorter break end
  end
  if not text:find("[%.eEn]") then text = text .. ".0" end
  return text
end

local write

-- A value made by `out` or `copied`: plain values, and lists of a tag
-- and what is inside.
write = function (v)
  local kind = type(v)
  if v == nil then return "null" end
  if kind == "boolean" then return v and "true" or "false" end
  if kind == "number" then return write_number(v, false) end
  if kind == "string" then return write_text(v) end
  if v[1] == "i" then return write_number(v[2], true) end
  if v[1] == "p" then return '["p",' .. write_number(v[2], true) .. "]" end
  if v[1] == "f" then return '["f","' .. v[2] .. '"]' end
  local parts = {}
  if v[1] == "l" then
    for i = 1, #v[2] do parts[i] = write(v[2][i]) end
    return '["l",[' .. table.concat(parts, ",") .. "]]"
  end
  for i, pair in ipairs(v[2]) do parts[i] = "[" .. write(pair[1]) .. "," .. write(pair[2]) .. "]" end
  return '["d",[' .. table.concat(parts, ",") .. "]]"
end

-- An object of JSON as a table with its names; an array as a table
-- with `n`, since it can hold null. A whole number is tagged as an int.
local function read(text)
  local at = 1
  local value
  local function space()
    at = text:find("[^ \t\r\n]", at) or #text + 1
  end
  local function utf8_of(code)
    if code < 0x80 then return string.char(code) end
    if code < 0x800 then return string.char(0xC0 + floor(code / 64), 0x80 + code % 64) end
    if code < 0x10000 then
      return string.char(0xE0 + floor(code / 4096), 0x80 + floor(code / 64) % 64, 0x80 + code % 64)
    end
    return string.char(0xF0 + floor(code / 262144), 0x80 + floor(code / 4096) % 64, 0x80 + floor(code / 64) % 64, 0x80 + code % 64)
  end
  local function str()
    local parts = {}
    at = at + 1
    while true do
      local stop = text:find('["\\]', at)
      parts[#parts + 1] = text:sub(at, stop - 1)
      if text:sub(stop, stop) == '"' then at = stop + 1 break end
      local c = text:sub(stop + 1, stop + 1)
      if c == "u" then
        local code = tonumber(text:sub(stop + 2, stop + 5), 16)
        at = stop + 6
        if code >= 0xD800 and code <= 0xDBFF and text:sub(at, at + 1) == "\\u" then
          local low = tonumber(text:sub(at + 2, at + 5), 16)
          code = 0x10000 + (code - 0xD800) * 1024 + (low - 0xDC00)
          at = at + 6
        end
        parts[#parts + 1] = utf8_of(code)
      else
        parts[#parts + 1] = ({ b = "\b", f = "\f", n = "\n", r = "\r", t = "\t" })[c] or c
        at = stop + 2
      end
    end
    return table.concat(parts)
  end
  value = function ()
    space()
    local c = text:sub(at, at)
    if c == "{" then
      local made = {}
      at = at + 1
      space()
      if text:sub(at, at) == "}" then at = at + 1 return made end
      while true do
        space()
        local name = str()
        space()
        at = at + 1
        made[name] = value()
        space()
        local sep = text:sub(at, at)
        at = at + 1
        if sep == "}" then return made end
      end
    elseif c == "[" then
      local made = { n = 0 }
      at = at + 1
      space()
      if text:sub(at, at) == "]" then at = at + 1 return made end
      while true do
        made.n = made.n + 1
        made[made.n] = value()
        space()
        local sep = text:sub(at, at)
        at = at + 1
        if sep == "]" then return made end
      end
    elseif c == '"' then
      return str()
    elseif text:sub(at, at + 3) == "true" then at = at + 4 return true
    elseif text:sub(at, at + 4) == "false" then at = at + 5 return false
    elseif text:sub(at, at + 3) == "null" then at = at + 4 return nil
    end
    local number = text:match("^-?[%d%.eE+-]+", at)
    at = at + #number
    if number:find("[%.eE]") then return tonumber(number) end
    return { "i", math.tointeger and math.tointeger(tonumber(number)) or tonumber(number) }
  end
  return value()
end

-- A value of the lines as `read` gives it, as `back` takes it: an
-- int as `{"i", n}`, and a tagged list as the tag and what is inside.
-- Every array of the lines is tagged, so its first item says what it
-- is.
local function unread(v)
  if type(v) ~= "table" or v.n == nil then return v end
  local tag, inside = v[1], v[2]
  if tag == "l" then
    local items = {}
    for i = 1, inside.n do items[i] = unread(inside[i]) end
    return { "l", items }
  elseif tag == "d" then
    local pairs_ = {}
    for i = 1, inside.n do pairs_[i] = { unread(inside[i][1]), unread(inside[i][2]) } end
    return { "d", pairs_ }
  elseif tag == "p" then
    return { "p", inside[2] }
  end
  return { tag, inside }
end

---------------------------------------------------------------------
-- What Zeno asks for.

-- The kind of error Zeno raises for an error of Lua. Lua has no kinds
-- of error, so it is `Error`, with what Lua said.
local function kind_of(e)
  if getmetatable(e) == TOLD then return e.kind end
  return "Error"
end

local function said(e)
  if getmetatable(e) == TOLD then return e.text end
  -- Where in this helper a function of Lua was called from says
  -- nothing to a program of Zeno.
  if type(e) == "string" then return (e:gsub("^%(command line%):%d+: ", "")) end
  local ok, text = pcall(tostring, e)
  return ok and tostring(text) or "an error that cannot be written"
end

-- Does this function want what it was read from as `self`? A function
-- of Lua written `function T:name()` has a first parameter called
-- `self`, and Lua 5.2 and later can say so. Where Lua cannot say (5.1,
-- and a function written in C), a function that the table has from
-- its metatable is taken for a method: that is how Lua makes objects,
-- where a module holds its functions itself.
local function wants_self(thing, name, fn)
  local written_in_c = debug.getinfo(fn, "S").what == "C"
  local ok, first = pcall(debug.getlocal, fn, 1)
  if ok and not written_in_c then return first == "self" end
  return rawget(thing, name) == nil and getmetatable(thing) ~= nil
end

local function get(thing, name)
  local kind = type(thing)
  if kind ~= "table" and kind ~= "userdata" and kind ~= "string" or getmetatable(thing) == BOUND then
    told("Error", described(thing) .. " has no field `" .. name .. "`")
  end
  local value = thing[name]
  if value == nil and kind ~= "table" then
    told("Error", described(thing) .. " has no field or method `" .. name .. "`")
  end
  -- What the methods of a userdata (a file, a socket) are read from is
  -- what they are called on, as `file:read()`.
  if type(value) == "function" and (kind ~= "table" or wants_self(thing, name, value)) then
    return keep(bound(thing, value, name))
  end
  return out(value)
end

-- Several values, as Lua gives them: none, one, or a list. `nil` and
-- a message is how most functions of Lua say they failed (what
-- `assert` turns into an error), and here it is an error: a
-- `FileError` when a number of the system comes third.
local function results(n, ...)
  local first, second, third = ...
  if n >= 2 and first == nil and type(second) == "string" then
    told(n >= 3 and type(third) == "number" and "FileError" or "Error", second)
  end
  if n <= 1 then return out(first) end
  local items = {}
  local all = { ... }
  for i = 1, n do items[i] = out(all[i]) end
  return { "l", items }
end

local function call(thing, args, named)
  local fn, given = thing, {}
  local n = 0
  if getmetatable(thing) == BOUND then
    fn = thing.fn
    n = 1
    given[1] = thing.on
  end
  local callable = type(fn) == "function" or (getmetatable(fn) and getmetatable(fn).__call)
  if not callable then
    told("Error", described(thing) .. " is not a function, and cannot be called")
  end
  for i = 1, #args do given[n + i] = back(args[i]) end
  n = n + #args
  -- Named arguments are the table of options that Lua takes last.
  if #named > 0 then
    local options = {}
    for _, pair in ipairs(named) do options[pair[1]] = back(pair[2]) end
    n = n + 1
    given[n] = options
  end
  return (function (...) return results(select("#", ...), ...) end)(fn(unpack(given, 1, n)))
end

local function length_of(thing)
  local kind = type(thing)
  local meta = getmetatable(thing)
  if (kind == "table" and meta ~= BOUND) or kind == "string" or (meta and meta.__len) then
    return { "i", #thing }
  end
  told("Error", described(thing) .. " has no length")
end

-- A table that is a sequence gives its items, in order; any other
-- table gives its keys, as a dict does. Something with `__pairs`
-- gives what that gives.
local function items_of(thing)
  local made = {}
  if type(thing) == "string" then
    for i = 1, #thing do made[i] = { "i", thing:byte(i) } end
    return made
  end
  -- A function is what Lua's own `for` goes through: it is called
  -- until it gives nil, as `string.gmatch` and `io.lines` give.
  if type(thing) == "function" then
    while true do
      local item = thing()
      if item == nil then return made end
      made[#made + 1] = out(item)
    end
  end
  if type(thing) ~= "table" or getmetatable(thing) == BOUND then
    told("Error", described(thing) .. " cannot be gone through with `for`")
  end
  local n = sequence_length(thing)
  if n then
    for i = 1, n do made[i] = out(thing[i]) end
    return made
  end
  for key in pairs(thing) do made[#made + 1] = out(key) end
  return made
end

local function has(thing, item)
  if type(thing) ~= "table" or getmetatable(thing) == BOUND then
    told("Error", described(thing) .. " cannot be asked what is `in` it")
  end
  local n = sequence_length(thing)
  if n and n > 0 then
    for i = 1, n do
      if thing[i] == item then return true end
    end
    return false
  end
  return thing[item] ~= nil
end

local function shown(thing, how)
  if getmetatable(thing) == BOUND then
    return "method `" .. thing.name .. "` of " .. described(thing.on)
  end
  if type(thing) == "string" then
    return '"' .. thing:gsub('[^\32-\126]', function (c) return string.format("\\x%02X", c:byte()) end):gsub('"', '\\"') .. '"'
  end
  local meta = getmetatable(thing)
  if type(meta) == "table" and meta.__tostring then return tostring(thing) end
  if type(thing) == "table" then
    local ok, made = pcall(copied, thing)
    if ok then
      -- What a table holds, as Zeno prints a list or a dict.
      local function show(v)
        if type(v) == "string" then return write_text(v) end
        if v == nil then return "nil" end
        if type(v) ~= "table" then return tostring(v) end
        if v[1] == "i" then return write_number(v[2], true) end
        if v[1] == "f" then return v[2] end
        if v[1] == "p" then return shown(KEPT[v[2]], "repr") end
        local parts = {}
        if v[1] == "l" then
          for i = 1, #v[2] do parts[i] = show(v[2][i]) end
          return "{" .. table.concat(parts, ", ") .. "}"
        end
        for i, pair in ipairs(v[2]) do
          local key = pair[1]
          key = type(key) == "table" and "[" .. show(key) .. "]" or (key:match("^[%a_][%w_]*$") and key or "[" .. write_text(key) .. "]")
          parts[i] = key .. " = " .. show(pair[2])
        end
        return "{" .. table.concat(parts, ", ") .. "}"
      end
      return show(made)
    end
  end
  return tostring(thing)
end

local function equal(one, other)
  if one == other then return true end
  return false
end

local function act(ask)
  local op, name = ask.op, ask.name
  if op == "import" then
    local found, module = pcall(require, name)
    if not found then
      if type(module) == "string" and module:find("module '" .. name .. "' not found", 1, true) then
        told("FileError", "Lua has no module `" .. name .. "`. Is it installed for this Lua (`luarocks install`)? It is " .. _VERSION)
      end
      error(module, 0)
    end
    return out(module)
  elseif op == "global" then
    local value = _G[name]
    if value == nil then told("Error", "Lua has no global `" .. name .. "`") end
    return out(value)
  end
  local thing = back(ask.on)
  if op == "get" then return get(thing, name)
  elseif op == "set" then
    if type(thing) ~= "table" and type(thing) ~= "userdata" then
      told("Error", described(thing) .. " has no fields that can be set")
    end
    thing[name] = back(ask.to)
    return nil
  elseif op == "call" then return call(thing, ask.args, ask.named)
  elseif op == "item" then
    if type(thing) ~= "table" and type(thing) ~= "userdata" then told("Error", described(thing) .. " has no items") end
    return out(thing[back(ask.at)])
  elseif op == "set_item" then
    if type(thing) ~= "table" and type(thing) ~= "userdata" then told("Error", described(thing) .. " has no items that can be set") end
    local at = back(ask.at)
    if at == nil then told("Error", "a table of Lua has no key `none`") end
    thing[at] = back(ask.to)
    return nil
  elseif op == "len" then return length_of(thing)
  elseif op == "has" then return has(thing, back(ask.item))
  elseif op == "items" then return { "l", items_of(thing) }
  elseif op == "copy" then return copied(thing)
  elseif op == "str" or op == "repr" then
    local text = shown(thing, op)
    return is_text(text) and text or shown(text, "repr")
  elseif op == "equal" then return equal(thing, back(ask.to))
  end
  told("Error", "Zeno asked for `" .. tostring(op) .. "`, which this Lua end does not know")
end

-- The two ways to talk: a pair of named pipes, or a socket.
local function link(bridge)
  local first, second = bridge:match("^pipe (.*)|(.*) %S+ %S+$")
  if first then
    -- Zeno opens its end of the first, then of the second. Each open
    -- waits for the other end, so the order is the same here.
    local to_zeno = assert(io.open(first, "wb"))
    local from_zeno = assert(io.open(second, "rb"))
    to_zeno:setvbuf("no")
    return function () return from_zeno:read("*l") end, function (line) to_zeno:write(line) end
  end
  local port = bridge:match("^(%d+) ")
  local found, socket = pcall(require, "socket")
  if not found then
    io.stderr:write("Lua needs LuaSocket to talk to Zeno here: luarocks install luasocket\n")
    os.exit(1)
  end
  local client = assert(socket.connect("127.0.0.1", tonumber(port)))
  client:setoption("tcp-nodelay", true)
  return function () return client:receive("*l") end, function (line) client:send(line) end
end

local function main()
  local bridge = os.getenv("ZENO_BRIDGE") or ""
  local token, first = bridge:match("(%S+) (%S+)$")
  COUNT = tonumber(first)
  -- A module is looked for where Lua looks for the modules of a
  -- program, and first in the folder the program is in.
  HOME = os.getenv("ZENO_PROGRAM") or ""
  if HOME ~= "" then
    local sep = package.config:sub(1, 1)
    package.path = HOME .. sep .. "?.lua;" .. HOME .. sep .. "?" .. sep .. "init.lua;" .. package.path
    package.cpath = HOME .. sep .. "?" .. (sep == "\\" and ".dll" or ".so") .. ";" .. package.cpath
  end
  local receive, send = link(bridge)
  send('{"hello":' .. write_text(token) .. ',"version":' .. write_text(_VERSION) .. "}\n")
  while true do
    local line = receive()
    if not line then break end
    local ok, answer = pcall(function ()
      local ask = read(line)
      local free = ask.free or { n = 0 }
      for i = 1, free.n do KEPT[free[i][2]] = nil end
      ask.on = unread(ask.on)
      ask.to = unread(ask.to)
      ask.at = unread(ask.at)
      ask.item = unread(ask.item)
      if ask.args then
        local args = {}
        for i = 1, ask.args.n do args[i] = unread(ask.args[i]) end
        ask.args = args
        local named = {}
        for i = 1, ask.named.n do named[i] = { ask.named[i][1], unread(ask.named[i][2]) } end
        ask.named = named
      end
      return '{"ok":' .. write(act(ask)) .. "}"
    end)
    if not ok then
      answer = '{"err":[' .. write_text(kind_of(answer)) .. "," .. write_text(said(answer)) .. "]}"
    end
    -- What Lua printed is on the screen before Zeno goes on.
    io.stdout:flush()
    send(answer .. "\n")
  end
end

main()
