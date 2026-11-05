// Captures the desk for hotline.dev at 2x, whatever display this machine has,
// and frames each capture as the Mac draws the window.
//
//   node capture.mjs [--room DIR] [--hotline BIN] [--ui URL] [--only NAME,NAME]
//   node capture.mjs frame IMAGE OUT [--title TITLE]
//
// The first form takes every shot in shots.json (`--only` names the ones to retake, by
// their file's name without its extension). It never opens the room it is
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
	// moves its work into the copy and turns its computer off. A teammate whose agent
	// bypasses its permission prompts is shown accepting edits instead: the page shows
	// a mode a visitor would pick, not the one the owner runs the room on.
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
			if (record.modeId === "bypassPermissions") record.modeId = "acceptEdits";
			return `${JSON.stringify(record)}\n`;
		});
	fs.appendFileSync(roomFile, moved.join(""));
	const roster = [...teammates.values()].filter((teammate) => !teammate.deleted).map(({ id, name, avatar }) => ({ id, name, avatarHash: avatar?.hash }));
	return { copy, room, roster };
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
function standInShell({ origin, token, voice }) {
	let next = 0;
	const desks = [{ id: "local", name: "This computer", kind: "local", origin, token }];
	window.__hotlineDesks = desks;
	window.__hotlineDesk = { platform: "macos", origin, token };
	window.__TAURI_INTERNALS__ = {
		metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
		invoke: async (command) => {
			if (command === "desk_list") return desks;
			if (command === "speech_capability") return { available: voice };
			if (String(command).startsWith("plugin:event|listen")) return next++;
			return null;
		},
		transformCallback: () => next++,
		unregisterCallback: () => {},
		convertFileSrc: (file) => file,
	};
	window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
}

