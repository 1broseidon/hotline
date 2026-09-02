import { useEffect, useRef, useState } from "react";
import type { ScheduleKind, ScheduledJob } from "../generated/contract";
import { durationText, firstLine, nextText } from "../room";
import { wire } from "../wire";

const MINUTE = 60_000;
const HOUR = 3_600_000;
const DAY = 86_400_000;
const MIN_WAIT = 1_000;
const MAX_AHEAD = 30 * DAY;
const MIN_LOOP = 15_000;
const MAX_LOOP = 7 * DAY;

type Unit = "minutes" | "hours" | "days";

const UNITS: { id: Unit; ms: number; label: string }[] = [
	{ id: "minutes", ms: MINUTE, label: "minutes" },
	{ id: "hours", ms: HOUR, label: "hours" },
	{ id: "days", ms: DAY, label: "days" },
];

/**
 * Work this teammate asked Toad to wake it for — or that you set here.
 *
 * A job is once (`when`, ms since epoch) or a loop (`every`, ms). Quiet
 * means the run's words go to the tape as thoughts, by event kind, not by
 * asking the model to stay quiet.
 */
export function Schedules({
	personaId,
	jobs,
	focus,
}: {
	personaId: string;
	jobs: ScheduledJob[];
	focus: boolean;
}) {
	const heading = useRef<HTMLHeadingElement>(null);
	const now = useNow();

	useEffect(() => {
		if (!focus) return;
		heading.current?.scrollIntoView({ block: "start" });
		heading.current?.focus();
	}, [focus]);

	return (
		<section className="flex flex-col gap-3">
			<h3 ref={heading} id="schedules" tabIndex={-1} className="label outline-none">
				Schedules
			</h3>
			{jobs.length === 0 ? (
				<p className="text-xs leading-relaxed text-ink-3">Nothing scheduled.</p>
			) : (
				<ul className="flex flex-col">
					{jobs.map((job) => (
						<JobRow key={job.id} job={job} now={now} />
					))}
				</ul>
			)}
			<AddJob personaId={personaId} />
		</section>
	);
}

function JobRow({ job, now }: { job: ScheduledJob; now: number }) {
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

	const kind = job.kind === "loop" && job.every !== undefined ? `loop · ${durationText(job.every)}` : "once";

	return (
		<li className="flex items-start gap-2 border-b border-rule py-1.5 text-xs">
			<div className="min-w-0 flex-1">
				<p className="text-ink-2">
					<span className="text-ink-3">{kind}</span>
					<span className="mx-1.5 text-ink-3">·</span>
					<span>{nextText(job.nextAt, now)}</span>
					{job.quiet === true && (
						<>
							<span className="mx-1.5 text-ink-3">·</span>
							<span className="text-ink-3">quiet</span>
						</>
					)}
				</p>
				<p className="truncate text-ink-2">{firstLine(job.prompt)}</p>
				{said !== null && <p className="mt-1 text-[var(--danger)]">{said}</p>}
			</div>
			<div className="flex shrink-0 flex-col items-end gap-1">
				<label className="flex items-center gap-1.5 text-ink-2">
					<input
						type="checkbox"
						checked={job.quiet === true}
						disabled={busy}
						onChange={(event) => void setQuiet(event.target.checked)}
					/>
					Quiet
				</label>
				<button
					type="button"
					className="text-[var(--danger)]"
					disabled={busy}
					onClick={() => void cancel()}
				>
					Cancel
				</button>
			</div>
		</li>
	);
}

function AddJob({ personaId }: { personaId: string }) {
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
			setPrompt("");
			setQuiet(false);
			setWhen(defaultWhenInput());
			setCount("1");
			setUnit("hours");
		} catch (error) {
			setSaid(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="flex flex-col gap-3">
			<p className="label">Add a schedule</p>
			<div>
				<p className="label" id="job-kind">
					When
				</p>
				<div role="radiogroup" aria-labelledby="job-kind" className="flex flex-col gap-1.5">
					<label className="flex items-center gap-2 text-sm text-ink-2">
						<input
							type="radio"
							name={`job-kind-${personaId}`}
							checked={kind === "schedule"}
							onChange={() => setKind("schedule")}
						/>
						Once
					</label>
					<label className="flex items-center gap-2 text-sm text-ink-2">
						<input
							type="radio"
							name={`job-kind-${personaId}`}
							checked={kind === "loop"}
							onChange={() => setKind("loop")}
						/>
						Every
					</label>
				</div>
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
							className="field w-24"
							min={1}
							step={1}
							value={count}
							onChange={(event) => setCount(event.target.value)}
						/>
						<select
							className="field w-auto"
							aria-label="Interval unit"
							value={unit}
							onChange={(event) => setUnit(event.target.value as Unit)}
						>
							{UNITS.map((one) => (
								<option key={one.id} value={one.id}>
									{one.label}
								</option>
							))}
						</select>
					</div>
				</div>
			)}
			<div>
				<label className="label" htmlFor="job-prompt">
					Prompt
				</label>
				<textarea
					id="job-prompt"
					className="field resize-none"
					rows={3}
					value={prompt}
					onChange={(event) => setPrompt(event.target.value)}
				/>
			</div>
			<label className="flex items-center gap-2 text-sm text-ink-2">
				<input type="checkbox" checked={quiet} onChange={(event) => setQuiet(event.target.checked)} />
				Quiet
			</label>
			<p className="text-xs leading-relaxed text-ink-3">
				Quiet writes the run&rsquo;s words to the tape as thoughts.
			</p>
			{said !== null && <p className="text-xs text-[var(--danger)]">{said}</p>}
			<div className="flex justify-end">
				<button type="button" className="btn-primary" disabled={busy || prompt.trim() === ""} onClick={() => void add()}>
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
