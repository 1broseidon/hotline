import { RECEIVER, RECEIVER_SMALL, receiverBox, receiverPath } from "./receiver";

/**
 * The mark: two eyes on a body, cut off by the waterline it sits in, with
 * the receiver resting across the eyes — the eyes are where a desk phone's
 * handset sits in its cradle. One colour and one shape, so it sits in text
 * at any size; the pupils are punched through and the handset is cut from
 * the eyes by a gap, so there is never a second fill to keep in step. At
 * 24px and under it draws the small receiver, whose cut survives as a
 * pixel and whose handset is slim and straight, the same drawing the working glyph
 * rests in. `plain` is the toad alone, off the hook: the app's own chrome
 * (the titlebar, and the tray from assets/hotline-mark-plain.svg) wears it,
 * because there the mark is the app's name, not a teammate at work. The
 * source of truth is assets/hotline-mark.svg (and -small),
 * and the app tile is the same drawing.
 *
 * Decorative by default. It takes a label only where it is the only thing
 * naming the app on screen; beside a title that already says Hotline, a second
 * announcement of the same word is noise.
 */
/** The toad alone: its eyes' tops to the waterline. */
const PLAIN_BOX = { x: 4, y: 19.5, w: 56, h: 28.5 };

export function HotlineMark({ className, label, width = 20, plain = false }: { className?: string; label?: string; width?: number; plain?: boolean }) {
	const g = width <= 24 ? RECEIVER_SMALL : RECEIVER;
	const box = plain ? PLAIN_BOX : receiverBox(g);
	return (
		<svg
			className={className}
			viewBox={`${box.x} ${box.y} ${box.w} ${box.h}`}
			width={width}
			height={(width * box.h) / box.w}
			role={label ? "img" : undefined}
			aria-label={label}
			aria-hidden={label ? undefined : true}
			focusable="false"
		>
			<mask id="hotline-mark-pupils" maskUnits="userSpaceOnUse" x="0" y="0" width="64" height="64">
				<rect width="64" height="64" fill="#fff" />
				<rect x="14.5" y="28" width="11" height="4" rx="2" fill="#000" />
				<rect x="38.5" y="28" width="11" height="4" rx="2" fill="#000" />
			</mask>
			<g fill="currentColor">
				<g mask="url(#hotline-mark-pupils)">
					<rect x="4" y="30" width="56" height="18" rx="6" />
					<circle cx="20" cy="30" r="10.5" />
					<circle cx="44" cy="30" r="10.5" />
				</g>
				{!plain && <path d={receiverPath(g)} />}
			</g>
		</svg>
	);
}

