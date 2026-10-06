import { useEffect, useRef } from "react";

/**
 * Keeps `value` with `save`, once it has held still for `ms`. A drag changes a
 * width on every pointer move and storage is a synchronous write, so only
 * where the drag came to rest is written. What is waiting goes out as the
 * window hides or the owner unmounts, so a quit mid-drag loses nothing.
 */
export function useSaved<T>(value: T, save: (value: T) => void, ms = 300): void {
	const waiting = useRef<{ value: T } | null>(null);
	const saver = useRef(save);
	saver.current = save;
	const flush = useRef(() => {
		if (waiting.current === null) return;
		const { value: held } = waiting.current;
		waiting.current = null;
		saver.current(held);
	});
	useEffect(() => {
		waiting.current = { value };
		const timer = window.setTimeout(flush.current, ms);
		return () => window.clearTimeout(timer);
	}, [value, ms]);
	useEffect(() => {
		const out = flush.current;
		window.addEventListener("pagehide", out);
		return () => {
			window.removeEventListener("pagehide", out);
			out();
		};
	}, []);
}
