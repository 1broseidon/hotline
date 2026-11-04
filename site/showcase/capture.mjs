// Captures the desk for hotline.dev at 2x, whatever display this machine has,
// and frames each capture as the Mac draws the window.
//
//   node capture.mjs [--room DIR] [--hotline BIN] [--ui URL]
//   node capture.mjs frame IMAGE OUT [--title TITLE]
//
// The first form takes every shot in shots.json. It never opens the room it is
// given: it copies the room's streams to a temporary directory, cuts them at
// shots.json's `cut` (whatever happened in the room after the story it shows),
// and serves the copy with `hotline serve`. In the copy every teammate works in
// the temporary directory and none has a computer, because a teammate's
// container is named for the teammate, and the copy's would be the room's own.
// The window is the one `make dev` (or `bun run dev` in ui/) serves, opened in
// headless Chromium with a stand-in for the Tauri shell, at a device scale of 2.
//
// The second form frames an image that already exists, such as a still of a
// teammate's computer, in a window with a title bar.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { chromium } from "playwright-core";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "../..");

// What a copy of the room keeps: the roster and settings, the tapes, the
// threads, and what they show. Not the vault, the workspaces, the jobs (a
// scheduled job would run) or the search index (the copy builds its own).
const STREAMS = ["room.jsonl", "transcripts", "threads", "asides", "sides", "files", "avatars", "runs", "tool-output"];
// The folders whose events are cut at `cut`; the room stream keeps its roster.
const CUT = ["transcripts", "threads", "asides", "sides"];

function options(argv) {
	const named = {};
	const plain = [];
	for (let i = 0; i < argv.length; i++) {
		if (argv[i].startsWith("--")) named[argv[i].slice(2)] = argv[++i];
		else plain.push(argv[i]);
	}
	return { named, plain };
}

function freePort() {
	return new Promise((resolve, reject) => {
		const server = net.createServer();
		server.on("error", reject);
		server.listen(0, "127.0.0.1", () => {
			const { port } = server.address();
			server.close(() => resolve(port));
		});
	});
}

function eventLines(file) {
	return fs.readFileSync(file, "utf8").split("\n").filter((line) => line.trim() !== "");
}

function copyRoom(source, cutAt) {
	if (!fs.existsSync(path.join(source, "room.jsonl"))) throw new Error(`${source} has no room.jsonl; pass the room with --room.`);
	const copy = fs.mkdtempSync(path.join(os.tmpdir(), "hotline-showcase-"));
	const room = path.join(copy, "room");
	const work = path.join(copy, "work");
	fs.mkdirSync(room);
	fs.mkdirSync(work);
	for (const name of STREAMS) {
		const from = path.join(source, name);
		if (fs.existsSync(from)) fs.cpSync(from, path.join(room, name), { recursive: true });
	}
	// A room records its secret store; the copy has none, so it is a file room.
	fs.writeFileSync(path.join(room, "store.json"), JSON.stringify({ backend: "file" }));

	for (const folder of CUT) {
		const dir = path.join(room, folder);
		if (!fs.existsSync(dir)) continue;
		for (const name of fs.readdirSync(dir, { recursive: true })) {
			const file = path.join(dir, name);
			if (!file.endsWith(".jsonl")) continue;
			const kept = eventLines(file).filter((line) => {
				const ts = JSON.parse(line).ts;
				return typeof ts !== "number" || ts < cutAt;
			});
			fs.writeFileSync(file, kept.map((line) => `${line}\n`).join(""));
		}
	}

	// The last record of each teammate wins the fold, so one more record each
	// moves its work into the copy and turns its computer off.
	const roomFile = path.join(room, "room.jsonl");
	const teammates = new Map();
	for (const line of eventLines(roomFile)) {
		const event = JSON.parse(line);
		if (event.kind === "persona") teammates.set(event.id, event);
	}
	const moved = [...teammates.values()]
		.filter((teammate) => !teammate.deleted)
		.map((teammate) => {
			const record = { ...teammate, cwd: work };
			if (record.computer) record.computer = { ...record.computer, enabled: false };
			return `${JSON.stringify(record)}\n`;
		});
	fs.appendFileSync(roomFile, moved.join(""));
	return { copy, room };
}

