import { type CSSProperties, Fragment, type KeyboardEvent, type ReactNode, type RefObject, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { ChoiceAccount, ConfigChoice, CredentialKind, PlanUsage } from "../generated/contract";
import { nearLimit, paceTick, planAccount, resetShort, shownWindows, windowHover, windowName } from "../plan";
import { ChevronDownIcon, ChevronRightIcon, CheckIcon, PlusIcon, SwapIcon } from "../icons";
import { harnessMark, Mark, modelMark, providerMark } from "../ui/Mark";
import type { MenuEntry } from "../ui/Menu";
import { wire } from "../wire";
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

/** A model's efforts, as `models.efforts` answers for a model not yet in use. */
type Efforts = { choices: ConfigChoice[]; defaultId: string | null };

type Effort = { value: string; choices: ConfigChoice[] };

/** The panels that open beside the menu: the effort, every model, or the other agents. */
type Flyout = "effort" | "more" | "agents";

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
 * A row of the menu itself: one of this agent's models, More models, Change
 * agent, or one of the teammate's actions (`rule`: a rule stands above it).
 */
type MainRow = { kind: "model"; choice: ConfigChoice } | { kind: "more" } | { kind: "agents" } | { kind: "action"; action: Action; rule: boolean };

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

/**
 * A teammate's model, account and effort in one pull-down (task-285), and
 * with `teammate` the teammate's own menu too: one pill, one menu.
 *
 * The pill names the model, with its maker's mark, and the effort after a
 * dot, then the amber dot when a plan nears its limit and the teammate's
 * face; the account is in the tooltip and the menu, never the pill. A
 * `compact` pill is the mark and the face. The menu says who the teammate
 * is, then what it runs on: the agent and its plan, the plan's lines, this
 * agent's models, one click each, the one in use with its effort; each
 * model with levels opens them beside it (the one in use at its effort,
 * another at the teammate's effort when it has it, else its default), and
 * picking a level of a model not in use sets the model and the level
 * together. Change agent opens every other account and agent: an account
 * opens its models, a model opens its efforts. Then the teammate's own
 * actions. Picking a model keeps the effort when the model has it, else the
 * model's default; picking an effort sets both.
 */
export function ModelPicker({
	choices,
	modelId,
	backendId = "hotline",
	agents = [],
	effort,
	label,
	onModel,
	onAgent,
	onEffort,
	onModelEffort,
	onAddAccount,
	fallbackName,
	knownName,
	onOpen,
	plan,
	teammate,
	compact = false,
}: {
	/** The account's plan limits, as the agent last reported them. */
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
	label: string;
	onModel(modelId: string): Promise<void> | void;
	onEffort(value: string): Promise<void> | void;
	/** One of this agent's models not in use, at one of its own levels; else the model, then the effort. */
	onModelEffort?: ((modelId: string, value: string) => Promise<void> | void) | undefined;
	/** Settings › Providers, where accounts are added. */
	onAddAccount?: (() => void) | undefined;
}) {
	const button = useRef<HTMLButtonElement>(null);
	const [open, setOpen] = useState(false);
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
	const effortName = effort?.choices.find((choice) => choice.id === effort.value)?.name ?? null;
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
				aria-label={`${teammate === undefined ? label : `${teammate.name}${teammate.working ? ", working" : ""}`}: ${modelName}${effortName === null ? "" : ` · ${effortName}`}`}
				aria-haspopup="menu"
				aria-expanded={open}
				title={open ? undefined : [teammate?.name ?? null, modelName, accountLine, effortName].filter((one) => one !== null).join(" · ")}
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
				{nearLimit(plan) && <span className="plan-dot" aria-label="Near the plan's limit" />}
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
					plan={plan}
					models={models}
					inUse={inUse}
					effort={effort}
					others={elsewhere}
					modelId={modelId}
					onModel={(id) => {
						close();
						if (id !== inUse) void onModel(id);
					}}
					onEffort={(value) => {
						close();
						if (value !== effort?.value) void onEffort(value);
					}}
					onModelEffort={(id, value) => {
						close();
						void (async () => {
							if (onModelEffort !== undefined) {
								await onModelEffort(id, value);
								return;
							}
							await onModel(id);
							await onEffort(value);
						})();
					}}
					onPick={(row, id) => {
						close();
						void pick(row, id);
					}}
					onPickEffort={(row, id, value) => {
						close();
						void (async () => {
							await pick(row, id);
							await onEffort(value);
						})();
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

function Panels({
	anchor,
	teammate,
	head,
	plan,
	models,
	inUse,
	effort,
	others,
	modelId,
	onModel,
	onEffort,
	onModelEffort,
	onPick,
	onPickEffort,
	onAddAccount,
	onClose,
}: {
	anchor: HTMLElement;
	teammate: MenuTeammate | undefined;
	head: { mark: string | null; name: string; kind: string };
	plan: PlanUsage | undefined;
	/** This agent's models, every one; the menu shows the first few. */
	models: ConfigChoice[];
	inUse: string | null;
	effort: Effort | null;
	/** Every other account and agent, for Change agent. */
	others: AccountRow[];
	modelId: string;
	onModel(id: string): void;
	onEffort(value: string): void;
	onModelEffort(id: string, value: string): void;
	onPick(row: AccountRow, id: string): void;
	onPickEffort(row: AccountRow, id: string, value: string): void;
	onAddAccount?: () => void;
	onClose(): void;
}) {
	const main = useRef<HTMLDivElement>(null);
	const side = useRef<HTMLDivElement>(null);
	const sub = useRef<HTMLDivElement>(null);
	const fly = useRef<HTMLDivElement>(null);
	const [mainStyle, setMainStyle] = useState<CSSProperties>({ visibility: "hidden" });
	// The open flyout beside the menu, the account open beside Change agent's
	// list, the model open beside that, and where each panel sits.
	// An effort flyout names its model; More models may open one model's levels beside it.
	const [flyout, setFlyout] = useState<{ kind: Flyout; top: number; model?: string } | null>(null);
	const [moreModel, setMoreModel] = useState<{ id: string; top: number } | null>(null);
	const [account, setAccount] = useState<{ key: string; top: number } | null>(null);
	const [model, setModel] = useState<{ id: string; top: number } | null>(null);
	const [efforts, setEfforts] = useState<Record<string, Efforts>>({});
	// Keyboard: which panel has focus and which row in each. Nothing is lit
	// until the keyboard moves; the check marks the model in use.
	const [level, setLevel] = useState<0 | 1 | 2 | 3>(0);
	const [active, setActive] = useState(-1);
	const [sideActive, setSideActive] = useState(0);
	const [subActive, setSubActive] = useState(0);
	const [flyActive, setFlyActive] = useState(0);

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
			for (const panel of [main, side, sub, fly]) if (panel.current?.contains(target)) return;
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

	const levels = effort !== null && effort.choices.length > 1 ? effort.choices : [];
	// A model's levels: the one in use has the session's, another its own,
	// as its agent lists them. A single level is nothing to choose.
	const levelsOf = (id: string | undefined): ConfigChoice[] => {
		if (id === undefined) return [];
		if (id === inUse) return levels;
		const own = models.find((choice) => choice.id === id)?.efforts ?? [];
		return own.length > 1 ? own : [];
	};
	// The level checked for a model: the one in use, else the teammate's
	// effort where the model has it, else the model's own default.
	const levelOf = (id: string): string | undefined => {
		if (id === inUse) return effort?.value;
		if (effort !== null && levelsOf(id).some((one) => one.id === effort.value)) return effort.value;
		return models.find((choice) => choice.id === id)?.defaultEffort ?? undefined;
	};
	const pickLevel = (id: string, value: string) => (id === inUse ? onEffort(value) : onModelEffort(id, value));
	// The levels of the model in use are its Effort; another's are named for it.
	const levelsLabel = (id: string) => (id === inUse ? "Effort" : `${models.find((one) => one.id === id)?.name ?? id} effort`);
	const flyLevels = flyout?.kind === "effort" ? levelsOf(flyout.model) : [];
	const moreLevels = flyout?.kind === "more" ? levelsOf(moreModel?.id) : [];
	const shown = shownModels(models, inUse);
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
		...shown.map((choice) => ({ kind: "model" as const, choice })),
		...(models.length > shown.length ? [{ kind: "more" as const }] : []),
		...(changes ? [{ kind: "agents" as const }] : []),
		...actions,
	];
	const firstAgent = others.findIndex((row) => sectionOf(row) === "agents");
	// Change agent's list: every other account and agent, then Add an account.
	const agentList: (AccountRow | "add")[] = [...others, ...(onAddAccount !== undefined ? ["add" as const] : [])];
	const sideCount = flyout?.kind === "effort" ? flyLevels.length : flyout?.kind === "more" ? models.length : agentList.length;

	const opens = (row: AccountRow) => row.unavailable === undefined && row.choices.length > 0;
	const openRow = others.find((row) => row.key === account?.key);
	const openModel = openRow?.choices.find((choice) => choice.id === model?.id);

	const showFlyout = (kind: Flyout, element: HTMLElement, modelId?: string) => {
		if (flyout?.kind === kind && flyout.model === modelId) return;
		setFlyout({ kind, top: element.getBoundingClientRect().top, ...(modelId !== undefined ? { model: modelId } : {}) });
		setAccount(null);
		setModel(null);
		setMoreModel(null);
		const at =
			kind === "effort"
				? levelsOf(modelId).findIndex((one) => one.id === levelOf(modelId ?? ""))
				: kind === "more"
					? models.findIndex((one) => one.id === inUse)
					: 0;
		setSideActive(Math.max(0, at));
	};
	const hideFlyout = () => {
		setFlyout(null);
		setAccount(null);
		setModel(null);
		setMoreModel(null);
	};
	// A row of More models with levels opens them beside the list.
	const showMoreModel = (choice: ConfigChoice, element: HTMLElement) => {
		if (levelsOf(choice.id).length === 0) {
			setMoreModel(null);
			return;
		}
		if (moreModel?.id === choice.id) return;
		setMoreModel({ id: choice.id, top: element.getBoundingClientRect().top });
		setSubActive(Math.max(0, levelsOf(choice.id).findIndex((one) => one.id === levelOf(choice.id))));
	};
	const showAccount = (row: AccountRow, element: HTMLElement) => {
		// An agent that is not ready does not open: its row says why. One
		// with no list yet has nothing to open; its row is the pick.
		if (!opens(row)) {
			setAccount(null);
			setModel(null);
			return;
		}
		setAccount({ key: row.key, top: element.getBoundingClientRect().top });
		setModel(null);
	};
	const showModel = (choice: ConfigChoice, element: HTMLElement) => {
		setModel({ id: choice.id, top: element.getBoundingClientRect().top });
		// Efforts are Hotline Agent's to list; a harness's models have none here.
		if (openRow?.agent === undefined && choice.id !== "" && efforts[choice.id] === undefined) {
			void wire.command("models.efforts", { modelId: choice.id }).then(
				(answer) => setEfforts((was) => ({ ...was, [choice.id]: { choices: answer.choices, defaultId: answer.defaultId ?? null } })),
				() => setEfforts((was) => ({ ...was, [choice.id]: { choices: [], defaultId: null } })),
			);
		}
	};
	// What a main row opens beside the menu, if anything.
	const flyoutOf = (row: MainRow | undefined): Flyout | null => {
		if (row === undefined) return null;
		if (row.kind === "more") return "more";
		if (row.kind === "agents") return "agents";
		if (row.kind === "action") return null;
		return levelsOf(row.choice.id).length > 0 ? "effort" : null;
	};
	const back = () => {
		if (level === 3) setModel(null);
		else if (level === 2) {
			setAccount(null);
			setMoreModel(null);
		} else hideFlyout();
		setLevel((was) => Math.max(0, was - 1) as 0 | 1 | 2);
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
		const list =
			level === 0
				? rows.length
				: level === 1
					? sideCount
					: level === 2
						? flyout?.kind === "more"
							? moreLevels.length
							: (openRow?.choices.length ?? 0)
						: (efforts[model?.id ?? ""]?.choices.length ?? 0);
		const step = (value: number, by: number) => (list === 0 ? 0 : (value + by + list) % list);
		if (key === "ArrowDown" || key === "ArrowUp") {
			event.preventDefault();
			const by = key === "ArrowDown" ? 1 : -1;
			if (level === 0) {
				const from = Math.max(0, rows.findIndex((row) => row.kind === "model" && row.choice.id === inUse));
				setActive((was) => (was < 0 ? from : step(was, by)));
			} else if (level === 1) setSideActive((was) => step(was, by));
			else if (level === 2) setSubActive((was) => step(was, by));
			else setFlyActive((was) => step(was, by));
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
			const modelRow = row?.kind === "model" ? row.choice.id : undefined;
			// Enter on a model not in use picks it; → opens its levels.
			const opens = kind !== null && (!enter || modelRow === undefined || modelRow === inUse);
			if (opens && element) {
				showFlyout(kind, element, modelRow);
				setLevel(1);
			} else if (modelRow !== undefined && enter) onModel(modelRow);
			else if (row?.kind === "action" && enter) act(row.action);
		} else if (level === 1) {
			if (flyout?.kind === "effort") {
				const value = flyLevels[sideActive];
				if (value && enter && flyout.model !== undefined) pickLevel(flyout.model, value.id);
			} else if (flyout?.kind === "more") {
				const choice = models[sideActive];
				const element = side.current?.querySelector<HTMLElement>(`[data-row="${sideActive}"]`);
				if (choice && !enter && element && levelsOf(choice.id).length > 0) {
					showMoreModel(choice, element);
					setLevel(2);
				} else if (choice && enter) onModel(choice.id);
			} else {
				chooseAgent(agentList[sideActive], side.current?.querySelector<HTMLElement>(`[data-row="${sideActive}"]`) ?? null, enter);
			}
		} else if (level === 2 && flyout?.kind === "more") {
			const value = moreLevels[subActive];
			if (value && enter && moreModel !== null) pickLevel(moreModel.id, value.id);
		} else if (level === 2) {
			const choice = openRow?.choices[subActive];
			if (!choice || !openRow) return;
			const element = sub.current?.querySelector<HTMLElement>(`[data-row="${subActive}"]`);
			if (!enter) {
				if (openRow.agent === undefined && element) {
					showModel(choice, element);
					setLevel(3);
					setFlyActive(0);
				}
			} else onPick(openRow, choice.id);
		} else {
			const value = efforts[model?.id ?? ""]?.choices[flyActive];
			if (value && model && openRow && enter) onPickEffort(openRow, model.id, value.id);
		}
	};

	// A panel opens beside the one it came from, toward the window's middle.
	const beside = (panel: HTMLElement | null, top: number, width: number): CSSProperties => {
		const box = panel?.getBoundingClientRect();
		if (!box) return { visibility: "hidden" };
		const left = box.left - width - 4 >= 8 ? box.left - width - 4 : box.right + 4;
		return { top: Math.max(8, Math.min(top - 4, window.innerHeight - 160)), left, width };
	};
	const lit = (here: 0 | 1 | 2 | 3, at: number, current: number) => (level === here && current === at ? "true" : undefined);
	// An action closes the menu first, as any menu's item does.
	const act = (action: Action) => {
		if (action.disabled) return;
		onClose();
		action.onSelect();
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
				{plan !== undefined && plan.windows.length > 0 && <PlanRows plan={plan} />}
				{rows.map((row, at) => {
					if (row.kind === "model") {
						const { choice } = row;
						const used = choice.id === inUse;
						const opensEffort = levelsOf(choice.id).length > 0;
						const effortOpen = opensEffort && flyout?.kind === "effort" && flyout.model === choice.id;
						return (
							<button
								key={choice.id}
								type="button"
								role="menuitemradio"
								aria-checked={used}
								aria-haspopup={opensEffort ? "menu" : undefined}
								aria-expanded={opensEffort ? effortOpen : undefined}
								data-row={at}
								data-current={used ? "true" : undefined}
								data-active={lit(0, at, active) ?? (effortOpen ? "true" : undefined)}
								tabIndex={-1}
								className="menu-item model-row"
								onMouseEnter={(event) => {
									setActive(at);
									setLevel(0);
									if (opensEffort) showFlyout("effort", event.currentTarget, choice.id);
									else hideFlyout();
								}}
								onClick={(event) => {
									if (used && opensEffort) showFlyout("effort", event.currentTarget, choice.id);
									else onModel(choice.id);
								}}
							>
								<span className="menu-item-check model-check">{used && <CheckIcon />}</span>
								<span className="menu-item-text">{choice.name}</span>
								{used && effort !== null && <span className="model-row-detail">{effort.choices.find((one) => one.id === effort.value)?.name ?? effort.value}</span>}
								{opensEffort && <ChevronRightIcon className="model-row-more" />}
							</button>
						);
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
									showFlyout(row.kind, event.currentTarget);
								}}
								onClick={(event) => showFlyout(row.kind, event.currentTarget)}
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
			{flyout?.kind === "effort" && flyout.model !== undefined && (
				<Levels
					panel={side}
					label={levelsLabel(flyout.model)}
					levels={flyLevels}
					checked={levelOf(flyout.model)}
					active={level === 1 ? sideActive : -1}
					style={beside(main.current, flyout.top, 160)}
					onHover={(at) => {
						setSideActive(at);
						setLevel(1);
					}}
					onPick={(value) => pickLevel(flyout.model ?? "", value)}
				/>
			)}
			{flyout?.kind === "more" && (
				<div ref={side} role="menu" tabIndex={-1} aria-label={`${head.name} models`} className="menu model-menu" style={beside(main.current, flyout.top, 248)}>
					<div className="menu-heading">{head.name}</div>
					{models.map((choice, at) => {
						const opensEffort = levelsOf(choice.id).length > 0;
						return (
							<button
								key={choice.id}
								type="button"
								role="menuitemradio"
								aria-checked={choice.id === inUse}
								aria-haspopup={opensEffort ? "menu" : undefined}
								aria-expanded={opensEffort ? moreModel?.id === choice.id : undefined}
								data-row={at}
								data-active={lit(1, at, sideActive) ?? (moreModel?.id === choice.id ? "true" : undefined)}
								tabIndex={-1}
								className="menu-item model-row"
								onMouseEnter={(event) => {
									setSideActive(at);
									setLevel(1);
									showMoreModel(choice, event.currentTarget);
								}}
								onClick={() => onModel(choice.id)}
							>
								<span className="menu-item-check model-check">{choice.id === inUse && <CheckIcon />}</span>
								<span className="menu-item-text">{choice.name}</span>
								{opensEffort && <ChevronRightIcon className="model-row-more" />}
							</button>
						);
					})}
				</div>
			)}
			{flyout?.kind === "more" && moreModel !== null && (
				<Levels
					panel={sub}
					label={levelsLabel(moreModel.id)}
					levels={moreLevels}
					checked={levelOf(moreModel.id)}
					active={level === 2 ? subActive : -1}
					style={beside(side.current, moreModel.top, 160)}
					onHover={(at) => {
						setSubActive(at);
						setLevel(2);
					}}
					onPick={(value) => pickLevel(moreModel.id, value)}
				/>
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
									setModel(null);
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
							aria-haspopup={openRow.agent === undefined ? "menu" : undefined}
							data-row={at}
							data-active={lit(2, at, subActive) ?? (model?.id === choice.id ? "true" : undefined)}
							tabIndex={-1}
							className="menu-item model-row"
							onMouseEnter={(event) => {
								setSubActive(at);
								setLevel(2);
								showModel(choice, event.currentTarget);
							}}
							onClick={() => onPick(openRow, choice.id)}
						>
							<span className="menu-item-check">{choice.id === modelId && <CheckIcon />}</span>
							{/* A harness's own names ("Opus 5.5", "Default") carry the harness's mark. */}
							<Mark id={modelMark(choice.id, choice.name) ?? (openRow.agent === undefined ? null : harnessMark(openRow.agent === MORE_AGENTS ? choice.id : openRow.agent))} />
							<span className="menu-item-text">{choice.name}</span>
							{/* Only Hotline Agent's models open their efforts. */}
							{openRow.agent === undefined && <ChevronRightIcon className="model-row-more" />}
						</button>
					))}
				</div>
			)}
			{openRow !== undefined && openModel !== undefined && model !== null && (efforts[model.id]?.choices.length ?? 0) > 0 && (
				<div ref={fly} role="menu" tabIndex={-1} aria-label={`${openModel.name} effort`} className="menu model-menu" style={beside(sub.current, model.top, 160)}>
					<div className="menu-heading">{openModel.name}</div>
					{efforts[model.id]!.choices.map((one, at) => {
						const kept = effort?.value !== undefined && efforts[model.id]!.choices.some((each) => each.id === effort.value) ? effort.value : efforts[model.id]!.defaultId;
						return (
							<button
								key={one.id}
								type="button"
								role="menuitemradio"
								aria-checked={one.id === kept}
								data-active={lit(3, at, flyActive)}
								tabIndex={-1}
								className="menu-item model-row"
								onMouseEnter={() => {
									setFlyActive(at);
									setLevel(3);
								}}
								onClick={() => onPickEffort(openRow, model.id, one.id)}
							>
								<span className="menu-item-check">{one.id === kept && <CheckIcon />}</span>
								<span className="menu-item-text">{one.name}</span>
							</button>
						);
					})}
				</div>
			)}
		</div>,
		document.body,
	);
}

/** A model's effort levels, beside the row that opened them, the one it runs at checked. */
function Levels({
	panel,
	label,
	levels,
	checked,
	active,
	style,
	onHover,
	onPick,
}: {
	panel: RefObject<HTMLDivElement | null>;
	label: string;
	levels: ConfigChoice[];
	checked: string | undefined;
	/** The row the keyboard is on, or -1. */
	active: number;
	style: CSSProperties;
	onHover(at: number): void;
	onPick(value: string): void;
}) {
	return (
		<div ref={panel} role="menu" tabIndex={-1} aria-label={label} className="menu model-menu" style={style}>
			<div className="menu-heading">Effort</div>
			{levels.map((one, at) => (
				<button
					key={one.id}
					type="button"
					role="menuitemradio"
					aria-checked={one.id === checked}
					data-row={at}
					data-active={at === active ? "true" : undefined}
					tabIndex={-1}
					className="menu-item model-row"
					title={one.description ?? undefined}
					onMouseEnter={() => onHover(at)}
					onClick={() => onPick(one.id)}
				>
					<span className="menu-item-check model-check">{one.id === checked && <CheckIcon />}</span>
					<span className="menu-item-text">{one.name}</span>
				</button>
			))}
		</div>
	);
}

/**
 * The plan under the head: a line per window, its name, a thin bar, the use
 * and when it resets. A tick on the bar marks where use would be if spread
 * evenly over the window so far; the line's hover says its runway.
 */
function PlanRows({ plan }: { plan: PlanUsage }) {
	const windows = shownWindows(plan);
	if (windows.length === 0) return null;
	const now = Date.now();
	return (
		<div className="plan-rows" title={`Plan usage as of ${new Date(plan.at).toLocaleString()}`}>
			{windows.map((window) => {
				const tick = paceTick(window, now);
				return (
					<div key={window.kind} className="plan-row" data-status={window.status} title={windowHover(window, now)}>
						<span className="plan-name">{windowName(window)}</span>
						<span className="plan-bar" aria-hidden="true">
							<span className="plan-fill" style={{ width: `${Math.min(100, window.status === "limited" ? 100 : (window.usedPercent ?? 0))}%` }} />
							{tick !== undefined && <span className="plan-tick" style={{ left: `${tick}%` }} />}
						</span>
						<span className="plan-use">{window.status === "limited" ? "100%" : window.usedPercent !== undefined ? `${Math.round(window.usedPercent)}%` : "–"}</span>
						<span className="plan-reset">{window.resetsAt !== undefined ? resetShort(window.resetsAt, now) : ""}</span>
					</div>
				);
			})}
		</div>
	);
}
