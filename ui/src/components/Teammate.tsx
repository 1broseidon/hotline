import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import type {
	ComputerMount,
	ComputerStatus,
	McpPolicy,
	PeerThreadSummary,
	Persona,
	PersonaComputer,
	PolicyMode,
	ScheduledJob,
	SessionInfo,
	SessionState,
	SkillEntry,
	SkillPolicy,
	TeammateToolLedger,
	ToolLedgerRow,
} from "../generated/contract";
import { chordKeys } from "../chords";
import { CheckIcon, ChevronDownIcon, ChevronRightIcon, CloseIcon, FolderIcon, InfoIcon, PlusIcon, RevealIcon, WarningIcon } from "../icons";
import { useMcpServers, type McpServer } from "../mcp";
import { COMPUTER_STATUS_EVERY_MS } from "../computer";
import { confirmRemove, pickDirectory, revealPath } from "../native";
import { firstLine } from "../room";
import { Band } from "../ui/Band";
import { Picker } from "../ui/Menu";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";
import type { RosterEntry } from "../wire";
import { PathField } from "./PathField";
import { Schedules } from "./Schedules";
import type { OpenThread } from "./Thread";

/**
 * One teammate, beside their conversation, in a column 320px wide.
 *
 * The name is the band's heading and edits in place; the goal is the one
 * field. The working directory is a row — the folder's name and three
 * quiet keys: the full path, choose, reveal — picked rather than typed.
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
}: {
	persona: Persona;
	session: SessionInfo;
	jobs: ScheduledJob[];
	roster: RosterEntry[];
	focusSchedules: boolean;
	onClose(): void;
	onDeleted(): void;
	onOpenThread(thread: OpenThread): void;
}) {
	const servers = useMcpServers();
	const [name, setName] = useState(persona.name);
	const [goal, setGoal] = useState(persona.goal);
	const [pathShown, setPathShown] = useState(false);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [harnessName, setHarnessName] = useState<string | null>(session.agentName ?? null);
	const goalField = useRef<HTMLTextAreaElement>(null);
	const toad = persona.backendId === "toad";

	useEffect(() => {
		setName(persona.name);
		setGoal(persona.goal);
	}, [persona.name, persona.goal]);

	useEffect(() => {
		setPathShown(false);
	}, [persona.id]);

	// The goal is as tall as its text, never a well with room to spare.
	useEffect(() => {
		const field = goalField.current;
		if (!field) return;
		field.style.height = "0";
		field.style.height = `${field.scrollHeight}px`;
	}, [goal]);

	useEffect(() => {
		if (toad) {
			setHarnessName(null);
			return;
		}
		if (session.agentName !== undefined && session.agentName !== "") {
			setHarnessName(session.agentName);
			return;
		}
		let cancelled = false;
		void wire.command("backends.list", {}).then(
			(list) => {
				if (!cancelled) setHarnessName(list.find((one) => one.id === persona.backendId)?.name ?? null);
			},
			() => {
				if (!cancelled) setHarnessName(null);
			},
		);
		return () => {
			cancelled = true;
		};
	}, [persona.backendId, session.agentName, toad]);

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

	const saveGoal = () => {
		if (goal !== persona.goal) save({ goal });
	};

	const chooseCwd = async () => {
		const dir = await pickDirectory();
		if (dir !== null && dir !== persona.cwd) save({ cwd: dir });
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
				<input
					aria-label="Name"
					className="min-w-0 flex-1 rounded-[var(--radius-control)] bg-transparent px-1 text-lg font-semibold text-ink outline-none hover:bg-hover focus:bg-hover"
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
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
					<CloseIcon />
				</button>
			</Band>
			<Scroll>
				<div className="flex flex-col gap-5 px-4 py-4">
					<div>
						<label className="label" htmlFor="edit-goal">
							Goal
						</label>
						<textarea
							ref={goalField}
							id="edit-goal"
							className="field min-h-[46px] overflow-hidden"
							rows={2}
							placeholder="What this teammate is for."
							value={goal}
							onChange={(event) => setGoal(event.target.value)}
							onBlur={saveGoal}
						/>
					</div>

					<section>
						<h3 className="label">Working directory</h3>
						<div className="grouped">
							<div className="group-row">
								<RowText title={folderName(persona.cwd)} />
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
										title="Reveal in the file manager"
										aria-label="Reveal in the file manager"
										onClick={() => void revealPath(persona.cwd)}
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
						</div>
					</section>

					<section>
						<h3 className="label">Access</h3>
						<div className="grouped">
							{toad ? (
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
							<ComputerRows
								personaId={persona.id}
								computer={persona.computer}
								disabled={busy}
								onChange={(computer) => save({ computer })}
							/>
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

					<Threads personaId={persona.id} onOpen={onOpenThread} />

					<div className="border-t border-line pt-3">
						<button type="button" className="control btn-danger -ml-2.5" disabled={busy} onClick={() => void remove()}>
							Remove teammate…
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

/** A row's words: the title, and after a dot the value, if it has one. */
function RowText({ title, value }: { title: string; value?: string }) {
	return (
		<span className="group-row-text">
			<span className="group-row-title">
				{title}
				{value !== undefined && <span className="text-ink-3"> · {value}</span>}
			</span>
		</span>
	);
}

