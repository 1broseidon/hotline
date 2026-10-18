import { useEffect, useState } from "react";
import type { McpPolicy, PolicyMode, SkillEntry, SkillPolicy } from "../../generated/contract";
import type { McpServer } from "../../mcp";
import { Picker } from "../../ui/Menu";
import { wire } from "../../wire";
import { FoldRow, NESTED, RowText } from "./rows";

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
export function McpRows({
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
	{ id: "all", name: "All", detail: "Every offered skill, including ones offered later" },
];

/**
 * Which of the offered skills — the gateway's, and the person's own that are
 * switched on under Settings → Skills — this teammate is given, drawn like
 * the MCP rows: the row's value is the answer, the fold holds the grant and
 * the ticks. Under them, what the teammate has of its own: the built-ins
 * every teammate carries and the skills it wrote itself, read from its
 * workspace when the fold opens, so a skill it saved during the last turn is
 * there.
 */
export function SkillRows({
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

	// The gateway's valid entries and the person's own switched on, one per
	// name: the gateway lists first, and a name it has is its own.
	const offered = entries
		.filter((one) => one.invalid === undefined && (one.source === "gateway" || (one.source === "home" && one.offered === true)))
		.filter((one, index, all) => all.findIndex((other) => other.name === one.name) === index);
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
							label="Which offered skills this teammate gets"
							disabled={disabled}
							onChange={(mode) => {
								if (mode !== policy.mode) onChange({ ...policy, mode: mode as PolicyMode });
							}}
						/>
					</div>
					{policy.mode === "some" &&
						(offered.length === 0 ? (
							<div className={NESTED}>
								<RowText title="No skills offered yet" value="switch one on under Settings → Skills" />
							</div>
						) : (
							offered.map((entry) => (
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
