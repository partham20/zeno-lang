// Open a page that `zeno build --target html` wrote, in a browser with no
// window, and print what the program printed in it. It ends with status 1
// when the program stopped with an error, as `zeno run` does.
//
//     node tools/run-page.js hello.html
//
// tools/targets.py uses it to hold `--target html` to `zeno run`. It needs
// Playwright and its Chromium (`npm install playwright`, then
// `npx playwright install chromium`).

const path = require("path");
const { chromium } = require("playwright");

(async () => {
  const file = path.resolve(process.argv[2]);
  const browser = await chromium.launch();
  try {
    const page = await browser.newPage();
    const problems = [];
    page.on("pageerror", (error) => problems.push(String(error)));
    await page.goto("file://" + file);
    await page.waitForLoadState("load");
    const shown = await page.evaluate(() => {
      const output = document.getElementById("zeno-output");
      return { text: output ? output.textContent : "", error: globalThis.zenoError || null };
    });
    let text = shown.text;
    if (shown.error !== null) {
      // The error is shown in the page under the output; `zeno run` says
      // it on stderr.
      const at = text.lastIndexOf(shown.error);
      if (at >= 0) text = text.slice(0, at);
      process.stderr.write(shown.error + "\n");
    }
    process.stdout.write(text);
    for (const problem of problems) process.stderr.write(problem + "\n");
    process.exitCode = shown.error !== null || problems.length > 0 ? 1 : 0;
  } finally {
    await browser.close();
  }
})();
