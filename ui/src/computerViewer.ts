import { invoke } from "@tauri-apps/api/core";

/**
 * A teammate's computer on a desk on a server (BRO-145), in a window of its
 * own. On this computer's desk the window opens the container's own viewer
 * page; a server's container is not reachable from here, so this page
 * speaks the same protocol through the desk's bridge instead, which carries
 * the frames sealed and keeps the container's bearer on the server.
 *
 * The protocol is the container's: binary frames are a 12-byte header of
 * little-endian u16s (x, y, width, height, screen width, screen height)
 * followed by a PNG of that rectangle; text frames are JSON (`state`,
 * `pointer`, `paste`, `error`). The page sends JSON: `control`, `move`,
 * `button`, `wheel`, `key` and `paste`. It opens watching, and nothing it
 * does reaches the desktop until the person takes the screen.
 *
 * Files go by the desk rather than the container: `{"type":"files", id, op}`
 * requests (list, download, upload_start/chunk/finish/cancel) are answered
 * by the desk with `{"type":"files", id, ok, result|error}`, in chunks of at
 * most 512 KiB, and never reach the container's screen socket.
 *
 * The window is opened at computer.html#origin=…&token=…&persona=…&name=…
 * (computer.ts).
 */

const params = new URLSearchParams(location.hash.slice(1));
const origin = params.get("origin") ?? "";
const token = params.get("token") ?? "";
const persona = params.get("persona") ?? "";
document.title = `${params.get("name") ?? "A teammate"}'s computer`;

const element = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const canvas = element<HTMLCanvasElement>("screen");
const context = canvas.getContext("2d") as CanvasRenderingContext2D;
const bar = element("bar");
const status = element("status");
const sub = element("sub");
const control = element<HTMLButtonElement>("control");
const paste = element<HTMLButtonElement>("paste");
const back = element<HTMLButtonElement>("back");
const browse = element<HTMLButtonElement>("browse");
const hint = element("hint");
const toast = element("toast");
const pointer = element("pointer");
const mac = /Mac|iPhone|iPad/.test(navigator.platform);

type Machine = { holder: "none" | "agent" | "person"; running: number; completed: number; failed: number };

let socket: WebSocket | null = null;
let drawing: Promise<void> = Promise.resolve();
let connected = false;
let driving = false;
let machine: Machine = { holder: "none", running: 0, completed: 0, failed: 0 };
let quiet: ReturnType<typeof setTimeout> | undefined;
let hands: { x: number; y: number } | null = null;
let handsIdle: ReturnType<typeof setTimeout> | undefined;

function placePointer(): [number, number] | undefined {
	if (!hands) return undefined;
	const rect = canvas.getBoundingClientRect();
	const px = rect.left + (hands.x * rect.width) / canvas.width;
	const py = rect.top + (hands.y * rect.height) / canvas.height;
	pointer.style.transform = `translate(${px.toFixed(1)}px, ${py.toFixed(1)}px)`;
	return [px, py];
}

function showHands(message: { x: number; y: number; button?: number; down?: boolean }) {
	hands = { x: message.x, y: message.y };
	const at = placePointer();
	// A person driving has their own pointer over the screen already.
	pointer.classList.toggle("shown", !driving);
	if (message.button && message.down && !driving && at) {
		const ring = document.createElement("div");
		ring.className = message.button === 3 ? "ring right" : "ring";
		ring.style.left = `${at[0]}px`;
		ring.style.top = `${at[1]}px`;
		document.body.appendChild(ring);
		ring.addEventListener("animationend", () => ring.remove());
	}
	clearTimeout(handsIdle);
	handsIdle = setTimeout(() => pointer.classList.remove("shown"), 4000);
}

function say(text: string, fade = true) {
	toast.textContent = text;
	toast.classList.remove("quiet");
	clearTimeout(quiet);
	if (fade) quiet = setTimeout(() => toast.classList.add("quiet"), 1200);
}

function render() {
	const held = machine.holder === "person" && !driving;
	if (driving) pointer.classList.remove("shown");
	bar.className = !connected ? "lost" : driving ? "driving" : held ? "held" : "";
	control.hidden = !connected || driving;
	control.disabled = held;
	paste.hidden = back.hidden = hint.hidden = !(connected && driving);
	browse.hidden = !connected;
	if (!connected) {
		status.textContent = "Reconnecting…";
		sub.textContent = "";
		return;
	}
	if (driving) {
		status.textContent = "You’re in control";
		sub.textContent = "";
		hint.textContent = `${mac ? "⌥⌘V" : "Ctrl+Alt+V"} pastes from your computer`;
		return;
	}
	if (held) {
		status.textContent = "Someone else has the screen";
		sub.textContent = "";
		return;
	}
	status.textContent = "Watching";
	const { running, completed, failed } = machine;
	const jobs = failed
		? `${failed} failed · ${running} running`
		: running
			? `${running} running · ${completed} done`
			: completed
				? `${completed} done`
				: "no jobs";
	sub.textContent = `· ${machine.holder === "agent" ? "agent in control" : "agent at work"} · ${jobs}`;
}

