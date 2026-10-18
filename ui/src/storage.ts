/**
 * A record of numbers kept in localStorage, such as where the window last
 * stood in each tape or thread. Anything unreadable comes back empty and a
 * failed write is dropped: private mode, a full disk or a hand-edited value
 * only means some things look unread until they are opened again.
 */
export function storedNumbers(key: string): Record<string, number> {
	try {
		const raw = localStorage.getItem(key);
		if (!raw) return {};
		const parsed: unknown = JSON.parse(raw);
		if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return {};
		const numbers: Record<string, number> = {};
		for (const [name, value] of Object.entries(parsed)) {
			if (typeof value === "number" && Number.isFinite(value)) numbers[name] = value;
		}
		return numbers;
	} catch {
		return {};
	}
}

export function storeNumbers(key: string, numbers: Record<string, number>): void {
	try {
		localStorage.setItem(key, JSON.stringify(numbers));
	} catch {
		// Quota or private mode: the next load starts from what was there.
	}
}
