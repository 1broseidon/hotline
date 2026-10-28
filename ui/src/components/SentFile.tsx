import { type ReactNode, useEffect, useState } from "react";
import type { Attachment, FileChunk } from "../generated/contract";
import { FileIcon, FolderIcon } from "../icons";
import { openSentFile } from "../native";
import { onServer, openSent, saveSent, showPath } from "../serverFiles";
import { sizeText } from "../sizes";
import { Viewer } from "../ui/Viewer";
import { wire } from "../wire";
import { SentPreview, isMarkdown } from "./SentPreview";

/**
 * A file a teammate sent with `send_file`, under the words it came with.
 *
 * A picture is drawn in the bubble, read over the wire by its message the
 * way a phone reads it, with its place held at its own shape until it
 * arrives. Anything else is a card that names it. Nothing opens on its own:
 * a picture opens in the window's own viewer when pressed, a PDF in the
 * system's, and any file can be saved where the person chooses or shown in
 * its folder. Where
 * the file came from is on hover. On a desk on a server the file is brought
 * down to open or save it, and its folder is the server's (serverFiles.ts).
 */
export function SentFile({
	personaId,
	eventId,
	index,
	file,
	compact = false,
}: {
	personaId: string;
	eventId: string;
	index: number;
	file: Attachment;
	compact?: boolean;
}) {
	const sent = { personaId, eventId, index, file };
	const [error, setError] = useState<string | null>(null);
	const act = (work: () => Promise<unknown>) => {
		setError(null);
		work().catch((failed: unknown) => setError(failed instanceof Error ? failed.message : String(failed)));
	};
	const pdf = file.mimeType === "application/pdf";
	const [viewing, setViewing] = useState(false);
	const actions = (
		<span className="sent-actions">
			{!viewing && (compact || file.kind !== "image") && (
				<button type="button" className="control btn btn-sm" onClick={() => pdf || file.kind === "image" ? act(() => openSent(sent)) : setViewing(true)}>
					Open
				</button>
			)}
			<button type="button" className="control btn btn-sm" onClick={() => act(() => saveSent(sent))}>
				Save…
			</button>
			<button
				type="button"
				className="control btn btn-sm"
				title={onServer() ? "Show on the server" : "Show in folder"}
				aria-label={onServer() ? "Show in its folder on the server" : "Show in folder"}
				onClick={() => showPath(file.path)}
			>
				<FolderIcon />
			</button>
		</span>
	);
	const refused = error !== null && <p className="sent-error">{error}</p>;

	if (file.kind === "image" && !compact) {
		return (
			<figure className="sent-file" title={file.origin}>
				<SentPicture
					personaId={personaId}
					eventId={eventId}
					index={index}
					file={file}
					viewing={viewing}
					onOpen={() => setViewing(true)}
					onClose={() => setViewing(false)}
					actions={actions}
				/>
				<figcaption className="sent-caption">
					<span className="chip-name">{file.name}</span>
					{actions}
				</figcaption>
				{refused}
			</figure>
		);
	}
	const what = [file.created === undefined ? undefined : file.created ? "Created" : "Edited", pdf ? "PDF" : kindOf(file), file.size !== undefined ? sizeText(file.size) : undefined]
		.filter((part) => part !== undefined)
		.join(" · ");
	return (
		<div className="sent-file" title={file.origin}>
			{viewing && <SentPreview sent={sent} actions={actions} onClose={() => setViewing(false)} />}
			<div className="sent-card">
				<FileIcon className="shrink-0 text-ink-3" />
				<span className="min-w-0 flex-1">
					<span className="block truncate">{file.name}</span>
					<span className="instrument block">{what}</span>
				</span>
				{actions}
			</div>
			{refused}
		</div>
	);
}

/** The picture, at most a column wide; pressed, it opens in the viewer with the same Save and folder actions. */
function SentPicture({
	personaId,
	eventId,
	index,
	file,
	viewing,
	onOpen,
	onClose,
	actions,
}: {
	personaId: string;
	eventId: string;
	index: number;
	file: Attachment;
	viewing: boolean;
	onOpen(): void;
	onClose(): void;
	actions: ReactNode;
}) {
	const url = useSentFile(personaId, eventId, index);
	const shape = file.width !== undefined && file.height !== undefined ? `${file.width} / ${file.height}` : "4 / 3";
	if (url === null) return <p className="sent-missing">{`${file.name} could not be read.`}</p>;
	if (url === undefined) return <div className="sent-picture sent-placeholder" style={{ aspectRatio: shape }} />;
	return (
		<>
			<button type="button" className="sent-open picture-open" title="Open full size" onClick={onOpen}>
				<img className="sent-picture" decoding="async" src={url} alt={file.name} width={file.width} height={file.height} />
			</button>
			{viewing && <Viewer src={url} alt={file.name} onClose={onClose} actions={actions} />}
		</>
	);
}

/** The longer side a picture you sent is read back at: the phone's size for it. */
const OWN_PICTURE_EDGE = 1280;

/**
 * A picture you sent, drawn over your bubble as the phone draws it: read back
 * from the copy the desk kept when it was sent, at the phone's size, and full
 * size on a press. A quiet shape of its size holds the place until it is read
 * (and while the line is still on its way, with no id the desk knows); a
 * message with nothing kept for it, from before the desk kept copies, keeps
 * its chip.
 */
