import { useCallback, useEffect, useRef, useState } from "react";
import type { ConfigChoice, Provider, Welcome as WelcomeState } from "../generated/contract";
import { CheckIcon, ChevronRightIcon } from "../icons";
import { useRoomSettings } from "../room";
import { Band } from "../ui/Band";
import { Refusal } from "../ui/Refusal";
import { Scroll } from "../ui/Scroll";
import { wire, type Connection } from "../wire";
import { ConnectProvider, ProviderRow } from "./ConnectProvider";
import { NewTeammateForm } from "./NewTeammate";

/**
 * The goal the first teammate is offered. A person who has never written a
 * goal for an agent should not have to start from a blank line; one who has
 * will replace it.
 */
const FIRST_GOAL =
	"Help me with whatever I bring you. Ask when something is unclear, and say what you did when you are done.";

const HOTLINE_AGENT = "hotline";

/** The pages, in order. `where` is only on this computer's own desk; `providers` and `harness` are the two forks of one step. */
type Page = "where" | "how" | "engine" | "providers" | "harness" | "teammate";

/** The steps the progress line names, and which pages belong to each. */
const STEPS: { title: string; pages: Page[] }[] = [
	{ title: "How it works", pages: ["how"] },
	{ title: "What runs it", pages: ["engine"] },
	{ title: "Connect", pages: ["providers", "harness"] },
	{ title: "First teammate", pages: ["teammate"] },
];

