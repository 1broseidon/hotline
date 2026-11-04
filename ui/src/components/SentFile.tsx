import { useEffect, useState } from "react";
import type { Attachment, FileChunk } from "../generated/contract";
import { FolderIcon, SaveIcon } from "../icons";
import { openSentFile } from "../native";
import { onServer, openSent, saveSent, showPath, type Sent } from "../serverFiles";
import { sizeText } from "../sizes";
import { Viewer } from "../ui/Viewer";
import { wire } from "../wire";
import { SentPreview, isMarkdown } from "./SentPreview";

/** One file a message carried: the message, its place in it, and the file. */
export type SentItem = { eventId: string; index: number; file: Attachment };

/**
 * The files that came with one turn, drawn as a messages app draws them:
 * pictures as themselves, one capped with its name and size under it, or
 * several in a strip; any other file a one-line row in one block, whose head
 * counts them, sums their size and saves them all.
 *
 * A teammate's file is read over the wire by its message, the way a phone
 * reads it, and nothing opens on its own: a picture opens in the window's own
 * viewer when pressed, a PDF in the system's, anything else in the window's
 * preview; any of them can be saved where the person chooses or shown in its
 * folder. On a desk on a server the file is brought down to open or save it,
 * and its folder is the server's (serverFiles.ts).
 *
 * Yours (`mine`) is read back from the copy the desk kept when it was sent,
 * at the phone's size, and full size on a press; a file that is not a
 * picture opens from where it was on this computer, since the desk keeps no
 * copy of it, so a desk on a server has nothing to open.
 */
export function SentFiles({ personaId, items, mine = false }: { personaId: string; items: SentItem[]; mine?: boolean }) {
	const pictures = items.filter((item) => item.file.kind === "image");
	const files = items.filter((item) => item.file.kind !== "image");
	return (
		<>
			{pictures.length === 1 && <Picture personaId={personaId} item={pictures[0]!} mine={mine} />}
			{pictures.length > 1 && <PictureStrip personaId={personaId} items={pictures} mine={mine} />}
			{files.length > 0 && <FileBlock personaId={personaId} items={files} mine={mine} />}
		</>
	);
}

/** "PDF", "HTML", "MD": what kind of file a row is, from its name, in a few letters. */
function markOf(file: Attachment): string {
	if (file.mimeType === "application/pdf") return "PDF";
	if (isMarkdown(file)) return "MD";
	const dot = file.name.lastIndexOf(".");
	return dot > 0 && dot < file.name.length - 1 ? file.name.slice(dot + 1, dot + 5).toUpperCase() : "FILE";
}

/** What a row says at its end: whether the teammate made or changed the file, and its size. */
function factsOf(file: Attachment): string {
	const made = file.created === undefined ? undefined : file.created ? "Created" : "Edited";
	return [made, file.size !== undefined ? sizeText(file.size) : undefined].filter((part) => part !== undefined).join(" · ");
}

/** What went wrong with the last press, said under the block. */
function useRefusal(): [string | null, (work: () => Promise<unknown>) => void] {
	const [error, setError] = useState<string | null>(null);
	const act = (work: () => Promise<unknown>) => {
		setError(null);
		work().catch((failed: unknown) => setError(failed instanceof Error ? failed.message : String(failed)));
	};
	return [error, act];
}

/**
 * Files that are not pictures, one row each: a type mark, the name and the
 * size. A row's keys show where the size was on hover or keyboard focus, so
 * a column of files reads as a list and not as a form. Several have a head:
 * how many, their size together, and Save all, which asks where to put each
 * in turn and stops when a dialog is dismissed.
 */
