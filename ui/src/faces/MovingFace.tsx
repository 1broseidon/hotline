import { DrawnFace } from "./DrawnFace";
import type { Moving } from "./face";
import type { Mood } from "./mood";
import { VideoFace } from "./VideoFace";

/** A face that moves, in the mood it is given: its clips, or its drawing. */
export function MovingFace({ personaId, face, mood }: { personaId: string; face: Moving; mood: Mood }) {
	return face.kind === "video" ? <VideoFace personaId={personaId} clips={face.clips} mood={mood} /> : <DrawnFace personaId={personaId} drawing={face.drawing} mood={mood} />;
}
