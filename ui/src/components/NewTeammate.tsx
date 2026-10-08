import { useEffect, useState } from "react";
import type { BackendChoice, ConfigChoice, PersonaDraft } from "../generated/contract";
import { chordKeys } from "../chords";
import { CloseIcon } from "../icons";
import { useRoomSettings } from "../room";
import { Band } from "../ui/Band";
import { Picker } from "../ui/Menu";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";
import { BackendPicker } from "./BackendPicker";
import { PathField } from "./PathField";
import { suggestName } from "../names";
import { BACKGROUND_ABOUT, COMPUTER_ABOUT, MACHINE_ABOUT, SwitchRow } from "./Teammate";

/** Hotline Agent's stored backend id. Any other id is an ACP harness. */
const HOTLINE_AGENT = "hotline";

/**
 * Creating a teammate: the things the person decides, and nothing else.
 *
 * A teammate is an identity (`goal`), a workspace (`cwd`), a harness
 * (`backendId`) and — for Hotline Agent only — a disposition (`modelId`) under
 * a name. The harness defaults to the room's `defaultBackendId`. An ACP
 * harness brings its own models once the session is up, so that field is
 * not asked here. The access choices people most often decide up front —
 * the whole machine, background work, a computer — are asked too, with the
 * pane's own words; MCP and skill grants stay on the pane.
 *
 * Created, the teammate is started at once and opened — nobody adds a
 * colleague in order to look at them in a list.
 */
export function NewTeammate({
	models,
	onCreated,
	onClose,
	onProviders,
}: {
	models: ConfigChoice[];
	onCreated(personaId: string): void;
	onClose(): void;
	onProviders?: (() => void) | undefined;
}) {
	return (
		<div className="pane">
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">New teammate</h2>
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
					<CloseIcon />
				</button>
			</Band>
			<Scroll>
				<NewTeammateForm className="pane-column" models={models} onCreated={onCreated} onCancel={onClose} onProviders={onProviders} />
			</Scroll>
		</div>
	);
}

/**
 * The form itself, without the pane around it: the new-teammate pane and
 * the welcome pane's second step both render this one, so the first
 * teammate is made the way every later one is. `goal` is a starting point
 * the person can keep or replace; the welcome pane suggests one, the pane
 * from the plus suggests nothing.
 */
