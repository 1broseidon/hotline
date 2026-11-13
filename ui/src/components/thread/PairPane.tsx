import { useCallback, useEffect, useMemo, useState } from "react";
import type { Aside, DeliveryCause, ThreadId, ThreadSummary, TranscriptEvent } from "../../generated/contract";
import type { Person } from "../../avatars";
import type { LinkEvent } from "../../dock";
import { ArrowLeftIcon } from "../../icons";
import { startedWords, threadFamily } from "../../threadKinds";
import { useThread } from "../../tape";
import { toBlocks, withStreaming } from "../../transcriptFolds";
import { wire } from "../../wire";
import { Avatar } from "../../ui/Avatar";
import { Band } from "../../ui/Band";
import { PaneClose } from "../../ui/PaneClose";
import { Markdown } from "../Markdown";
import { ExchangePaused } from "../transcript/Asks";
import { BandKind, LiveStream, PaneHead } from "./Pane";
import { WorkBlocks } from "./WorkStream";

type Teammate = { id: string; name: string; hash?: string | undefined };

type Paused = Extract<TranscriptEvent, { kind: "exchange_paused" }>;

/** A handoff's request line: the work thread it runs in, and what the pane needs to read that work under it. */
type HandedWork = { thread: string; receiver: Teammate; latest: boolean; result: string | undefined };

/** One line of a pair: who said it, on which side, and what; or the card its exchange paused on. */
export type PairLine =
	| { kind: "said"; id: string; who: Teammate; right: boolean; text: string; chip?: "Handoff" | "Aside"; work?: HandedWork }
	| { kind: "paused"; id: string; event: Paused };

/**
 * An aside one of the two left the other, as `aside.list` keeps it. The ones
 * Hotline leaves about a handoff's result or stop are left out: the result is
 * a line of the pair already, and the handoff says it stopped.
 */
function betweenThem(aside: Aside, self: Teammate, other: Teammate): boolean {
	const ids = [self.id, other.id];
	return aside.by !== undefined && aside.by !== aside.to && ids.includes(aside.by) && ids.includes(aside.to) && aside.loopedIn !== true && !aside.id.startsWith("result:") && !aside.id.startsWith("stopped:");
}

/**
 * A pair's lines, oldest first, each with the teammate who said it. A pair's
 * key is its two ids, and its stream writes the first's lines as `user` and
 * the second's as `agent`; a colleague's delivery into it is its sender's.
 * `self` is on the left, the teammate it is with on the right. A line the desk
 * stamped with a work thread is a handoff or its result (`result:<request>`);
 * the last request into a thread is the one still going on in it. The asides
 * between the two stand among the lines at their times.
 */
export function pairLines(key: string, events: readonly TranscriptEvent[], self: Teammate, other: Teammate, asides: readonly Aside[] = []): PairLine[] {
	const first = key.split("~")[0];
	const whose = (id: string | undefined) => (id === self.id ? self : other);
	const saidBy = (who: Teammate) => ({ who, right: who.id !== self.id });
	const left = asides.filter((aside) => betweenThem(aside, self, other)).sort((one, two) => one.ts - two.ts);
	const lines: PairLine[] = [];
	const asidesBefore = (ts: number) => {
		while (left[0] !== undefined && left[0].ts <= ts) {
			const aside = left.shift()!;
			lines.push({ kind: "said", id: `aside:${aside.id}`, ...saidBy(whose(aside.by)), text: aside.text, chip: "Aside" });
		}
	};
	for (const event of events) {
		asidesBefore(event.ts - 1);
		let who: Teammate;
		if (event.kind === "user") who = whose(first);
		else if (event.kind === "agent") who = first === self.id ? other : self;
		else if (event.kind === "delivery" && event.cause.kind !== "answer") who = whose(event.cause.personaId);
		else {
			if (event.kind === "exchange_paused") lines.push({ kind: "paused", id: event.id, event });
			continue;
		}
		const thread = event.kind === "delivery" ? undefined : event.handoffThread;
		const line: PairLine = { kind: "said", id: event.id, ...saidBy(who), text: event.text };
		if (thread !== undefined) {
			line.chip = "Handoff";
			if (!event.id.startsWith("result:")) {
				const result = events.find((one) => one.id === `result:${event.id}`);
				for (const earlier of lines) if (earlier.kind === "said" && earlier.work?.thread === thread) earlier.work.latest = false;
				line.work = { thread, receiver: who.id === self.id ? other : self, latest: true, result: result !== undefined && "text" in result ? result.text : undefined };
			}
		}
		lines.push(line);
	}
	asidesBefore(Number.POSITIVE_INFINITY);
	return lines;
}

