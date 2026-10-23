import { useEffect, useState } from "react";
import type { AccountSection, AccountsList, AccountView, BackendChoice, ConfigChoice, Credential, FolderGrant, PersonaDraft, Provider } from "../generated/contract";
import { chordKeys } from "../chords";
import { CloseIcon } from "../icons";
import { useActiveDesk, useDesks } from "../desks";
import { preferredModel, useRoomSettings } from "../room";
import { Band } from "../ui/Band";
import { Picker } from "../ui/Menu";
import { Fold } from "../ui/Fold";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";
import { AccountPicker } from "./accounts/AccountPicker";
import { AccountSignIn, settledAccounts } from "./accounts/AccountSignIn";
import { AddAccountForm } from "./accounts/AddAccountForm";
import { accountsFor, HOTLINE_AGENT, inUse, kindsIn, providerOf } from "./accounts/fits";
import { NEVER_SEES_SIGN_IN, SYSTEM_DEFAULT } from "./accounts/words";
import { BackendPicker } from "./BackendPicker";
import { ConnectProvider } from "./ConnectProvider";
import { PathField } from "./PathField";
import { suggestName } from "../names";
import { BACKGROUND_ABOUT, COMPUTER_ABOUT, FolderRows, MACHINE_ABOUT, SwitchRow, foldersNote } from "./Teammate";

/**
 * Creating a teammate: the things the person decides, and nothing else.
 *
 * A teammate is an identity (`goal`), a workspace (`cwd`), a harness
 * (`backendId`) and — for Hotline Agent only — a disposition (`modelId`) under
 * a name. The harness defaults to the room's `defaultBackendId`. An ACP
 * harness brings its own models once the session is up, so that field is
 * not asked here. The access choices people most often decide up front —
 * the whole machine, background work, a computer, the folders it may also
 * read — are asked too, with the pane's own words; MCP and skill grants stay
 * on the pane.
 *
 * Created, the teammate is opened at once and started — nobody adds a
 * colleague in order to look at them in a list.
 */
