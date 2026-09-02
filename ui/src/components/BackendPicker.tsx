import { useEffect, useState, type KeyboardEvent } from "react";
import type { BackendChoice } from "../generated/contract";
import { ChevronDownIcon, ChevronRightIcon } from "../icons";

/** Toad Agent's stored backend id. The picker puts this row first even if
 *  the caller hands the array in another order. */
const TOAD_AGENT = "pi";

/**
 * The harnesses this machine can start, first — Toad Agent, then every
 * row with no unavailable sentence — because the catalogue is dozens
 * long and most of it is a PATH miss. The rest sit behind a disclosure,
 * each still carrying the sentence that names what is missing.
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
}: {
	backends: BackendChoice[];
	selected: string;
	name: string;
	labelledBy: string;
	onSelect(id: string): void;
}) {
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

	return (
		<div role="radiogroup" aria-labelledby={labelledBy} className="grouped" onKeyDown={onKey}>
			{ready.map((backend) => (
				<Choice
					key={backend.id}
					backend={backend}
					detail={backend.description}
					off={false}
					name={name}
					selected={selected}
					tabIndex={stop === backend.id ? 0 : -1}
					onSelect={onSelect}
				/>
			))}
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
					<span className="text-sm text-ink-3">More harnesses ({more.length})</span>
				</button>
			)}
			{open &&
				more.map((backend) => (
					<Choice
						key={backend.id}
						backend={backend}
						detail={backend.unavailable ?? backend.description}
						off
						name={name}
						selected={selected}
						tabIndex={stop === backend.id ? 0 : -1}
						onSelect={onSelect}
					/>
				))}
		</div>
	);
}

function arrange(backends: BackendChoice[]): { ready: BackendChoice[]; more: BackendChoice[] } {
	const toad = backends.find((one) => one.id === TOAD_AGENT);
	const rest = backends.filter((one) => one.id !== TOAD_AGENT);
	const ready = rest.filter((one) => one.unavailable === undefined);
	const more = rest.filter((one) => one.unavailable !== undefined);
	return { ready: toad === undefined ? ready : [toad, ...ready], more };
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
	detail: string;
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
				<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
					{detail}
				</span>
			</span>
		</label>
	);
}
