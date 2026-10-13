import { useEffect, useRef, useState } from "react";
import { avatarUrl } from "../avatars";
import type { Clip, Look } from "../generated/contract";
import { wire } from "../wire";
import type { Mood } from "./mood";
import "./faces.css";

const readClip = (personaId: string, hash: string, offset: number) => wire.command("look.clip_read", { personaId, hash, offset });

/**
 * A face made of a clip per mood. A clip plays to its end before the next
 * mood's starts, so a mood always finishes; a mood with no clip of its own
 * plays the idle clip. A still face holds the frame it is on.
 */
export function VideoFace({ personaId, clips, mood, still }: { personaId: string; clips: Look["clips"]; mood: Mood; still: boolean }) {
	const wanted: Clip | undefined = clips[mood] ?? clips.idle;
	const [playing, setPlaying] = useState<Clip | undefined>(wanted);
	const [url, setUrl] = useState<string | undefined>(undefined);
	const video = useRef<HTMLVideoElement>(null);

	if (playing === undefined && wanted !== undefined) setPlaying(wanted);

	useEffect(() => {
		if (playing === undefined) return;
		let gone = false;
		avatarUrl(personaId, playing.hash, readClip).then(
			(next) => {
				if (!gone) setUrl(next);
			},
			() => {},
		);
		return () => {
			gone = true;
		};
	}, [personaId, playing]);

	useEffect(() => {
		const element = video.current;
		if (element === null) return;
		if (still) element.pause();
		else void element.play().catch(() => {});
	}, [still, url]);

	if (url === undefined) return null;
	return (
		<video
			ref={video}
			className="face-video"
			src={url}
			autoPlay={!still}
			muted
			playsInline
			onEnded={() => {
				if (still) return;
				if (wanted !== undefined && wanted.hash !== playing?.hash) {
					setPlaying(wanted);
					return;
				}
				void video.current?.play();
			}}
		/>
	);
}
