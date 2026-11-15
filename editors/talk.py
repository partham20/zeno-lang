#!/usr/bin/env python3
"""Talk to `zeno lsp` the way an editor does, and print both sides.

    python3 editors/talk.py path/to/zeno examples/thermostat/main.zeno

It starts the language server, opens the file, puts a mistake into it,
takes the mistake out again, asks for the file to be laid out, and stops
the server. Each message is printed as it is sent (-->) or comes back
(<--). docs/lsp.md walks through what you see.
"""
import json
import pathlib
import subprocess
import sys


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    zeno, file = sys.argv[1], pathlib.Path(sys.argv[2]).resolve()
    text = file.read_text()
    server = subprocess.Popen([zeno, "lsp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE)

    def send(method, params=None, id=None):
        message = {"jsonrpc": "2.0", "method": method}
        if id is not None:
            message["id"] = id
        if params is not None:
            message["params"] = params
        body = json.dumps(message).encode()
        print("-->", shorten(message))
        server.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
        server.stdin.flush()

    def receive():
        length = 0
        while True:
            line = server.stdout.readline().strip()
            if not line:
                break
            name, _, value = line.partition(b":")
            if name.lower() == b"content-length":
                length = int(value)
        message = json.loads(server.stdout.read(length))
        print("<--", shorten(message))
        return message

    document = {"uri": file.as_uri()}
    send("initialize", {"rootUri": file.parent.as_uri(), "capabilities": {}}, id=1)
    receive()
    send("initialized", {})

    # The file as it is on disk. The server answers with its mistakes:
    # an empty list, if it has none.
    send("textDocument/didOpen", {"textDocument": {**document, "languageId": "zeno", "version": 1, "text": text}})
    receive()

    # A mistake is typed: a name nothing declares, on a new last line.
    broken = text + "print(no_such_name)\n"
    send("textDocument/didChange", {"textDocument": {**document, "version": 2}, "contentChanges": [{"text": broken}]})
    found = receive()["params"]["diagnostics"]

    # It is taken out again, and the layout is spoiled instead.
    untidy = text.replace(" = ", "=", 1)
    send("textDocument/didChange", {"textDocument": {**document, "version": 3}, "contentChanges": [{"text": untidy}]})
    receive()
    send("textDocument/formatting", {"textDocument": document, "options": {"tabSize": 4, "insertSpaces": True}}, id=2)
    edits = receive()["result"]

    send("shutdown", id=3)
    receive()
    send("exit")
    server.stdin.close()
    status = server.wait()

    print()
    print("mistakes shown for the typed line:", len(found))
    print("the file was laid out again:", bool(edits) and edits[0]["newText"] == text)
    print("the server stopped with status", status)
    return 0 if len(found) == 1 and edits and edits[0]["newText"] == text and status == 0 else 1


def shorten(message):
    """One line for a message, with long texts cut."""
    def cut(value):
        if isinstance(value, str) and len(value) > 60:
            return value[:57].replace("\n", "\\n") + "..."
        if isinstance(value, dict):
            return {name: cut(inner) for name, inner in value.items()}
        if isinstance(value, list):
            return [cut(inner) for inner in value]
        return value
    return json.dumps(cut(message))


if __name__ == "__main__":
    sys.exit(main())
