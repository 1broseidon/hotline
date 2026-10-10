import { useEffect, useRef, useState } from "react";
import type { LookStyle, Mood, Persona } from "../generated/contract";
import { Chips } from "../ui/Chips";
import { wire } from "../wire";
import { useFacePrefs } from "./prefs";

const STYLES: { id: LookStyle; name: string; title: string }[] = [
	{ id: "initial", name: "Initial", title: "The first letter of the name" },
	{ id: "photo", name: "Photo", title: "A still picture" },
	{ id: "drawn", name: "Drawn", title: "An SVG drawn to the face rig, moved per mood" },
	{ id: "video", name: "Video", title: "A video clip per mood" },
];

/** The moods a video face has a clip for, in the order they are explained. */
const MOODS: { id: Mood; name: string; when: string }[] = [
	{ id: "idle", name: "Idle", when: "at rest" },
	{ id: "sleep", name: "Sleep", when: "away, or long idle" },
	{ id: "attentive", name: "Attentive", when: "just woke, or reading" },
	{ id: "think", name: "Think", when: "thinking" },
	{ id: "work", name: "Work", when: "running a tool, or you typing" },
	{ id: "done", name: "Done", when: "a turn just ended" },
	{ id: "grumpy", name: "Grumpy", when: "an error" },
	{ id: "crossed-arms", name: "Arms crossed", when: "stopped" },
	{ id: "needy", name: "Needy", when: "waiting on you" },
];

/** A picture the desk will square anyway, made small first so it crosses the wire in one message. */
const SEND_EDGE = 1024;
/** What the desk keeps for one clip, and for a drawing. */
const MAX_CLIP_BYTES = 4 * 1024 * 1024;
const MAX_DRAWING_BYTES = 1024 * 1024;

async function pictureData(file: File): Promise<string> {
	const bitmap = await createImageBitmap(file);
	const scale = Math.min(1, SEND_EDGE / Math.max(bitmap.width, bitmap.height));
	const canvas = document.createElement("canvas");
	canvas.width = Math.round(bitmap.width * scale);
	canvas.height = Math.round(bitmap.height * scale);
	canvas.getContext("2d")?.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
	return canvas.toDataURL("image/png").split(",")[1] ?? "";
}

async function fileData(file: File): Promise<string> {
	const bytes = new Uint8Array(await file.arrayBuffer());
	let text = "";
	for (let index = 0; index < bytes.length; index += 0x8000) text += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
	return btoa(text);
}

/**
 * How this teammate's face is drawn: the style, and what each style needs —
 * a picture, a drawing, or a clip per mood. It is the person's choice and
 * lives on the teammate's record; whether this window draws faces at all is
 * Settings → Avatars.
 */
