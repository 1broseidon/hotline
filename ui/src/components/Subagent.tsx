import type { TranscriptEvent } from "../generated/contract";
import { chordKeys } from "../chords";
import { CloseIcon } from "../icons";
import { useRun } from "../tape";
import { Band } from "../ui/Band";
import { Transcript, subagentState, type SubagentEvent } from "./Transcript";

/**
 * A subagent's run, in the inspector's place.
 *
 * The run is the teammate's work, not its conversation: the task it handed
 * over sits on the teammate's side, and what the worker said and did sits
 * across from it. Nothing here is answerable — a run is told its task once
 * and reports once — so there is no composer and no reply.
 */
export type OpenSubagent = {
	runId: string;
	title: string;
};

export function Subagent({
	open,
	selfId,
	selfName,
	onClose,
}: {
	open: OpenSubagent;
	selfId: string;
	selfName: string;
	onClose(): void;
}) {
	const { events } = useRun(open.runId);
	const marker = events.find((event): event is SubagentEvent => event.kind === "subagent");
	const said = events.filter((event): event is Exclude<TranscriptEvent, SubagentEvent> => event.kind !== "subagent");
	const title = marker?.title ?? open.title;

	return (
		<aside className="inspector" aria-label={`Subagent: ${title}`}>
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">{title}</h2>
				{marker !== undefined && <span className="instrument shrink-0 pr-1">{subagentState(marker)}</span>}
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
					<CloseIcon />
				</button>
			</Band>
			<Transcript
				personaId={selfId}
				name="Subagent"
				events={said}
				streaming={[]}
				live={marker?.status === "running"}
				focus={null}
				speakers={{ me: selfName, them: "Subagent", mine: "user" }}
			/>
		</aside>
	);
}
