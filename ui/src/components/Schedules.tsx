import { useEffect, useRef, useState } from "react";
import type { ScheduleKind, ScheduledJob } from "../generated/contract";
import { CloseIcon, PlusIcon } from "../icons";
import { durationText, firstLine, nextText } from "../room";
import { onTablistKey, Picker } from "../ui/Menu";
import { wire } from "../wire";

const MINUTE = 60_000;
const HOUR = 3_600_000;
const DAY = 86_400_000;
const MIN_WAIT = 1_000;
const MAX_AHEAD = 30 * DAY;
const MIN_LOOP = 15_000;
const MAX_LOOP = 7 * DAY;

type Unit = "minutes" | "hours" | "days";

const UNITS: { id: Unit; ms: number; name: string }[] = [
	{ id: "minutes", ms: MINUTE, name: "minutes" },
	{ id: "hours", ms: HOUR, name: "hours" },
	{ id: "days", ms: DAY, name: "days" },
];

/**
 * Work this teammate asked Toad to wake it for — or that you set here.
 *
 * A job is once (`when`, ms since epoch) or a loop (`every`, ms). Quiet
 * means the run's words go to the tape as thoughts, by event kind, not by
 * asking the model to stay quiet. The form to add one is closed until the
 * add row is pressed, because a teammate with nothing scheduled should
 * show a list with an add row, not a form.
 */
export function Schedules({
	personaId,
	jobs,
	backgroundWork,
	focus,
}: {
	personaId: string;
	jobs: ScheduledJob[];
	backgroundWork: boolean;
	focus: boolean;
}) {
	const heading = useRef<HTMLHeadingElement>(null);
	const now = useNow();
	const [adding, setAdding] = useState(false);
	const paused = !backgroundWork && jobs.some((job) => !job.operatorCreated);

	useEffect(() => {
		if (!focus) return;
		heading.current?.scrollIntoView({ block: "start" });
		heading.current?.focus();
	}, [focus]);

	useEffect(() => {
		setAdding(false);
	}, [personaId]);

	return (
		<section>
			<h3 ref={heading} id="schedules" tabIndex={-1} className="label outline-none">
				Schedules
			</h3>
			<div className="grouped">
				{jobs.map((job) => (
					<JobRow key={job.id} job={job} now={now} backgroundWork={backgroundWork} />
				))}
				{!adding && (
					<button type="button" className="group-row group-row-add" onClick={() => setAdding(true)}>
						<PlusIcon />
						Add a schedule
					</button>
				)}
			</div>
			{paused && <p className="group-hint">Paused jobs wake when Background work is on again.</p>}
			{adding && <AddJob personaId={personaId} onDone={() => setAdding(false)} />}
		</section>
	);
}

