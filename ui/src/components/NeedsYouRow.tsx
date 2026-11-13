import type { ReactNode } from "react";
import { askKey, useAnswer } from "../answering";
import type { ThreadId } from "../generated/contract";
import { answerCard, useThread } from "../tape";
import type { ListItem } from "../threadList";
import { wire } from "../wire";
import { type Ask, askLine, openAsks } from "./transcript/AskCard";
import type { ThreadRef } from "./Transcript";

/** One of a card's own answers, as a key on the row. */
type Key = { word: string; send(): Promise<unknown> };

/**
 * The keys a waiting card's row carries: the card's own answers, sent the
 * way the card sends them, or none where answering takes more than a key (a
 * sign-in, a credential, a handoff's question, whose answer is written), and
 * the row has Open instead.
 */
function keysOf(ask: Ask, thread: ThreadId, owner: string, handoff: boolean): Key[] {
	switch (ask.kind) {
		case "permission":
			return ask.options.map((option) => ({
				word: option.name,
				send: () => answerCard(thread, { kind: "permission", requestId: ask.requestId, optionId: option.optionId }),
			}));
		case "human_action": {
			if (handoff) return [];
			const human = (status: "done" | "declined") => () => answerCard(thread, { kind: "human", actionId: ask.actionId, status });
			return ask.question
				? [{ word: "Yes", send: human("done") }, { word: "No", send: human("declined") }]
				: [{ word: "Done", send: human("done") }, { word: "Decline", send: human("declined") }];
		}
		case "passkey_ask": {
			const passkey = (approved: boolean) => () => wire.command("secrets.passkey.answer", { personaId: owner, askId: ask.askId, approved });
			return [{ word: "Approve", send: passkey(true) }, { word: "Deny", send: passkey(false) }];
		}
		case "secret_use_ask": {
			const use = (answer: "approve" | "deny") => () => wire.command("secrets.use.answer", { personaId: ask.teammate, useId: ask.useId, answer });
			return [{ word: "Approve", send: use("approve") }, { word: "Deny", send: use("deny") }];
		}
		case "credential_ask":
		case "cookie_ask":
			return [];
	}
}

/**
 * A row in Needs you: the waiting card's question for its line, and under
 * it the card's own answers as keys, which answer it as the card in the
 * thread would, sharing its answering state, so a press here and one there
 * are one answer. A card that takes more than a key, or one not read yet,
 * has Open, which opens the thread where it is answered.
 */
export function NeedsYouRow({ item, row, onOpen }: { item: ListItem; row(line?: string): ReactNode; onOpen(open: ThreadRef): void }) {
	const { events, loaded } = useThread(item.thread ?? null);
	const ask = openAsks(events)[0];
	return (
		<div className="list-needs">
			{row(ask === undefined ? undefined : askLine(ask))}
			{loaded && (
				<div className="list-keys">
					{ask === undefined || item.thread === undefined ? (
						<OpenKey item={item} onOpen={onOpen} />
					) : (
						<CardKeys ask={ask} item={item} thread={item.thread} onOpen={onOpen} />
					)}
				</div>
			)}
		</div>
	);
}

function CardKeys({ ask, item, thread, onOpen }: { ask: Ask; item: ListItem; thread: ThreadId; onOpen(open: ThreadRef): void }) {
	const { answering, failure, answer } = useAnswer(askKey(ask), false);
	const keys = keysOf(ask, thread, item.owner ?? "", item.kind === "handoff");
	if (keys.length === 0) return <OpenKey item={item} onOpen={onOpen} />;
	return (
		<>
			{keys.map((key) => (
				<button
					key={key.word}
					type="button"
					className="control btn btn-sm"
					aria-label={`${key.word}: ${item.title}`}
					disabled={answering}
					onClick={() => answer(key.send)}
				>
					{key.word}
				</button>
			))}
			{failure !== null && (
				<p role="alert" className="list-refused">
					{failure}
				</p>
			)}
		</>
	);
}

function OpenKey({ item, onOpen }: { item: ListItem; onOpen(open: ThreadRef): void }) {
	const open = item.open;
	if (open === undefined) return null;
	return (
		<button type="button" className="control btn btn-sm" aria-label={`Open ${item.title}`} onClick={() => onOpen(open)}>
			Open
		</button>
	);
}