export function FaceSection({ persona, disabled }: { persona: Persona; disabled: boolean }) {
	const prefs = useFacePrefs();
	const look = persona.look;
	const style: LookStyle = look?.style ?? (persona.avatar !== undefined ? "photo" : "initial");
	const [working, setWorking] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [canDraw, setCanDraw] = useState(false);
	const picture = useRef<HTMLInputElement>(null);
	const clipFor = useRef<Mood>("idle");
	const clip = useRef<HTMLInputElement>(null);
	const drawing = useRef<HTMLInputElement>(null);
	const busy = disabled || working;

	useEffect(() => {
		let gone = false;
		void wire.command("images.status", {}).then(
			(status) => {
				if (!gone) setCanDraw(status.available);
			},
			() => {},
		);
		return () => {
			gone = true;
		};
	}, []);

	const run = (work: () => Promise<unknown>) => {
		setWorking(true);
		setRefusal(null);
		work()
			.catch((error: Error) => setRefusal(error.message))
			.finally(() => setWorking(false));
	};

	const setStyle = (next: LookStyle) => run(() => wire.command("look.set", { personaId: persona.id, style: next }));

	const choosePicture = (file: File | undefined) => {
		if (file === undefined) return;
		run(async () => wire.command("avatar.set", { personaId: persona.id, data: await pictureData(file) }));
	};

	const chooseClip = (file: File | undefined) => {
		if (file === undefined) return;
		if (file.size > MAX_CLIP_BYTES) {
			setRefusal("That clip is too large; keep a mood under 4 MB.");
			return;
		}
		const mood = clipFor.current;
		run(async () => wire.command("look.clip_set", { personaId: persona.id, mood, mimeType: file.type || "video/mp4", data: await fileData(file) }));
	};

	const chooseDrawing = (file: File | undefined) => {
		if (file === undefined) return;
		if (file.size > MAX_DRAWING_BYTES) {
			setRefusal("That drawing is too large; keep it under 1 MB.");
			return;
		}
		run(async () => wire.command("look.drawing_set", { personaId: persona.id, svg: await file.text() }));
	};

	return (
		<section>
			<h3 className="label">Face</h3>
			<div className="grouped">
				<div className="group-row">
					<span className="group-row-text">
						<span className="group-row-title">Style</span>
						{!prefs.on && (style === "video" || style === "drawn") && <span className="group-row-detail">Turn on animated avatars in Settings → Avatars to see it</span>}
					</span>
					<Chips value={style} choices={STYLES} label="Face style" disabled={busy} onChange={setStyle} />
				</div>

				{style === "photo" && (
					<div className="group-row">
						<span className="group-row-text">
							<span className="group-row-title">Picture</span>
							<span className="group-row-detail">{persona.avatar === undefined ? "None yet" : persona.avatar.by === "person" ? "Chosen by you" : "Set by the teammate"}</span>
						</span>
						<span className="-my-1 flex items-center gap-1">
							<button type="button" className="control btn-quiet" disabled={busy} onClick={() => picture.current?.click()}>
								Choose a photo…
							</button>
							{canDraw && (
								<button type="button" className="control btn-quiet" disabled={busy} onClick={() => run(() => wire.command("avatar.generate", { personaId: persona.id }))}>
									Draw one
								</button>
							)}
						</span>
						<input
							ref={picture}
							type="file"
							accept="image/*"
							hidden
							onChange={(event) => {
								choosePicture(event.target.files?.[0]);
								event.target.value = "";
							}}
						/>
					</div>
				)}

				{style === "drawn" && (
					<div className="group-row">
						<span className="group-row-text">
							<span className="group-row-title">Drawing</span>
							<span className="group-row-detail">{look?.drawing === undefined ? "None yet" : "Added"}</span>
						</span>
						<span className="-my-1 flex items-center gap-1">
							<button type="button" className="control btn-quiet" disabled={busy} onClick={() => drawing.current?.click()}>
								{look?.drawing === undefined ? "Choose an SVG…" : "Replace"}
							</button>
							{look?.drawing !== undefined && (
								<button type="button" className="control btn-quiet" disabled={busy} onClick={() => run(() => wire.command("look.drawing_clear", { personaId: persona.id }))}>
									Remove
								</button>
							)}
						</span>
						<input
							ref={drawing}
							type="file"
							accept="image/svg+xml,.svg"
							hidden
							onChange={(event) => {
								chooseDrawing(event.target.files?.[0]);
								event.target.value = "";
							}}
						/>
					</div>
				)}

				{style === "video" &&
					MOODS.map((mood) => {
						const kept = look?.clips?.[mood.id];
						return (
							<div className="group-row" key={mood.id}>
								<span className="group-row-text">
									<span className="group-row-title">{mood.name}</span>
									<span className="group-row-detail">
										{mood.when} · {kept !== undefined ? "clip added" : mood.id === "idle" ? "needed for the video face" : "plays the idle clip"}
									</span>
								</span>
								<span className="-my-1 flex items-center gap-1">
									<button
										type="button"
										className="control btn-quiet"
										disabled={busy}
										onClick={() => {
											clipFor.current = mood.id;
											clip.current?.click();
										}}
									>
										{kept !== undefined ? "Replace" : "Add clip…"}
									</button>
									{kept !== undefined && (
										<button
											type="button"
											className="control btn-quiet"
											disabled={busy}
											onClick={() => run(() => wire.command("look.clip_clear", { personaId: persona.id, mood: mood.id }))}
										>
											Remove
										</button>
									)}
								</span>
							</div>
						);
					})}
				<input
					ref={clip}
					type="file"
					accept="video/mp4,video/webm"
					hidden
					onChange={(event) => {
						chooseClip(event.target.files?.[0]);
						event.target.value = "";
					}}
				/>
			</div>
			{style === "drawn" && <p className="hint">A 200 × 200 SVG whose eyes, lids, ears, brows and paws are named the way the face rig expects; Hotline's docs/faces.md has the names and a prompt that draws one.</p>}
			{style === "video" && <p className="hint">A square MP4 or WebM of 2–4 seconds per mood, each starting and ending on the same pose.</p>}
			{refusal !== null && (
				<p role="status" className="selectable text-sm text-danger">
					{refusal}
				</p>
			)}
		</section>
	);
}
