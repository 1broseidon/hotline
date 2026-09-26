import { useCallback, useEffect, useMemo, useState } from "react";
import type { Attachment, ConfigChoice, ScheduledJob, TranscriptEvent } from "../generated/contract";
import { chordGlyph, chordKeys } from "../chords";
import { openComputer, useComputerViewer } from "../computer";
import { ChainIcon, ClockIcon, ComputerIcon, MoreIcon, ProgressRing, WarningIcon } from "../icons";
import { revealPath } from "../native";
import { nextText } from "../room";
import { useTape } from "../tape";
import { Avatar } from "../ui/Avatar";
import { Band } from "../ui/Band";
import { MenuButton, type MenuEntry } from "../ui/Menu";
import { useNarrow } from "../narrow";
import { wire, type RosterEntry } from "../wire";
import { Composer, isDown } from "./Composer";
import { SessionPickers } from "./Pickers";
import { Search } from "./Search";
import { Starters } from "./Starters";
import type { OpenSubagent } from "./Subagent";
import type { OpenThread } from "./Thread";
import { Transcript, turnCauseLine, type ReplyTarget } from "./Transcript";

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
	onOpenSubagent,
	models,
	onSaid,
}: {
	entry: RosterEntry;
	/** The room's models, for the model picker in the band. */
	models: ConfigChoice[];
	/** Where the band's model or effort picker hands a refusal. */
	onSaid(said: string | null): void;
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
	onOpenThread(thread: OpenThread): void;
	onOpenSubagent(run: OpenSubagent): void;
}) {
	const { persona, session } = entry;
	const personaId = persona.id;
	const { events, streaming, loaded, pulling } = useTape(personaId);
	const [replying, setReplying] = useState<ReplyTarget | null>(null);
	/* What was said, from the moment it was said. The core writes the line
	 * only once a session is up, and starting one is a second or two in which
	 * a composer that has already emptied looks like it did nothing. */
	const [saying, setSaying] = useState<Saying | null>(null);
	const [refill, setRefill] = useState<Refill | null>(null);
	/* A refusal the core handed up: a chapter that would not open, a message
	 * that would not send. The band is the one place with room for a sentence. */
	const [refused, setRefused] = useState<string | null>(null);
	const [chapterBusy, setChapterBusy] = useState(false);

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
			void started
				.then(() =>
					wire.command("session.prompt", {
						personaId,
						text,
						...(answered ? { replyTo: answered.eventId } : {}),
						...(attachments.length > 0 ? { attachments } : {}),
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
	const untouched = loaded && saying === null && !events.some((event) => event.kind === "user");

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
	const openScreen = screen === undefined ? undefined : () => void openComputer(personaId, persona.name, screen);
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
		{ kind: "item", id: "reveal", text: "Reveal working directory", onSelect: () => void revealPath(persona.cwd) },
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
	/* The band is the one place that says who this is, so it also says what
	 * they are doing: the kind of work while a turn runs, else what they are
	 * for. The window's title bar carries only the mark. Before the first
	 * tool call lands there is no activity yet; if the turn began answering
	 * a delivery rather than a fresh word from the person, that is worth
	 * saying instead of a bare "Working". */
	const status =
		session.state === "thinking" ? (entry.activity ?? turnCauseLine(events) ?? "Working") : persona.goal.split("\n")[0]!.trim();

	const notice = session.error !== undefined && session.error !== "" ? session.error : (said ?? refused);
	const links = entry.links ?? [];
	const nameFor = (id: string) => roster.find((one) => one.persona.id === id)?.persona.name ?? id;
	const unlink = useCallback(
		(withPersonaId: string) => void wire.command("teammates.unlink", { a: personaId, b: withPersonaId }),
		[personaId],
	);

	return (
		<section className="conversation pane" aria-label={`Conversation with ${persona.name}`}>
			<Band>
				<button
					type="button"
					className="control btn-quiet -ml-1 min-w-0 shrink gap-2 pl-1 pr-2"
					// The name is the one way into the teammate's pane, the way a
					// messages app opens a contact from its header.
					title={`Teammate (${chordKeys("teammate")})`}
					aria-label={session.state === "thinking" ? `${persona.name}, working` : persona.name}
					aria-expanded={inspectorOpen}
					onClick={onToggleInspector}
				>
					<Avatar id={persona.id} name={persona.name} size={20} />
					<span className="shrink-0 text-lg font-semibold text-ink">{persona.name}</span>
					{session.state === "thinking" && (
						<span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
					)}
					{status !== "" && <span className="band-status">{status}</span>}
				</button>

				<span className="min-w-0 flex-1" />

				<SessionPickers key={persona.id} entry={entry} models={models} onSaid={onSaid} />

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

			{links.length > 0 && (
				<div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 px-4 py-1.5 text-sm text-ink-3">
					<ChainIcon className="shrink-0" />
					{links.map((link) => (
						<span key={link.withPersonaId} className={`flex items-center gap-1.5 ${link.paused ? "text-ink-4" : ""}`}>
							{`Linked with ${nameFor(link.withPersonaId)}${link.paused ? " · paused" : ""}`}
							<button type="button" className="control btn-quiet btn-sm" onClick={() => unlink(link.withPersonaId)}>
								Unlink
							</button>
						</span>
					))}
				</div>
			)}

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
					events={shown}
					streaming={streaming}
					live={session.state === "thinking"}
					focus={focus}
					onReply={setReplying}
					{...(openScreen !== undefined ? { onOpenScreen: openScreen } : {})}
					onOpenThread={(event) =>
						onOpenThread({
							key: event.threadKey,
							withName: event.withName,
						})
					}
					onOpenSubagent={(event) => onOpenSubagent({ runId: event.runId, title: event.title })}
				/>
				{untouched && (
					<div className="relative shrink-0 px-6">
						<Starters persona={persona} onPick={(text) => setRefill({ text, attachments: [], nonce: Date.now() })} />
					</div>
				)}
				<Composer
					personaId={personaId}
					name={persona.name}
					state={session.state}
					replyQuote={replying?.text ?? null}
					onSend={send}
					{...(refill !== null ? { refill } : {})}
					onCancel={cancel}
					onClearReply={() => setReplying(null)}
				/>
				{searchOpen && (
					<Search personaId={personaId} roster={roster} onClose={onCloseSearch} onPick={onPick} />
				)}
			</div>
		</section>
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
