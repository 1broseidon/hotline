import { type ReactNode, useLayoutEffect, useRef, useState } from "react";
import type { PaneKind } from "../../threadKinds";
import { Avatar } from "../../ui/Avatar";
import { FollowWindow } from "../FollowWindow";
import { Markdown } from "../Markdown";

/*
 * The frame every thread's pane is drawn in (docs/design/thread-panes.html):
 * a head that says what the thread is and what the person is to it, the task
 * or request it was handed, its work as it happens, and what came of it. Each
 * kind's pane is these parts in its own order; none of them knows the kind.
 */

/**
 * The band's heading on a pane of its own: the kind, glyph and word, in the
 * place the state used to stand, with the dot saying the state (beating while
 * it runs, quiet once it rests, red when it failed). The band's instrument
 * already says how long and how much, so the word is the kind, and the state
 * is kept for a reader in the heading's name.
 */
export function BandKind({ kind, live, tone, state }: { kind: PaneKind; live: boolean; tone?: "done" | "quiet" | "failed" | undefined; state: string }) {
	return (
		<h2 className="flex min-w-0 flex-1 items-center gap-2 truncate pl-1 font-mono text-sm font-medium text-ink" aria-label={`${kind.word}, ${state}`}>
			<span aria-hidden="true" className={`run-dot ${live ? "beat" : ""}`} data-tone={live ? "live" : (tone ?? "done")} />
			<span className="band-kind truncate">
				{kind.glyph !== "" && <span aria-hidden="true">{kind.glyph} </span>}
				{kind.word}
			</span>
		</h2>
	);
}

/** Who handed a thread its task, as its card names them. */
export type From = { personaId: string; name: string; hash?: string | undefined };

/**
 * Under the band: the kind's label and a plain fact beside it where one is
 * needed ("Read only", "Started by Poe"), the title, and one meta line of facts
 * joined by " · ", empty ones left out. The band keeps the state and the
 * keys; this says what the thread is.
 */
export function PaneHead({
	kind,
	family,
	title,
	meta = [],
	kindInBand = false,
}: {
	kind: PaneKind;
	/** The band already says the kind ([`BandKind`]): the head leaves its label out, and its fact leads the meta line. */
	kindInBand?: boolean;
	/** Said in place of the kind's own family line: a job's "Loop", a command's "Started by Poe". */
	family?: string | undefined;
	/** None where the band already carries it. */
	title?: string | undefined;
	meta?: ReadonlyArray<string | false | null | undefined>;
}) {
	const fact = family ?? kind.family;
	const facts = [kindInBand && fact, ...meta].filter((part): part is string => typeof part === "string" && part !== "");
	return (
		<header className="pane-head">
			{!kindInBand && <p className="pane-head-kind">
				<span className="pane-kind" data-watching={kind.watching || undefined}>
					{kind.glyph !== "" && <span aria-hidden="true">{kind.glyph}</span>}
					{kind.word}
				</span>
				{fact !== "" && <span className="pane-family">{fact}</span>}
			</p>}
			{title !== undefined && title !== "" && <h3 className="pane-title selectable">{title}</h3>}
			{facts.length > 0 && <p className="pane-meta selectable">{facts.join(" · ")}</p>}
		</header>
	);
}

/**
 * What the thread was handed, and by whom: a subagent's task, a handoff's
 * request. Three lines of it, with Show all only when there is more.
 */
export function TaskCard({ eyebrow, from, text }: { eyebrow: string; from?: From | undefined; text: string }) {
	const body = useRef<HTMLParagraphElement>(null);
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
	}, [text, open]);
	return (
		<section className="pane-task" aria-label={eyebrow}>
			<p className="pane-task-head">
				<span className="eyebrow">{eyebrow}</span>
				{from !== undefined && (
					<span className="pane-task-from">
						<Avatar id={from.personaId} name={from.name} size={16} hash={from.hash} />
						from {from.name}
					</span>
				)}
			</p>
			<p ref={body} className="pane-task-body selectable" data-open={open || undefined}>
				{text}
			</p>
			{clipped && (
				<button type="button" className="pane-task-more" aria-expanded={open} onClick={() => setOpen((was) => !was)}>
					{open ? "Show less" : "Show all"}
				</button>
			)}
		</section>
	);
}

/**
 * The thread's work as it happens, steps and words in one stream: it follows
 * the newest until the person scrolls up, and Latest takes them back down.
 */
export function LiveStream({
	following,
	count,
	content,
	live,
	children,
}: {
	/** What the stream is of: a new one starts at its newest. */
	following: string | undefined;
	count: number;
	/** Changes when what is shown changes, including words growing in place. */
	content: unknown;
	/** The work is still going. */
	live: boolean;
	children: ReactNode;
}) {
	return (
		<FollowWindow following={following} count={count} content={content} frame="pane-stream" back="Latest" live={live}>
			{children}
		</FollowWindow>
	);
}

/** What came of the work, closing the stream: an eyebrow ("Result · sent to Poe"), and the words. */
export function ResultCard({ eyebrow, aside, text }: { eyebrow: string; aside?: string | undefined; text: string }) {
	return (
		<section className="pane-result" aria-label={eyebrow}>
			<p className="pane-result-head eyebrow">
				<span>{eyebrow}</span>
				{aside !== undefined && <span>{aside}</span>}
			</p>
			<div className="pane-result-body selectable">
				<Markdown text={text} />
			</div>
		</section>
	);
}
