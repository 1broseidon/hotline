import { useEffect, useState } from "react";
import { deskKey } from "./desks";

/**
 * Where this window last stood in each teammate's tape, by persona, kept
 * per desk. Opening a teammate, or sitting on one while a new line lands, is
 * what "shown" means. The rail then has a ts to compare against.
 */
export function useSeen(selectedId: string | null, latest: number | null | undefined): Record<string, number> {
	const [seen, setSeen] = useState<Record<string, number>>(loadSeen);
	useEffect(() => {
		if (selectedId === null || latest == null) return;
		setSeen((known) => {
			if (known[selectedId] === latest) return known;
			const next = { ...known, [selectedId]: latest };
			saveSeen(next);
			return next;
		});
	}, [selectedId, latest]);
	return seen;
}

/** Private mode or a full disk just means every teammate looks unread until
 * you open them again. */
const SEEN_KEY = "hotline.rail.seen";

function loadSeen(): Record<string, number> {
	try {
		const raw = localStorage.getItem(deskKey(SEEN_KEY));
		if (!raw) return {};
		const parsed: unknown = JSON.parse(raw);
		if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return {};
		const seen: Record<string, number> = {};
		for (const [id, ts] of Object.entries(parsed)) {
			if (typeof ts === "number" && Number.isFinite(ts)) seen[id] = ts;
		}
		return seen;
	} catch {
		return {};
	}
}

function saveSeen(seen: Record<string, number>): void {
	try {
		localStorage.setItem(deskKey(SEEN_KEY), JSON.stringify(seen));
	} catch {
		// Quota, private mode — the next load treats everything as unread.
	}
}