function fit() {
	const scale = Math.min(innerWidth / canvas.width, innerHeight / canvas.height);
	canvas.style.width = `${Math.floor(canvas.width * scale)}px`;
	canvas.style.height = `${Math.floor(canvas.height * scale)}px`;
}
addEventListener("resize", () => {
	fit();
	placePointer();
});

function connect() {
	const base = origin.replace(/^http/, "ws");
	socket = new WebSocket(`${base}/computer/${encodeURIComponent(persona)}/ws?token=${encodeURIComponent(token)}`);
	socket.binaryType = "arraybuffer";
	// A fresh socket is a machine that has not been told who is driving.
	socket.onopen = () => {
		connected = true;
		render();
		if (driving) send({ t: "control", take: true });
	};
	socket.onclose = () => {
		for (const waiting of asked.values()) waiting.reject(new Error("The connection to the computer dropped."));
		asked.clear();
		connected = false;
		driving = false;
		render();
		setTimeout(connect, 1000);
	};
	socket.onmessage = (event) => {
		if (typeof event.data === "string") {
			const message = JSON.parse(event.data);
			if (message.type === "files") {
				answered(message);
				return;
			}
			if (message.t === "state") {
				machine = message;
				render();
			}
			if (message.t === "pointer") showHands(message);
			if (message.t === "paste") say("Pasted");
			if (message.t === "error") {
				say(message.error, false);
				if (!message.driving) {
					driving = false;
					render();
				}
			}
			return;
		}
		draw(event.data as ArrayBuffer);
	};
}

function draw(data: ArrayBuffer) {
	if (data.byteLength < 12) return;
	const view = new DataView(data);
	const [x, y, , , sw, sh] = [0, 2, 4, 6, 8, 10].map((offset) => view.getUint16(offset, true)) as [
		number,
		number,
		number,
		number,
		number,
		number,
	];
	if (canvas.width !== sw || canvas.height !== sh) {
		canvas.width = sw;
		canvas.height = sh;
		fit();
	}
	const blob = new Blob([data.slice(12)], { type: "image/png" });
	drawing = drawing
		.then(() => createImageBitmap(blob))
		.then((bitmap) => {
			context.drawImage(bitmap, x, y);
			bitmap.close();
		})
		.catch(() => {});
}

function send(message: Record<string, unknown>) {
	if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(message));
}

// Keys sent down and not yet up. Losing focus lets go of all of them, or a
// modifier left down turns every later key into a chord.
const held = new Set<string>();
function letGo() {
	for (const key of held) send({ t: "key", key, down: false });
	held.clear();
}
addEventListener("blur", letGo);
document.addEventListener("visibilitychange", () => {
	if (document.hidden) letGo();
});

let idle: ReturnType<typeof setTimeout> | undefined;
function wake() {
	bar.classList.remove("idle");
	clearTimeout(idle);
	if (driving) idle = setTimeout(() => bar.classList.add("idle"), 2000);
}
addEventListener("pointermove", wake);

function drive(taking: boolean) {
	if (!taking) letGo();
	driving = taking;
	send({ t: "control", take: taking });
	render();
	if (taking) canvas.focus();
	else canvas.blur();
	say(taking ? "The screen is yours" : "The screen is the agent’s");
	wake();
}
control.addEventListener("click", () => {
	if (!control.disabled) drive(true);
});
back.addEventListener("click", () => drive(false));

function pasteText(text: string) {
	if (!driving || socket?.readyState !== WebSocket.OPEN) return;
	if (new TextEncoder().encode(text).length > 1048576) {
		say("Paste exceeds 1 MiB", false);
		return;
	}
	send({ t: "paste", text });
	canvas.focus();
}
function pasteFromHere() {
	navigator.clipboard
		.readText()
		.then(pasteText)
		.catch(() => say("The clipboard was not shared.", false));
}
paste.addEventListener("click", () => {
	if (driving) pasteFromHere();
});
// The machine has its own clipboard; text from this computer comes in on
// Ctrl+Alt+V (⌥⌘V on a Mac), or the Paste button.
const hostPaste = (event: KeyboardEvent) => (event.ctrlKey || event.metaKey) && event.altKey && event.code === "KeyV";
canvas.addEventListener("paste", (event) => {
	if (!driving) return;
	event.preventDefault();
	pasteText(event.clipboardData?.getData("text/plain") ?? "");
});

