import { useEffect, useRef, useState } from "react";
import type { FolderGrant, Persona, ScheduledJob, SessionInfo } from "../generated/contract";
import { chordKeys } from "../chords";
import { CloseIcon, FolderIcon, PlusIcon, RevealIcon, TrashIcon } from "../icons";
import { confirmRemove, confirmRemovePicture } from "../native";
import { chooseFolder, revealLabel, showPath } from "../serverFiles";
import { useRoomRead } from "../roomReads";
import { preferredModel, useRoomSettings } from "../room";
import { MAX_PINS } from "../pins";
import { Avatar } from "../ui/Avatar";
import { Band } from "../ui/Band";
import { Picker } from "../ui/Menu";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";
import type { RosterEntry } from "../wire";
import { Schedules } from "./Schedules";
import type { ThreadRef } from "./Transcript";
import { HOTLINE_AGENT } from "./accounts/fits";
import { AccountRow } from "./teammate/AccountRow";
import { ComputerRows } from "./teammate/ComputerRows";
import { McpRows, SkillRows } from "./teammate/GrantRows";
import { BACKGROUND_ABOUT, FoldRow, InfoKey, MACHINE_ABOUT, NESTED, RowText, SwitchRow } from "./teammate/rows";
import { Threads } from "./teammate/Threads";
import { ToolLedger } from "./teammate/ToolLedger";

// The pane's rows live in ./teammate by concern; what other components take from here is re-exported.
export { BACKGROUND_ABOUT, COMPUTER_ABOUT, MACHINE_ABOUT, SwitchRow } from "./teammate/rows";

/**
 * One teammate, beside their conversation, in a column 320px wide.
 *
 * The name is the band's heading and edits in place; the goal is the one
 * field. The working directory is a row — the folder's name and three
 * quiet keys: the full path, choose, reveal — picked rather than typed.
 * Under it, the folders it may also read, each with its own "Can edit".
 * Everything the teammate is allowed is one Access list, one row per
 * grant: a title, a value where there is one, and a switch or a chevron.
 * It runs from authority to equipment to outcome: the grants that widen
 * what it may do without you — reach, background work, who may hand it
 * work — each with an info key for the risk; then what it is given, a
 * computer and MCP servers; last, what actually attached at the last
 * start, with the failures always out. A row's second line is a value or
 * nothing; anything with more to it opens in place, indented under its
 * row. Schedules and threads follow the same shape. Removing is
 * the footer, and asks through the system dialog like the rail does.
 */
