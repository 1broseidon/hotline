import { useCallback, useEffect, useState } from "react";
import type { ConfigChoice } from "./generated/contract";
import { useTape } from "./tape";
import { wire, type Connection, type RosterEntry } from "./wire";
import { ChatHeader } from "./components/ChatHeader";
import { Composer } from "./components/Composer";
import { Keys } from "./components/Keys";
import { NewTeammate } from "./components/NewTeammate";
import { Rail } from "./components/Rail";
import { Transcript } from "./components/Transcript";

type SheetKind = "new-teammate" | "keys" | null;

export function App() {
	const [connection, setConnection] = useState<Connection>("connecting");
	const [roster, setRoster] = useState<RosterEntry[]>([]);
	const [models, setModels] = useState<ConfigChoice[]>([]);
	const [selectedId, setSelectedId] = useState<string | null>(null);
	const [sheet, setSheet] = useState<SheetKind>(null);

	useEffect(() => {
		wire.connect();
		return wire.onConnection(setConnection);
	}, []);

	useEffect(() => {
		return wire.subscribe<RosterEntry>(
			{ view: "roster" },
			{
				snapshot: setRoster,
				event: (entry) =>
					setRoster((known) => {
						const at = known.findIndex((one) => one.persona.id === entry.persona.id);
						if (at === -1) return [...known, entry];
						const next = known.slice();
						next[at] = entry;
						return next;
					}),
				removed: (personaId) =>
					setRoster((known) => known.filter((one) => one.persona.id !== personaId)),
			},
		);
	}, []);

	/* The room's models are asked for once a socket is up, and again after a
	 * reconnect: a key added on another seat changes the answer. */
	useEffect(() => {
		if (connection !== "open") return;
		wire
			.command("models.list", {})
			.then(setModels)
			.catch(() => setModels([]));
	}, [connection]);

	const selected = roster.find((one) => one.persona.id === selectedId) ?? null;

	// Opening a teammate is Ctrl+1 through Ctrl+9, in the rail's own order; the
	// rail says so on each row, because a shortcut nobody can see is no
	// shortcut. Ctrl+N adds one, Ctrl+, is the keys.
	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			if (!event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return;
			if (event.key === "n") {
				event.preventDefault();
				setSheet("new-teammate");
				return;
			}
			if (event.key === ",") {
				event.preventDefault();
				setSheet("keys");
				return;
			}
			const seat = Number(event.key);
			if (!Number.isInteger(seat) || seat < 1 || seat > 9) return;
			const entry = roster[seat - 1];
			if (!entry) return;
			event.preventDefault();
			setSelectedId(entry.persona.id);
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [roster]);

	return (
		<div className="relative flex h-full">
			<Rail
				entries={roster}
				selectedId={selectedId}
				onSelect={setSelectedId}
				onNew={() => setSheet("new-teammate")}
			/>

			<main className="flex min-w-0 flex-1 flex-col bg-paper">
				{connection !== "open" && (
					<p className="bg-paper-3 px-6 py-1 text-center text-xs text-ink-3">
						{connection === "connecting" ? "Connecting to Toad…" : "Toad is not answering. Retrying…"}
					</p>
				)}
				{selected ? (
					<Conversation key={selected.persona.id} entry={selected} models={models} onOpenKeys={() => setSheet("keys")} />
				) : (
					<div className="flex flex-1 items-center justify-center px-6">
						<p className="max-w-sm text-center text-ink-3">
							Pick a teammate on the left, or add one.
						</p>
					</div>
				)}
			</main>

			{sheet === "new-teammate" && (
				<NewTeammate
					models={models}
					onCreated={(personaId) => {
						setSelectedId(personaId);
						setSheet(null);
					}}
					onClose={() => setSheet(null)}
				/>
			)}
			{sheet === "keys" && <Keys onClose={() => setSheet(null)} />}
		</div>
	);
}

/**
 * One teammate's conversation. Keyed by teammate above, so switching tears the
 * tape subscription down and puts up another rather than folding two
 * conversations into one column.
 */
function Conversation({
	entry,
	models,
	onOpenKeys,
}: {
	entry: RosterEntry;
	models: ConfigChoice[];
	onOpenKeys(): void;
}) {
	const personaId = entry.persona.id;
	const { events, streaming } = useTape(personaId);

	const send = useCallback(
		(text: string) => void wire.command("session.prompt", { personaId, text }),
		[personaId],
	);
	const start = useCallback(() => void wire.command("session.start", { personaId }), [personaId]);
	const cancel = useCallback(() => void wire.command("session.cancel", { personaId }), [personaId]);

	return (
		<>
			<ChatHeader
				entry={entry}
				models={models}
				onSetModel={(modelId) => void wire.command("session.set_model", { personaId, modelId })}
				onOpenKeys={onOpenKeys}
			/>
			<Transcript events={events} streaming={streaming} />
			<Composer
				personaId={personaId}
				state={entry.session.state}
				onSend={send}
				onStart={start}
				onCancel={cancel}
			/>
		</>
	);
}