async function serve(hotline, room) {
	if (!fs.existsSync(hotline)) throw new Error(`No ${hotline}: build it with \`cargo build -p hotline-cli\`, or pass --hotline.`);
	const port = await freePort();
	const log = fs.openSync(path.join(room, "..", "serve.log"), "w");
	const desk = spawn(
		hotline,
		["serve", "--store", "file", "--listen", `127.0.0.1:${port}`, "--public-url", `https://127.0.0.1:${port}`, "--tls", "self", "--capabilities", "agents", "--data", room],
		{ stdio: ["ignore", log, log] },
	);
	const doorFile = path.join(room, "door.json");
	for (let waited = 0; waited < 30_000; waited += 250) {
		if (desk.exitCode !== null) break;
		if (fs.existsSync(doorFile)) {
			try {
				return { desk, door: JSON.parse(fs.readFileSync(doorFile, "utf8")) };
			} catch {
				// The core is still writing it.
			}
		}
		await new Promise((resolve) => setTimeout(resolve, 250));
	}
	desk.kill("SIGTERM");
	throw new Error(`The copy's desk did not open; its log is ${path.join(room, "..", "serve.log")}.`);
}

function stop(desk) {
	if (desk.exitCode !== null) return Promise.resolve();
	return new Promise((resolve) => {
		desk.once("exit", resolve);
		desk.kill("SIGTERM");
	});
}

// The window asks the shell for its desks and a handful of window facts. It
// is told it runs on a Mac, so its title strip leaves the lights their room.
function standInShell({ origin, token }) {
	let next = 0;
	const desks = [{ id: "local", name: "This computer", kind: "local", origin, token }];
	window.__hotlineDesks = desks;
	window.__hotlineDesk = { platform: "macos", origin, token };
	window.__TAURI_INTERNALS__ = {
		metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
		invoke: async (command) => {
			if (command === "desk_list") return desks;
			if (String(command).startsWith("plugin:event|listen")) return next++;
			return null;
		},
		transformCallback: () => next++,
		unregisterCallback: () => {},
		convertFileSrc: (file) => file,
	};
	window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
}

async function shoot(browser, ui, door, shot) {
	const [width, height] = shot.window;
	const context = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 2, colorScheme: "dark" });
	await context.addInitScript(standInShell, { origin: `http://127.0.0.1:${door.port}`, token: door.token });
	const page = await context.newPage();
	await page.goto(ui);
	await page.getByText(shot.teammate, { exact: true }).first().click();
	// Long enough for the tape to fold and the session's model to arrive.
	await page.waitForTimeout(3000);
	await page.evaluate((fromEnd) => {
		const tapes = [...document.querySelectorAll(".scroller")].sort((a, b) => b.scrollHeight - a.scrollHeight);
		tapes[0].scrollTop = tapes[0].scrollHeight - tapes[0].clientHeight - fromEnd;
	}, shot.fromEnd);
	await page.waitForTimeout(800);
	if (shot.open) {
		await page.getByText(shot.open, { exact: false }).first().click();
		await page.waitForTimeout(2500);
	}
	// It shows only because the tape is scrolled back; the mouse goes where it hovers nothing.
	await page.addStyleTag({ content: 'button[aria-label="Jump to the latest"] { visibility: hidden !important; }' });
	await page.mouse.move(width / 2, height - 20);
	await page.waitForTimeout(400);
	const capture = await page.screenshot();
	await context.close();
	return capture;
}

// A teammate's desk browser as its pane shows it: Hotline's bar (the window's
// own browser.html, told whose browser it is and where) over the page, 1280 by
// 800 as the desk browser opens it. The page is a file in the room, a site a
// teammate made, opened where it lies.
async function shootBrowser(browser, ui, roomSource, shot) {
	const state = { name: shot.teammate, url: shot.url, title: shot.title, taken: false };
	const bar = await browser.newContext({ viewport: { width: 1280, height: 44 }, deviceScaleFactor: 2, colorScheme: "dark" });
	await bar.addInitScript((answer) => {
		window.__TAURI_INTERNALS__ = {
			metadata: { currentWindow: { label: "bar" }, currentWebview: { windowLabel: "bar", label: "bar" } },
			invoke: async (command) => (command === "browser_bar_state" ? answer : null),
			transformCallback: () => 0,
		};
	}, state);
	const barPage = await bar.newPage();
	await barPage.goto(new URL("browser.html", ui).href);
	await barPage.waitForTimeout(1500);
	const barPng = await barPage.screenshot();
	await bar.close();

	const site = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2 });
	const sitePage = await site.newPage();
	await sitePage.goto(pathToFileURL(path.join(roomSource, shot.page)).href);
	await sitePage.waitForTimeout(1000);
	const sitePng = await sitePage.screenshot();
	await site.close();
	return [
		{ png: barPng, height: 44 },
		{ png: sitePng, height: 800 },
	];
}

