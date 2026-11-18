# The Ruby end of `import ruby` (docs/ruby.md).
#
# `zeno run` starts Ruby with this file as its program. It connects
# back to Zeno over a socket of this machine, then does what each line
# it is sent asks for, and answers with one line. The lines are the
# ones bridge.py reads and writes:
#
#     none, a bool, an int, a float, a str     as JSON writes them
#     a float JSON cannot write                ["f", "nan"], "inf", "-inf"
#     a list                                   ["l", [item, ...]]
#     a dict                                   ["d", [[key, value], ...]]
#     anything else, which stays here          ["p", number]
#
# What stays here is kept in KEPT under its number until Zeno says
# that nobody there holds it any more.
require "json"
require "socket"
require "rbconfig"

KEPT = {}
$count = 0

INT_MIN = -(2**63)
INT_MAX = 2**63 - 1

# Something this end has to say itself, as an error of the kind Zeno
# knows it by.
class Told < StandardError
  attr_reader :kind

  def initialize(kind, text)
    super(text)
    @kind = kind
  end
end

# `thing.name`: a method and what it will be called on, kept until it
# is called. In Ruby the dot alone is the call; in Zeno a call has
# brackets.
Bound = Struct.new(:on, :name)

def keep(thing)
  $count += 1
  KEPT[$count] = thing
  ["p", $count]
end

# A value of Ruby, as it goes to Zeno. nil, true, false, an Integer
# that fits 64 bits, a Float and a String of text go as themselves.
# Everything else stays here, an Array and a Hash too.
def out(value)
  case value
  when nil, true, false
    value
  when Integer
    value >= INT_MIN && value <= INT_MAX ? value : keep(value)
  when Float
    return ["f", "nan"] if value.nan?
    return ["f", value > 0 ? "inf" : "-inf"] if value.infinite?
    value
  when String
    return keep(value) unless value.instance_of?(String)
    # Ruby has one String for text and for bytes. Bytes with no
    # encoding said (what a socket or `File.binread` gives) are text
    # where they read as UTF-8, and text in another encoding is turned
    # into it. Bytes that are not text stay here.
    text =
      if value.encoding == Encoding::UTF_8 then value
      elsif value.encoding == Encoding::BINARY then value.dup.force_encoding(Encoding::UTF_8)
      else (value.encode(Encoding::UTF_8) rescue nil)
      end
    text && text.valid_encoding? ? text : keep(value)
  else
    keep(value)
  end
end

# The same, but an Array, a Set and a Hash go too, item by item, and a
# Symbol goes as its name: a copy is plain data.
def copied(value, depth = 0)
  raise Told.new("ValueError", "this is nested more than 200 deep, or holds itself, and cannot be copied") if depth > 200
  if value.is_a?(Hash)
    pairs = value.map do |key, item|
      key = key.to_s if key.is_a?(Symbol)
      unless (key.instance_of?(String) && out(key).instance_of?(String)) || (key.is_a?(Integer) && key >= INT_MIN && key <= INT_MAX)
        raise Told.new("ValueError", "a dict in Zeno has keys that are str or int, and this one has the key " + key.inspect)
      end
      [key, copied(item, depth + 1)]
    end
    return ["d", pairs]
  end
  if value.is_a?(Array) || (defined?(Set) && value.is_a?(Set))
    return ["l", value.map { |item| copied(item, depth + 1) }]
  end
  return value.to_s if value.is_a?(Symbol) && out(value.to_s).instance_of?(String)
  out(value)
end

# A value Zeno sent, as a value of Ruby.
def back(value)
  return value unless value.is_a?(Array)
  tag, inside = value
  case tag
  when "p"
    raise Told.new("Error", "this value was kept by a Ruby that has stopped since") unless KEPT.key?(inside)
    KEPT[inside]
  when "l"
    inside.map { |item| back(item) }
  when "d"
    inside.to_h { |key, item| [back(key), back(item)] }
  else
    { "nan" => Float::NAN, "inf" => Float::INFINITY, "-inf" => -Float::INFINITY }.fetch(inside) { Float(inside) }
  end
