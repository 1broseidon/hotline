import type { BackendChoice } from "../generated/contract";

/**
 * The harnesses this machine can name. An unavailable row stays in the
 * list so the missing piece is a sentence next to the name, not a hole.
 */
export function BackendPicker({
	backends,
	selected,
	name,
	labelledBy,
	onSelect,
}: {
	backends: BackendChoice[];
	selected: string;
	name: string;
	labelledBy: string;
	onSelect(id: string): void;
}) {
	return (
		<div role="radiogroup" aria-labelledby={labelledBy} className="flex flex-col gap-1.5">
			{backends.map((backend) => (
				<BackendRow
					key={backend.id}
					backend={backend}
					name={name}
					selected={selected === backend.id}
					onSelect={() => onSelect(backend.id)}
				/>
			))}
		</div>
	);
}

function BackendRow({
	backend,
	name,
	selected,
	onSelect,
}: {
	backend: BackendChoice;
	name: string;
	selected: boolean;
	onSelect(): void;
}) {
	const missing = backend.unavailable;
	return (
		<label
			className={`flex items-start gap-2 border border-rule px-2.5 py-2 text-sm ${
				missing ? "text-ink-3 opacity-60" : "bg-paper-2 text-ink-2"
			}`}
		>
			<input
				type="radio"
				name={name}
				className="mt-0.5"
				checked={selected}
				disabled={missing !== undefined}
				onChange={onSelect}
			/>
			<span className="min-w-0 flex-1">
				<span className={`font-medium ${missing ? "text-ink-3" : "text-ink"}`}>{backend.name}</span>
				{backend.description !== "" && (
					<span className="mt-0.5 block text-xs leading-relaxed text-ink-3">{backend.description}</span>
				)}
				{missing !== undefined && <span className="mt-0.5 block text-xs leading-relaxed">{missing}</span>}
			</span>
		</label>
	);
}
