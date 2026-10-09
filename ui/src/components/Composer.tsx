import { type PointerEvent as ButtonPointerEvent, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { Attachment, SessionState } from "../generated/contract";
import type { Refill } from "./Conversation";
import { VoiceMeter } from "./VoiceMeter";
import { ArrowUpIcon, CloseIcon, MicIcon, PlusIcon, StopIcon, VoiceIcon } from "../icons";
import { readImage } from "@tauri-apps/plugin-clipboard-manager";
import { pickAttachments, stage } from "../serverFiles";
import { sizeText } from "../sizes";
import { readDraft, writeDraft } from "../drafts";
import { hotkeyLabel, useHotkeys } from "../hotkeys";
import { useCall, useCallSnapshot } from "../voice/call";
import {
	Dictation,
	SEND_AFTER_MS,
	SendCountdown,
	TapOrHold,
	afterDictation,
	dictationEngine,
	takeDictationRequests,
	useDictationAvailable,
} from "../voice/dictation";

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
 * asks the person to know the difference. An empty field offers voice:
 * dictation where this Mac or the desk hears speech itself, the words
 * landing in the field to be read and sent by hand, and a call elsewhere. Words or files
 * replace it with Send. Stop stays separate while the teammate is
 * working, so a correction never needs an interruption first. Attach is the
 * plus at the left end. A reply being composed is a one-line quote at the
 * head of the pill, and chips there are files picked, dropped or pasted,
 * never a path typed or pasted into it as words. Escape gives up a dictation
 * first, then puts the chips down, then the quote, then it interrupts a turn.
 */
export function Composer({
	personaId,
	name,
	state,
	replyQuote,
	onSend,
	onCall,
	refill,
	onCancel,
	onClearReply,
	onDraftChange,
	embedded = false,
}: {
	personaId: string;
	name: string;
	state: SessionState;
	replyQuote: string | null;
	onSend(text: string, attachments: Attachment[]): void;
	onCall?: (() => void) | undefined;
	/** Words a refused send handed back; a new nonce fills the field again. */
	refill?: Refill;
	onCancel(): void;
	onClearReply(): void;
	onDraftChange?(hasContent: boolean): void;
	/**
	 * Inside another composer's window (a side thread's): the same size and
	 * place as the conversation's, and it leaves file drops to the conversation's own composer, which is the one a drop
	 * on the window is meant for.
	 */
	embedded?: boolean;
}) {
	// Each conversation's draft is its own: switching teammates or threads
	// puts this one away and takes that one's out, rather than carrying the
	// words across or dropping them.
	const [text, setText] = useState(() => readDraft(personaId).text);
	const [attachments, setAttachments] = useState<Attachment[]>(() => readDraft(personaId).attachments);
	const [draftOf, setDraftOf] = useState(personaId);
	if (draftOf !== personaId) {
		const draft = readDraft(personaId);
		setDraftOf(personaId);
		setText(draft.text);
		setAttachments(draft.attachments);
	}
	useEffect(() => writeDraft(draftOf, { text, attachments }), [draftOf, text, attachments]);
	const [pasteFailed, setPasteFailed] = useState<string | null>(null);
	const area = useRef<HTMLTextAreaElement>(null);
	const working = isWorking(state);

	// Dictation writes into the field as the words come (voice/dictation.ts),
	// so it reads and writes the text through a ref that is never a render behind.
	// Where the person asked for it, words dictated are sent a moment after
	// they stop, unless they call it off; the send is the Send key's own.
	const textNow = useRef(text);
	textNow.current = text;
	const submitNow = useRef(() => {});
	const [countdown] = useState(() => new SendCountdown(() => submitNow.current()));
	const sendingSince = useSyncExternalStore(countdown.watch, () => countdown.counting);
	const [dictation] = useState(
		() =>
			new Dictation(dictationEngine(), {
				read: () => textNow.current,
				write: (next) => {
					textNow.current = next;
					setText(next);
				},
				done: (words) => {
					if (afterDictation() === "send") countdown.start(words);
				},
			}),
	);
	const heard = useSyncExternalStore(dictation.watch, () => dictation.view);
	// A key or the button tells a tap (start, and stop on the next) from a hold (talk while held).
	const keyPress = useRef(new TapOrHold());
	const pointerPress = useRef(new TapOrHold());
	const act = (action: "start" | "stop" | null) => {
		if (action === null) return;
		if (action === "stop") {
			void dictation.stop();
			return;
		}
		countdown.cancel();
		void dictation.start();
	};
	const dictationHere = useDictationAvailable();
	// A call has the microphone, so there is no dictating over one.
	const callLive = useCallSnapshot(useCall()).phase !== "ended";
	const canDictate = dictationHere && !callLive;
	const dictating = heard.phase !== "idle";
	const dictateKeys = hotkeyLabel(useHotkeys().dictate);

	const hasContent = text.trim().length > 0 || attachments.length > 0;
	const hasDraft = text.length > 0 || attachments.length > 0 || replyQuote !== null;
	const mic = !hasDraft && canDictate;
	const voice = !hasDraft && !dictationHere && onCall !== undefined;
	const actionShown = hasDraft || voice || mic;
	useEffect(() => { onDraftChange?.(hasContent); }, [hasContent, onDraftChange]);

	// What was heard stays in the field when a call takes the microphone,
	// the draft is put away, or the composer goes.
	useEffect(() => {
		if (callLive) dictation.release();
	}, [callLive, dictation]);
	useEffect(
		() => () => {
			dictation.release();
			countdown.cancel();
		},
		[dictation, countdown, draftOf],
	);

	// The Dictate shortcut is the conversation's, not a side thread's.
	const actNow = useRef(act);
	actNow.current = act;
	useEffect(() => {
		if (embedded || !canDictate) return;
		return takeDictationRequests((edge, at) => {
			const listening = dictation.view.phase !== "idle";
			const action = edge === "down" ? keyPress.current.down(at, listening) : keyPress.current.up(at);
			if (action === "start") area.current?.focus();
			actNow.current(action);
		});
	}, [embedded, dictation, canDictate]);

	// While a send counts down, Escape calls it off and Enter sends now,
	// wherever focus is.
	useEffect(() => {
		if (sendingSince === null) return;
		const onKey = (event: KeyboardEvent) => {
			if (event.key !== "Escape" && event.key !== "Enter") return;
			if ((event.target as Element | null)?.closest("[data-private-terminal]")) return;
			event.preventDefault();
			event.stopImmediatePropagation();
			if (event.key === "Escape") countdown.cancel();
			else countdown.sendNow();
		};
		window.addEventListener("keydown", onKey, true);
		return () => window.removeEventListener("keydown", onKey, true);
	}, [sendingSince, countdown]);

	/** The button: down starts or stops at once, and the release ends a hold, wherever the pointer is by then. */
	const pressKey = (event: ButtonPointerEvent) => {
		if (event.button !== 0) return;
		act(pointerPress.current.down(event.timeStamp, dictating));
		const release = (up: PointerEvent) => {
			window.removeEventListener("pointerup", release);
			window.removeEventListener("pointercancel", release);
			actNow.current(pointerPress.current.up(up.timeStamp));
		};
		window.addEventListener("pointerup", release);
		window.addEventListener("pointercancel", release);
	};

	// Escape gives up a dictation wherever focus is, before anything else
	// on the window hears it.
	useEffect(() => {
		if (!dictating) return;
		const onKey = (event: KeyboardEvent) => {
			if (event.key !== "Escape" || (event.target as Element | null)?.closest("[data-private-terminal]")) return;
			event.preventDefault();
			event.stopImmediatePropagation();
			dictation.cancel();
		};
		window.addEventListener("keydown", onKey, true);
		return () => window.removeEventListener("keydown", onKey, true);
	}, [dictating, dictation]);

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
		if (embedded) return;
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
	}, [embedded]);

	// Chips belong to the window, not only the field, so Escape puts them
	// down even when the field is not focused — and it does so before the
	// conversation's listener puts the quote down.
	useEffect(() => {
		if (attachments.length === 0 || dictating) return;
		const onKey = (event: KeyboardEvent) => {
			if (event.key !== "Escape" || (event.target as Element | null)?.closest("[data-private-terminal]")) return;
			event.preventDefault();
			event.stopPropagation();
			setAttachments([]);
		};
		window.addEventListener("keydown", onKey, true);
		return () => window.removeEventListener("keydown", onKey, true);
	}, [attachments.length, dictating]);

	const submit = () => {
		countdown.cancel();
		const trimmed = text.trim();
		if (!trimmed && attachments.length === 0) return;
		setText("");
		const sending = attachments;
		setAttachments([]);
		onSend(trimmed, sending);
	};
	submitNow.current = submit;

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

	// A pasted picture or file is bytes, not a path: the desk keeps a copy
	// and the chip names that.
	const paste = async (files: File[]) => {
		setPasteFailed(null);
		try {
			for (const file of files) {
				const name = pastedName(file);
				const kept = await stage(name, new Uint8Array(await file.arrayBuffer()));
				setAttachments((known) => [...known, { ...fromDroppedPath(kept.path), name, size: kept.size }]);
			}
		} catch (error) {
			setPasteFailed(`Couldn't attach what was pasted: ${error instanceof Error ? error.message : String(error)}`);
		}
	};

	const attach = async () => {
		const picked = await pickAttachments();
		if (picked.length > 0) setAttachments((known) => mergeDropped(known, picked.map((file) => file.path)));
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
					{sendingSince !== null && (
						<p className="composer-sending" role="status">
							<svg key={sendingSince} className="send-ring" viewBox="0 0 16 16" aria-hidden="true">
								<circle cx="8" cy="8" r="6" pathLength="1" style={{ animationDuration: `${SEND_AFTER_MS}ms` }} />
							</svg>
							<span className="min-w-0 flex-1 truncate">Sending to {name}…</span>
							<span className="text-ink-3">Esc to cancel</span>
						</p>
					)}
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
						placeholder={heard.phase === "listening" ? "Listening…" : heard.phase === "starting" ? "Getting ready to listen…" : "Message"}
						readOnly={heard.phase === "listening" || heard.phase === "finishing"}
						onChange={(event) => {
							setText(event.target.value);
							dictation.clearError();
							countdown.cancel();
						}}
						onPointerDown={() => countdown.cancel()}
						onPaste={(event) => {
							const files = Array.from(event.clipboardData.files);
							if (files.length > 0) {
								event.preventDefault();
								void paste(files);
								return;
							}
							// Some webviews (WebKitGTK) keep a copied picture from the page; the
							// shell can still read it when there is no text.
							if (event.clipboardData.getData("text/plain") === "") {
								void shellPicture().then((file) => file && paste([file]));
							}
						}}
						onKeyDown={(event) => {
							// While dictating, Enter stops, as a tap would.
							if (dictating && event.key === "Enter") {
								event.preventDefault();
								if (heard.phase === "listening") void dictation.stop();
								return;
							}
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
					{pasteFailed !== null && <p className="pt-1 text-xs text-danger">{pasteFailed}</p>}
					{heard.error !== null && <p className="pt-1 text-xs text-danger">{heard.error}</p>}
				</div>
				{working && (
					<button type="button" className="composer-key composer-stop" title="Interrupt (Esc)" aria-label="Interrupt" onClick={onCancel}>
						<StopIcon />
					</button>
				)}
				{dictating ? (
					<button
						type="button"
						className="composer-key composer-send composer-dictating"
						title={heard.phase === "starting" ? "Stop (Esc)" : withKeys("Stop dictating", dictateKeys)}
						aria-label={heard.phase === "starting" ? "Stop" : "Stop dictating"}
						data-shown="true"
						disabled={heard.phase === "finishing"}
						onPointerDown={pressKey}
						// A pointer acted on the way down; this is the keyboard's Enter or Space.
						onClick={(event) => {
							if (event.detail === 0) dictation.toggle();
						}}
					>
						<VoiceMeter
							source={dictation.watchLevel}
							state={heard.phase === "listening" ? "listening" : heard.phase === "finishing" ? "finishing" : "waiting"}
						/>
						<StopIcon className="composer-dictating-stop" />
					</button>
				) : (!working || actionShown) && (
					<button
						type="button"
						className={`composer-key composer-send${voice || mic ? " composer-voice" : ""}`}
						title={mic ? withKeys("Dictate", dictateKeys) : voice ? `Talk to ${name}` : "Send (Enter)"}
						aria-label={mic ? "Dictate" : voice ? `Talk to ${name}` : "Send"}
						aria-hidden={!actionShown}
						tabIndex={actionShown ? 0 : -1}
						data-shown={actionShown ? "true" : undefined}
						disabled={!voice && !mic && !hasContent}
						onPointerDown={mic ? pressKey : undefined}
						onClick={mic ? (event) => {
							if (event.detail === 0) dictation.toggle();
						} : voice ? onCall : submit}
					>
						{mic ? <MicIcon /> : voice ? <VoiceIcon /> : <ArrowUpIcon />}
					</button>
				)}
			</div>
		</div>
	);
}


/** "Dictate (⌃⌥H)", or the bare words when the shortcut is off. */
function withKeys(words: string, keys: string): string {
	return keys === "" ? words : `${words} (${keys})`;
}

/** A screenshot pastes as `image.png`; one name per paste keeps chips apart. */
function pastedName(file: File): string {
	if (file.name !== "" && !/^image\.[a-z]+$/i.test(file.name)) return file.name;
	const extension = file.type.startsWith("image/") ? file.type.slice(6).replace("jpeg", "jpg").replace("svg+xml", "svg") : "png";
	const now = new Date();
	const time = [now.getHours(), now.getMinutes(), now.getSeconds()].map((part) => String(part).padStart(2, "0")).join(".");
	return `Pasted image ${time}.${extension}`;
}

/** The picture on the system clipboard as a PNG, through the shell; null when there is none. */
async function shellPicture(): Promise<File | null> {
	try {
		const image = await readImage();
		const [{ width, height }, rgba] = await Promise.all([image.size(), image.rgba()]);
		const canvas = document.createElement("canvas");
		canvas.width = width;
		canvas.height = height;
		canvas.getContext("2d")?.putImageData(new ImageData(new Uint8ClampedArray(rgba), width, height), 0, 0);
		const png = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
		return png && new File([png], "image.png", { type: "image/png" });
	} catch {
		// No picture there, or no shell: a browser tab.
		return null;
	}
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
