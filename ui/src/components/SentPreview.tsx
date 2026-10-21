import { type ReactNode, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import type { Sent } from "../serverFiles";
import { sizeText } from "../sizes";
import { wire } from "../wire";

/** Read stored bytes as text, never as HTML or an executable file association. */
export function SentPreview({ sent, actions, onClose }: { sent: Sent; actions: ReactNode; onClose(): void }) {
	const { personaId, eventId, index, file } = sent;
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
			const bytes = Uint8Array.from(atob(chunk.data), character => character.charCodeAt(0));
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
				{actions}
				<button type="button" className="control btn btn-sm" onClick={onClose}>Close</button>
			</div>
			<div className="m-4 min-h-0 flex-1 overflow-auto rounded bg-canvas p-4 text-ink">
				<p>{error ?? page?.label ?? "Reading file…"}</p>
				{page && <pre className="whitespace-pre-wrap break-words font-mono text-sm">{page.text}</pre>}
				{offsets.length > 1 && <button type="button" className="control btn btn-sm" onClick={() => setOffsets(previous => previous.slice(0, -1))}>Previous</button>}
				{page?.next !== undefined && <button type="button" className="control btn btn-sm" onClick={() => setOffsets(previous => [...previous, page.next!])}>Next</button>}
			</div>
		</div>, document.body,
	);
}
