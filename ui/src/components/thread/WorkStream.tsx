import type { ReactNode } from "react";
import type { ThreadId, TranscriptEvent } from "../../generated/contract";
import type { Block } from "../../scheduledRuns";
import { CookieAskCard } from "../transcript/CookieAskCard";
import { CredentialAsk } from "../transcript/CredentialAsk";
import { HumanAction, PasskeyAskCard, Permission } from "../transcript/Asks";
import { NamedSay } from "../transcript/Bubbles";
import { Markdown } from "../Markdown";
import { Plan } from "../transcript/Plan";
import { SecretUseAsk } from "../transcript/SecretUseAsk";
import { TrailSteps, useHot } from "../transcript/Trail";
import { LiveStream } from "./Pane";

/** Whose work it is: the teammate a card it raises speaks of. */
export type Worker = { id: string; name: string; hash?: string | undefined };

/** A line said in the thread: the person's, the worker's, or a colleague's delivered into it. */
type Said = Extract<TranscriptEvent, { kind: "user" | "agent" | "delivery" }>;

type WorkProps = {
	thread: ThreadId;
	/** The thread's lines as blocks, steps where they fell (`toBlocks(…, true)`), with what streams after them. */
	blocks: Block[];
	/** The worker's turn is running. */
	live: boolean;
	worker: Worker;
	/**
	 * Who said a line, and whether it is the person's own: null leaves it out
	 * of the stream. `plain`: a line of the work rather than of a
	 * conversation, under its speaker's name without a bubble, as a
	 * subagent's words between its steps are (a run is activity, not a chat).
	 */
	said(event: Said): Speaker | null;
};

type Speaker = { name: string; mine: boolean; plain?: boolean };

/**
 * A thread's work where it is drawn: each run of steps where it fell between
 * the words, a step opening to its output, the words under whose they are, and
 * a card the worker raises answered here as anywhere. It says the worker is
 * thinking between its steps and its words while it runs. A pair's pane reads
 * a handoff's work through this, in its own scrolling window.
 */
export function WorkBlocks({ thread, blocks, live, worker, said }: WorkProps) {
	const hot = useHot();
	const last = blocks.at(-1);
	// Between its steps and its words a running turn is still the worker's work.
	const thinking = live && !(last?.kind === "steps" && last.open === true) && !(last?.kind === "event" && last.live === true);
	return (
		<>
			{blocks.map((block) => (
				<StreamBlock key={block.kind === "event" ? block.event.id : block.id} block={block} thread={thread} worker={worker} said={said} hot={hot} />
			))}
			{thinking && (
				<p className="pane-thinking">
					<span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
					Thinking
				</p>
			)}
		</>
	);
}

/**
 * A watched thread's work as it happens (docs/design/thread-panes.html), the
 * one stream a subagent's pane and a handoff's share: [`WorkBlocks`] in a
 * window that follows the newest until the person scrolls up. What closes it,
 * a subagent's result, comes after (`children`).
 */
export function WorkStream({ children, ...work }: WorkProps & { children?: ReactNode }) {
	const { thread, blocks, live } = work;
	return (
		<LiveStream following={`${thread.kind}:${thread.key}`} count={blocks.length} content={blocks} live={live}>
			<WorkBlocks {...work} />
			{children}
		</LiveStream>
	);
}

function StreamBlock({
	block,
	thread,
	worker,
	said,
	hot,
}: {
	block: Block;
	thread: ThreadId;
	worker: Worker;
	said(event: Said): Speaker | null;
	hot: ReturnType<typeof useHot>;
}) {
	if (block.kind === "steps") return <TrailSteps items={block.items} settled={block.open !== true} hot={hot} />;
	if (block.kind !== "event") return null;
	const event = block.event;
	switch (event.kind) {
		case "agent":
		case "user":
		case "delivery": {
			const who = said(event);
			if (who === null) return null;
			if (who.plain === true) {
				return (
					<div className="pane-said">
						<p className="pane-said-who">{who.name}</p>
						<div className="pane-said-text selectable">
							<Markdown text={event.text} />
						</div>
					</div>
				);
			}
			return <NamedSay name={who.name} mine={who.mine} text={event.text} />;
		}
		case "turn":
			return event.stopReason === "end_turn" ? null : <p className="instrument mt-1 text-right text-ink-4">{event.stopReason.replace(/_/g, " ")}</p>;
		case "notice":
			return (
				<p className="rule-line rule-line-plain selectable" style={{ color: event.level === "info" ? "var(--ink-3)" : event.level === "warn" ? "var(--warn)" : "var(--danger)" }}>
					{event.text}
				</p>
			);
		case "plan":
			return <Plan entries={event.entries} />;
		case "permission":
			return <Permission thread={thread} event={event} who={worker} />;
		case "human_action":
			return <HumanAction thread={thread} event={event} who={worker} />;
		case "credential_ask":
			return <CredentialAsk event={event} teammate={worker.name} />;
		case "cookie_ask":
			return <CookieAskCard event={event} personaId={worker.id} teammate={worker.name} />;
		case "secret_use_ask":
			return <SecretUseAsk event={event} teammate={worker.name} />;
		case "passkey_ask":
			return <PasskeyAskCard personaId={worker.id} event={event} teammate={worker.name} />;
		default:
			return null;
	}
}
