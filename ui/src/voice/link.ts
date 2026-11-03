import type { VoiceEndReason } from "../generated/contract";

/**
 * Where a call stands with the desk, apart from what is being said: one
 * value, moved only by `move`. Each variant carries what is true only in it,
 * so a mute cannot outlive the link it belonged to, and a hold taken while
 * the line is down is still there when it comes back.
 */
export type Link =
	| { kind: "live"; /** A front the person talked over: its late audio is dropped until they are heard. */ muted: boolean }
	| { kind: "held" }
	| {
			kind: "reconnecting";
			/** The socket to the desk is down. Any fresh snapshot ends this. */
			socket: boolean;
			/** The desk's own live provider is reconnecting. Its state event ends this. */
			provider: boolean;
			/** The person's hold, which stands for after. */
			held: boolean;
	  }
	/** The desk ended the call; the last clip plays first. */
	| { kind: "closing"; reason: VoiceEndReason };

export type LinkEvent =
	| { type: "hold" }
	| { type: "release" }
	| { type: "barge" }
	| { type: "heard" }
	| { type: "socket"; open: boolean }
	| { type: "attached" }
	| { type: "provider"; reconnecting: boolean }
	| { type: "closing"; reason: VoiceEndReason };

export const LIVE: Link = { kind: "live", muted: false };

const away = (held: boolean, socket: boolean, provider: boolean): Link =>
	socket || provider ? { kind: "reconnecting", socket, provider, held } : held ? { kind: "held" } : LIVE;

/** The one function that moves a call's link. */
export function move(link: Link, event: LinkEvent): Link {
	if (link.kind === "closing") return link;
	if (event.type === "closing") return { kind: "closing", reason: event.reason };
	const held = link.kind === "held" || (link.kind === "reconnecting" && link.held);
	const socket = link.kind === "reconnecting" && link.socket;
	const provider = link.kind === "reconnecting" && link.provider;
	// A live link that stays live keeps its mute: only leaving it clears the mute.
	const to = (held: boolean, socket: boolean, provider: boolean): Link => {
		const next = away(held, socket, provider);
		return link.kind === "live" && next.kind === "live" ? link : next;
	};
	switch (event.type) {
		case "hold": return to(true, socket, provider);
		case "release": return to(false, socket, provider);
		case "socket": return event.open ? link : to(held, true, provider);
		case "attached": return to(held, false, provider);
		case "provider": return to(held, socket, event.reconnecting);
		case "barge": return link.kind === "live" ? { kind: "live", muted: true } : link;
		case "heard": return link.kind === "live" ? LIVE : link;
	}
}
