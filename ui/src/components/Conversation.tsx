import { type ReactNode, useCallback, useEffect, useMemo, useState } from "react";
import type { Attachment, ConfigChoice, RunningSubagent, ScheduledJob, ThreadId, TranscriptEvent } from "../generated/contract";
import { shownState } from "../activity";
import { sideTitle } from "../links";
import { chordGlyph, chordKeys } from "../chords";
import { openComputer, useComputerViewer } from "../computer";
import { ClockIcon, ComputerIcon, HangUpIcon, MoreIcon, PhoneIcon, ProgressRing, WarningIcon } from "../icons";
import { carry, onServer, showPath } from "../serverFiles";
import { nextText } from "../room";
import { dmOf, sameThread, useThread } from "../tape";
import { usePeople } from "../avatars";
import { Avatar } from "../ui/Avatar";
import { Band } from "../ui/Band";
import { MenuButton, type MenuEntry } from "../ui/Menu";
import { useNarrow } from "../narrow";
import { wire, type RosterEntry } from "../wire";
import { Composer, isDown } from "./Composer";
import { SessionPickers } from "./Pickers";
import { Search } from "./Search";
import { Starters, untouched } from "./Starters";
import { reactionQuote, Transcript, type ReactTarget, type ReplyTarget, type ThreadRef } from "./Transcript";

/**
 * One teammate's conversation: the band naming them, with their model and
 * effort, the transcript, the composer, and the search over it. The name
 * opens their pane beside it. A picker's refusal comes back up as `said`
 * for the band to say. Keyed by teammate above, so switching
 * tears the tape subscription down and puts up another rather than folding
 * two conversations into one column.
 *
 * A teammate is always there. Whether a session is up behind them is
 * plumbing the band never reports: the one state it shows is the beat
 * while they work, and a message to a resting teammate starts the session
 * on its way. Stopping is the one deliberate act, under More.
 */
/** A message on screen before the core has written it down. */
type Saying = { text: string; at: number; replyTo?: string };

/** The words and files a refused send hands back to the composer. */
export type Refill = { text: string; attachments: Attachment[]; nonce: number };

