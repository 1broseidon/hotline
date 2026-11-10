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
 *
 * The DOM's element classes outlive the window a test installs, so `restore`
 * puts back what this changed on them; call it when the test is over.
 */
export function layOut({ row, view }: { row: number; view: number }): {
	tall: Map<string, number>;
	column: { width: number };
	scrollsIntoView: { id: string | undefined; options: ScrollIntoViewOptions | undefined }[];
	restore(): void;
} {
	const tall = new Map<string, number>();
	const column = { width: 700 };
	const scrollsIntoView: { id: string | undefined; options: ScrollIntoViewOptions | undefined }[] = [];
	const proto = (window as unknown as { HTMLElement: typeof HTMLElement }).HTMLElement.prototype;
	const changed = ["getBoundingClientRect", "scrollHeight", "scrollTop", "clientHeight", "clientWidth", "scrollIntoView"] as const;
	const before = changed.map((name) => [name, Object.getOwnPropertyDescriptor(proto, name)] as const);
	const restore = () => {
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
	/** How far down the column a row or gap starts. */
	const startOf = (el: HTMLElement): number => {
		let y = 0;
		for (let before = el.previousElementSibling; before !== null; before = before.previousElementSibling) y += heightOf(before as HTMLElement);
		return y;
	};
	const scrollerOf = (el: Element) => el.closest(".scroller") as HTMLElement | null;
	const box = (top: number, height: number) => ({ top, bottom: top + height, height, left: 0, right: 0, width: 0, x: 0, y: top, toJSON() {} }) as DOMRect;
	proto.getBoundingClientRect = function (this: HTMLElement) {
		if (is(this, "scroller")) return box(0, view);
		const parent = this.parentElement;
		if (parent !== null && is(parent, "tape-column") && (is(this, "tape-row") || is(this, "tape-gap"))) return box(startOf(this) - (scrollerOf(this)?.scrollTop ?? 0), heightOf(this));
		// Off to the side, where nothing is in view.
		if (parent !== null && is(parent, "tape-probe")) return box(-1_000_000, heightOf(this));
		return own.call(this);
	};
	Object.defineProperty(proto, "scrollHeight", {
		configurable: true,
		get(this: HTMLElement) {
			if (!is(this, "scroller")) return 0;
			let total = 0;
			for (const part of this.querySelectorAll<HTMLElement>(".tape-column > .tape-row, .tape-column > .tape-gap")) total += heightOf(part);
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
			scrolled.set(this, Math.max(0, Math.min(to, this.scrollHeight - this.clientHeight)));
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
	// A row's height as laid out, which is how a row being measured off to the side is read.
	const styled = window.getComputedStyle.bind(window);
	window.getComputedStyle = ((el: Element, pseudo?: string | null) => {
		const style = styled(el, pseudo);
		if (!is(el, "tape-row")) return style;
		return new Proxy(style, { get: (target, key) => (key === "height" ? `${heightOf(el as HTMLElement)}px` : Reflect.get(target, key)) });
	}) as typeof window.getComputedStyle;
	// A row scrolled into view, as the engine does it at once: to the middle, or just far enough to show.
	proto.scrollIntoView = function (this: HTMLElement, options?: ScrollIntoViewOptions | boolean) {
		const scroller = scrollerOf(this);
		const target = this.closest<HTMLElement>(".tape-column > .tape-row");
		const asked = typeof options === "object" ? options : undefined;
		scrollsIntoView.push({ id: target?.dataset["eventId"], options: asked });
		if (scroller === null || target === null) return;
		const start = startOf(target);
		const height = heightOf(target);
		if (asked?.block === "center") scroller.scrollTop = Math.max(0, start + height / 2 - view / 2);
		else if (start < scroller.scrollTop) scroller.scrollTop = start;
		else if (start + height > scroller.scrollTop + view) scroller.scrollTop = start + height - view;
		scroller.dispatchEvent(new window.Event("scroll") as unknown as Event);
	};
	return { tall, column, scrollsIntoView, restore };
}