function at(event: MouseEvent) {
	const rect = canvas.getBoundingClientRect();
	return {
		x: Math.max(0, Math.min(canvas.width - 1, Math.round(((event.clientX - rect.left) * canvas.width) / rect.width))),
		y: Math.max(0, Math.min(canvas.height - 1, Math.round(((event.clientY - rect.top) * canvas.height) / rect.height))),
	};
}

// One pointer position per frame is plenty for a hand.
let pending: { x: number; y: number } | null = null;
canvas.addEventListener("pointermove", (event) => {
	if (!driving) return;
	const first = pending === null;
	pending = at(event);
	if (first)
		requestAnimationFrame(() => {
			if (pending) send({ t: "move", ...pending });
			pending = null;
		});
});
const buttons: Record<number, number> = { 0: 1, 1: 2, 2: 3 };
canvas.addEventListener("pointerdown", (event) => {
	if (!driving) return;
	canvas.focus();
	send({ t: "move", ...at(event) });
	const button = buttons[event.button];
	if (button !== undefined) send({ t: "button", b: button, down: true });
});
canvas.addEventListener("pointerup", (event) => {
	if (!driving) return;
	const button = buttons[event.button];
	if (button !== undefined) send({ t: "button", b: button, down: false });
});
canvas.addEventListener("contextmenu", (event) => event.preventDefault());

// One wheel notch per fifty pixels' worth, from a trackpad or a mouse.
let wheelX = 0;
let wheelY = 0;
canvas.addEventListener(
	"wheel",
	(event) => {
		if (!driving) return;
		event.preventDefault();
		const unit = event.deltaMode === 1 ? 50 : 1;
		wheelX += event.deltaX * unit;
		wheelY += event.deltaY * unit;
		while (Math.abs(wheelY) >= 50) {
			send({ t: "wheel", dy: Math.sign(wheelY) });
			wheelY -= Math.sign(wheelY) * 50;
		}
		while (Math.abs(wheelX) >= 50) {
			send({ t: "wheel", dx: Math.sign(wheelX) });
			wheelX -= Math.sign(wheelX) * 50;
		}
	},
	{ passive: false },
);

// The machine repeats a held key itself, so the browser's repeats are dropped.
canvas.addEventListener("keydown", (event) => {
	if (!driving) return;
	event.preventDefault();
	if (hostPaste(event)) {
		if (!event.repeat) pasteFromHere();
		return;
	}
	if (!event.repeat) {
		held.add(event.key);
		send({ t: "key", key: event.key, down: true });
	}
});
canvas.addEventListener("keyup", (event) => {
	if (!driving) return;
	event.preventDefault();
	if (hostPaste(event)) return;
	held.delete(event.key);
	send({ t: "key", key: event.key, down: false });
});
canvas.addEventListener("blur", () => {
	letGo();
	if (driving) say("Click the screen to type");
});

/* Files. */

type Asked = { resolve(result: unknown): void; reject(error: Error): void };
const asked = new Map<number, Asked>();
let nextId = 1;

function ask<T>(op: string, params: Record<string, unknown>): Promise<T> {
	return new Promise<T>((resolve, reject) => {
		if (socket?.readyState !== WebSocket.OPEN) {
			reject(new Error("The computer is not connected."));
			return;
		}
		const id = nextId++;
		asked.set(id, { resolve: resolve as (result: unknown) => void, reject });
		socket.send(JSON.stringify({ type: "files", id, op, ...params }));
	});
}

function answered(message: { id: number; ok: boolean; result?: unknown; error?: string }) {
	const waiting = asked.get(message.id);
	if (!waiting) return;
	asked.delete(message.id);
	if (message.ok) waiting.resolve(message.result);
	else waiting.reject(new Error(message.error ?? "The computer refused that."));
}

type Listing = { path: string; home: string; entries: { name: string; is_dir: boolean; size: number }[] };
type Chunk = { name: string; size: number; offset: number; data: string; next: number | null };

const files = element("files");
const filesPath = element("files-path");
const filesUp = element<HTMLButtonElement>("files-up");
const filesList = element("files-list");
const filesPick = element<HTMLInputElement>("files-pick");
let folder = "";
let home = "";

