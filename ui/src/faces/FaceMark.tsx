import type { ActivityPhase } from "../activity";
import { useFace } from "./face";
import { type Mood, useHeldMood } from "./mood";
import { VideoFace } from "./VideoFace";

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
	if (face?.kind !== "video") return null;
	return (
		<span className="avatar face-mark" aria-hidden="true">
			<VideoFace personaId={personaId} clips={face.clips} mood={mood} />
		</span>
	);
}
