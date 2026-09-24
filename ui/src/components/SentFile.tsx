import { useEffect, useState } from "react";
import type { Attachment, FileChunk } from "../generated/contract";
import { FileIcon, FolderIcon } from "../icons";
import { openSentFile, revealPath, saveSentFile } from "../native";
import { sizeText } from "../sizes";
import { wire } from "../wire";

/**
 * A file a teammate sent with `send_file`, under the words it came with.
 *
 * A picture is drawn in the bubble, read over the wire by its message the
 * way a phone reads it, with its place held at its own shape until it
 * arrives. Anything else is a card that names it. Nothing opens on its own:
 * a picture or a PDF opens in the system's viewer when pressed, and any
 * file can be saved where the person chooses or shown in its folder. Where
 * the file came from is on hover.
 */
export function SentFile({ personaId, eventId, file }: { personaId: string; eventId: string; file: Attachment }) {
	const [error, setError] = useState<string | null>(null);
	const act = (work: () => Promise<unknown>) => {
		setError(null);
		work().catch((failed: unknown) => setError(failed instanceof Error ? failed.message : String(failed)));
	};
	const pdf = file.mimeType === "application/pdf";
	const actions = (
		<span className="sent-actions">
			{pdf && (
				<button type="button" className="control btn btn-sm" onClick={() => act(() => openSentFile(file.path))}>
					Open
				</button>
			)}
			<button type="button" className="control btn btn-sm" onClick={() => act(() => saveSentFile(file.path))}>
				Save…
			</button>
			<button
				type="button"
				className="control btn btn-sm"
				title="Show in folder"
				aria-label="Show in folder"
				onClick={() => act(() => revealPath(file.path))}
			>
				<FolderIcon />
			</button>
		</span>
	);
	const refused = error !== null && <p className="sent-error">{error}</p>;

	if (file.kind === "image") {
		return (
			<figure className="sent-file" title={file.origin}>
				<SentPicture personaId={personaId} eventId={eventId} file={file} onOpen={() => act(() => openSentFile(file.path))} />
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

/** The picture, at most a column wide; pressed, it opens full size in the system's viewer. */
function SentPicture({
	personaId,
	eventId,
	file,
	onOpen,
}: {
	personaId: string;
	eventId: string;
	file: Attachment;
	onOpen(): void;
}) {
	const url = useSentFile(personaId, eventId);
	const shape = file.width !== undefined && file.height !== undefined ? `${file.width} / ${file.height}` : "4 / 3";
	if (url === null) return <p className="sent-missing">{`${file.name} could not be read.`}</p>;
	if (url === undefined) return <div className="sent-picture sent-placeholder" style={{ aspectRatio: shape }} />;
	return (
		<button type="button" className="sent-open" title="Open full size" onClick={onOpen}>
			<img className="sent-picture" src={url} alt={file.name} width={file.width} height={file.height} />
		</button>
	);
}

/**
 * The file's bytes as a URL the page can draw, read a part at a time.
 * Undefined while it comes, null when it cannot be read.
 */
function useSentFile(personaId: string, eventId: string): string | null | undefined {
	const [url, setUrl] = useState<string | null | undefined>(undefined);
	useEffect(() => {
		let gone = false;
		let made: string | undefined;
		void (async () => {
			const parts: Uint8Array<ArrayBuffer>[] = [];
			let type = "";
			let offset: number | undefined = 0;
			while (offset !== undefined) {
				const chunk: FileChunk = await wire.command("file.read", { personaId, eventId, offset });
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
	}, [personaId, eventId]);
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