function FileBlock({ personaId, items, mine }: { personaId: string; items: SentItem[]; mine: boolean }) {
	const [error, act] = useRefusal();
	const [previewing, setPreviewing] = useState<Sent | null>(null);
	const sent = (item: SentItem): Sent => ({ personaId, ...item });
	const sizes = items.map((item) => item.file.size);
	const total = sizes.every((size) => size !== undefined) ? sizeText(sizes.reduce<number>((sum, size) => sum + size!, 0)) : undefined;
	const count = `${items.length} files`;
	const saveAll = async () => {
		for (const item of items) if ((await saveSent(sent(item))) === null) return;
	};
	return (
		<section className="file-block" aria-label={items.length === 1 ? items[0]!.file.name : count}>
			{items.length > 1 && (
				<div className="file-block-head">
					<span className="line-label">
						{count}
						{total !== undefined && (
							<>
								{" · "}
								<b>{total}</b>
							</>
						)}
					</span>
					{!mine && (
						<button type="button" className="line-key" onClick={() => act(saveAll)}>
							<SaveIcon />
							Save all
						</button>
					)}
				</div>
			)}
			{items.map((item) => (
				<div key={`${item.eventId}/${item.index}`} className="file-line">
					<span className="file-mark" aria-hidden="true">
						{markOf(item.file)}
					</span>
					<span className="file-name">{item.file.name}</span>
					<span className="file-end">
						<span className="file-size">{factsOf(item.file)}</span>
						<span className="file-keys">
							{mine ? (
								!onServer() && (
									<button type="button" className="line-key" onClick={() => act(() => openSentFile(item.file.path))}>
										Open
									</button>
								)
							) : (
								<>
									<button
										type="button"
										className="line-key"
										onClick={() =>
											item.file.mimeType === "application/pdf" ? act(() => openSent(sent(item))) : setPreviewing(sent(item))
										}
									>
										Open
									</button>
									<SentKeys sent={sent(item)} act={act} />
								</>
							)}
						</span>
					</span>
				</div>
			))}
			{error !== null && <p className="sent-error">{error}</p>}
			{previewing !== null && <SentPreview sent={previewing} actions={<SentKeys sent={previewing} act={act} />} onClose={() => setPreviewing(null)} />}
		</section>
	);
}

/** Save and Show in folder, on a row and over the viewer. */
function SentKeys({ sent, act }: { sent: Sent; act: (work: () => Promise<unknown>) => void }) {
	return (
		<>
			<button type="button" className="line-key" onClick={() => act(() => saveSent(sent))}>
				<SaveIcon />
				Save
			</button>
			<button
				type="button"
				className="line-key"
				title={onServer() ? "Show on the server" : "Show in folder"}
				aria-label={onServer() ? "Show in its folder on the server" : "Show in folder"}
				onClick={() => showPath(sent.file.path)}
			>
				<FolderIcon />
			</button>
		</>
	);
}

/** The longer side a picture you sent is read back at: the phone's size for it. */
const OWN_PICTURE_EDGE = 1280;

/** A picture's URL to draw it in the conversation: yours at the phone's size, once the desk knows its message. */
function usePicture(personaId: string, item: SentItem, mine: boolean): string | null | undefined {
	return useSentFile(personaId, item.eventId, item.index, mine ? OWN_PICTURE_EDGE : undefined, !mine || !item.eventId.startsWith("saying:"));
}

/**
 * One picture, at most a little over a phone's width and 260px tall, cropped
 * from the top when it is taller, with its name and size under it. Its place
 * is held at its own shape until it arrives; pressed, it opens full size.
 */
