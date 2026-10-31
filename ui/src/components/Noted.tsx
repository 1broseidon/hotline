import { useEffect, useRef } from "react";
import type { Aside } from "../generated/contract";
import { asideTime, groupNoted, levelWord, sourceName, sourceThread, sourceWords, useAsides } from "../asides";
import type { Person } from "../avatars";
import { ChevronRightIcon } from "../icons";
import { Avatar } from "../ui/Avatar";
import { Band } from "../ui/Band";
import { PaneClose } from "../ui/PaneClose";
import { Scroll } from "../ui/Scroll";
import { wire, type RosterEntry } from "../wire";
import type { ThreadRef } from "./Transcript";

/**
 * A teammate's Noted page, in the right-hand pane: what its threads and
 * colleagues told it without telling the person, newest first, the ones the
 * person had not seen under New and the rest under Earlier. Each says who and
 * from what kind of thread, its level ("to you" in amber), its time and its
 * words, and opens the thread it came from. Opening the page marks them seen
 * for the person (`aside.seen`), which is not the teammate hearing them; the
 * groups stay as they opened until the page closes.
 */
export function NotedPage({
	entry,
	people,
	onOpen,
	onClose,
}: {
	entry: RosterEntry;
	people: ReadonlyMap<string, Person>;
	onOpen(open: ThreadRef): void;
	onClose(): void;
}) {
	const personaId = entry.persona.id;
	const asides = useAsides(personaId, entry);
	/* What was new when the page opened, so marking it seen does not move it. */
	const fresh = useRef(new Set<string>());
	const unseen = (asides ?? []).filter((aside) => aside.seen === undefined);
	for (const aside of unseen) fresh.current.add(aside.id);
	const anyUnseen = unseen.length > 0;
	useEffect(() => {
		if (anyUnseen) void wire.command("aside.seen", { personaId }).catch(() => undefined);
	}, [personaId, anyUnseen]);
	const { fresh: shownNew, earlier } = groupNoted(asides ?? [], fresh.current);
	const now = Date.now();

	return (
		<section className="dock-page" aria-label={`${entry.persona.name}'s Noted`}>
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Noted</h2>
				<PaneClose onClose={onClose} />
			</Band>
			<Scroll>
				<div className="px-2 pb-3">
					{asides === undefined ? null : asides.length === 0 ? (
						<p className="px-2 py-3 text-sm text-ink-3">Nothing noted for {entry.persona.name} yet.</p>
					) : (
						<>
							{shownNew.length > 0 && (
								<NotedGroup title={`New · ${shownNew.length}`} asides={shownNew} people={people} now={now} onOpen={onOpen} />
							)}
							{earlier.length > 0 && <NotedGroup title="Earlier" asides={earlier} people={people} now={now} onOpen={onOpen} />}
							<p className="px-2 pt-3 text-sm text-ink-3">
								What {entry.persona.name}'s threads told it, never said to you. Open one to see its thread.
							</p>
						</>
					)}
				</div>
			</Scroll>
		</section>
	);
}

function NotedGroup({
	title,
	asides,
	people,
	now,
	onOpen,
}: {
	title: string;
	asides: Aside[];
	people: ReadonlyMap<string, Person>;
	now: number;
	onOpen(open: ThreadRef): void;
}) {
	return (
		<div className="mt-2">
			<h3 className="instrument px-2 py-1">{title}</h3>
			<div className="noted-group">
				{asides.map((aside) => (
					<NotedRow key={aside.id} aside={aside} people={people} now={now} onOpen={onOpen} />
				))}
			</div>
		</div>
	);
}

function NotedRow({ aside, people, now, onOpen }: { aside: Aside; people: ReadonlyMap<string, Person>; now: number; onOpen(open: ThreadRef): void }) {
	const who = aside.by !== undefined ? (people.get(aside.by)?.name ?? "A teammate") : sourceName(aside);
	const from = sourceThread(aside);
	const body = (
		<>
			{aside.by !== undefined ? (
				<Avatar id={aside.by} name={who} size={24} hash={people.get(aside.by)?.hash} />
			) : (
				<span className="aside-mark instrument" aria-hidden="true">
					(·)
				</span>
			)}
			<span className="min-w-0 flex-1">
				<span className="flex items-center gap-2">
					<span className="instrument min-w-0 flex-1 truncate">
						{who} · {sourceWords(aside)}
					</span>
					<span className="noted-level instrument" data-level={aside.level}>
						{levelWord(aside.level)}
					</span>
					<span className="instrument shrink-0">{asideTime(aside.ts, now)}</span>
				</span>
				<span className="noted-text">{aside.text}</span>
			</span>
		</>
	);
	if (from === undefined) return <div className="noted-row">{body}</div>;
	return (
		<button type="button" className="noted-row" title="Open its thread" onClick={() => onOpen({ thread: from })}>
			{body}
			<ChevronRightIcon className="shrink-0 text-ink-3" />
		</button>
	);
}
