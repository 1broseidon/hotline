import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import type { ConfigChoice, Provider, Welcome as WelcomeState } from "../generated/contract";
import { CheckIcon, PlusIcon } from "../icons";
import { useRoomSettings } from "../room";
import { HotlineMark } from "../ui/HotlineMark";
import { Refusal } from "../ui/Refusal";
import { wire, type Connection } from "../wire";
import { ConnectProvider, ProviderRow, connectionMethod } from "./ConnectProvider";
import { NewTeammateForm } from "./NewTeammate";

/**
 * The goal the first teammate is offered. A person who has never written a
 * goal for an agent should not have to start from a blank line; one who has
 * will replace it.
 */
const FIRST_GOAL =
	"Help me with whatever I bring you. Ask when something is unclear, and say what you did when you are done.";

const HOTLINE_AGENT = "hotline";

/** The pages, in order. `providers` and `harness` are the two forks of one step. */
type Page = "hello" | "why" | "engine" | "providers" | "harness" | "teammate";

/** The steps the progress line names, and which pages belong to each. The greeting is not a step. */
const STEPS: { title: string; pages: Page[] }[] = [
	{ title: "Why Hotline", pages: ["why"] },
	{ title: "How they think", pages: ["engine"] },
	{ title: "Connect", pages: ["providers", "harness"] },
	{ title: "Your teammate", pages: ["teammate"] },
];

/**
 * What Hotline is, in four lines anyone can read: where it runs, whose AI
 * it uses, the computer a teammate can have, and reaching it from anywhere.
 * Each is a promise the product keeps today.
 */
const PILLARS: { title: string; text: string; icon: ReactNode }[] = [
	{
		title: "Runs on your computer",
		text: "Your teammates, their files and your chats stay here, not in someone else's cloud. Run it on this computer or on a server of your own.",
		icon: <HomeGlyph />,
	},
	{
		title: "Bring your own AI",
		text: "Use ChatGPT, Claude, Grok, Gemini, or a model running on your own machine. Sign in with a plan you already pay for, and switch any time.",
		icon: <SparkGlyph />,
	},
	{
		title: "A computer of their own",
		text: "Give a teammate its own private computer with a browser, so it can do real work without touching yours.",
		icon: <ScreenGlyph />,
	},
	{
		title: "With you anywhere",
		text: "Check in from your phone wherever you are. Your team keeps working on your computer while you're away.",
		icon: <PhoneGlyph />,
	},
];

/**
 * The room before anyone is in it, as a short full-window wizard that
 * teaches while it sets up: a greeting, what makes Hotline Hotline, the two
 * ways a teammate can think, connecting what that needs, then the first
 * teammate. A person who finishes it has met every word the rest of the
 * window uses.
 *
 * What is done is still derived from what the room knows — `welcome`'s
 * credentials, harnesses and roster — never from a stored flag: this screen
 * exists exactly as long as there is no teammate, and a room with one never
 * sees it. Only which page is in front is the window's own, so Back always
 * works; a room that can already run opens on the first teammate, the
 * earlier pages a Back away. The connect page renders the same forms as
 * Settings › Providers and the last page is the same form as the plus, so
 * nothing is learned twice.
 *
 * On this computer's own desk the greeting also offers a server the person
 * runs (BRO-151), as the quieter of two ways in. `onConnectServer` opens
 * that pane; it is null on a server's own desk, which has nothing to choose
 * between.
 */
