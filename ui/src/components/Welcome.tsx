import { useCallback, useEffect, useState } from "react";
import type { ConfigChoice, Provider, Welcome as WelcomeState } from "../generated/contract";
import { CheckIcon } from "../icons";
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

/**
 * The room before anyone is in it: three steps to a first conversation, in
 * place of a placeholder that only said to add a teammate.
 *
 * Which step is open is derived from what the room knows — a live
 * credential or a startable harness made the default, then the roster —
 * and never from a flag: this pane exists exactly as long as there is no
 * teammate to show, and a room with one never sees it. Step one renders the
 * same forms as Settings › Providers, so nothing is learned twice; step two
 * is the same form as the plus. Step three lives on the first conversation's
 * composer and is only named here.
 */
export function Welcome({ models, onCreated }: { models: ConfigChoice[]; onCreated(personaId: string): void }) {
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
	 * on another seat, or in Settings, changes what step one says. */
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

	const useHarness = async (id: string) => {
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

	const canRun = state?.canRun ?? false;
	const harness = state?.harnesses.find((one) => one.id === state.defaultBackendId);
	const runsOn =
		state === null
			? ""
			: state.providers.length > 0
				? `${state.providers.join(", ")} connected.`
				: harness !== undefined
					? `Teammates run on ${harness.name}.`
					: "";

	return (
		<div className="pane">
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Welcome</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					<p className="text-ink-2">
						A teammate is an agent with a name, a goal and a folder of its own. Three steps to the first conversation.
					</p>

					<Step
						n={1}
						title="A way to run agents"
						sentence="Connect a provider with a key or a sign-in, or make a harness on this machine the room's default."
						done={canRun}
						doneText={runsOn}
						open={state !== null && !canRun}
					>
						{connecting !== null ? (
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
								<section>
									<h3 className="group-title">Providers</h3>
									<div className="grouped">
										{providers.length === 0 ? (
											<p className="group-row text-sm text-ink-3">Reading…</p>
										) : (
											providers.map((provider) => (
												<ProviderRow key={provider.id} provider={provider} disabled={busy} onPick={() => setConnecting(provider)} />
											))
										)}
									</div>
									<p className="group-hint">Toad Agent runs on these. Keys stay in your OS credential store.</p>
								</section>
								{state !== null && state.harnesses.length > 0 && (
									<section>
										<h3 className="group-title">Harnesses on this machine</h3>
										<div className="grouped">
											{state.harnesses.map((one) => (
												<div key={one.id} className="group-row">
													<span className="group-row-text">
														<span className="group-row-title">{one.name}</span>
														<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
															{one.description}
														</span>
													</span>
													<button type="button" className="control btn" disabled={busy} onClick={() => void useHarness(one.id)}>
														Use
													</button>
												</div>
											))}
										</div>
										<p className="group-hint">A harness brings its own sign-in and models. Choosing one makes it what new teammates run on.</p>
									</section>
								)}
							</>
						)}
					</Step>

					<Step
						n={2}
						title="Your first teammate"
						sentence="A name, a goal and a folder; they start as soon as you add them."
						done={(state?.teammates ?? 0) > 0}
						doneText=""
						open={canRun}
					>
						<NewTeammateForm models={models} goal={FIRST_GOAL} submitLabel="Add teammate" onCreated={onCreated} />
					</Step>

					<Step n={3} title="Say hello" sentence="The first conversation opens with a few things to try." done={false} doneText="" open={false} />

					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}

/**
 * One step: its number or a check, its name, one sentence, and — while it
 * is the step to do — what doing it takes.
 */
function Step({
	n,
	title,
	sentence,
	done,
	doneText,
	open,
	children,
}: {
	n: number;
	title: string;
	sentence: string;
	done: boolean;
	doneText: string;
	open: boolean;
	children?: React.ReactNode;
}) {
	return (
		<section aria-labelledby={`welcome-step-${n}`} className="flex gap-3">
			<span
				aria-hidden="true"
				className={`mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-sm font-medium ${
					done ? "bg-accent text-white" : open ? "bg-ink text-well" : "bg-fill text-ink-3"
				}`}
			>
				{done ? <CheckIcon /> : n}
			</span>
			<div className="flex min-w-0 flex-1 flex-col gap-3">
				<div>
					<h3 id={`welcome-step-${n}`} className={`font-semibold ${open || done ? "text-ink" : "text-ink-3"}`}>
						{title}
					</h3>
					<p className="text-sm text-ink-3">{done && doneText !== "" ? doneText : sentence}</p>
				</div>
				{open && children}
			</div>
		</section>
	);
}
