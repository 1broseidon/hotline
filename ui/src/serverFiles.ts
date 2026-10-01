import { invoke } from "@tauri-apps/api/core";
import { useSyncExternalStore } from "react";
import { activeDeskId, allDesks } from "./desks";
import type { Attachment } from "./generated/contract";
import { openSentFile, pickDirectory, pickFiles, revealPath, saveSentFile } from "./native";
import { toBase64 } from "./voice/wav";
import { wire } from "./wire";

/**
 * Files, wherever the open desk keeps them (BRO-145).
 *
 * On this computer's desk a path is a path on this disk, and the system's
 * pickers and file manager handle it. On a desk on a server every path is
 * the server's: a folder is chosen, and a folder is shown, in the window's
 * own browser of the server's disk (ServerFiles.tsx); a file goes up in
 * chunks and comes down in chunks, over the desk's own wire. The shell
 * reads only files the person picked or dropped, and writes only where they
 * chose to save (crates/hotline-app/src/transfer.rs).
 *
 * Callers use these instead of native.ts's pickers and reveal, and never
 * need to know which kind of desk is open.
 */

/** Whether the open desk is on a server, so its paths are not this computer's. */
export function onServer(): boolean {
	const id = activeDeskId();
	return allDesks().find((desk) => desk.id === id)?.kind === "remote";
}

/** What a "reveal" is called where it happens: the file manager here, the window's own browser on a server. */
export function revealLabel(): string {
	return onServer() ? "Show on the server" : "Reveal in the file manager";
}

/** What the server browser is doing: choosing a folder, or showing one. */
export type Browsing =
	| { mode: "folder"; start: string; title: string; resolve(path: string | null): void }
	| { mode: "show"; start: string; title: string; highlight?: string; resolve(path: string | null): void };

let browsing: Browsing | null = null;
const listeners = new Set<() => void>();

function set(next: Browsing | null) {
	browsing?.resolve(null);
	browsing = next;
	for (const listener of listeners) listener();
}

