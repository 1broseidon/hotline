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

/** How long typing must pause before the draft is written to storage. */
const SETTLE_MS = 400;
const dirty = new Set<string>();
let settle: ReturnType<typeof setTimeout> | undefined;

/**
 * The words are held at once and kept in storage a pause later: a keystroke
 * is a synchronous write otherwise. An emptied draft stays held as empty until
 * it is written, so a read in between does not find the old words in storage.
 * Whatever is waiting goes out as the window hides.
 */
export function writeDraft(id: string, draft: Draft): void {
	const empty = draft.text === "" && draft.attachments.length === 0;
	held.set(id, empty ? EMPTY : draft);
	dirty.add(id);
	clearTimeout(settle);
	settle = setTimeout(flushDrafts, SETTLE_MS);
}

function flushDrafts(): void {
	clearTimeout(settle);
	for (const id of dirty) {
		const draft = held.get(id) ?? EMPTY;
		try {
			if (draft === EMPTY) localStorage.removeItem(KEY + id);
			else localStorage.setItem(KEY + id, JSON.stringify(draft));
		} catch {
			// Private mode or a full store: the draft lives as long as the window.
		}
	}
	dirty.clear();
}

globalThis.addEventListener?.("pagehide", flushDrafts);
globalThis.addEventListener?.("beforeunload", flushDrafts);