export function NewTeammateForm({
	models,
	goal: suggestedGoal = "",
	submitLabel = "Add teammate",
	className,
	onCreated,
	onCancel,
	onProviders,
	simple = false,
}: {
	models: ConfigChoice[];
	goal?: string;
	submitLabel?: string;
	className?: string;
	onCreated(personaId: string): void;
	onCancel?: () => void;
	onProviders?: (() => void) | undefined;
	/** The welcome's first teammate: what it runs on was just chosen, and access keeps its safe defaults, so neither is asked again. */
	simple?: boolean;
}) {
	const { defaultBackendId, defaultModelId, lastModelId } = useRoomSettings();
	const [name, setName] = useState("");
	const [goal, setGoal] = useState(suggestedGoal);
	const [cwd, setCwd] = useState("");
	const [picked, setPicked] = useState<string | null>(null);
	const [backends, setBackends] = useState<BackendChoice[]>([]);
	const [pickedModel, setPickedModel] = useState<string | null>(null);
	const [machine, setMachine] = useState(false);
	const [backgroundWork, setBackgroundWork] = useState(false);
	const [computer, setComputer] = useState(false);
	// A computer is offered only where one can start.
	const [computerReady, setComputerReady] = useState(false);
	// A picture is offered only where the room can draw one: the provider that would.
	const [imagesBy, setImagesBy] = useState<string | null>(null);
	const [picture, setPicture] = useState(true);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const modelId = pickedModel ?? defaultModelId ?? lastModelId ?? "";
	// A blank draft runs on what the room prefers, else the first choice.
	const fallback = models.find((one) => one.id === (defaultModelId ?? lastModelId)) ?? models[0];

	useEffect(() => {
		void wire
			.command("backends.list", {})
			.then(setBackends)
			.catch((error: Error) => setRefusal(error.message));
		void wire
			.command("computer.runtimes", {})
			.then((reports) => setComputerReady(reports.some((one) => one.state === "ready")))
			.catch(() => setComputerReady(false));
		void wire
			.command("capabilities.options", {})
			.then(({ images }) => setImagesBy((images.selected ?? images.automatic)?.providerName ?? null))
			.catch(() => setImagesBy(null));
	}, []);

	const available = (id: string) => backends.some((one) => one.id === id && one.unavailable === undefined);
	const backendId =
		picked ??
		(available(defaultBackendId)
			? defaultBackendId
			: (backends.find((one) => one.unavailable === undefined)?.id ?? ""));
	const onHotline = backendId === HOTLINE_AGENT || backendId === "";

	const submit = async () => {
		const trimmed = name.trim();
		if (!trimmed || busy) return;
		setBusy(true);
		setRefusal(null);
		const draft: PersonaDraft = { name: trimmed };
		if (goal.trim()) draft.goal = goal.trim();
		if (cwd.trim()) draft.cwd = cwd.trim();
		if (backendId) draft.backendId = backendId;
		if (onHotline && modelId) draft.modelId = modelId;
		if (onHotline && machine) draft.reach = "machine";
		if (backgroundWork) draft.backgroundWork = true;
		if (computerReady && computer) draft.computer = { enabled: true };
		try {
			const persona = await wire.command("persona.create", { draft });
			await wire.command("session.start", { personaId: persona.id });
			// Drawn while they settle in: the initial shows until the roster
			// brings the picture, and a refusal leaves the initial in place.
			if (imagesBy !== null && picture) {
				void wire.command("avatar.generate", { personaId: persona.id }).catch((error: Error) => console.warn("avatar.generate", error.message));
			}
			onCreated(persona.id);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	const modelName = models.find((one) => one.id === modelId)?.name ?? fallback?.name ?? "The best one available";
	const backendName = backends.find((one) => one.id === backendId)?.name ?? "Hotline Agent";
	const thinks = onHotline ? `Hotline Agent · ${modelName}` : backendName;
	const abilities = [
		onHotline && machine ? "Whole machine" : "Its folder only",
		backgroundWork ? "Works while you're away" : null,
		computerReady && computer ? "Own computer" : null,
	]
		.filter(Boolean)
		.join(" · ");
	const [open, setOpen] = useState<"thinks" | "folder" | "abilities" | null>(null);
	const fold = (which: "thinks" | "folder" | "abilities") => setOpen((was) => (was === which ? null : which));

	return (
		<form
			className={`${className ?? ""} new-teammate flex flex-col gap-4`}
			onSubmit={(event) => {
				event.preventDefault();
				void submit();
			}}
		>
			{!simple && <p className="text-ink-2">An AI helper with a name and a job. Everything else starts with a good default.</p>}

			<section className="nt-card">
				<div className="flex items-center gap-3">
					<span className="nt-avatar" aria-hidden="true">
						{name.trim().charAt(0).toUpperCase() || "?"}
					</span>
					<div className="min-w-0 flex-1">
						<label className="label" htmlFor="new-name">
							Name
						</label>
						<div className="relative">
							<input
								id="new-name"
								className="field pr-9"
								value={name}
								autoFocus
								autoComplete="off"
								placeholder="Name your teammate"
								onChange={(event) => setName(event.target.value)}
							/>
							<button
								type="button"
								className="nt-dice"
								title="Suggest a name"
								aria-label="Suggest a name"
								onClick={() => setName((was) => suggestName(was))}
							>
								<DiceGlyph />
							</button>
						</div>
					</div>
				</div>
				<div>
					<label className="label" htmlFor="new-goal">
						Goal
					</label>
					<textarea
						id="new-goal"
						className="field"
						rows={3}
						placeholder="What this teammate is for."
						value={goal}
						onChange={(event) => setGoal(event.target.value)}
					/>
					<div className="mt-2 flex flex-wrap items-center gap-1.5">
						<span className="text-sm text-ink-3">Start from</span>
						{GOALS.map((one) => (
							<button
								key={one.label}
								type="button"
								className="nt-chip"
								data-on={goal === one.goal ? "" : undefined}
								onClick={() => setGoal(one.goal)}
							>
								{one.label}
							</button>
						))}
					</div>
					<p className="hint">
						{simple
							? "Its job, in your own words. It reads this every time it starts."
							: "Its job description, written into its folder as AGENTS.md. It reads it every time it starts."}
					</p>
				</div>
			</section>

			<section className="nt-folds">
				<Fold title="Thinks with" value={thinks} open={open === "thinks"} onToggle={() => fold("thinks")}>
					{backends.length > 0 && !simple && (
						<div>
							<p className="label" id="new-backend">
								Agent
							</p>
							<BackendPicker
								backends={backends}
								selected={backendId}
								name="new-backend"
								labelledBy="new-backend"
								onSelect={setPicked}
								onProviders={onProviders}
							/>
							{!onHotline && (
								<p className="hint">
									Permissions are managed by this external harness. Selecting it trusts its tools and configuration; Hotline's shell sandbox does not
									confine it.
								</p>
							)}
						</div>
					)}
					{onHotline && (
						<div>
							<p className="label" id="new-model">
								Model
							</p>
							<Picker
								field
								value={modelId}
								choices={[{ id: "", name: fallback === undefined ? "The best one available" : `${fallback.name} — the default` }, ...models]}
								placeholder="Model"
								label="Model"
								onChange={setPickedModel}
							/>
						</div>
					)}
				</Fold>
				<Fold title="Folder" value={cwd.trim() || "A new folder of its own"} open={open === "folder"} onToggle={() => fold("folder")}>
					<PathField
						id="new-cwd"
						value={cwd}
						placeholder={simple ? "A new folder of its own, unless you pick one" : "A folder under the data directory, unless you pick one"}
						onChange={setCwd}
					/>
					<p className="hint">Where it keeps its work. It only touches files here unless you give it the whole machine.</p>
				</Fold>
				{!simple && (
					<Fold title="Abilities" value={abilities} open={open === "abilities"} onToggle={() => fold("abilities")}>
						<div className="grouped">
							{onHotline && <SwitchRow title="Whole machine" about={MACHINE_ABOUT} checked={machine} disabled={busy} onChange={setMachine} />}
							<SwitchRow title="Background work" about={BACKGROUND_ABOUT} checked={backgroundWork} disabled={busy} onChange={setBackgroundWork} />
							{computerReady && <SwitchRow title="Computer" about={COMPUTER_ABOUT} checked={computer} disabled={busy} onChange={setComputer} />}
						</div>
						<p className="hint">These can always be changed later on the teammate's pane.</p>
					</Fold>
				)}
				{imagesBy !== null && (
					<label className="nt-fold-row nt-fold-head">
						<span className="nt-fold-title">Picture</span>
						<span className="nt-fold-value">{picture ? `Drawn from its name and goal with ${imagesBy}` : "Its initial"}</span>
						<input type="checkbox" className="switch" checked={picture} disabled={busy} onChange={(event) => setPicture(event.target.checked)} />
					</label>
				)}
			</section>

			{refusal !== null && (
				<p role="status" className="selectable text-sm text-danger">
					{refusal}
				</p>
			)}

			<div className="flex items-center justify-end gap-2">
				{onCancel !== undefined && (
					<button type="button" className="control btn-quiet" onClick={onCancel}>
						Cancel
					</button>
				)}
				<button type="submit" className="control btn btn-primary nt-submit" disabled={busy || name.trim() === ""}>
					{busy ? "Setting up…" : submitLabel}
				</button>
			</div>
		</form>
	);
}

function DiceGlyph() {
	return (
		<svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinejoin="round" aria-hidden="true">
			<rect x="2.25" y="2.25" width="11.5" height="11.5" rx="2.5" />
			<circle cx="5.5" cy="5.5" r="0.9" fill="currentColor" stroke="none" />
			<circle cx="10.5" cy="5.5" r="0.9" fill="currentColor" stroke="none" />
			<circle cx="8" cy="8" r="0.9" fill="currentColor" stroke="none" />
			<circle cx="5.5" cy="10.5" r="0.9" fill="currentColor" stroke="none" />
			<circle cx="10.5" cy="10.5" r="0.9" fill="currentColor" stroke="none" />
		</svg>
	);
}

/** Goals to start from, for a person who has not written one for an agent before. Each is a whole job, in plain words. */
const GOALS: { label: string; goal: string }[] = [
	{
		label: "Anything",
		goal: "Help me with whatever I bring you. Ask when something is unclear, and say what you did when you are done.",
	},
	{
		label: "Code",
		goal: "Build and fix code in this folder. Run the tests before saying something works, and explain what you changed.",
	},
	{
		label: "Research",
		goal: "Research what I ask about. Find good sources, compare them, and give me a short answer with links.",
	},
	{
		label: "Writing",
		goal: "Help me write and edit. Keep my voice, make it clear and short, and suggest what to cut.",
	},
];

/** A choice that has a good default: one line saying what it is now, opened only to change it. */
function Fold({
	title,
	value,
	open,
	onToggle,
	children,
}: {
	title: string;
	value: string;
	open: boolean;
	onToggle(): void;
	children: React.ReactNode;
}) {
	return (
		<div className="nt-fold" data-open={open ? "" : undefined}>
			<button type="button" className="nt-fold-row nt-fold-head" aria-expanded={open} onClick={onToggle}>
				<span className="nt-fold-title">{title}</span>
				<span className="nt-fold-value">{value}</span>
				<span className="nt-fold-action">{open ? "Done" : "Change"}</span>
			</button>
			{open && <div className="nt-fold-body">{children}</div>}
		</div>
	);
}
