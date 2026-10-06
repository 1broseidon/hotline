import type { LinkPreview } from "./generated/contract";
import { wire } from "./wire";

/**
 * The one link in a message worth a card: the first https address written
 * out bare, outside code. A link given words of its own, `[like this](…)`,
 * is already part of the sentence and stays inline. The phone picks the same
 * one, so a message has the same card on both.
 */
export function previewLink(text: string): string | null {
	const prose = text.replace(/```[\s\S]*?(```|$)/g, " ").replace(/`[^`\n]*`/g, " ");
	for (const match of prose.matchAll(/(\]\()?(<)?(https:\/\/[^\s<>()[\]"'`]+)/g)) {
		if (match[1] !== undefined) continue;
		const url = match[3]!.replace(/[.,;:!?*_~]+$/, "");
		if (fetchable(url)) return url;
	}
	return null;
}

/** A message that is nothing but its link. */
export function onlyLink(text: string, url: string): boolean {
	return text.trim().replace(/^<|>$/g, "").replace(/\/$/, "") === url.replace(/\/$/, "");
}

/** Only https, and never a machine on this network; the desk refuses those too. */
export function fetchable(value: string): boolean {
	try {
		const url = new URL(value);
		if (url.protocol !== "https:") return false;
		const host = url.hostname.toLowerCase();
		return !(
			host === "localhost" ||
			host.endsWith(".local") ||
			host.endsWith(".internal") ||
			host.startsWith("[") ||
			/^(10|127)\./.test(host) ||
			/^192\.168\./.test(host) ||
			/^172\.(1[6-9]|2\d|3[01])\./.test(host) ||
			/^169\.254\./.test(host) ||
			/^100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\./.test(host) ||
			!host.includes(".")
		);
	} catch {
		return false;
	}
}

const known = new Map<string, LinkPreview | null>();
const reading = new Map<string, Promise<LinkPreview | null>>();

/** The card for a link if this window already has one; `null` means it has none. */
export function knownPreview(url: string): LinkPreview | null | undefined {
	return known.get(url);
}

/** The card for a link, asked of the desk once per window. */
export function fetchPreview(url: string): Promise<LinkPreview | null> {
	const have = known.get(url);
	if (have !== undefined) return Promise.resolve(have);
	let pending = reading.get(url);
	if (pending === undefined) {
		pending = wire
			.command("link.preview", { url })
			.then(
				(preview) => preview ?? null,
				() => null,
			)
			.then((preview) => {
				known.set(url, preview);
				return preview;
			})
			.finally(() => reading.delete(url));
		reading.set(url, pending);
	}
	return pending;
}