end

# The kind of error Zeno raises for an error of Ruby: its own kind
# where the two mean one thing, and `Error` for the rest.
def kind_of(error)
  case error
  when Told then error.kind
  when ZeroDivisionError then "ZeroDivisionError"
  when KeyError then "KeyError"
  when IndexError, StopIteration then "IndexError"
  when ArgumentError then "ValueError"
  when SystemCallError, IOError then "FileError"
  else "Error"
  end
end

def said(error)
  return error.message if error.is_a?(Told)
  name = error.class.name.to_s
  text = error.message.to_s
  # `raise "text"` makes a RuntimeError, which is the plain error of
  # Ruby: its text is all there is to say.
  return text if (name == kind_of(error) || error.instance_of?(RuntimeError)) && !text.empty?
  text.empty? || text == name ? name : name + ": " + text
end

def described(thing)
  return "a method" if thing.is_a?(Bound)
  if thing.is_a?(Module)
    kind = thing.is_a?(Class) ? "class" : "module"
    return thing.name ? "the " + kind + " " + thing.name : "a " + kind + " with no name"
  end
  # A class made without a name is called by the one it was made from.
  name = thing.class.ancestors.map(&:name).compact.first.to_s
  (name =~ /\A[AEIOU]/ ? "an " : "a ") + name
end

def named_ones(named)
  named.to_h { |name, arg| [name.to_sym, back(arg)] }
end

# A constant by its name as Ruby writes it: `Date`, `Net::HTTP`.
def constant(name)
  name.split("::").reject(&:empty?).inject(Object) { |inside, part| inside.const_get(part) }
rescue NameError
  raise Told.new("Error", "Ruby has no constant `" + name + "`. One that a library brings is there after `ruby.require` of the library")
end

def get(thing, name)
  if thing.is_a?(Module) && name =~ /\A[A-Z]/ && thing.const_defined?(name)
    return out(thing.const_get(name))
  end
  if !thing.is_a?(Bound) && thing.respond_to?(name)
    return keep(Bound.new(thing, name))
  end
  # `empty?` and `sort!` are names Zeno cannot write after a dot.
  near = ["?", "!"].map { |mark| name + mark }.find { |other| !thing.is_a?(Bound) && thing.respond_to?(other) }
  hint = near ? ". It has `" + near + "`, which is written `.send(\"" + near + "\")` in Zeno" : ""
  raise Told.new("Error", described(thing) + " has no method `" + name + "`" + hint)
end

def call(thing, args, named)
  # Before Ruby 3 an empty `**named` was one more argument, so it is
  # only given when there is something in it.
  if thing.is_a?(Bound)
    return named.empty? ? thing.on.public_send(thing.name, *args) : thing.on.public_send(thing.name, *args, **named)
  end
  if thing.is_a?(Class)
    raise Told.new("Error", described(thing) + " is not called in Ruby. An object of it is made with `.new(...)`")
  end
  if thing.respond_to?(:call)
    return named.empty? ? thing.call(*args) : thing.call(*args, **named)
  end
  raise Told.new("Error", described(thing) + " is not a method, and cannot be called")
end

def length_of(thing)
  return thing.length if thing.respond_to?(:length)
  return thing.size if thing.respond_to?(:size)
  raise Told.new("Error", described(thing) + " has no length")
end

def has(thing, item)
  return thing.include?(item) if thing.respond_to?(:include?)
  raise Told.new("Error", described(thing) + " cannot be asked what is `in` it")
end