/** The small key that shows a row's one sentence: on hover as a tooltip, on press as a line under the row. */
function InfoKey({ about, label, open, onToggle }: { about: string; label: string; open: boolean; onToggle(): void }) {
	return (
		<button
			type="button"
			className="control btn-icon btn-quiet h-6 w-6 text-ink-3"
			title={about}
			aria-label={label}
			aria-expanded={open}
			onClick={onToggle}
		>
			<InfoIcon />
		</button>
	);
}

/** The last segment of a path: the name a person knows the folder by. */
function folderName(path: string): string {
	const parts = path.split(/[\\/]+/).filter((part) => part !== "");
	return parts[parts.length - 1] ?? path;
}

/**
 * A grant that is on or off. A grant with a risk worth a sentence gets an
 * info key after its title; the sentence opens as a line under the row,
 * so the row itself stays one line.
 */
/** The words for the access switches, shared with the new-teammate form so both say the same thing. */
export const MACHINE_ABOUT =
	"On gives built-in tools this account's full access and lets it delegate without asking. Off isolates supported built-in tools from unrelated host files and other workspaces, except installed runtimes. Network and granted MCP access remain available.";
export const BACKGROUND_ABOUT =
	"It may set its own schedules and wake itself, including after you stop a session. Off pauses the ones it made; jobs you add here run either way.";
export const COMPUTER_ABOUT =
	"A desktop of its own in a container, driven through its capture, input, browser and shell tools. Off keeps its work on this machine.";

export function SwitchRow({
	title,
	value,
	about,
	checked,
	disabled,
	onChange,
}: {
	title: string;
	value?: string;
	about?: string;
	checked: boolean;
	disabled: boolean;
	onChange(checked: boolean): void;
}) {
	const [told, setTold] = useState(false);
	return (
		<>
			<label className="group-row group-row-choice">
				<RowText title={title} {...(value !== undefined ? { value } : {})} />
				{about !== undefined && (
					<span className="-my-1 -ml-2 flex">
						<InfoKey about={about} label={`About ${title}`} open={told} onToggle={() => setTold((was) => !was)} />
					</span>
				)}
				<input
					type="checkbox"
					className="switch"
					checked={checked}
					disabled={disabled}
					onChange={(event) => onChange(event.target.checked)}
				/>
			</label>
			{told && about !== undefined && (
				<div className={NESTED}>
					<span className="group-row-text">
						<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
							{about}
						</span>
					</span>
				</div>
			)}
		</>
	);
}

/** A row with more under it, opened in place. */
function FoldRow({
	title,
	value,
	open,
	onToggle,
	mark,
}: {
	title: string;
	value?: string;
	open: boolean;
	onToggle(): void;
	mark?: ReactNode;
}) {
	return (
		<button type="button" className="group-row group-row-choice w-full text-left" aria-expanded={open} onClick={onToggle}>
			<RowText title={title} {...(value !== undefined ? { value } : {})} />
			{mark}
			{open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />}
		</button>
	);
}

/** What a fold opens is set in from its row, so the list reads as a tree. */
const NESTED = "group-row pl-7";

