// Shared with Toad Mobile: the phone copies this file with `npm run sync:contract`.
/**
 * How an agent's reply is shown as chat: one string becomes a few bubbles.
 *
 * A port of the desktop's `session/pacing.rs`, so the bubbles the phone draws
 * while text streams are the ones the desktop writes when the turn ends.
 * Blank-line units outside fences become bubbles, lists stay whole, a
 * colon-intro and a stub join their neighbour.
 */

/** A stub shorter than this is a lead-in ("Here's the fix:") or a leftover ("ok."), not a bubble. */
export const SHORT_UNIT_CHARS = 60;

/** Split a finished reply into chat bubbles, exactly as the desktop does. */
export function paced(text: string): string[] {
	return merge(units(text), false);
}

/**
 * Split a reply that is still arriving. Every cut before the last unit is the
 * one `paced` will make, and the last unit is a bubble of its own as soon as
 * it starts, because it will grow into one. The one rule that waits for the
 * end is the final-stub rule: a short closing line such as "ok." joins the
 * bubble before it only once the desktop writes the reply down.
 */
export function pacedLive(text: string): string[] {
	return merge(units(text), true);
}

/** The id the desktop gives bubble `index` of message `base`. */
export function bubbleId(base: string, index: number): string {
	return index === 0 ? base : `${base}-${index + 1}`;
}

function units(text: string): string[] {
	const lines = text.split(/\r?\n/);
	const out: string[] = [];
	let i = 0;
	while (i < lines.length) {
		const fence = openFence(lines[i]!);
		if (fence) {
			const start = i;
			i += 1;
			while (i < lines.length && !isCloseFence(lines[i]!, fence.marker, fence.count)) i += 1;
			if (i < lines.length) i += 1;
			pushUnit(out, lines.slice(start, i));
			continue;
		}
		if (isListLine(lines[i]!)) {
			const start = i;
			i += 1;
			while (i < lines.length) {
				if (isListLine(lines[i]!)) {
					i += 1;
					continue;
				}
				if (!lines[i]!.trim()) {
					let j = i + 1;
					while (j < lines.length && !lines[j]!.trim()) j += 1;
					if (j < lines.length && isListLine(lines[j]!)) {
						i = j;
						continue;
					}
					break;
				}
				break;
			}
			pushUnit(out, lines.slice(start, i));
			continue;
		}
		if (!lines[i]!.trim()) {
			i += 1;
			continue;
		}
		const start = i;
		i += 1;
		while (i < lines.length && lines[i]!.trim() && !openFence(lines[i]!) && !isListLine(lines[i]!))
			i += 1;
		pushUnit(out, lines.slice(start, i));
	}
	return out;
}

function pushUnit(out: string[], lines: string[]) {
	const unit = lines.join("\n").trim();
	if (unit) out.push(unit);
}

/**
 * Join a lead-in to what it introduces, left to right, once. A unit that ends
 * with `:` introduces the next. A stub shorter than SHORT_UNIT_CHARS joins the
 * next unit, or the previous when it is last and the text is finished.
 */
function merge(units: string[], live: boolean): string[] {
	if (units.length <= 1) return units;
	const out: string[] = [];
	const n = units.length;
	let i = 0;
	while (i < n) {
		const unit = units[i]!;
		const short = Array.from(unit).length < SHORT_UNIT_CHARS;
		const introduces = unit.endsWith(":");
		if (i + 1 < n && (introduces || short)) {
			out.push(`${unit}\n\n${units[i + 1]}`);
			i += 2;
			continue;
		}
		if (i + 1 === n && short && !live) {
			if (out.length) out[out.length - 1] = `${out[out.length - 1]}\n\n${unit}`;
			else out.push(unit);
			break;
		}
		out.push(unit);
		i += 1;
	}
	return out;
}

function openFence(line: string): { marker: string; count: number } | null {
	const trimmed = line.trimStart();
	const marker = trimmed.startsWith("`") ? "`" : trimmed.startsWith("~") ? "~" : null;
	if (!marker) return null;
	let count = 0;
	while (trimmed[count] === marker) count += 1;
	return count >= 3 ? { marker, count } : null;
}

function isCloseFence(line: string, marker: string, count: number): boolean {
	const trimmed = line.trim();
	let n = 0;
	while (trimmed[n] === marker) n += 1;
	return n >= count && !trimmed.slice(n).trim();
}

function isListLine(line: string): boolean {
	const trimmed = line.trimStart();
	return (
		trimmed.startsWith("- ") ||
		trimmed.startsWith("* ") ||
		trimmed.startsWith("+ ") ||
		/^\d+\. /.test(trimmed)
	);
}
