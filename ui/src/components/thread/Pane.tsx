import type { ReactNode } from "react";
import type { PaneKind } from "../../threadKinds";
import { Avatar } from "../../ui/Avatar";
import { FollowWindow } from "../FollowWindow";
import { Markdown } from "../Markdown";
import { Clamped, type FactRows, Facts, SectionRule, SpineHead } from "./Spine";

/*
 * The frame every thread's pane is drawn in (docs/design/thread-panes.html),
 * on the pane's one skeleton (./Spine.tsx): a head that says what the thread
 * is and how it stands, the task or request it was handed, its work as it
 * happens, and what came of it. Each kind's pane is these parts in its own
 * order; none of them knows the kind.
 */

/**
 * The band's heading on a pane of its own: the kind's word alone, starting
 * on the text edge. The state is the state line's under the band, so the
 * band carries no dot or glyph; the state is kept for a reader in the
 * heading's name.
 */
export function BandKind({ kind, state }: { kind: PaneKind; state: string }) {
	return (
		<h2 className="band-kind min-w-0 flex-1 truncate pl-1 font-mono text-sm font-medium text-ink" aria-label={`${kind.word}, ${state}`}>
			{kind.word}
		</h2>
	);
}

/** Who handed a thread its task, as its card names them. */
export type From = { personaId: string; name: string; hash?: string | undefined };

/**
 * Under the band, on every thread's pane: the title on the text edge, the
 * state line under it (`StateLine`), then at most three facts as rows of a
 * label and its value. Spacing, not a hairline, closes the head.
 */
export function PaneHead({ title, line, facts = [] }: { title: string; line: ReactNode; facts?: FactRows }) {
	return (
		<>
			<SpineHead title={title} state={line} />
			<Facts rows={facts} />
		</>
	);
}

/**
 * What the thread was handed, and by whom: a subagent's task, a handoff's
 * request, an item's message, the inbox message a work thread was picked up
 * from, which `onOpen` opens in the pane. Its words at reading size on a
 * rule, real bullets where it has a list; six lines of it, with Show all
 * only when there is more (`Clamped`).
 */
export function TaskCard({
	eyebrow,
	from,
	text,
	aside,
	onOpen,
}: {
	eyebrow: string;
	from?: From | undefined;
	text: string;
	/** Said at the rule's end in place of the face: an item's who and what. */
	aside?: ReactNode;
	/** Opens what the words came from, by "Open ›" at the rule's end: a work thread's message. */
	onOpen?: (() => void) | undefined;
}) {
	return (
		<section className="pane-task" aria-label={eyebrow}>
			<SectionRule
				title={eyebrow}
				aside={
					<>
						{aside ??
							(from !== undefined && (
								<span className="pane-task-from">
									<Avatar id={from.personaId} name={from.name} size={16} hash={from.hash} />
									{from.name}
								</span>
							))}
						{onOpen !== undefined && (
							<button type="button" className="pane-task-open" title="Open the message" onClick={onOpen}>
								Open ›
							</button>
						)}
					</>
				}
			/>
			<Clamped className="spine-said" watch={text}>
				<Markdown text={text} />
			</Clamped>
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

/** What came of the work, closing the stream: a section ("Result", how long at the rule's end), and the words on a rule. */
export function ResultCard({ eyebrow, aside, text }: { eyebrow: string; aside?: string | undefined; text: string }) {
	return (
		<section className="pane-result" aria-label={eyebrow}>
			<SectionRule title={eyebrow} count={aside} />
			<div className="spine-said selectable">
				<Markdown text={text} />
			</div>
		</section>
	);
}
