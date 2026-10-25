import { type CSSProperties, Fragment, type KeyboardEvent, type PointerEvent, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { ChoiceAccount, ConfigChoice, CredentialKind } from "../generated/contract";
import { ChevronDownIcon, ChevronRightIcon, CheckIcon, PlusIcon } from "../icons";
import { harnessMark, Mark, modelMark, providerMark } from "../ui/Mark";
import { wire } from "../wire";
import { accountName, plainModel } from "../accounts";

/**
 * One row of the top level: an account of Hotline Agent and the models it
 * serves, or another agent (`agent`, its backend id) and its models.
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
	/** What the teammate runs on, pinned above the API and Agents sections. */
	pinned?: boolean;
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

/** The model a harness picks for itself, until it has listed its own. */

/** A model's efforts, as `models.efforts` answers for a model not yet in use. */
type Efforts = { choices: ConfigChoice[]; defaultId: string | null };

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

/** The part of the menu a row stands in: pinned at the top, API, or Agents. */
function sectionOf(row: AccountRow): "pinned" | "api" | "agents" {
	if (row.pinned === true) return "pinned";
	return row.agent === undefined ? "api" : "agents";
}

/** Which row a model id is under: the one listing it, else the first serving its plain id. */
function rowOf(rows: AccountRow[], modelId: string): AccountRow | undefined {
	return rows.find((row) => row.choices.some((choice) => choice.id === modelId)) ?? rows.find((row) => row.choices.some((choice) => plainModel(choice.id) === modelId));
}

/**
 * A teammate's model, account and effort in one pull-down (task-285).
 *
 * The button names the model, with its maker's mark, and the effort after a
 * dot; the account is in the tooltip and the menu's head, never the button.
 * The head names what the teammate runs on now, with an effort slider: one
 * stop per level the model has, and moving it changes nothing but the effort.
 * Below it every account, one row each; an account opens its models, a model
 * opens its efforts. Picking a model keeps the effort when the model has it,
 * else the model's default; picking an effort sets both.
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
	onAddAccount,
	fallbackName,
	onOpen,
}: {
	/** Each time the menu opens, so lists the desk fills in later can be read again. */
	onOpen?: (() => void) | undefined;
	/** What the button says before a harness has named its model: the harness. */
	fallbackName?: string | undefined;
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
	effort: { value: string; choices: ConfigChoice[] } | null;
	label: string;
	onModel(modelId: string): Promise<void> | void;
	onEffort(value: string): Promise<void> | void;
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
	// What the teammate runs on is pinned at the top, and not listed again
	// under API (Hotline Agent's accounts, straight to a provider) or Agents.
	const rows = [
		...(currentRow === undefined ? [] : [{ ...currentRow, pinned: true }]),
		...accounts.filter((row) => row !== currentRow),
		...agentRows.filter((row) => row !== currentRow),
	];
	const current = currentRow?.choices.find((choice) => choice.id === modelId || plainModel(choice.id) === modelId);
	// A model on an account that is gone still has its name from another account serving it.
	const sameModel = choices.find((choice) => plainModel(choice.id) === plainModel(modelId));
	const modelName = current?.name ?? sameModel?.name ?? (modelId.split("/").slice(1).join("/") || modelId || fallbackName || "Model");
	const effortName = effort?.choices.find((choice) => choice.id === effort.value)?.name ?? null;
	const rowLine = currentRow === undefined ? null : `${currentRow.name}${currentRow.detail === "" ? "" : ` · ${currentRow.detail}`}`;
	// An agent that has not named its model yet is already the head's name.
	const accountLine = rowLine === modelName ? null : rowLine;
	const mark = onHotline ? modelMark(modelId, modelName) : (modelMark(modelId, modelName) ?? harnessMark(backendId));
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
				className="control picker model-picker"
				aria-label={`${label}: ${modelName}${effortName === null ? "" : ` · ${effortName}`}`}
				aria-haspopup="menu"
				aria-expanded={open}
				title={open ? undefined : [modelName, accountLine, effortName].filter((one) => one !== null).join(" · ")}
				onClick={() => {
					if (!open) onOpen?.();
					setOpen((was) => !was);
				}}
			>
				<Mark id={mark} />
				<span className="picker-label">
					{modelName}
					{effortName !== null && <span className="text-ink-3"> · {effortName}</span>}
				</span>
				<ChevronDownIcon className="picker-chevron" />
			</button>
			{open && button.current && (
				<Panels
					anchor={button.current}
					rows={rows}
					modelId={modelId}
					modelName={modelName}
					mark={mark}
					accountLine={accountLine}
					currentRow={currentRow?.key ?? null}
					effort={effort}
					onModel={(row, id) => {
						close();
						void pick(row, id);
					}}
					onModelEffort={(row, id, value) => {
						close();
						void (async () => {
							await pick(row, id);
							await onEffort(value);
						})();
					}}
					onEffort={(value) => void onEffort(value)}
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
	rows,
	modelId,
	modelName,
	mark,
	accountLine,
	currentRow,
	effort,
	onModel,
	onModelEffort,
	onEffort,
	onAddAccount,
	onClose,
}: {
	anchor: HTMLElement;
	rows: AccountRow[];
	modelId: string;
	modelName: string;
	mark: string | null;
	accountLine: string | null;
	currentRow: string | null;
	effort: { value: string; choices: ConfigChoice[] } | null;
	onModel(row: AccountRow, id: string): void;
	onModelEffort(row: AccountRow, id: string, value: string): void;
	onEffort(value: string): void;
	onAddAccount?: () => void;
	onClose(): void;
}) {
	const main = useRef<HTMLDivElement>(null);
	const sub = useRef<HTMLDivElement>(null);
	const fly = useRef<HTMLDivElement>(null);
	const [mainStyle, setMainStyle] = useState<CSSProperties>({ visibility: "hidden" });
	// The open account (its row's key), the open model, and where each panel sits.
	const [account, setAccount] = useState<{ key: string; top: number } | null>(null);
	const [model, setModel] = useState<{ id: string; top: number } | null>(null);
	const [efforts, setEfforts] = useState<Record<string, Efforts>>({});
	// Keyboard: which level has focus and which row in it.
	const [level, setLevel] = useState<0 | 1 | 2>(0);
	// Nothing is lit until the keyboard moves; the dot marks the row in use.
	const [active, setActive] = useState(-1);
	const inUse = Math.max(0, rows.findIndex((row) => row.key === currentRow));
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
			for (const panel of [main, sub, fly]) if (panel.current?.contains(target)) return;
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

	const opens = (row: AccountRow) => row.unavailable === undefined && row.choices.length > 0;
	const pinned = rows[0]?.pinned === true ? rows[0] : undefined;
	const openRow = rows.find((row) => row.key === account?.key);
	const openModel = openRow?.choices.find((choice) => choice.id === model?.id);

	const showAccount = (row: AccountRow, element: HTMLElement) => {
		// An agent that is not ready does not open: its row says why. One
		// with no list yet has nothing to open; its row is the pick.
		if (row.unavailable !== undefined || row.choices.length === 0) {
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

	const onKey = (event: KeyboardEvent) => {
		const key = event.key;
		if (key === "Escape") {
			event.preventDefault();
			event.stopPropagation();
			if (level > 0) {
				setLevel((was) => (was - 1) as 0 | 1);
				if (level === 2) setModel(null);
				else setAccount(null);
			} else onClose();
			return;
		}
		if (key === "Tab") {
			event.preventDefault();
			onClose();
			return;
		}
		const list = level === 0 ? rows.length : level === 1 ? (openRow?.choices.length ?? 0) : (efforts[model?.id ?? ""]?.choices.length ?? 0);
		const step = (value: number, by: number) => (list === 0 ? 0 : (value + by + list) % list);
		if (key === "ArrowDown" || key === "ArrowUp") {
			event.preventDefault();
			const by = key === "ArrowDown" ? 1 : -1;
			if (level === 0) setActive((was) => (was < 0 ? inUse : step(was, by)));
			else if (level === 1) setSubActive((was) => step(was, by));
			else setFlyActive((was) => step(was, by));
			return;
		}
		if (key === "ArrowRight" || key === "Enter" || key === " ") {
			event.preventDefault();
			if (level === 0) {
				const row = rows[active];
				const element = main.current?.querySelector<HTMLElement>(`[data-row="${active}"]`);
				if (row && row.unavailable === undefined && row.choices.length === 0) {
					if (key !== "ArrowRight") onModel(row, "");
				} else if (row && element) {
					showAccount(row, element);
					setLevel(1);
					setSubActive(Math.max(0, row.choices.findIndex((choice) => choice.id === modelId)));
				}
			} else if (level === 1) {
				const choice = openRow?.choices[subActive];
				if (!choice) return;
				const element = sub.current?.querySelector<HTMLElement>(`[data-row="${subActive}"]`);
				if (key === "ArrowRight" && element) {
					showModel(choice, element);
					setLevel(2);
					setFlyActive(0);
				} else if (key !== "ArrowRight" && openRow) onModel(openRow, choice.id);
			} else {
				const value = efforts[model?.id ?? ""]?.choices[flyActive];
				if (value && model && openRow) onModelEffort(openRow, model.id, value.id);
			}
			return;
		}
		if (key === "ArrowLeft" && level > 0) {
			event.preventDefault();
			if (level === 2) setModel(null);
			else setAccount(null);
			setLevel((was) => (was - 1) as 0 | 1);
		}
	};

	// The submenu opens beside the main menu, toward the window's middle;
	// the efforts beside it, the same way.
	const beside = (panel: HTMLElement | null, top: number, width: number): CSSProperties => {
		const box = panel?.getBoundingClientRect();
		if (!box) return { visibility: "hidden" };
		const left = box.left - width - 4 >= 8 ? box.left - width - 4 : box.right + 4;
		return { top: Math.max(8, Math.min(top - 4, window.innerHeight - 160)), left, width };
	};

	return createPortal(
		// biome-ignore lint/a11y/noStaticElementInteractions: the panels share one keyboard handler; each panel is a menu
		<div onKeyDown={onKey}>
			<div ref={main} role="menu" tabIndex={-1} aria-label="Model" className="menu model-menu model-menu-main" style={mainStyle}>
				<div className="model-head">
					{/* What the teammate runs on is the head itself: one large key that
					 * opens its models, so it is never looked for further down. */}
					{pinned !== undefined && opens(pinned) ? (
						<button
							type="button"
							role="menuitem"
							aria-haspopup="menu"
							aria-expanded={account?.key === pinned.key}
							aria-label={`${modelName}, ${accountLine ?? pinned.name}, in use`}
							data-row={0}
							data-active={(level === 0 && active === 0) || account?.key === pinned.key ? "true" : undefined}
							tabIndex={-1}
							className="model-head-key"
							onMouseEnter={(event) => {
								setActive(0);
								setLevel(0);
								showAccount(pinned, event.currentTarget);
							}}
							onClick={(event) => showAccount(pinned, event.currentTarget)}
						>
							<span className="min-w-0 flex-1">
								<span className="model-head-now">
									<Mark id={mark} />
									<span className="truncate">{modelName}</span>
								</span>
								{accountLine !== null && <span className="model-head-account">{accountLine}</span>}
							</span>
							<ChevronRightIcon className="model-row-more" />
						</button>
					) : (
						<div className="model-head-plain">
							<div className="model-head-now">
								<Mark id={mark} />
								<span className="truncate">{modelName}</span>
							</div>
							{accountLine !== null && <div className="model-head-account">{accountLine}</div>}
						</div>
					)}
					{effort !== null && effort.choices.length > 1 && <EffortSlider effort={effort} onEffort={onEffort} />}
				</div>
				<hr className="menu-rule" />
				{/* The pinned row is the head's key, never a row again below. */}
				{rows.map((row, at) => row.pinned === true ? null : (
					<Fragment key={row.key}>
					{(at === 0 || sectionOf(row) !== sectionOf(rows[at - 1]!)) && (
						<>
							{at > 0 && rows[at - 1]!.pinned !== true && <hr className="menu-rule" />}
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
						data-active={(level === 0 && active === at) || account?.key === row.key ? "true" : undefined}
						tabIndex={-1}
						className="menu-item model-row"
						onMouseEnter={(event) => {
							setActive(at);
							setLevel(0);
							showAccount(row, event.currentTarget);
						}}
						onClick={(event) => {
							if (row.unavailable === undefined && row.choices.length === 0) onModel(row, "");
							else showAccount(row, event.currentTarget);
						}}
					>
						<span className="menu-item-check">
							{row.key === currentRow && (
								<>
									<span className="model-dot" aria-hidden="true" />
									<span className="sr-only">In use</span>
								</>
							)}
						</span>
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
						<hr className="menu-rule" />
						<button type="button" role="menuitem" tabIndex={-1} className="menu-item model-add" onClick={onAddAccount}>
							<span className="menu-item-check" />
							<PlusIcon className="model-add-mark" />
							<span className="menu-item-text">Add an account…</span>
						</button>
					</>
				)}
			</div>
			{openRow !== undefined && account !== null && (
				<div ref={sub} role="menu" tabIndex={-1} aria-label={openRow.name} className="menu model-menu" style={beside(main.current, account.top, 248)}>
					<div className="menu-heading">{openRow.name}</div>
					{openRow.choices.map((choice, at) => (
						<button
							key={choice.id}
							type="button"
							role="menuitemradio"
							aria-checked={choice.id === modelId}
							aria-haspopup={openRow.agent === undefined ? "menu" : undefined}
							data-row={at}
							data-active={(level === 1 && subActive === at) || model?.id === choice.id ? "true" : undefined}
							tabIndex={-1}
							className="menu-item model-row"
							onMouseEnter={(event) => {
								setSubActive(at);
								setLevel(1);
								showModel(choice, event.currentTarget);
							}}
							onClick={() => onModel(openRow, choice.id)}
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
					{efforts[model.id]!.choices.map((level2, at) => {
						const kept = effort?.value !== undefined && efforts[model.id]!.choices.some((one) => one.id === effort.value) ? effort.value : efforts[model.id]!.defaultId;
						return (
							<button
								key={level2.id}
								type="button"
								role="menuitemradio"
								aria-checked={level2.id === kept}
								data-active={level === 2 && flyActive === at ? "true" : undefined}
								tabIndex={-1}
								className="menu-item"
								onMouseEnter={() => {
									setFlyActive(at);
									setLevel(2);
								}}
								onClick={() => onModelEffort(openRow, model.id, level2.id)}
							>
								<span className="menu-item-check">{level2.id === kept && <CheckIcon />}</span>
								<span className="menu-item-text">{level2.name}</span>
							</button>
						);
					})}
				</div>
			)}
		</div>,
		document.body,
	);
}

/**
 * Effort as a slider: one stop per level the model has, the thumb on the one
 * in use. A click on a stop or a drag along the track sets it; the arrow keys
 * step it. It changes the effort and nothing else.
 */
function EffortSlider({ effort, onEffort }: { effort: { value: string; choices: ConfigChoice[] }; onEffort(value: string): void }) {
	const track = useRef<HTMLDivElement>(null);
	const count = effort.choices.length;
	const at = Math.max(0, effort.choices.findIndex((choice) => choice.id === effort.value));
	const [shown, setShown] = useState(at);
	useEffect(() => setShown(at), [at]);
	const position = (index: number) => (count < 2 ? 0 : (index / (count - 1)) * 100);
	const nearest = (clientX: number) => {
		const box = track.current?.getBoundingClientRect();
		if (!box || count < 2) return 0;
		const ratio = Math.min(1, Math.max(0, (clientX - box.left) / box.width));
		return Math.round(ratio * (count - 1));
	};
	const settle = (index: number) => {
		setShown(index);
		const choice = effort.choices[index];
		if (choice && choice.id !== effort.value) onEffort(choice.id);
	};
	const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
		event.currentTarget.setPointerCapture(event.pointerId);
		setShown(nearest(event.clientX));
	};
	const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
		if (event.currentTarget.hasPointerCapture(event.pointerId)) setShown(nearest(event.clientX));
	};
	const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
		if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
		event.currentTarget.releasePointerCapture(event.pointerId);
		settle(nearest(event.clientX));
	};
	const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
		const by = event.key === "ArrowRight" || event.key === "ArrowUp" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowDown" ? -1 : 0;
		if (by === 0) return;
		event.preventDefault();
		event.stopPropagation();
		settle(Math.min(count - 1, Math.max(0, shown + by)));
	};
	const name = effort.choices[shown]?.name ?? "";
	return (
		<div className="effort-slider">
			<div className="effort-slider-label">Effort</div>
			<div
				ref={track}
				role="slider"
				tabIndex={0}
				aria-label="Effort"
				aria-valuemin={0}
				aria-valuemax={count - 1}
				aria-valuenow={shown}
				aria-valuetext={name}
				className="effort-track"
				onPointerDown={onPointerDown}
				onPointerMove={onPointerMove}
				onPointerUp={onPointerUp}
				onKeyDown={onKey}
			>
				<span className="effort-rail" />
				<span className="effort-fill" style={{ width: `${position(shown)}%` }} />
				{effort.choices.map((choice, index) => (
					<span key={choice.id} className="effort-stop" data-lit={index <= shown ? "true" : undefined} style={{ left: `${position(index)}%` }} />
				))}
				<span className="effort-thumb" style={{ left: `${position(shown)}%` }} />
			</div>
			<div className="effort-ticks" aria-hidden="true">
				{effort.choices.map((choice, index) => (
					<button
						key={choice.id}
						type="button"
						tabIndex={-1}
						className="effort-tick"
						data-on={index === shown ? "true" : undefined}
						style={{ left: `${position(index)}%` }}
						onClick={() => settle(index)}
					>
						{choice.name}
					</button>
				))}
			</div>
		</div>
	);
}