# Whatever Ruby goes through with `each`, taken now, all of it. A Hash
# is gone through by its keys, as a dict is.
def items_of(thing)
  return thing.keys if thing.is_a?(Hash)
  return thing.dup if thing.instance_of?(Array)
  raise Told.new("Error", described(thing) + " cannot be gone through with `for`") unless thing.respond_to?(:each)
  endless = (thing.is_a?(Range) && thing.end.nil?) || (thing.respond_to?(:size) && thing.size == Float::INFINITY rescue false)
  raise Told.new("ValueError", described(thing) + " that has no end cannot be gone through with `for`. `.first(n)` takes the first n of it") if endless
  items = []
  thing.each { |*given| items << (given.length == 1 ? given[0] : given) }
  items
end

def shown(thing, how)
  if thing.is_a?(Bound)
    return "method `" + thing.name + "` of " + described(thing.on) + ". In Zeno a call has brackets: `." + thing.name + "()`"
  end
  how == "repr" ? thing.inspect : thing.to_s
end

def act(ask)
  op = ask["op"]
  name = ask["name"]
  case op
  when "import"
    begin
      require name
    rescue LoadError => error
      raise Told.new(
        "FileError",
        "Ruby has no library `" + name + "` (" + error.message.lines.first.to_s.strip + "). " \
        "Is its gem installed for this Ruby? It is " + RbConfig.ruby
      )
    end
    return nil
  when "global"
    return out(constant(name))
  when "symbol"
    return keep(name.to_sym)
  end
  thing = back(ask["on"])
  case op
  when "get" then get(thing, name)
  when "set"
    thing.public_send(name + "=", back(ask["to"]))
    nil
  when "call" then out(call(thing, ask["args"].map { |arg| back(arg) }, named_ones(ask["named"])))
  when "item" then out(thing[back(ask["at"])])
  when "set_item"
    thing[back(ask["at"])] = back(ask["to"])
    nil
  when "len" then length_of(thing)
  when "has" then has(thing, back(ask["item"])) ? true : false
  when "items" then ["l", items_of(thing).map { |item| out(item) }]
  when "copy" then copied(thing)
  when "str", "repr" then out(shown(thing, op)).then { |text| text.is_a?(String) ? text : thing.inspect.scrub }
  when "equal" then (thing == back(ask["to"])) ? true : false
  else raise Told.new("Error", "Zeno asked for `" + op.to_s + "`, which this Ruby end does not know")
  end
end

def main
  port, token, first = ENV.delete("ZENO_BRIDGE").split
  $count = Integer(first)
  # A library is looked for where Ruby looks for the libraries of a
  # program, and first in the folder the program is in.
  home = ENV.delete("ZENO_PROGRAM")
  if home && !home.empty?
    $LOAD_PATH.unshift(home)
    # A Gemfile beside the program is the one `bundler/setup` reads.
    gemfile = File.join(home, "Gemfile")
    ENV["BUNDLE_GEMFILE"] ||= gemfile if File.file?(gemfile)
  end
  Encoding.default_external = Encoding::UTF_8
  link = TCPSocket.new("127.0.0.1", Integer(port))
  link.setsockopt(Socket::IPPROTO_TCP, Socket::TCP_NODELAY, 1)
  link.set_encoding("UTF-8")
  link.write(JSON.generate({ "hello" => token, "version" => RUBY_VERSION, "program" => RbConfig.ruby }) + "\n")
  while (line = link.gets)
    begin
      ask = JSON.parse(line, max_nesting: false)
      (ask["free"] || []).each { |number| KEPT.delete(number) }
      answer = { "ok" => act(ask) }
    rescue Interrupt, SystemExit
      raise
    rescue Exception => error # rubocop:disable Lint/RescueException
      answer = { "err" => [kind_of(error), said(error).to_s.scrub] }
    end
    # What Ruby printed is on the screen before Zeno goes on.
    [$stdout, $stderr].each { |stream| stream.flush rescue nil }
    link.write(JSON.generate(answer, max_nesting: false) + "\n")
  end
end

begin
  main
rescue Interrupt, SystemCallError, IOError
  # Ctrl-C, or Zeno is gone: there is nobody to answer.
end