export function Teammate({
	persona,
	session,
	jobs,
	roster,
	focusSchedules,
	onClose,
	onDeleted,
	onOpenThread,
	onAccountSetup,
}: {
	persona: Persona;
	session: SessionInfo;
	jobs: ScheduledJob[];
	roster: RosterEntry[];
	focusSchedules: boolean;
	onClose(): void;
	onDeleted(): void;
	onOpenThread(thread: ThreadRef): void;
	onAccountSetup?: ((accountId: string) => void) | undefined;
}) {
	const room = useRoomSettings();
	const servers = room.mcpServers;
	const [name, setName] = useState(persona.name);
	const [goal, setGoal] = useState(persona.goal);
	const [pathShown, setPathShown] = useState(false);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const goalField = useRef<HTMLTextAreaElement>(null);
	const hotline = persona.backendId === HOTLINE_AGENT;
	// Only a session that is up has said what it takes.
	const running = session.state === "ready" || session.state === "thinking";
	const folders = persona.folders ?? [];

	useEffect(() => {
		setName(persona.name);
		setGoal(persona.goal);
	}, [persona.name, persona.goal]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: a new teammate is what hides the path again.
	useEffect(() => {
		setPathShown(false);
	}, [persona.id]);

	// The goal is as tall as its text, never a well with room to spare, up to
	// the card's four lines; past them it fades out.
	const [goalClipped, setGoalClipped] = useState(false);
	// biome-ignore lint/correctness/useExhaustiveDependencies: the goal's text is what changes its height.
	useEffect(() => {
		const field = goalField.current;
		if (!field) return;
		field.style.height = "0";
		field.style.height = `${field.scrollHeight}px`;
		setGoalClipped(field.scrollHeight > field.clientHeight + 1);
	}, [goal]);

	const backends = useRoomRead("backends.list", "", !hotline && !session.agentName);
	const harnessName = session.agentName || backends?.find((one) => one.id === persona.backendId)?.name || null;

	const save = (patch: Partial<Persona>) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		void wire
			.command("persona.update", { id: persona.id, patch })
			.catch((error: Error) => setRefusal(error.message))
			.finally(() => setBusy(false));
	};

	const saveName = () => {
		const trimmed = name.trim();
		if (!trimmed || trimmed === persona.name) {
			setName(persona.name);
			return;
		}
		save({ name: trimmed });
	};

	const removePicture = async () => {
		if (await confirmRemovePicture(persona.name)) save({ avatar: null } as unknown as Partial<Persona>);
	};

	const saveGoal = () => {
		if (goal !== persona.goal) save({ goal });
	};

	const chooseCwd = async () => {
		const dir = await chooseFolder(persona.cwd);
		if (dir !== null && dir !== persona.cwd) save({ cwd: dir });
	};

	const pinned = roster.some((entry) => entry.persona.id === persona.id && entry.pin != null);
	const pinCount = roster.filter((entry) => entry.pin != null).length;
	const togglePin = async () => {
		setRefusal(null);
		try {
			await wire.command("persona.pin", pinned ? { id: persona.id } : { id: persona.id, slot: pinCount });
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		}
	};

	const leader = roster.find((entry) => entry.lead === true)?.persona;
	const lead = leader?.id === persona.id;
	// The only teammate on a desk is its lead, and the core keeps it so.
	const alone = roster.length === 1;
	const toggleLead = async () => {
		setRefusal(null);
		try {
			await wire.command("persona.lead", lead ? {} : { id: persona.id });
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		}
	};

	const remove = async () => {
		if (busy) return;
		if (!(await confirmRemove(persona.name))) return;
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("persona.delete", { id: persona.id });
			onDeleted();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	return (
		<aside className="inspector" aria-label={`${persona.name}'s settings`}>
			<Band>
				<h2 className="eyebrow min-w-0 flex-1 truncate pl-1">Teammate</h2>
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
					<CloseIcon />
				</button>
			</Band>
			<Scroll>
				<div className="flex flex-col gap-5 px-4 pb-4 pt-2">
					{/* Who they are, the way a contact card opens: the face beside the
					    name and what they are for, each edited where it stands, and
					    the voice they answer a call in when the desk can speak. */}
					<div className="grouped profile-card">
						<div className="profile">
							{persona.avatar === undefined ? (
								<Avatar id={persona.id} name={name.trim() || persona.name} size={64} />
							) : (
								<button
									type="button"
									className="profile-picture"
									title="Remove picture"
									aria-label={`Remove ${persona.name}'s picture`}
									disabled={busy}
									onClick={() => void removePicture()}
								>
									<Avatar id={persona.id} name={name.trim() || persona.name} size={64} hash={persona.avatar.hash} />
									<span className="profile-picture-remove" aria-hidden>
										<TrashIcon />
									</span>
								</button>
							)}
							<div className="profile-text">
								<input
									aria-label="Name"
									className="profile-name"
									value={name}
									autoComplete="off"
									spellCheck={false}
									onChange={(event) => setName(event.target.value)}
									onBlur={saveName}
									onKeyDown={(event) => {
										if (event.key === "Enter") event.currentTarget.blur();
										if (event.key === "Escape") {
											setName(persona.name);
											event.currentTarget.blur();
										}
									}}
								/>
								<textarea
									ref={goalField}
									aria-label="Goal"
									className="profile-goal"
									data-clipped={goalClipped || undefined}
									rows={1}
									placeholder="What this teammate is for"
									value={goal}
									onChange={(event) => setGoal(event.target.value)}
									onBlur={saveGoal}
								/>
							</div>
						</div>
						<VoiceRow persona={persona} disabled={busy} onSave={save} />
						<div className="group-row">
							<RowText title="Team lead" value={leader?.name ?? "none"} />
							<button
								type="button"
								className="control btn-quiet btn-sm -mr-1.5"
								disabled={busy || (lead && alone)}
								title={lead && alone ? "Only teammate" : undefined}
								onClick={() => void toggleLead()}
							>
								{lead ? "Remove as lead" : "Make team lead"}
							</button>
						</div>
					</div>

					<AccountRow
						persona={persona}
						session={session}
						// As the core reads it: an empty current model is no model, and no model is the room's preferred one.
						modelId={session.currentModelId || (persona.modelId ?? preferredModel(room))}
						disabled={busy}
						onSetup={onAccountSetup}
					/>

					<section>
						<h3 className="label">Folders</h3>
						<div className="grouped">
							<div className="group-row">
								<RowText title={folderName(persona.cwd) === persona.id ? "Main workspace" : folderName(persona.cwd)} />
								<span className="-my-1 -mr-2 flex items-center gap-0.5">
									<InfoKey about={persona.cwd} label="Show the full path" open={pathShown} onToggle={() => setPathShown((was) => !was)} />
									<button
										type="button"
										className="control btn-icon btn-quiet h-6 w-6 text-ink-3"
										title="Choose another folder"
										aria-label="Choose another folder"
										disabled={busy}
										onClick={() => void chooseCwd()}
									>
										<FolderIcon />
									</button>
									<button
										type="button"
										className="control btn-icon btn-quiet h-6 w-6 text-ink-3"
										title={revealLabel()}
										aria-label={revealLabel()}
										onClick={() => showPath(persona.cwd)}
									>
										<RevealIcon />
									</button>
								</span>
							</div>
							{pathShown && (
								<div className={NESTED}>
									<span className="group-row-text">
										<span className="group-row-detail selectable font-mono" style={{ whiteSpace: "normal", wordBreak: "break-all" }}>
											{persona.cwd}
										</span>
									</span>
								</div>
							)}
							<FolderRows folders={folders} disabled={busy} onChange={(next) => save({ folders: next })} />
						</div>
						{folders.length > 0 && (
							<p className="group-hint">
								{foldersNote({
									hotline,
									machine: persona.reach === "machine",
									takesFolders: !running || session.capabilities.additionalDirectories,
									computer: persona.computer?.enabled === true,
								})}
							</p>
						)}
					</section>

					{/* What it may do without you: reach, waking itself, who may hand it work. */}
					<section>
						<h3 className="label">Permissions</h3>
						<div className="grouped">
							{hotline ? (
								<SwitchRow
									title="Whole machine"
									about={MACHINE_ABOUT}
									checked={persona.reach === "machine"}
									disabled={busy}
									onChange={(on) => save({ reach: on ? "machine" : "workspace" })}
								/>
							) : (
								<HarnessRows
									personaId={persona.id}
									session={session}
									harness={harnessName ?? "the ACP harness"}
									disabled={busy}
									onRefusal={setRefusal}
								/>
							)}
							<SwitchRow
								title="Background work"
								about={BACKGROUND_ABOUT}
								checked={persona.backgroundWork}
								disabled={busy}
								onChange={(backgroundWork) => save({ backgroundWork })}
							/>
							<CollaborationRows
								allowed={persona.allowedSenders}
								roster={roster}
								disabled={busy}
								onChange={(allowedSenders) => save({ allowedSenders })}
							/>
						</div>
					</section>

					<section>
						<h3 className="label">Computer</h3>
						<div className="grouped">
							<ComputerRows
								personaId={persona.id}
								teammate={persona.name}
								computer={persona.computer}
								disabled={busy}
								onChange={(computer) => save({ computer })}
							/>
						</div>
					</section>

					{/* What it is given to work with, and what actually attached at the last start. */}
					<section>
						<h3 className="label">Tools</h3>
						<div className="grouped">
							<McpRows
								policy={persona.mcpPolicy}
								servers={servers}
								disabled={busy}
								onChange={(mcpPolicy) => save({ mcpPolicy })}
							/>
							<SkillRows
								personaId={persona.id}
								policy={persona.skillPolicy}
								disabled={busy}
								onChange={(skillPolicy) => save({ skillPolicy })}
							/>
							<ToolLedger personaId={persona.id} sessionState={session.state} servers={servers} />
						</div>
					</section>

					<Schedules
						personaId={persona.id}
						jobs={jobs}
						backgroundWork={persona.backgroundWork}
						focus={focusSchedules}
					/>

					<Threads personaId={persona.id} roster={roster} onOpen={onOpenThread} />

					<div className="flex items-center gap-1 border-t border-line pt-3">
						<button type="button" className="control btn-danger -ml-2.5" disabled={busy} onClick={() => void remove()}>
							Remove teammate…
						</button>
						<button
							type="button"
							className="control btn-quiet ml-auto"
							disabled={busy || (!pinned && pinCount >= MAX_PINS)}
							title={!pinned && pinCount >= MAX_PINS ? "Pins full" : undefined}
							onClick={() => void togglePin()}
						>
							{pinned ? "Unpin" : "Pin to top"}
						</button>
					</div>

					{refusal !== null && (
						<p role="status" className="selectable text-sm text-danger">
							{refusal}
						</p>
					)}
				</div>
			</Scroll>
		</aside>
	);
}

/** What the desk speaks with now, and the voices that model offers. */
type Speaking = { providerId: string; modelId: string; voice?: string; voices: string[] };

/**
 * What calls speak with and its voices, or null while the desk cannot speak:
 * the voice front when the desk runs one (Grok Voice, GPT-Live, GPT
 * Realtime), else Legacy voice's speaking model.
 */
function useSpeaking(): Speaking | null {
	const status = useRoomRead("voice.status");
	const options = useRoomRead("capabilities.options");
	if (status === undefined || options === undefined) return null;
	if (status.front !== undefined) return status.front;
	const tts = status.tts;
	const voices = options.tts.options
		.find((provider) => provider.providerId === tts?.providerId)
		?.models.find((model) => model.id === tts?.modelId)?.voices;
	return tts === undefined || voices === undefined || voices.length === 0 ? null : { ...tts, voices };
}

/** Voices read as names: "eve" is Eve. */
function voiceName(voice: string): string {
	return voice.charAt(0).toUpperCase() + voice.slice(1);
}

const DESK_VOICE = "";

/**
 * The voice this teammate answers a direct call in. It is one of the voices
 * of whatever calls speak with now, the voice front or Legacy voice's
 * speaking model; a voice picked for another reads as the desk's, which is
 * what a call would use.
 */
function VoiceRow({ persona, disabled, onSave }: { persona: Persona; disabled: boolean; onSave(patch: Partial<Persona>): void }) {
	const speaking = useSpeaking();
	if (speaking === null) return null;
	const own = persona.voice;
	const current =
		own !== undefined && own.providerId === speaking.providerId && own.modelId === speaking.modelId && speaking.voices.includes(own.voice)
			? own.voice
			: DESK_VOICE;
	const desk = speaking.voice ?? speaking.voices[0];
	const choices = [
		{ id: DESK_VOICE, name: desk === undefined ? "The desk's" : `The desk's · ${voiceName(desk)}` },
		...speaking.voices.map((voice) => ({ id: voice, name: voiceName(voice) })),
	];
	return (
		<div className="group-row">
			<RowText title="Voice" />
			<Picker
				value={current}
				choices={choices}
				placeholder="Voice"
				label={`The voice ${persona.name} answers a call in`}
				disabled={disabled}
				onChange={(voice) => {
					if (voice === current) return;
					onSave({
						voice: voice === DESK_VOICE ? null : { providerId: speaking.providerId, modelId: speaking.modelId, voice },
					} as unknown as Partial<Persona>);
				}}
			/>
		</div>
	);
}

/** The last segment of a path: the name a person knows the folder by. */
function folderName(path: string): string {
	const parts = path.split(/[\\/]+/).filter((part) => part !== "");
	return parts[parts.length - 1] ?? path;
}

/**
 * An ACP teammate's reach is the harness's own. What Hotline can offer is
 * the runtime mode the harness advertised, if any.
 */
function HarnessRows({
	personaId,
	session,
	harness,
	disabled,
	onRefusal,
}: {
	personaId: string;
	session: SessionInfo;
	harness: string;
	disabled: boolean;
	onRefusal(text: string | null): void;
}) {
	const modeLabel = session.modeLabel ?? "Runtime mode";
	return (
		<>
			<div className="group-row">
				<RowText title={`Managed by ${harness}`} />
			</div>
			<div className="group-row">
				<RowText title={modeLabel} {...(session.modes.length === 0 ? { value: "not advertised" } : {})} />
				{session.modes.length > 0 && (
					<Picker
						value={session.currentModeId ?? ""}
						choices={session.modes}
						placeholder={modeLabel}
						label={modeLabel}
						disabled={disabled}
						onChange={(modeId) => {
							onRefusal(null);
							void wire
								.command("session.set_mode", { personaId, modeId })
								.catch((error: Error) => onRefusal(error.message));
						}}
					/>
				)}
			</div>
		</>
	);
}

/**
 * Who may hand this teammate work without asking first. A teammate with
 * the whole machine asks nobody; a workspace teammate asks once, and the
 * answer that says "always" lands here as a row you can revoke.
 */
function CollaborationRows({
	allowed,
	roster,
	disabled,
	onChange,
}: {
	allowed: string[];
	roster: RosterEntry[];
	disabled: boolean;
	onChange(allowed: string[]): void;
}) {
	const [open, setOpen] = useState(false);
	const nameFor = (id: string) => roster.find((entry) => entry.persona.id === id)?.persona.name ?? id;

	if (allowed.length === 0) {
		return (
			<div className="group-row">
				<RowText title="Collaboration" value="none" />
			</div>
		);
	}

	return (
		<>
			<FoldRow
				title="Collaboration"
				value={allowed.map(nameFor).join(", ")}
				open={open}
				onToggle={() => setOpen((was) => !was)}
			/>
			{open &&
				allowed.map((senderId) => (
					<div className={NESTED} key={senderId}>
						<RowText title={nameFor(senderId)} value="may hand it work" />
						<button
							type="button"
							className="control btn-quiet btn-sm"
							disabled={disabled}
							onClick={() => onChange(allowed.filter((id) => id !== senderId))}
						>
							Revoke
						</button>
					</div>
				))}
		</>
	);
}

/**
 * The line under the folder list: who holds the teammate to them. Hotline
 * Agent's own tools do, unless it has the whole machine; an ACP harness
 * enforces its own access, and is handed the folders only when it said it
 * takes them (`takesFolders` is false only for a running session that did not).
 */
export function foldersNote({
	hotline,
	machine,
	takesFolders,
	computer,
}: {
	hotline: boolean;
	machine: boolean;
	takesFolders: boolean;
	computer: boolean;
}): string {
	const who = hotline
		? machine
			? "Whole machine already reaches everything; these folders apply if you turn it off."
			: "Its file tools and protected shell reach these too; the rest of the machine stays hidden."
		: takesFolders
			? "This agent enforces its own access; Hotline passes these folders to it."
			: "This agent enforces its own access. It does not take extra folders, so only Hotline's own file callbacks honour them.";
	return computer ? `${who} Its computer sees them under /home/agent/folders.` : who;
}

/**
 * Folders outside the workspace the teammate may also read, and edit where
 * "Can edit" is on: one row each, and the row that adds one. The caller
 * puts them in a group under the workspace. The list is saved whole on
 * every change; the pane renders what the record holds, which is the path
 * as the core resolved it.
 */
export function FolderRows({
	folders,
	disabled,
	onChange,
}: {
	folders: FolderGrant[];
	disabled: boolean;
	onChange(folders: FolderGrant[]): void;
}) {
	const add = async () => {
		const path = await chooseFolder(undefined, "Choose a folder for it to read");
		if (path === null || folders.some((folder) => folder.path === path)) return;
		onChange([...folders, { path, writable: false }]);
	};

	return (
		<>
			{folders.map((folder) => (
				<div key={folder.path} className="group-row items-start">
					<span className="group-row-text flex flex-col gap-1.5">
						<span className="group-row-title selectable font-mono text-sm" style={{ whiteSpace: "normal", wordBreak: "break-all" }}>
							{folder.path}
						</span>
						<label className="flex items-center gap-2 text-sm text-ink-2">
							<input
								type="checkbox"
								className="switch"
								checked={folder.writable}
								disabled={disabled}
								onChange={(event) =>
									onChange(folders.map((one) => (one.path === folder.path ? { ...one, writable: event.target.checked } : one)))
								}
							/>
							Can edit
						</label>
					</span>
					<button
						type="button"
						className="control btn-icon -mr-1.5"
						aria-label={`Remove ${folder.path}`}
						title="Remove"
						disabled={disabled}
						onClick={() => onChange(folders.filter((one) => one.path !== folder.path))}
					>
						<CloseIcon />
					</button>
				</div>
			))}
			<button type="button" className="group-row group-row-add" disabled={disabled} onClick={() => void add()}>
				<PlusIcon />
				Add folder
			</button>
		</>
	);
}