export function useBrowsing(): Browsing | null {
	return useSyncExternalStore(
		(listener) => {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		() => browsing,
	);
}

/** The browser answers, and closes. */
export function endBrowsing(path: string | null) {
	const was = browsing;
	browsing = null;
	for (const listener of listeners) listener();
	was?.resolve(path);
}

/** Where a browser starts when nothing better is known. */
const ROOT = "/";

/** A folder, on this computer or the server; null when the person backed out. */
export function chooseFolder(start?: string, title = "Choose a folder on the server"): Promise<string | null> {
	if (!onServer()) return pickDirectory();
	return new Promise((resolve) => set({ mode: "folder", start: start || ROOT, title, resolve }));
}

/** Shows a folder, or the folder a file is in. */
export function showPath(path: string): void {
	if (path === "") return;
	if (!onServer()) {
		void revealPath(path);
		return;
	}
	// A folder opens on itself; a file, on its folder, lit (ServerFiles climbs).
	set({ mode: "show", start: path, title: "On the server", highlight: path, resolve: () => {} });
}

/** A Windows server's drive root, `C:\`, when the path starts on one. */
function driveOf(path: string): string | undefined {
	return /^[A-Za-z]:[\\/]/.exec(path)?.[0];
}

/** One folder up, on either kind of slash; a root is its own parent. */
export function parentOf(path: string): string {
	const drive = driveOf(path);
	const trimmed = path.replace(/[/\\]+$/, "");
	const slash = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
	if (drive !== undefined) return slash < drive.length ? drive : trimmed.slice(0, slash);
	return slash <= 0 ? ROOT : trimmed.slice(0, slash);
}

/** A name inside a folder, joined with the slash that folder is written in. */
export function childOf(folder: string, name: string): string {
	const slash = driveOf(folder) !== undefined || folder.startsWith("\\\\") ? "\\" : "/";
	return `${folder.replace(/[/\\]+$/, "")}${slash}${name}`;
}

export function nameOf(path: string): string {
	const trimmed = path.replace(/[/\\]+$/, "");
	return trimmed.slice(Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\")) + 1);
}

/* Down. */

type Chunk = { data: string; next?: number | null };

/**
 * Brings a file down into a save dialog's choice, or into a private place to
 * open it. Answers where it went, or null when the dialog was dismissed.
 */
async function bringDown(name: string, open: boolean, read: (offset: number) => Promise<Chunk>): Promise<string | null> {
	const id = await invoke<string | null>("transfer_begin", { name, open });
	if (id === null) return null;
	let finished = false;
	try {
		let offset: number | null | undefined = 0;
		while (offset !== null && offset !== undefined) {
			const chunk: Chunk = await read(offset);
			await invoke("transfer_write", { id, data: chunk.data });
			offset = chunk.next;
		}
		finished = true;
	} finally {
		if (!finished) await invoke("transfer_end", { id, finished: false }).catch(() => {});
	}
	return invoke<string | null>("transfer_end", { id, finished: true });
}

/** Saves a copy of any server file where the person chooses. */
export function downloadServerFile(path: string): Promise<string | null> {
	return bringDown(nameOf(path), false, (offset) => wire.command("files.download", { path, offset }));
}

/** A file a teammate sent, by its message, wherever the desk is. */
export type Sent = { personaId: string; eventId: string; index: number; file: Attachment };

function readSent(sent: Sent) {
	return (offset: number) =>
		wire.command("file.read", { personaId: sent.personaId, eventId: sent.eventId, index: sent.index, offset });
}

export async function openSent(sent: Sent): Promise<void> {
	if (!onServer()) return openSentFile(sent.file.path);
	await bringDown(sent.file.name, true, readSent(sent));
}

export async function saveSent(sent: Sent): Promise<string | null> {
	if (!onServer()) return saveSentFile(sent.file.path);
	return bringDown(sent.file.name, false, readSent(sent));
}

/* Up. */

/** Files to attach: the system's picker, through the shell when they must be carried to a server. */
export async function pickAttachments(): Promise<{ path: string; size?: number }[]> {
	if (!onServer()) return (await pickFiles()).map((path) => ({ path }));
	try {
		return await invoke<{ path: string; size: number }[]>("transfer_pick");
	} catch {
		return [];
	}
}

/**
 * Carries what a message has attached to the server, when the desk is on
 * one, and answers the attachments as the server will find them. On this
 * computer's desk they are already where the desk can read them.
 */
export async function carry(attachments: Attachment[], onProgress?: (sent: number, total: number) => void): Promise<Attachment[]> {
	if (attachments.length === 0 || !onServer()) return attachments;
	const carried: Attachment[] = [];
	for (const attachment of attachments) {
		carried.push(staged.has(attachment.path) ? attachment : { ...attachment, path: await upload(attachment, onProgress) });
	}
	return carried;
}

/** Paths [stage] put on the desk already, which [carry] leaves where they are. */
const staged = new Set<string>();

/** The wire's largest upload chunk. */
const UPLOAD_CHUNK = 512 * 1024;

/**
 * Bytes the window holds, a pasted picture, as a file the desk can read: put
 * in the desk's own uploads folder over its wire, which a desk on this
 * computer and one on a server both keep. Answers where the desk has it.
 */
export async function stage(name: string, bytes: Uint8Array): Promise<{ path: string; size: number }> {
	if (bytes.length === 0) throw new Error("There was nothing in it.");
	const started = await wire.command("files.upload_start", { name });
	let finished = false;
	try {
		let offset = 0;
		while (offset < bytes.length) {
			const data = toBase64(bytes.subarray(offset, offset + UPLOAD_CHUNK));
			const wrote = await wire.command("files.upload_chunk", { uploadId: started.uploadId, offset, data });
			offset = wrote.offset;
		}
		const done = await wire.command("files.upload_finish", { uploadId: started.uploadId });
		finished = true;
		staged.add(done.path);
		return { path: done.path, size: bytes.length };
	} finally {
		if (!finished) await wire.command("files.upload_cancel", { uploadId: started.uploadId }).catch(() => {});
	}
}

type LocalChunk = { data: string; size: number; next: number | null };

async function upload(attachment: Attachment, onProgress?: (sent: number, total: number) => void): Promise<string> {
	const first = await invoke<LocalChunk>("transfer_read", { path: attachment.path, offset: 0 });
	// A name, not a path: the desk stages it in a private folder of its own.
	const started = await wire.command("files.upload_start", { name: attachment.name });
	let finished = false;
	try {
		let chunk = first;
		let offset = 0;
		for (;;) {
			const wrote = await wire.command("files.upload_chunk", { uploadId: started.uploadId, offset, data: chunk.data });
			offset = wrote.offset;
			onProgress?.(offset, chunk.size);
			if (chunk.next === null) break;
			chunk = await invoke<LocalChunk>("transfer_read", { path: attachment.path, offset: chunk.next });
		}
		const done = await wire.command("files.upload_finish", { uploadId: started.uploadId });
		finished = true;
		return done.path;
	} finally {
		if (!finished) await wire.command("files.upload_cancel", { uploadId: started.uploadId }).catch(() => {});
	}
}