function JobRow({ job, now, backgroundWork }: { job: ScheduledJob; now: number; backgroundWork: boolean }) {
	const [busy, setBusy] = useState(false);
	const [said, setSaid] = useState<string | null>(null);

	const cancel = async () => {
		if (busy) return;
		setBusy(true);
		setSaid(null);
		try {
			await wire.command("schedule.cancel", { id: job.id });
		} catch (error) {
			setSaid(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	const setQuiet = async (quiet: boolean) => {
		if (busy) return;
		setBusy(true);
		setSaid(null);
		try {
			await wire.command("schedule.set_quiet", { id: job.id, quiet });
		} catch (error) {
			setSaid(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const kind = job.kind === "loop" && job.every !== undefined ? `Every ${durationText(job.every)}` : "Once";
	const source = job.operatorCreated ? "Added by you" : "Background work";
	const status = !job.operatorCreated && !backgroundWork ? "Paused" : nextText(job.nextAt, now);

	return (
		<div className="group-row items-start gap-2 py-2">
			<span className="group-row-text">
				<span className="group-row-title" title={job.prompt}>
					{firstLine(job.prompt)}
				</span>
				<span className="group-row-detail">
					{kind} · {source} · {status}
				</span>
				<label className="mt-1.5 flex items-center gap-2 text-sm text-ink-2">
					<input
						type="checkbox"
						className="check"
						checked={job.quiet === true}
						disabled={busy}
						onChange={(event) => void setQuiet(event.target.checked)}
					/>
					Quiet
				</label>
				{said !== null && <span className="mt-1 block text-sm text-danger">{said}</span>}
			</span>
			<button
				type="button"
				className="control btn-icon -mr-1.5 -mt-1"
				aria-label="Cancel this schedule"
				title="Cancel"
				disabled={busy}
				onClick={() => void cancel()}
			>
				<CloseIcon />
			</button>
		</div>
	);
}

function AddJob({ personaId, onDone }: { personaId: string; onDone(): void }) {
	const [kind, setKind] = useState<ScheduleKind>("schedule");
	const [when, setWhen] = useState(defaultWhenInput);
	const [count, setCount] = useState("1");
	const [unit, setUnit] = useState<Unit>("hours");
	const [prompt, setPrompt] = useState("");
	const [quiet, setQuiet] = useState(false);
	const [busy, setBusy] = useState(false);
	const [said, setSaid] = useState<string | null>(null);

	const add = async () => {
		const text = prompt.trim();
		if (!text || busy) return;
		const params = buildCreate(personaId, kind, when, count, unit, text, quiet);
		if (typeof params === "string") {
			setSaid(params);
			return;
		}
		setBusy(true);
		setSaid(null);
		try {
			await wire.command("schedule.create", params);
			onDone();
		} catch (error) {
			setSaid(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="mt-3 flex flex-col gap-3">
			<div className="segmented self-start" role="tablist" aria-label="When" onKeyDown={onTablistKey}>
				<button type="button" role="tab" className="segment" aria-selected={kind === "schedule"} onClick={() => setKind("schedule")}>
					Once
				</button>
				<button type="button" role="tab" className="segment" aria-selected={kind === "loop"} onClick={() => setKind("loop")}>
					Repeating
				</button>
			</div>
			{kind === "schedule" ? (
				<div>
					<label className="label" htmlFor="job-when">
						At
					</label>
					<input
						id="job-when"
						type="datetime-local"
						className="field"
						value={when}
						onChange={(event) => setWhen(event.target.value)}
					/>
				</div>
			) : (
				<div>
					<label className="label" htmlFor="job-every">
						Every
					</label>
					<div className="flex items-center gap-2">
						<input
							id="job-every"
							type="number"
							className="field w-16 text-right"
							min={1}
							step={1}
							value={count}
							onChange={(event) => setCount(event.target.value)}
						/>
						<div className="flex-1">
							<Picker
								field
								value={unit}
								choices={UNITS}
								placeholder="Unit"
								label="Interval unit"
								onChange={(next) => setUnit(next as Unit)}
							/>
						</div>
					</div>
				</div>
			)}
			<div>
				<label className="label" htmlFor="job-prompt">
					Say
				</label>
				<textarea
					id="job-prompt"
					className="field"
					rows={2}
					autoFocus
					placeholder="What to ask when it fires."
					value={prompt}
					onChange={(event) => setPrompt(event.target.value)}
				/>
			</div>
			<label className="flex items-start gap-2 text-sm text-ink-2">
				<input type="checkbox" className="check mt-px" checked={quiet} onChange={(event) => setQuiet(event.target.checked)} />
				<span>
					Quiet
					<span className="block text-xs text-ink-3">The run&rsquo;s words land in the tape as thoughts.</span>
				</span>
			</label>
			{said !== null && <p className="text-sm text-danger">{said}</p>}
			<div className="flex justify-end gap-2">
				<button type="button" className="control btn-quiet" disabled={busy} onClick={onDone}>
					Cancel
				</button>
				<button type="button" className="control btn" disabled={busy || prompt.trim() === ""} onClick={() => void add()}>
					{busy ? "Scheduling…" : "Add schedule"}
				</button>
			</div>
		</div>
	);
}

function buildCreate(
	personaId: string,
	kind: ScheduleKind,
	whenInput: string,
	countRaw: string,
	unit: Unit,
	prompt: string,
	quiet: boolean,
): { personaId: string; kind: ScheduleKind; when?: number; every?: number; prompt: string; quiet?: boolean } | string {
	const quietField = quiet ? { quiet: true as const } : {};
	if (kind === "schedule") {
		const when = new Date(whenInput).getTime();
		if (!Number.isFinite(when)) return "Pick a date and time.";
		const wait = when - Date.now();
		if (wait < MIN_WAIT || wait > MAX_AHEAD) {
			return "A one-shot has to be between a second and 30 days from now.";
		}
		return { personaId, kind, when, prompt, ...quietField };
	}
	const count = Number(countRaw);
	if (!Number.isInteger(count) || count < 1) return "Every needs a whole number of intervals.";
	const step = UNITS.find((one) => one.id === unit)?.ms ?? HOUR;
	const every = count * step;
	if (every < MIN_LOOP || every > MAX_LOOP) {
		return "A loop has to be between 15 seconds and 7 days.";
	}
	return { personaId, kind, every, prompt, ...quietField };
}

function defaultWhenInput(): string {
	const at = new Date(Date.now() + HOUR);
	at.setSeconds(0, 0);
	return toLocalInput(at.getTime());
}

/** `datetime-local` is local civil time, no zone; pad so the field can parse it. */
function toLocalInput(ms: number): string {
	const at = new Date(ms);
	const pad = (n: number) => String(n).padStart(2, "0");
	return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}T${pad(at.getHours())}:${pad(at.getMinutes())}`;
}

function useNow(interval = 15_000): number {
	const [now, setNow] = useState(Date.now);
	useEffect(() => {
		const tick = window.setInterval(() => setNow(Date.now()), interval);
		return () => window.clearInterval(tick);
	}, [interval]);
	return now;
}