export function OwnPicture({ personaId, eventId, index, file }: { personaId: string; eventId: string; index: number; file: Attachment }) {
	const url = useSentFile(personaId, eventId, index, OWN_PICTURE_EDGE, !eventId.startsWith("saying:"));
	const [viewing, setViewing] = useState(false);
	const ratio = file.width !== undefined && file.height !== undefined ? Math.min(2, Math.max(0.5, file.width / file.height)) : 4 / 3;
	if (url === null) {
		return (
			<span className="chip max-w-full pr-2" title={file.path}>
				<span className="chip-name">{file.name}</span>
			</span>
		);
	}
	if (url === undefined) return <div className="own-picture sent-placeholder" style={{ aspectRatio: ratio }} />;
	return (
		<>
			<button type="button" className="own-picture picture-open" style={{ aspectRatio: ratio }} title="Open full size" onClick={() => setViewing(true)}>
				<img decoding="async" src={url} alt={file.name} />
			</button>
			{viewing && <OwnViewer personaId={personaId} eventId={eventId} index={index} file={file} preview={url} onClose={() => setViewing(false)} />}
		</>
	);
}

/** Full size in the viewer, shown at the bubble's size until the whole picture is read. */
function OwnViewer({ personaId, eventId, index, file, preview, onClose }: { personaId: string; eventId: string; index: number; file: Attachment; preview: string; onClose(): void }) {
	const full = useSentFile(personaId, eventId, index);
	return <Viewer src={full ?? preview} alt={file.name} onClose={onClose} />;
}

/**
 * A file you sent that is not a picture: a card with what it is, as a
 * teammate's file has, opened from where it was on this computer. The desk
 * keeps no copy of it, so a desk on a server has nothing to open.
 */
export function OwnFile({ file }: { file: Attachment }) {
	const [error, setError] = useState<string | null>(null);
	const pdf = file.mimeType === "application/pdf";
	const what = [pdf ? "PDF" : kindOf(file), file.size !== undefined ? sizeText(file.size) : undefined].filter((part) => part !== undefined).join(" · ");
	return (
		<div className="own-file" title={file.path}>
			<div className="sent-card">
				<FileIcon className="shrink-0 text-ink-3" />
				<span className="min-w-0 flex-1">
					<span className="block truncate">{file.name}</span>
					<span className="instrument block">{what}</span>
				</span>
				{!onServer() && (
					<button
						type="button"
						className="control btn btn-sm"
						onClick={() => {
							setError(null);
							openSentFile(file.path).catch((failed: unknown) => setError(failed instanceof Error ? failed.message : String(failed)));
						}}
					>
						Open
					</button>
				)}
			</div>
			{error !== null && <p className="sent-error">{error}</p>}
		</div>
	);
}

/** How many pictures stay readable after their bubble leaves the screen. */
const KEPT = 48;
const pictures = new Map<string, Promise<string>>();

/**
 * The file's bytes as a URL the page can draw, read a part at a time. A sent
 * file never changes, so it is read once and the URL kept: switching back to a
 * teammate draws their pictures at once, and a URL is only let go when it is
 * the least recently asked for. A failed read is forgotten, so the next ask
 * tries again.
 */
function sentUrl(personaId: string, eventId: string, index: number, edge?: number): Promise<string> {
	const key = `${personaId}/${eventId}/${index}/${edge ?? "full"}`;
	const known = pictures.get(key);
	if (known !== undefined) {
		pictures.delete(key);
		pictures.set(key, known);
		return known;
	}
	const fetched = readSent(personaId, eventId, index, edge);
	pictures.set(key, fetched);
	fetched.catch(() => {
		if (pictures.get(key) === fetched) pictures.delete(key);
	});
	if (pictures.size > KEPT) {
		const oldest = pictures.keys().next().value as string;
		const gone = pictures.get(oldest);
		pictures.delete(oldest);
		void gone?.then((url) => URL.revokeObjectURL(url), () => {});
	}
	return fetched;
}

async function readSent(personaId: string, eventId: string, index: number, edge?: number): Promise<string> {
	const parts: Uint8Array<ArrayBuffer>[] = [];
	let type = "";
	let offset: number | undefined = 0;
	while (offset !== undefined) {
		const chunk: FileChunk = await wire.command("file.read", { personaId, eventId, index, offset, ...(edge !== undefined ? { size: edge } : {}) });
		type = chunk.mimeType;
		parts.push(await bytesOf(chunk.data));
		offset = chunk.next;
	}
	return URL.createObjectURL(new Blob(parts, { type }));
}

function useSentFile(personaId: string, eventId: string, index: number, edge?: number, ready = true): string | null | undefined {
	const [url, setUrl] = useState<string | null | undefined>(undefined);
	useEffect(() => {
		if (!ready) return;
		let gone = false;
		sentUrl(personaId, eventId, index, edge).then(
			(made) => {
				if (!gone) setUrl(made);
			},
			() => {
				if (!gone) setUrl(null);
			},
		);
		return () => {
			gone = true;
		};
	}, [personaId, eventId, index, edge, ready]);
	return url;
}

/** Base64 to bytes by the engine's own decoder where there is one: a loop over a megabyte of characters is its own wait. */
async function bytesOf(base64: string): Promise<Uint8Array<ArrayBuffer>> {
	const native = (Uint8Array as unknown as { fromBase64?: (text: string) => Uint8Array<ArrayBuffer> }).fromBase64;
	if (native !== undefined) return native.call(Uint8Array, base64);
	const response = await fetch(`data:application/octet-stream;base64,${base64}`);
	return new Uint8Array(await response.arrayBuffer());
}

/** What kind of file a card names, from its name's ending. */
function kindOf(file: Attachment): string {
	if (isMarkdown(file)) return "Markdown";
	const dot = file.name.lastIndexOf(".");
	return dot > 0 && dot < file.name.length - 1 ? `${file.name.slice(dot + 1).toUpperCase()} file` : "File";
}
