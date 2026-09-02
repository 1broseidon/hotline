import { useEffect, useState } from "react";
import type {
	McpPolicy,
	Persona,
	PolicyMode,
	ScheduledJob,
	TeammateToolLedger,
	ToolLedgerRow,
} from "../generated/contract";
import { CheckIcon, CloseIcon, RevealIcon, WarningIcon } from "../icons";
import { mcpServerDetail, useMcpServers, type McpServer } from "../mcp";
import { revealPath } from "../native";
import { Band } from "../ui/Band";
import { Picker } from "../ui/Menu";
import { wire } from "../wire";
import { PathField } from "./PathField";
import { Schedules } from "./Schedules";

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
}: {
	persona: Persona;
	jobs: ScheduledJob[];
	focusSchedules: boolean;
	onClose(): void;
	onDeleted(): void;
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
				<button type="button" className="control btn-icon" title="Close (Esc)" aria-label="Close" onClick={onClose}>
					<CloseIcon />
				</button>
			</Band>
			<div className="pane-scroll">
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

					<McpGrant
						policy={persona.mcpPolicy}
						servers={servers}
						disabled={busy}
						onChange={(mcpPolicy) => save({ mcpPolicy })}
					/>

					<ToolLedger personaId={persona.id} />

					<Schedules personaId={persona.id} jobs={jobs} focus={focusSchedules} />

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
			</div>
		</aside>
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
function ToolLedger({ personaId }: { personaId: string }) {
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
							<p className="border-b border-line bg-hover px-3 py-1 font-mono text-xs text-ink-3">{origin}</p>
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
