import { useEffect, useState } from "react";
import type { BackendChoice, ConfigChoice } from "../../generated/contract";
import { DEFAULT_IDLE_HOURS, useModelsRevision } from "../../room";
import { Chips } from "../../ui/Chips";
import { Fold, toggled } from "../../ui/Fold";
import { Picker } from "../../ui/Menu";
import { setTheme, THEMES, useTheme } from "../../theme";
import { HOTLINE_AGENT } from "../accounts/fits";
import { wire } from "../../wire";
import { BackendPicker } from "../BackendPicker";
import { HotkeysSection } from "../HotkeysSection";

const MIN_IDLE_HOURS = 1;
const MAX_IDLE_HOURS = 336;

function AppearanceSection() {
	const theme = useTheme();
	return (
		<section>
			<h3 className="group-title">Appearance</h3>
			<div className="grouped">
				<div className="group-row">
					<span className="group-row-text">
						<span className="group-row-title">Theme</span>
					</span>
					<Chips value={theme} choices={THEMES} label="Theme" onChange={setTheme} />
				</div>
			</div>
		</section>
	);
}

export function GeneralSection({
	personName,
	onPersonName,
	idleHours,
	defaultBackendId,
	defaultModelId,
	onIdleHours,
	onBackend,
	onDefaultModel,
	onProviders,
}: {
	personName: string;
	onPersonName(name: string): void;
	idleHours: number;
	defaultBackendId: string;
	defaultModelId: string | null;
	onIdleHours(hours: number): void;
	onBackend(id: string): void;
	onDefaultModel(id: string | null): void;
	onProviders: (() => void) | undefined;
}) {
	const [name, setName] = useState(personName);
	useEffect(() => setName(personName), [personName]);
	const [hours, setHours] = useState(String(idleHours));
	const [backends, setBackends] = useState<BackendChoice[]>([]);
	const [models, setModels] = useState<ConfigChoice[]>([]);

	useEffect(() => {
		setHours(String(idleHours));
	}, [idleHours]);

	const modelsRevision = useModelsRevision();

	useEffect(() => {
		void wire
			.command("backends.list", {})
			.then(setBackends)
			.catch(() => setBackends([]));
	}, []);

	// biome-ignore lint/correctness/useExhaustiveDependencies: modelsRevision is the room saying the list changed.
	useEffect(() => {
		let current = true;
		void wire
			.command("models.list", {})
			.then((next) => current && setModels(next))
			.catch(() => current && setModels([]));
		return () => {
			current = false;
		};
	}, [modelsRevision]);

	const commitHours = (raw: string) => {
		setHours(raw);
		const next = Number(raw);
		if (!Number.isInteger(next) || next < MIN_IDLE_HOURS || next > MAX_IDLE_HOURS) return;
		if (next !== idleHours) onIdleHours(next);
	};

	const [open, setOpen] = useState<"agent" | "model" | null>(null);
	const agentName = backends.find((one) => one.id === defaultBackendId)?.name ?? "Hotline Agent";
	const modelName = defaultModelId === null ? "Last used" : (models.find((one) => one.id === defaultModelId)?.name ?? defaultModelId);

	return (
		<>
			<section>
				<h3 className="group-title">You</h3>
				<div className="grouped">
					<div className="group-row">
						<label className="group-row-text" htmlFor="setting-person-name">
							<span className="group-row-title">Your name</span>
						</label>
						<input
							id="setting-person-name"
							className="field w-48"
							autoComplete="given-name"
							maxLength={100}
							value={name}
							onChange={(event) => setName(event.target.value)}
							onBlur={() => {
								const next = name.trim();
								setName(next);
								if (next !== personName) onPersonName(next);
							}}
							onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }}
						/>
					</div>
				</div>
			</section>
			<AppearanceSection />
			<HotkeysSection />
			<section>
				<h3 className="group-title">New teammates</h3>
				<div className="grouped">
					<Fold title="Thinks with" value={agentName} open={open === "agent"} onToggle={() => setOpen((was) => toggled(was, "agent"))}>
						{backends.length > 0 ? (
							<BackendPicker
								backends={backends}
								selected={defaultBackendId}
								name="setting-backend"
								labelledBy="setting-backend"
								onSelect={onBackend}
								onProviders={onProviders}
							/>
						) : (
							<p className="text-sm text-ink-3">Reading which agents this computer can start…</p>
						)}
					</Fold>
					{(defaultBackendId === HOTLINE_AGENT || defaultBackendId === "") && <Fold title="Model" value={modelName} open={open === "model"} onToggle={() => setOpen((was) => toggled(was, "model"))}>
						<Picker
							field
							value={defaultModelId ?? ""}
							choices={[{ id: "", name: "Last used" }, ...models]}
							placeholder="Last used"
							label="Default model"
							onChange={(id) => onDefaultModel(id === "" ? null : id)}
						/>
					</Fold>}
				</div>
			</section>
			<section>
				<h3 className="group-title">Conversations</h3>
				<div className="grouped">
					<div className="group-row">
						<label className="group-row-text" htmlFor="setting-idle">
							<span className="group-row-title">Start a new chapter after</span>
						</label>
						<span className="flex items-center gap-2 text-sm text-ink-2">
							<input
								id="setting-idle"
								type="number"
								className="field w-16 text-right"
								min={MIN_IDLE_HOURS}
								max={MAX_IDLE_HOURS}
								step={1}
								placeholder={String(DEFAULT_IDLE_HOURS)}
								value={hours}
								onChange={(event) => commitHours(event.target.value)}
							/>
							quiet hours
						</span>
					</div>
				</div>
			</section>
		</>
	);
}