export function Welcome({
	models,
	onCreated,
	onConnectServer,
}: {
	models: ConfigChoice[];
	onCreated(personaId: string): void;
	onConnectServer: (() => void) | null;
}) {
	const [page, setPage] = useState<Page>("hello");
	const [kind, setKind] = useState<"hotline" | "harness" | null>(null);
	const [state, setState] = useState<WelcomeState | null>(null);
	const [providers, setProviders] = useState<Provider[]>([]);
	const [connecting, setConnecting] = useState<Provider | null>(null);
	// With one service connected the choices fold away; this opens them for another.
	const [adding, setAdding] = useState(false);
	// The popular services are cards; the long tail waits behind More.
	const [more, setMore] = useState(false);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [connection, setConnection] = useState<Connection>("connecting");
	const { defaultBackendId } = useRoomSettings();
	useEffect(() => wire.onConnection(setConnection), []);

	const read = useCallback(() => {
		void wire
			.command("welcome", {})
			.then((next) => {
				setState(next);
				setRefusal(null);
			})
			.catch((error: Error) => setRefusal(error.message));
	}, []);

	/* Read once the socket is up, and again after a reconnect: the screen
	 * mounts with the window, before the wire has dialled. A default set
	 * on another seat, or in Settings, changes what is done. */
	useEffect(() => {
		if (connection !== "open") return;
		read();
		if (providers.length > 0) return;
		void wire
			.command("providers.list", {})
			.then((list) => setProviders(list.slice().sort((a, b) => a.name.localeCompare(b.name))))
			.catch((error: Error) => setRefusal(error.message));
		// The list is asked for while empty; asking again on every read is noise.
	}, [connection, read, defaultBackendId]);

	/* The first read decides where a returning person lands: a room that can
	 * already run has nothing left before the first teammate. Once only. */
	const placed = useRef(false);
	useEffect(() => {
		if (state === null || placed.current) return;
		placed.current = true;
		if (state.canRun) {
			setKind(state.defaultBackendId === HOTLINE_AGENT ? "hotline" : "harness");
			setPage("teammate");
		}
	}, [state]);

	const setDefault = async (id: string) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("settings.update", { patch: { defaultBackendId: id } });
			read();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const connected = state?.providers ?? [];
	const harnesses = state?.harnesses ?? [];
	const harnessChosen = harnesses.find((one) => one.id === state?.defaultBackendId);

	/** Going forward from the engine page also makes the choice the room's default, so the first teammate lands on it. */
	const next = async () => {
		if (page === "hello") setPage("why");
		else if (page === "why") setPage("engine");
		else if (page === "engine") {
			if (kind === "hotline") {
				if (state?.defaultBackendId !== HOTLINE_AGENT) await setDefault(HOTLINE_AGENT);
				setPage("providers");
			} else if (kind === "harness") setPage("harness");
		} else if (page === "providers" || page === "harness") setPage("teammate");
	};
	const back = () => {
		setConnecting(null);
		if (page === "why") setPage("hello");
		else if (page === "engine") setPage("why");
		else if (page === "providers" || page === "harness") setPage("engine");
		else if (page === "teammate") setPage(kind === "harness" ? "harness" : kind === "hotline" ? "providers" : "engine");
	};
	const canNext =
		page === "why" ||
		(page === "engine" && kind !== null && !busy) ||
		(page === "providers" && connected.length > 0 && connecting === null) ||
		(page === "harness" && harnessChosen !== undefined);

	if (page === "hello") {
		return (
			<Stage page={page}>
				<div className="welcome-hero">
					<span className="welcome-mark">
						<HotlineMark width={112} plain />
					</span>
					<h1 className="welcome-title">Welcome to Hotline</h1>
					<p className="welcome-lead">
						A team of AI helpers that work together for you. You chat with them like colleagues, and they get things done.
					</p>
				</div>
				<div className="welcome-actions">
					<button type="button" className="control btn btn-primary welcome-cta" onClick={() => void next()}>
						Get started
					</button>
					{onConnectServer !== null && (
						<button type="button" className="control btn-quiet" onClick={onConnectServer}>
							Connect to a server instead
						</button>
					)}
				</div>
			</Stage>
		);
	}

	return (
		<Stage page={connecting === null ? page : `${page}-${connecting.id}`}>
			<Progress page={page} />

			{page === "why" && (
				<>
					<Lead title="Why Hotline" text="Other AI helpers live in someone else's cloud. Yours live with you." />
					<div className="welcome-pillars">
						{PILLARS.map((pillar) => (
							<div key={pillar.title} className="welcome-pillar">
								<span className="welcome-pillar-icon">{pillar.icon}</span>
								<span className="welcome-pillar-title">{pillar.title}</span>
								<span className="welcome-pillar-text">{pillar.text}</span>
							</div>
						))}
					</div>
				</>
			)}

			{page === "engine" && (
				<>
					<Lead title="How your teammates think" text="Teammates need an AI to think with. Pick whichever is easiest for you." />
					<div role="radiogroup" aria-label="How new teammates think" className="flex flex-col gap-2.5">
						<Card
							checked={kind === "hotline"}
							title="Hotline Agent"
							badge="Recommended"
							text="Hotline's built-in helper. Connect an AI service you already use, like ChatGPT, Claude or Grok, and it does the rest."
							onPick={() => setKind("hotline")}
						/>
						<Card
							checked={kind === "harness"}
							disabled={harnesses.length === 0}
							title="An AI coding tool you already have"
							text={
								harnesses.length === 0
									? "For people who use tools like Codex, Claude Code or Cursor. None is on this computer."
									: `Use ${knownTools(harnesses.map((one) => one.name))}, signed in with your own account.`
							}
							onPick={() => setKind("harness")}
						/>
					</div>
					<p className="welcome-note">Not sure? Pick Hotline Agent. You can change this for any teammate later.</p>
				</>
			)}

			{page === "providers" &&
				(connecting !== null ? (
					<ConnectProvider
						key={connecting.id}
						provider={connecting}
						onConnected={() => {
							setConnecting(null);
							setAdding(false);
							read();
						}}
						onCancel={() => {
							// A sign-in that completed just before Cancel is already recorded.
							setConnecting(null);
							read();
						}}
					/>
				) : (
					<>
						<Lead
							title="Connect an AI service"
							text="This is where your teammates' thinking comes from. Sign in with an account you already have, or paste a key. It's kept safe in your computer's keychain."
						/>
						{connected.length > 0 && (
							<div className="welcome-connected">
								<span className="welcome-connected-tick">
									<CheckIcon />
								</span>
								<span className="flex min-w-0 flex-1 flex-col gap-1.5">
									<span className="welcome-card-title">You're connected</span>
									<span className="flex flex-wrap gap-1">
										{connected.map((one) => (
											<span key={one} className="provider-chip">
												{one}
											</span>
										))}
									</span>
								</span>
								<button type="button" className="control btn-quiet shrink-0" aria-expanded={adding} onClick={() => setAdding((was) => !was)}>
									{adding ? (
										"Hide"
									) : (
										<span className="flex items-center gap-1.5">
											<PlusIcon /> Add another
										</span>
									)}
								</button>
							</div>
						)}
						{(connected.length === 0 || adding) && (
							<>
								<div className="welcome-services">
									{popular(providers).map(({ provider, title, by }) => {
										const done = connected.includes(provider.name);
										return (
											<button
												key={provider.id}
												type="button"
												className="welcome-service"
												data-done={done ? "" : undefined}
												disabled={busy || done}
												onClick={() => setConnecting(provider)}
											>
												<span className="welcome-service-title">
													{title}
													{done && <CheckIcon className="text-accent" />}
												</span>
												<span className="welcome-service-text">
													{done ? "Connected" : [by, provider.credentialKinds.map(connectionMethod).join(" or ")].filter(Boolean).join(" · ")}
												</span>
											</button>
										);
									})}
								</div>
								{providers.length > 0 && (
									<button type="button" className="control btn-quiet self-center" aria-expanded={more} onClick={() => setMore((was) => !was)}>
										{more ? "Fewer services" : `${rest(providers).length} more services, like Gemini and Mistral`}
									</button>
								)}
								{more && (
									<div className="grouped welcome-list">
										{rest(providers)
											.filter((provider) => !connected.includes(provider.name))
											.map((provider) => (
												<ProviderRow key={provider.id} provider={provider} disabled={busy} onPick={() => setConnecting(provider)} />
											))}
									</div>
								)}
							</>
						)}
						<p className="welcome-note">
							{connected.length === 0 ? "One is enough to start. You can add more later in Settings." : "That's all you need. You can add more any time in Settings."}
						</p>
					</>
				))}

			{page === "harness" && (
				<>
					<Lead title="Pick your tool" text="Your teammates will use it with your own account. The first time, it may ask you to sign in." />
					<div role="radiogroup" aria-label="The tool new teammates use" className="flex flex-col gap-2.5">
						{harnesses.map((one) => (
							<Card
								key={one.id}
								checked={state?.defaultBackendId === one.id}
								disabled={busy}
								title={one.name}
								text={one.description}
								onPick={() => void setDefault(one.id)}
							/>
						))}
					</div>
				</>
			)}

			{page === "teammate" && (
				<>
					<Lead title="Meet your first teammate" text="Give it a name and say what it should help with. Then say hello." />
					<div className="welcome-panel">
						<NewTeammateForm models={models} goal={FIRST_GOAL} submitLabel="Add teammate" onCreated={onCreated} simple />
					</div>
					<p className="welcome-note">You can give it more abilities later, like its own computer or work it does while you're away.</p>
				</>
			)}

			{refusal !== null && <Refusal message={refusal} />}

			{connecting === null && page === "providers" && connected.length > 0 && !adding ? (
				// Connected and nothing more to do here: going on is the one thing to press.
				<div className="welcome-actions">
					<button type="button" className="control btn btn-primary welcome-cta" onClick={() => void next()}>
						Continue
					</button>
					<button type="button" className="control btn-quiet" onClick={back}>
						Back
					</button>
				</div>
			) : connecting === null && (
				<div className="welcome-nav">
					<button type="button" className="control btn-quiet" onClick={back}>
						Back
					</button>
					{page !== "teammate" && (
						<button type="button" className="control btn btn-primary welcome-next" disabled={!canNext} onClick={() => void next()}>
							Continue
						</button>
					)}
				</div>
			)}
		</Stage>
	);
}

