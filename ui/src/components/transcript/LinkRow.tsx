import type { Person } from "../../avatars";
import type { LinkEvent } from "../../dock";
import { linkLine, linkRow } from "../../links";

/**
 * Every way out of the conversation into a thread it holds, a work thread, a
 * pair or a call, as one quiet line (a helper or a command has none: it is
 * in the path of the turn that started it): a dot for how it stands, the title, what it is doing or what came of
 * it, and Open on a hover. No glyph: the chip above the conversation already
 * shows it. No kind word either: the conversation says what the work is and
 * how it went, and the kind is where the person goes looking for detail (the
 * opened thread's header, the Threads list). The one-sentence `linkLine`,
 * which names the kind, is its accessible name: heard alone, without the row
 * around it, "Side thread" or "Call" says what Open will open.
 */
export function LinkRow({
	link,
	owner,
	people,
	working = false,
	onOpen,
}: {
	link: LinkEvent;
	owner: string;
	people: ReadonlyMap<string, Person> | undefined;
	working?: boolean;
	onOpen(): void;
}) {
	const facts = linkRow(link, owner, people, working);
	return (
		<button type="button" className="link-row" data-kind={facts.kind} aria-label={`${linkLine(link, owner, people, working)}. Open it`} onClick={onOpen}>
			<span className={`link-row-dot ${facts.tone === "live" ? "beat" : ""}`} data-tone={facts.tone} />
			<span className="link-row-words">
				<span className="link-row-title">{facts.title}</span>
				{facts.outcome !== "" && <span className="link-row-outcome">· {facts.outcome}</span>}
			</span>
			<span className="link-row-open">Open ›</span>
		</button>
	);
}

/**
 * A job's post, headed by the job's line in the same shape: the job's name,
 * "posted", and Open on a hover, into the job's thread. The words posted are
 * the bubble under it; this is the way to the run that wrote them.
 */
export function JobPostRow({ title, onOpen }: { title: string; onOpen(): void }) {
	return (
		<button type="button" className="link-row" data-kind="job" aria-label={`Job · ${title} · posted. Open it`} onClick={onOpen}>
			<span className="link-row-dot" data-tone="done" />
			<span className="link-row-words">
				<span className="link-row-title">{title}</span>
				<span className="link-row-outcome">· posted</span>
			</span>
			<span className="link-row-open">Open ›</span>
		</button>
	);
}