/**
 * What one handoff did, from its work thread's stream: what came after its
 * own delivery, up to the next handoff into the same thread. The thread's own
 * link is not work, and once the result is on the pair the words it was made
 * of are that line, so they are not said twice.
 */
export function handoffWork(events: readonly TranscriptEvent[], request: string, result: string | undefined): TranscriptEvent[] {
	const handed = (event: TranscriptEvent) => event.kind === "delivery" && event.cause.kind === "handoff";
	const start = events.findIndex((event) => handed(event) && event.kind === "delivery" && event.cause.kind === "handoff" && event.cause.requestId === request);
	if (start === -1) return [];
	const after = events.slice(start + 1);
	const next = after.findIndex(handed);
	return (next === -1 ? after : after.slice(0, next)).filter((event) => event.kind !== "link" && !(event.kind === "agent" && result?.includes(event.text.trim()) === true));
}

/** The one plain line a handoff that did not finish leaves, as its thread's link records; one that finished needs none. */
export function handoffCut(events: readonly TranscriptEvent[]): string | null {
	const link = events.find((event): event is LinkEvent => event.kind === "link");
	if (link === undefined) return null;
	if (link.state === "closed" && link.end === "failed") return "Handoff failed";
	if (link.handoffEnded === true || (link.state === "closed" && (link.end === "stopped" || link.end === "cancelled"))) return "Handoff stopped";
	return null;
}

/**
 * A handoff's work, read where its request was said: the steps and words of
 * its thread as they happen, whole. Every line is fetched, however long the
 * thread: the pane is opened once and read, with nothing more to press.
 */
function HandoffWork({ request, work, running, onGrew }: { request: string; work: HandedWork; running: boolean; onGrew(): void }) {
	const thread = useMemo<ThreadId>(() => ({ kind: "side", key: work.thread }), [work.thread]);
	const { events, streaming, more, earlier } = useThread(thread);
	// biome-ignore lint/correctness/useExhaustiveDependencies: each page that arrives may say there is another.
	useEffect(() => {
		if (more) earlier().catch(() => {});
	}, [more, earlier, events.length]);
	const blocks = useMemo(
		() => withStreaming(toBlocks(handoffWork(events, request, work.result), true), work.latest ? streaming : []).filter((block) => !(block.kind === "event" && block.event.kind === "turn")),
		[events, request, work.result, work.latest, streaming],
	);
	// biome-ignore lint/correctness/useExhaustiveDependencies: new work is what the pane has to follow.
	useEffect(() => onGrew(), [blocks]);
	const cut = work.latest ? handoffCut(events) : null;
	return (
		<div className="pair-work">
			<WorkBlocks
				thread={thread}
				blocks={blocks}
				live={work.latest && running}
				worker={work.receiver}
				said={(event) =>
					event.kind === "agent"
						? { name: work.receiver.name, mine: false }
						: event.kind === "user"
							? { name: "You", mine: true }
							: event.cause.kind === "answer"
								? null
								: { name: event.cause.name, mine: false }
				}
			/>
			{cut !== null && <p className="pair-work-cut">{cut}</p>}
		</div>
	);
}

/**
 * A pair open in the pane (docs/design/thread-panes.html, Pair): two
 * teammates talking, read along as it happens. Each line on its speaker's
 * side under their face and name; a teammate on a linked desk by the name
 * that says so ("Lou on grizzly"). A handoff between them is its request, the
 * work it became as it happens, and its result, and an aside between them is
 * a line too, each with a small chip by the name: one press opened this, and
 * nothing here needs another to be read. While the exchange stands the band
 * says they are talking and offers Stop exchange, which is the person's to end.
 */
