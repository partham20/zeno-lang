// Starts a Zeno program in a page.
//
// `zeno build --web` puts this file into the page it writes, with the
// program and with `zeno.wasm`, which is Zeno itself built for a browser
// (src/web_host.rs). The last line of the page calls `Zeno.start`.
//
// WebAssembly cannot touch a page. It can only call the functions it is
// given when it starts. This file is those functions: the ones that show
// what a program prints, and the ones that do what the `web` module
// asks (`ops`, below). It also goes the other way: when a button is
// pressed, it calls the function of the program that `on` was given.
//
// Text crosses between the two as a place in the program's memory and a
// length in bytes, in UTF-8. docs/web.md walks through all of it.

(function () {
  "use strict";

  const decoder = new TextDecoder();
  const encoder = new TextEncoder();

  let zeno = null;      // what zeno.wasm lets the page call
  let stopped = false;  // an error ended the program: call nothing more
  let running = false;  // the program is in the middle of something
  const waiting = [];   // calls that arrived while it was

  // The program knows an element by a number. This is which is which.
  const elements = [];
  // The timers that are waiting, by the number of their function.
  const timers = {};
  const numbers = new Map();

  function numberOf(element) {
    if (!element) return null;
    if (!numbers.has(element)) {
      numbers.set(element, elements.length);
      elements.push(element);
    }
    return numbers.get(element);
  }

  // The text the program put at `place`.
  function read(place, length) {
    return decoder.decode(new Uint8Array(zeno.memory.buffer, place, length));
  }

  // Put text where the program can read it. The program frees the room.
  function give(text) {
    const bytes = encoder.encode(text);
    const place = zeno.zeno_alloc(bytes.length);
    new Uint8Array(zeno.memory.buffer, place, bytes.length).set(bytes);
    return [place, bytes.length];
  }

  // An answer to the program: four bytes that say how long it is, and
  // then the answer as JSON.
  function answer(value) {
    const bytes = encoder.encode(JSON.stringify(value === undefined ? null : value));
    const place = zeno.zeno_alloc(4 + bytes.length);
    new DataView(zeno.memory.buffer).setUint32(place, bytes.length, true);
    new Uint8Array(zeno.memory.buffer, place + 4, bytes.length).set(bytes);
    return place;
  }

  // What `print` prints goes to the browser's console, and into the
  // element with the id `zeno-output` if the page has one.
  let line = "";
  function print(text) {
    line += text;
    while (line.includes("\n")) {
      const end = line.indexOf("\n");
      console.log(line.slice(0, end));
      line = line.slice(end + 1);
    }
    const output = document.getElementById("zeno-output");
    if (output) output.textContent += text;
  }

  // An error that nothing caught stops the program, as it does under
  // `zeno run`. It is shown at the foot of the page, where it cannot
  // be missed.
  function error(text) {
    stopped = true;
    console.error(text);
    let box = document.getElementById("zeno-error");
    if (!box) {
      box = document.createElement("pre");
      box.id = "zeno-error";
      box.style.cssText =
        "position:fixed;left:0;right:0;bottom:0;margin:0;padding:12px 16px;max-height:40%;" +
        "overflow:auto;background:#fff0f0;color:#900;border-top:2px solid #c00;" +
        "font:14px/1.4 ui-monospace,monospace;white-space:pre-wrap;z-index:99999";
      document.body.appendChild(box);
    }
    box.textContent += text;
  }

  // Run something of the program's. If the program is already in the
  // middle of something, this waits its turn.
  function run(step) {
    if (stopped) return;
    if (running) {
      waiting.push(step);
      return;
    }
    running = true;
    try {
      step();
    } catch (trap) {
      // Not a mistake in the program: Zeno itself fell over.
      error("Zeno itself stopped, which is a bug in Zeno and not in this program: " + trap + "\n");
    }
    running = false;
    if (waiting.length > 0) run(waiting.shift());
  }

  // Call a function the program gave to `on`, `after`, `on_key` or
  // `fetch`. `carried` is what the call hands over, if anything:
  // {key: "Enter"}, {event: {...}} or {reply: [200, "..."]}.
  function call(handler, carried) {
    run(function () {
      if (carried === undefined) zeno.zeno_call(handler);
      else zeno.zeno_call_with(handler, ...give(JSON.stringify(carried)));
    });
  }

  // How the program shows an element when it prints one: `#count` if it
  // has an id, and `<li>` if not.
  function nameOf(element) {
    return element.id ? "#" + element.id : "<" + element.tagName.toLowerCase() + ">";
  }

  // What the program is told of an event. The place of the mouse is
  // counted from the top left corner of the element `on` was given to.
  function told(happened, kind, element) {
    const box = element.getBoundingClientRect();
    const mouse = typeof happened.clientX === "number";
    const target = happened.target instanceof Element ? happened.target : element;
    return {
      kind: kind,
      target: numberOf(target),
      name: nameOf(target),
      x: mouse ? happened.clientX - box.left : 0,
      y: mouse ? happened.clientY - box.top : 0,
      key: typeof happened.key === "string" ? happened.key : "",
    };
  }

  // Drawing on a <canvas>. `n` is the numbers and `words` the text and
  // the colour, which comes last.
  function draw(canvas, what, n, words) {
    if (!(canvas instanceof HTMLCanvasElement)) return false;
    const pen = canvas.getContext("2d");
    const colour = words[words.length - 1];
    pen.fillStyle = colour;
    pen.strokeStyle = colour;
    if (what === "rect") {
      pen.fillRect(n[0], n[1], n[2], n[3]);
    } else if (what === "circle") {
      pen.beginPath();
      pen.arc(n[0], n[1], n[2], 0, 2 * Math.PI);
      pen.fill();
    } else if (what === "line") {
      pen.beginPath();
      pen.moveTo(n[0], n[1]);
      pen.lineTo(n[2], n[3]);
      pen.stroke();
    } else if (what === "text") {
      // The text is in the canvas's own font, as CSS gave it, and the
      // place is its top left corner, as it is for a rectangle.
      pen.font = getComputedStyle(canvas).font;
      pen.textBaseline = "top";
      pen.fillText(words[0], n[0], n[1]);
    } else {
      pen.clearRect(0, 0, canvas.width, canvas.height);
    }
    return true;
  }

  // What is kept between visits. A browser may refuse to keep anything
  // (a private window, a setting), and then nothing is kept.
  function kept(does) {
    try {
      return does(window.localStorage);
    } catch (refused) {
      return null;
    }
  }

  // What the `web` module asks of the page. Each is named in a request
  // such as ["set_text", 3, "Hello"], and what it returns is the answer.
  const ops = {
    element: (selector) => numberOf(document.querySelector(selector)),
    elements: (selector) => Array.from(document.querySelectorAll(selector), numberOf),
    create: (tag, text) => {
      const element = document.createElement(tag);
      element.textContent = text;
      return numberOf(element);
    },
    text: (id) => elements[id].textContent,
    set_text: (id, text) => { elements[id].textContent = text; },
    value: (id) => String(elements[id].value ?? ""),
    set_value: (id, value) => { elements[id].value = value; },
    get: (id, name) => elements[id].getAttribute(name),
    set: (id, name, value) => { elements[id].setAttribute(name, value); },
    has_class: (id, name) => elements[id].classList.contains(name),
    add_class: (id, name) => { elements[id].classList.add(name); },
    remove_class: (id, name) => { elements[id].classList.remove(name); },
    show: (id) => { elements[id].hidden = false; },
    hide: (id) => { elements[id].hidden = true; },
    is_inside: (inner, outer) => elements[outer].contains(elements[inner]),
    append: (parent, child) => { elements[parent].appendChild(elements[child]); },
    remove: (id) => { elements[id].remove(); },
    clear: (id) => { elements[id].replaceChildren(); },
    name: (id) => nameOf(elements[id]),
    get_style: (id, name) => elements[id].style.getPropertyValue(name) || null,
    set_style: (id, name, value) => {
      // `font-size` is `fontSize` to the browser, which is how it is
      // asked whether there is such a style.
      const asked = name.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
      if (!name.startsWith("--") && !(asked in elements[id].style)) return false;
      elements[id].style.setProperty(name, value);
      return true;
    },
    draw: (id, what, numbers, words) => draw(elements[id], what, numbers, words),
    save: (name, value) => { kept((store) => store.setItem(name, value)); },
    load: (name) => kept((store) => store.getItem(name)),
    forget: (name) => { kept((store) => store.removeItem(name)); },
    on: (id, event, handler) => {
      const element = elements[id];
      element.addEventListener(event, function (happened) {
        // A form that is sent loads the page again, and the program
        // with it. The program is asked what to do instead.
        if (event === "submit") happened.preventDefault();
        call(handler, { event: told(happened, event, element) });
      });
    },
    // A timer is known by the number of its function, which is its
    // own: `stop` is given that number.
    after: (milliseconds, handler) => { timers[handler] = setTimeout(() => call(handler), milliseconds); },
    every: (milliseconds, handler) => { timers[handler] = setInterval(() => call(handler), milliseconds); },
    // The browser keeps one row of numbers for both kinds, so one
    // call stops either. One that has stopped, or been called, is left.
    stop: (handler) => { clearTimeout(timers[handler]); },
    on_key: (handler) => {
      document.addEventListener("keydown", (pressed) => call(handler, { key: pressed.key }));
    },
    // The answer of the network comes later, and the program is called
    // with it then. An address that cannot be reached answers 0.
    fetch: (address, body, handler) => {
      const how = body === null ? {} : { method: "POST", body: body };
      fetch(address, how)
        .then((answered) => answered.text().then((text) => call(handler, { reply: [answered.status, text] })))
        .catch(() => call(handler, { reply: [0, ""] }));
    },
  };

  // What zeno.wasm is given when it starts (see src/web_host.rs).
  const host = {
    zeno_host_print: (place, length) => print(read(place, length)),
    zeno_host_error: (place, length) => error(read(place, length)),
    zeno_host_page: (place, length) => {
      const [op, ...rest] = JSON.parse(read(place, length));
      let result = null;
      try {
        result = ops[op](...rest);
      } catch (refused) {
        // A selector the browser cannot read, for one. The program is
        // told there is nothing there.
        result = null;
      }
      return answer(result);
    },
    zeno_host_input: (place, length) => answer(window.prompt(read(place, length))),
    zeno_host_now: () => performance.now(),
  };

  // `wasm` is the bytes of zeno.wasm, or zeno.wasm as the browser has
  // already read it in (a WebAssembly.Module, which the playground
  // makes once for every program it runs). `program` names the first
  // file and holds the text of every file:
  // {main: "counter.zeno", files: {...}}.
  async function start(wasm, program) {
    const made = await WebAssembly.instantiate(await wasm, { zeno: host });
    zeno = (made.instance || made).exports;
    run(() => zeno.zeno_start(...give(JSON.stringify(program))));
  }

  // The bytes that `zeno build --web` wrote into the page as text.
  function bytes(base64) {
    return Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
  }

  // zeno.wasm is in the page packed, as a .zip file packs (src/deflate.rs),
  // and a browser unpacks that by itself.
  async function unpacked(base64) {
    if (typeof DecompressionStream === "undefined") {
      error("This browser is too old to run this page: it cannot unpack Zeno. One from 2023 or later can.\n");
      throw new Error("no DecompressionStream");
    }
    const packed = new Blob([bytes(base64)]).stream();
    return new Response(packed.pipeThrough(new DecompressionStream("deflate-raw"))).arrayBuffer();
  }

  window.Zeno = { start, bytes, unpacked };
})();
