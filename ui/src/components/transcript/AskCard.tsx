import type { ThreadId, TranscriptEvent } from "../../generated/contract";
import { HumanAction, PasskeyAskCard, Permission } from "./Asks";
import { CookieOfferCard } from "./CookieOfferCard";
import { CredentialAsk } from "./CredentialAsk";
import { SecretUseAsk } from "./SecretUseAsk";

/** A card waiting on the person: the kinds that hold a turn, or a teammate's computer, until answered. */
export type Ask = Extract<
	TranscriptEvent,
	{ kind: "permission" | "human_action" | "credential_ask" | "passkey_ask" | "secret_use_ask" | "cookie_offer" }
>;

/** Whether a card still waits on the person. */
function waits(event: TranscriptEvent): event is Ask {
	switch (event.kind) {
		case "permission":
			return event.decision === undefined;
		case "human_action":
		case "passkey_ask":
		case "secret_use_ask":
		case "cookie_offer":
			return event.status === "pending";
		case "credential_ask":
			return event.status === "pending" && !event.armed;
		default:
			return false;
	}
}

/** The cards waiting on the person, oldest first: what the composer's chip opens to answer. */
export function openAsks(events: readonly TranscriptEvent[]): Ask[] {
	return events.filter(waits);
}

/** A waiting card in a few words, for the chip. */
export function askLine(ask: Ask): string {
	switch (ask.kind) {
		case "permission":
			return ask.title;
		case "human_action":
			return ask.reason;
		case "credential_ask":
			return `Sign in to ${ask.site}`;
		case "passkey_ask":
			return `Passkey for ${ask.rpName ?? ask.rpId}`;
		case "secret_use_ask":
			return `Use ${ask.secret} on ${ask.site}`;
		case "cookie_offer":
			return `Your session on ${ask.site}`;
	}
}

/** The card itself, as the conversation draws it, to be answered where the person is. */
export function AskCard({
	ask,
	personaId,
	thread,
	ownerName,
	onOpenScreen,
}: {
	ask: Ask;
	personaId: string;
	thread: ThreadId;
	ownerName: string;
	onOpenScreen?: (() => void) | undefined;
}) {
	switch (ask.kind) {
		case "permission":
			return <Permission thread={thread} event={ask} />;
		case "human_action":
			return <HumanAction thread={thread} event={ask} {...(onOpenScreen !== undefined ? { onOpenScreen } : {})} />;
		case "credential_ask":
			return <CredentialAsk event={ask} teammate={ownerName} />;
		case "passkey_ask":
			return <PasskeyAskCard personaId={personaId} event={ask} />;
		case "secret_use_ask":
			return <SecretUseAsk event={ask} teammate={ownerName} />;
		case "cookie_offer":
			return <CookieOfferCard offer={ask} teammate={ownerName} />;
	}
}