export function NewTeammate({
	models,
	onCreated,
	onClose,
	onProviders,
	onDesk,
}: {
	models: ConfigChoice[];
	onCreated(personaId: string): void;
	onClose(): void;
	onProviders?: (() => void) | undefined;
	/** Make it on another desk instead: the pane moves there (BRO-178). */
	onDesk?: ((deskId: string) => void) | undefined;
}) {
	const desks = useDesks();
	const here = useActiveDesk();
	return (
		<div className="pane">
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">New teammate</h2>
				{/* With more than one desk in the window, which one it lives on is the first thing asked. */}
				{desks.length > 1 && here !== null && onDesk !== undefined && (
					<Picker
						value={here.id}
						choices={desks.map((desk) => ({ id: desk.id, name: `On ${desk.name}`, group: desk.kind === "local" ? "On this computer" : "Remote" }))}
						placeholder="Desk"
						label="Desk"
						onChange={(deskId) => {
							if (deskId !== here.id) onDesk(deskId);
						}}
					/>
				)}
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
	const room = useRoomSettings();
	const { defaultBackendId } = room;
	const preferred = preferredModel(room);
	const [name, setName] = useState("");
	const [goal, setGoal] = useState(suggestedGoal);
	const [cwd, setCwd] = useState("");
	const [picked, setPicked] = useState<string | null>(null);
	const [backends, setBackends] = useState<BackendChoice[]>([]);
	const [pickedModel, setPickedModel] = useState<string | null>(null);
	const [machine, setMachine] = useState(false);
	const [backgroundWork, setBackgroundWork] = useState(false);
	const [computer, setComputer] = useState(false);
	const [folders, setFolders] = useState<FolderGrant[]>([]);
	// A computer is offered only where one can start.
	const [computerReady, setComputerReady] = useState(false);
	// A picture is offered only where the room can draw one: the provider that would.
	const [imagesBy, setImagesBy] = useState<string | null>(null);
	const [picture, setPicture] = useState(true);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [accountsList, setAccountsList] = useState<AccountsList | null>(null);
	const [providers, setProviders] = useState<Provider[]>([]);
	const [accountSettling, setAccountSettling] = useState<{ accountId: string } | null>(null);
	const [accountId, setAccountId] = useState(SYSTEM_DEFAULT);
	const [addingAccount, setAddingAccount] = useState<AccountSection | null>(null);
	const [signingAccount, setSigningAccount] = useState<{ view: AccountView; email: string | undefined } | null>(null);
	const [accountFromDesk, setAccountFromDesk] = useState<string | null>(null);
	const modelId = pickedModel ?? preferred ?? "";
	// A blank draft runs on what the room prefers, else the first choice.
	const fallback = models.find((one) => one.id === preferred) ?? models[0];

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
			.then(({ images }) => setImagesBy(images.selected?.providerId === "off" ? null : ((images.selected ?? images.automatic)?.providerName ?? null)))
			.catch(() => setImagesBy(null));
	}, []);

	const available = (id: string) => backends.some((one) => one.id === id && one.unavailable === undefined);
	const backendId =
		picked ??
		(available(defaultBackendId)
			? defaultBackendId
			: (backends.find((one) => one.unavailable === undefined)?.id ?? ""));
	const onHotline = backendId === HOTLINE_AGENT || backendId === "";
	// With no model sent, the core checks a choice against the room's preferred one.
	const accountModelId = modelId || preferred;
	const accountProvider = onHotline ? providerOf(accountModelId) : undefined;

	// Read again once each sign-in here has ended.
	useEffect(() => {
		if (simple) return;
		let cancelled = false;
		const read =
			accountSettling === null ? wire.command("accounts.list", {}) : settledAccounts(accountSettling.accountId, () => !cancelled);
		void read.then(
			(answer) => {
				if (!cancelled && answer !== null) setAccountsList(answer);
			},
			() => {
				if (!cancelled) setAccountsList(null);
			},
		);
		return () => {
			cancelled = true;
		};
	}, [simple, accountSettling]);

	useEffect(() => {
		if (simple) return;
		void wire.command("providers.list", {}).then(setProviders, () => setProviders([]));
	}, [simple]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: a change of agent, or of the model's provider, is what resets the choice.
	useEffect(() => {
		setAccountId(SYSTEM_DEFAULT);
		setAddingAccount(null);
		setSigningAccount(null);
		setAccountFromDesk(null);
	}, [backendId, accountProvider]);

	const agent = onHotline ? undefined : accountsList?.agents.find((one) => one.backendId === backendId && one.supportsAccounts);
	const provider = providers.find((one) => one.id === accountProvider);
	const accountChoices =
		accountsList === null
			? []
			: accountsFor(accountsList, onHotline ? { backendId: HOTLINE_AGENT, modelId: accountModelId } : { backendId, modelId: undefined });
	const accountNames = accountChoices.filter((one) => one.id !== SYSTEM_DEFAULT).map((one) => one.name);
	const selectedAccount = inUse(accountChoices, accountId);
	const pickedAccount = accountChoices.find((one) => one.id === selectedAccount);
	const draftAccount = pickedAccount !== undefined && pickedAccount.id !== SYSTEM_DEFAULT && !pickedAccount.isDefault ? pickedAccount : undefined;
	const signingId = signingAccount?.view.account.id;
	const shownChoices = accountChoices.map((one) => (one.id === signingId ? { ...one, detail: "Signing in…" } : one));
	const readAccounts = () => wire.command("accounts.list", {}).then(setAccountsList, () => {});
	const addAccount = (section: AccountSection) => () => {
		setAccountFromDesk(null);
		setAddingAccount(section);
	};
	const accountAdds = onHotline
		? {
				subscription: kindsIn(provider, "subscription").length > 0 ? addAccount("subscription") : undefined,
				apiKey: kindsIn(provider, "api_key").length > 0 ? addAccount("api_key") : undefined,
			}
		: { subscription: signingAccount === null ? addAccount("subscription") : undefined };
	const credentialAdded = async (made: Credential) => {
		await readAccounts();
		setAccountId(made.id);
		setAddingAccount(null);
	};
	// A sign-in cancelled late may still have landed.
	const credentialCancelled = () => {
		setAddingAccount(null);
		void readAccounts();
	};
	const accountAdded = (made: AccountView, email: string | undefined) => {
		setAddingAccount(null);
		setAccountId(made.account.id);
		setAccountsList((was) => was && { ...was, accounts: [...was.accounts.filter((one) => one.account.id !== made.account.id), made] });
		if (made.canSignInHere) setSigningAccount({ view: made, email });
		else setAccountFromDesk(made.account.id);
	};
	const accountSignInEnded = () => {
		if (signingAccount !== null) setAccountSettling({ accountId: signingAccount.view.account.id });
		setSigningAccount(null);
	};

	const submit = async () => {
		const trimmed = name.trim();
		if (!trimmed || busy || signingAccount !== null) return;
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
		if (folders.length > 0) draft.folders = folders;
		if (draftAccount !== undefined) draft.accountId = draftAccount.id;
		try {
			const persona = await wire.command("persona.create", { draft });
			// Opened as soon as it exists, not once it has started: a start
			// takes seconds, and the roster already shows the teammate, so
			// waiting left the window on no one. A start that fails says so in
			// the conversation, as an error notice and a session to retry.
			onCreated(persona.id);
			void wire.command("session.start", { personaId: persona.id }).catch((error: Error) => console.warn("session.start", error.message));
			// Drawn while they settle in: the initial shows until the roster
			// brings the picture, and a refusal leaves the initial in place.
			if (imagesBy !== null && picture) {
				void wire.command("avatar.generate", { personaId: persona.id }).catch((error: Error) => console.warn("avatar.generate", error.message));
			}
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	const modelName = models.find((one) => one.id === modelId)?.name ?? fallback?.name ?? "The best one available";
	const backendName = backends.find((one) => one.id === backendId)?.name ?? "Hotline Agent";
	const accountNote = draftAccount === undefined ? "" : ` · ${draftAccount.name}`;
	const thinks = onHotline ? `Hotline Agent · ${modelName}${accountNote}` : `${backendName}${accountNote}`;
	const abilities = [
		onHotline && machine ? "Whole machine" : "Its folder only",
		backgroundWork ? "Works while you're away" : null,
		computerReady && computer ? "Own computer" : null,
	]
		.filter(Boolean)
		.join(" · ");
	const editable = folders.filter((folder) => folder.writable).length;
	const where = [
		cwd.trim() || "A new folder of its own",
		folders.length > 0 ? ` + ${folders.length} more` : "",
		editable > 0 ? ` (${editable} can edit)` : "",
	].join("");
	const [open, setOpen] = useState<"thinks" | "folder" | "abilities" | null>(null);
	const fold = (which: "thinks" | "folder" | "abilities") => setOpen((was) => (was === which ? null : which));

	// Shown only when there is a choice: one account that fits is nothing to pick.
	const accountField = accountChoices.length >= 2 && (
		<div>
			<p className="label" id="new-account">
				Account
			</p>
			<AccountPicker choices={shownChoices} selected={selectedAccount} label="Account" disabled={busy} onSelect={setAccountId} adds={accountAdds} />
			{agent !== undefined && (signingAccount !== null || accountFromDesk !== null || addingAccount !== null) && (
				<div className="grouped mt-2">
					{signingAccount !== null && (
						<div className="group-row flex-col items-stretch">
							<AccountSignIn
								account={signingAccount.view}
								agentName={agent.name}
								email={signingAccount.email}
								cancelLabel="Sign in later"
								onEnd={accountSignInEnded}
							/>
						</div>
					)}
					{accountFromDesk !== null && signingAccount === null && <p className="group-row text-sm text-ink-3">Sign in from the desk itself</p>}
					{addingAccount !== null && (
						<AddAccountForm
							compact
							backendId={agent.backendId}
							agentName={agent.name}
							takesEmail={agent.takesEmail}
							taken={accountNames}
							onAdded={accountAdded}
							onRefused={() => void readAccounts()}
							onCancel={() => setAddingAccount(null)}
						/>
					)}
				</div>
			)}
			{agent !== undefined && (signingAccount !== null || addingAccount !== null) && <p className="hint">{NEVER_SEES_SIGN_IN}</p>}
			{onHotline && addingAccount !== null && provider !== undefined && (
				<div className="mt-3">
					<ConnectProvider
						key={addingAccount}
						provider={provider}
						kinds={kindsIn(provider, addingAccount)}
						taken={accountNames}
						onConnected={(made) => void credentialAdded(made)}
						onCancel={credentialCancelled}
					/>
				</div>
			)}
			<p className="hint">Rename or remove accounts later in Settings › Providers.</p>
		</div>
	);

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
								// biome-ignore lint/a11y/noAutofocus: the pane opens only when New teammate is asked for, and the name is the first thing it asks.
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
					{!onHotline && accountField}
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
					{onHotline && accountField}
				</Fold>
				<Fold title="Folders" value={where} open={open === "folder"} onToggle={() => fold("folder")}>
					<PathField
						id="new-cwd"
						value={cwd}
						placeholder={simple ? "A new folder of its own, unless you pick one" : "A folder under the data directory, unless you pick one"}
						onChange={setCwd}
					/>
					<p className="hint">
						{simple
							? "Where it keeps its work. It only touches files here unless you give it the whole machine."
							: "Where it keeps its work. It only touches files here and in the folders below, unless you give it the whole machine."}
					</p>
					{!simple && (
						<div>
							<p className="label">Also let it read</p>
							<div className="grouped">
								<FolderRows folders={folders} disabled={busy} onChange={setFolders} />
							</div>
							{folders.length > 0 && (
								<p className="hint">{foldersNote({ hotline: onHotline, machine, takesFolders: true, computer: computerReady && computer })}</p>
							)}
						</div>
					)}
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
				<button type="submit" className="control btn btn-primary nt-submit" disabled={busy || name.trim() === "" || signingAccount !== null}>
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
