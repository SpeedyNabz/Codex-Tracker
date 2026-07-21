import { chromium } from "playwright";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const outputDirectory = new URL("../src-tauri/installer/", import.meta.url);
const assetDirectory = new URL("../src/assets/", import.meta.url);
const browser = await chromium.launch({ headless: true });

try {
  const hmRight = `data:image/svg+xml;base64,${Buffer.from(
    await readFile(new URL("heavymask-hm-right.svg", assetDirectory), "utf8"),
  ).toString("base64")}`;
  const sammy = `data:image/svg+xml;base64,${Buffer.from(
    await readFile(new URL("heavymask-sammy.svg", assetDirectory), "utf8"),
  ).toString("base64")}`;

  for (const name of ["header", "sidebar"]) {
    const svg = (await readFile(new URL(`../src-tauri/installer/${name}.svg`, import.meta.url), "utf8"))
      .replaceAll("__HM_RIGHT_ASSET__", hmRight)
      .replaceAll("__SAMMY_ASSET__", sammy);
    const page = await browser.newPage({ viewport: { width: name === "header" ? 150 : 164, height: name === "header" ? 57 : 314 } });
    await page.setContent(svg);
    await page.locator("svg").screenshot({ path: fileURLToPath(new URL(`${name}.png`, outputDirectory)) });
    await page.close();
  }
} finally {
  await browser.close();
}