/**
 * An ACP teammate's reach is the harness's own. What Toad can offer is
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

const STATE_WORDS: Record<ComputerStatus["state"], { value: string; action: string }> = {
	running: { value: "running", action: "Stopping keeps the container for the next start." },
	stopped: { value: "stopped", action: "Kept for the next start. Removing it starts over." },
	absent: { value: "no container yet", action: "Built the first time this teammate starts." },
};

const UPDATE_ABOUT =
	"Updating recreates the computer on the newer release. The workspace, prepared environments, job history and browser profile live on volumes and come back with it; anything installed into the container itself outside them is gone, and running jobs stop.";

const LIMITS_ABOUT =
	"Blank is 4g of memory and 1024 processes; 0 processes is unlimited. Larger builds can request more memory or processes.";

/** The settings without one of them, so blank means absent on the wire rather than an empty string. */
function without(computer: PersonaComputer, key: "image" | "memory" | "pids" | "mounts"): PersonaComputer {
	const next = { ...computer };
	delete next[key];
	return next;
}

/**
 * The teammate's computer: the switch, and under it, while there is a
 * desktop to speak of, one row carrying its state that opens to what the
 * container is built with — image, memory, process limit, the host
 * folders bound in — and its Stop or Remove. Every change spreads the
 * settings it does not touch, so flipping the switch never drops a mount.
 * Blank fields are absent on the wire, which is the default; a value the
 * runtime rejects surfaces as a start failure, not here. The status is a
 * peek every few seconds while the pane is open — asking never wakes
 * anything, so the row can be honest about a desktop that stopped on its
 * own. Opening the desktop lives in the conversation's band, which is on
 * screen whenever it runs.
 */
