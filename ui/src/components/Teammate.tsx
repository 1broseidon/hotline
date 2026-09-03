import { useEffect, useState } from "react";
import type {
	ComputerStatus,
	McpPolicy,
	PeerThreadSummary,
	Persona,
	PersonaComputer,
	PolicyMode,
	ScheduledJob,
	TeammateToolLedger,
	ToolLedgerRow,
} from "../generated/contract";
import { chordKeys } from "../chords";
import { CheckIcon, CloseIcon, RevealIcon, WarningIcon } from "../icons";
import { mcpServerDetail, useMcpServers, type McpServer } from "../mcp";
import { openLink, revealPath } from "../native";
import { firstLine } from "../room";
import { Band } from "../ui/Band";
import { Picker } from "../ui/Menu";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";
import { PathField } from "./PathField";
import { Schedules } from "./Schedules";
import type { OpenThread } from "./Thread";

/**
 * One teammate, beside their conversation: the four things the person
 * decides, the tools they actually have, the jobs that will wake them, and
 * the way out.
 *
 * Name, goal and working directory are the identity and the wall. Reach is
 * the one policy — the working directory, or the whole machine — and it is
 * a switch because those are the only two answers. Every field saves when
 * you leave it. Removing asks for the name typed back, so a misfire does
 * not take a colleague with it.
 */