function sizeText(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	const units = ["KiB", "MiB", "GiB"];
	let value = bytes / 1024;
	let unit = 0;
	while (value >= 1024 && unit < units.length - 1) {
		value /= 1024;
		unit += 1;
	}
	return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

const joined = (name: string) => `${folder.replace(/\/$/, "")}/${name}`;

async function showFolder(path: string) {
	let listing: Listing;
	try {
		listing = await ask<Listing>("list", { path });
	} catch (error) {
		say(error instanceof Error ? error.message : String(error), false);
		return;
	}
	folder = listing.path;
	home = listing.home;
	filesPath.textContent = folder === home ? "~" : folder.startsWith(`${home}/`) ? `~${folder.slice(home.length)}` : folder;
	filesPath.title = folder;
	filesUp.disabled = folder === home;
	filesList.replaceChildren();
	if (listing.entries.length === 0) {
		const empty = document.createElement("li");
		empty.className = "empty";
		empty.textContent = "Nothing here";
		filesList.appendChild(empty);
	}
	for (const entry of listing.entries) {
		const item = document.createElement("li");
		item.className = entry.is_dir ? "dir" : "file";
		const name = document.createElement("span");
		name.className = "name";
		name.textContent = entry.name;
		item.appendChild(name);
		if (!entry.is_dir) {
			const size = document.createElement("span");
			size.className = "size";
			size.textContent = sizeText(entry.size);
			item.appendChild(size);
		}
		item.addEventListener("click", () => {
			if (entry.is_dir) void showFolder(joined(entry.name));
			else void saveFile(joined(entry.name), entry.name);
		});
		filesList.appendChild(item);
	}
	files.hidden = false;
}

/** Brings a file down into the place the person picks in the save dialog (the shell's transfer.rs). */
async function saveFile(path: string, name: string) {
	let id: string | null = null;
	let finished = false;
	try {
		id = await invoke<string | null>("transfer_begin", { name, open: false });
		if (id === null) return;
		say(`Saving ${name}…`, false);
		let offset: number | null = 0;
		while (offset !== null) {
			const chunk: Chunk = await ask<Chunk>("download", { path, offset });
			await invoke("transfer_write", { id, data: chunk.data });
			offset = chunk.next;
		}
		finished = true;
		await invoke("transfer_end", { id, finished: true });
		say(`Saved ${name}`);
	} catch (error) {
		say(error instanceof Error ? error.message : String(error), false);
	} finally {
		if (id !== null && !finished) await invoke("transfer_end", { id, finished: false }).catch(() => {});
	}
}

const CHUNK = 512 * 1024;

function base64(bytes: Uint8Array): string {
	let text = "";
	for (let at = 0; at < bytes.length; at += 0x8000) text += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
	return btoa(text);
}

/** Sends files from this computer into the folder the panel shows, a chunk at a time. */
async function sendFiles(list: File[]) {
	for (const file of list) {
		say(`Sending ${file.name}…`, false);
		let uploadId: string | null = null;
		let finished = false;
		try {
			uploadId = (await ask<{ uploadId: string; offset: number }>("upload_start", { path: joined(file.name) })).uploadId;
			let offset = 0;
			do {
				const bytes = new Uint8Array(await file.slice(offset, offset + CHUNK).arrayBuffer());
				offset = (await ask<{ offset: number }>("upload_chunk", { uploadId, offset, data: base64(bytes) })).offset;
			} while (offset < file.size);
			await ask("upload_finish", { uploadId });
			finished = true;
			say(`Sent ${file.name}`);
		} catch (error) {
			say(error instanceof Error ? error.message : String(error), false);
		} finally {
			if (uploadId !== null && !finished) await ask("upload_cancel", { uploadId }).catch(() => {});
		}
	}
	void showFolder(folder);
}

element("files-add").addEventListener("click", () => filesPick.click());
filesPick.addEventListener("change", () => {
	void sendFiles([...(filesPick.files ?? [])]);
	filesPick.value = "";
});
files.addEventListener("dragover", (event) => {
	event.preventDefault();
	files.classList.add("arriving");
});
files.addEventListener("dragleave", () => files.classList.remove("arriving"));
files.addEventListener("drop", (event) => {
	event.preventDefault();
	files.classList.remove("arriving");
	void sendFiles([...(event.dataTransfer?.files ?? [])]);
});
function closeFiles() {
	files.hidden = true;
	if (driving) canvas.focus();
}
browse.addEventListener("click", () => {
	if (files.hidden) void showFolder(folder);
	else closeFiles();
});
element("files-close").addEventListener("click", closeFiles);
filesUp.addEventListener("click", () => {
	if (folder !== home) void showFolder(folder.replace(/\/[^/]*$/, "") || "/");
});
addEventListener("keydown", (event) => {
	if (event.key === "Escape" && !files.hidden && document.activeElement !== canvas) closeFiles();
});

render();
connect();

