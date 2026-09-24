import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { Attachment, SessionState } from "../generated/contract";
import type { Refill } from "./Conversation";
import { ArrowUpIcon, CloseIcon, PlusIcon, StopIcon } from "../icons";
import { pickFiles } from "../native";
import { sizeText } from "../sizes";

/** The field stops growing here, and scrolls from then on. */
const MAX_HEIGHT = 220;

/** A session that is between turns and can be spoken to right now. */
function isWorking(state: SessionState): boolean {
	return state === "starting" || state === "thinking";
}

/** A session that is started on the way, before what was typed is said. */
export function isDown(state: SessionState): boolean {
	return state === "idle" || state === "stopped" || state === "error";
}

/**
 * Where you type: one pill, the way a message to a person is typed.
 *
 * The field is always open, because the teammate is always there. Whether
 * a session is up behind them is plumbing: a message typed at one that is
 * not running starts it and then says the message, and nothing on screen
 * asks the person to know the difference. The send key is there only when
 * there is something to send. Stop stays separate while the teammate is
 * working, so a correction never needs an interruption first. Attach is the
 * plus at the left end. A reply being composed is a one-line quote at the
 * head of the pill, and chips there are files picked or dropped, never a
 * path typed or pasted into it. Escape puts the chips down first, then the
 * quote, then it interrupts a turn.
 */
export function Composer({
	personaId,
	name,
	state,
	replyQuote,
	onSend,
	refill,
	onCancel,
	onClearReply,
}: {
	personaId: string;
	name: string;
	state: SessionState;
	replyQuote: string | null;
	onSend(text: string, attachments: Attachment[]): void;
	/** Words a refused send handed back; a new nonce fills the field again. */
	refill?: Refill;
	onCancel(): void;
	onClearReply(): void;
}) {
	const [text, setText] = useState("");
	const [attachments, setAttachments] = useState<Attachment[]>([]);
	const area = useRef<HTMLTextAreaElement>(null);
	const working = isWorking(state);
	const hasContent = text.trim().length > 0 || attachments.length > 0;

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

	// A drop or the picker is how a path becomes a chip. The field never
	// parses what was typed or pasted, so a path you meant as words stays words.
	useEffect(() => {
		let cancelled = false;
		let stop: (() => void) | undefined;
		// getCurrentWebview() throws in a browser tab before a promise exists,
		// so the catch below never runs unless the call itself is guarded.
		try {
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
					// The desk's drop channel is missing; chips still come from nowhere.
				});
		} catch {
			// A browser tab is not the desk; drops are a desktop thing.
		}
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
		setText("");
		const sending = attachments;
		setAttachments([]);
		onSend(trimmed, sending);
	};

	// A refused send hands its words back. Keyed by the moment they were sent,
	// so the same words can come back twice and still fill the field.
	const filled = useRef(0);
	useEffect(() => {
		if (refill === undefined || refill.nonce === filled.current) return;
		filled.current = refill.nonce;
		setText(refill.text);
		setAttachments(refill.attachments);
		area.current?.focus();
	}, [refill]);

	const attach = async () => {
		const paths = await pickFiles();
		if (paths.length > 0) setAttachments((known) => mergeDropped(known, paths));
		area.current?.focus();
	};

	return (
		/* Positioned, so it paints over the scroller before it, which is
		   positioned too and would otherwise sit on the pill's top edge. */
		<div className="relative shrink-0 px-6 pb-4">
			<div className="composer mx-auto w-full max-w-[46rem]">
				<button type="button" className="composer-key composer-attach" title="Attach a file" aria-label="Attach a file" onClick={() => void attach()}>
					<PlusIcon />
				</button>
				<div className="composer-body">
					{replyQuote !== null && (
						<div className="flex items-center gap-2 pt-1">
							<p className="quote mb-0 min-w-0 flex-1">
								<span className="text-ink-3">Replying to </span>
								{replyQuote}
							</p>
							<button type="button" className="chip-x" aria-label="Stop replying" onClick={onClearReply}>
								<CloseIcon />
							</button>
						</div>
					)}
					{attachments.length > 0 && (
						<ul className="flex flex-wrap gap-1.5 pt-1.5">
							{attachments.map((item) => (
								<li key={item.path} className="chip" title={item.path}>
									<span className="chip-name">{item.name}</span>
									{item.size !== undefined && <span className="chip-size">{sizeText(item.size)}</span>}
									<button
										type="button"
										className="chip-x"
										aria-label={`Remove ${item.name}`}
										onClick={() => setAttachments((known) => known.filter((one) => one.path !== item.path))}
									>
										<CloseIcon />
									</button>
								</li>
							))}
						</ul>
					)}
					<textarea
						ref={area}
						rows={1}
						value={text}
						aria-label={`Message ${name}`}
						placeholder="Message"
						onChange={(event) => setText(event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
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
				</div>
				{working && (
					<button type="button" className="composer-key composer-stop" title="Interrupt (Esc)" aria-label="Interrupt" onClick={onCancel}>
						<StopIcon />
					</button>
				)}
				{(!working || hasContent) && (
					<button
						type="button"
						className="composer-key composer-send"
						title="Send (Enter)"
						aria-label="Send"
						aria-hidden={!hasContent}
						tabIndex={hasContent ? 0 : -1}
						data-shown={hasContent ? "true" : undefined}
						onClick={submit}
					>
						<ArrowUpIcon />
					</button>
				)}
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

const IMAGE_EXTENSIONS = new Set([
	"avif", "bmp", "gif", "heic", "heif", "ico", "jpeg", "jpg", "png", "svg", "tif", "tiff", "webp",
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
