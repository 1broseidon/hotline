import { useEffect, useState } from "react";
import type { Credential, Report } from "../generated/contract";
import { wire } from "../wire";
import { Sheet } from "./Sheet";

/** The providers Toad can run a model on today, in the order they are offered. */
const PROVIDERS = [
	{ id: "anthropic", name: "Anthropic" },
	{ id: "openai", name: "OpenAI" },
	{ id: "openrouter", name: "OpenRouter" },
] as const;

/** A chapter closes after this many idle hours unless someone has set another. */
const DEFAULT_IDLE_HOURS = 8;
const MIN_IDLE_HOURS = 1;
const MAX_IDLE_HOURS = 336;

/**
 * One line on the room stream. Settings are folded from `kind: "setting"`:
 * `id` is the key, `value` the value, and `deleted` puts the default back.
 */
type RoomItem = {
	kind?: string;
	id?: string;
	value?: unknown;
	deleted?: boolean;
};

/**
 * The room's preferences, as this window reads them.
 *
 * There is no `settings.get`. The room stream is the store, so a subscription
 * to `"room"` and a fold of its setting events is the whole of reading, and
 * `settings.update` is the whole of writing.
 */
export function Settings({ onClose }: { onClose(): void }) {
	const settings = useRoomSettings();
	const [held, setHeld] = useState<Credential[]>([]);
	const [providerId, setProviderId] = useState<string>(PROVIDERS[0].id);
	const [secret, setSecret] = useState("");
	const [hours, setHours] = useState(String(DEFAULT_IDLE_HOURS));
	const [from, setFrom] = useState(previousToadDir);
	const [report, setReport] = useState<Report | null>(null);
	const [busy, setBusy] = useState<"key" | "import" | null>(null);
	const [refusal, setRefusal] = useState<string | null>(null);

	useEffect(() => {
		wire
			.command("credential.list", {})
			.then(setHeld)
			.catch((error: Error) => setRefusal(error.message));
	}, []);

	useEffect(() => {
		setHours(String(settings.chapterIdleHours));
	}, [settings.chapterIdleHours]);

	const saveKey = async () => {
		if (!secret.trim() || busy) return;
		setBusy("key");
		setRefusal(null);
		try {
			const label = PROVIDERS.find((one) => one.id === providerId)?.name ?? providerId;
			const made = await wire.command("credential.create", {
				providerId,
				label,
				secret: secret.trim(),
			});
			setHeld((known) => [...known, made]);
			setSecret("");
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(null);
		}
	};

	const saveHours = (raw: string) => {
		setHours(raw);
		const next = Number(raw);
		if (!Number.isInteger(next) || next < MIN_IDLE_HOURS || next > MAX_IDLE_HOURS) return;
		if (next === settings.chapterIdleHours) return;
		void wire.command("settings.update", { patch: { chapterIdleHours: next } }).catch((error: Error) => {
			setRefusal(error.message);
		});
	};

	const runImport = async () => {
		const path = from.trim();
		if (!path || busy) return;
		setBusy("import");
		setRefusal(null);
		setReport(null);
		try {
			setReport(await wire.command("room.import", { from: path }));
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(null);
		}
	};

	return (
		<Sheet title="Settings" onClose={onClose}>
			<div className="flex flex-col gap-6">
				<section className="flex flex-col gap-3">
					<h3 className="text-xs font-medium uppercase tracking-wider text-ink-3">Keys</h3>
					{held.length > 0 && (
						<ul className="flex flex-col gap-1">
							{held.map((one) => (
								<li
									key={one.id}
									className="flex items-center gap-2 rounded-lg bg-paper-3 px-2.5 py-1.5 text-xs"
								>
									<span className="font-medium text-ink-2">{one.label}</span>
									<span className="font-mono text-ink-3">{one.providerId}</span>
									<span className="ml-auto text-ink-3">{one.revoked ? "revoked" : "in use"}</span>
								</li>
							))}
						</ul>
					)}
					<form
						className="flex flex-col gap-3"
						onSubmit={(event) => {
							event.preventDefault();
							void saveKey();
						}}
					>
						<div>
							<label className="label" htmlFor="key-provider">
								Provider
							</label>
							<select
								id="key-provider"
								className="field"
								value={providerId}
								onChange={(event) => setProviderId(event.target.value)}
							>
								{PROVIDERS.map((one) => (
									<option key={one.id} value={one.id}>
										{one.name}
									</option>
								))}
							</select>
						</div>
						<div>
							<label className="label" htmlFor="key-secret">
								API key
							</label>
							<input
								id="key-secret"
								type="password"
								className="field font-mono text-xs"
								spellCheck={false}
								value={secret}
								onChange={(event) => setSecret(event.target.value)}
							/>
						</div>
						<div className="flex justify-end">
							<button
								type="submit"
								className="btn-primary"
								disabled={busy !== null || secret.trim() === ""}
							>
								Save key
							</button>
						</div>
					</form>
				</section>

				<section className="flex flex-col gap-3 border-t border-rule pt-6">
					<h3 className="text-xs font-medium uppercase tracking-wider text-ink-3">General</h3>
					<div>
						<label className="label" htmlFor="setting-idle">
							Chapters close after
						</label>
						<div className="flex items-center gap-2">
							<input
								id="setting-idle"
								type="number"
								className="field w-24"
								min={MIN_IDLE_HOURS}
								max={MAX_IDLE_HOURS}
								step={1}
								value={hours}
								onChange={(event) => saveHours(event.target.value)}
							/>
							<span className="text-xs text-ink-3">hours idle</span>
						</div>
						<p className="mt-1 text-xs leading-relaxed text-ink-3">
							How long a teammate sits quiet before its working context closes. Eight hours is a
							night&rsquo;s sleep.
						</p>
					</div>
					<div>
						<label className="label" htmlFor="setting-backend">
							Default backend
						</label>
						<input
							id="setting-backend"
							className="field font-mono text-xs"
							value={settings.defaultBackendId}
							readOnly
						/>
						<p className="mt-1 text-xs leading-relaxed text-ink-3">
							What a new teammate runs on. Only Toad Agent (pi) is wired in this build.
						</p>
					</div>
				</section>

				<section className="flex flex-col gap-3 border-t border-rule pt-6">
					<h3 className="text-xs font-medium uppercase tracking-wider text-ink-3">Import</h3>
					<div>
						<label className="label" htmlFor="import-from">
							Previous Toad data directory
						</label>
						<input
							id="import-from"
							className="field font-mono text-xs"
							spellCheck={false}
							value={from}
							onChange={(event) => setFrom(event.target.value)}
						/>
					</div>
					<div className="flex justify-end">
						<button
							type="button"
							className="btn-primary"
							disabled={busy !== null || from.trim() === ""}
							onClick={() => void runImport()}
						>
							{busy === "import" ? "Importing…" : "Import"}
						</button>
					</div>
					{report !== null && <ImportReport report={report} />}
				</section>

				{refusal !== null && <p className="text-xs text-[var(--danger)]">{refusal}</p>}

				<div className="flex justify-end">
					<button type="button" className="btn-quiet" onClick={onClose}>
						Done
					</button>
				</div>
			</div>
		</Sheet>
	);
}

