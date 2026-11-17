import type { Person } from "../../avatars";
import type { ThreadId, TranscriptEvent } from "../../generated/contract";
import { collaborationAsk, ExchangePaused, HumanAction, PasskeyAskCard, Permission, QuietPaused, quietPausedLine } from "./Asks";
import { CookieAskCard } from "./CookieAskCard";
import { CredentialAsk } from "./CredentialAsk";
import { Prompted } from "./InboxLines";
import { SecretUseAsk } from "./SecretUseAsk";

/** A card waiting on the person: the kinds that hold a turn, a teammate's computer, two teammates paused together, or one paused at its hourly limit, until answered. */
export type Ask = Extract<
	TranscriptEvent,
	{ kind: "permission" | "human_action" | "credential_ask" | "passkey_ask" | "secret_use_ask" | "cookie_ask" | "exchange_paused" | "quiet_paused" }
>;

/** A card with a time limit that has run out on this clock: it can no longer be answered. */
function lapsed(event: TranscriptEvent, now: number): boolean {
	return (event.kind === "credential_ask" || event.kind === "secret_use_ask" || event.kind === "cookie_ask") && now >= event.expiresAt;
}

/** Whether a card still waits on the person. */
function waits(event: TranscriptEvent, now: number): event is Ask {
	if (lapsed(event, now)) return false;
	switch (event.kind) {
		case "permission":
			return event.decision === undefined;
		case "human_action":
		case "passkey_ask":
		case "secret_use_ask":
		case "cookie_ask":
		case "exchange_paused":
		case "quiet_paused":
			return event.status === "pending";
		case "credential_ask":
			return event.status === "pending" && !event.armed;
		default:
			return false;
	}
}

/**
 * The cards waiting on the person, oldest first: what the composer's chip
 * opens to answer. A card whose time has run out on `now` no longer waits,
 * whether or not the tape has said so.
 */
export function openAsks(events: readonly TranscriptEvent[], now: number = Date.now()): Ask[] {
	return events.filter((event): event is Ask => waits(event, now));
}

/** When the next pending card with a time limit runs out after `now`, if any does: when the composer must count again. */
export function nextLapse(events: readonly TranscriptEvent[], now: number): number | undefined {
	let next: number | undefined;
	for (const event of events) {
		if ((event.kind !== "credential_ask" && event.kind !== "secret_use_ask" && event.kind !== "cookie_ask") || event.status !== "pending") continue;
		if (event.expiresAt > now && (next === undefined || event.expiresAt < next)) next = event.expiresAt;
	}
	return next;
}

/** A waiting card in a few words, for the chip. `ownerName` is the teammate whose card it is. */
export function askLine(ask: Ask, ownerName: string): string {
	switch (ask.kind) {
		case "permission": {
			// A request up in access is one short question.
			const asking = collaborationAsk(ask);
			if (asking === undefined) return ask.title;
			return asking.from !== undefined && asking.to !== undefined ? `Allow ${asking.to} to do work for ${asking.from}?` : asking.heading;
		}
		case "human_action":
			return ask.reason;
		case "credential_ask":
			return `Sign in to ${ask.site}`;
		case "passkey_ask":
			return `Passkey for ${ask.rpName ?? ask.rpId}`;
		case "secret_use_ask":
			return `Use ${ask.secret} on ${ask.site}`;
		case "cookie_ask":
			return `Sign in to ${ask.site}`;
		case "exchange_paused":
			return `Paused with ${ask.withName}`;
		case "quiet_paused":
			return quietPausedLine(ownerName, ask.turns);
	}
}

/**
 * The card itself, as the conversation draws it, to be answered where the
 * person is: under the line saying whose message prompted it, when a
 * colleague's did (`fromItem`), as the conversation draws it there.
 */
export function AskCard(props: {
	ask: Ask;
	personaId: string;
	thread: ThreadId;
	ownerName: string;
	/** The teammate's picture, for the face on the card. */
	ownerHash?: string | undefined;
	onOpenScreen?: (() => void) | undefined;
	/** The team by id, to name the colleague whose message prompted the card. */
	people?: ReadonlyMap<string, Person> | undefined;
}) {
	const { ask, people } = props;
	return (
		<Prompted itemId={"fromItem" in ask ? ask.fromItem : undefined} people={people}>
			<AskBody {...props} />
		</Prompted>
	);
}

function AskBody({
	ask,
	personaId,
	thread,
	ownerName,
	ownerHash,
	onOpenScreen,
}: {
	ask: Ask;
	personaId: string;
	thread: ThreadId;
	ownerName: string;
	ownerHash?: string | undefined;
	onOpenScreen?: (() => void) | undefined;
}) {
	const who = { id: personaId, name: ownerName, hash: ownerHash };
	switch (ask.kind) {
		case "permission":
			return <Permission thread={thread} event={ask} who={who} />;
		case "human_action":
			return <HumanAction thread={thread} event={ask} who={who} {...(onOpenScreen !== undefined ? { onOpenScreen } : {})} />;
		case "credential_ask":
			return <CredentialAsk event={ask} teammate={ownerName} />;
		case "passkey_ask":
			return <PasskeyAskCard personaId={personaId} event={ask} teammate={ownerName} />;
		case "secret_use_ask":
			return <SecretUseAsk event={ask} teammate={ownerName} />;
		case "cookie_ask":
			return <CookieAskCard event={ask} personaId={personaId} teammate={ownerName} onOpenScreen={onOpenScreen} />;
		case "exchange_paused":
			return <ExchangePaused personaId={personaId} ownerName={ownerName} event={ask} />;
		case "quiet_paused":
			return <QuietPaused personaId={personaId} ownerName={ownerName} event={ask} />;
	}
}
