/**
 * A press puts its tooltip away. WebKitGTK (Linux) keeps a `title` tooltip
 * on screen after the element under it is clicked, so it stands over the
 * menu or pane the click opened until the pointer leaves. On a press the
 * title is taken off the element, and given back when the pointer leaves
 * it; an element whose press opened something it still holds open
 * (`aria-expanded`) gets its title back from React as it closes.
 */
export function quietTooltipsOnPress(target: Document = document): () => void {
	const held = new WeakMap<Element, string>();
	const press = (event: Event) => {
		const element = (event.target as Element | null)?.closest?.("[title]");
		if (!element) return;
		const title = element.getAttribute("title");
		if (title === null || title === "") return;
		held.set(element, title);
		element.removeAttribute("title");
		const leave = () => {
			element.removeEventListener("pointerleave", leave);
			const was = held.get(element);
			held.delete(element);
			if (was === undefined || element.hasAttribute("title")) return;
			if (element.getAttribute("aria-expanded") === "true") return;
			element.setAttribute("title", was);
		};
		element.addEventListener("pointerleave", leave);
	};
	target.addEventListener("pointerdown", press, true);
	return () => target.removeEventListener("pointerdown", press, true);
}
