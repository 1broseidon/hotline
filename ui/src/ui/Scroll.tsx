import { useEffect, useRef, useState, type PointerEvent, type ReactNode, type RefObject } from "react";

/** A thumb shorter than this is a target nobody can catch. */
const MIN_THUMB = 32;
/** How long the thumb stays after the last scroll before it fades. */
const LINGER_MS = 900;

/**
 * A scroller that draws its own thumb.
 *
 * The toolkit's bar is hidden, because WebKitGTK ignores what a page says
 * about scrollbars and draws Adwaita's 21px trough with a border down the
 * side of every pane, and the other platforms each draw something else.
 * This is the one moving part that makes the bar the same six pixels
 * everywhere. Scrolling itself stays native — wheel, keys, touch, and
 * scrollIntoView all act on the inner element — and the thumb only reads
 * that element back: where it is, how much of it shows. Dragging the thumb
 * and pressing the track are the two things a hand expects of one. The
 * thumb shows while the content moves and for a moment after, and under
 * a pointer near the edge; at rest it is not there, because a bar that is
 * always there is a frame, and the words are the frame.
 *
 * `scrollerRef` hands the inner element to a caller that needs it, the way
 * the transcript pins itself to the latest line.
 */
export function Scroll({
	className,
	scrollerRef,
	children,
}: {
	className?: string;
	scrollerRef?: RefObject<HTMLDivElement | null>;
	children: ReactNode;
}) {
	const own = useRef<HTMLDivElement>(null);
	const ref = scrollerRef ?? own;
	const [thumb, setThumb] = useState<{ top: number; height: number } | null>(null);
	const [dragging, setDragging] = useState(false);
	const [moving, setMoving] = useState(false);
	const drag = useRef<{ fromY: number; fromTop: number } | null>(null);
	const linger = useRef<number | undefined>(undefined);

	useEffect(() => {
		const el = ref.current;
		if (!el) return;
		const measure = () => {
			const { scrollHeight, clientHeight, scrollTop } = el;
			if (scrollHeight <= clientHeight + 1) {
				setThumb(null);
				return;
			}
			const height = Math.max(MIN_THUMB, (clientHeight / scrollHeight) * clientHeight);
			const top = (scrollTop / (scrollHeight - clientHeight)) * (clientHeight - height);
			setThumb({ top, height });
		};
		const onScroll = () => {
			measure();
			setMoving(true);
			window.clearTimeout(linger.current);
			linger.current = window.setTimeout(() => setMoving(false), LINGER_MS);
		};
		measure();
		el.addEventListener("scroll", onScroll, { passive: true });
		// Content lays out after it lands, so the height changes without a
		// scroll event; the first child is the column that grows.
		const observer = new ResizeObserver(measure);
		observer.observe(el);
		if (el.firstElementChild) observer.observe(el.firstElementChild);
		return () => {
			el.removeEventListener("scroll", onScroll);
			observer.disconnect();
			window.clearTimeout(linger.current);
		};
	}, [ref]);

	/** How far the content moves for one pixel of thumb. */
	const ratio = (el: HTMLDivElement, height: number) =>
		(el.scrollHeight - el.clientHeight) / Math.max(1, el.clientHeight - height);

	const onThumbDown = (event: PointerEvent<HTMLDivElement>) => {
		const el = ref.current;
		if (!el || event.button !== 0) return;
		event.preventDefault();
		event.stopPropagation();
		event.currentTarget.setPointerCapture(event.pointerId);
		drag.current = { fromY: event.clientY, fromTop: el.scrollTop };
		setDragging(true);
	};
	const onThumbMove = (event: PointerEvent<HTMLDivElement>) => {
		const el = ref.current;
		if (!el || drag.current === null || thumb === null) return;
		el.scrollTop = drag.current.fromTop + (event.clientY - drag.current.fromY) * ratio(el, thumb.height);
	};
	const onThumbUp = () => {
		drag.current = null;
		setDragging(false);
	};
	/* A press on the track puts the thumb under the pointer. */
	const onTrackDown = (event: PointerEvent<HTMLDivElement>) => {
		const el = ref.current;
		if (!el || thumb === null || event.button !== 0) return;
		event.preventDefault();
		const y = event.clientY - event.currentTarget.getBoundingClientRect().top;
		el.scrollTop = (y - thumb.height / 2) * ratio(el, thumb.height);
	};

	return (
		<div className={`scroll-host ${className ?? ""}`}>
			<div ref={ref} className="scroller">
				{children}
			</div>
			{thumb !== null && (
				<div className="scroll-track" onPointerDown={onTrackDown}>
					<div
						className={`scroll-thumb ${dragging ? "scroll-thumb-held" : ""} ${moving ? "scroll-thumb-shown" : ""}`}
						style={{ top: thumb.top, height: thumb.height }}
						onPointerDown={onThumbDown}
						onPointerMove={onThumbMove}
						onPointerUp={onThumbUp}
						onPointerCancel={onThumbUp}
					/>
				</div>
			)}
		</div>
	);
}
