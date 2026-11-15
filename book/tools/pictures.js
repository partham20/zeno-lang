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

(async () => {
  fs.mkdirSync(images, { recursive: true });
  const browser = await chromium.launch();
  await todo(browser);
  await browser.close();
  console.log("pictures: " + images);
})();