function Picture({ personaId, item, mine }: { personaId: string; item: SentItem; mine: boolean }) {
	const url = usePicture(personaId, item, mine);
	const [viewing, setViewing] = useState(false);
	const { file } = item;
	const frame = {
		aspectRatio: file.width !== undefined && file.height !== undefined ? `${file.width} / ${file.height}` : "4 / 3",
		...(file.width !== undefined ? { width: `min(${file.width}px, 320px)` } : {}),
	};
	if (url === null && mine) {
		return (
			<span className="chip max-w-full pr-2">
				<span className="chip-name">{file.name}</span>
			</span>
		);
	}
	if (url === null) return <p className="sent-missing">{`${file.name} could not be read.`}</p>;
	return (
		<figure className="picture" data-mine={mine || undefined}>
			{url === undefined ? (
				<div className="picture-frame sent-placeholder" style={frame} />
			) : (
				<button type="button" className="picture-frame picture-open" style={frame} title="Open full size" onClick={() => setViewing(true)}>
					<img decoding="async" src={url} alt={file.name} />
				</button>
			)}
			<figcaption className="picture-caption">
				<span className="chip-name">{file.name}</span>
				{file.size !== undefined && <span className="shrink-0"> · {sizeText(file.size)}</span>}
			</figcaption>
			{viewing && url !== undefined && <PictureViewer personaId={personaId} item={item} mine={mine} preview={url} onClose={() => setViewing(false)} />}
		</figure>
	);
}

/** How many pictures a strip shows before its last tile counts the rest. */
const STRIP = 3;

/**
 * Several pictures from one turn, side by side in one shape with its outer
 * corners rounded alike. Past three, the last tile says how many more, and
 * pressing it lays them all out; any other tile opens its picture full size.
 */
function PictureStrip({ personaId, items, mine }: { personaId: string; items: SentItem[]; mine: boolean }) {
	const [all, setAll] = useState(false);
	const shown = all ? items : items.slice(0, STRIP);
	const more = items.length - shown.length;
	return (
		<section className="picture-strip" data-tiles={all ? "all" : shown.length} data-mine={mine || undefined} aria-label={`${items.length} pictures`}>
			{shown.map((item, at) => (
				<PictureTile
					key={`${item.eventId}/${item.index}`}
					personaId={personaId}
					item={item}
					mine={mine}
					{...(at === shown.length - 1 && more > 0 ? { more, onMore: () => setAll(true) } : {})}
				/>
			))}
		</section>
	);
}

function PictureTile({ personaId, item, mine, more, onMore }: { personaId: string; item: SentItem; mine: boolean; more?: number; onMore?(): void }) {
	const url = usePicture(personaId, item, mine);
	const [viewing, setViewing] = useState(false);
	const ready = typeof url === "string";
	// The count of the rest works before its picture arrives; a picture opens once it has.
	return (
		<>
			<button
				type="button"
				className={`picture-tile ${ready ? "picture-open" : "sent-placeholder"}`}
				disabled={more === undefined && !ready}
				aria-label={more !== undefined ? `Show ${more} more` : url === null ? `${item.file.name} could not be read` : item.file.name}
				title={more !== undefined ? "Show all" : ready ? "Open full size" : undefined}
				onClick={() => (onMore !== undefined ? onMore() : setViewing(true))}
			>
				{ready && <img decoding="async" src={url} alt="" />}
				{more !== undefined && (
					<span className="picture-more">
						<span>+{more}</span>
					</span>
				)}
			</button>
			{viewing && ready && <PictureViewer personaId={personaId} item={item} mine={mine} preview={url} onClose={() => setViewing(false)} />}
		</>
	);
}

/**
 * Full size in the window's viewer. Yours is shown at the bubble's size until
 * the whole picture is read; a teammate's was read whole, and carries Save and
 * Show in folder over it.
 */
function PictureViewer({ personaId, item, mine, preview, onClose }: { personaId: string; item: SentItem; mine: boolean; preview: string; onClose(): void }) {
	const full = useSentFile(personaId, item.eventId, item.index, undefined, mine);
	const [error, act] = useRefusal();
	if (mine) return <Viewer src={full ?? preview} alt={item.file.name} onClose={onClose} />;
	return (
		<Viewer
			src={preview}
			alt={item.file.name}
			onClose={onClose}
			actions={
				<>
					<SentKeys sent={{ personaId, ...item }} act={act} />
					{error !== null && <span className="sent-error">{error}</span>}
				</>
			}
		/>
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

