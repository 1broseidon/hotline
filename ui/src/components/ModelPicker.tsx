import { type CSSProperties, Fragment, type KeyboardEvent, type ReactNode, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { ChoiceAccount, ConfigChoice, CredentialKind, PlanUsage } from "../generated/contract";
import { planAccount } from "../plan";
import { ChevronDownIcon, ChevronRightIcon, CheckIcon, FastIcon, PlusIcon, SwapIcon } from "../icons";
import { harnessMark, Mark, modelMark, providerMark } from "../ui/Mark";
import type { MenuEntry } from "../ui/Menu";
import { accountName, plainModel } from "../accounts";

/**
 * One row of the Change agent list: an account of Hotline Agent and the
 * models it serves, or another agent (`agent`, its backend id) and its models.
 */
type AccountRow = {
	key: string;
	name: string;
	detail: string;
	mark: string | null;
	choices: ConfigChoice[];
	agent?: string;
	/** Why an agent cannot be picked: it is not installed or not signed in. */
	unavailable?: string;
	/** The agent wants its own sign-in first: its row is a Sign in key. */
	signIn?: boolean;
};

/**
 * Another agent a teammate can run on: a harness, with the models it last
 * reported, and `signIn` when it last asked to be signed in first (new on
 * this desk, or its sign-in expired).
 */
export type AgentChoice = { id: string; name: string; unavailable?: string; signIn?: boolean; choices: ConfigChoice[] };

/** The row that gathers the registry's other ready agents, each a choice. */
const MORE_AGENTS = "__more__";

/** The agents people know by name: each stands on its own row, ready or not. */
const MAJOR_AGENTS = ["antigravity-acp", "claude-acp", "codex-acp", "cursor", "grok-build"];

/** How many of the agent's models the menu lists before More models. */
const SHOWN_MODELS = 8;

type Effort = { value: string; choices: ConfigChoice[] };

/**
 * Fast mode, where the model offers it: whether it is on, a few words on
 * what it costs, and why the agent cannot turn it on now, when it says.
 */
export type Fast = { on: boolean; hint: string; note?: string | undefined };

/** The panels that open beside the menu: every model, or the other agents. */
type Flyout = "more" | "agents";

/** How long a choice the desk has answered waits for the teammate's row to say it. */
const HELD_MS = 4000;

/** One of the teammate's own actions, under what it runs on. */
type Action = Extract<MenuEntry, { kind: "item" }>;

/**
 * The teammate the button is the menu of: its face ends the button, beating
 * while it works, and heads the menu over its own actions (Settings, New
 * chapter, Threads, Remove), each a rule apart as the entries say.
 */
export type MenuTeammate = {
	name: string;
	goal: string;
	face(size: number): ReactNode;
	working: boolean;
	actions: MenuEntry[];
};

/**
 * A row of the menu itself: the effort tabs, one of this agent's models,
 * More models, Change agent, or one of the teammate's actions (`rule`: a
 * rule stands above it).
 */
type MainRow =
	| { kind: "efforts" }
	| { kind: "fast" }
	| { kind: "model"; choice: ConfigChoice }
	| { kind: "more" }
	| { kind: "agents" }
	| { kind: "action"; action: Action; rule: boolean };

const KIND: Record<CredentialKind, string> = { oauth: "subscription", api_key: "API key", local: "this Mac" };

/**
 * The accounts the choices come from, in the order the desk lists them. A
 * provider with one account is its name; with two or more each is
 * "Provider · Name". Choices with no account (an agent's own list) are one
 * row named for their group.
 */
export function accountRows(choices: ConfigChoice[]): AccountRow[] {
	const rows: AccountRow[] = [];
	const at = new Map<string, AccountRow>();
	for (const choice of choices) {
		const account: ChoiceAccount | undefined = choice.account ?? undefined;
		const key = account?.id ?? `group:${choice.group ?? ""}`;
		let row = at.get(key);
		if (row === undefined) {
			const provider = choice.group ?? "Models";
			row = {
				key,
				name: account === undefined ? provider : accountName(provider, account.label, account.shared),
				detail: account === undefined ? "" : KIND[account.kind],
				mark: providerMark(provider) ?? modelMark(choice.id, choice.name),
				choices: [],
			};
			at.set(key, row);
			rows.push(row);
		}
		row.choices.push(choice);
	}
	return rows;
}

/** The part of the Change agent list a row stands in: API or Agents. */
function sectionOf(row: AccountRow): "api" | "agents" {
	return row.agent === undefined ? "api" : "agents";
}

/** Which row a model id is under: the one listing it, else the first serving its plain id. */
function rowOf(rows: AccountRow[], modelId: string): AccountRow | undefined {
	return rows.find((row) => row.choices.some((choice) => choice.id === modelId)) ?? rows.find((row) => row.choices.some((choice) => plainModel(choice.id) === modelId));
}

/**
 * What Change agent hints at: the first two ready agents by their first
 * word ("Claude", "Grok"), "API" for the accounts, and an ellipsis when the
 * list holds more.
 */
function changeHint(others: AccountRow[]): string {
	const ready = others.filter((row) => row.agent !== undefined && row.agent !== MORE_AGENTS && row.unavailable === undefined);
	const names = ready.slice(0, 2).map((row) => row.name.replace(/^Google /, "").split(" ")[0]!);
	if (others.some((row) => row.agent === undefined)) names.push("API");
	const more = ready.length > 2 || others.some((row) => row.agent === MORE_AGENTS || row.unavailable !== undefined);
	return names.length === 0 ? "" : `${names.join(", ")}${more ? "…" : ""}`;
}

/**
 * The agent's models with the one in use among the first few: past
 * SHOWN_MODELS the rest wait under More models.
 */
function shownModels(models: ConfigChoice[], inUse: string | null): ConfigChoice[] {
	if (models.length <= SHOWN_MODELS) return models;
	const shown = models.slice(0, SHOWN_MODELS);
	const current = models.find((choice) => choice.id === inUse);
	if (current !== undefined && !shown.includes(current)) shown[SHOWN_MODELS - 1] = current;
	return shown;
}

/** A level as its tab says it: Auto for the agent's own choice, Med for Medium. */
export function tabLabel(level: ConfigChoice, backendId: string): { label: string; title?: string } {
	if (isAuto(level)) return { label: "Auto", title: backendId === "claude-acp" ? "Claude decides" : "The agent decides" };
	const short: Record<string, string> = { Medium: "Med", "Extra high": "Xhigh", Minimal: "Min" };
	const label = short[level.name] ?? level.name;
	return label === level.name ? { label } : { label, title: level.name };
}

/** The level that leaves the effort to the agent: Claude Code's "Default". */
function isAuto(level: ConfigChoice): boolean {
	const id = level.id.toLowerCase();
	return id === "default" || id === "auto";
}

/**
 * A teammate's model, account and effort in one pull-down (task-285), and
 * with `teammate` the teammate's own menu too: one pill, one menu.
 *
 * The pill names the model, with its maker's mark, and the effort after a
 * dot, then the teammate's face; the account is in the tooltip and the
 * menu, never the pill, and the plan is the band's gauge. A `compact` pill
 * is the mark and the face. The menu says who the teammate is, then what
 * it runs on: the agent and its plan's name, the effort levels of the model
 * in use as one row of tabs, and this agent's models, one click each.
 * Choosing a model or a tab keeps the menu open: a model switch shows the
 * new model's tabs at once, dimmed with the new model's check beating
 * until the desk answers with the effort it landed on. Change agent opens
 * every other account and agent: an account opens its models. Then the
 * teammate's own actions.
 */
export function ModelPicker({
	choices,
	modelId,
	backendId = "hotline",
	agents = [],
	effort,
	fast = null,
	label,
	onModel,
	onAgent,
	onEffort,
	onFast,
	landsOn,
	onAddAccount,
	fallbackName,
	knownName,
	onOpen,
	plan,
	teammate,
	compact = false,
}: {
	/** The account's plan, for the plan's name beside the agent. */
	plan?: PlanUsage | undefined;
	/** Whose menu this is as well: the face, the name and goal, and its actions. */
	teammate?: MenuTeammate | undefined;
	/** A narrow band: the pill is the mark and the face. */
	compact?: boolean;
	/** Each time the menu opens, so lists the desk fills in later can be read again. */
	onOpen?: (() => void) | undefined;
	/** What the button says before a harness has named its model: the harness. */
	fallbackName?: string | undefined;
	/** The name the strip last knew this model by, for one no list names now. */
	knownName?: string | undefined;
	/** Hotline Agent's models, each with its account. */
	choices: ConfigChoice[];
	modelId: string;
	/** The agent the teammate runs on now: Hotline Agent, or a harness's id. */
	backendId?: string;
	/** The other agents it could run on (task-287), the one in use with its models. */
	agents?: AgentChoice[];
	/** Moves the teammate to another agent, with a model when one is named. */
	onAgent?: ((backendId: string, modelId: string | undefined) => Promise<void> | void) | undefined;
	/** The current model's efforts and the one in use; none when it has no levels. */
	effort: Effort | null;
	/** Fast mode, when the model in use offers it. */
	fast?: Fast | null;
	/** Turns fast mode on or off. */
	onFast?: ((on: boolean) => Promise<void> | void) | undefined;
	label: string;
	/** Switches the model; answers with the effort the switch landed on, when it says. */
	onModel(modelId: string): Promise<string | null | undefined | void> | void;
	onEffort(value: string): Promise<void> | void;
	/** The effort a switch to this model is likely to land on, shown until the desk answers. */
	landsOn?: ((modelId: string) => string | undefined) | undefined;
	/** Settings › Providers, where accounts are added. */
	onAddAccount?: (() => void) | undefined;
}) {
	const button = useRef<HTMLButtonElement>(null);
	const [open, setOpen] = useState(false);
	// A model switch on its way: the model, whether the desk has answered,
	// and the effort it answered with. A tab chosen, the same way.
	const [switching, setSwitching] = useState<{ id: string; answered: boolean; effort: string | null } | null>(null);
	const [chosen, setChosen] = useState<{ value: string; answered: boolean } | null>(null);
	// Fast mode flipped, until the teammate's row says it.
	const [flipped, setFlipped] = useState<{ on: boolean; answered: boolean } | null>(null);
	const onHotline = backendId === "hotline";
	const accounts = accountRows(choices);
	// The agents people know by name stand on their own, ready or not, in
	// alphabetical order; the registry's many others that are ready share one
	// row, and those not ready are left out. The agent in use always stands
	// on its own.
	const harnesses = (onAgent === undefined ? [] : agents).filter((agent) => agent.id !== "hotline");
	const known = (agent: AgentChoice) => agent.id === backendId || MAJOR_AGENTS.includes(agent.id);
	const others = harnesses.filter((agent) => !known(agent) && agent.unavailable === undefined);
	const agentRows: AccountRow[] = harnesses
		.filter(known)
		.sort((a, b) => a.name.localeCompare(b.name))
		.map((agent) => ({
			key: `agent:${agent.id}`,
			name: agent.name,
			detail: agent.unavailable !== undefined ? "Not ready" : "",
			mark: harnessMark(agent.id),
			// Until it is signed in its list may be stale or empty: the row is the way in.
			choices: agent.signIn === true ? [] : agent.choices,
			agent: agent.id,
			...(agent.unavailable !== undefined ? { unavailable: agent.unavailable } : {}),
			...(agent.signIn === true && agent.unavailable === undefined ? { signIn: true } : {}),
		}));
	if (others.length > 0) {
		agentRows.push({
			key: "agent:more",
			name: "Other agents",
			detail: String(others.length),
			mark: null,
			choices: others.map((agent) => ({ id: agent.id, name: agent.name })),
			agent: MORE_AGENTS,
		});
	}
	const currentRow = onHotline ? rowOf(accounts, modelId) : agentRows.find((row) => row.agent === backendId);
	// Everything but what the teammate runs on: Hotline Agent's accounts
	// (API, straight to a provider), then the other agents.
	const elsewhere = [...accounts, ...agentRows].filter((row) => row !== currentRow);
	const current = currentRow?.choices.find((choice) => choice.id === modelId || plainModel(choice.id) === modelId);
	// A model on an account that is gone still has its name from another account serving it.
	const sameModel = choices.find((choice) => plainModel(choice.id) === plainModel(modelId));
	const modelName = current?.name ?? sameModel?.name ?? knownName ?? (modelId.split("/").slice(1).join("/") || modelId || fallbackName || "Model");
	// The pill says the level as its tab does: Auto for the agent's own choice.
	const effortChoice = effort?.choices.find((choice) => choice.id === effort.value);
	const effortName = effortChoice === undefined ? null : isAuto(effortChoice) ? "Auto" : effortChoice.name;
	const rowLine = currentRow === undefined ? null : `${currentRow.name}${currentRow.detail === "" ? "" : ` · ${currentRow.detail}`}`;
	// An agent that has not named its model yet is already the button's name.
	const accountLine = rowLine === modelName ? null : rowLine;
	const mark = onHotline ? modelMark(modelId, modelName) : (modelMark(modelId, modelName) ?? harnessMark(backendId));
	// This agent's models; one that lists none has the model in use, once named.
	const listed = currentRow?.choices ?? [];
	const models: ConfigChoice[] = listed.length > 0 ? listed : modelId !== "" ? [{ id: modelId, name: modelName }] : [];
	const inUse = current?.id ?? (listed.length === 0 && modelId !== "" ? modelId : null);
	// The head is the agent and its account. Hotline Agent is the account it
	// runs on; a harness is itself, on a subscription when it reports a plan,
	// named when the plan says its name ("Max subscription").
	const head = onHotline
		? { mark: currentRow?.mark ?? mark, name: currentRow?.name ?? "Hotline Agent", kind: currentRow?.detail ?? "" }
		: {
				mark: harnessMark(backendId),
				name: agents.find((agent) => agent.id === backendId)?.name ?? fallbackName ?? "Agent",
				kind: planAccount(plan),
			};

	// A switch is done once the teammate's row says the new model; a tab
	// once it says the level. An answer the row never catches up with lets
	// go after a while, so the menu never sticks on a guess.
	useEffect(() => {
		if (switching === null) return;
		if (switching.answered && switching.id === inUse) {
			setSwitching(null);
			return;
		}
		if (!switching.answered) return;
		const late = setTimeout(() => setSwitching(null), HELD_MS);
		return () => clearTimeout(late);
	}, [switching, inUse]);
	useEffect(() => {
		if (chosen === null) return;
		if (chosen.value === effort?.value) {
			setChosen(null);
			return;
		}
		if (!chosen.answered) return;
		const late = setTimeout(() => setChosen(null), HELD_MS);
		return () => clearTimeout(late);
	}, [chosen, effort?.value]);

	useEffect(() => {
		if (flipped === null) return;
		if (flipped.on === fast?.on) {
			setFlipped(null);
			return;
		}
		if (!flipped.answered) return;
		const late = setTimeout(() => setFlipped(null), HELD_MS);
		return () => clearTimeout(late);
	}, [flipped, fast?.on]);
	// The toggle, while the model in use, or the one being switched to, is
	// not known to lack fast mode.
	const fastOn = flipped?.on ?? fast?.on ?? false;
	const toward = switching === null ? undefined : models.find((choice) => choice.id === switching.id);
	const fastShown = fast !== null && onFast !== undefined && toward?.fast !== false ? { ...fast, on: fastOn } : null;
	const flip = () => {
		if (fastShown === null) return;
		const on = !fastShown.on;
		setFlipped({ on, answered: false });
		void Promise.resolve(onFast?.(on)).then(
			() => setFlipped((was) => (was?.on === on ? { on, answered: true } : was)),
			() => setFlipped((was) => (was?.on === on ? null : was)),
		);
	};

	const switchTo = (id: string) => {
		if (id === (switching?.id ?? inUse)) return;
		setChosen(null);
		setSwitching({ id, answered: false, effort: null });
		const settle = (effort: string | null) => setSwitching((was) => (was?.id === id ? { id, answered: true, effort } : was));
		void Promise.resolve(onModel(id)).then(
			(landed) => settle(typeof landed === "string" ? landed : null),
			() => setSwitching((was) => (was?.id === id ? null : was)),
		);
	};
	// The tabs: the levels of the model in use, or of the one being switched
	// to as its catalogue lists them, at the effort the desk answered, else
	// the one it is likely to land on, else the model's default. A model
	// whose levels only its session says keeps the old tabs, dimmed, until
	// the teammate's row has the new ones.
	const target = switching === null ? undefined : models.find((choice) => choice.id === switching.id)?.efforts;
	const tabLevels = target ?? effort?.choices ?? [];
	const tabValue =
		switching !== null && target !== undefined
			? (switching.effort ?? landsOn?.(switching.id) ?? models.find((choice) => choice.id === switching.id)?.defaultEffort ?? undefined)
			: (chosen?.value ?? effort?.value);
	const tabBusy = switching !== null && (!switching.answered || target === undefined);
	const tabs = tabLevels.length > 1 ? { levels: tabLevels, value: tabValue, busy: tabBusy } : null;
	const chooseTab = (value: string) => {
		if (tabs === null || tabs.busy || value === tabs.value) return;
		setChosen({ value, answered: false });
		void Promise.resolve(onEffort(value)).then(
			() => setChosen((was) => (was?.value === value ? { value, answered: true } : was)),
			() => setChosen((was) => (was?.value === value ? null : was)),
		);
	};

	const close = useCallback(() => setOpen(false), []);
	// A model under the agent in use is a model change; under another agent,
	// a move to that agent (Hotline Agent's accounts included).
	const pick = async (row: AccountRow, id: string) => {
		if (row.agent === MORE_AGENTS) {
			await onAgent?.(id, undefined);
			return;
		}
		const agent = row.agent ?? "hotline";
		// An agent with no list yet is picked by its row, and runs its own model.
		if (agent === backendId && id === "") return;
		if (agent === backendId) await onModel(id);
		else await onAgent?.(agent, id === "" ? undefined : id);
	};
	return (
		<>
			<button
				ref={button}
				type="button"
				className="control model-pill"
				data-compact={compact ? "true" : undefined}
				aria-label={`${teammate === undefined ? label : `${teammate.name}${teammate.working ? ", working" : ""}`}: ${modelName}${effortName === null ? "" : ` · ${effortName}`}${fastShown?.on ? ", fast" : ""}`}
				aria-haspopup="menu"
				aria-expanded={open}
				title={open ? undefined : [teammate?.name ?? null, modelName, accountLine, effortName, fastShown?.on ? "Fast" : null].filter((one) => one !== null).join(" · ")}
				onClick={() => {
					if (!open) onOpen?.();
					setOpen((was) => !was);
				}}
			>
				<Mark id={mark} />
				{!compact && (
					<span className="model-pill-label">
						{modelName}
						{effortName !== null && <span className="text-ink-3"> · {effortName}</span>}
					</span>
				)}
				{fastShown?.on && <FastIcon className="model-pill-fast" />}
				{teammate !== undefined ? (
					<span className="model-pill-face">
						{teammate.face(24)}
						{teammate.working && <span aria-hidden="true" className="model-pill-beat beat" />}
					</span>
				) : (
					<ChevronDownIcon className="model-pill-chevron" />
				)}
			</button>
			{open && button.current && (
				<Panels
					anchor={button.current}
					teammate={teammate}
					head={head}
					backendId={backendId}
					models={models}
					checked={switching?.id ?? inUse}
					beating={switching !== null && !switching.answered ? switching.id : null}
					tabs={tabs}
					fast={fastShown}
					onFast={flip}
					others={elsewhere}
					modelId={modelId}
					onModel={switchTo}
					onEffort={chooseTab}
					onPick={(row, id) => {
						close();
						void pick(row, id).catch(() => {});
					}}
					{...(onAddAccount !== undefined
						? {
								onAddAccount: () => {
									close();
									onAddAccount();
								},
							}
						: {})}
					onClose={close}
				/>
			)}
		</>
	);
}

/** The effort tabs: what Panels draws under the head, when the model has levels. */
type Tabs = { levels: ConfigChoice[]; value: string | undefined; busy: boolean };

function Panels({
	anchor,
	teammate,
	head,
	backendId,
	models,
	checked,
	beating,
	tabs,
	fast,
	onFast,
	others,
	modelId,
	onModel,
	onEffort,
	onPick,
	onAddAccount,
	onClose,
}: {
	anchor: HTMLElement;
	teammate: MenuTeammate | undefined;
	head: { mark: string | null; name: string; kind: string };
	backendId: string;
	/** This agent's models, every one; the menu shows the first few. */
	models: ConfigChoice[];
	/** The model checked: the one in use, or the one being switched to. */
	checked: string | null;
	/** The model a switch is waiting on, its check beating. */
	beating: string | null;
	tabs: Tabs | null;
	/** Fast mode's toggle, under the tabs, when the model offers it. */
	fast: Fast | null;
	onFast(): void;
	/** Every other account and agent, for Change agent. */
	others: AccountRow[];
	modelId: string;
	onModel(id: string): void;
	onEffort(value: string): void;
	onPick(row: AccountRow, id: string): void;
	onAddAccount?: () => void;
	onClose(): void;
}) {
	const main = useRef<HTMLDivElement>(null);
	const side = useRef<HTMLDivElement>(null);
	const sub = useRef<HTMLDivElement>(null);
	const [mainStyle, setMainStyle] = useState<CSSProperties>({ visibility: "hidden" });
	// The open flyout beside the menu, the account open beside Change agent's
	// list, and where each panel sits.
	const [flyout, setFlyout] = useState<{ kind: Flyout; top: number } | null>(null);
	const [account, setAccount] = useState<{ key: string; top: number } | null>(null);
	// Keyboard: which panel has focus and which row in each, and which tab
	// on the effort row. Nothing is lit until the keyboard moves; the check
	// marks the model in use.
	const [level, setLevel] = useState<0 | 1 | 2>(0);
	const [active, setActive] = useState(-1);
	const [sideActive, setSideActive] = useState(0);
	const [subActive, setSubActive] = useState(0);
	const [tabActive, setTabActive] = useState(-1);

	useLayoutEffect(() => {
		const el = main.current;
		if (!el) return;
		const at = anchor.getBoundingClientRect();
		const width = el.offsetWidth;
		const left = Math.min(Math.max(8, at.right - width), window.innerWidth - width - 8);
		const top = at.bottom + 4;
		setMainStyle({ top, left, width, maxHeight: window.innerHeight - top - 8 });
	}, [anchor]);
	const placed = mainStyle.visibility !== "hidden";
	useEffect(() => {
		if (placed) main.current?.focus({ preventScroll: true });
	}, [placed]);

	useEffect(() => {
		const away = (event: MouseEvent) => {
			const target = event.target as Node | null;
			for (const panel of [main, side, sub]) if (panel.current?.contains(target)) return;
			if (anchor.contains(target)) return;
			onClose();
		};
		const gone = () => onClose();
		document.addEventListener("mousedown", away, true);
		window.addEventListener("resize", gone);
		window.addEventListener("blur", gone);
		return () => {
			document.removeEventListener("mousedown", away, true);
			window.removeEventListener("resize", gone);
			window.removeEventListener("blur", gone);
			anchor.focus({ preventScroll: true });
		};
	}, [anchor, onClose]);

	const shown = shownModels(models, checked);
	const changes = others.length > 0 || onAddAccount !== undefined;
	// The teammate's actions, under a rule, and a rule wherever the entries put one.
	const actions: MainRow[] = [];
	let ruled = true;
	for (const entry of teammate?.actions ?? []) {
		if (entry.kind === "rule") ruled = true;
		else if (entry.kind === "item") {
			actions.push({ kind: "action", action: entry, rule: ruled });
			ruled = false;
		}
	}
	const rows: MainRow[] = [
		...(tabs !== null ? [{ kind: "efforts" as const }] : []),
		...(fast !== null ? [{ kind: "fast" as const }] : []),
		...shown.map((choice) => ({ kind: "model" as const, choice })),
		...(models.length > shown.length ? [{ kind: "more" as const }] : []),
		...(changes ? [{ kind: "agents" as const }] : []),
		...actions,
	];
	const firstAgent = others.findIndex((row) => sectionOf(row) === "agents");
	// Change agent's list: every other account and agent, then Add an account.
	const agentList: (AccountRow | "add")[] = [...others, ...(onAddAccount !== undefined ? ["add" as const] : [])];
	const sideCount = flyout?.kind === "more" ? models.length : agentList.length;

	const opens = (row: AccountRow) => row.unavailable === undefined && row.choices.length > 0;
	const openRow = others.find((row) => row.key === account?.key);

	const showFlyout = (kind: Flyout, element: HTMLElement) => {
		if (flyout?.kind === kind) return;
		setFlyout({ kind, top: element.getBoundingClientRect().top });
		setAccount(null);
		setSideActive(Math.max(0, kind === "more" ? models.findIndex((one) => one.id === checked) : 0));
	};
	const hideFlyout = () => {
		setFlyout(null);
		setAccount(null);
	};
	const showAccount = (row: AccountRow, element: HTMLElement) => {
		// An agent that is not ready does not open: its row says why. One
		// with no list yet has nothing to open; its row is the pick.
		if (!opens(row)) {
			setAccount(null);
			return;
		}
		setAccount({ key: row.key, top: element.getBoundingClientRect().top });
	};
	// What a main row opens beside the menu, if anything.
	const flyoutOf = (row: MainRow | undefined): Flyout | null => (row?.kind === "more" ? "more" : row?.kind === "agents" ? "agents" : null);
	const back = () => {
		if (level === 2) setAccount(null);
		else hideFlyout();
		setLevel((was) => Math.max(0, was - 1) as 0 | 1);
	};
	// Change agent's row, picked: an account or agent with models opens
	// them; one with none moves the teammate there.
	const chooseAgent = (entry: AccountRow | "add" | undefined, element: HTMLElement | null, enter: boolean) => {
		if (entry === undefined) return;
		if (entry === "add") {
			if (enter) onAddAccount?.();
			return;
		}
		if (entry.unavailable === undefined && entry.choices.length === 0) {
			if (enter) onPick(entry, "");
		} else if (opens(entry) && element) {
			showAccount(entry, element);
			setLevel(2);
			setSubActive(Math.max(0, entry.choices.findIndex((choice) => choice.id === modelId)));
		}
	};
	const selectedTab = Math.max(0, tabs?.levels.findIndex((one) => one.id === tabs.value) ?? 0);
	// The effort row comes and goes with the model's levels: the lit row
	// stays the row it was.
	const hadTabs = useRef(tabs !== null);
	useLayoutEffect(() => {
		if (hadTabs.current === (tabs !== null)) return;
		hadTabs.current = tabs !== null;
		setActive((was) => (was < 0 ? was : Math.max(0, was + (tabs !== null ? 1 : -1))));
	}, [tabs]);

	const onKey = (event: KeyboardEvent) => {
		const key = event.key;
		if (key === "Escape") {
			event.preventDefault();
			event.stopPropagation();
			if (level > 0) back();
			else onClose();
			return;
		}
		if (key === "Tab") {
			event.preventDefault();
			onClose();
			return;
		}
		const list = level === 0 ? rows.length : level === 1 ? sideCount : (openRow?.choices.length ?? 0);
		const step = (value: number, by: number) => (list === 0 ? 0 : (value + by + list) % list);
		const onTabs = level === 0 && rows[active]?.kind === "efforts" && tabs !== null;
		if (key === "ArrowDown" || key === "ArrowUp") {
			event.preventDefault();
			const by = key === "ArrowDown" ? 1 : -1;
			if (level === 0) {
				const from = Math.max(0, rows.findIndex((row) => row.kind === "model" && row.choice.id === checked));
				const next = active < 0 ? from : step(active, by);
				setActive(next);
				setTabActive(rows[next]?.kind === "efforts" ? selectedTab : -1);
			} else if (level === 1) setSideActive((was) => step(was, by));
			else setSubActive((was) => step(was, by));
			return;
		}
		// On the effort row, ← and → walk the tabs.
		if (onTabs && (key === "ArrowLeft" || key === "ArrowRight")) {
			event.preventDefault();
			const count = tabs.levels.length;
			setTabActive((was) => (Math.max(0, was) + (key === "ArrowRight" ? 1 : -1) + count) % count);
			return;
		}
		if (key === "ArrowLeft") {
			if (level > 0) {
				event.preventDefault();
				back();
			}
			return;
		}
		if (key !== "ArrowRight" && key !== "Enter" && key !== " ") return;
		event.preventDefault();
		const enter = key !== "ArrowRight";
		if (level === 0) {
			const row = rows[active];
			const kind = flyoutOf(row);
			const element = main.current?.querySelector<HTMLElement>(`[data-row="${active}"]`);
			if (onTabs && enter) {
				const value = tabs.levels[tabActive < 0 ? selectedTab : tabActive];
				if (value) onEffort(value.id);
			} else if (kind !== null && element) {
				showFlyout(kind, element);
				setLevel(1);
			} else if (row?.kind === "fast" && enter) onFast();
			else if (row?.kind === "model" && enter) onModel(row.choice.id);
			else if (row?.kind === "action" && enter) act(row.action);
		} else if (level === 1) {
			if (flyout?.kind === "more") {
				const choice = models[sideActive];
				if (choice && enter) onModel(choice.id);
			} else {
				chooseAgent(agentList[sideActive], side.current?.querySelector<HTMLElement>(`[data-row="${sideActive}"]`) ?? null, enter);
			}
		} else {
			const choice = openRow?.choices[subActive];
			if (choice && openRow && enter) onPick(openRow, choice.id);
		}
	};

	// A panel opens beside the one it came from, toward the window's middle.
	const beside = (panel: HTMLElement | null, top: number, width: number): CSSProperties => {
		const box = panel?.getBoundingClientRect();
		if (!box) return { visibility: "hidden" };
		const left = box.left - width - 4 >= 8 ? box.left - width - 4 : box.right + 4;
		return { top: Math.max(8, Math.min(top - 4, window.innerHeight - 160)), left, width };
	};
	const lit = (here: 0 | 1 | 2, at: number, current: number) => (level === here && current === at ? "true" : undefined);
	// An action closes the menu first, as any menu's item does.
	const act = (action: Action) => {
		if (action.disabled) return;
		onClose();
		action.onSelect();
	};
	// A model row: the one checked, its check beating while a switch waits on it.
	const modelRow = (choice: ConfigChoice, at: number, here: 0 | 1, current: number, hovered: () => void) => {
		const isChecked = choice.id === checked;
		return (
			<button
				key={choice.id}
				type="button"
				role="menuitemradio"
				aria-checked={isChecked}
				aria-busy={choice.id === beating ? "true" : undefined}
				data-row={at}
				data-current={isChecked ? "true" : undefined}
				data-active={lit(here, at, current)}
				tabIndex={-1}
				className="menu-item model-row"
				onMouseEnter={hovered}
				onClick={() => onModel(choice.id)}
			>
				<span className={`menu-item-check model-check${choice.id === beating ? " beat" : ""}`} data-beat={choice.id === beating ? "true" : undefined}>
					{isChecked && <CheckIcon />}
				</span>
				<span className="menu-item-text">{choice.name}</span>
			</button>
		);
	};

	return createPortal(
		// biome-ignore lint/a11y/noStaticElementInteractions: the panels share one keyboard handler; each panel is a menu
		<div onKeyDown={onKey}>
			<div
				ref={main}
				role="menu"
				tabIndex={-1}
				aria-label={teammate?.name ?? "Model"}
				className={`menu model-menu model-menu-main${teammate !== undefined ? " model-menu-teammate" : ""}`}
				style={mainStyle}
			>
				{teammate !== undefined && (
					<div className="menu-heading menu-card model-who" role="presentation">
						{teammate.face(30)}
						<span className="menu-card-text">
							<span className="menu-card-name">{teammate.name}</span>
							{teammate.goal !== "" && <span className="menu-card-detail">{teammate.goal}</span>}
						</span>
					</div>
				)}
				<div className="model-head">
					<Mark id={head.mark} size={14} />
					<span className="model-head-name">{head.name}</span>
					{head.kind !== "" && <span className="model-head-kind">{head.kind}</span>}
				</div>
				{rows.map((row, at) => {
					if (row.kind === "efforts" && tabs !== null) {
						return (
							<fieldset
								key="efforts"
								aria-label="Effort"
								data-row={at}
								data-active={lit(0, at, active)}
								data-busy={tabs.busy ? "true" : undefined}
								className="segmented effort-tabs"
							>
								{tabs.levels.map((one, index) => {
									const { label, title } = tabLabel(one, backendId);
									return (
										<button
											key={one.id}
											type="button"
											role="menuitemradio"
											aria-checked={one.id === tabs.value}
											aria-disabled={tabs.busy ? "true" : undefined}
											data-active={level === 0 && active === at && tabActive === index ? "true" : undefined}
											tabIndex={-1}
											className="segment effort-tab"
											title={title}
											onMouseEnter={() => {
												setActive(at);
												setTabActive(index);
												setLevel(0);
												hideFlyout();
											}}
											onClick={() => onEffort(one.id)}
										>
											{label}
										</button>
									);
								})}
							</fieldset>
						);
					}
					if (row.kind === "fast" && fast !== null) {
						return (
							<button
								key="fast"
								type="button"
								role="menuitemcheckbox"
								aria-checked={fast.on}
								data-row={at}
								data-active={lit(0, at, active)}
								tabIndex={-1}
								className="menu-item model-row model-fast"
								title={fast.note}
								onMouseEnter={() => {
									setActive(at);
									setLevel(0);
									hideFlyout();
								}}
								onClick={onFast}
							>
								<FastIcon className="menu-item-check model-fast-icon" />
								<span className="menu-item-text">Fast</span>
								<span className="model-fast-hint">{fast.note !== undefined && !fast.on ? "Unavailable" : fast.hint}</span>
								<span aria-hidden="true" className="switch" data-on={fast.on ? "true" : undefined} />
							</button>
						);
					}
					if (row.kind === "model") {
						return modelRow(row.choice, at, 0, active, () => {
							setActive(at);
							setLevel(0);
							hideFlyout();
						});
					}
					if (row.kind === "action") {
						const { action } = row;
						return (
							<Fragment key={`action:${action.id}`}>
								{row.rule && <hr className="menu-rule" />}
								<button
									type="button"
									role="menuitem"
									data-row={at}
									data-active={lit(0, at, active)}
									aria-disabled={action.disabled ? "true" : undefined}
									tabIndex={-1}
									className={`menu-item model-row${action.danger ? " model-danger" : ""}`}
									onMouseEnter={() => {
										setActive(at);
										setLevel(0);
										hideFlyout();
									}}
									onClick={() => act(action)}
								>
									<span className="menu-item-check model-check">{action.checked && <CheckIcon />}</span>
									<span className="menu-item-text">
										{action.text}
										{action.detail !== undefined && <span className="menu-item-detail">{action.detail}</span>}
									</span>
									{action.shortcut !== undefined && <span className="menu-shortcut">{action.shortcut}</span>}
								</button>
							</Fragment>
						);
					}
					if (row.kind !== "more" && row.kind !== "agents") return null;
					const agents = row.kind === "agents";
					const hint = agents ? changeHint(others) : "";
					return (
						<Fragment key={row.kind}>
							{agents && teammate === undefined && <hr className="menu-rule" />}
							<button
								type="button"
								role="menuitem"
								aria-haspopup="menu"
								aria-expanded={flyout?.kind === row.kind}
								data-row={at}
								data-active={lit(0, at, active) ?? (flyout?.kind === row.kind ? "true" : undefined)}
								tabIndex={-1}
								className={`menu-item model-row ${agents ? "model-change" : "model-more"}`}
								onMouseEnter={(event) => {
									setActive(at);
									setLevel(0);
									showFlyout(row.kind as Flyout, event.currentTarget);
								}}
								onClick={(event) => showFlyout(row.kind as Flyout, event.currentTarget)}
							>
								<span className="menu-item-check">{agents && <SwapIcon className="model-change-mark" />}</span>
								<span className="menu-item-text">{agents ? "Change agent" : "More models"}</span>
								{hint !== "" && <span className="model-change-hint">{hint}</span>}
								<ChevronRightIcon className="model-row-more" />
							</button>
						</Fragment>
					);
				})}
			</div>
			{flyout?.kind === "more" && (
				<div ref={side} role="menu" tabIndex={-1} aria-label={`${head.name} models`} className="menu model-menu" style={beside(main.current, flyout.top, 248)}>
					<div className="menu-heading">{head.name}</div>
					{models.map((choice, at) =>
						modelRow(choice, at, 1, sideActive, () => {
							setSideActive(at);
							setLevel(1);
						}),
					)}
				</div>
			)}
			{flyout?.kind === "agents" && (
				<div ref={side} role="menu" tabIndex={-1} aria-label="Change agent" className="menu model-menu model-menu-agents" style={beside(main.current, flyout.top, 280)}>
					{others.map((row, at) => (
						<Fragment key={row.key}>
							{(at === 0 || at === firstAgent) && (
								<>
									{at > 0 && <hr className="menu-rule" />}
									<div className="menu-heading">{sectionOf(row) === "api" ? "API" : "Agents"}</div>
								</>
							)}
							<button
								type="button"
								role="menuitem"
								aria-haspopup={opens(row) ? "menu" : undefined}
								aria-disabled={row.unavailable !== undefined ? "true" : undefined}
								title={row.unavailable ?? (row.signIn === true ? "Opens its sign-in" : undefined)}
								aria-expanded={opens(row) ? account?.key === row.key : undefined}
								data-row={at}
								data-active={lit(1, at, sideActive) ?? (account?.key === row.key ? "true" : undefined)}
								tabIndex={-1}
								className="menu-item model-row model-account"
								onMouseEnter={(event) => {
									setSideActive(at);
									setLevel(1);
									showAccount(row, event.currentTarget);
								}}
								onClick={(event) => {
									if (row.unavailable === undefined && row.choices.length === 0) onPick(row, "");
									else showAccount(row, event.currentTarget);
								}}
							>
								<span className="menu-item-check" />
								<Mark id={row.mark} />
								<span className="menu-item-text">{row.name}</span>
								{row.signIn === true ? (
									<span className="model-row-signin">Sign in</span>
								) : (
									row.detail !== "" && <span className="model-row-detail">{row.detail}</span>
								)}
								{opens(row) && <ChevronRightIcon className="model-row-more" />}
							</button>
						</Fragment>
					))}
					{onAddAccount !== undefined && (
						<>
							{others.length > 0 && <hr className="menu-rule" />}
							<button
								type="button"
								role="menuitem"
								data-row={others.length}
								data-active={lit(1, others.length, sideActive)}
								tabIndex={-1}
								className="menu-item model-row model-add"
								onMouseEnter={() => {
									setSideActive(others.length);
									setLevel(1);
									setAccount(null);
								}}
								onClick={onAddAccount}
							>
								<span className="menu-item-check" />
								<PlusIcon className="model-add-mark" />
								<span className="menu-item-text">Add an account…</span>
							</button>
						</>
					)}
				</div>
			)}
			{flyout?.kind === "agents" && openRow !== undefined && account !== null && (
				<div ref={sub} role="menu" tabIndex={-1} aria-label={openRow.name} className="menu model-menu" style={beside(side.current, account.top, 248)}>
					<div className="menu-heading">{openRow.name}</div>
					{openRow.choices.map((choice, at) => (
						<button
							key={choice.id}
							type="button"
							role="menuitemradio"
							aria-checked={choice.id === modelId}
							data-row={at}
							data-active={lit(2, at, subActive)}
							tabIndex={-1}
							className="menu-item model-row"
							onMouseEnter={() => {
								setSubActive(at);
								setLevel(2);
							}}
							onClick={() => onPick(openRow, choice.id)}
						>
							<span className="menu-item-check">{choice.id === modelId && <CheckIcon />}</span>
							{/* A harness's own names ("Opus 5.5", "Default") carry the harness's mark. */}
							<Mark id={modelMark(choice.id, choice.name) ?? (openRow.agent === undefined ? null : harnessMark(openRow.agent === MORE_AGENTS ? choice.id : openRow.agent))} />
							<span className="menu-item-text">{choice.name}</span>
						</button>
					))}
				</div>
			)}
		</div>,
		document.body,
	);
}
