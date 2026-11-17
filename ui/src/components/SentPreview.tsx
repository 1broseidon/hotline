import { type ReactNode, useCallback, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import type { Attachment, FileChunk } from "../generated/contract";
import type { Sent } from "../serverFiles";
import { sizeText } from "../sizes";
import { wire } from "../wire";
import { ViewerBar } from "../ui/Viewer";
import { SourceIcon } from "../icons";
import { MarkdownDocument } from "./Markdown";
import { SheetKey } from "./Sheet";

/** The most of a markdown file read whole to render it; past it, it reads as text a page at a time. */
const DOCUMENT_BYTES = 2 * 1024 * 1024;

/** A markdown file, by its type or its name's ending. */
export function isMarkdown(file: Pick<Attachment, "name" | "mimeType">): boolean {
	return file.mimeType === "text/markdown" || /\.(md|markdown|mdown|mkd)$/i.test(file.name);
}

/** The Word, Excel and PowerPoint types, which the desk gives a file only when its ZIP says it is that kind (sent/office.rs). */
const OFFICE_TYPES = new Set([
	"application/vnd.openxmlformats-officedocument.wordprocessingml.document",
	"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
	"application/vnd.openxmlformats-officedocument.presentationml.presentation",
]);

/**
 * Text, by its type. The desk names text by its bytes, always as `text/…`;
 * a desk before 0.50 named it by its ending, and gave JSON, scripts and
 * XML these types too.
 */
const TEXT_TYPE = /^text\/|^application\/(json|javascript|xml|x-sh|x-sql)$|\+(json|xml)$/;

/**
 * Where Open takes a teammate's file: out of Hotline to the system's own app
 * for a PDF or a Word, Excel or PowerPoint file, which the shell opens only
 * when the name and the contents agree (files.rs); into the viewer for text;
 * nowhere for anything else, which is only saved or shown in its folder. A
 * file with no type, from a desk that gave none, opens as text.
 */
export function opensIn(file: Pick<Attachment, "mimeType">): "outside" | "viewer" | null {
	const type = file.mimeType;
	if (type === undefined) return "viewer";
	if (type === "application/pdf" || OFFICE_TYPES.has(type)) return "outside";
	return TEXT_TYPE.test(type) ? "viewer" : null;
}

/**
 * A sent file opened on the desk: only text reaches it (`opensIn`). Markdown
 * is rendered as a document, with its source a press away; any other text is
 * read as text, never as HTML or an executable file association, a page at
 * a time.
 */
export function SentPreview({ sent, actions, onClose }: { sent: Sent; actions: ReactNode; onClose(): void }) {
	const { file } = sent;
	const renders = isMarkdown(file) && (file.size === undefined || file.size <= DOCUMENT_BYTES);
	const [source, setSource] = useState(false);
	const [unreadable, setUnreadable] = useState(false);
	const rendered = renders && !source && !unreadable;
	const giveUp = useCallback(() => setUnreadable(true), []);
	useEffect(() => {
		const close = (event: KeyboardEvent) => {
			if (event.key === "Escape") { event.stopPropagation(); onClose(); }
		};
		window.addEventListener("keydown", close, true);
		return () => window.removeEventListener("keydown", close, true);
	}, [onClose]);
	return createPortal(
		<div className="viewer" role="dialog" aria-modal="true" aria-label={file.name}>
			<ViewerBar fact={[file.name, file.size !== undefined ? sizeText(file.size) : null].filter(Boolean).join(" · ")} onClose={onClose}>
				{renders && !unreadable && (
					<SheetKey icon={<SourceIcon />} label="Source" title={source ? "Show it rendered" : "Show the source"} pressed={source} onClick={() => setSource((was) => !was)} />
				)}
				{actions}
			</ViewerBar>
			{rendered ? <DocumentPage sent={sent} onUnreadable={giveUp} /> : <TextPages sent={sent} />}
		</div>, document.body,
	);
}

/** The whole file read, then rendered as a document; a file that is not text goes to the text view. */
function DocumentPage({ sent, onUnreadable }: { sent: Sent; onUnreadable(): void }) {
	const { personaId, eventId, index } = sent;
	const [text, setText] = useState<string | null>(null);
	const [error, setError] = useState<string | null>(null);
	useEffect(() => {
		let gone = false;
		(async () => {
			const parts: Uint8Array[] = [];
			let read = 0;
			let offset: number | undefined = 0;
			while (offset !== undefined && read <= DOCUMENT_BYTES) {
				const chunk: FileChunk = await wire.command("file.read", { personaId, eventId, index, offset });
				const bytes = bytesOf(chunk.data);
				parts.push(bytes);
				read += bytes.length;
				offset = chunk.next;
			}
			if (gone) return;
			if (offset !== undefined) return onUnreadable();
			const whole = new Uint8Array(read);
			let at = 0;
			for (const part of parts) {
				whole.set(part, at);
				at += part.length;
			}
			try {
				setText(new TextDecoder("utf-8", { fatal: true }).decode(whole));
			} catch {
				onUnreadable();
			}
		})().catch((failure: unknown) => {
			if (!gone) setError(failure instanceof Error ? failure.message : String(failure));
		});
		return () => { gone = true; };
	}, [personaId, eventId, index, onUnreadable]);
	return (
		<div className="m-4 min-h-0 flex-1 overflow-auto rounded bg-canvas px-8 py-6 text-ink">
			{text !== null ? <MarkdownDocument text={text} /> : <p className="text-ink-3">{error ?? "Reading file…"}</p>}
		</div>
	);
}

/**
 * The file as text, a part at a time. The desk called it text by its start,
 * so a byte further on that is not text shows as a mark in its place.
 */
function TextPages({ sent }: { sent: Sent }) {
	const { personaId, eventId, index } = sent;
	const [page, setPage] = useState<{ text: string; label: string; next?: number } | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [offsets, setOffsets] = useState([0]);
	const offset = offsets[offsets.length - 1]!;
	useEffect(() => {
		let gone = false;
		setPage(null);
		setError(null);
		wire.command("file.read", { personaId, eventId, index, offset }).then(chunk => {
			if (gone) return;
			const bytes = bytesOf(chunk.data);
			const text = new TextDecoder("utf-8").decode(bytes);
			setPage({ text, label: `Text preview · ${sizeText(bytes.length)}${offset ? ` from byte ${offset}` : ""}`, ...(chunk.next !== undefined ? { next: chunk.next } : {}) });
		}).catch((failure: unknown) => {
			if (!gone) setError(failure instanceof Error ? failure.message : String(failure));
		});
		return () => { gone = true; };
	}, [personaId, eventId, index, offset]);
	return (
		<div className="m-4 min-h-0 flex-1 overflow-auto rounded bg-canvas p-4 text-ink">
			<p>{error ?? page?.label ?? "Reading file…"}</p>
			{page && <pre className="whitespace-pre-wrap break-words font-mono text-sm">{page.text}</pre>}
			{offsets.length > 1 && <button type="button" className="control btn btn-sm" onClick={() => setOffsets(previous => previous.slice(0, -1))}>Previous</button>}
			{page?.next !== undefined && <button type="button" className="control btn btn-sm" onClick={() => setOffsets(previous => [...previous, page.next!])}>Next</button>}
		</div>
	);
}

function bytesOf(base64: string): Uint8Array {
	return Uint8Array.from(atob(base64), character => character.charCodeAt(0));
}
