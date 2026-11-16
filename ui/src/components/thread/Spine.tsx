import { type CSSProperties, type ReactNode, useLayoutEffect, useRef, useState } from "react";
import type { TranscriptEvent } from "../../generated/contract";

/*
 * The right-hand pane's one skeleton (ui/design.md, "The right-hand pane"):
 * every page of the pane draws its head and its sections from these, so the
 * eye learns the page once. Two columns: a narrow gutter for marks, and one
 * text edge every word starts on; times and counts stand on one right edge.
 */

/**
 * The one mark vocabulary, for things in order on the pane's line: done is a
 * filled dot, waiting a ring, live a mint dot that breathes, expired a dotted
 * ring, failed a cross. Stopped is done in a dimmer ink: it ended, by
 * someone's hand or its own.
 */
export type MarkKind = "done" | "waiting" | "live" | "expired" | "failed" | "stopped";

export function Mark({ kind, blocked = false }: { kind: MarkKind; blocked?: boolean }) {
	return <span aria-hidden="true" className={`spine-mark ${kind === "live" ? "beat" : ""}`} data-mark={kind} data-blocked={blocked || undefined} />;
}

/**
 * How the page's live work stands, as the one status mark on its section's
 * heading says it: active (mint, breathing), blocked on the person (an amber
 * ring), stopped or ended (a dim dot), failed (a red cross).
 */
export type Status = "live" | "blocked" | "stopped" | "failed";

/** The words a reader hears for the status mark, which is drawn, not written. */
const STATUS_WORDS: Record<Status, string> = { live: "Active", blocked: "Needs you", stopped: "Ended", failed: "Failed" };

/** A card in the stream still waits on the person: the work is blocked on them. */
export function waitsOnPerson(events: readonly TranscriptEvent[], since = 0): boolean {
	return events.some(
		(event) =>
			event.ts >= since &&
			((event.kind === "human_action" && event.status === "pending") || (event.kind === "permission" && event.decision === undefined) || (event.kind === "credential_ask" && event.status === "pending")),
	);
}

/** How many lines words someone wrote show before Show all: an item's message and replies, a helper's task, an old conversation's lines. The pane's styles read it (`--clamp-lines`). */
const CLAMP_LINES = 6;

/**
 * Words someone wrote, at most `CLAMP_LINES` lines of them, so an email-sized
 * message never pushes the rest of the page away; Show all opens it in
 * place, Show less folds it again. Measured, so a short one has no key.
 */
export function Clamped({ children, className = "", watch }: { children: ReactNode; className?: string; /** What, changing, means the lines are measured again. */ watch: unknown }) {
	const body = useRef<HTMLDivElement>(null);
	const [open, setOpen] = useState(false);
	const [clipped, setClipped] = useState(false);
	// Measured while clamped, and again when the pane's width moves the lines.
	// biome-ignore lint/correctness/useExhaustiveDependencies: new words are new lines to measure.
	useLayoutEffect(() => {
		const el = body.current;
		if (el === null || open) return;
		const measure = () => setClipped(el.scrollHeight > el.clientHeight + 1);
		measure();
		if (typeof ResizeObserver === "undefined") return;
		const resized = new ResizeObserver(measure);
		resized.observe(el);
		return () => resized.disconnect();
	}, [watch, open]);
	return (
		<>
			<div
				ref={body}
				className={className === "" ? "spine-clamp selectable" : `spine-clamp selectable ${className}`}
				style={{ "--clamp-lines": CLAMP_LINES } as CSSProperties}
				data-open={open || undefined}
				data-clipped={(clipped && !open) || undefined}
			>
				{children}
			</div>
			{(clipped || open) && (
				<button type="button" className="spine-more" aria-expanded={open} onClick={() => setOpen((was) => !was)}>
					{open ? "Show less" : "Show all"}
				</button>
			)}
		</>
	);
}

