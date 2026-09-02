import type { BackendChoice } from "../generated/contract";

/**
 * The harnesses this machine can name, as rows you pick one of. An
 * unavailable row stays in the list so the missing piece is a sentence
 * next to the name, not a hole.
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
		<div role="radiogroup" aria-labelledby={labelledBy} className="grouped">
			{backends.map((backend) => {
				const missing = backend.unavailable;
				return (
					<label
						key={backend.id}
						className="group-row group-row-choice"
						data-off={missing !== undefined ? "true" : undefined}
					>
						<input
							type="radio"
							className="radio"
							name={name}
							checked={selected === backend.id}
							disabled={missing !== undefined}
							onChange={() => onSelect(backend.id)}
						/>
						<span className="group-row-text">
							<span className="group-row-title">{backend.name}</span>
							<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
								{missing ?? backend.description}
							</span>
						</span>
					</label>
				);
			})}
		</div>
	);
}