function ComputerRows({
	personaId,
	computer,
	disabled,
	onChange,
}: {
	personaId: string;
	computer: PersonaComputer | undefined;
	disabled: boolean;
	onChange(computer: PersonaComputer): void;
}) {
	const current: PersonaComputer = computer ?? { enabled: false };
	const mounts = current.mounts ?? [];
	const [image, setImage] = useState(current.image ?? "");
	const [memory, setMemory] = useState(current.memory ?? "");
	const [pids, setPids] = useState(current.pids === undefined ? "" : String(current.pids));
	const [status, setStatus] = useState<ComputerStatus | null>(null);
	const [open, setOpen] = useState(false);
	const [adding, setAdding] = useState(false);
	const [acting, setActing] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [updateTold, setUpdateTold] = useState(false);

	useEffect(() => {
		setImage(current.image ?? "");
	}, [current.image]);
	useEffect(() => {
		setMemory(current.memory ?? "");
	}, [current.memory]);
	useEffect(() => {
		setPids(current.pids === undefined ? "" : String(current.pids));
	}, [current.pids]);

	useEffect(() => {
		setOpen(false);
		setAdding(false);
		let gone = false;
		const ask = () => {
			void wire
				.command("computer.status", { personaId })
				.then((seen) => {
					if (!gone) setStatus(seen);
				})
				.catch(() => {
					if (!gone) setStatus(null);
				});
		};
		ask();
		const timer = setInterval(ask, COMPUTER_STATUS_EVERY_MS);
		return () => {
			gone = true;
			clearInterval(timer);
		};
	}, [personaId]);

	const commitText = (key: "image" | "memory", draft: string, setDraft: (value: string) => void) => {
		const trimmed = draft.trim();
		setDraft(trimmed);
		if (trimmed === (current[key] ?? "")) return;
		onChange(trimmed === "" ? without(current, key) : { ...current, [key]: trimmed });
	};

	const commitPids = () => {
		const trimmed = pids.trim();
		if (trimmed === "") {
			setPids("");
			if (current.pids !== undefined) onChange(without(current, "pids"));
			return;
		}
		const count = Number(trimmed);
		if (!Number.isInteger(count) || count < 0) {
			setPids(current.pids === undefined ? "" : String(current.pids));
			return;
		}
		setPids(String(count));
		if (count !== current.pids) onChange({ ...current, pids: count });
	};

	const setMounts = (next: ComputerMount[]) => onChange(next.length === 0 ? without(current, "mounts") : { ...current, mounts: next });

	const act = async (cmd: "computer.stop" | "computer.remove" | "computer.update") => {
		if (acting) return;
		setActing(true);
		setRefusal(null);
		try {
			await wire.command(cmd, { personaId });
			setStatus(await wire.command("computer.status", { personaId }));
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setActing(false);
		}
	};

	const state = status?.state ?? "absent";
	const words = STATE_WORDS[state];
	const onEnter = (commit: () => void) => (event: KeyboardEvent<HTMLInputElement>) => {
		if (event.key !== "Enter") return;
		event.preventDefault();
		commit();
	};

	return (
		<>
			<SwitchRow title="Computer" about={COMPUTER_ABOUT} checked={current.enabled} disabled={disabled} onChange={(on) => onChange({ ...current, enabled: on })} />
			{(current.enabled || state !== "absent") && (
				<>
					<FoldRow title="Desktop" value={words.value} open={open} onToggle={() => setOpen((was) => !was)} />
					{open && (
						<>
							<div className={`${NESTED} flex-col items-stretch gap-1.5`}>
								<label className="group-row-text" htmlFor="edit-computer-image">
									<span className="group-row-title">Image</span>
								</label>
								<input
									id="edit-computer-image"
									className="field w-full font-mono text-sm"
									placeholder="Room default"
									autoComplete="off"
									spellCheck={false}
									disabled={disabled}
									value={image}
									onChange={(event) => setImage(event.target.value)}
									onBlur={() => commitText("image", image, setImage)}
									onKeyDown={onEnter(() => commitText("image", image, setImage))}
								/>
							</div>
							<LimitRow id="edit-computer-memory" title="Memory" about={LIMITS_ABOUT} placeholder="4g" disabled={disabled} value={memory} onChange={setMemory} onCommit={() => commitText("memory", memory, setMemory)} />
							<LimitRow id="edit-computer-pids" title="Processes" placeholder="1024" numeric disabled={disabled} value={pids} onChange={setPids} onCommit={commitPids} />
							{mounts.map((mount, index) => (
								<div key={`${mount.host}:${mount.path}`} className={NESTED}>
									<span className="group-row-text">
										<span className="group-row-title font-mono text-sm" title={mount.host}>
											{mount.host}
										</span>
										<span className="group-row-detail font-mono" title={mount.path}>
											{mount.path}
											{mount.readonly && <span className="font-sans"> · read-only</span>}
										</span>
									</span>
									<button
										type="button"
										className="control btn-icon -mr-1.5"
										aria-label="Remove this mount"
										title="Remove"
										disabled={disabled}
										onClick={() => setMounts(mounts.filter((_, at) => at !== index))}
									>
										<CloseIcon />
									</button>
								</div>
							))}
							{adding ? (
								<AddMount
									disabled={disabled}
									onAdd={(mount) => {
										setMounts([...mounts, mount]);
										setAdding(false);
									}}
									onCancel={() => setAdding(false)}
								/>
							) : (
								<button type="button" className={`${NESTED} group-row-add`} disabled={disabled} onClick={() => setAdding(true)}>
									<PlusIcon />
									Mount a folder
								</button>
							)}
							{status?.available !== undefined && (
								<>
									<div className={NESTED}>
										<RowText title={`Running ${status.release ?? "an older release"}`} value={`${status.available} is available`} />
										<span className="-my-1 flex">
											<InfoKey about={UPDATE_ABOUT} label="About updating" open={updateTold} onToggle={() => setUpdateTold((was) => !was)} />
										</span>
										<button type="button" className="control btn-quiet btn-sm" disabled={acting || disabled} onClick={() => void act("computer.update")}>
											Update
										</button>
									</div>
									{updateTold && (
										<div className={NESTED}>
											<span className="group-row-text">
												<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
													{UPDATE_ABOUT}
												</span>
											</span>
										</div>
									)}
								</>
							)}
							<div className={NESTED}>
								<span className="group-row-text">
									<span
										className={`group-row-detail${refusal !== null ? " text-danger" : ""}`}
										style={{ whiteSpace: "normal" }}
									>
										{refusal ?? words.action}
									</span>
								</span>
								{state === "running" && (
									<button type="button" className="control btn-quiet btn-sm" disabled={acting} onClick={() => void act("computer.stop")}>
										Stop
									</button>
								)}
								{state === "stopped" && (
									<button type="button" className="control btn-quiet btn-sm" disabled={acting} onClick={() => void act("computer.remove")}>
										Remove
									</button>
								)}
							</div>
						</>
					)}
				</>
			)}
		</>
	);
}

