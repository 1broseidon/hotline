import { chromium } from "../../ui-climb/tools/node_modules/playwright/index.mjs";
import { fileURLToPath, pathToFileURL } from "node:url";

// Shoots each scene of mock.html into shots/. Run: bun shoot.mjs (or node).
const here = fileURLToPath(new URL("./", import.meta.url));
const browser = await chromium.launch();
try {
	for (const s of ["today-docx", "today-html", "html", "html-source", "csv", "rows"]) {
		const ctx = await browser.newContext({ viewport: { width: 960, height: 700 }, deviceScaleFactor: 2, colorScheme: "dark" });
		const page = await ctx.newPage();
		await page.goto(`${pathToFileURL(`${here}mock.html`).href}?s=${s}`);
		await page.evaluate(() => document.fonts.ready);
		// The rows scene shows one row as the pointer finds it, so its keys are drawn.
		if (s === "rows") await page.locator(".file-line[data-hover]").hover();
		await page.waitForTimeout(250);
		await page.locator("#root").screenshot({ path: `${here}shots/${s}.png` });
		await ctx.close();
	}
} finally {
	await browser.close();
}