// A Mac window around captures stacked top to bottom: rounded corners, a
// hairline edge and the traffic lights. Without a title the lights sit on the
// page's own 40px strip, 12px in, where Hotline's main window seats them
// (crates/hotline-app/src/lights.rs); with one they sit on macOS's own 28px
// title bar, dark as Hotline sets it (crates/hotline-app/src/browser.rs).
async function frame(browser, parts, width, title, scale) {
	const bar = title ? 28 : 0;
	const height = bar + parts.reduce((sum, part) => sum + part.height, 0);
	const centre = title ? 14 : 20;
	const lights = [
		["#ff5f57", "#e0443e"],
		["#febc2e", "#d89e24"],
		["#28c840", "#1aab29"],
	]
		.map(([fill, edge], i) => `<i style="position:absolute;left:${13 + i * 20}px;top:${centre - 6}px;width:12px;height:12px;border-radius:50%;background:${fill};box-shadow:inset 0 0 0 .5px ${edge}"></i>`)
		.join("");
	const titleBar = title
		? `<div style="height:${bar}px;background:#1c1c1f;display:grid;place-items:center;font:600 13px -apple-system,system-ui,sans-serif;color:#a1a1a6">${title}</div>`
		: "";
	const images = parts
		.map((part) => `<img src="data:image/png;base64,${part.png.toString("base64")}" style="display:block;width:${width}px;height:${part.height}px">`)
		.join("");
	const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: scale });
	await page.setContent(`<body style="margin:0;background:transparent">
<div style="position:relative;width:${width}px;height:${height}px;border-radius:12px;overflow:hidden;background:#0d0e11">
${titleBar}${images}${lights}
<div style="position:absolute;inset:0;border-radius:12px;box-shadow:inset 0 0 0 1px rgba(255,255,255,.13)"></div>
</div></body>`);
	const framed = await page.screenshot({ omitBackground: true, clip: { x: 0, y: 0, width, height } });
	await page.close();
	return framed;
}

// The page wants WebP; anything else is written as the PNG it is.
function write(png, out, width) {
	fs.mkdirSync(path.dirname(out), { recursive: true });
	if (!out.endsWith(".webp")) {
		fs.writeFileSync(out, png);
		return;
	}
	const temporary = path.join(os.tmpdir(), `hotline-showcase-${process.pid}.png`);
	fs.writeFileSync(temporary, png);
	const resize = width ? ["-resize", String(width), "0"] : [];
	execFileSync("cwebp", ["-quiet", "-q", "82", ...resize, temporary, "-o", out]);
	fs.rmSync(temporary);
}

function imageSize(file) {
	const bytes = fs.readFileSync(file);
	// A PNG's width and height are the IHDR's first two big-endian words.
	if (bytes.toString("ascii", 1, 4) === "PNG") return { png: bytes, width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) };
	throw new Error(`${file} is not a PNG; convert it first (dwebp in.webp -o in.png).`);
}

async function main() {
	const { named, plain } = options(process.argv.slice(2));
	const browser = await chromium.launch().catch((error) => {
		throw new Error(`No Chromium for Playwright (${error.message.split("\n")[0]}); run \`bunx playwright-core install chromium\`.`);
	});
	try {
		if (plain[0] === "frame") {
			const [, image, out] = plain;
			const { png, width, height } = imageSize(image);
			write(await frame(browser, [{ png, height }], width, named.title ?? "", 1), path.resolve(out));
			console.log(`framed ${out}`);
			return;
		}

		const { cut, shots } = JSON.parse(fs.readFileSync(path.join(HERE, "shots.json"), "utf8"));
		const ui = named.ui ?? "http://localhost:5174/";
		await fetch(ui).catch(() => {
			throw new Error(`Nothing serves the window at ${ui}: run \`make dev\` (or \`bun run dev\` in ui/), or pass --ui.`);
		});
		const roomSource = path.resolve(named.room ?? path.join(REPO, ".hotline-dev"));
		const { copy, room } = copyRoom(roomSource, Date.parse(cut));
		const { desk, door } = await serve(path.resolve(named.hotline ?? path.join(REPO, "target/debug/hotline")), room);
		try {
			for (const shot of shots) {
				const out = path.resolve(HERE, shot.out);
				const framed =
					shot.kind === "browser"
						? await frame(browser, await shootBrowser(browser, ui, roomSource, shot), 1280, `${shot.teammate}'s browser`, 2)
						: await frame(browser, [{ png: await shoot(browser, ui, door, shot), height: shot.window[1] }], shot.window[0], "", 2);
				write(framed, out, shot.width);
				console.log(`captured ${path.relative(REPO, out)}`);
			}
		} finally {
			await stop(desk);
			fs.rmSync(copy, { recursive: true, force: true });
		}
	} finally {
		await browser.close();
	}
}

main().catch((error) => {
	console.error(error.message);
	process.exit(1);
});
