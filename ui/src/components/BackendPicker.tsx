import { useEffect, useState, type KeyboardEvent, type ReactNode } from "react";
import type { BackendChoice } from "../generated/contract";
import { wire } from "../wire";
import { ChevronDownIcon, ChevronRightIcon } from "../icons";

/** Hotline Agent's stored backend id. The picker puts this row first even if
 *  the caller hands the array in another order. */
const HOTLINE_AGENT = "hotline";

/**
 * The harnesses shown above the fold, beside Hotline Agent: the ones with a
 * flagship model of their own, so choosing one is choosing a lab. The
 * rest of the catalogue is mostly multi-provider harnesses, which would
 * only compete with Hotline Agent for the same keys, so they wait behind
 * the disclosure — still there, still startable.
 */
const FEATURED = new Set(["claude-acp", "codex-acp", "cursor", "grok-build"]);

/**
 * Hotline Agent first, then the featured harnesses by name, each greyed with
 * the sentence that names what is missing when this machine cannot start
 * it. Everything else sits behind a disclosure, the startable ones first.
 *
 * New teammate and Settings › General share this component so the two
 * lists cannot drift.
 */
export function BackendPicker({
	backends,
	selected,
	name,
	labelledBy,
	onSelect,
	onProviders,
}: {
	backends: BackendChoice[];
	selected: string;
	name: string;
	labelledBy: string;
	onSelect(id: string): void;
	/** Opens Settings › Providers; without it the words are plain text. */
	onProviders?: (() => void) | undefined;
}) {
	const connected = useConnectedProviders();
	const { ready, more } = arrange(backends);
	const selectedIsMore = more.some((one) => one.id === selected);
	const [open, setOpen] = useState(selectedIsMore);

	useEffect(() => {
		if (selectedIsMore) setOpen(true);
	}, [selectedIsMore]);

	const stop =
		ready.some((one) => one.id === selected) || (open && more.some((one) => one.id === selected))
			? selected
			: (ready[0]?.id ?? "");

	const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
		if (event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== "ArrowRight" && event.key !== "ArrowLeft") {
			return;
		}
		// Native radios skip anything that is not a radio, so the disclosure
		// would be unreachable and hidden rows would still take an arrow.
		const rows = Array.from(event.currentTarget.querySelectorAll<HTMLElement>("[data-picker-row]"));
		if (rows.length === 0) return;
		const target = event.target as Node;
		const from = rows.findIndex((row) => row === target || row.contains(target) || target.contains(row));
		if (from < 0) return;
		event.preventDefault();
		const step = event.key === "ArrowUp" || event.key === "ArrowLeft" ? -1 : 1;
		const next = rows[(from + step + rows.length) % rows.length];
		if (next === undefined) return;
		next.focus();
		if (next.dataset.off === undefined && next.dataset.backendId !== undefined) {
			onSelect(next.dataset.backendId);
		}
	};

	const choice = (backend: BackendChoice) => (
		<Choice
			key={backend.id}
			backend={backend}
			detail={backend.id === HOTLINE_AGENT ? <HotlineAgentDetail connected={connected} onProviders={onProviders} /> : (backend.unavailable ?? backend.description)}
			off={backend.unavailable !== undefined}
			name={name}
			selected={selected}
			tabIndex={stop === backend.id ? 0 : -1}
			onSelect={onSelect}
		/>
	);
	const builtIn = ready.filter((one) => one.id === HOTLINE_AGENT);
	const external = ready.filter((one) => one.id !== HOTLINE_AGENT);

	// Two lists, not one: Hotline Agent is the agent and any model, the rest
	// are someone else's agent with its own login. In one list the two read
	// as the same kind of choice.
	return (
		<div role="radiogroup" aria-labelledby={labelledBy} className="flex flex-col" onKeyDown={onKey}>
			{builtIn.length > 0 && <div className="grouped">{builtIn.map(choice)}</div>}
			{(external.length > 0 || more.length > 0) && (
				<>
					<p className="group-title mt-4">Or an agent you already use</p>
					<div className="grouped">
						{external.map(choice)}
						{more.length > 0 && (
							<button
								type="button"
								data-picker-row=""
								tabIndex={stop === "" ? 0 : -1}
								className="group-row group-row-choice w-full text-left"
								aria-expanded={open}
								onClick={() => setOpen((was) => !was)}
							>
								{open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />}
								<span className="text-sm text-ink-3">More agents ({more.length})</span>
							</button>
						)}
						{open && more.map(choice)}
					</div>
					<p className="group-hint">Hotline starts that agent's own CLI. It signs in, picks its models and runs its tools its own way.</p>
				</>
			)}
		</div>
	);
}

