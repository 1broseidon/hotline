import { useMemo, useState } from "react";
import type { DeliveryCause, ThreadSummary, TranscriptEvent } from "../../generated/contract";
import type { Person } from "../../avatars";
import { ArrowLeftIcon } from "../../icons";
import { startedWords, threadFamily } from "../../threadKinds";
import { wire } from "../../wire";
import { Avatar } from "../../ui/Avatar";
import { Band } from "../../ui/Band";
import { PaneClose } from "../../ui/PaneClose";
import { Markdown } from "../Markdown";
import { ExchangePaused } from "../transcript/Asks";
import { BandKind, LiveStream, PaneHead } from "./Pane";

type Teammate = { id: string; name: string; hash?: string | undefined };

type Paused = Extract<TranscriptEvent, { kind: "exchange_paused" }>;

/** One line of a pair: who said it, on which side, and what; or the card its exchange paused on. */
export type PairLine = { kind: "said"; id: string; who: Teammate; right: boolean; text: string } | { kind: "paused"; id: string; event: Paused };

/**
 * A pair's lines, oldest first, each with the teammate who said it. A pair's
 * key is its two ids, and its stream writes the first's lines as `user` and
 * the second's as `agent`; a colleague's delivery into it is its sender's.
 * `self` is on the left, the teammate it is with on the right.
 */
export function pairLines(key: string, events: readonly TranscriptEvent[], self: Teammate, other: Teammate): PairLine[] {
	const first = key.split("~")[0];
	const whose = (id: string | undefined) => (id === self.id ? self : other);
	const lines: PairLine[] = [];
	for (const event of events) {
		let who: Teammate;
		if (event.kind === "user") who = whose(first);
		else if (event.kind === "agent") who = first === self.id ? other : self;
		else if (event.kind === "delivery" && event.cause.kind !== "answer") who = whose(event.cause.personaId);
		else {
			if (event.kind === "exchange_paused") lines.push({ kind: "paused", id: event.id, event });
			continue;
		}
		lines.push({ kind: "said", id: event.id, who, right: who.id !== self.id, text: event.text });
	}
	return lines;
}

/**
 * A pair open in the pane (docs/design/thread-panes.html, Pair): two
 * teammates talking, read along as it happens. Each line on its speaker's
 * side under their face and name; a teammate on a linked desk by the name
 * that says so ("Lou on grizzly"). While the exchange stands the band says
 * they are talking and offers Stop exchange, which is the person's to end.
 */
export function PairPane({
	threadKey,
	self,
	other,
	row,
	events,
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
	people: ReadonlyMap<string, Person>;
	/** The handoff it was opened from, whose request it carries. */
	handoff?: Extract<DeliveryCause, { kind: "handoff" }> | undefined;
	onBack(): void;
	onClose(): void;
}) {
	const lines = useMemo(() => pairLines(threadKey, events, self, other), [threadKey, events, self, other]);
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
				kindInBand
				title={`${self.name} ↔ ${other.name}`}
				meta={[handoff !== undefined && `handed off by ${handoff.name}`, row !== undefined && startedWords(row.startedAt)]}
			/>
			<LiveStream following={threadKey} count={lines.length} content={lines} live={talking}>
				{lines.map((line) =>
					line.kind === "paused" ? (
						<ExchangePaused key={line.id} personaId={self.id} ownerName={self.name} event={line.event} />
					) : (
						<div key={line.id} className="pair-line" data-right={line.right || undefined}>
							<Avatar id={line.who.id} name={line.who.name} size={22} hash={line.who.hash ?? people.get(line.who.id)?.hash} />
							<div className="pair-line-said selectable">
								<p className="said-name">{line.who.name}</p>
								<Markdown text={line.text} />
							</div>
						</div>
					),
				)}
			</LiveStream>
			{refused !== null && <p className="dock-note selectable" style={{ color: "var(--warn)" }}>{refused}</p>}
		</section>
	);
}
