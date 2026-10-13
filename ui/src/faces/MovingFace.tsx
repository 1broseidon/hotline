import { DrawnFace } from "./DrawnFace";
import type { Moving } from "./face";
import { type Mood, useWindowFocused } from "./mood";
import { VideoFace } from "./VideoFace";

/**
 * A face that moves, in the mood it is given: its clips, or its drawing. It
 * stands still when asked, as small copies of a face are, and whenever the
 * window is in the background.
 */
export function MovingFace({ personaId, face, mood, still = false }: { personaId: string; face: Moving; mood: Mood; still?: boolean }) {
	const resting = still || !useWindowFocused();
	return face.kind === "video" ? (
		<VideoFace personaId={personaId} clips={face.clips} mood={mood} still={resting} />
	) : (
		<DrawnFace personaId={personaId} drawing={face.drawing} mood={mood} still={resting} />
	);
}
