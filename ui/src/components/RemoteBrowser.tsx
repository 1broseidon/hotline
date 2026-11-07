import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { activeDeskId, wireFor } from "../desks";
import type { BrowserSnapshot, BrowserStep, BrowserView } from "../generated/contract";
import { CloseIcon } from "../icons";
import { isDesktop } from "../native";
import { InputQueue, POLL_MS, isNewer, keyInput, lockKind, pagePoint, scrollInput, textInputs } from "../remoteBrowser";
import { ScreenPainter, driveFrom } from "../screenSocket";
import type { RosterEntry, Wire } from "../wire";

/**
 * A teammate's desk browser that runs on another desk (a server, or another
 * computer), in this window. On its own desk the PIP opens a native pane
 * with the bar above the page (browser.html); here there is no page to put
 * under a bar, so the viewer shows the desk's pictures of it and carries the
 * person's clicks, wheel and keys back over the wire, as the phone does.
 *
 * A desk whose hello says `browserStream` (a server whose helper streams its
 * pages, task-390) is watched and driven live instead, as a teammate's
 * computer is: the page is a screen in the computer viewer's protocol
 * (screenSocket.ts) through the desk's bridge, only what changed is sent,
 * and the person's pointer, wheel and keys go up as they happen, hover and
 * drag included, without waiting on a picture. Take control and Give it back
 * go on that socket, which is the browser's own take-over on the desk; the
 * address, ‹ › ↻ and Visit anyway stay wire commands.
 *
 * It wears the pane's bar (browserBar.css) so the two read as one thing, and
 * offers only what this seat may do: a desk whose hello does not grant
 * `browserDrive` is watched, never driven, and shows no control that could
 * not work.
 *
 * The PIP and a cookie ask's Open Browser open it with `openRemoteBrowser`;
 * the window mounts one `RemoteBrowser`, which shows it over everything.
 */

type Opened = { personaId: string; name: string };
let opened: Opened | null = null;
const listeners = new Set<() => void>();

