import type { Aside, DeliveryCause, ThreadId } from "../../generated/contract";
import { asideTime, leftHere, levelWord, sourceName, sourceThread, sourceWords } from "../../asides";
import type { Person } from "../../avatars";
import { ChevronRightIcon, PersonPlusIcon } from "../../icons";
import { Avatar } from "../../ui/Avatar";
import { Markdown } from "../Markdown";
import type { ThreadRef } from "../Transcript";

/**
 * An aside as a conversation draws it: a quiet line, never a bubble. In the
 * conversation it was left in (the receiver's main conversation), the `(·)`
 * mark, who it is from and the kind of thread, its time, and the words; it
 * opens the thread it came from. On the thread that left it, whom it went to
 * and at what level, and "Quill was looped in" for the person's Loop in….
 */
export function AsideLine({
	aside,
	personaId,
	here,
	people,
	onOpenThread,
}: {
	aside: Aside;
	/** The teammate whose conversation or thread this is. */
	personaId: string;
	here: ThreadId;
	people: ReadonlyMap<string, Person> | undefined;
	onOpenThread?: ((thread: ThreadRef) => void) | undefined;
}) {
	const nameOf = (id: string) => people?.get(id)?.name ?? "A teammate";
	const when = asideTime(aside.ts);
	const level = levelWord(aside.level);
	const sent = leftHere(aside, personaId, here);
	if (sent && aside.loopedIn === true) {
		return (
			<p className="aside-line" data-level={aside.level}>
				<PersonPlusIcon className="aside-mark" />
				<span className="min-w-0 flex-1">
					<span className="aside-text">{nameOf(aside.to)} was looped in</span>{" "}
					<span className="instrument">
						<span className="aside-level">{level}</span> · {when}
					</span>
				</span>
			</p>
		);
	}
	const who = aside.by !== undefined ? nameOf(aside.by) : sourceName(aside);
	const meta = sent
		? (
			<>
				to {aside.to === personaId ? "main conversation" : nameOf(aside.to)} · <span className="aside-level">{level}</span> · {when}
			</>
		)
		: (
			<>
				from {who} · {sourceWords(aside)}
				{aside.level !== "note" && (
					<>
						{" "}· <span className="aside-level">{level}</span>
					</>
				)}{" "}
				· {when}
			</>
		);
	const body = (
		<>
			<span className="aside-mark instrument" aria-hidden="true">
				(·)
			</span>
			<span className="min-w-0 flex-1">
				<span className="instrument block truncate">{meta}</span>
				<span className="aside-text selectable">{aside.text}</span>
			</span>
		</>
	);
	const from = sourceThread(aside);
	if (sent || from === undefined || onOpenThread === undefined) {
		return (
			<p className="aside-line" data-level={aside.level}>
				{body}
			</p>
		);
	}
	return (
		<button
			type="button"
			className="aside-line"
			data-level={aside.level}
			title="Open its thread"
			onClick={() => onOpenThread({ thread: from })}
		>
			{body}
			<ChevronRightIcon />
		</button>
	);
}

/**
 * A looped-in colleague's one answer, in the thread it was looped into:
 * under its face and name, whole, as the colleague said it.
 */
export function AnswerUnderFace({ cause, text, ts, people }: { cause: Extract<DeliveryCause, { kind: "peer" }>; text: string; ts: number; people: ReadonlyMap<string, Person> | undefined }) {
	return (
		<div className="said-group relative mt-3">
			<p className="said-name flex items-center gap-1.5">
				<Avatar id={cause.personaId} name={cause.name} size={16} hash={people?.get(cause.personaId)?.hash} />
				<span>{cause.name}</span>
				<span className="instrument">· {asideTime(ts)}</span>
			</p>
			<div className="speech said-them">
				<Markdown text={text} />
			</div>
		</div>
	);
}
