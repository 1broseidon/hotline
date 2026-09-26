import { useEffect } from "react";
import type { TranscriptEvent } from "../generated/contract";
import { chordKeys } from "../chords";
import { CloseIcon } from "../icons";
import { useThread } from "../tape";
import { Band } from "../ui/Band";
import { wire } from "../wire";
import { Transcript, type ThreadRef } from "./Transcript";

/**
 * A conversation between two teammates, in the inspector's place.
 *
 * The thread's `user` side is the first id in the key (UTF-16 order) and
 * the `agent` side is the second. The marker's role says caller or
 * target — who started this run — but the file is stored by the key, so
 * this teammate's chair is whichever of those two ids they are, and
 * their words sit on the right.
 */
export type OpenThread = {
	key: string;
	withName: string;
	handoff?: ThreadRef["handoff"];
};

export function Thread({
	open,
	selfId,
	selfName,
	onClose,
}: {
	open: OpenThread;
	selfId: string;
	selfName: string;
	onClose(): void;
}) {
	const { events } = useThread(open.key);
	const mine = selfIsUser(open.key, selfId) ? "user" : "agent";

	useEffect(() => {
		const eventIds = visibleIds(events);
		if (eventIds.length === 0) return;
		void wire.command("peers.mark_read", { key: open.key, eventIds });
	}, [open.key, events]);

	return (
		<aside className="inspector" aria-label={`Thread with ${open.withName}`}>
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">
					{selfName} & {open.withName}
				</h2>
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
					<CloseIcon />
				</button>
			</Band>
			{open.handoff !== undefined && (
				<details open className="mx-4 mb-3 text-sm text-ink-3">
					<summary className="cursor-pointer">Handed off from {open.handoff.name}</summary>
					<dl className="selectable mt-2 space-y-2 break-words">
						<div><dt className="eyebrow">Sender</dt><dd>{open.handoff.name} · {open.handoff.personaId}</dd></div>
						<div><dt className="eyebrow">Request</dt><dd>{open.handoff.requestId}</dd></div>
						<div>
							<dt className="eyebrow">Reply goes to</dt>
							<dd>{open.handoff.name} in this originating exchange, even if they have moved on.</dd>
							<dd className="mt-1 font-mono text-xs">{open.handoff.threadKey}</dd>
						</div>
					</dl>
				</details>
			)}
			<Transcript
				personaId={selfId}
				name={selfName}
				events={events}
				streaming={[]}
				live={false}
				focus={null}
				speakers={{ me: selfName, them: open.withName, mine }}
			/>
		</aside>
	);
}

/** The key's first participant is the stored `user` side. */
function selfIsUser(key: string, selfId: string): boolean {
	const first = key.split("~")[0];
	return first === selfId;
}

function visibleIds(events: TranscriptEvent[]): string[] {
	return events.filter((event) => event.kind === "user" || event.kind === "agent").map((event) => event.id);
}
