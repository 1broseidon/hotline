/**
 * The receiver: the handset that rests on the mark's eyes.
 *
 * The eyes already sit where a desk phone's earpiece and mouthpiece rest in
 * the cradle, so the handset is drawn around them rather than on its own
 * grid: each cup is an arc concentric with an eye, set out by a gap and as
 * thick as the handset, and the handle bridges the two cup tops. On the
 * large drawing it sits a little lower than they stand, so the cups read as
 * earcups on a headband as well as the two ends of a receiver; on the small
 * one, which the working glyph moves, it runs straight across, because a dip
 * in the middle is one more thing every pose would have to carry. The lips hang past each eye's shoulder;
 * they are what says "phone" at every size.
 *
 * One colour, one fill: the gap is the cut. At 16px a 2.4-unit cut is 0.6px
 * and the handset fuses with the eyes into goggles, so there is a small
 * drawing, the brow, with a wider cut and a slim straight handset: at a
 * glance a toad with a heavy brow, the receiver on a second look. That is
 * the one the working glyph moves; the way a typeface has an optical size.
 *
 * assets/hotline-mark.svg and assets/hotline-mark-small.svg are these paths
 * written out; change the numbers here and re-render them.
 */

export type Receiver = { gap: number; thick: number; lips: number; sag: number };

/** The drawing, from 25px up. */
export const RECEIVER: Receiver = { gap: 2.4, thick: 6.5, lips: 166, sag: 1.3 };
/** The brow: the drawing at 24px and under and on the moving glyph, a slim straight handset whose cut survives as a pixel. */
export const RECEIVER_SMALL: Receiver = { gap: 3.4, thick: 3.8, lips: 156, sag: 0 };


const LEFT = 20;
const RIGHT = 44;
const Y = 30;
const EYE = 10.5;
const rad = (deg: number) => (deg * Math.PI) / 180;
const n = (v: number) => +v.toFixed(2);
const at = (cx: number, r: number, deg: number): [number, number] => [cx + r * Math.cos(rad(deg)), Y - r * Math.sin(rad(deg))];
const pt = (p: [number, number]) => `${n(p[0])} ${n(p[1])}`;

/** The handset as one filled outline, resting on the cradle. */
export function receiverPath(g: Receiver): string {
	const r = EYE + g.gap;
	const R = r + g.thick;
	const e = g.thick / 2;
	// Where the outer arc meets the handle, which sits `sag` below the cup tops.
	const join = (Math.asin(Math.min(1, (R - g.sag) / R)) * 180) / Math.PI;
	const outerL = at(LEFT, R, g.lips);
	const innerL = at(LEFT, r, g.lips);
	const outerR = at(RIGHT, R, 180 - g.lips);
	const innerR = at(RIGHT, r, 180 - g.lips);
	return (
		`M${pt(outerL)}A${n(R)} ${n(R)} 0 0 1 ${pt(at(LEFT, R, join))}` +
		`L${pt(at(RIGHT, R, 180 - join))}A${n(R)} ${n(R)} 0 0 1 ${pt(outerR)}` +
		`A${n(e)} ${n(e)} 0 0 1 ${pt(innerR)}A${n(r)} ${n(r)} 0 0 0 ${RIGHT} ${n(Y - r)}` +
		`L${LEFT} ${n(Y - r)}A${n(r)} ${n(r)} 0 0 0 ${pt(innerL)}A${n(e)} ${n(e)} 0 0 1 ${pt(outerL)}Z`
	);
}

/** The drawing's box, for a viewBox that crops to the ink. */
export function receiverBox(g: Receiver): { x: number; y: number; w: number; h: number } {
	const lip = LEFT + (EYE + g.gap + g.thick / 2) * Math.cos(rad(g.lips)) - g.thick / 2;
	const x = n(Math.min(4, lip - 0.2));
	const y = n(Y - (EYE + g.gap + g.thick) - 0.3);
	return { x, y, w: n(64 - 2 * x), h: n(48 - y) };
}

/**
 * The handset's centreline, lip to lip, as `count` evenly spaced points. A
 * round-capped stroke this wide along it is the handset, near enough that the
 * swap from fill to stroke does not show while it is moving; that stroke is
 * what bends into the reply.
 */
export function receiverLine(g: Receiver, count: number): { points: [number, number][]; centre: [number, number] } {
	const r = EYE + g.gap;
	const mid = r + g.thick / 2;
	const arc = mid * rad(g.lips - 90);
	const span = RIGHT - LEFT;
	const total = 2 * arc + span;
	const points: [number, number][] = [];
	for (let i = 0; i < count; i++) {
		const s = (total * i) / (count - 1);
		if (s < arc) points.push(at(LEFT, mid, g.lips - (s / mid) * (180 / Math.PI)));
		else if (s < arc + span) {
			const x = LEFT + (s - arc);
			points.push([x, Y - mid + (g.sag / 2) * Math.sin((Math.PI * (x - LEFT)) / span)]);
		} else points.push(at(RIGHT, mid, 90 - ((s - arc - span) / mid) * (180 / Math.PI)));
	}
	const top = Y - r - g.thick;
	const bottom = Y - mid * Math.sin(rad(g.lips)) + g.thick / 2;
	return { points, centre: [32, (top + bottom) / 2] };
}
