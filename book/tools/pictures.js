// Makes the pictures in book/images: pages that `zeno build --web`
// built, as a browser shows them. Run it from the top of the
// repository, with `zeno` (and zeno.wasm) built and Playwright at hand:
//
//     node book/tools/pictures.js
//
// The book is built from the pictures that are kept in git, so this is
// only run again when a page changes.
const { chromium } = require("playwright");
const { execFileSync } = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const repo = path.resolve(__dirname, "..", "..");
const images = path.join(repo, "book", "images");

async function todo(browser) {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "zeno-todo-"));
  for (const name of ["todo.zeno", "todo.html"]) {
    fs.copyFileSync(path.join(repo, "examples", name), path.join(work, name));
  }
  execFileSync("zeno", ["build", "--web", "todo.zeno"], { cwd: work });
  const page = await browser.newPage({ viewport: { width: 640, height: 470 }, deviceScaleFactor: 2 });
  await page.goto("file://" + path.join(work, "todo.web.html"));
  await page.waitForSelector("#list li");
  for (const what of ["Solder the header", "Flash the board"]) {
    await page.fill("#what", what);
    await page.press("#what", "Enter");
  }
  await page.click("#list li >> nth=0");
  await page.waitForTimeout(200);
  await page.screenshot({ path: path.join(images, "todo.png") });
  await page.close();
}

function built(name) {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "zeno-" + name + "-"));
  for (const file of [name + ".zeno", name + ".html"]) {
    fs.copyFileSync(path.join(repo, "examples", file), path.join(work, file));
  }
  execFileSync("zeno", ["build", "--web", name + ".zeno"], { cwd: work });
  return "file://" + path.join(work, name + ".web.html");
}

async function sketch(browser) {
  const page = await browser.newPage({ viewport: { width: 760, height: 560 }, deviceScaleFactor: 2 });
  await page.goto(built("sketch"));
  await page.waitForTimeout(800);
  const paper = await page.$("#paper");
  const box = await paper.boundingBox();
  // A wave in the first colour, then a box in the second.
  await page.mouse.move(box.x + 40, box.y + 120);
  await page.mouse.down();
  for (let i = 0; i <= 60; i++) {
    await page.mouse.move(box.x + 40 + i * 7, box.y + 120 + 50 * Math.sin(i / 6));
  }
  await page.mouse.up();
  await page.click(".colour >> nth=1");
  const corners = [[120, 220], [360, 220], [360, 300], [120, 300], [120, 220]];
  await page.mouse.move(box.x + corners[0][0], box.y + corners[0][1]);
  await page.mouse.down();
  for (const [x, y] of corners.slice(1)) {
    await page.mouse.move(box.x + x, box.y + y, { steps: 8 });
  }
  await page.mouse.up();
  await page.waitForTimeout(200);
  await page.screenshot({ path: path.join(images, "sketch.png") });
  await page.close();
}

async function playground(browser) {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "zeno-playground-"));
  execFileSync("zeno", ["playground"], { cwd: work });
  const page = await browser.newPage({ viewport: { width: 1100, height: 520 }, deviceScaleFactor: 2 });
  await page.goto("file://" + path.join(work, "playground.html"));
  await page.waitForTimeout(1500);
  // The list of things to do, run, with one thing added in the frame.
  const select = await page.$("select");
  const options = await select.$$eval("option", (all) => all.map((o) => [o.value, o.textContent]));
  const todo = options.find(([, text]) => /things to do/i.test(text));
  if (todo) await select.selectOption(todo[0]);
  await page.click("text=Run");
  await page.waitForTimeout(1500);
  const frame = page.frames().find((f) => f !== page.mainFrame());
  if (frame) {
    await frame.fill("#what", "Try the playground");
    await frame.press("#what", "Enter");
    await page.waitForTimeout(300);
  }
  await page.screenshot({ path: path.join(images, "playground.png") });
  await page.close();
}

(async () => {
  fs.mkdirSync(images, { recursive: true });
  const browser = await chromium.launch();
  await todo(browser);
  await sketch(browser);
  await playground(browser);
  await browser.close();
  console.log("pictures: " + images);
})();
