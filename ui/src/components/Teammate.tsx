import { useEffect, useState } from "react";
import type {
	McpPolicy,
	Persona,
	PolicyMode,
	ScheduledJob,
	TeammateToolLedger,
	ToolLedgerRow,
} from "../generated/contract";
import { RevealIcon } from "../icons";
import { mcpServerDetail, useMcpServers, type McpServer } from "../mcp";
import { revealPath } from "../native";
import { wire } from "../wire";
import { PathField } from "./PathField";
import { Schedules } from "./Schedules";

/**
 * Editing a teammate: the four things the person decides, the jobs that
 * will wake them, and the way out.
 *
 * Name, goal and working directory are the identity and the wall. Reach is
 * the one policy — the working directory, or the whole machine — and it is a
 * toggle because those are the only two answers. Deleting asks for the name
 * typed back, so a misfire does not take a colleague with it.
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
		if (goal === persona.goal) return;
		save({ goal });
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
		<div className="flex flex-col gap-3">
			<form
				className="flex flex-col gap-3"
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
						className="field resize-none"
						rows={3}
						placeholder="What this teammate is for."
						value={goal}
						onChange={(event) => setGoal(event.target.value)}
						onBlur={saveGoal}
					/>
				</div>

				<div>
					<label className="label" htmlFor="edit-cwd">
						Working directory
					</label>
					<PathField id="edit-cwd" value={cwd} onChange={setCwd} onCommit={(value) => saveCwd(value)} />
					<div className="mt-2">
						<button
							type="button"
							className="btn-quiet inline-flex items-center gap-1.5"
							onClick={() => void revealPath(persona.cwd)}
						>
							<RevealIcon />
							Reveal workspace
						</button>
					</div>
				</div>

				<div>
					<p className="label">Reach</p>
					<label className="flex items-center gap-2 text-sm text-ink-2">
						<input
							type="checkbox"
							checked={machine}
							disabled={busy}
							onChange={(event) =>
								// A missing key leaves the old reach. The generated
								// patch is Partial<Persona>, so the wall is the
								// word, not JSON null.
								save({ reach: event.target.checked ? "machine" : "workspace" })
							}
						/>
						Whole machine
					</label>
					<p className="mt-1 text-xs leading-relaxed text-ink-3">
						{machine
							? "Tools can touch the rest of the machine. The working directory is where they start, not a wall."
							: "Tools stop at the working directory: nothing outside it can be read, changed, or run."}
					</p>
				</div>

				<McpGrant
					personaId={persona.id}
					policy={persona.mcpPolicy}
					servers={servers}
					disabled={busy}
					onChange={(mcpPolicy) => save({ mcpPolicy })}
				/>

				<ToolLedger personaId={persona.id} />
			</form>

			<Schedules personaId={persona.id} jobs={jobs} focus={focusSchedules} />

			<section className="mt-2 border-t border-rule pt-4">
				<p className="label">Remove teammate</p>
				<p className="mb-2 text-xs leading-relaxed text-ink-3">
					Type <span className="font-medium text-ink-2">{persona.name}</span> to confirm. Their
					conversation goes too.
				</p>
				<input
					id="edit-confirm"
					className="field"
					aria-label="Type the teammate's name to confirm removal"
					value={confirm}
					onChange={(event) => setConfirm(event.target.value)}
					onKeyDown={(event) => {
						if (event.key === "Enter") {
							event.preventDefault();
							void remove();
						}
					}}
				/>
				<div className="mt-3 flex justify-end">
					<button
						type="button"
						className="btn-quiet text-[var(--danger)]"
						disabled={busy || confirm !== persona.name}
						onClick={() => void remove()}
					>
						Remove teammate
					</button>
				</div>
			</section>

			{refusal !== null && <p className="text-xs text-[var(--danger)]">{refusal}</p>}

			<div className="mt-1 flex justify-end">
				<button type="button" className="btn-quiet" onClick={onClose}>
					Done
				</button>
			</div>
		</div>
	);
}

const GRANT_MODES: { id: PolicyMode; label: string }[] = [
	{ id: "all", label: "Every server" },
	{ id: "none", label: "None" },
	{ id: "some", label: "Some" },
];

/**
 * Which of the app's servers this teammate is given. The list is kept when
 * the mode is not `some`, so toggling back does not lose the ticks.
 */
function McpGrant({
	personaId,
	policy,
	servers,
	disabled,
	onChange,
}: {
	personaId: string;
	policy: McpPolicy;
	servers: McpServer[];
	disabled: boolean;
	onChange(policy: McpPolicy): void;
}) {
	const setMode = (mode: PolicyMode) => {
		if (mode === policy.mode) return;
		onChange({ ...policy, mode });
	};

	const toggle = (id: string) => {
		const serverIds = policy.serverIds.includes(id)
			? policy.serverIds.filter((item) => item !== id)
			: [...policy.serverIds, id];
		onChange({ ...policy, serverIds });
	};

	return (
		<div>
			<p className="label">MCP servers</p>
			<div className="flex flex-col gap-1.5">
				{GRANT_MODES.map((mode) => (
					<label key={mode.id} className="flex items-center gap-2 text-sm text-ink-2">
						<input
							type="radio"
							name={`mcp-policy-${personaId}`}
							checked={policy.mode === mode.id}
							disabled={disabled}
							onChange={() => setMode(mode.id)}
						/>
						{mode.label}
					</label>
				))}
			</div>
			{policy.mode === "some" &&
				(servers.length === 0 ? (
					<p className="mt-2 text-xs leading-relaxed text-ink-3">
						No servers yet. Add one under Settings → Tools.
					</p>
				) : (
					<ul className="mt-2 flex flex-col">
						{servers.map((server) => (
							<li key={server.id}>
								<label className="flex items-start gap-2 border-b border-rule py-1.5 text-xs">
									<input
										type="checkbox"
										className="mt-0.5"
										checked={policy.serverIds.includes(server.id)}
										disabled={disabled}
										onChange={() => toggle(server.id)}
									/>
									<span className="min-w-0 flex-1">
										<span className="font-medium text-ink-2">{server.name}</span>
										<span className="ml-2 text-ink-3">{server.type}</span>
										<span className="block truncate font-mono text-ink-3">
											{mcpServerDetail(server)}
										</span>
									</span>
								</label>
							</li>
						))}
					</ul>
				))}
			<p className="mt-1 text-xs leading-relaxed text-ink-3">A change reaches the teammate on its next start.</p>
		</div>
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
		<div>
			<p className="label">Tools</p>
			{ledger === null ? (
				<p className="text-xs leading-relaxed text-ink-3">Tools attach at start.</p>
			) : (
				<div className="flex flex-col gap-3">
					{groupedByOrigin(ledger.rows).map(([origin, rows]) => (
						<div key={origin}>
							<p className="mb-1 font-mono text-xs text-ink-3">{origin}</p>
							<ul className="flex flex-col">
								{rows.map((row) => (
									<li key={`${row.source}-${row.origin}-${row.name}`} className="border-b border-rule py-1.5">
										<p className="flex flex-wrap items-baseline gap-x-2 text-xs">
											<span className="font-mono text-ink">{row.name}</span>
											<span className="text-ink-3">{row.state}</span>
										</p>
										<p className="text-xs leading-relaxed text-ink-3">{row.reason}</p>
									</li>
								))}
							</ul>
						</div>
					))}
				</div>
			)}
		</div>
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