/**
 * The services most people already pay for, by the name the desk lists
 * them under, with the name people call them by. They are cards; the rest
 * of the desk's list waits behind More, so the page fits without scrolling.
 */
const POPULAR: { name: string; title: string; by?: string }[] = [
	{ name: "Anthropic", title: "Anthropic", by: "Claude" },
	{ name: "ChatGPT", title: "ChatGPT", by: "Your plan" },
	{ name: "OpenRouter", title: "OpenRouter", by: "Every model" },
	{ name: "xAI", title: "xAI", by: "Grok" },
	{ name: "Ollama Local", title: "Ollama", by: "On this computer" },
	{ name: "GitHub Copilot", title: "Copilot", by: "GitHub" },
];

function popular(providers: Provider[]): { provider: Provider; title: string; by: string | undefined }[] {
	return POPULAR.flatMap((one) => {
		const provider = providers.find((candidate) => candidate.name === one.name);
		return provider === undefined ? [] : [{ provider, title: one.title, by: one.by }];
	});
}

function rest(providers: Provider[]): Provider[] {
	return providers.filter((provider) => !POPULAR.some((one) => one.name === provider.name));
}

/** The tools people know by name, in this order, when this machine has them; the rest are counted, not listed. */
const KNOWN_TOOLS = ["Claude Code", "Codex", "Cursor", "Gemini CLI", "Grok Build", "GitHub Copilot"];