/** A limit as one row: the title, and a short field at the right whose placeholder is the default. */
function LimitRow({
	id,
	title,
	about,
	placeholder,
	numeric = false,
	disabled,
	value,
	onChange,
	onCommit,
}: {
	id: string;
	title: string;
	about?: string;
	placeholder: string;
	numeric?: boolean;
	disabled: boolean;
	value: string;
	onChange(value: string): void;
	onCommit(): void;
}) {
	const [told, setTold] = useState(false);
	return (
		<>
			<div className={NESTED}>
				<label className="group-row-text" htmlFor={id}>
					<span className="group-row-title">{title}</span>
				</label>
				{about !== undefined && (
					<span className="-my-1 -ml-2 flex">
						<InfoKey about={about} label={`About ${title}`} open={told} onToggle={() => setTold((was) => !was)} />
					</span>
				)}
				<input
					id={id}
					className="field w-20 text-right font-mono text-sm"
					placeholder={placeholder}
					autoComplete="off"
					spellCheck={false}
					inputMode={numeric ? "numeric" : "text"}
					disabled={disabled}
					value={value}
					onChange={(event) => onChange(event.target.value)}
					onBlur={onCommit}
					onKeyDown={(event) => {
						if (event.key !== "Enter") return;
						event.preventDefault();
						onCommit();
					}}
				/>
			</div>
			{told && about !== undefined && (
				<div className={NESTED}>
					<span className="group-row-text">
						<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
							{about}
						</span>
					</span>
				</div>
			)}
		</>
	);
}

/**
 * A new mount: a host folder, where it lands inside, and whether the
 * teammate may write to it. Read-only is the default because a teammate
 * tests a checkout, it does not edit it in place. The container path must
 * be absolute; whether it exists or collides with one of Toad's is the
 * core's call, at the next start.
 */
