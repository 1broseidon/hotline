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

/** Who handed a thread its task, as its card names them. */
export type From = { personaId: string; name: string; hash?: string | undefined };

/**
 * Under the band: the kind's label and its family line ("Read only", "You
 * can step in", "You're in it"), the title, and one meta line of facts
 * joined by " · ", empty ones left out. The band keeps the state and the
 * keys; this says what the thread is.
 */
export function PaneHead({
	kind,
	family,
	title,
	meta = [],
}: {
	kind: PaneKind;
	/** Said in place of the kind's own family line: a job's "Loop", a command's "Left running by Poe". */
	family?: string | undefined;
	/** None where the band already carries it. */
	title?: string | undefined;
	meta?: ReadonlyArray<string | false | null | undefined>;
}) {
	const facts = meta.filter((part): part is string => typeof part === "string" && part !== "");
	return (
		<header className="pane-head">
			<p className="pane-head-kind">
				<span className="pane-kind" data-watching={kind.watching || undefined}>
					{kind.glyph !== "" && <span aria-hidden="true">{kind.glyph}</span>}
					{kind.word}
				</span>
				<span className="pane-family">{family ?? kind.family}</span>
			</p>
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