export function PairPane({
	threadKey,
	self,
	other,
	row,
	events,
	asides = [],
	workingSides = [],
	people,
	handoff,
	onBack,
	onClose,
}: {
	threadKey: string;
	/** The teammate whose pane it is open in. */
	self: Teammate;
	/** The teammate it is with. */
	other: Teammate;
	row: ThreadSummary | undefined;
	/** The pair's lines, its link left out. */
	events: TranscriptEvent[];
	/** What each of the two has been left, whoever by. */
	asides?: readonly Aside[];
	/** The work threads with a turn running now, by the roster. */
	workingSides?: readonly string[];
	people: ReadonlyMap<string, Person>;
	/** The handoff it was opened from, whose request it carries. */
	handoff?: Extract<DeliveryCause, { kind: "handoff" }> | undefined;
	onBack(): void;
	onClose(): void;
}) {
	const lines = useMemo(() => pairLines(threadKey, events, self, other, asides), [threadKey, events, self, other, asides]);
	// A handoff's work grows under its line without the pair's own lines moving.
	const [grew, setGrew] = useState(0);
	const onGrew = useCallback(() => setGrew((times) => times + 1), []);
	const shown = useMemo(() => ({ lines, grew }), [lines, grew]);
	const working = lines.some((line) => line.kind === "said" && line.work?.latest === true && workingSides.includes(line.work.thread));
	const said = lines.filter((line) => line.kind === "said").length;
	// A pair is live in the desk's record while a request of it is being answered: the exchange stands.
	const talking = row?.state === "live";
	const state = talking ? "Talking" : row?.state === "closed" ? "Ended" : "Idle";
	const [refused, setRefused] = useState<string | null>(null);
	const stop = () => {
		setRefused(null);
		void wire.command("thread.cancel", { thread: { kind: "pair", key: threadKey } }).catch((error: unknown) => setRefused(error instanceof Error ? error.message : String(error)));
	};
	return (
		<section className="dock-page" aria-label={`Pair: ${self.name} and ${other.name}`}>
			<Band>
				<button type="button" className="control btn-icon -ml-1" title="Back" aria-label="Back" onClick={onBack}>
					<ArrowLeftIcon />
				</button>
				<BandKind kind={threadFamily("pair")} live={talking} state={state.toLowerCase()} tone="quiet" />
				<span className="instrument shrink-0 pr-1">{said === 1 ? "1 line" : `${said} lines`}</span>
				{talking && (
					<button type="button" className="control btn-quiet px-2 text-sm" title="Stop the exchange" onClick={stop}>
						Stop exchange
					</button>
				)}
				<PaneClose onClose={onClose} />
			</Band>
			<PaneHead
				kind={threadFamily("pair")}
				title={`${self.name} ↔ ${other.name}`}
				meta={[handoff !== undefined && `handed off by ${handoff.name}`, row !== undefined && startedWords(row.startedAt)]}
			/>
			<LiveStream following={threadKey} count={lines.length} content={shown} live={talking || working}>
				{lines.map((line) =>
					line.kind === "paused" ? (
						<ExchangePaused key={line.id} personaId={self.id} ownerName={self.name} event={line.event} />
					) : (
						<div key={line.id}>
							<div className="pair-line" data-right={line.right || undefined}>
								<Avatar id={line.who.id} name={line.who.name} size={22} hash={line.who.hash ?? people.get(line.who.id)?.hash} />
								<div className="pair-line-body">
									<p className="said-name pair-line-name">
										{line.who.name}
										{line.chip !== undefined && <span className="pair-chip">{line.chip}</span>}
									</p>
									<div className="pair-line-said selectable">
										<Markdown text={line.text} />
									</div>
								</div>
							</div>
							{line.work !== undefined && <HandoffWork request={line.id} work={line.work} running={workingSides.includes(line.work.thread)} onGrew={onGrew} />}
						</div>
					),
				)}
			</LiveStream>
			{refused !== null && <p className="dock-note selectable" style={{ color: "var(--warn)" }}>{refused}</p>}
		</section>
	);
}