function arrange(backends: BackendChoice[]): { ready: BackendChoice[]; more: BackendChoice[] } {
	const byName = (a: BackendChoice, b: BackendChoice) => a.name.localeCompare(b.name);
	const hotline = backends.filter((one) => one.id === HOTLINE_AGENT);
	const featured = backends.filter((one) => FEATURED.has(one.id)).sort(byName);
	const rest = backends.filter((one) => one.id !== HOTLINE_AGENT && !FEATURED.has(one.id));
	const startable = rest.filter((one) => one.unavailable === undefined).sort(byName);
	const missing = rest.filter((one) => one.unavailable !== undefined).sort(byName);
	return { ready: [...hotline, ...featured], more: [...startable, ...missing] };
}

/**
 * The names of the providers connected now, live ones only, for the chips
 * under Hotline Agent: it runs on these, and nothing else in the list does.
 */
function useConnectedProviders(): string[] | null {
	const [names, setNames] = useState<string[] | null>(null);
	useEffect(() => {
		let current = true;
		void Promise.all([wire.command("credential.list", {}), wire.command("providers.list", {})])
			.then(([credentials, providers]) => {
				if (!current) return;
				const live = credentials
					.filter((one) => !one.revoked)
					.map((one) => providers.find((provider) => provider.id === one.providerId)?.name ?? one.label);
				setNames([...new Set(live)].sort((a, b) => a.localeCompare(b)));
			})
			.catch(() => {
				if (current) setNames([]);
			});
		return () => {
			current = false;
		};
	}, []);
	return names;
}

function HotlineAgentDetail({ connected, onProviders }: { connected: string[] | null; onProviders: (() => void) | undefined }) {
	return (
		<>
			<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
				Hotline's own agent. Runs any model from the providers you connect in{" "}
				{onProviders === undefined ? (
					"Settings › Providers"
				) : (
					<button type="button" className="text-accent-ink hover:underline" onClick={onProviders}>
						Settings › Providers
					</button>
				)}
				.
			</span>
			{connected !== null && (
				<span className="mt-1.5 flex flex-wrap gap-1">
					{connected.length === 0 ? (
						<span className="text-sm text-warn">None connected yet</span>
					) : (
						connected.map((one) => (
							<span key={one} className="provider-chip">
								{one}
							</span>
						))
					)}
				</span>
			)}
		</>
	);
}

function Choice({
	backend,
	detail,
	off,
	name,
	selected,
	tabIndex,
	onSelect,
}: {
	backend: BackendChoice;
	detail: ReactNode;
	off: boolean;
	name: string;
	selected: string;
	tabIndex: number;
	onSelect(id: string): void;
}) {
	return (
		<label className="group-row group-row-choice" data-off={off ? "true" : undefined}>
			<input
				type="radio"
				className="radio"
				data-picker-row=""
				data-backend-id={backend.id}
				data-off={off ? "true" : undefined}
				name={name}
				checked={selected === backend.id}
				tabIndex={tabIndex}
				aria-disabled={off ? true : undefined}
				onChange={() => {
					if (off) return;
					onSelect(backend.id);
				}}
			/>
			<span className="group-row-text">
				<span className="group-row-title">{backend.name}</span>
				{typeof detail === "string" ? (
					<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
						{detail}
					</span>
				) : (
					detail
				)}
			</span>
		</label>
	);
}
