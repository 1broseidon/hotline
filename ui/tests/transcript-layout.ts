/**
 * A transcript laid out as a browser would lay it out, for a DOM that has no
 * layout: each row as tall as the test says, a gap as tall as its style, a
 * row being measured off to the side as tall as it would be in the column,
 * and the scroller a fixed view on them. Install it before the first render,
 * so the transcript's first measurements are of this layout.
 *
 * `tall` holds the heights by row id; a row not in it is `row` tall. Change
 * an entry and the row is that tall from then on, as when a picture arrives.
 * `column.width` is how wide the column is, for a window made narrower.
 * With `whole`, the scroll is kept in whole pixels, the fraction dropped, as
 * WebKit keeps it; otherwise to the fraction it was set to.
 *
 * A column is laid out once for each state of the page, not once for each
 * question asked of it: where its rows start is worked out on the first read
 * after the page or a height in `tall` changed, and every read until the next
 * change is answered from that. The transcript reads every row's box each
 * time it places its window, so working a row's place out afresh for each
 * read cost the square of the rows in the page, at every placing.
 *
 * The DOM's element classes outlive the window a test installs, so `restore`
 * puts back what this changed on them; call it when the test is over.
 */
export function layOut({ row, view, whole = false }: { row: number; view: number; whole?: boolean }): {
	tall: Map<string, number>;
	column: { width: number };
	scrollsIntoView: { id: string | undefined; options: ScrollIntoViewOptions | undefined }[];
	restore(): void;
} {
	/* Set when what a column's layout was worked out from has changed: a row's height here, or the page (below). */
	let stale = false;
	class Tall extends Map<string, number> {
		override set(id: string, height: number) {
			stale = true;
			return super.set(id, height);
		}
		override delete(id: string) {
			stale = true;
			return super.delete(id);
		}
		override clear() {
			stale = true;
			super.clear();
		}
	}
	const tall = new Tall();
	const column = { width: 700 };
	const scrollsIntoView: { id: string | undefined; options: ScrollIntoViewOptions | undefined }[] = [];
	const proto = (window as unknown as { HTMLElement: typeof HTMLElement }).HTMLElement.prototype;
	const changed = ["getBoundingClientRect", "scrollHeight", "scrollTop", "clientHeight", "clientWidth", "scrollIntoView"] as const;
	const before = changed.map((name) => [name, Object.getOwnPropertyDescriptor(proto, name)] as const);
	// Every change to the page that moves a row: one joined or left, or a gap was given another height.
	const changes = new window.MutationObserver(() => {
		stale = true;
	});
	changes.observe(document as unknown as Node, { subtree: true, childList: true, attributes: true, attributeFilter: ["style"] });
	const restore = () => {
		changes.disconnect();
		for (const [name, descriptor] of before) {
			if (descriptor !== undefined) Object.defineProperty(proto, name, descriptor);
			else Reflect.deleteProperty(proto, name);
		}
	};
	const own = proto.getBoundingClientRect;
	const is = (el: Element, name: string) => el.classList.contains(name);
	const heightOf = (el: HTMLElement): number => {
		if (is(el, "tape-gap")) return Number.parseFloat(el.style.height);
		if (is(el, "tape-row")) return tall.get(el.dataset["eventId"] ?? el.dataset["probe"] ?? "") ?? row;
		return 0;
	};
	/** A column as laid out: where each of its rows and gaps starts and how tall it is, how tall they are in all, and the scroller it is in. */
	type Laid = { parts: Map<Element, { start: number; height: number }>; total: number; scroller: HTMLElement | null };
	const columns = new Map<Element, Laid>();
	const scrollers = new Map<Element, number>();
	/** Forgets every layout worked out before the page last changed. The changes not yet reported are taken here, so a read straight after one sees it. */
	const current = () => {
		if (changes.takeRecords().length > 0) stale = true;
		if (!stale) return;
		columns.clear();
		scrollers.clear();
		stale = false;
	};
	/** The layout of the column an element is a row or a gap of, or null when it is not in one. */
	const laidIn = (parent: Element | null): Laid | null => {
		if (parent === null) return null;
		current();
		const known = columns.get(parent);
		if (known !== undefined) return known;
		if (!is(parent, "tape-column")) return null;
		const laid: Laid = { parts: new Map(), total: 0, scroller: parent.closest(".scroller") as HTMLElement | null };
		for (const part of parent.children) {
			if (!is(part, "tape-row") && !is(part, "tape-gap")) continue;
			const height = heightOf(part as HTMLElement);
			laid.parts.set(part, { start: laid.total, height });
			laid.total += height;
		}
		columns.set(parent, laid);
		return laid;
	};
	const box = (top: number, height: number) => ({ top, bottom: top + height, height, left: 0, right: 0, width: 0, x: 0, y: top, toJSON() {} }) as DOMRect;
	proto.getBoundingClientRect = function (this: HTMLElement) {
		const parent = this.parentElement;
		const laid = laidIn(parent);
		const part = laid?.parts.get(this);
		if (laid && part) return box(part.start - (laid.scroller?.scrollTop ?? 0), part.height);
		if (is(this, "scroller")) return box(0, view);
		// Off to the side, where nothing is in view.
		if (parent !== null && is(parent, "tape-probe")) return box(-1_000_000, heightOf(this));
		return own.call(this);
	};
	Object.defineProperty(proto, "scrollHeight", {
		configurable: true,
		get(this: HTMLElement) {
			current();
			const known = scrollers.get(this);
			if (known !== undefined) return known;
			if (!is(this, "scroller")) return 0;
			let total = 0;
			for (const inside of this.querySelectorAll(".tape-column")) total += laidIn(inside)!.total;
			scrollers.set(this, total);
			return total;
		},
	});
	// The scroll stops at the column's ends, as an engine's does.
	const scrolled = new WeakMap<HTMLElement, number>();
	Object.defineProperty(proto, "scrollTop", {
		configurable: true,
		get(this: HTMLElement) {
			return Math.max(0, Math.min(scrolled.get(this) ?? 0, this.scrollHeight - this.clientHeight));
		},
		set(this: HTMLElement, to: number) {
			scrolled.set(this, Math.max(0, Math.min(whole ? Math.floor(to) : to, this.scrollHeight - this.clientHeight)));
		},
	});
	Object.defineProperty(proto, "clientHeight", {
		configurable: true,
		get(this: HTMLElement) {
			return is(this, "scroller") ? view : 0;
		},
	});
	Object.defineProperty(proto, "clientWidth", {
		configurable: true,
		get(this: HTMLElement) {
			return is(this, "tape-column") ? column.width : 0;
		},
	});
	// A row's height as laid out, which is how a row being measured off to
	// the side is read: its height and nothing else. The DOM's own style
	// for a row is not asked for; it works out every property there is.
	const styled = window.getComputedStyle.bind(window);
	window.getComputedStyle = ((el: Element, pseudo?: string | null) =>
		is(el, "tape-row") ? ({ height: `${heightOf(el as HTMLElement)}px` } as CSSStyleDeclaration) : styled(el, pseudo)) as typeof window.getComputedStyle;
	// A row scrolled into view, as the engine does it at once: to the middle, or just far enough to show.
	proto.scrollIntoView = function (this: HTMLElement, options?: ScrollIntoViewOptions | boolean) {
		const target = this.closest<HTMLElement>(".tape-column > .tape-row");
		const laid = laidIn(target?.parentElement ?? null);
		const part = target ? laid?.parts.get(target) : undefined;
		const asked = typeof options === "object" ? options : undefined;
		scrollsIntoView.push({ id: target?.dataset["eventId"], options: asked });
		const scroller = laid?.scroller;
		if (!scroller || !part) return;
		const { start, height } = part;
		if (asked?.block === "center") scroller.scrollTop = Math.max(0, start + height / 2 - view / 2);
		else if (start < scroller.scrollTop) scroller.scrollTop = start;
		else if (start + height > scroller.scrollTop + view) scroller.scrollTop = start + height - view;
		scroller.dispatchEvent(new window.Event("scroll") as unknown as Event);
	};
	return { tall, column, scrollsIntoView, restore };
}