export function Conversation({
	entry,
	roster,
	jobs,
	said,
	searchOpen,
	inspectorOpen,
	focus,
	onToggleInspector,
	onOpenSchedules,
	onCloseSearch,
	onDelete,
	onPick,
	onOpenThread,
	onOpenThreadList,
	onOpenWork,
	workOpen,
	threadOpen,
	paneOpen = false,
	dock,
	models,
	onSaid,
	onCall,
	onHangUp,
}: {
	entry: RosterEntry;
	/** The room's models, for the model picker in the band. */
	models: ConfigChoice[];
	/** Where the band's model or effort picker hands a refusal. */
	onSaid(said: string | null): void;
	/** Rings this teammate, where the desk can put a call through to them. */
	onCall?: (() => void) | undefined;
	/** Ends the call, while one with this teammate is live. */
	onHangUp?: (() => void) | undefined;
	roster: RosterEntry[];
	jobs: ScheduledJob[];
	/** What the band's model or effort picker was refused with, or nothing. */
	said: string | null;
	searchOpen: boolean;
	inspectorOpen: boolean;
	focus: { eventId: string; at: number } | null;
	onToggleInspector(): void;
	onOpenSchedules(): void;
	onCloseSearch(): void;
	onDelete(): void;
	onPick(personaId: string, eventId: string): void;
	/** Opens a thread, of any kind, in the right-hand pane. */
	onOpenThread(thread: ThreadRef): void;
	/** Opens the right-hand pane on this teammate's list of threads. */
	onOpenThreadList(): void;
	/** Opens a turn's work beside the conversation; see Transcript's `onOpenWork`. */
	onOpenWork(blockId: string | null): void;
	/** Which turn's work is open beside it, if any. */
	workOpen: string | null | undefined;
	/** Which thread is open in the right-hand pane, if any. */
	threadOpen: ThreadId | undefined;
	/** The threads pane is beside the conversation, listing what the band's side-thread chips would. */
	paneOpen?: boolean;
	/** The work card, docked under the composer when the window is too narrow for it to float. */
	dock?: ReactNode;
}) {
	const { persona, session } = entry;
	/* A turn open only for its subagents reads as done; see `shownState`. */
	const state = shownState(session);
	const people = usePeople(roster);
	const opened = (thread: ThreadId) => threadOpen !== undefined && sameThread(threadOpen, thread);
	const subagents = entry.subagents ?? [];
	const sides = entry.sides ?? [];
	const personaId = persona.id;
	const { events, streaming, loaded, pulling, more: olderOnDesk, earlier } = useThread(dmOf(personaId));
	const [replying, setReplying] = useState<ReplyTarget | null>(null);
	/* What was said, from the moment it was said. The core writes the line
	 * only once a session is up, and starting one is a second or two in which
	 * a composer that has already emptied looks like it did nothing. */
	const [saying, setSaying] = useState<Saying | null>(null);
	const [refill, setRefill] = useState<Refill | null>(null);
	const [draftHasContent, setDraftHasContent] = useState(false);
	/* A refusal the core handed up: a chapter that would not open, a message
	 * that would not send. The band is the one place with room for a sentence. */
	const [refused, setRefused] = useState<string | null>(null);
	const [chapterBusy, setChapterBusy] = useState(false);

	/* A side thread opens empty and at once, beside this one; nothing about
	 * it touches the main conversation's line or session. Its first line,
	 * said in it, names it. */
	const startSide = useCallback(() => {
		setRefused(null);
		void wire.command("thread.open", { personaId, text: "" }).then(
			(summary) => onOpenThread({ thread: summary.thread, ...(summary.title !== undefined ? { title: summary.title } : {}) }),
			(error: unknown) => setRefused(error instanceof Error ? error.message : String(error)),
		);
	}, [personaId, onOpenThread]);
	const send = useCallback(
		(text: string, attachments: Attachment[]) => {
			const answered = replying;
			const at = Date.now();
			setSaying({ text, at, ...(answered ? { replyTo: answered.eventId } : {}) });
			setReplying(null);
			// A teammate that is not running is started on the way, and the
			// words wait for it: a start that is refused is the reason shown,
			// where a send racing it could only say the teammate is not running.
			const started = isDown(session.state)
				? wire.command("session.start", { personaId }).then(() => setRefused(null))
				: Promise.resolve();
			// On a desk on a server, what is attached goes up first.
			void started
				.then(() => carry(attachments))
				.then((carried) =>
					wire.command("session.prompt", {
						personaId,
						text,
						...(answered ? { replyTo: answered.eventId } : {}),
						...(carried.length > 0 ? { attachments: carried } : {}),
					}),
				)
				.then(
					() => setSaying(null),
					(error: unknown) => {
						// Nothing was said after all: the words go back where
						// they came from, with what was attached to them.
						setSaying(null);
						setRefill({ text, attachments, nonce: at });
						if (answered) setReplying(answered);
						setRefused(error instanceof Error ? error.message : String(error));
					},
				);
		},
		[personaId, replying, session.state],
	);
	/* The line the core will write, standing in until it does: the same words
	 * at or after the moment they were sent. */
	const shown = useMemo(() => {
		if (!saying) return events;
		const landed = events.some(
			(event) => event.kind === "user" && event.ts >= saying.at - 1000 && event.text === saying.text,
		);
		if (landed) return events;
		return [
			...events,
			{
				kind: "user" as const,
				id: `saying:${saying.at}`,
				ts: saying.at,
				text: saying.text,
				...(saying.replyTo !== undefined ? { replyTo: saying.replyTo } : {}),
			},
		];
	}, [events, saying]);
	/* A reaction is a line of its own, the shape the phone sends: the quoted
	 * line and the emoji, answering it. The teammate reads it as a reply; both
	 * windows fold it onto the bubble. */
	const react = useCallback(
		(target: ReactTarget, emoji: string) => {
			const started = isDown(session.state)
				? wire.command("session.start", { personaId }).then(() => setRefused(null))
				: Promise.resolve();
			void started
				.then(() =>
					wire.command("session.prompt", { personaId, text: `${reactionQuote(target.text)}\n\n${emoji}`, replyTo: target.eventId }),
				)
				.catch((error: unknown) => setRefused(error instanceof Error ? error.message : String(error)));
		},
		[personaId, session.state],
	);
	const stop = useCallback(() => void wire.command("session.stop", { personaId }), [personaId]);
	const cancel = useCallback(() => void wire.command("session.cancel", { personaId }), [personaId]);
	/* Success is the tape: the marker is superseded in place and the title
	 * lands on its line. Only a refusal needs a sentence here. */
	const startChapter = useCallback(() => {
		setRefused(null);
		setChapterBusy(true);
		void wire
			.command("chapter.start_fresh", { personaId })
			.catch((error: Error) => setRefused(error.message))
			.finally(() => setChapterBusy(false));
	}, [personaId]);
	const resumeChapter = useCallback(() => {
		setRefused(null);
		setChapterBusy(true);
		void wire
			.command("chapter.resume", { personaId })
			.catch((error: Error) => setRefused(error.message))
			.finally(() => setChapterBusy(false));
	}, [personaId]);
	const resumeBlocked = resumeRefusal(events, persona.backendId);
	/* The first conversation, before a word: what the starter card reads.
	 * A line on its way (`saying`) already ends it. */
	const fresh = loaded && saying === null && untouched(events, olderOnDesk);

	// Escape clears a quote that is up even when the field is not focused.
	// Chips are put down first, on the window in capture, so this listener
	// does not also drop the quote on the same press.
	useEffect(() => {
		if (replying === null) return;
		const onKey = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			setReplying(null);
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [replying]);

	const running = session.state === "ready" || session.state === "thinking" || session.state === "starting";
	const screen = useComputerViewer(personaId, persona.computer?.enabled ?? false);
	const openScreen = useMemo(
		() => (screen === undefined ? undefined : () => void openComputer(personaId, persona.name, screen)),
		[screen, personaId, persona.name],
	);
	// Stable across renders, so the transcript's rows can skip a render while a reply streams.
	const retryMessage = useCallback(
		(message: Extract<TranscriptEvent, { kind: "user" }>) => {
			setRefill({ text: message.text, attachments: message.attachments ?? [], nonce: Date.now() });
			const original = events.find((event) => event.id === message.replyTo);
			setReplying(message.replyTo ? { eventId: message.replyTo, text: original && "text" in original ? original.text : "Earlier message" } : null);
		},
		[events],
	);
	const next = jobs.filter((job) => job.operatorCreated || persona.backgroundWork === true).reduce<ScheduledJob | null>(
		(soonest, job) => (soonest === null || job.nextAt < soonest.nextAt ? job : soonest),
		null,
	);
	const scheduleDetail = next === null ? "Paused" : `Next ${nextText(next.nextAt)}`;

	/* A narrow band keeps the name and its keys; the schedule line folds into
	 * the More menu, one press further away, rather than a band that clips it. */
	const narrow = useNarrow();
	const folded: MenuEntry[] = [];
	if (narrow && jobs.length > 0) {
		folded.push({
			kind: "item",
			id: "schedules",
			text: jobs.length === 1 ? "1 scheduled" : `${jobs.length} scheduled`,
			detail: scheduleDetail,
			onSelect: onOpenSchedules,
		});
	}

	const more: MenuEntry[] = [
		...folded,
		{ kind: "item", id: "chapter", text: "Start a new chapter", detail: "Closes this one with a handoff note", disabled: chapterBusy, onSelect: startChapter },
		{
			kind: "item",
			id: "resume",
			text: "Reopen previous chapter",
			detail: resumeBlocked ?? "The chapter immediately before this one",
			disabled: chapterBusy || resumeBlocked !== null,
			onSelect: resumeChapter,
		},
		{
			kind: "item",
			id: "side",
			text: "Start a side thread",
			detail: `Another topic with ${persona.name}, in parallel`,
			onSelect: startSide,
		},
		{ kind: "item", id: "side-list", text: "Threads", detail: "Work threads, conversations between teammates, runs and calls", onSelect: onOpenThreadList },
		{ kind: "item", id: "reveal", text: onServer() ? "Show working directory on the server" : "Reveal working directory", onSelect: () => showPath(persona.cwd) },
		{ kind: "rule" },
		{ kind: "item", id: "teammate", text: inspectorOpen ? "Hide teammate" : "Show teammate", shortcut: chordGlyph("teammate"), onSelect: onToggleInspector },
		...(running ? [{ kind: "item", id: "stop", text: "Stop the session", onSelect: stop } as MenuEntry] : []),
		{ kind: "rule" },
		{ kind: "item", id: "delete", text: "Remove teammate…", danger: true, onSelect: onDelete },
	];

	/* The band says what just happened: a session error, or a refusal of
	 * something that was asked for. Why the previous chapter cannot be
	 * reopened is a standing fact rather than an event, so it stays on the
	 * greyed menu item, read at the moment somebody goes looking for it. */
	/* The band says who this is and nothing more: the goal is on the
	 * teammate's pane, and while a turn runs the pulse beside the name says
	 * so on its own. */

	const notice = session.error !== undefined && session.error !== "" ? session.error : (said ?? refused);

	return (
		<section className="conversation pane" aria-label={`Conversation with ${persona.name}`}>
			<Band>
				<button
					type="button"
					className="control btn-quiet -ml-1 min-w-0 shrink gap-2 pl-1 pr-2"
					// The name is the one way into the teammate's pane, the way a
					// messages app opens a contact from its header.
					title={`Teammate (${chordKeys("teammate")})`}
					aria-label={state === "thinking" ? `${persona.name}, working` : persona.name}
					aria-expanded={inspectorOpen}
					onClick={onToggleInspector}
				>
					<Avatar id={persona.id} name={persona.name} size={20} hash={persona.avatar?.hash} />
					<span className="shrink-0 text-lg font-semibold text-ink">{persona.name}</span>
					{state === "thinking" && (
						<span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
					)}
				</button>

				<span className="min-w-0 flex-1" />

				<SessionPickers key={persona.id} entry={entry} models={models} onSaid={onSaid} />

				{/* Side threads still live: a chip each. They are conversations of
				 * their own rather than this turn's work, so they stay in the band
				 * when the subagents sit at the composer. They leave the band when
				 * archived; the conversation's own line keeps what came of each.
				 * With the pane open they are in its list, and the band has no
				 * room to name them. */}
				{paneOpen ? null : sides.length === 1 ? (
					<button
						type="button"
						className="control btn-quiet min-w-0 shrink gap-1.5 px-2 text-sm"
						title="Open the side thread"
						aria-label={`Side thread: ${sideTitle(sides[0]!.title)}`}
						aria-pressed={opened({ kind: "side", key: sides[0]!.sideId })}
						onClick={() => onOpenThread({ thread: { kind: "side", key: sides[0]!.sideId }, title: sides[0]!.title })}
					>
						<span className="truncate">{narrow ? "Side" : `Side: ${sideTitle(sides[0]!.title)}`}</span>
						{sides[0]!.working && <span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />}
					</button>
				) : sides.length > 1 ? (
					<MenuButton
						className="control btn-quiet shrink-0 gap-1.5 px-2 text-sm"
						label={`${sides.length} side threads`}
						entries={sides.map((side) => ({
							kind: "item",
							id: side.sideId,
							text: sideTitle(side.title),
							checked: opened({ kind: "side", key: side.sideId }),
							onSelect: () => onOpenThread({ thread: { kind: "side", key: side.sideId }, title: side.title }),
						}))}
					>
						{`${sides.length} side threads`}
						{sides.some((side) => side.working) && <span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />}
					</MenuButton>
				) : null}

				{jobs.length > 0 && !narrow && (
					<button
						type="button"
						className="control btn-quiet gap-1.5 px-2 text-sm"
						title="Schedules"
						aria-label={`${jobs.length} scheduled, ${scheduleDetail}`}
						onClick={onOpenSchedules}
					>
						<ClockIcon className="text-ink-3" />
						{jobs.length === 1 ? "1 scheduled" : `${jobs.length} scheduled`}
						<span className="text-ink-3">{scheduleDetail}</span>
					</button>
				)}

				{/* A call is its own key, beside the teammate's other ways in: the
				 * composer's voice is dictation where this Mac can hear. */}
				{onHangUp !== undefined ? (
					<button type="button" className="control btn-icon" title={`End the call with ${persona.name}`} aria-label="End the call" onClick={onHangUp}>
						<HangUpIcon />
					</button>
				) : onCall !== undefined && (
					<button type="button" className="control btn-icon" title={`Call ${persona.name}`} aria-label={`Call ${persona.name}`} onClick={onCall}>
						<PhoneIcon />
					</button>
				)}

				{/* A computer on its way fills in where its button will be; the
				 * conversation carries on around it. */}
				{pulling !== null ? (
					<span
						role="progressbar"
						className="control btn-icon text-ink-3"
						title={pulling.total > 0 ? `Setting up the computer · ${Math.round((pulling.done / pulling.total) * 100)}%` : "Setting up the computer"}
						aria-label="Setting up the teammate's computer"
						aria-valuemin={0}
						aria-valuemax={100}
						{...(pulling.total > 0 ? { "aria-valuenow": Math.round((pulling.done / pulling.total) * 100) } : {})}
					>
						<ProgressRing value={pulling.total > 0 ? pulling.done / pulling.total : null} />
					</span>
				) : openScreen !== undefined && (
					<button
						type="button"
						className="control btn-icon"
						title="Open the teammate's computer"
						aria-label="Open the teammate's computer"
						onClick={openScreen}
					>
						<ComputerIcon />
					</button>
				)}
				<MenuButton className="control btn-icon" label="More" entries={more}>
					<MoreIcon />
				</MenuButton>
			</Band>

			{notice !== null && (
				<p
					role="status"
					className="flex shrink-0 items-center gap-2 bg-danger-soft px-4 py-1.5 text-sm text-ink"
				>
					<WarningIcon className="shrink-0 text-danger" />
					<span className="min-w-0 flex-1 selectable">{notice}</span>
				</p>
			)}

			<div className="relative flex min-h-0 flex-1 flex-col">
				<Transcript
					personaId={personaId}
					name={persona.name}
					avatarHash={persona.avatar?.hash}
					people={people}
					events={shown}
					streaming={streaming}
					live={state === "thinking"}
					cornered={subagents.length > 0}
					focus={focus}
					onReply={setReplying}
					onReact={react}
					more={olderOnDesk}
					onEarlier={earlier}
					{...(!draftHasContent ? { onRetryMessage: retryMessage } : {})}
					{...(openScreen !== undefined ? { onOpenScreen: openScreen } : {})}
					onOpenThread={onOpenThread}
					onOpenThreads={onOpenThreadList}
					onOpenWork={onOpenWork}
					workOpen={workOpen}
				/>
				{fresh && (
					<div className="relative shrink-0 px-6">
						<Starters persona={persona} onPick={(text) => setRefill({ text, attachments: [], nonce: Date.now() })} />
					</div>
				)}
				<Composer
					onDraftChange={setDraftHasContent}
					personaId={personaId}
					name={persona.name}
					state={state}
					replyQuote={replying?.text ?? null}
					onSend={send}
					onCall={onCall}
					{...(refill !== null ? { refill } : {})}
					onCancel={cancel}
					onClearReply={() => setReplying(null)}
					{...(subagents.length > 0
						? { corner: <SubagentChips subagents={subagents} opened={(runId) => opened({ kind: "run", key: runId })} onOpenThread={onOpenThread} /> }
						: {})}
				/>
				{dock !== undefined && <div className="work-dock">{dock}</div>}
				{searchOpen && (
					<Search personaId={personaId} roster={roster} onClose={onCloseSearch} onPick={onPick} />
				)}
			</div>
		</section>
	);
}

/**
 * Subagents still running, wherever their lines have scrolled to, at the
 * composer's top right: what the teammate left going is beside where you
 * would speak to it. One is named, several are counted with a menu, and each
 * opens its run in the pane. They go when they finish; their lines in the
 * conversation keep how each went.
 */
export function SubagentChips({
	subagents,
	opened,
	onOpenThread,
}: {
	subagents: RunningSubagent[];
	/** Whether a run is the one open in the pane. */
	opened(runId: string): boolean;
	onOpenThread(thread: ThreadRef): void;
}) {
	const open = (run: RunningSubagent) => onOpenThread({ thread: { kind: "run", key: run.runId }, title: run.title });
	if (subagents.length === 1) {
		const run = subagents[0]!;
		return (
			<button
				type="button"
				className="control btn btn-sm min-w-0 shrink gap-1.5"
				title="Open the subagent's run"
				aria-label={`Subagent working: ${run.title}`}
				aria-pressed={opened(run.runId)}
				onClick={() => open(run)}
			>
				<span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
				<span className="truncate">{`Subagent · ${run.title}`}</span>
			</button>
		);
	}
	return (
		<MenuButton
			className="control btn btn-sm shrink-0 gap-1.5"
			label={`${subagents.length} subagents working`}
			entries={subagents.map((run) => ({
				kind: "item",
				id: run.runId,
				text: run.title,
				checked: opened(run.runId),
				onSelect: () => open(run),
			}))}
		>
			<span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
			{`${subagents.length} subagents`}
		</MenuButton>
	);
}

/** The model Hotline Agent starts on: see `model_for` in the core. */
/**
 * What `chapter.resume` would say without asking. The room refuses a
 * second hop and a missing predecessor; the window greys the item so
 * the click is not the first they hear of it.
 */
function resumeRefusal(events: TranscriptEvent[], backendId: string): string | null {
	const chapters = events.filter((event): event is Extract<TranscriptEvent, { kind: "chapter" }> => event.kind === "chapter");
	let openAt = -1;
	for (let index = chapters.length - 1; index >= 0; index--) {
		if (chapters[index]!.endedAt === undefined) {
			openAt = index;
			break;
		}
	}
	if (openAt <= 0) return "There is no previous chapter to reopen.";
	const previous = chapters[openAt - 1]!;
	if (previous.closedBy === "resume") return "There is no previous chapter to reopen.";
	if (previous.backendId !== backendId) {
		return "The previous chapter ran on a different agent; its context cannot be reopened here.";
	}
	return null;
}