export function Teammate({
	persona,
	jobs,
	focusSchedules,
	onClose,
	onDeleted,
	onOpenThread,
}: {
	persona: Persona;
	jobs: ScheduledJob[];
	focusSchedules: boolean;
	onClose(): void;
	onDeleted(): void;
	onOpenThread(thread: OpenThread): void;
}) {
	const servers = useMcpServers();
	const [name, setName] = useState(persona.name);
	const [goal, setGoal] = useState(persona.goal);
	const [cwd, setCwd] = useState(persona.cwd);
	const [confirm, setConfirm] = useState("");
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	useEffect(() => {
		setName(persona.name);
		setGoal(persona.goal);
		setCwd(persona.cwd);
	}, [persona.name, persona.goal, persona.cwd]);

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

	const saveCwd = (next = cwd) => {
		const trimmed = next.trim();
		if (!trimmed || trimmed === persona.cwd) {
			setCwd(persona.cwd);
			return;
		}
		save({ cwd: trimmed });
	};

	const remove = async () => {
		if (confirm !== persona.name || busy) return;
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

	const machine = persona.reach === "machine";

	return (
		<aside className="inspector" aria-label={`${persona.name}'s settings`}>
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">{persona.name}</h2>
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
					<CloseIcon />
				</button>
			</Band>
			<Scroll>
				<div className="flex flex-col gap-5 px-4 py-4">
					<form
						className="flex flex-col gap-4"
						onSubmit={(event) => {
							event.preventDefault();
							saveName();
							saveGoal();
							saveCwd();
						}}
					>
						<div>
							<label className="label" htmlFor="edit-name">
								Name
							</label>
							<input
								id="edit-name"
								className="field"
								value={name}
								autoComplete="off"
								onChange={(event) => setName(event.target.value)}
								onBlur={saveName}
							/>
						</div>

						<div>
							<label className="label" htmlFor="edit-goal">
								Goal
							</label>
							<textarea
								id="edit-goal"
								className="field"
								rows={4}
								placeholder="What this teammate is for."
								value={goal}
								onChange={(event) => setGoal(event.target.value)}
								onBlur={saveGoal}
							/>
						</div>

						<div>
							<div className="mb-1 flex items-center justify-between">
								<label className="label mb-0" htmlFor="edit-cwd">
									Working directory
								</label>
								<button
									type="button"
									className="control btn-quiet btn-sm -mr-2 gap-1"
									title="Reveal in the file manager"
									onClick={() => void revealPath(persona.cwd)}
								>
									<RevealIcon />
									Reveal
								</button>
							</div>
							<PathField id="edit-cwd" value={cwd} onChange={setCwd} onCommit={(value) => saveCwd(value)} />
						</div>
					</form>

					<section>
						<h3 className="label">Reach</h3>
						<div className="grouped">
							<label className="group-row group-row-choice">
								<span className="group-row-text">
									<span className="group-row-title">Whole machine</span>
									<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
										{machine
											? "Tools can touch the rest of the machine. The working directory is where they start, not a wall."
											: "Off: tools stop at the working directory. Nothing outside it is read, changed or run."}
									</span>
								</span>
								<input
									type="checkbox"
									className="switch"
									checked={machine}
									disabled={busy}
									onChange={(event) => save({ reach: event.target.checked ? "machine" : "workspace" })}
								/>
							</label>
						</div>
					</section>

					<ComputerSection
						personaId={persona.id}
						computer={persona.computer}
						disabled={busy}
						onChange={(computer) => save({ computer })}
					/>

					<McpGrant
						policy={persona.mcpPolicy}
						servers={servers}
						disabled={busy}
						onChange={(mcpPolicy) => save({ mcpPolicy })}
					/>

					<ToolLedger personaId={persona.id} servers={servers} />

					<Schedules personaId={persona.id} jobs={jobs} focus={focusSchedules} />

					<Threads personaId={persona.id} onOpen={onOpenThread} />

					<section className="border-t border-line pt-4">
						<h3 className="label">Remove teammate</h3>
						<p className="hint mt-0 mb-2">
							Type <span className="font-medium text-ink-2">{persona.name}</span> to confirm. Their conversation goes too.
						</p>
						<div className="flex items-center gap-2">
							<input
								id="edit-confirm"
								className="field min-w-0 flex-1"
								aria-label="Type the teammate's name to confirm removal"
								placeholder={persona.name}
								autoComplete="off"
								value={confirm}
								onChange={(event) => setConfirm(event.target.value)}
								onKeyDown={(event) => {
									if (event.key === "Enter") {
										event.preventDefault();
										void remove();
									}
								}}
							/>
							<button
								type="button"
								className="control btn btn-danger"
								disabled={busy || confirm !== persona.name}
								onClick={() => void remove()}
							>
								Remove
							</button>
						</div>
					</section>

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

/** How often the pane asks after the container while it is open. */
const COMPUTER_STATUS_EVERY_MS = 5000;

const STATE_WORDS: Record<ComputerStatus["state"], { title: string; detail: string }> = {
	running: { title: "Running", detail: "The desktop is up. Stopping it keeps the container for the next start." },
	stopped: { title: "Stopped", detail: "The container is kept and wakes on the next start. Removing it starts over." },
	absent: { title: "No container yet", detail: "One is built the first time this teammate starts." },
};

/**
 * The teammate's computer: the switch, the image it wakes with, and what
 * the container is doing now. The status is a peek every few seconds while
 * the pane is open — asking never wakes anything, so the line can be
 * honest about a desktop that stopped on its own. Stop and Remove act on
 * the container, not the teammate; the switch is what the next start reads.
 */
function ComputerSection({
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
	const enabled = computer?.enabled ?? false;
	const image = computer?.image ?? "";
	const [draft, setDraft] = useState(image);
	const [status, setStatus] = useState<ComputerStatus | null>(null);
	const [acting, setActing] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	useEffect(() => {
		setDraft(image);
	}, [image]);

	useEffect(() => {
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

	const commitImage = () => {
		const trimmed = draft.trim();
		setDraft(trimmed);
		if (trimmed === image) return;
		onChange(trimmed === "" ? { enabled } : { enabled, image: trimmed });
	};

	const act = async (cmd: "computer.stop" | "computer.remove") => {
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
	const viewer = status?.state === "running" ? status.viewer : undefined;

	return (
		<section>
			<h3 className="label">Computer</h3>
			<div className="grouped">
				<label className="group-row group-row-choice">
					<span className="group-row-text">
						<span className="group-row-title">A desktop of its own</span>
						<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
							{enabled
								? "A Linux desktop in a container, with tools to drive it. It wakes when the teammate starts."
								: "Off: the teammate works on this machine alone, with no desktop to drive."}
						</span>
					</span>
					<input
						type="checkbox"
						className="switch"
						checked={enabled}
						disabled={disabled}
						onChange={(event) =>
							onChange(image === "" ? { enabled: event.target.checked } : { enabled: event.target.checked, image })
						}
					/>
				</label>
				{enabled && (
					<div className="group-row">
						<label className="group-row-text" htmlFor="edit-computer-image">
							<span className="group-row-title">Image</span>
							<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
								Blank takes the room&rsquo;s, under Settings → Computer.
							</span>
						</label>
						<input
							id="edit-computer-image"
							className="field w-40 min-w-0 font-mono text-sm"
							placeholder="Room default"
							autoComplete="off"
							spellCheck={false}
							disabled={disabled}
							value={draft}
							onChange={(event) => setDraft(event.target.value)}
							onBlur={commitImage}
							onKeyDown={(event) => {
								if (event.key !== "Enter") return;
								event.preventDefault();
								commitImage();
							}}
						/>
					</div>
				)}
			</div>
			<p className="hint">A change reaches the teammate on its next start.</p>
			{(enabled || state !== "absent") && (
				<div className="grouped mt-2">
					<div className="group-row">
						<span className="group-row-text">
							<span className="group-row-title">{words.title}</span>
							<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
								{words.detail}
							</span>
						</span>
						{viewer !== undefined && (
							<button type="button" className="control btn" onClick={() => void openLink(viewer)}>
								Open desktop
							</button>
						)}
						{state === "running" && (
							<button type="button" className="control btn-quiet" disabled={acting} onClick={() => void act("computer.stop")}>
								Stop
							</button>
						)}
						{state === "stopped" && (
							<button
								type="button"
								className="control btn-quiet"
								disabled={acting}
								onClick={() => void act("computer.remove")}
							>
								Remove
							</button>
						)}
					</div>
				</div>
			)}
			{refusal !== null && (
				<p role="status" className="selectable mt-2 text-sm text-danger">
					{refusal}
				</p>
			)}
		</section>
	);
}

const GRANT_MODES: { id: PolicyMode; name: string; detail: string }[] = [
	{ id: "all", name: "Every server", detail: "Whatever the room has, now and later" },
	{ id: "none", name: "None", detail: "Only the agent's own tools" },
	{ id: "some", name: "Some", detail: "Only the servers ticked below" },
];

/**
 * Which of the app's servers this teammate is given. The list is kept when
 * the mode is not `some`, so toggling back does not lose the ticks.
 */
function McpGrant({
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
	const toggle = (id: string) => {
		const serverIds = policy.serverIds.includes(id)
			? policy.serverIds.filter((item) => item !== id)
			: [...policy.serverIds, id];
		onChange({ ...policy, serverIds });
	};

	return (
		<section>
			<h3 className="label">MCP servers</h3>
			<Picker
				field
				value={policy.mode}
				choices={GRANT_MODES}
				placeholder="Grant"
				label="Which MCP servers this teammate gets"
				disabled={disabled}
				onChange={(mode) => {
					if (mode !== policy.mode) onChange({ ...policy, mode: mode as PolicyMode });
				}}
			/>
			{policy.mode === "some" &&
				(servers.length === 0 ? (
					<p className="hint">No servers yet. Add one under Settings → Tools.</p>
				) : (
					<div className="grouped mt-2">
						{servers.map((server) => (
							<label key={server.id} className="group-row group-row-choice">
								<input
									type="checkbox"
									className="check"
									checked={policy.serverIds.includes(server.id)}
									disabled={disabled}
									onChange={() => toggle(server.id)}
								/>
								<span className="group-row-text">
									<span className="group-row-title">{server.name}</span>
									<span className="group-row-detail font-mono">{mcpServerDetail(server)}</span>
								</span>
							</label>
						))}
					</div>
				))}
			<p className="hint">A change reaches the teammate on its next start.</p>
		</section>
	);
}

/**
 * What this teammate actually has. The grant above is the intent; this is
 * the outcome of the last start, read once when the pane opens because a
 * ledger is a fact of that start, not a live feed.
 */
function ToolLedger({ personaId, servers }: { personaId: string; servers: McpServer[] }) {
	const [ledger, setLedger] = useState<TeammateToolLedger | null | undefined>(undefined);

	useEffect(() => {
		let cancelled = false;
		setLedger(undefined);
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
	}, [personaId]);

	if (ledger === undefined) return null;

	return (
		<section>
			<h3 className="label">Tools</h3>
			{ledger === null ? (
				<p className="hint mt-0">Tools attach when the session starts. Nothing has started yet.</p>
			) : (
				<div className="grouped">
					{groupedByOrigin(ledger.rows).map(([origin, rows]) => (
						<div key={origin}>
							<p className="border-b border-line bg-hover px-3 py-1 text-xs text-ink-3" title={origin}>
								{originName(origin, servers)}
							</p>
							{rows.map((row) => (
								<div key={`${row.source}-${row.origin}-${row.name}`} className="group-row items-start gap-2 py-2">
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
										<span className="group-row-title font-mono text-sm">
											{row.name}
											<span className="ml-2 font-sans text-xs text-ink-3">{row.state}</span>
										</span>
										<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
											{row.reason}
										</span>
									</span>
								</div>
							))}
						</div>
					))}
				</div>
			)}
		</section>
	);
}

/** The supplier as a person knows it: Toad Agent, or the server's name from Settings → Tools. */
function originName(origin: string, servers: McpServer[]): string {
	if (origin === "pi") return "Toad Agent";
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
 * This teammate's side conversations. Unread is lastAt against the latest
 * the window has shown, the same way the rail counts a tape.
 */
function Threads({ personaId, onOpen }: { personaId: string; onOpen(thread: OpenThread): void }) {
	const [threads, setThreads] = useState<PeerThreadSummary[] | undefined>(undefined);
	const [seen, setSeen] = useState(loadThreadSeen);

	useEffect(() => {
		let cancelled = false;
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

	const open = (thread: PeerThreadSummary) => {
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

	return (
		<section>
			<h3 className="label">Threads</h3>
			{threads.length === 0 ? (
				<p className="hint mt-0">Nothing yet. Threads appear when this teammate talks to another one.</p>
			) : (
				<div className="grouped">
					{threads.map((thread) => {
						const unread = thread.lastAt > (seen[thread.threadKey] ?? 0);
						const line = thread.preview
							? `${thread.preview.fromName}: ${firstLine(thread.preview.text)}`
							: thread.exchanges === 1
								? "1 exchange"
								: `${thread.exchanges} exchanges`;
						return (
							<button
								key={thread.threadKey}
								type="button"
								className="group-row group-row-choice w-full text-left"
								onClick={() => open(thread)}
							>
								<span className="group-row-text">
									<span className="group-row-title">{thread.withName}</span>
									<span className="group-row-detail">{line}</span>
								</span>
								{unread && (
									<span
										aria-label="Unread"
										className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent"
									/>
								)}
								<span className="shrink-0 text-xs text-ink-3">{threadStamp(thread.lastAt)}</span>
							</button>
						);
					})}
				</div>
			)}
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
