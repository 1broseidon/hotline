import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { Attachment, SessionState } from "../generated/contract";

/** The field stops growing here, and scrolls from then on. */
const MAX_HEIGHT = 200;

/** A session that is between turns and can be spoken to right now. */
export function isWorking(state: SessionState): boolean {
	return state === "starting" || state === "thinking";
}

/** A session that has to be started before anything can be said to it. */
function isDown(state: SessionState): boolean {
	return state === "idle" || state === "stopped" || state === "error";
}

/**
 * Where you type.
 *
 * A stopped teammate is started by talking to it: a message typed at a session
 * that is not running starts one and then says the message, because the person
 * meant to send it either way. The Start button is for the other case — waking
 * a teammate with nothing to say yet. A reply being composed sits as a
 * one-line quote above the field; chips above the field are files dropped on
 * the window, never a path typed or pasted into it. Escape puts the chips
 * down first, then the quote, then it interrupts a turn.
 */
export function Composer({
	personaId,
	state,
	replyQuote,
	onSend,
	onStart,
	onCancel,
	onClearReply,
}: {
	personaId: string;
	state: SessionState;
	replyQuote: string | null;
	onSend(text: string, attachments: Attachment[]): void;
	onStart(): void;
	onCancel(): void;
	onClearReply(): void;
}) {
	const [text, setText] = useState("");
	const [attachments, setAttachments] = useState<Attachment[]>([]);
	const area = useRef<HTMLTextAreaElement>(null);
	const working = isWorking(state);
	const down = isDown(state);
	const hasText = text.trim().length > 0;
	const hasContent = hasText || attachments.length > 0;

	// Grow with content, up to a ceiling. Before paint, because measuring after
	// it draws a wrapped line at the old height for one frame first.
	useLayoutEffect(() => {
		const el = area.current;
		if (!el) return;
		el.style.height = "auto";
		el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT)}px`;
	}, [text, personaId]);

	// The quote is a decision just made: the field is where the answer goes.
	useLayoutEffect(() => {
		if (replyQuote !== null) area.current?.focus();
	}, [replyQuote]);

	// A drop is the only way a path becomes a chip. The field never parses
	// what was typed or pasted, so a path you meant as words stays words.
	useEffect(() => {
		let cancelled = false;
		let stop: (() => void) | undefined;
		void getCurrentWebview()
			.onDragDropEvent((event) => {
				const payload = event.payload;
				if (payload.type !== "drop") return;
				setAttachments((known) => mergeDropped(known, payload.paths));
			})
			.then((unlisten) => {
				if (cancelled) {
					unlisten();
					return;
				}
				stop = unlisten;
			})
			.catch(() => {
				// A browser tab is not the desk; drops are a desktop thing.
			});
		return () => {
			cancelled = true;
			stop?.();
		};
	}, []);

	// Chips belong to the window, not only the field, so Escape puts them
	// down even when the field is not focused — and it does so before the
	// conversation's listener puts the quote down.
	useEffect(() => {
		if (attachments.length === 0) return;
		const onKey = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			event.preventDefault();
			event.stopPropagation();
			setAttachments([]);
		};
		window.addEventListener("keydown", onKey, true);
		return () => window.removeEventListener("keydown", onKey, true);
	}, [attachments.length]);

	const submit = () => {
		const trimmed = text.trim();
		if (!trimmed && attachments.length === 0) return;
		if (down) onStart();
		setText("");
		const sending = attachments;
		setAttachments([]);
		onSend(trimmed, sending);
	};

	return (
		<div className="border-t border-rule bg-paper px-6 py-3">
			<div className="mx-auto w-full max-w-[46rem]">
				{replyQuote !== null && (
					<div className="reply-chip">
						<p className="reply-chip-quote">{replyQuote}</p>
						<button
							type="button"
							className="reply-chip-clear"
							aria-label="Stop replying"
							onClick={onClearReply}
						>
							×
						</button>
					</div>
				)}
				{attachments.length > 0 && (
					<ul className="chip-tray">
						{attachments.map((item) => (
							<li key={item.path} className="chip" title={item.path}>
								<span className="chip-name">{item.name}</span>
								{item.size !== undefined && (
									<span className="chip-size">{sizeText(item.size)}</span>
								)}
								<button
									type="button"
									className="chip-drop"
									aria-label={`Remove ${item.name}`}
									onClick={() =>
										setAttachments((known) => known.filter((one) => one.path !== item.path))
									}
								>
									×
								</button>
							</li>
						))}
					</ul>
				)}
				<div className="flex items-end gap-2">
					<textarea
						ref={area}
						rows={1}
						value={text}
						aria-label="Message your teammate"
						placeholder={down ? "Message — sending starts the session" : "Message"}
						className="field resize-none"
						onChange={(event) => setText(event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Enter" && !event.shiftKey) {
								event.preventDefault();
								submit();
								return;
							}
							// Chips are put down on the window first (capture), so
							// Escape clears them before this field sees the key.
							if (event.key === "Escape" && replyQuote !== null) {
								event.preventDefault();
								onClearReply();
								return;
							}
							// Interrupting with nothing to say is still just Escape,
							// whatever is sitting half-written in the field.
							if (event.key === "Escape" && working) {
								event.preventDefault();
								onCancel();
							}
						}}
					/>

					{working ? (
						<button type="button" className="btn-quiet" title="Interrupt (Esc)" onClick={onCancel}>
							Stop
						</button>
					) : down && !hasContent ? (
						<button type="button" className="btn-quiet" onClick={onStart}>
							Start
						</button>
					) : (
						<button
							type="button"
							className="btn-primary"
							title="Send (Enter)"
							disabled={!hasContent}
							onClick={submit}
						>
							Send
						</button>
					)}
				</div>
			</div>
		</div>
	);
}

/**
 * A dropped path becomes a chip. Size is omitted unless the drop named it —
 * Tauri's drop names paths, not bytes, and guessing from disk is a second
 * hop this window does not take.
 */
function mergeDropped(known: Attachment[], paths: string[]): Attachment[] {
	const have = new Set(known.map((item) => item.path));
	const next = known.slice();
	for (const path of paths) {
		if (have.has(path)) continue;
		have.add(path);
		next.push(fromDroppedPath(path));
	}
	return next;
}

function fromDroppedPath(path: string): Attachment {
	const name = basename(path);
	const extension = name.includes(".") ? (name.split(".").pop() ?? "").toLowerCase() : "";
	const kind = IMAGE_EXTENSIONS.has(extension) ? "image" : "file";
	const mimeType = MIME_BY_EXTENSION[extension];
	return mimeType === undefined ? { kind, name, path } : { kind, name, path, mimeType };
}

function basename(path: string): string {
	const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
	return slash === -1 ? path : path.slice(slash + 1);
}

function sizeText(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	const kb = bytes / 1024;
	return kb < 1024 ? `${Math.round(kb)} KB` : `${(kb / 1024).toFixed(1)} MB`;
}

const IMAGE_EXTENSIONS = new Set([
	"avif",
	"bmp",
	"gif",
	"heic",
	"heif",
	"ico",
	"jpeg",
	"jpg",
	"png",
	"svg",
	"tif",
	"tiff",
	"webp",
]);

const MIME_BY_EXTENSION: Record<string, string> = {
	avif: "image/avif",
	bmp: "image/bmp",
	css: "text/css",
	csv: "text/csv",
	gif: "image/gif",
	heic: "image/heic",
	heif: "image/heif",
	htm: "text/html",
	html: "text/html",
	ico: "image/x-icon",
	jpeg: "image/jpeg",
	jpg: "image/jpeg",
	js: "text/javascript",
	json: "application/json",
	md: "text/markdown",
	mp3: "audio/mpeg",
	mp4: "video/mp4",
	pdf: "application/pdf",
	png: "image/png",
	svg: "image/svg+xml",
	tif: "image/tiff",
	tiff: "image/tiff",
	txt: "text/plain",
	wav: "audio/wav",
	webm: "video/webm",
	webp: "image/webp",
	xml: "application/xml",
	zip: "application/zip",
};
