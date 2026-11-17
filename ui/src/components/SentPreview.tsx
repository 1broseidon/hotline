import { type ReactNode, useCallback, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import type { Attachment, FileChunk } from "../generated/contract";
import type { Sent } from "../serverFiles";
import { sizeText } from "../sizes";
import { wire } from "../wire";
import { ViewerBar } from "../ui/Viewer";
import { SourceIcon } from "../icons";
import { parseDelimited } from "../delimited";
import { count, MarkdownDocument } from "./Markdown";
import { SheetKey } from "./Sheet";

/** The most of a markdown or table file read whole to render it; past it, it reads as text a page at a time. */
const DOCUMENT_BYTES = 2 * 1024 * 1024;

/** The most rows a table file is drawn with; past it, it reads as text a page at a time. */
const TABLE_ROWS = 5000;

/** A markdown file, by its type or its name's ending. */
export function isMarkdown(file: Pick<Attachment, "name" | "mimeType">): boolean {
	return file.mimeType === "text/markdown" || /\.(md|markdown|mdown|mkd)$/i.test(file.name);
}

/** What separates a table file's cells, by its type or its name's ending: a CSV's comma, a TSV's tab; none for any other file. */
function separatorOf(file: Pick<Attachment, "name" | "mimeType">): "," | "\t" | null {
	if (file.mimeType === "text/csv" || /\.csv$/i.test(file.name)) return ",";
	if (file.mimeType === "text/tab-separated-values" || /\.tsv$/i.test(file.name)) return "\t";
	return null;
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
 * Source and config files, by their name's ending or their whole name. This
 * serves files kept before 0.50: a desk then named a file by its ending
 * alone, and gave these `application/octet-stream` (or a video's type, for
 * `.ts`), though they opened as text. A 0.50 desk names text by its bytes,
 * so it never needs this; a file it kept under one of these names that is
 * not text opens as text with replacement marks where its bytes are not,
 * which is no worse than its name promised.
 */
const SOURCE_NAME =
	/(^|\.)(ts|tsx|js|jsx|mjs|cjs|vue|svelte|go|rb|swift|kt|kts|java|scala|gradle|rs|py|c|h|cc|cpp|hpp|cs|php|lua|pl|r|dart|ex|exs|hs|sql|graphql|proto|tf|sh|bash|zsh|fish|ps1|toml|yaml|yml|ini|cfg|conf|env|properties|lock|log|diff|patch|gitignore|dockerignore|editorconfig|dockerfile|makefile)$/i;

/**
 * Where Open takes a teammate's file: out of Hotline to the system's own app
 * for a PDF, a web page, or a Word, Excel or PowerPoint file, which the shell
 * opens only when the name and the contents agree (files.rs); a web page
 * goes to the default browser and is never drawn in the window, since a
 * scripted page in a frame of the window can send data out. Into the viewer
 * for other text;
 * nowhere for anything else, which is only saved or shown in its folder. A
 * file with no type, from a desk that gave none, goes by its name: a page
 * named `.html` or `.htm` to the browser, anything else into the viewer as
 * text. A source or config file an earlier desk gave a type that is not
 * text opens as text too (`SOURCE_NAME`).
 */
export function opensIn(file: Pick<Attachment, "name" | "mimeType">): "outside" | "viewer" | null {
	const type = file.mimeType;
	if (type === undefined) return /\.html?$/i.test(file.name) ? "outside" : "viewer";
	if (type === "application/pdf" || type === "text/html" || OFFICE_TYPES.has(type)) return "outside";
	return TEXT_TYPE.test(type) || SOURCE_NAME.test(file.name) ? "viewer" : null;
}

/** How big a table file is, for the band: its rows under the head, and its widest row's cells. */
type Shape = { rows: number; columns: number };

/**
 * A sent file opened on the desk: only text other than a web page reaches it
 * (`opensIn`), and nothing in it is drawn as HTML. Markdown
 * is rendered as a document, and a CSV or TSV as a table, each with its
 * source a press away; any other text is read as text, never as HTML or an
 * executable file association, a page at a time.
 */
export function SentPreview({ sent, actions, onClose }: { sent: Sent; actions: ReactNode; onClose(): void }) {
	const { file } = sent;
	const separator = separatorOf(file);
	const renders = (isMarkdown(file) || separator !== null) && (file.size === undefined || file.size <= DOCUMENT_BYTES);
	const [source, setSource] = useState(false);
	const [unreadable, setUnreadable] = useState(false);
	const [shape, setShape] = useState<Shape | null>(null);
	const rendered = renders && !source && !unreadable;
	const giveUp = useCallback(() => setUnreadable(true), []);
	const fact = shape !== null && !unreadable ? `${count(shape.rows, "row")} × ${count(shape.columns, "column")}` : file.size !== undefined ? sizeText(file.size) : null;
	useEffect(() => {
		const close = (event: KeyboardEvent) => {
			if (event.key === "Escape") { event.stopPropagation(); onClose(); }
		};
		window.addEventListener("keydown", close, true);
		return () => window.removeEventListener("keydown", close, true);
	}, [onClose]);
	return createPortal(
		<div className="viewer" role="dialog" aria-modal="true" aria-label={file.name}>
			<ViewerBar fact={[file.name, fact].filter(Boolean).join(" · ")} onClose={onClose}>
				{renders && !unreadable && (
					<SheetKey icon={<SourceIcon />} label="Source" title={source ? "Show it rendered" : "Show the source"} pressed={source} onClick={() => setSource((was) => !was)} />
				)}
				{actions}
			</ViewerBar>
			{rendered ? <DocumentPage sent={sent} separator={separator} onShape={setShape} onUnreadable={giveUp} /> : <TextPages sent={sent} />}
		</div>, document.body,
	);
}

/**
 * The whole file read, then rendered: as a table when it has a separator,
 * else as a markdown document. A file that is not UTF-8, or a table of more
 * than `TABLE_ROWS` rows, goes to the text view.
 */
function DocumentPage({ sent, separator, onShape, onUnreadable }: { sent: Sent; separator: "," | "\t" | null; onShape(shape: Shape): void; onUnreadable(): void }) {
	const { personaId, eventId, index } = sent;
	const [drawn, setDrawn] = useState<{ markdown: string } | { table: string[][] } | null>(null);
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
			let text: string;
			try {
				text = new TextDecoder("utf-8", { fatal: true }).decode(whole);
			} catch {
				return onUnreadable();
			}
			if (separator === null) return setDrawn({ markdown: text });
			const table = parseDelimited(text, separator);
			if (table.length === 0 || table.length - 1 > TABLE_ROWS) return onUnreadable();
			onShape({ rows: table.length - 1, columns: Math.max(...table.map((row) => row.length)) });
			setDrawn({ table });
		})().catch((failure: unknown) => {
			if (!gone) setError(failure instanceof Error ? failure.message : String(failure));
		});
		return () => { gone = true; };
	}, [personaId, eventId, index, separator, onShape, onUnreadable]);
	return (
		<div className="m-4 min-h-0 flex-1 overflow-auto rounded bg-canvas px-8 py-6 text-ink">
			{drawn === null ? (
				<p className="text-ink-3">{error ?? "Reading file…"}</p>
			) : "markdown" in drawn ? (
				<MarkdownDocument text={drawn.markdown} />
			) : (
				<ValuesTable rows={drawn.table} />
			)}
		</div>
	);
}

/**
 * A table file drawn as the app draws a Markdown table in a document: its
 * first row the head, and every row as wide as the widest, a short one
 * filled out with empty cells.
 */
function ValuesTable({ rows }: { rows: string[][] }) {
	const columns = Math.max(...rows.map((row) => row.length));
	const cells = (row: string[]) => Array.from({ length: columns }, (_, column) => row[column] ?? "");
	const [head = [], ...body] = rows;
	return (
		<article className="md md-doc">
			<div className="md-table">
				<table>
					<thead>
						<tr>
							{/* biome-ignore lint/suspicious/noArrayIndexKey: a column is its place; the file never changes under it. */}
							{cells(head).map((cell, column) => <th key={column}>{cell}</th>)}
						</tr>
					</thead>
					<tbody>
						{body.map((row, line) => (
							// biome-ignore lint/suspicious/noArrayIndexKey: a row is its place; the file never changes under it.
							<tr key={line}>
								{/* biome-ignore lint/suspicious/noArrayIndexKey: a column is its place; the file never changes under it. */}
								{cells(row).map((cell, column) => <td key={column}>{cell}</td>)}
							</tr>
						))}
					</tbody>
				</table>
			</div>
		</article>
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
