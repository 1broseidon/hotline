import { useCallback, useEffect, useState, type KeyboardEvent } from "react";
import { type Capacity, CPU_STEP, MEMORY_STEP_MB, capacityNote, cpuCeiling, cpuLabel, memoryCeilingMb, memoryLabel, memoryMb, memorySpelling, snap } from "../../computer-limits";
import type { ComputerMount, ComputerStatus, PersonaComputer } from "../../generated/contract";
import { CloseIcon, PlusIcon } from "../../icons";
import { COMPUTER_STATUS_EVERY_MS } from "../../computer";
import { wire } from "../../wire";
import { CookieImport, CookieImports } from "../CookieImport";
import { PasskeyArm } from "../PasskeyArm";
import { ComputerSecrets } from "../Secrets";
import { PathField } from "../PathField";
import { COMPUTER_ABOUT, FoldRow, InfoKey, NESTED, RowText, SwitchRow } from "./rows";

const STATE_WORDS: Record<ComputerStatus["state"], { value: string; action: string }> = {
	running: { value: "running", action: "Stopping keeps the container for the next start." },
	stopped: { value: "stopped", action: "Kept for the next start. Removing it starts over." },
	absent: { value: "no container yet", action: "Built the first time this teammate starts." },
};

const UPDATE_ABOUT =
	"Updating recreates the computer on the newer release. The workspace, prepared environments, job history and browser profile live on volumes and come back with it; anything installed into the container itself outside them is gone, and running jobs stop.";

const LIMITS_ABOUT = "Blank is 1024 processes; 0 is unlimited. A parallel build needs more.";

/** The settings without one of them, so blank means absent on the wire rather than an empty string. */
function without(computer: PersonaComputer, key: "image" | "memory" | "cpus" | "pids" | "mounts" | "secrets"): PersonaComputer {
	const next = { ...computer };
	delete next[key];
	return next;
}

/**
 * The teammate's computer: the switch, and under it, while there is a
 * desktop to speak of, three folds. Desktop carries the machine's state and
 * opens to what is done with it — Stop, Remove, Update — and what the
 * container is built with: image, memory, process limit, the host folders
 * bound in. Browser is what its browser signs in with: the cookies brought
 * over from the person's own browsers, and the passkeys made for the
 * teammate under an arming started here. Secrets is the stored names, one
 * tick each. Every change spreads the settings it does not touch, so
 * flipping the switch never drops a mount. Blank fields are absent on the
 * wire, which is the default; a value the runtime rejects surfaces as a
 * start failure, not here. The status is a peek every few seconds while
 * the pane is open — asking never wakes anything, so the row can be honest
 * about a desktop that stopped on its own. Opening the desktop lives in
 * the conversation's band, which is on screen whenever it runs.
 */