async function shoot(browser, ui, door, roster, shot) {
	const [width, height] = shot.window;
	const context = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 2, colorScheme: "dark" });
	await context.addInitScript(standInShell, { origin: `http://127.0.0.1:${door.port}`, token: door.token, voice: shot.voice === true });
	const page = await context.newPage();
	// The window's socket runs through here when a shot needs the room shown as it is rather than
	// as the copy has it. `voice` shows the controls a desk with a voice provider shows (the mic,
	// the call key beside it, the call key in the rail): the copy has no provider keys, so the shell
	// says this Mac hears speech and the desk's answer to voice.status is given here. `computer`
	// shows each teammate's computer switched on as the room has it, where the copy turned it off.
	if (shot.voice || shot.computer) {
		await page.routeWebSocket(`ws://127.0.0.1:${door.port}/**`, (socket) => {
			const desk = socket.connectToServer();
			socket.onMessage((message) => {
				const frame = JSON.parse(String(message));
				if (!shot.voice || frame.cmd !== "voice.status") return desk.send(message);
				socket.send(JSON.stringify({ id: frame.id, ok: true, result: { available: true, directAvailable: true, capabilities: ["voiceDirectCalls"] } }));
			});
			desk.onMessage((message) => {
				socket.send(shot.computer ? String(message).replace(/("computer":\{[^{}]*?"enabled":)false/g, "$1true") : message);
			});
		});
	}
	await page.goto(ui);
	// The rail names each teammate in its key's label ("Goldie, team lead", "Kit, unread"), which
	// holds at a width where the rail is avatars alone.
	await page.locator(`[aria-label="${shot.teammate}"], [aria-label^="${shot.teammate}, "]`).first().click();
	// The tape folds, then the session's model arrives: its key reads "Goldie: Opus · High" once
	// the agent has said what it runs, and only the bare model id before that.
	await page.waitForTimeout(3000);
	// `call` shows the teammate on a live call: the window's own Call, started against the dev
	// server's source with its start stubbed (the copy has no voice provider), set to the phase
	// named ("speaking", "listening"), its clock at `seconds` of talk, and its loudness driven
	// so the voice bars and the face's ring are mid-swell when the shot is taken.
	if (shot.call) {
		const teammate = roster.find((entry) => entry.name === shot.teammate);
		const names = Object.fromEntries(roster.map((entry) => [entry.id, entry.name]));
		await page.evaluate(async ({ teammate, names, phase, seconds }) => {
			const voice = await import("/src/voice/call.ts");
			voice.Call.prototype.start = async () => {};
			const call = await voice.startCall((id) => names[id], { personaId: teammate.id, name: teammate.name, avatarHash: teammate.avatarHash });
			call.set({ phase, clock: { base: seconds * 1000, since: null } });
			let t = 0;
			setInterval(() => {
				t += 1 / 60;
				const level = 0.45 + 0.3 * Math.sin(t * 9) + 0.2 * Math.sin(t * 23);
				for (const listener of call.levels) listener(Math.max(0, Math.min(1, level)));
			}, 16);
		}, { teammate, names, phase: shot.call.phase ?? "speaking", seconds: shot.call.seconds ?? 0 });
		await page.waitForTimeout(1500);
	}
	await page
		.locator(`button[aria-label^="${shot.teammate}: "][aria-label*="·"]`)
		.first()
		.waitFor({ timeout: 30_000 })
		.catch(() => console.log(`${shot.teammate}'s model had not arrived; the capture shows its id.`));
	// The tape grows as what it shows arrives (a picture at its end loads once it is near), so the
	// scroll is set twice: the second time against the tape's settled height.
	for (let pass = 0; pass < 2; pass++) {
		await page.evaluate((fromEnd) => {
			const tapes = [...document.querySelectorAll(".scroller")].sort((a, b) => b.scrollHeight - a.scrollHeight);
			tapes[0].scrollTop = tapes[0].scrollHeight - tapes[0].clientHeight - fromEnd;
		}, shot.fromEnd);
		await page.waitForTimeout(800);
	}
	// `top` names a line of the tape by its text and puts it at the top of the window (`above`
	// keeps that many CSS pixels of the tape above it), for a window whose width changes how far
	// from the end a moment sits. The tape draws its past as it is scrolled back to, so the
	// search climbs a few screens at a time until the line is there.
	if (shot.top) {
		const seat = ({ text, above }) => {
			const tapes = [...document.querySelectorAll(".scroller")].sort((a, b) => b.scrollHeight - a.scrollHeight);
			const tape = tapes[0];
			const line = [...tape.querySelectorAll("p, li, div")].filter((el) => el.textContent.includes(text)).pop();
			if (!line) {
				if (tape.scrollTop === 0) return "top";
				tape.scrollTop = Math.max(0, tape.scrollTop - tape.clientHeight * 3);
				return "climbing";
			}
			tape.scrollTop += line.getBoundingClientRect().top - tape.getBoundingClientRect().top - 12 - above;
			return "seated";
		};
		let state = "climbing";
		for (let climb = 0; climb < 200 && state === "climbing"; climb++) {
			state = await page.evaluate(seat, { text: shot.top, above: shot.above ?? 0 });
			await page.waitForTimeout(state === "seated" ? 800 : 250);
		}
		if (state !== "seated") throw new Error(`No line of the tape reads "${shot.top}"`);
	}
	// `rewrite` shortens a line of the tape for the page: each entry's `find` is replaced in the
	// tape's text by its `replace`. It is for the owner's own words, when a shot shows a message
	// that ran on past the point it makes; a teammate's words are shown as said.
	for (const { find, replace } of shot.rewrite ?? []) {
		const hits = await page.evaluate(({ find, replace }) => {
			const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
			let count = 0;
			for (let node = walker.nextNode(); node; node = walker.nextNode()) {
				if (!node.nodeValue.includes(find)) continue;
				node.nodeValue = node.nodeValue.replace(find, replace);
				count++;
			}
			return count;
		}, { find, replace });
		if (hits === 0) throw new Error(`Nothing on the tape reads "${find}"`);
	}
	if (shot.open) {
		await page.getByText(shot.open, { exact: false }).first().click();
		await page.waitForTimeout(2500);
	}
	// Then whatever the shot opens, in order: a menu, an entry in it, a panel.
	for (const step of shot.clicks ?? []) {
		await (step.text ? page.getByText(step.text, { exact: true }) : page.locator(step.selector)).first().click();
		await page.waitForTimeout(1500);
	}
	// It shows only because the tape is scrolled back; the mouse goes where it hovers nothing. A shot
	// may hide more by name: a key whose content the copy cannot load, such as a model whose
	// provider key stayed in the room's vault.
	// The copy may be setting a computer up while the shot is taken, a teammate's side thread is a
	// story of its own, a screen the tape's past links to is not on, and the plan gauge is how much
	// of the owner's subscription this week has used: none of those keys is the page's.
	const hidden = ['button[aria-label="Jump to the latest"]', '[role="progressbar"]', '[aria-label^="Side thread:"]', '[aria-label="Open the teammate\'s computer"]', ".plan-gauge", ...(shot.hide ?? [])];
	await page.addStyleTag({ content: `${hidden.join(", ")} { display: none !important; }` });
	await page.mouse.move(width / 2, height - 20);
	await page.waitForTimeout(400);
	// A piece of the window, such as a menu, is cut to the element and left unframed.
	let clip;
	if (shot.clip) {
		const box = await page.locator(shot.clip.selector).first().boundingBox();
		clip = { x: box.x + (shot.clip.left ?? 0), y: box.y + (shot.clip.top ?? 0), width: shot.clip.width ?? box.width, height: shot.clip.height ?? box.height };
	}
	const capture = await page.screenshot(clip ? { clip } : {});
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

		const { cut, keys = [], shots } = JSON.parse(fs.readFileSync(path.join(HERE, "shots.json"), "utf8"));
		const ui = named.ui ?? "http://localhost:5174/";
		await fetch(ui).catch(() => {
			throw new Error(`Nothing serves the window at ${ui}: run \`make dev\` (or \`bun run dev\` in ui/), or pass --ui.`);
		});
		const roomSource = path.resolve(named.room ?? path.join(REPO, ".hotline-dev"));
		const { copy, room, roster } = copyRoom(roomSource, Date.parse(cut));
		const hotline = path.resolve(named.hotline ?? path.join(REPO, "target/debug/hotline"));
		const { desk, door } = await serve(hotline, room);
		try {
			// A provider named in `keys` gets a placeholder key in the copy, so the window names its
			// models from the bundled catalogue (a teammate's chip reads "Grok 4.7 · High") where the
			// room's real key stayed in its vault. Nothing runs on it: no session is started here.
			for (const providerId of keys) {
				execFileSync(hotline, ["wire", "credential.create", "--data", room], { input: JSON.stringify({ providerId, label: providerId, secret: "placeholder" }) });
			}
			const only = named.only ? new Set(named.only.split(",")) : null;
			for (const shot of shots) {
				const out = path.resolve(HERE, shot.out);
				if (only && !only.has(path.basename(out, ".webp"))) continue;
				const framed =
					shot.kind === "browser"
						? await frame(browser, await shootBrowser(browser, ui, roomSource, shot), 1280, `${shot.teammate}'s browser`, 2)
						: shot.clip
							? await shoot(browser, ui, door, roster, shot)
							: await frame(browser, [{ png: await shoot(browser, ui, door, roster, shot), height: shot.window[1] }], shot.window[0], "", 2);
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
