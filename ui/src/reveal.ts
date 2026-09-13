// Shared with Toad Mobile: the phone copies this file with `npm run sync:contract`.
/**
 * How streaming text is shown: not a growing blob, but complete lines, each
 * fading down into place as it finishes. A half-typed line is never drawn.
 */

/** The part of a streaming reply that is finished: every line closed by a newline. */
export function revealed(text: string): string {
	return text.slice(0, text.lastIndexOf("\n") + 1);
}

export type Segment = { kind: "line" | "code" | "gap"; text: string };

/**
 * One bubble's text as the pieces that fade in: a line, a whole code block,
 * or the gap a blank line leaves. A code block that is still open is one
 * segment that grows, not a new segment per line.
 */
export function revealSegments(text: string): Segment[] {
	const lines = text.split("\n");
	const out: Segment[] = [];
	let i = 0;
	while (i < lines.length) {
		const line = lines[i]!;
		if (/^\s*(```|~~~)/.test(line)) {
			const marker = line.trim().startsWith("`") ? "```" : "~~~";
			const body: string[] = [];
			i += 1;
			while (i < lines.length && !lines[i]!.trim().startsWith(marker)) {
				body.push(lines[i]!);
				i += 1;
			}
			if (i < lines.length) i += 1;
			out.push({ kind: "code", text: body.join("\n").trimEnd() });
			continue;
		}
		if (!line.trim()) {
			if (out.length && out[out.length - 1]!.kind !== "gap") out.push({ kind: "gap", text: "" });
			i += 1;
			continue;
		}
		out.push({ kind: "line", text: line });
		i += 1;
	}
	while (out.length && out[out.length - 1]!.kind === "gap") out.pop();
	return out;
}
