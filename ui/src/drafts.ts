import type { Attachment } from "./generated/contract";

/**
 * What is being written to each conversation and side thread, by its
 * composer's id: words and the files picked to go with them. A composer
 * that leaves the screen when its teammate does finds its draft again when
 * it comes back, and so does a desk that was quit with words unsent.
 */
export type Draft = { text: string; attachments: Attachment[] };

const KEY = "hotline.draft.";
const EMPTY: Draft = { text: "", attachments: [] };
const held = new Map<string, Draft>();

export function readDraft(id: string): Draft {
	const kept = held.get(id);
	if (kept !== undefined) return kept;
	try {
		const raw = localStorage.getItem(KEY + id);
		if (raw === null) return EMPTY;
		const parsed = JSON.parse(raw) as Partial<Draft>;
		const draft = {
			text: typeof parsed.text === "string" ? parsed.text : "",
			attachments: Array.isArray(parsed.attachments) ? parsed.attachments : [],
		};
		held.set(id, draft);
		return draft;
	} catch {
		return EMPTY;
	}
}

export function writeDraft(id: string, draft: Draft): void {
	const empty = draft.text === "" && draft.attachments.length === 0;
	if (empty) held.delete(id);
	else held.set(id, draft);
	try {
		if (empty) localStorage.removeItem(KEY + id);
		else localStorage.setItem(KEY + id, JSON.stringify(draft));
	} catch {
		// Private mode or a full store: the draft lives as long as the window.
	}
}
