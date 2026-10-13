import type { ActivityPhase } from "../activity";
import { moves, useFace } from "./face";
import { type Mood, useHeldMood } from "./mood";
import { MovingFace } from "./MovingFace";

/** What each kind of work looks like on the face, in place of the Glyph's poses. */
const MOOD_OF: Record<ActivityPhase, Mood> = {
	thinking: "think",
	read: "attentive",
	search: "attentive",
	edit: "work",
	execute: "work",
	doing: "work",
	writing: "work",
	blocked: "needy",
	landed: "done",
	rest: "sleep",
};

/** The ambient mark under the transcript, as the teammate's own face. */
export function FaceMark({ personaId, phase }: { personaId: string; phase: ActivityPhase }) {
	const face = useFace(personaId);
	const mood = useHeldMood(MOOD_OF[phase]);
	if (!moves(face)) return null;
	return (
		<span className="avatar face-mark" aria-hidden="true">
			<MovingFace personaId={personaId} face={face} mood={mood} />
		</span>
	);
}