/**
 * Under the title, on every page: the state in a word or two, a detail after
 * it, and at the right edge a time (when it moved, or how long it ran). No
 * dot: the one status mark is on the heading of the section that holds the
 * live work (`SectionRule`). Waiting on the person, the word is in amber.
 */
export function StateLine({ word, detail, time, you = false }: { word: string; detail?: string | undefined; time?: string | undefined; /** Waiting on the person: the word in the amber that asks. */ you?: boolean }) {
	return (
		<p className="spine-state" data-you={you || undefined}>
			{word !== "" && <span className="spine-state-word">{word}</span>}
			{detail !== undefined && detail !== "" && (
				<span className="spine-state-detail">
					{word !== "" ? "· " : ""}
					{detail}
				</span>
			)}
			{time !== undefined && <span className="spine-time">{time}</span>}
		</p>
	);
}

/**
 * A page's head: its title on the text edge, the state line under it, then
 * what else belongs to the head (a path's shape, a list's search and
 * chips). Spacing, not a hairline, parts it from what follows.
 */
export function SpineHead({ title, state, children, className = "" }: { title?: ReactNode; state?: ReactNode; children?: ReactNode; className?: string }) {
	return (
		<header className={className === "" ? "spine-head" : `spine-head ${className}`}>
			{title !== undefined && <h3 className="spine-title selectable">{title}</h3>}
			{state}
			{children}
		</header>
	);
}

/** A fact of a pane: a label and its value, on one line. An empty value leaves its row out. */
type Fact = readonly [label: string, value: string | undefined];
type FactSlot = Fact | false | undefined;
/** A pane's facts: three at most, so the type will not take a fourth. */
export type FactRows = readonly [] | readonly [FactSlot] | readonly [FactSlot, FactSlot] | readonly [FactSlot, FactSlot, FactSlot];

/**
 * Facts, not events: a mono label on the text edge and its value after it,
 * one line each, and never more than three (the title and the state line
 * say the rest).
 */
export function Facts({ rows }: { rows: FactRows }) {
	const shown = (rows as readonly FactSlot[]).filter((row): row is readonly [string, string] => row !== false && row !== undefined && row[1] !== undefined && row[1] !== "");
	if (shown.length === 0) return null;
	return (
		<dl className="spine-facts">
			{shown.map(([label, value]) => (
				<div key={label}>
					<dt>{label}</dt>
					<dd className="selectable">{value}</dd>
				</div>
			))}
		</dl>
	);
}

/**
 * A section's name on a hairline that runs to the right edge, its count at
 * the rule's end. The section that holds the page's live work carries the
 * page's one status mark in the gutter beside its name (`status`).
 */
export function SectionRule({
	title,
	count,
	aside,
	status,
	className = "",
	level = 4,
}: {
	title: string;
	count?: number | string | undefined;
	aside?: ReactNode;
	status?: Status | undefined;
	className?: string;
	/** A list's groups are under the band's own heading; a page's sections under its title. */
	level?: 3 | 4;
}) {
	const Heading = level === 3 ? "h3" : "h4";
	return (
		<Heading className={className === "" ? "spine-rule" : `spine-rule ${className}`} data-status={status}>
			{status !== undefined && (
				<span className="spine-rule-mark" role="img" aria-label={STATUS_WORDS[status]}>
					<Mark kind={status === "blocked" ? "waiting" : status} blocked={status === "blocked"} />
				</span>
			)}
			<span className="spine-rule-title">{title}</span>
			<span aria-hidden="true" className="spine-rule-line" />
			{aside}
			{count !== undefined && <span className="spine-rule-count">{count}</span>}
		</Heading>
	);
}

/** The page's one action, pinned at its foot under a hairline: a line of fact and the key. */
export function SpineFoot({ children, action }: { children?: ReactNode; action?: ReactNode }) {
	return (
		<footer className="spine-foot">
			<p className="min-w-0 flex-1">{children}</p>
			{action}
		</footer>
	);
}