export function ComputerRows({
	personaId,
	teammate,
	computer,
	disabled,
	onChange,
}: {
	personaId: string;
	teammate: string;
	computer: PersonaComputer | undefined;
	disabled: boolean;
	onChange(computer: PersonaComputer): void;
}) {
	const current: PersonaComputer = computer ?? { enabled: false };
	const enabled = current.enabled;
	const mounts = current.mounts ?? [];
	const granted = current.secrets ?? [];
	const [image, setImage] = useState(current.image ?? "");
	const [memory, setMemory] = useState(() => memoryMb(current.memory));
	const [cpus, setCpus] = useState<number | null>(current.cpus ?? null);
	const [capacity, setCapacity] = useState<Capacity | null>(null);
	const [pids, setPids] = useState(current.pids === undefined ? "" : String(current.pids));
	const [status, setStatus] = useState<ComputerStatus | null>(null);
	const [openDesktop, setOpenDesktop] = useState(false);
	const [openBrowser, setOpenBrowser] = useState(false);
	const [openSecrets, setOpenSecrets] = useState(false);
	const [adding, setAdding] = useState(false);
	const [importing, setImporting] = useState(false);
	const [importsVersion, setImportsVersion] = useState(0);
	const [acting, setActing] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [updateTold, setUpdateTold] = useState(false);
	const [checking, setChecking] = useState(false);
	const [checked, setChecked] = useState<string | null>(null);

	useEffect(() => {
		setImage(current.image ?? "");
	}, [current.image]);
	useEffect(() => {
		setMemory(memoryMb(current.memory));
	}, [current.memory]);
	useEffect(() => {
		setCpus(current.cpus ?? null);
	}, [current.cpus]);
	// What the machine has to give, so the sliders stop there. Asked when the
	// grant is on; the desk caches its answer, so asking again is cheap.
	useEffect(() => {
		if (!enabled) return;
		let gone = false;
		void wire
			.command("computer.capacity", {})
			.then((seen) => {
				if (!gone) setCapacity(seen);
			})
			.catch(() => {
				if (!gone) setCapacity(null);
			});
		return () => {
			gone = true;
		};
	}, [enabled]);
	useEffect(() => {
		setPids(current.pids === undefined ? "" : String(current.pids));
	}, [current.pids]);

	// The ticks re-read themselves when the grant changes, and a passkey the
	// room stored is ticked by the room, so a stored passkey needs nothing
	// more from here; the callback is stable so the row's pick-up runs once.
	const stored = useCallback(() => {}, []);

	useEffect(() => {
		setOpenDesktop(false);
		setOpenBrowser(false);
		setOpenSecrets(false);
		setAdding(false);
		setImporting(false);
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
		// Without the grant, one look is enough to find a computer left from
		// before it was turned off; every tick costs the desk a runtime probe.
		const timer = enabled ? setInterval(ask, COMPUTER_STATUS_EVERY_MS) : undefined;
		return () => {
			gone = true;
			clearInterval(timer);
		};
	}, [personaId, enabled]);

	// A slider saves where it is let go, and only when it moved.
	const commitMemory = () => {
		if (memory === memoryMb(current.memory)) return;
		onChange({ ...current, memory: memorySpelling(memory) });
	};
	const commitCpus = () => {
		if (cpus === (current.cpus ?? null)) return;
		onChange(cpus === null ? without(current, "cpus") : { ...current, cpus });
	};

	// One step past the most CPUs the machine has is no cap at all.
	const cpuTop = capacity ? cpuCeiling(capacity) : Math.max(cpus ?? 0, 1);

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

	// The desk asks the release endpoint on its own every few hours; this
	// asks now, and re-reads the status so an offer to update appears at
	// once rather than on the next poll.
	const checkReleases = async () => {
		if (checking) return;
		setChecking(true);
		try {
			const releases = await wire.command("computer.releases.check", {});
			const seen = await wire.command("computer.status", { personaId });
			setStatus(seen);
			const when = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
			setChecked(
				releases.error !== undefined
					? releases.error
					: seen.available !== undefined
						? `${seen.available} is available · checked ${when}`
						: `Up to date · checked ${when}`,
			);
		} catch (error) {
			setChecked(error instanceof Error ? error.message : String(error));
		} finally {
			setChecking(false);
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
			<SwitchRow title="A computer of its own" about={COMPUTER_ABOUT} checked={current.enabled} disabled={disabled} onChange={(on) => onChange({ ...current, enabled: on })} />
			{(current.enabled || state !== "absent") && (
				<>
					<FoldRow title="Desktop" value={words.value} open={openDesktop} onToggle={() => setOpenDesktop((was) => !was)} />
					{openDesktop && (
						<>
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
							<div className={NESTED}>
								<RowText title="Updates" value={checking ? "Checking…" : (checked ?? `Running ${status?.release ?? "an unknown release"}`)} />
								<button type="button" className="control btn-quiet btn-sm" disabled={checking || disabled} onClick={() => void checkReleases()}>
									{checking ? "Checking…" : "Check now"}
								</button>
							</div>
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
									{status.updateFailed !== undefined && (
										<div className={NESTED}>
											<span className="group-row-text">
												<span className="group-row-detail text-danger" style={{ whiteSpace: "normal" }}>
													{`The update did not finish, so the computer keeps ${status.release ?? "the release it has"}: ${status.updateFailed}`}
												</span>
											</span>
										</div>
									)}
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
							<SliderRow
								id="edit-computer-memory"
								title="Memory"
								label={memoryLabel(memory)}
								min={MEMORY_STEP_MB}
								max={capacity ? Math.max(memoryCeilingMb(capacity), memory) : Math.max(16 * 1024, memory)}
								step={MEMORY_STEP_MB}
								value={memory}
								disabled={disabled}
								onChange={(value) => setMemory(snap(value, MEMORY_STEP_MB, MEMORY_STEP_MB, capacity ? Math.max(memoryCeilingMb(capacity), memory) : Math.max(16 * 1024, memory)))}
								onCommit={commitMemory}
							/>
							<SliderRow
								id="edit-computer-cpus"
								title="CPU"
								label={cpuLabel(cpus, capacity)}
								min={CPU_STEP}
								max={cpuTop + CPU_STEP}
								step={CPU_STEP}
								value={cpus ?? cpuTop + CPU_STEP}
								disabled={disabled || !capacity}
								onChange={(value) => {
									const at = snap(value, CPU_STEP, CPU_STEP, cpuTop + CPU_STEP);
									setCpus(at > cpuTop ? null : at);
								}}
								onCommit={commitCpus}
							/>
							{capacity && capacityNote(capacity) && (
								<div className={NESTED}>
									<span className="group-row-text">
										<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
											{capacityNote(capacity)}
										</span>
									</span>
								</div>
							)}
							<LimitRow id="edit-computer-pids" title="Processes" about={LIMITS_ABOUT} placeholder="1024" numeric disabled={disabled} value={pids} onChange={setPids} onCommit={commitPids} />
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
						</>
					)}
					<FoldRow title="Browser" value="cookies and passkeys" open={openBrowser} onToggle={() => setOpenBrowser((was) => !was)} />
					{openBrowser && (
						<>
							{importing ? (
								<CookieImport
									personaId={personaId}
									running={state === "running"}
									onClose={() => {
										setImporting(false);
										setImportsVersion((version) => version + 1);
									}}
								/>
							) : (
								<button type="button" className={`${NESTED} group-row-add`} disabled={disabled} onClick={() => setImporting(true)}>
									<PlusIcon />
									Bring over browser cookies
								</button>
							)}
							<CookieImports personaId={personaId} disabled={disabled} refresh={importsVersion} />
							<PasskeyArm personaId={personaId} teammate={teammate} disabled={disabled} onStored={stored} />
						</>
					)}
					<FoldRow
						title="Secrets"
						value={granted.length === 0 ? "none ticked" : `${granted.length} ticked`}
						open={openSecrets}
						onToggle={() => setOpenSecrets((was) => !was)}
					/>
					{openSecrets && (
						<ComputerSecrets
							granted={granted}
							disabled={disabled}
							onChange={(next) => onChange(next.length === 0 ? without(current, "secrets") : { ...current, secrets: next })}
						/>
					)}
				</>
			)}
		</>
	);
}

/**
 * A limit on a slider: its name, what it is set to, and the track. It saves
 * where it is let go, by pointer or by keyboard, so a drag is one change.
 */
function SliderRow({
	id,
	title,
	label,
	min,
	max,
	step,
	value,
	disabled,
	onChange,
	onCommit,
}: {
	id: string;
	title: string;
	label: string;
	min: number;
	max: number;
	step: number;
	value: number;
	disabled: boolean;
	onChange(value: number): void;
	onCommit(): void;
}) {
	return (
		<div className={`${NESTED} flex-wrap gap-y-1`}>
			<label className="group-row-text" htmlFor={id}>
				<span className="group-row-title">{title}</span>
			</label>
			<span className="font-mono text-sm tabular-nums" aria-hidden>
				{label}
			</span>
			<input
				id={id}
				type="range"
				className="w-full"
				style={{ accentColor: "var(--accent)" }}
				min={min}
				max={max}
				step={step}
				value={value}
				aria-valuetext={label}
				disabled={disabled}
				onChange={(event) => onChange(Number(event.target.value))}
				onPointerUp={onCommit}
				onKeyUp={onCommit}
				onBlur={onCommit}
			/>
		</div>
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
 * be absolute; whether it exists or collides with one of Hotline's is the
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