function ImportReport({ report }: { report: Report }) {
	return (
		<div className="rounded-lg bg-paper-3 px-2.5 py-2 text-xs text-ink-2">
			<p>
				{report.teammates} teammate{report.teammates === 1 ? "" : "s"} · {report.tapes} tape
				{report.tapes === 1 ? "" : "s"} · {report.settings} setting
				{report.settings === 1 ? "" : "s"} · {report.keys} key
				{report.keys === 1 ? "" : "s"}
			</p>
			{report.skipped.length > 0 && (
				<ul className="mt-2 flex flex-col gap-1 text-ink-3">
					{report.skipped.map((one, index) => (
						<li key={`${one.item}:${index}`}>
							<span className="text-ink-2">{one.item}</span>
							{`: ${one.reason}`}
						</li>
					))}
				</ul>
			)}
		</div>
	);
}

/**
 * Subscribe to the room and fold its setting events over the defaults. A
 * reconnect delivers a fresh snapshot, so the map is replaced rather than
 * merged.
 */
function useRoomSettings(): { chapterIdleHours: number; defaultBackendId: string } {
	const [events, setEvents] = useState<Map<string, RoomItem>>(new Map());

	useEffect(() => {
		return wire.subscribe<RoomItem>("room", {
			snapshot: (items) => setEvents(takeSettings(items)),
			event: (item) => {
				if (item.kind !== "setting" || typeof item.id !== "string") return;
				const id = item.id;
				setEvents((known) => {
					const next = new Map(known);
					next.set(id, item);
					return next;
				});
			},
		});
	}, []);

	return {
		chapterIdleHours: numberSetting(events.get("chapterIdleHours"), DEFAULT_IDLE_HOURS),
		defaultBackendId: stringSetting(events.get("defaultBackendId"), "pi"),
	};
}

function takeSettings(items: RoomItem[]): Map<string, RoomItem> {
	const map = new Map<string, RoomItem>();
	for (const item of items) {
		if (item.kind === "setting" && typeof item.id === "string") map.set(item.id, item);
	}
	return map;
}

function numberSetting(event: RoomItem | undefined, fallback: number): number {
	if (!event || event.deleted || typeof event.value !== "number") return fallback;
	return event.value;
}

function stringSetting(event: RoomItem | undefined, fallback: string): string {
	if (!event || event.deleted || typeof event.value !== "string") return fallback;
	return event.value;
}

/**
 * Where the previous Toad keeps its data. The window does not know `$HOME`,
 * so this is the path that edition uses, written the way a person would type
 * it. The core receives the string as typed.
 */
function previousToadDir(): string {
	const platform = window.__toadDesk?.platform ?? "linux";
	if (platform === "macos") return "~/Library/Application Support/Toad";
	if (platform === "windows") return "~/AppData/Roaming/Toad";
	return "~/.local/share/toad";
}
