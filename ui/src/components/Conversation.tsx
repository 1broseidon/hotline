import { useCallback, useEffect, useState } from "react";
import type { Attachment, ConfigChoice, ScheduledJob, SessionConfig, TranscriptEvent } from "../generated/contract";
import { chordGlyph, chordKeys } from "../chords";
import { ClockIcon, InfoIcon, MoreIcon, SearchIcon, WarningIcon } from "../icons";
import { revealPath } from "../native";
import { nextText, useRoomSettings } from "../room";
import { useTape } from "../tape";
import { Avatar } from "../ui/Avatar";
import { BackKey, Band } from "../ui/Band";
import { MenuButton, Picker, type MenuEntry } from "../ui/Menu";
import { wire, type RosterEntry } from "../wire";
import { Composer } from "./Composer";
import { Search } from "./Search";
import type { OpenThread } from "./Thread";
import { Transcript, type ReplyTarget } from "./Transcript";

/** Toad Agent's stored backend id. Any other id is an ACP harness. */
const TOAD_AGENT = "pi";

/**
 * One teammate's conversation: the band naming them, the transcript, the
 * composer, and the search over it. Keyed by teammate above, so switching
 * tears the tape subscription down and puts up another rather than folding
 * two conversations into one column.
 *
 * A teammate is always there. Whether a session is up behind them is
 * plumbing the band never reports: the one state it shows is the beat
 * while they work, and a message to a resting teammate starts the session
 * on its way. Stopping is the one deliberate act, under More.
 */
