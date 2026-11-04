import type { Person } from "../../avatars";
import type { LinkEvent } from "../../dock";
import { BranchIcon, PhoneIcon, ThreadIcon } from "../../icons";
import { linkLine, linkRow } from "../../links";

const KIND = { thread: "Thread", subagent: "Subagent", call: "Call" } as const;

/**
 * Every way out of the conversation into a thread it holds, a work thread, a
 * subagent's run or a call, as one row: the kind's glyph with a dot for how
 * it stands, the title, what came of it, and Open. The one-sentence
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
	const Glyph = facts.kind === "call" ? PhoneIcon : facts.kind === "subagent" ? BranchIcon : ThreadIcon;
	return (
		<button type="button" className="link-row" data-kind={facts.kind} aria-label={`${linkLine(link, owner, people, working)}. Open it`} onClick={onOpen}>
			<span className="link-row-glyph">
				<Glyph />
				<span className={`link-row-dot ${facts.tone === "live" ? "beat" : ""}`} data-tone={facts.tone} />
			</span>
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
