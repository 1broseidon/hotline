import { type ReactNode, useEffect, useState } from "react";
import type { Attachment, FileChunk } from "../generated/contract";
import { FileIcon, FolderIcon } from "../icons";
import { onServer, openSent, saveSent, showPath } from "../serverFiles";
import { sizeText } from "../sizes";
import { Viewer } from "../ui/Viewer";
import { wire } from "../wire";

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
}: {
	personaId: string;
	eventId: string;
	index: number;
	file: Attachment;
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
			{pdf && (
				<button type="button" className="control btn btn-sm" onClick={() => act(() => openSent(sent))}>
					Open
				</button>
			)}
			<button type="button" className="control btn btn-sm" onClick={() => act(() => saveSent(sent))}>
				Save…
			</button>
			<button
				type="button"
				className="control btn btn-sm"
				title={onServer() ? "Show in its folder on the server" : "Show in folder"}
				aria-label={onServer() ? "Show in its folder on the server" : "Show in folder"}
				onClick={() => showPath(file.path)}
			>
				<FolderIcon />
			</button>
		</span>
	);
	const refused = error !== null && <p className="sent-error">{error}</p>;

	if (file.kind === "image") {
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
	const what = [pdf ? "PDF" : kindOf(file), file.size !== undefined ? sizeText(file.size) : undefined]
		.filter((part) => part !== undefined)
		.join(" · ");
	return (
		<div className="sent-file" title={file.origin}>
			<div className="sent-card">
				<FileIcon className="shrink-0 text-ink-3" />
				<span className="min-w-0 flex-1">
					<span className="block truncate">{file.name}</span>
					<span className="block text-ink-3">{what}</span>
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
				<img className="sent-picture" src={url} alt={file.name} width={file.width} height={file.height} />
			</button>
			{viewing && <Viewer src={url} alt={file.name} onClose={onClose} actions={actions} />}
		</>
	);
}

/**
 * The file's bytes as a URL the page can draw, read a part at a time.
 * Undefined while it comes, null when it cannot be read.
 */
function useSentFile(personaId: string, eventId: string, index: number): string | null | undefined {
	const [url, setUrl] = useState<string | null | undefined>(undefined);
	useEffect(() => {
		let gone = false;
		let made: string | undefined;
		void (async () => {
			const parts: Uint8Array<ArrayBuffer>[] = [];
			let type = "";
			let offset: number | undefined = 0;
			while (offset !== undefined) {
				const chunk: FileChunk = await wire.command("file.read", { personaId, eventId, index, offset });
				if (gone) return;
				type = chunk.mimeType;
				parts.push(bytesOf(chunk.data));
				offset = chunk.next;
			}
			made = URL.createObjectURL(new Blob(parts, { type }));
			setUrl(made);
		})().catch(() => {
			if (!gone) setUrl(null);
		});
		return () => {
			gone = true;
			if (made !== undefined) URL.revokeObjectURL(made);
		};
	}, [personaId, eventId, index]);
	return url;
}

function bytesOf(base64: string): Uint8Array<ArrayBuffer> {
	const text = atob(base64);
	const bytes = new Uint8Array(new ArrayBuffer(text.length));
	for (let index = 0; index < text.length; index++) bytes[index] = text.charCodeAt(index);
	return bytes;
}

/** What kind of file a card names, from its name's ending. */
function kindOf(file: Attachment): string {
	const dot = file.name.lastIndexOf(".");
	return dot > 0 && dot < file.name.length - 1 ? `${file.name.slice(dot + 1).toUpperCase()} file` : "File";
}
