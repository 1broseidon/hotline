import { useEffect, useState } from "react";
import { deskKey } from "./desks";
import { storedNumbers, storeNumbers } from "./storage";

const SEEN_KEY = "hotline.rail.seen";

/**
 * Where this window last stood in each teammate's tape, by persona, kept
 * per desk. Opening a teammate, or sitting on one while a new line lands, is
 * what "shown" means. The rail then has a ts to compare against.
 */
export function useSeen(selectedId: string | null, latest: number | null | undefined): Record<string, number> {
	const [seen, setSeen] = useState(() => storedNumbers(deskKey(SEEN_KEY)));
	useEffect(() => {
		if (selectedId === null || latest == null) return;
		setSeen((known) => {
			if (known[selectedId] === latest) return known;
			const next = { ...known, [selectedId]: latest };
			storeNumbers(deskKey(SEEN_KEY), next);
			return next;
		});
	}, [selectedId, latest]);
	return seen;
}
