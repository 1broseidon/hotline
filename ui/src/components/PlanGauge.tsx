import { type CSSProperties, type RefObject, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { PlanUsage, PlanWindow } from "../generated/contract";
import { gaugeLabel, gaugeWindows, nearLimit, paceTick, planAccount, resetShort, shownWindows, usedShare, windowName, windowNote } from "../plan";
import { Mark } from "../ui/Mark";

/** The ring radii: the outer (weekly) and the inner (5-hour), on an 18px face. */
const RINGS = [7, 3.75];

/**
 * The plan's gauge in the band, beside the teammate's computer: a small
 * round dial, the outer ring the weekly window and the inner the 5-hour,
 * each an arc from twelve o'clock as far round as it is used, over a faint
 * track. Amber, both arcs, once any window is near its limit. Hovered or
 * focused it shows the plan as a card: the agent and the plan's name, then
 * a line per window with its bar, pace tick, use and reset, and under each
 * when it resets and its runway. A click pins the card; a click elsewhere or Escape lets it go.
 * Shown only while the teammate's login has a reading.
 */
export function PlanGauge({
	plan,
	agent,
	mark,
	onOpen,
}: {
	plan: PlanUsage | undefined;
	/** The agent the plan is for, as the card's head names it. */
	agent: string;
	mark: string | null;
	/** Each time the card opens, so a stale reading is read again. */
	onOpen?: (() => void) | undefined;
}) {
	const button = useRef<HTMLButtonElement>(null);
	const card = useRef<HTMLDivElement>(null);
	const id = useId();
	const [pinned, setPinned] = useState(false);
	const [hovered, setHovered] = useState(false);
	const [focused, setFocused] = useState(false);
	const leave = useRef<ReturnType<typeof setTimeout> | null>(null);
	const open = pinned || hovered || focused;
	const rings = gaugeWindows(plan);

	const wasOpen = useRef(false);
	useEffect(() => {
		if (open && !wasOpen.current) onOpen?.();
		wasOpen.current = open;
	}, [open, onOpen]);
	useEffect(() => () => {
		if (leave.current !== null) clearTimeout(leave.current);
	}, []);
	useEffect(() => {
		if (!open) return;
		const away = (event: MouseEvent) => {
			const target = event.target as Node | null;
			if (button.current?.contains(target) || card.current?.contains(target)) return;
			setPinned(false);
			setFocused(false);
			setHovered(false);
		};
		document.addEventListener("mousedown", away, true);
		return () => document.removeEventListener("mousedown", away, true);
	}, [open]);

	if (plan === undefined || rings.length === 0) return null;
	const near = nearLimit(plan);
	const hover = (on: boolean) => {
		if (leave.current !== null) clearTimeout(leave.current);
		leave.current = null;
		if (on) setHovered(true);
		// A moment's grace, so the pointer can cross onto the card.
		else leave.current = setTimeout(() => setHovered(false), 120);
	};
	const shut = () => {
		setPinned(false);
		setHovered(false);
		setFocused(false);
	};
	return (
		<>
			<button
				ref={button}
				type="button"
				className="control btn-icon plan-gauge"
				data-near={near ? "true" : undefined}
				aria-label={gaugeLabel(rings)}
				aria-expanded={open}
				aria-describedby={open ? id : undefined}
				onMouseEnter={() => hover(true)}
				onMouseLeave={() => hover(false)}
				onFocus={() => setFocused(true)}
				onBlur={() => {
					if (!pinned) setFocused(false);
				}}
				onKeyDown={(event) => {
					if (event.key === "Escape" && open) {
						event.preventDefault();
						event.stopPropagation();
						shut();
					}
				}}
				onClick={() => {
					if (pinned) shut();
					else setPinned(true);
				}}
			>
				<Gauge rings={rings} />
			</button>
			{open && button.current && (
				<PlanCard
					ref={card}
					id={id}
					anchor={button.current}
					plan={plan}
					agent={agent}
					mark={mark}
					onHover={hover}
				/>
			)}
		</>
	);
}

/** The dial: a track and an arc per ring, the outer first. */
function Gauge({ rings }: { rings: PlanWindow[] }) {
	return (
		<svg className="plan-gauge-face" width="18" height="18" viewBox="0 0 18 18" aria-hidden="true">
			{rings.map((window, at) => {
				const radius = RINGS[at]!;
				const around = 2 * Math.PI * radius;
				const used = usedShare(window) / 100;
				return (
					<g key={window.kind} data-ring={at === 0 ? "outer" : "inner"} data-used={Math.round(used * 100)}>
						<circle className="plan-gauge-track" cx="9" cy="9" r={radius} />
						{used > 0 && (
							<circle
								className="plan-gauge-arc"
								cx="9"
								cy="9"
								r={radius}
								strokeDasharray={`${around * used} ${around}`}
								transform="rotate(-90 9 9)"
							/>
						)}
					</g>
				);
			})}
		</svg>
	);
}

/** The card the gauge opens: the agent and its plan, then each window's line and runway. */
function PlanCard({
	ref,
	id,
	anchor,
	plan,
	agent,
	mark,
	onHover,
}: {
	ref: RefObject<HTMLDivElement | null>;
	id: string;
	anchor: HTMLElement;
	plan: PlanUsage;
	agent: string;
	mark: string | null;
	onHover(on: boolean): void;
}) {
	const [style, setStyle] = useState<CSSProperties>({ visibility: "hidden" });
	useLayoutEffect(() => {
		const el = ref.current;
		if (!el) return;
		const at = anchor.getBoundingClientRect();
		const width = el.offsetWidth;
		const left = Math.min(Math.max(8, at.right - width), window.innerWidth - width - 8);
		setStyle({ top: at.bottom + 6, left });
	}, [anchor, ref]);
	const kind = planAccount(plan);
	return createPortal(
		<div
			ref={ref}
			id={id}
			role="tooltip"
			className="menu plan-card"
			style={style}
			onMouseEnter={() => onHover(true)}
			onMouseLeave={() => onHover(false)}
		>
			<div className="model-head">
				<Mark id={mark} size={14} />
				<span className="model-head-name">{agent}</span>
				{kind !== "" && <span className="model-head-kind">{kind}</span>}
			</div>
			<PlanRows plan={plan} />
		</div>,
		document.body,
	);
}

/**
 * A line per window: its name, a thin bar, the use and when it resets. A
 * tick on the bar marks where use would be if spread evenly over the
 * window so far; under the line, its reset and runway.
 */
function PlanRows({ plan }: { plan: PlanUsage }) {
	const windows = shownWindows(plan);
	if (windows.length === 0) return null;
	const now = Date.now();
	return (
		<div className="plan-rows">
			{windows.map((window) => {
				const tick = paceTick(window, now);
				const note = windowNote(window, now);
				return (
					<div key={window.kind} className="plan-window" data-status={window.status}>
						<div className="plan-row" data-status={window.status}>
							<span className="plan-name">{windowName(window)}</span>
							<span className="plan-bar" aria-hidden="true">
								<span className="plan-fill" style={{ width: `${usedShare(window)}%` }} />
								{tick !== undefined && <span className="plan-tick" style={{ left: `${tick}%` }} />}
							</span>
							<span className="plan-use">{window.status === "limited" ? "100%" : window.usedPercent !== undefined ? `${Math.round(window.usedPercent)}%` : "–"}</span>
							<span className="plan-reset">{window.resetsAt !== undefined ? resetShort(window.resetsAt, now) : ""}</span>
						</div>
						{note !== undefined && <div className="plan-runway">{note}</div>}
					</div>
				);
			})}
		</div>
	);
}