function AddMount({
	disabled,
	onAdd,
	onCancel,
}: {
	disabled: boolean;
	onAdd(mount: ComputerMount): void;
	onCancel(): void;
}) {
	const [host, setHost] = useState("");
	const [path, setPath] = useState("");
	const [readonly, setReadonly] = useState(true);
	const ready = host.trim() !== "" && path.trim().startsWith("/");

	return (
		<div className={`${NESTED} flex-col items-stretch gap-3 py-3`}>
			<div>
				<label className="label" htmlFor="mount-host">
					Host folder
				</label>
				<PathField id="mount-host" value={host} placeholder="~/Projects/checkout" onChange={setHost} />
			</div>
			<div>
				<label className="label" htmlFor="mount-path">
					Inside the desktop
				</label>
				<input
					id="mount-path"
					className="field font-mono text-sm"
					placeholder="/home/agent/checkout"
					autoComplete="off"
					spellCheck={false}
					value={path}
					onChange={(event) => setPath(event.target.value)}
				/>
			</div>
			<label className="flex items-center gap-2 text-sm text-ink-2">
				<input type="checkbox" className="check" checked={readonly} onChange={(event) => setReadonly(event.target.checked)} />
				Read-only
			</label>
			<div className="flex justify-end gap-2">
				<button type="button" className="control btn-quiet" onClick={onCancel}>
					Cancel
				</button>
				<button
					type="button"
					className="control btn"
					disabled={disabled || !ready}
					onClick={() => onAdd({ host: host.trim(), path: path.trim(), readonly })}
				>
					Mount
				</button>
			</div>
		</div>
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

const GRANT_MODES: { id: PolicyMode; name: string; detail: string }[] = [
	{ id: "none", name: "None", detail: "Default for new teammates" },
	{ id: "some", name: "Selected", detail: "Only the servers ticked below" },
	{ id: "all", name: "All", detail: "Every gateway server, including ones added later" },
];

/**
 * Which of the app's servers this teammate is given. The row's value is
 * the answer; the fold holds the grant and the ticks. The list is kept
 * when the mode is not `some`, so toggling back does not lose the ticks.
 */
function McpRows({
	policy,
	servers,
	disabled,
	onChange,
}: {
	policy: McpPolicy;
	servers: McpServer[];
	disabled: boolean;
	onChange(policy: McpPolicy): void;
}) {
	const [open, setOpen] = useState(false);
	const toggle = (id: string) => {
		const serverIds = policy.serverIds.includes(id)
			? policy.serverIds.filter((item) => item !== id)
			: [...policy.serverIds, id];
		onChange({ ...policy, serverIds });
	};

	const picked = servers.filter((server) => policy.serverIds.includes(server.id)).map((server) => server.name);
	const value =
		policy.mode === "all" ? "all" : policy.mode === "none" ? "none" : picked.length === 0 ? "none picked" : picked.join(", ");

	return (
		<>
			<FoldRow title="MCP servers" value={value} open={open} onToggle={() => setOpen((was) => !was)} />
			{open && (
				<>
					<div className={NESTED}>
						<RowText title="Grant" />
						<Picker
							value={policy.mode}
							choices={GRANT_MODES}
							placeholder="Grant"
							label="Which MCP servers this teammate gets"
							disabled={disabled}
							onChange={(mode) => {
								if (mode !== policy.mode) onChange({ ...policy, mode: mode as PolicyMode });
							}}
						/>
					</div>
					{policy.mode === "some" &&
						(servers.length === 0 ? (
							<div className={NESTED}>
								<RowText title="No servers yet" value="add one under Settings → Tools" />
							</div>
						) : (
							servers.map((server) => (
								<label key={server.id} className={`${NESTED} group-row-choice`}>
									<input
										type="checkbox"
										className="check"
										checked={policy.serverIds.includes(server.id)}
										disabled={disabled}
										onChange={() => toggle(server.id)}
									/>
									<RowText title={server.name} />
								</label>
							))
						))}
				</>
			)}
		</>
	);
}

const SKILL_GRANT_MODES: { id: PolicyMode; name: string; detail: string }[] = [
	{ id: "none", name: "None", detail: "Default for new teammates" },
	{ id: "some", name: "Selected", detail: "Only the skills ticked below" },
	{ id: "all", name: "All", detail: "Every gateway skill, including ones added later" },
];

/**
 * Which of the gateway's skills this teammate is given, drawn like the MCP
 * rows: the row's value is the answer, the fold holds the grant and the
 * ticks. Under them, what the teammate has of its own: the built-ins every
 * teammate carries and the skills it wrote itself, read from its workspace
 * when the fold opens, so a skill it saved during the last turn is there.
 */
function SkillRows({
	personaId,
	policy,
	disabled,
	onChange,
}: {
	personaId: string;
	policy: SkillPolicy;
	disabled: boolean;
	onChange(policy: SkillPolicy): void;
}) {
	const [open, setOpen] = useState(false);
	const [entries, setEntries] = useState<SkillEntry[]>([]);

	useEffect(() => {
		if (!open) return;
		let cancelled = false;
		void wire
			.command("skills.list", { personaId })
			.then((listed) => {
				if (!cancelled) setEntries(listed);
			})
			.catch(() => {});
		return () => {
			cancelled = true;
		};
	}, [open, personaId]);

	const gateway = entries.filter((one) => one.source === "gateway" && one.invalid === undefined);
	const own = entries.filter((one) => one.source === "workspace");
	const computer = entries.find((one) => one.source === "computer");
	const toggle = (name: string) => {
		const names = policy.names.includes(name) ? policy.names.filter((item) => item !== name) : [...policy.names, name];
		onChange({ ...policy, names });
	};

	const value =
		policy.mode === "all" ? "all" : policy.mode === "none" ? "none" : policy.names.length === 0 ? "none picked" : policy.names.join(", ");

	return (
		<>
			<FoldRow title="Skills" value={value} open={open} onToggle={() => setOpen((was) => !was)} />
			{open && (
				<>
					<div className={NESTED}>
						<RowText title="Grant" />
						<Picker
							value={policy.mode}
							choices={SKILL_GRANT_MODES}
							placeholder="Grant"
							label="Which gateway skills this teammate gets"
							disabled={disabled}
							onChange={(mode) => {
								if (mode !== policy.mode) onChange({ ...policy, mode: mode as PolicyMode });
							}}
						/>
					</div>
					{policy.mode === "some" &&
						(gateway.length === 0 ? (
							<div className={NESTED}>
								<RowText title="No skills yet" value="add one under Settings → Skills" />
							</div>
						) : (
							gateway.map((entry) => (
								<label key={entry.name} className={`${NESTED} group-row-choice`}>
									<input
										type="checkbox"
										className="check"
										checked={policy.names.includes(entry.name)}
										disabled={disabled}
										onChange={() => toggle(entry.name)}
									/>
									<RowText title={entry.name} />
								</label>
							))
						))}
					{own.map((entry) => (
						<div key={entry.name} className={NESTED}>
							<span className="group-row-text">
								<span className="group-row-title">
									Its own<span className="text-ink-3"> · {entry.name}</span>
								</span>
								<span className={`group-row-detail${entry.invalid !== undefined ? " text-danger" : ""}`} style={{ whiteSpace: "normal" }}>
									{entry.invalid ?? entry.description}
								</span>
							</span>
						</div>
					))}
					{computer !== undefined && (
						<div className={NESTED}>
							<RowText title="Its computer" value={`${computer.name}, release ${computer.version ?? "unknown"}`} />
						</div>
					)}
				</>
			)}
		</>
	);
}

/**
 * What this teammate actually had at its last start: the grants above are
 * the intent, this is the outcome. One row carries the count, and the
 * tools that failed to attach are always shown under it, because those
 * are the only rows that need a person; the rest open on press. A ledger
 * is a fact of a start, not a live feed, so it is read when the pane
 * opens and again when the session changes state, the only time a new
 * one can exist.
 */
function ToolLedger({
	personaId,
	sessionState,
	servers,
}: {
	personaId: string;
	sessionState: SessionState;
	servers: McpServer[];
}) {
	const [ledger, setLedger] = useState<TeammateToolLedger | null | undefined>(undefined);
	const [open, setOpen] = useState(false);

	useEffect(() => {
		setLedger(undefined);
		setOpen(false);
	}, [personaId]);

	// The answer replaces what is shown; it never blanks it first.
	useEffect(() => {
		let cancelled = false;
		void wire
			.command("teammate.tools", { personaId })
			.then((next) => {
				if (!cancelled) setLedger(next);
			})
			.catch(() => {
				if (!cancelled) setLedger(null);
			});
		return () => {
			cancelled = true;
		};
	}, [personaId, sessionState]);

	if (ledger === undefined) return null;

	const rows = ledger?.rows ?? [];
	const missing = rows.filter((row) => row.state === "absent");
	const foldable = rows.length > missing.length;
	const shown = open ? rows : missing;
	const value =
		ledger === null ? "none yet" : `${rows.length}${missing.length > 0 ? `, ${missing.length} missing` : ""}`;

	return (
		<>
			{foldable ? (
				<FoldRow title="Tools" value={value} open={open} onToggle={() => setOpen((was) => !was)} />
			) : (
				<div className="group-row">
					<RowText title="Tools" value={value} />
				</div>
			)}
			{groupedByOrigin(shown).map(([origin, items]) => (
				<div key={origin}>
					<p className="border-y border-line bg-hover py-1 pr-3 pl-7 text-xs text-ink-3" title={origin}>
						{originName(origin, servers)}
					</p>
					{items.map((row) => (
						<div key={`${row.source}-${row.origin}-${row.name}`} className={`${NESTED} items-start gap-2 py-2`}>
							<span className="mt-px shrink-0" title={row.state}>
								{row.state === "verified" ? (
									<CheckIcon className="text-accent" />
								) : row.state === "absent" ? (
									<WarningIcon className="text-danger" />
								) : (
									<span className="grid h-4 w-4 place-items-center">
										<span className="h-1.5 w-1.5 rounded-full" style={{ boxShadow: "inset 0 0 0 1.5px var(--ink-4)" }} />
									</span>
								)}
							</span>
							<span className="group-row-text">
								<span className="group-row-title font-mono text-sm">{row.name}</span>
								<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
									{row.reason}
								</span>
							</span>
						</div>
					))}
				</div>
			))}
		</>
	);
}

/** The supplier as a person knows it: Toad Agent, or the server's name from Settings → Tools. */
function originName(origin: string, servers: McpServer[]): string {
	if (origin === "toad") return "Toad Agent";
	return servers.find((server) => server.id === origin)?.name ?? origin;
}

function groupedByOrigin(rows: ToolLedgerRow[]): [string, ToolLedgerRow[]][] {
	const groups = new Map<string, ToolLedgerRow[]>();
	for (const row of rows) {
		const known = groups.get(row.origin);
		if (known) known.push(row);
		else groups.set(row.origin, [row]);
	}
	return [...groups.entries()].map(([origin, items]) => [
		origin,
		items.slice().sort((a, b) => a.name.localeCompare(b.name)),
	]);
}

const THREAD_SEEN_KEY = "toad.threads.seen";

/**
 * This teammate's side conversations, behind one row that counts them.
 * Unread is lastAt against the latest the window has shown, the same way
 * the rail counts a tape.
 */
function Threads({ personaId, onOpen }: { personaId: string; onOpen(thread: OpenThread): void }) {
	const [threads, setThreads] = useState<PeerThreadSummary[] | undefined>(undefined);
	const [seen, setSeen] = useState(loadThreadSeen);
	const [open, setOpen] = useState(false);

	useEffect(() => {
		let cancelled = false;
		setOpen(false);
		void wire
			.command("peers.list", { personaId })
			.then((list) => {
				if (!cancelled) setThreads(list);
			})
			.catch(() => {
				if (!cancelled) setThreads([]);
			});
		return () => {
			cancelled = true;
		};
	}, [personaId]);

	useEffect(() => {
		if (threads === undefined) return;
		setSeen((current) => {
			let changed = false;
			const next = { ...current };
			for (const thread of threads) {
				if (next[thread.threadKey] === undefined) {
					next[thread.threadKey] = thread.lastAt;
					changed = true;
				}
			}
			if (!changed) return current;
			saveThreadSeen(next);
			return next;
		});
	}, [threads]);

	if (threads === undefined) return null;

	const isUnread = (thread: PeerThreadSummary) => thread.lastAt > (seen[thread.threadKey] ?? 0);
	const unread = threads.filter(isUnread).length;

	const openThread = (thread: PeerThreadSummary) => {
		setSeen((current) => {
			const next = { ...current, [thread.threadKey]: thread.lastAt };
			saveThreadSeen(next);
			return next;
		});
		onOpen({
			key: thread.threadKey,
			withName: thread.withName,
		});
	};

	const dot = <span aria-label="Unread" className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />;

	return (
		<section>
			<h3 className="label">Threads</h3>
			<div className="grouped">
				{threads.length === 0 ? (
					<div className="group-row">
						<RowText title="None yet" />
					</div>
				) : (
					<FoldRow
						title={threads.length === 1 ? "1 thread" : `${threads.length} threads`}
						{...(unread > 0 ? { value: `${unread} unread`, mark: dot } : {})}
						open={open}
						onToggle={() => setOpen((was) => !was)}
					/>
				)}
				{open &&
					threads.map((thread) => {
						const line = thread.preview
							? `${thread.preview.fromName}: ${firstLine(thread.preview.text)}`
							: thread.exchanges === 1
								? "1 exchange"
								: `${thread.exchanges} exchanges`;
						return (
							<button
								key={thread.threadKey}
								type="button"
								className={`${NESTED} group-row-choice w-full text-left`}
								onClick={() => openThread(thread)}
							>
								<span className="group-row-text">
									<span className="group-row-title">{thread.withName}</span>
									<span className="group-row-detail">{line}</span>
								</span>
								{isUnread(thread) && dot}
								<span className="shrink-0 text-xs text-ink-3">{threadStamp(thread.lastAt)}</span>
							</button>
						);
					})}
			</div>
		</section>
	);
}

function loadThreadSeen(): Record<string, number> {
	try {
		const raw = localStorage.getItem(THREAD_SEEN_KEY);
		if (!raw) return {};
		const parsed: unknown = JSON.parse(raw);
		if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return {};
		const seen: Record<string, number> = {};
		for (const [key, ts] of Object.entries(parsed)) {
			if (typeof ts === "number" && Number.isFinite(ts)) seen[key] = ts;
		}
		return seen;
	} catch {
		return {};
	}
}

function saveThreadSeen(seen: Record<string, number>): void {
	try {
		localStorage.setItem(THREAD_SEEN_KEY, JSON.stringify(seen));
	} catch {
		// Quota, private mode.
	}
}

const threadClock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const threadDay = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

function threadStamp(at: number): string {
	const when = new Date(at);
	const today = new Date();
	const sameDay =
		when.getFullYear() === today.getFullYear() &&
		when.getMonth() === today.getMonth() &&
		when.getDate() === today.getDate();
	return sameDay ? threadClock.format(when) : threadDay.format(when);
}
