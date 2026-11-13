import { type MouseEvent, type PointerEvent, type RefObject, useEffect, useRef, useState } from "react";

/**
 * Moving a pinned face along the pins, or a row along its desk's list, by
 * pressing it and moving the pointer. Pointer events rather than the page's
 * own drag and drop: the window takes the platform's drag sessions for the
 * files dropped on it (Tauri's `dragDropEnabled`, which the composer's file
 * drop needs), so on macOS and Windows alike a page's `dragover` and `drop`
 * never arrive.
 *
 * A press that moves less than `THRESHOLD` is a click and opens the
 * teammate; one that moves further is a drag, and the click that ends it
 * opens nothing. Escape, or the platform taking the pointer away, drops it
 * where it was. A face or a row is a place to drop only for one of its own
 * kind on its own desk.
 */

/** How far a press moves, in pixels, before it is a drag rather than a click. */
const THRESHOLD = 4;
/** Within this many pixels of the list's top or bottom, a drag scrolls it. */
const EDGE = 24;
/** How far each move of the pointer there scrolls it. */
const STEP = 8;

/** A face or a row on the move: where it is (`at`) and where it would land (`over`). */
type Drag = { deskId: string; personaId: string; pinned: boolean; at: number; over: number | null };
type Press = Omit<Drag, "over"> & { pointerId: number; x: number; y: number; moving: boolean; commit(to: number): void };

/** Spread on a face's or a row's button: it can be dragged, and dropped on. */
export type DragProps = {
	"data-drag": "pin" | "row";
	"data-drag-desk": string;
	"data-drag-at": number;
	"data-dragging": true | undefined;
	/** Where the one on the move would land: before this one or after it. */
	"data-over": "before" | "after" | undefined;
	onPointerDown(event: PointerEvent<HTMLElement>): void;
	onPointerMove(event: PointerEvent<HTMLElement>): void;
	onPointerUp(event: PointerEvent<HTMLElement>): void;
	onPointerCancel(): void;
	onClickCapture(event: MouseEvent<HTMLElement>): void;
};

/**
 * `list` is what scrolls when a drag nears its top or bottom; `onStart` is
 * told when a press becomes a drag.
 */
export function useRailDrag(list: RefObject<HTMLElement | null>, onStart: () => void) {
	const [drag, setDrag] = useState<Drag | null>(null);
	const press = useRef<Press | null>(null);
	/* The press that just ended was a drag, so the click it ends with is not a pick. */
	const dragged = useRef(false);
	const dragging = drag !== null;
	useEffect(() => {
		if (!dragging) return;
		const dropIt = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			event.preventDefault();
			event.stopPropagation();
			press.current = null;
			setDrag(null);
		};
		window.addEventListener("keydown", dropIt, true);
		return () => window.removeEventListener("keydown", dropIt, true);
	}, [dragging]);
	const end = () => {
		press.current = null;
		setDrag(null);
	};
	const dragOf = (deskId: string, personaId: string, pinned: boolean, at: number, commit: (to: number) => void): DragProps => {
		const mine = drag !== null && drag.deskId === deskId && drag.pinned === pinned;
		return {
			"data-drag": pinned ? "pin" : "row",
			"data-drag-desk": deskId,
			"data-drag-at": at,
			"data-dragging": (mine && drag.personaId === personaId) || undefined,
			"data-over": mine && drag.personaId !== personaId && drag.over === at ? (at < drag.at ? "before" : "after") : undefined,
			onPointerDown: (event) => {
				dragged.current = false;
				if (event.button !== 0) return;
				// The moves go on reaching this one when the pointer leaves it.
				event.currentTarget.setPointerCapture(event.pointerId);
				press.current = { deskId, personaId, pinned, at, commit, pointerId: event.pointerId, x: event.clientX, y: event.clientY, moving: false };
			},
			onPointerMove: (event) => {
				const held = press.current;
				if (held === null || held.pointerId !== event.pointerId) return;
				if (!held.moving) {
					if (Math.hypot(event.clientX - held.x, event.clientY - held.y) < THRESHOLD) return;
					held.moving = true;
					dragged.current = true;
					onStart();
				}
				scrollNear(list.current, event.clientY);
				const over = slotUnder(event.clientX, event.clientY, held);
				setDrag((was) =>
					was !== null && was.over === over ? was : { deskId: held.deskId, personaId: held.personaId, pinned: held.pinned, at: held.at, over },
				);
			},
			onPointerUp: (event) => {
				const held = press.current;
				if (held === null || held.pointerId !== event.pointerId) return;
				end();
				if (!held.moving) return;
				const over = slotUnder(event.clientX, event.clientY, held);
				if (over !== null && over !== held.at) held.commit(over);
			},
			onPointerCancel: end,
			onClickCapture: (event) => {
				if (!dragged.current) return;
				dragged.current = false;
				event.preventDefault();
				event.stopPropagation();
			},
		};
	};
	return dragOf;
}

/** The slot or place under the pointer that the one pressed may drop on, if any. */
function slotUnder(x: number, y: number, held: Press): number | null {
	const target = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drag-at]");
	if (target == null) return null;
	if (target.dataset["drag"] !== (held.pinned ? "pin" : "row") || target.dataset["dragDesk"] !== held.deskId) return null;
	return Number(target.dataset["dragAt"]);
}

/** A drag near the list's top or bottom scrolls it a step that way, each time the pointer moves. */
function scrollNear(list: HTMLElement | null, y: number) {
	if (list === null) return;
	const box = list.getBoundingClientRect();
	if (y < box.top + EDGE) list.scrollTop -= STEP;
	else if (y > box.bottom - EDGE) list.scrollTop += STEP;
}