function setOpened(next: Opened | null) {
	opened = next;
	for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

/** Shows a teammate's browser on the active desk in the window's viewer. */
export function openRemoteBrowser(personaId: string, name: string): void {
	setOpened({ personaId, name });
}

/** The viewer, over the whole window, while one is open. Mounted once, by the window. */
export function RemoteBrowser({ roster }: { roster: RosterEntry[] }) {
	const open = useSyncExternalStore(subscribe, () => opened);
	// The window is mounted afresh for each desk, so a viewer opened on one
	// desk never stays up over another's teammates.
	useEffect(() => () => setOpened(null), []);
	const deskId = activeDeskId();
	if (open === null || deskId === null) return null;
	const entry = roster.find((one) => one.persona.id === open.personaId);
	return (
		<Viewer
			key={open.personaId}
			deskId={deskId}
			line={wireFor(deskId)}
			personaId={open.personaId}
			name={entry?.persona.name ?? open.name}
			view={entry?.session.browser}
		/>
	);
}

/** What the screen said last of who has the browser, from this socket's side. */
type ScreenState = { holder: "none" | "agent" | "person" | "you"; driving: boolean };

/**
 * The teammate's page as a live screen through the desk's bridge, painted on
 * `canvas`: a token that opens it once is asked of the shell at every
 * connect, and a dropped socket is opened again, taking control again if it
 * had it. Answers what it last said and how to speak on it.
 */
function useScreen(deskId: string, personaId: string, live: boolean, canvas: React.RefObject<HTMLCanvasElement | null>, say: (reason: unknown) => void) {
	const [state, setState] = useState<ScreenState>({ holder: "none", driving: false });
	const [size, setSize] = useState<{ width: number; height: number } | null>(null);
	const socket = useRef<WebSocket | null>(null);
	const wanted = useRef(false);
	const send = useCallback((message: Record<string, unknown>) => {
		if (message["t"] === "control") wanted.current = message["take"] === true;
		if (socket.current?.readyState === WebSocket.OPEN) socket.current.send(JSON.stringify(message));
	}, []);
	useEffect(() => {
		const drawn = canvas.current;
		if (!live || drawn === null) return;
		let gone = false;
		let retry: ReturnType<typeof setTimeout> | undefined;
		const painter = new ScreenPainter(drawn, () => setSize({ width: drawn.width, height: drawn.height }));
		const connect = () => {
			invoke<{ origin: string; token: string }>("desk_viewer_token", { deskId, personaId, kind: "browser" }).then(
				({ origin, token }) => {
					if (gone) return;
					const opened = new WebSocket(`${origin.replace(/^http/, "ws")}/browser/${encodeURIComponent(personaId)}/ws`, [`hotline-viewer.${token}`]);
					opened.binaryType = "arraybuffer";
					socket.current = opened;
					opened.onopen = () => {
						if (wanted.current) opened.send(JSON.stringify({ t: "control", take: true }));
					};
					opened.onmessage = (event) => {
						if (typeof event.data !== "string") {
							painter.draw(event.data as ArrayBuffer);
							return;
						}
						const message = JSON.parse(event.data);
						if (message.t === "state") setState({ holder: message.holder, driving: message.driving === true });
						if (message.t === "error") {
							say(message.error);
							if (!message.driving) setState((was) => ({ ...was, driving: false }));
						}
					};
					opened.onclose = () => {
						if (socket.current === opened) socket.current = null;
						setState((was) => ({ ...was, driving: false }));
						if (!gone) retry = setTimeout(connect, 1000);
					};
				},
				() => {
					// The desk's bridge is not up yet; the shell retries it, and so do we.
					if (!gone) retry = setTimeout(connect, 1000);
				},
			);
		};
		connect();
		return () => {
			gone = true;
			clearTimeout(retry);
			socket.current?.close();
			socket.current = null;
		};
	}, [deskId, personaId, live, canvas, say]);
	return { state, size, send };
}

/** Whether the desk's hello lets this seat do `name`, asked again as the connection moves. */
function useCan(line: Wire, name: string): boolean {
	const [can, setCan] = useState(() => line.can(name));
	useEffect(() => line.onConnection(() => setCan(line.can(name))), [line, name]);
	return can;
}

function Viewer({ deskId, line, personaId, name, view }: { deskId: string; line: Wire; personaId: string; name: string; view: BrowserView | undefined }) {
	const drive = useCan(line, "browserDrive");
	const visitAnyway = useCan(line, "browserVisitAnyway");
	// A live screen needs the shell's bridge; anywhere else it is pictures.
	const live = useCan(line, "browserStream") && isDesktop();
	const page = useRef<HTMLDivElement>(null);
	const canvas = useRef<HTMLCanvasElement>(null);
	const field = useRef<HTMLInputElement>(null);

	/* Take control and Give it back answer the browser as it is then. That
	 * answer stands until the teammate's session says anything newer, so the
	 * bar moves on the press rather than on the roster's next word. */
	const [answered, setAnswered] = useState<{ over: BrowserView | undefined; view: BrowserView } | null>(null);
	const current = answered !== null && answered.over === view ? answered.view : view;
	const held = current?.phase === "person";

	const [snapshot, setSnapshot] = useState<BrowserSnapshot>();
	const show = useCallback((next: BrowserSnapshot | null | undefined) => setSnapshot((shown) => (isNewer(shown, next) ? next : shown)), []);

	// A refusal stays on the bar for five seconds; a newer one keeps its own five.
	const [error, setError] = useState<{ text: string } | null>(null);
	useEffect(() => {
		if (error === null) return;
		const timer = setTimeout(() => setError(null), 5_000);
		return () => clearTimeout(timer);
	}, [error]);
	const say = useCallback((reason: unknown) => setError({ text: reason instanceof Error ? reason.message : String(reason) }), []);

	const screen = useScreen(deskId, personaId, live, canvas, say);
	// Live, this seat drives once the screen says it took control; by
	// pictures, whenever the person has the browser.
	const driving = drive && (live ? screen.state.driving && held : held);
	const drivingNow = useRef(driving);
	drivingNow.current = driving;
	useEffect(() => {
		const drawn = canvas.current;
		if (!live || drawn === null) return;
		const hands = driveFrom(drawn, { send: screen.send, driving: () => drivingNow.current });
		return () => {
			hands.letGo();
			hands.detach();
		};
	}, [live, screen.send]);

	// While the teammate drives, the picture is the one its last move left: fetched again whenever a newer one is taken.
	const taken = current?.snapshotAt;
	useEffect(() => {
		if (live || held || taken === undefined) return;
		let gone = false;
		line.command("browser.snapshot", { personaId }).then(
			(next) => {
				if (!gone) show(next);
			},
			() => {},
		);
		return () => {
			gone = true;
		};
	}, [line, personaId, live, held, taken, show]);

	// While the person has it, the page is pictured afresh a few times a second, one ask at a time.
	useEffect(() => {
		if (live || !held) return;
		let gone = false;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const poll = async () => {
			const started = Date.now();
			try {
				const next = await line.command("browser.snapshot", { personaId, fresh: true });
				if (!gone) show(next);
			} catch {
				// A missed picture is followed by the next one.
			}
			if (!gone) timer = setTimeout(poll, Math.max(0, started + POLL_MS - Date.now()));
		};
		void poll();
		return () => {
			gone = true;
			clearTimeout(timer);
		};
	}, [line, personaId, live, held, show]);

	const queue = useMemo(() => new InputQueue((input) => line.command("browser.input", { personaId, input }), show, say), [line, personaId, show, say]);
	useEffect(() => {
		if (!driving) queue.clear();
	}, [driving, queue]);

	const close = useCallback(() => {
		// Closing never keeps the browser from the teammate: control held here goes back with it.
		if (driving) line.command("browser.hand_back", { personaId }).catch(() => {});
		setOpened(null);
	}, [driving, line, personaId]);

	/* Escape closes, except where it means something else: on the picture
	 * while the person drives it is the page's key, and in an address being
	 * typed it puts the address back. The close button is always the way out. */
	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			if ((event.target === page.current || event.target === canvas.current) && driving) return;
			if (event.target === field.current && driving) return;
			event.stopPropagation();
			event.preventDefault();
			close();
		};
		window.addEventListener("keydown", onKey, true);
		return () => window.removeEventListener("keydown", onKey, true);
	}, [close, driving]);

	useEffect(() => (live ? canvas.current : page.current)?.focus(), [live]);

	const [busy, setBusy] = useState(false);
	const toggle = () => {
		if (live) {
			// The screen's own control: the desk takes the browser over for
			// this seat, or hands it back, and says so on the socket.
			screen.send({ t: "control", take: !driving });
			if (!driving) canvas.current?.focus();
			return;
		}
		setBusy(true);
		const over = view;
		line
			.command(held ? "browser.hand_back" : "browser.take_over", { personaId })
			.then((next) => {
				if (next === null) return;
				setAnswered({ over, view: next });
				if (next.phase === "person") page.current?.focus();
			}, say)
			.finally(() => setBusy(false));
	};
	const run = (move: () => Promise<BrowserSnapshot | null>) => {
		if (driving) move().then(show, say);
	};
	const step = (which: BrowserStep) => run(() => line.command("browser.step", { personaId, step: which }));

	const [typing, setTyping] = useState<string | null>(null);
	const url = (held ? snapshot?.url : undefined) ?? current?.url ?? snapshot?.url ?? "";
	const shape = (live ? screen.size : null) ?? snapshot?.size ?? current?.viewport ?? { width: 16, height: 10 };
	const picture = snapshot === undefined ? undefined : `data:image/jpeg;base64,${snapshot.jpeg}`;
	/** Where a pointer on the picture is on the page, through the page's size (or, when the desk could not say it, the picture's own). */
	const pointOf = (event: { clientX: number; clientY: number }) => {
		const box = page.current;
		const image = box?.querySelector("img");
		if (!box || !image) return null;
		const size = snapshot?.size ?? { width: image.naturalWidth, height: image.naturalHeight };
		return pagePoint(box.getBoundingClientRect(), size, event.clientX, event.clientY);
	};

	return createPortal(
		<div className="viewer remote-browser" role="dialog" aria-modal="true" aria-label={`${name}'s browser`}>
			<div className="remote-browser-frame">
				<div className={`browser-bar${held ? " yours" : ""}`}>
					<span className="who">
						<span className="dot" />
						<span>{current === undefined ? `${name} has no page open` : held ? "You have control" : `${name} is driving`}</span>
					</span>
					{drive && (
						<>
							<button type="button" className="icon" title="Back" aria-label="Back" disabled={!driving} onClick={() => step("back")}>
								‹
							</button>
							<button type="button" className="icon" title="Forward" aria-label="Forward" disabled={!driving} onClick={() => step("forward")}>
								›
							</button>
							<button type="button" className="icon" title="Reload" aria-label="Reload" disabled={!driving} onClick={() => step("reload")}>
								↻
							</button>
						</>
					)}
					<form
						className="where"
						onSubmit={(event) => {
							event.preventDefault();
							if (!driving || typing === null) return;
							const address = typing.trim();
							setTyping(null);
							field.current?.blur();
							if (address !== "") run(() => line.command("browser.go", { personaId, url: address }));
						}}
					>
						<span className={`lock ${lockKind(url)}`} aria-hidden="true" />
						<input
							ref={field}
							className="url"
							aria-label="Address"
							spellCheck={false}
							autoComplete="off"
							readOnly={!driving}
							value={typing ?? url}
							onFocus={(event) => {
								if (!driving) return;
								setTyping(url);
								event.currentTarget.select();
							}}
							onBlur={() => setTyping(null)}
							onChange={(event) => setTyping(event.target.value)}
							onKeyDown={(event) => {
								if (event.key !== "Escape") return;
								event.stopPropagation();
								setTyping(null);
								event.currentTarget.blur();
							}}
						/>
					</form>
					{error !== null && (
						<span className="error" role="status">
							{error.text}
						</span>
					)}
					{current?.needs && (
						<span className="untrusted">
							<span>{current.needs}</span>
							{drive && visitAnyway && (
								<button
									type="button"
									className="visit"
									disabled={!held}
									title={held ? undefined : "Take control first"}
									onClick={() => run(() => line.command("browser.visit_anyway", { personaId }))}
								>
									Visit anyway
								</button>
							)}
						</span>
					)}
					{drive && current !== undefined && (
						<button type="button" className="primary" disabled={busy} onClick={toggle}>
							{(live ? driving : held) ? "Give it back" : "Take control"}
						</button>
					)}
					<button type="button" className="icon" title="Close" aria-label="Close" onClick={close}>
						<CloseIcon />
					</button>
				</div>
				<div className="remote-browser-stage">
					{live && (
						<canvas
							ref={canvas}
							className={`remote-browser-page live${driving ? " yours" : ""}`}
							aria-label={`${name}'s page`}
							tabIndex={0}
							width={1}
							height={1}
							style={{ aspectRatio: `${shape.width} / ${shape.height}`, width: `min(100cqw, 100cqh * ${shape.width} / ${shape.height})` }}
							onPaste={(event) => {
								if (!driving) return;
								const text = event.clipboardData.getData("text/plain");
								if (text === "") return;
								event.preventDefault();
								screen.send({ t: "paste", text });
							}}
						/>
					)}
					{/* The page itself, as far as the keyboard and the pointer go: while the person drives, what lands here goes to the teammate's page. */}
					{!live && (<div
						ref={page}
						className={`remote-browser-page${driving ? " yours" : ""}`}
						role="application"
						aria-label={`${name}'s page`}
						// biome-ignore lint/a11y/noNoninteractiveTabindex: an application is the page another desk draws; it takes the focus so typing reaches that page.
						tabIndex={0}
						style={{ aspectRatio: `${shape.width} / ${shape.height}`, width: `min(100cqw, 100cqh * ${shape.width} / ${shape.height})` }}
						onClick={(event) => {
							if (!driving) return;
							const at = pointOf(event);
							if (at !== null) queue.push({ kind: "tap", ...at });
						}}
						onWheel={(event) => {
							if (!driving) return;
							// A wheel that counts lines or pages is turned into pixels, as a page scrolls by them.
							const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? shape.height : 1;
							const input = scrollInput(event.deltaX * unit, event.deltaY * unit, pointOf(event));
							if (input.kind === "scroll" && (input.dx !== 0 || input.dy !== 0)) queue.push(input);
						}}
						onKeyDown={(event) => {
							if (!driving) return;
							const input = keyInput(event);
							if (input === null) return;
							event.preventDefault();
							event.stopPropagation();
							queue.push(input);
						}}
						onPaste={(event) => {
							if (!driving) return;
							const text = event.clipboardData.getData("text/plain");
							if (text === "") return;
							event.preventDefault();
							for (const piece of textInputs(text)) queue.push(piece);
						}}
					>
						{picture !== undefined && <img src={picture} alt={snapshot?.title ?? ""} draggable={false} />}
					</div>)}
				</div>
			</div>
		</div>,
		document.body,
	);
}