/**
 * The room before anyone is in it, as a short wizard that teaches while it
 * sets up: what a teammate, the room and the desk are; the two kinds of
 * agent a teammate can run on; connecting what that needs; then the first
 * teammate, with each field said in a sentence. A person who finishes it
 * has met every word the rest of the window uses.
 *
 * What is done is still derived from what the room knows — `welcome`'s
 * credentials, harnesses and roster — never from a stored flag: this pane
 * exists exactly as long as there is no teammate, and a room with one never
 * sees it. Only which page is in front is the window's own, so Back always
 * works; a room that can already run opens on the first teammate, the
 * earlier pages a Back away. The connect page renders the same forms as
 * Settings › Providers and the last page is the same form as the plus, so
 * nothing is learned twice.
 *
 * On this computer's own desk the pages wait behind one choice, of two
 * equals: teammates here, or teammates on a server the person runs
 * (BRO-151). `onConnectServer` opens that pane; it is null on a server's own
 * desk, which has nothing to choose between.
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
	const [page, setPage] = useState<Page>(onConnectServer === null ? "how" : "where");
	const [kind, setKind] = useState<"hotline" | "harness" | null>(null);
	const [state, setState] = useState<WelcomeState | null>(null);
	const [providers, setProviders] = useState<Provider[]>([]);
	const [connecting, setConnecting] = useState<Provider | null>(null);
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

	/* Read once the socket is up, and again after a reconnect: the pane
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
			setPage((was) => (was === "where" ? was : "teammate"));
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
	const providersReady = connected.length > 0;

	/** Going forward from the engine page also makes the choice the room's default, so the first teammate lands on it. */
	const next = async () => {
		if (page === "how") setPage("engine");
		else if (page === "engine") {
			if (kind === "hotline") {
				if (state?.defaultBackendId !== HOTLINE_AGENT) await setDefault(HOTLINE_AGENT);
				setPage("providers");
			} else if (kind === "harness") setPage("harness");
		} else if (page === "providers" || page === "harness") setPage("teammate");
	};
	const back = () => {
		setConnecting(null);
		if (page === "engine") setPage("how");
		else if (page === "providers" || page === "harness") setPage("engine");
		else if (page === "teammate") setPage(kind === "harness" ? "harness" : kind === "hotline" ? "providers" : "engine");
		else if (page === "how" && onConnectServer !== null) setPage("where");
	};
	const canNext =
		page === "how" ||
		(page === "engine" && kind !== null && !busy) ||
		(page === "providers" && providersReady && connecting === null) ||
		(page === "harness" && harnessChosen !== undefined);

	if (page === "where" && onConnectServer !== null) {
		return (
			<Frame onConnectServer={null}>
				<Lead title="Welcome to Hotline" text="A room for a team of agents, on your machine. First, where should they live?" />
				<Choice onHere={() => setPage("how")} onServer={onConnectServer} />
				{refusal !== null && <Refusal message={refusal} />}
			</Frame>
		);
	}

	return (
		<Frame onConnectServer={onConnectServer}>
			<Progress page={page} />

			{page === "how" && (
				<>
					<Lead title="How Hotline works" text="Three words, and you know your way around." />
					<Concepts
						rows={[
							{
								title: "Teammates",
								text: "Each is an agent with a name, a goal and a folder of its own. Talking to one is a conversation in the rail, like a chat.",
							},
							{
								title: "The room",
								text: "Where your teammates live together. They can ask each other for help and hand work over, and you see all of it.",
							},
							{
								title: "The desk",
								text: "The computer that runs the room — this one, or a server. It keeps the keys, the files and the history, and your phone can join it later.",
							},
						]}
					/>
				</>
			)}

			{page === "engine" && (
				<>
					<Lead
						title="What your teammates run on"
						text="Every teammate runs on an agent: the program that reads, writes and runs things for it. There are two kinds."
					/>
					<div role="radiogroup" aria-label="What new teammates run on" className="grouped">
						<EngineChoice
							checked={kind === "hotline"}
							title="Hotline Agent"
							text="Hotline's own agent. You pick the model, from any provider you connect: an API key, or a subscription you already pay for. It stays inside its folder unless you give it more."
							onPick={() => setKind("hotline")}
						/>
						<EngineChoice
							checked={kind === "harness"}
							disabled={harnesses.length === 0}
							title="An agent you already use"
							text={
								harnesses.length === 0
									? "Codex, Claude Code, Cursor and others. None is installed here yet; one you install shows up in Settings later."
									: `Runs ${listOf(harnesses.map((one) => one.name))} through its own CLI, with its own login, models and permissions.`
							}
							onPick={() => setKind("harness")}
						/>
					</div>
					<p className="group-hint">This is only the default. Each teammate can run on either, and you can mix them in one room.</p>
				</>
			)}

			{page === "providers" &&
				(connecting !== null ? (
					<ConnectProvider
						key={connecting.id}
						provider={connecting}
						onConnected={() => {
							setConnecting(null);
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
							title="Connect a provider"
							text="A provider is where Hotline Agent's models come from. Connect one with an API key, or sign in with a subscription. Keys stay in your OS keychain, and a provider can draw pictures and power voice calls too."
						/>
						{providersReady && (
							<div className="flex flex-wrap items-center gap-1.5">
								<span className="text-sm text-ink-2">Connected:</span>
								{connected.map((one) => (
									<span key={one} className="provider-chip">
										{one}
									</span>
								))}
							</div>
						)}
						<section>
							<div className="grouped">
								{providers.length === 0 ? (
									<p className="group-row text-sm text-ink-3">Reading…</p>
								) : (
									providers.map((provider) => (
										<ProviderRow key={provider.id} provider={provider} disabled={busy} onPick={() => setConnecting(provider)} />
									))
								)}
							</div>
							<p className="group-hint">One is enough to start. Add more any time in Settings › Providers.</p>
						</section>
					</>
				))}

			{page === "harness" && (
				<>
					<Lead
						title="Pick the agent"
						text="Hotline starts the agent's own CLI for each teammate. It signs in, picks its models and decides what it may touch the way it always does; the first start may ask you to sign in."
					/>
					<div role="radiogroup" aria-label="The agent new teammates run on" className="grouped">
						{harnesses.map((one) => (
							<EngineChoice
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
					<Lead title="Your first teammate" text="Give it a name and a job. It starts as soon as you add it, and the conversation opens with a few things to try." />
					<Concepts
						rows={[
							{ title: "Goal", text: "Written into its folder as AGENTS.md, so it reads it every time it starts." },
							{
								title: "Working directory",
								text: "Its own folder. Hotline Agent works inside it; Whole machine lets it reach everything else.",
							},
							{ title: "Agent", text: "Already set to what you picked. Change it here for this teammate only." },
						]}
					/>
					<NewTeammateForm models={models} goal={FIRST_GOAL} submitLabel="Add teammate" onCreated={onCreated} />
				</>
			)}

			{refusal !== null && <Refusal message={refusal} />}

			<div className="flex items-center justify-between gap-2 pt-2">
				{page !== "how" || onConnectServer !== null ? (
					<button type="button" className="control btn-quiet" onClick={back}>
						Back
					</button>
				) : (
					<span />
				)}
				{page !== "teammate" && connecting === null && (
					<button type="button" className="control btn btn-primary" disabled={!canNext} onClick={() => void next()}>
						Next
					</button>
				)}
			</div>
		</Frame>
	);
}

function listOf(names: string[]): string {
	if (names.length <= 1) return names[0] ?? "";
	return `${names.slice(0, -1).join(", ")} or ${names[names.length - 1]}`;
}

function Frame({ onConnectServer, children }: { onConnectServer: (() => void) | null; children: React.ReactNode }) {
	return (
		<div className="pane">
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Welcome</h2>
				{onConnectServer !== null && (
					<button type="button" className="control btn-quiet" onClick={onConnectServer}>
						Connect to a server
					</button>
				)}
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-5">{children}</div>
			</Scroll>
		</div>
	);
}

function Lead({ title, text }: { title: string; text: string }) {
	return (
		<div className="flex flex-col gap-1">
			<h3 className="text-xl font-semibold text-ink">{title}</h3>
			<p className="text-ink-2">{text}</p>
		</div>
	);
}

/** A few words, each with what it means, as one list. */
function Concepts({ rows }: { rows: { title: string; text: string }[] }) {
	return (
		<div className="grouped">
			{rows.map((row) => (
				<div key={row.title} className="group-row">
					<span className="group-row-text">
						<span className="group-row-title">{row.title}</span>
						<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
							{row.text}
						</span>
					</span>
				</div>
			))}
		</div>
	);
}

/** Where the person is: every step by name, the ones behind checked. */
function Progress({ page }: { page: Page }) {
	const at = STEPS.findIndex((step) => step.pages.includes(page));
	return (
		<ol className="flex items-center gap-2" aria-label="Setup steps">
			{STEPS.map((step, index) => (
				<li key={step.title} className="flex min-w-0 items-center gap-2" aria-current={index === at ? "step" : undefined}>
					<span
						aria-hidden="true"
						className={`flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-xs font-medium ${
							index < at ? "bg-accent text-white" : index === at ? "bg-ink text-well" : "bg-fill text-ink-3"
						}`}
					>
						{index < at ? <CheckIcon /> : index + 1}
					</span>
					<span className={`truncate text-sm ${index === at ? "text-ink" : "text-ink-3"}`}>{step.title}</span>
					{index < STEPS.length - 1 && <span aria-hidden="true" className="h-px w-4 shrink-0 bg-line" />}
				</li>
			))}
		</ol>
	);
}

function EngineChoice({
	checked,
	disabled,
	title,
	text,
	onPick,
}: {
	checked: boolean;
	disabled?: boolean;
	title: string;
	text: string;
	onPick(): void;
}) {
	return (
		<label className="group-row group-row-choice" data-off={disabled ? "true" : undefined}>
			<input type="radio" className="radio" checked={checked} disabled={disabled} onChange={onPick} />
			<span className="group-row-text">
				<span className="group-row-title">{title}</span>
				<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
					{text}
				</span>
			</span>
		</label>
	);
}

/**
 * Where the first teammates run, as two rows of one list: neither is the
 * primary, because neither is the right answer for everyone. Each says in one
 * line what it means.
 */
function Choice({ onHere, onServer }: { onHere(): void; onServer(): void }) {
	return (
		<section>
			<h3 className="group-title">Where your teammates run</h3>
			<div className="grouped">
				<button type="button" className="group-row group-row-choice w-full text-left" onClick={onHere}>
					<span className="group-row-text">
						<span className="group-row-title">On this computer</span>
						<span className="group-row-detail">Set up a first teammate here.</span>
					</span>
					<ChevronRightIcon className="shrink-0 text-ink-3" />
				</button>
				<button type="button" className="group-row group-row-choice w-full text-left" onClick={onServer}>
					<span className="group-row-text">
						<span className="group-row-title">On a server you run</span>
						<span className="group-row-detail">Connect to a Hotline server and use its teammates.</span>
					</span>
					<ChevronRightIcon className="shrink-0 text-ink-3" />
				</button>
			</div>
		</section>
	);
}
