# The Python end of `import python` (docs/python.md).
#
# `zeno run` starts Python with this file as its program. It connects
# back to Zeno over a socket of this machine, then does what each line
# it is sent asks for, and answers with one line.
#
# A line is JSON. A value in it is written like this:
#
#     none, a bool, an int, a float, a str     as JSON writes them
#     a float JSON cannot write                ["f", "nan"], "inf", "-inf"
#     a list                                   ["l", [item, ...]]
#     a dict                                   ["d", [[key, value], ...]]
#     anything else, which stays here          ["p", number]
#
# What stays here is kept in `kept` under its number until Zeno says
# that nobody there holds it any more.
import importlib
import json
import os
import socket
import sys

kept = {}
count = 0

INT_MIN, INT_MAX = -(2 ** 63), 2 ** 63 - 1


def keep(thing):
    global count
    count += 1
    kept[count] = thing
    return ["p", count]


def out(value):
    """A value of Python, as it goes to Zeno. None, a bool, an int, a
    float and a str go as themselves. Everything else stays here, a list
    and a dict too: Zeno then changes the one list there is, and not a
    copy of it."""
    kind = type(value)
    if value is None or kind is bool or kind is str:
        return value
    if kind is int:
        return value if INT_MIN <= value <= INT_MAX else keep(value)
    if kind is float:
        if value != value:
            return ["f", "nan"]
        if value in (float("inf"), float("-inf")):
            return ["f", "inf" if value > 0 else "-inf"]
        return value
    return keep(value)


def copied(value, depth=0):
    """The same, but a list, a tuple and a dict go too, item by item."""
    if depth > 200:
        raise Told("ValueError", "this is nested more than 200 deep, or holds itself, and cannot be copied")
    kind = type(value)
    if kind is list or kind is tuple:
        return ["l", [copied(item, depth + 1) for item in value]]
    if kind is dict:
        for key in value:
            if type(key) is not str and not (type(key) is int and INT_MIN <= key <= INT_MAX):
                raise Told("ValueError", "a dict in Zeno has keys that are str or int, and this one has the key " + repr(key))
        return ["d", [[key, copied(item, depth + 1)] for key, item in value.items()]]
    return out(value)


class Told(Exception):
    """Something this end has to say itself, as an error of the kind
    Zeno knows it by."""

    def __init__(self, kind, text):
        Exception.__init__(self, text)
        self.kind = kind


def back(value):
    """A value Zeno sent, as a value of Python."""
    if type(value) is not list:
        return value
    tag, inside = value
    if tag == "p":
        if inside not in kept:
            raise Told("Error", "this value was kept by a Python that has stopped since")
        return kept[inside]
    if tag == "l":
        return [back(item) for item in inside]
    if tag == "d":
        return {back(key): back(item) for key, item in inside}
    return float(inside)


def kind_of(error):
    """The kind of error Zeno raises for an error of Python: its own
    kind where it has one of that name, and `Error` for the rest."""
    if isinstance(error, Told):
        return error.kind
    for kind in (ZeroDivisionError, OverflowError, IndexError, KeyError, ValueError):
        if isinstance(error, kind):
            return kind.__name__
    if isinstance(error, OSError):
        return "FileError"
    return "Error"


def said(error):
    name = type(error).__name__
    text = str(error)
    if isinstance(error, KeyError) and error.args:
        text = "there is no key " + repr(error.args[0])
    if name == kind_of(error) or isinstance(error, Told):
        return text
    return name + ": " + text if text else name


def do(ask):
    op = ask["op"]
    if op == "import":
        try:
            return keep(importlib.import_module(ask["name"]))
        except ImportError as error:
            raise Told(
                "FileError",
                "Python has no module `" + ask["name"] + "` (" + str(error) + "). "
                "Is it installed for this Python? It is " + sys.executable,
            )
    thing = back(ask["on"]) if "on" in ask else None
    if op == "get":
        return out(getattr(thing, ask["name"]))
    if op == "set":
        setattr(thing, ask["name"], back(ask["to"]))
        return None
    if op == "call":
        args = [back(arg) for arg in ask["args"]]
        named = {name: back(arg) for name, arg in ask["named"]}
        return out(thing(*args, **named))
    if op == "item":
        return out(thing[back(ask["at"])])
    if op == "set_item":
        thing[back(ask["at"])] = back(ask["to"])
        return None
    if op == "len":
        return len(thing)
    if op == "has":
        return back(ask["item"]) in thing
    if op == "items":
        return ["l", [out(item) for item in thing]]
    if op == "copy":
        return copied(thing)
    if op == "str":
        return str(thing)
    if op == "repr":
        return repr(thing)
    if op == "equal":
        return bool(thing == back(ask["to"]))
    raise Told("Error", "Zeno asked for `" + op + "`, which this Python end does not know")


def main():
    global count
    port, token, first = os.environ.pop("ZENO_BRIDGE").split()
    count = int(first)
    link = socket.create_connection(("127.0.0.1", int(port)))
    link.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    lines = link.makefile("r", encoding="utf-8", newline="\n")

    def send(answer):
        link.sendall((json.dumps(answer) + "\n").encode("utf-8"))

    send({"hello": token, "version": sys.version.split()[0], "program": sys.executable})
    for line in lines:
        ask = json.loads(line)
        for number in ask.get("free", ()):
            kept.pop(number, None)
        try:
            answer = {"ok": do(ask)}
        except KeyboardInterrupt:
            raise
        except BaseException as error:
            answer = {"err": [kind_of(error), said(error)]}
        # What Python printed is on the screen before Zeno goes on.
        for stream in (sys.stdout, sys.stderr):
            try:
                stream.flush()
            except Exception:
                pass
        send(answer)


try:
    main()
except (KeyboardInterrupt, OSError):
    # Ctrl-C, or Zeno is gone: there is nobody to answer.
    pass