export function Conversation({
	entry,
	roster,
	models,
	jobs,
	searchOpen,
	inspectorOpen,
	focus,
	onBack,
	onToggleInspector,
	onOpenSchedules,
	onToggleSearch,
	onCloseSearch,
	onDelete,
	onPick,
	onOpenThread,
}: {
	entry: RosterEntry;
	/** A narrow window: the rail is a step back from here. */
	onBack?: () => void;
	roster: RosterEntry[];
	models: ConfigChoice[];
	jobs: ScheduledJob[];
	searchOpen: boolean;
	inspectorOpen: boolean;
	focus: { eventId: string; at: number } | null;
	onToggleInspector(): void;
	onOpenSchedules(): void;
	onToggleSearch(): void;
	onCloseSearch(): void;
	onDelete(): void;
	onPick(personaId: string, eventId: string): void;
	onOpenThread(thread: OpenThread): void;
}) {
	const { persona, session } = entry;
	const personaId = persona.id;
	const { events, streaming } = useTape(personaId);
	const [replying, setReplying] = useState<ReplyTarget | null>(null);
	const [chapterSaid, setChapterSaid] = useState<string | null>(null);
	const [modelSaid, setModelSaid] = useState<string | null>(null);
	const [chapterBusy, setChapterBusy] = useState(false);
	const [idleEfforts, setIdleEfforts] = useState<ConfigChoice[]>([]);
	const { defaultModelId, lastModelId } = useRoomSettings();

	const send = useCallback(
		(text: string, attachments: Attachment[]) => {
			void wire.command("session.prompt", {
				personaId,
				text,
				...(replying ? { replyTo: replying.eventId } : {}),
				...(attachments.length > 0 ? { attachments } : {}),
			});
			setReplying(null);
		},
		[personaId, replying],
	);
	const start = useCallback(() => void wire.command("session.start", { personaId }), [personaId]);
	const stop = useCallback(() => void wire.command("session.stop", { personaId }), [personaId]);
	const cancel = useCallback(() => void wire.command("session.cancel", { personaId }), [personaId]);
	/* Success is the tape: the marker is superseded in place and the title
	 * lands on its line. Only a refusal needs a sentence here. */
	const startChapter = useCallback(() => {
		setChapterSaid(null);
		setChapterBusy(true);
		void wire
			.command("chapter.start_fresh", { personaId })
			.catch((error: Error) => setChapterSaid(error.message))
			.finally(() => setChapterBusy(false));
	}, [personaId]);
	const resumeChapter = useCallback(() => {
		setChapterSaid(null);
		setChapterBusy(true);
		void wire
			.command("chapter.resume", { personaId })
			.catch((error: Error) => setChapterSaid(error.message))
			.finally(() => setChapterBusy(false));
	}, [personaId]);
	const resumeBlocked = resumeRefusal(events, persona.backendId);

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

	const toad = persona.backendId === TOAD_AGENT;
	const modelChoices = session.models.length > 0 ? session.models : toad ? models : [];
	// The band names the model a turn would run on, whether or not a session
	// is up. For Toad Agent that is the driver's own rule: the teammate's
	// choice when the list still has it, else the room default, else the
	// last model used, else the first choice — newest only on a desk that
	// has never run a model.
	const currentModel =
		session.currentModelId ??
		(toad ? toadModel(persona.modelId, defaultModelId, lastModelId, modelChoices) : (persona.modelId ?? ""));
	const showModel = modelChoices.length > 0 || (toad && currentModel !== "");
	const currentMode = session.currentModeId ?? persona.modeId ?? "";
	// An idle Toad Agent session carries no configs. The band derives the
	// effort picker the same way it derives currentModel: the catalogue
	// for the model a turn would run on.
	useEffect(() => {
		if (!toad || currentModel === "") {
			setIdleEfforts([]);
			return;
		}
		let cancelled = false;
		void wire.command("models.efforts", { modelId: currentModel }).then(
			(choices) => {
				if (!cancelled) setIdleEfforts(choices);
			},
			() => {
				if (!cancelled) setIdleEfforts([]);
			},
		);
		return () => {
			cancelled = true;
		};
	}, [toad, currentModel]);
	const configs: SessionConfig[] =
		session.configs.length > 0
			? session.configs
			: toad && idleEfforts.length > 0
				? [{ id: "effort", name: "Effort", currentId: persona.effortId ?? "", options: idleEfforts }]
				: [];
	const running = session.state === "ready" || session.state === "thinking" || session.state === "starting";
	const next = jobs.reduce<ScheduledJob | null>(
		(soonest, job) => (soonest === null || job.nextAt < soonest.nextAt ? job : soonest),
		null,
	);

	const more: MenuEntry[] = [
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

	const said =
		session.error !== undefined && session.error !== ""
			? session.error
			: (modelSaid ??
				chapterSaid ??
				(resumeBlocked !== null && resumeBlocked !== "There is no previous chapter to reopen."
					? resumeBlocked
					: null));

	return (
		<section className="conversation pane" aria-label={`Conversation with ${persona.name}`}>
			<Band>
				{onBack !== undefined && <BackKey onBack={onBack} />}
				<button
					type="button"
					className="control btn-quiet -ml-1 min-w-0 shrink gap-2 pl-1 pr-2"
					aria-label={session.state === "thinking" ? `${persona.name}, working` : persona.name}
					onClick={onToggleInspector}
				>
					<Avatar id={persona.id} name={persona.name} size={20} />
					<span className="truncate text-lg font-semibold text-ink">{persona.name}</span>
					{session.state === "thinking" && (
						<span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
					)}
				</button>

				<span className="min-w-0 flex-1" />

				{next !== null && (
					<button
						type="button"
						className="control btn-quiet gap-1.5 px-2 text-sm"
						title="Schedules"
						aria-label={`${jobs.length} scheduled, next ${nextText(next.nextAt)}`}
						onClick={onOpenSchedules}
					>
						<ClockIcon className="text-ink-3" />
						{jobs.length === 1 ? "1 scheduled" : `${jobs.length} scheduled`}
						<span className="text-ink-3">{nextText(next.nextAt)}</span>
					</button>
				)}

				{showModel && (
					<Picker
						value={currentModel}
						choices={modelChoices}
						placeholder="Model"
						label={session.modelLabel ?? "Model"}
						onChange={(modelId) => {
							setModelSaid(null);
							void wire
								.command("session.set_model", { personaId, modelId })
								.catch((error: Error) => setModelSaid(error.message));
						}}
					/>
				)}
				{session.modes.length > 0 && (
					<Picker
						value={currentMode}
						choices={session.modes}
						placeholder={session.modeLabel ?? "Mode"}
						label={session.modeLabel ?? "Mode"}
						onChange={(modeId) => void wire.command("session.set_mode", { personaId, modeId })}
					/>
				)}
				{configs.map((config) => (
					<Picker
						key={config.id}
						value={config.currentId ?? ""}
						choices={config.options}
						placeholder={config.name}
						label={config.name}
						onChange={(value) => {
							setModelSaid(null);
							void wire
								.command("session.set_config", { personaId, configId: config.id, value })
								.catch((error: Error) => setModelSaid(error.message));
						}}
					/>
				))}

				<button
					type="button"
					className="control btn-icon"
					title={`Search (${chordKeys("search")})`}
					aria-label="Search"
					aria-pressed={searchOpen}
					onClick={onToggleSearch}
				>
					<SearchIcon />
				</button>
				<button
					type="button"
					className="control btn-icon"
					title={`Teammate (${chordKeys("teammate")})`}
					aria-label="Teammate"
					aria-pressed={inspectorOpen}
					onClick={onToggleInspector}
				>
					<InfoIcon />
				</button>
				<MenuButton className="control btn-icon" label="More" entries={more}>
					<MoreIcon />
				</MenuButton>
			</Band>

			{said !== null && (
				<p
					role="status"
					className="flex shrink-0 items-center gap-2 bg-danger-soft px-4 py-1.5 text-sm text-ink"
				>
					<WarningIcon className="shrink-0 text-danger" />
					<span className="min-w-0 flex-1 selectable">{said}</span>
				</p>
			)}

			<div className="relative flex min-h-0 flex-1 flex-col">
				<Transcript
					personaId={personaId}
					name={persona.name}
					events={events}
					streaming={streaming}
					live={session.state === "thinking"}
					focus={focus}
					onReply={setReplying}
					onOpenThread={(event) =>
						onOpenThread({
							key: event.threadKey,
							withName: event.withName,
						})
					}
				/>
				<Composer
					personaId={personaId}
					name={persona.name}
					state={session.state}
					replyQuote={replying?.text ?? null}
					onSend={send}
					onStart={start}
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

/** The model Toad Agent starts on: see `model_for` in the core. */
function toadModel(
	chosen: string | undefined,
	defaultModelId: string | null,
	lastModelId: string | null,
	choices: ConfigChoice[],
): string {
	if (chosen !== undefined && choices.some((one) => one.id === chosen)) return chosen;
	if (defaultModelId !== null && choices.some((one) => one.id === defaultModelId)) return defaultModelId;
	if (lastModelId !== null && choices.some((one) => one.id === lastModelId)) return lastModelId;
	return choices[0]?.id ?? "";
}

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
