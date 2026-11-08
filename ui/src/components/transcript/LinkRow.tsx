import type { Person } from "../../avatars";
import type { LinkEvent } from "../../dock";
import { linkLine, linkRow } from "../../links";

const KIND = { thread: "Thread", subagent: "Subagent", command: "Command", pair: "Pair", call: "Call" } as const;

/**
 * Every way out of the conversation into a thread it holds, a work thread, a
 * subagent's run, a command left running, a pair or a call, as one quiet
 * line: a dot for how it stands, the title, the kind, what came of it, and Open on a hover. No glyph: the chip
 * above the conversation already shows it. The one-sentence
 * `linkLine` is its accessible name, so a reader hears it as it always read.
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
				<span className="link-row-sub">
					<span className="link-row-kind">{KIND[facts.kind]}</span>
					{facts.outcome !== "" && <span className="link-row-outcome">· {facts.outcome}</span>}
				</span>
			</span>
			<span className="link-row-open">Open ›</span>
		</button>
	);
}
