import { type ReactNode, useCallback, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import type { Attachment, FileChunk } from "../generated/contract";
import type { Sent } from "../serverFiles";
import { sizeText } from "../sizes";
import { wire } from "../wire";
import { MarkdownDocument } from "./Markdown";

/** The most of a markdown file read whole to render it; past it, it reads as text a page at a time. */
const DOCUMENT_BYTES = 2 * 1024 * 1024;

/** A markdown file, by its type or its name's ending. */
export function isMarkdown(file: Pick<Attachment, "name" | "mimeType">): boolean {
	return file.mimeType === "text/markdown" || /\.(md|markdown|mdown|mkd)$/i.test(file.name);
}

/**
 * A sent file opened on the desk. Markdown is rendered as a document, with its
 * source a press away; anything else is read as text, never as HTML or an
 * executable file association, a page at a time.
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
			<div className="viewer-bar bg-canvas text-ink">
				<span className="mr-auto truncate">{file.name}</span>
				<span className="flex gap-1.5">
					{renders && !unreadable && (
						<button type="button" className="control btn btn-sm" aria-pressed={source} onClick={() => setSource((was) => !was)}>
							{source ? "Rendered" : "Source"}
						</button>
					)}
					{actions}
				</span>
				<button type="button" className="control btn btn-sm" onClick={onClose}>Close</button>
			</div>
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

/** The file as text, or as bytes when it is not text, a part at a time. */
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
			let text: string;
			let binary = false;
			try {
				text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
				binary = text.includes("\0");
			} catch {
				text = "";
				binary = true;
			}
			if (binary) {
				const lines: string[] = [];
				for (let start = 0; start < bytes.length; start += 16) {
					lines.push(`${(offset + start).toString(16).padStart(8, "0")}  ${Array.from(bytes.subarray(start, start + 16), byte => byte.toString(16).padStart(2, "0")).join(" ")}`);
				}
				text = lines.join("\n");
			}
			setPage({ text, label: `${binary ? "Binary preview" : "Text preview"} · ${sizeText(bytes.length)}${offset ? ` from byte ${offset}` : ""}`, ...(chunk.next !== undefined ? { next: chunk.next } : {}) });
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