function knownTools(names: string[]): string {
	const known = KNOWN_TOOLS.filter((one) => names.includes(one)).slice(0, 3);
	const shown = known.length > 0 ? known : names.slice(0, 3);
	const rest = names.length - shown.length;
	if (rest > 0) return `${shown.join(", ")} or ${rest} more`;
	if (shown.length <= 1) return shown[0] ?? "";
	return `${shown.slice(0, -1).join(", ")} or ${shown[shown.length - 1]}`;
}

/** The whole window, the content centred on it; a new page fades in rather than cutting. */
function Stage({ page, children }: { page: string; children: ReactNode }) {
	return (
		<div className="welcome">
			<div key={page} className="welcome-stage">
				{children}
			</div>
		</div>
	);
}

function Lead({ title, text }: { title: string; text: string }) {
	return (
		<div className="flex flex-col items-center gap-2 text-center">
			<h2 className="welcome-heading">{title}</h2>
			<p className="welcome-lead">{text}</p>
		</div>
	);
}

/** Where the person is: every step by name, the ones behind checked. */
function Progress({ page }: { page: Page }) {
	const at = STEPS.findIndex((step) => step.pages.includes(page));
	return (
		<ol className="welcome-progress" aria-label="Setup steps">
			{STEPS.map((step, index) => (
				<li key={step.title} data-state={index < at ? "done" : index === at ? "now" : "later"} aria-current={index === at ? "step" : undefined}>
					<span aria-hidden="true" className="welcome-progress-dot">
						{index < at ? <CheckIcon /> : index + 1}
					</span>
					<span className="welcome-progress-label">{step.title}</span>
				</li>
			))}
		</ol>
	);
}

