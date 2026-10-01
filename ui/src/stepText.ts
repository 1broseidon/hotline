import type { ToolOutput } from "./generated/contract";

/*
 * A step's title and output as a person reads them, not as the harness wrote
 * them: tool ids as words, commands without the shell and the cd in front,
 * output without terminal colour codes or progress redraws, and an edit as
 * the lines it changed rather than the whole file twice.
 */

/** Lines of unchanged file kept either side of an edit. */
const CONTEXT = 2;

const HOME = /(?:\/home|\/Users)\/[^/\s"']+(?=\/|\s|$)/g;
const ANSI = /\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]/g;

/** A home directory as `~`, the way a shell prompt shows it. */
export function tidyPath(path: string): string {
	return path.replace(HOME, "~");
}

/** "computer__browser " → "computer · browser"; "zsh -lc 'cd x && make'" → "Run make". */
export function stepTitle(raw: string): string {
	// Some harnesses fence the command or pattern in markdown backticks.
	let title = raw.trim().replace(/`([^`]*)`/g, "$1");
	const tool = /^([a-z0-9-]+)__([a-z0-9_]+)$/i.exec(title);
	if (tool) return `${tool[1]} · ${tool[2]!.replace(/_/g, " ")}`;
	const shell = /^(?:\S*\/)?(?:ba|z)?sh\s+-l?c\s+(["'])([\s\S]*)$/.exec(title);
	if (shell) {
		const quote = shell[1]!;
		let command = shell[2]!;
		if (command.endsWith(quote)) command = command.slice(0, -1);
		title = `Run ${command}`;
	}
	const cd = /^Run cd\s+(?:"[^"]*"|'[^']*'|\S+)\s*(?:&&|;)\s*(?=\S)/.exec(title);
	if (cd) title = `Run ${title.slice(cd[0].length)}`;
	return tidyPath(title);
}

/** Text as it would have looked on the terminal once it settled. */
export function plainText(text: string): string {
	return text
		.replace(ANSI, "")
		.replace(/\r\n/g, "\n")
		.split("\n")
		.map((line) => line.slice(line.lastIndexOf("\r") + 1).trimEnd())
		.join("\n")
		.replace(/\n{3,}/g, "\n\n")
		.trim();
}

/** A shell tool's JSON envelope reduced to what the command printed. */
function shellPrinted(text: string): string | undefined {
	if (!text.trimStart().startsWith("{")) return undefined;
	let value: unknown;
	try {
		value = JSON.parse(text);
	} catch {
		return undefined;
	}
	if (value === null || typeof value !== "object") return undefined;
	const result = value as Record<string, unknown>;
	if (typeof result.stdout !== "string" && typeof result.stderr !== "string") return undefined;
	const parts = [result.stdout, result.stderr].flatMap((one) => (typeof one === "string" && plainText(one) !== "" ? [plainText(one)] : []));
	const code = result.exit_code ?? result.exitCode;
	if (typeof code === "number" && code !== 0) parts.push(`exit ${code}`);
	if (parts.length === 0) return typeof result.status === "string" ? result.status : "";
	return parts.join("\n");
}

/** One piece of a tool's output as readable text. */
export function outputText(one: ToolOutput): string {
	if (one.type === "text") return plainText(shellPrinted(one.text) ?? one.text);
	return [tidyPath(one.path), ...editLines(one.oldText ?? null, one.newText)].join("\n");
}

/** The changed lines of an edit, marked, with a little unchanged file around them. */
function editLines(oldText: string | null, newText: string): string[] {
	const after = newText.split("\n");
	if (oldText === null) return after.map((line) => `+ ${line}`);
	const before = oldText.split("\n");
	let head = 0;
	while (head < before.length && head < after.length && before[head] === after[head]) head++;
	let tail = 0;
	while (tail < before.length - head && tail < after.length - head && before[before.length - 1 - tail] === after[after.length - 1 - tail]) tail++;
	if (head === before.length && head === after.length) return ["  (no change)"];
	const from = Math.max(0, head - CONTEXT);
	const lines: string[] = [];
	if (from > 0) lines.push("  …");
	for (const line of before.slice(from, head)) lines.push(`  ${line}`);
	for (const line of before.slice(head, before.length - tail)) lines.push(`- ${line}`);
	for (const line of after.slice(head, after.length - tail)) lines.push(`+ ${line}`);
	const kept = after.slice(after.length - tail, after.length - tail + CONTEXT);
	for (const line of kept) lines.push(`  ${line}`);
	if (tail > kept.length) lines.push("  …");
	return lines;
}

/**
 * A run of thought pieces as one thought. Harnesses send thinking in pieces
 * that can break mid-sentence; a piece that ends one runs straight into the
 * next, and one that finishes a sentence starts a new paragraph.
 */
export function joinThoughts(pieces: string[]): string {
	let text = "";
	for (const piece of pieces) {
		if (text === "") text = piece;
		else if (/[.!?:)\]`"'…]\s*$|\n\s*$/.test(text)) text = `${text.trimEnd()}\n\n${piece.trimStart()}`;
		else if (/^[\s.,;:!?)\]…]/.test(piece) || /\s$/.test(text)) text += piece;
		else text += ` ${piece}`;
	}
	return text.trim();
}
