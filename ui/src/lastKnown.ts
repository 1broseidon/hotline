import { plainModel } from "./accounts";

/**
 * What the top strip last named for each teammate: its model's pretty name
 * and the effort it ran at, as a live session reported them. A resting
 * harness teammate has no session to ask, and its agent's list may not
 * name the model it ran, so the strip says what it last knew rather than a
 * model id or nothing. Kept by teammate in localStorage, across launches.
 */
export type LastKnown = { modelId: string; name: string; effort: string | null };

const KEY = "hotline.models.known";

let known: Record<string, LastKnown> | null = null;

function all(): Record<string, LastKnown> {
	if (known !== null) return known;
	known = {};
	try {
		const parsed: unknown = JSON.parse(localStorage.getItem(KEY) ?? "{}");
		if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)) {
			for (const [id, value] of Object.entries(parsed)) {
				const one = value as Partial<LastKnown>;
				if (typeof one.modelId === "string" && typeof one.name === "string") {
					known[id] = { modelId: one.modelId, name: one.name, effort: typeof one.effort === "string" ? one.effort : null };
				}
			}
		}
	} catch {
		// Unreadable: nothing known yet, and the next live session fills it in.
	}
	return known;
}

/** What the strip last knew of this teammate on this model, if it ran it. */
export function lastKnown(personaId: string, modelId: string): LastKnown | null {
	const one = all()[personaId];
	if (one === undefined || modelId === "") return null;
	return one.modelId === modelId || plainModel(one.modelId) === plainModel(modelId) ? one : null;
}

/** Keeps what a live session says it runs on, when it says something new. */
export function rememberKnown(personaId: string, next: LastKnown): void {
	const was = all()[personaId];
	if (was !== undefined && was.modelId === next.modelId && was.name === next.name && was.effort === next.effort) return;
	all()[personaId] = next;
	try {
		localStorage.setItem(KEY, JSON.stringify(all()));
	} catch {
		// Quota or private mode: the strip falls back as it did before.
	}
}