/** One choice as a card: the whole card is the control, and the chosen one wears the accent. */
function Card({
	checked,
	disabled,
	title,
	badge,
	text,
	onPick,
}: {
	checked: boolean;
	disabled?: boolean;
	title: string;
	badge?: string;
	text: string;
	onPick(): void;
}) {
	return (
		<label className="welcome-card" data-checked={checked ? "" : undefined} data-off={disabled ? "true" : undefined}>
			<input type="radio" className="radio" checked={checked} disabled={disabled} onChange={onPick} />
			<span className="flex min-w-0 flex-col gap-1">
				<span className="welcome-card-title">
					{title}
					{badge !== undefined && <span className="welcome-badge">{badge}</span>}
				</span>
				<span className="welcome-card-text">{text}</span>
			</span>
		</label>
	);
}

const glyph = {
	width: 20,
	height: 20,
	viewBox: "0 0 20 20",
	fill: "none",
	stroke: "currentColor",
	strokeWidth: 1.6,
	strokeLinecap: "round" as const,
	strokeLinejoin: "round" as const,
	"aria-hidden": true as const,
};

function HomeGlyph() {
	return (
		<svg {...glyph}>
			<path d="M3.5 9 10 3.5 16.5 9v7a1 1 0 0 1-1 1h-3.5v-5h-4v5H4.5a1 1 0 0 1-1-1V9Z" />
		</svg>
	);
}

function SparkGlyph() {
	return (
		<svg {...glyph}>
			<path d="M10 2.5c.6 3.6 1.9 4.9 5.5 5.5-3.6.6-4.9 1.9-5.5 5.5-.6-3.6-1.9-4.9-5.5-5.5 3.6-.6 4.9-1.9 5.5-5.5Z" />
			<path d="M15.5 13.5c.2 1.3.7 1.8 2 2-1.3.2-1.8.7-2 2-.2-1.3-.7-1.8-2-2 1.3-.2 1.8-.7 2-2Z" />
		</svg>
	);
}

function ScreenGlyph() {
	return (
		<svg {...glyph}>
			<rect x="2.75" y="3.5" width="14.5" height="10" rx="1.5" />
			<path d="M7.5 16.75h5M10 13.5v3.25" />
		</svg>
	);
}

function PhoneGlyph() {
	return (
		<svg {...glyph}>
			<rect x="5.75" y="2.25" width="8.5" height="15.5" rx="2" />
			<path d="M9 14.75h2" />
		</svg>
	);
}
