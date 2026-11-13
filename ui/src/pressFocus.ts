/**
 * A key pressed with a pointer does not keep the focus, the way a desktop
 * button does not. Left to the web's default it would, and the next
 * keystroke — Escape closing the pane it opened — promotes that quiet
 * pointer focus to a keyboard ring on a key nobody is looking at. Fields
 * keep theirs: a pointer in a field is where typing goes.
 *
 * A key that can be dragged (a teammate in the rail) keeps its press,
 * because a press the page cancels never starts a drag, and lets go of the
 * focus when the press ends instead.
 */
export function keepFocusOffPressedKeys(target: Window = window): () => void {
	const press = (event: MouseEvent) => {
		const pressed = event.target as Element | null;
		const key = pressed?.closest?.<HTMLElement>("button, [role='button']");
		if (!key || pressed?.closest("input, textarea, [contenteditable='true']")) return;
		if (key.getAttribute("draggable") === "true") {
			target.addEventListener("mouseup", () => target.document.activeElement === key && key.blur(), { once: true, capture: true });
			return;
		}
		event.preventDefault();
	};
	target.addEventListener("mousedown", press);
	return () => target.removeEventListener("mousedown", press);
}
