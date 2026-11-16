import { useEffect, useRef, useState } from "react";
import type { CalendarDay, ScheduledJob, SkippedRun } from "../generated/contract";
import { CloseIcon, PlusIcon } from "../icons";
import { calendarWords, DAY_NAMES, timeOfDay } from "../jobRuns";
import { durationText, firstLine, nextText } from "../room";
import { onTablistKey, Picker } from "../ui/Menu";
import { type Commands, wire } from "../wire";

const MINUTE = 60_000;
const HOUR = 3_600_000;
const DAY = 86_400_000;
const MIN_WAIT = 1_000;
const MAX_AHEAD = 30 * DAY;
const MIN_LOOP = 15_000;
const MAX_LOOP = 7 * DAY;

type Unit = "minutes" | "hours" | "days";

/** When a job the person adds runs: once, on chosen days at a time, or every interval. */
type When = "once" | "days" | "every";

const WHENS: { id: When; name: string }[] = [
	{ id: "once", name: "Once" },
	{ id: "days", name: "On days" },
	{ id: "every", name: "Every" },
];

/** The week as the day toggles lay it out, Monday first, as a calendar keeps its days. */
const WEEK: CalendarDay[] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const WEEKDAYS: CalendarDay[] = ["mon", "tue", "wed", "thu", "fri"];

type Create = Commands["schedule.create"]["params"];

const UNITS: { id: Unit; ms: number; name: string }[] = [
	{ id: "minutes", ms: MINUTE, name: "minutes" },
	{ id: "hours", ms: HOUR, name: "hours" },
	{ id: "days", ms: DAY, name: "days" },
];

/**
 * Work this teammate asked Hotline to wake it for — or that you set here.
 *
 * A job is once (`when`, ms since epoch) or a loop (`every`, ms, or a
 * `calendar`'s time of day and days on the desk's clock). Quiet
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
	nameOf,
}: {
	personaId: string;
	jobs: ScheduledJob[];
	backgroundWork: boolean;
	focus: boolean;
	/** A teammate's name, for whom a job waits on. */
	nameOf(personaId: string): string;
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

	// biome-ignore lint/correctness/useExhaustiveDependencies: a new teammate is what closes the form.
	useEffect(() => {
		setAdding(false);
	}, [personaId]);

	return (
		<section>
			<h3 ref={heading} id="schedules" tabIndex={-1} className="label outline-none">
				Schedules
			</h3>
			{/* With nothing scheduled and the form open, the group has no rows,
			 * and an empty card would draw as a stray line over the form. */}
			{(jobs.length > 0 || !adding) && (
				<div className="grouped">
					{jobs.map((job) => (
						<JobRow key={job.id} job={job} now={now} backgroundWork={backgroundWork} nameOf={nameOf} />
					))}
					{!adding && (
						<button type="button" className="group-row group-row-add" onClick={() => setAdding(true)}>
							<PlusIcon />
							Add a schedule
						</button>
					)}
				</div>
			)}
			{paused && <p className="group-hint">Paused jobs wake when Background work is on again.</p>}
			{adding && <AddJob personaId={personaId} onDone={() => setAdding(false)} />}
		</section>
	);
}

function JobRow({ job, now, backgroundWork, nameOf }: { job: ScheduledJob; now: number; backgroundWork: boolean; nameOf(personaId: string): string }) {
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

	const kind = repeatsText(job);
	const source = job.operatorCreated ? "Added by you" : "Background work";
	// A last run that sent a colleague something waits for the reply, and the job ends once it comes.
	const status =
		job.awaiting !== undefined
			? `Waiting on ${job.awaiting.map(nameOf).join(", ")}`
			: !job.operatorCreated && !backgroundWork
				? "Paused"
				: nextText(job.nextAt, now);

	return (
		<div className="group-row items-start gap-2 py-2">
			<span className="group-row-text">
				<span className="group-row-title" title={job.prompt}>
					{firstLine(job.prompt)}
				</span>
				{/* Unlike other rows' details this one wraps, so the pane's
				 * width (320px, or less in a narrow window) cuts off none of
				 * when it runs next. It wraps between its parts only, and the
				 * dot between them is drawn so a line starts and ends on
				 * words (index.css, .schedule-detail); the text read aloud or
				 * copied keeps its " · ". */}
				<span className="group-row-detail schedule-detail">
					<span className="schedule-detail-parts">
						<span>{kind}</span>
						<span>
							<span className="sr-only"> · </span>
							{source}
						</span>
						<span>
							<span className="sr-only"> · </span>
							{status}
						</span>
					</span>
				</span>
				{job.kind === "loop" && job.skipped !== undefined && <span className="group-row-detail">{skippedText(job.skipped)}</span>}
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

/** How often the row says the job runs: "Weekdays at 9:00 AM", "Every 2h", "Once". */
function repeatsText(job: ScheduledJob): string {
	if (job.kind !== "loop") return "Once";
	if (job.calendar !== undefined) return capitalized(calendarWords(job.calendar));
	return job.every !== undefined ? `Every ${durationText(job.every)}` : "Once";
}

/**
 * A loop's run skipped for starting over two hours late, as the day and time
 * it was due on this computer's clock: "Skipped Fri 8:30 AM, over 2 hours
 * late". The desk keeps it on the job until the loop next runs.
 */
function skippedText(skipped: SkippedRun): string {
	return `Skipped ${weekday.format(new Date(skipped.slot))} ${timeOfDay(skipped.slot)}, over 2 hours late`;
}

const weekday = new Intl.DateTimeFormat(undefined, { weekday: "short" });

function AddJob({ personaId, onDone }: { personaId: string; onDone(): void }) {
	const [when, setWhen] = useState<When>("once");
	const [day, setDay] = useState(() => defaultWhen().day);
	const [time, setTime] = useState(() => defaultWhen().time);
	const [days, setDays] = useState<CalendarDay[]>(WEEKDAYS);
	const [at, setAt] = useState(() => timeText(9, 0));
	const [count, setCount] = useState("1");
	const [unit, setUnit] = useState<Unit>("hours");
	const [prompt, setPrompt] = useState("");
	const [quiet, setQuiet] = useState(false);
	const [busy, setBusy] = useState(false);
	const [said, setSaid] = useState<string | null>(null);

	const add = async () => {
		const text = prompt.trim();
		if (!text || busy) return;
		const params = buildCreate(personaId, { when, day, time, days, at, count, unit }, text, quiet);
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
				{WHENS.map((choice) => (
					<button
						key={choice.id}
						type="button"
						role="tab"
						className="segment"
						aria-selected={when === choice.id}
						onClick={() => setWhen(choice.id)}
					>
						{choice.name}
					</button>
				))}
			</div>
			{when === "days" && <OnDays days={days} at={at} onDays={setDays} onAt={setAt} />}
			{when === "once" && (
				<div>
					<label className="label" htmlFor="job-time">
						At
					</label>
					{/* No native date field: WebKitGTK's calendar popup hangs the
					 * window on Linux. A one-shot is at most 30 days out, so
					 * the day is a short menu and the time is typed. */}
					<div className="flex items-center gap-2">
						<div className="flex-1">
							<Picker field value={day} choices={dayChoices()} placeholder="Day" label="Day" onChange={setDay} />
						</div>
						<input
							id="job-time"
							type="text"
							inputMode="text"
							className="field w-28"
							placeholder="9:30, 14:00, 2pm"
							aria-label="Time"
							value={time}
							onChange={(event) => setTime(event.target.value)}
							onBlur={() => {
								const parsed = parseTime(time);
								if (parsed !== null) setTime(timeText(parsed.hours, parsed.minutes));
							}}
						/>
					</div>
				</div>
			)}
			{when === "every" && (
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
					// biome-ignore lint/a11y/noAutofocus: the form opens on a press of Add a schedule, and what to say is the one thing it needs.
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
				<button
					type="button"
					className="control btn"
					disabled={busy || prompt.trim() === "" || (when === "days" && days.length === 0)}
					onClick={() => void add()}
				>
					{busy ? "Scheduling…" : "Add schedule"}
				</button>
			</div>
		</div>
	);
}

/** What the form says about when the job runs, as its fields hold it. */
type Timing = { when: When; day: string; time: string; days: CalendarDay[]; at: string; count: string; unit: Unit };

/** The `schedule.create` the form asks for, or what is wrong with it, in the words the form shows. */
function buildCreate(personaId: string, timing: Timing, prompt: string, quiet: boolean): Create | string {
	const quietField = quiet ? { quiet: true as const } : {};
	if (timing.when === "once") {
		const at = parseTime(timing.time);
		if (at === null) return TIME_HINT;
		const [year, month, date] = timing.day.split("-").map(Number);
		const when = new Date(year ?? NaN, (month ?? NaN) - 1, date, at.hours, at.minutes).getTime();
		if (!Number.isFinite(when)) return "Pick a day.";
		const wait = when - Date.now();
		if (wait < MIN_WAIT || wait > MAX_AHEAD) {
			return "A one-shot has to be between a second and 30 days from now.";
		}
		return { personaId, kind: "schedule", when, prompt, ...quietField };
	}
	if (timing.when === "days") {
		const at = calendarAt(timing.at);
		if (at === null) return TIME_HINT;
		if (timing.days.length === 0) return NO_DAYS;
		return { personaId, kind: "loop", at, days: timing.days, prompt, ...quietField };
	}
	const count = Number(timing.count);
	if (!Number.isInteger(count) || count < 1) return "Every needs a whole number of intervals.";
	const step = UNITS.find((one) => one.id === timing.unit)?.ms ?? HOUR;
	const every = count * step;
	if (every < MIN_LOOP || every > MAX_LOOP) {
		return "A loop has to be between 15 seconds and 7 days.";
	}
	return { personaId, kind: "loop", every, prompt, ...quietField };
}

const TIME_HINT = "Type a time like 9:30, 14:00 or 2pm.";
const NO_DAYS = "Pick at least one day.";

/**
 * The days and time of a loop on chosen days: a toggle for each day of the
 * week and a typed time, then the choice read back in the words its row in
 * the list will use. With no day on, that line says to pick one instead.
 */
function OnDays({
	days,
	at,
	onDays,
	onAt,
}: {
	days: CalendarDay[];
	at: string;
	onDays(days: CalendarDay[]): void;
	onAt(at: string): void;
}) {
	const toggle = (day: CalendarDay) => {
		const next = days.includes(day) ? days.filter((one) => one !== day) : [...days, day];
		onDays(WEEK.filter((one) => next.includes(one)));
	};
	const kept = calendarAt(at);
	const said =
		days.length === 0
			? NO_DAYS
			: kept === null
				? TIME_HINT
				: `${capitalized(calendarWords({ at: kept, days }))}, on this computer’s clock.`;
	return (
		<>
			<div>
				<span className="label" id="job-days">
					Days
				</span>
				<fieldset className="day-toggles" aria-labelledby="job-days">
					{WEEK.map((day) => (
						<button key={day} type="button" className="day-toggle" aria-pressed={days.includes(day)} onClick={() => toggle(day)}>
							{DAY_NAMES[day][0]}
						</button>
					))}
				</fieldset>
			</div>
			<div>
				<label className="label" htmlFor="job-at">
					At
				</label>
				{/* Typed, as Once's time is: no native time field, for the
				 * WebKitGTK reason Once's day menu gives. */}
				<input
					id="job-at"
					type="text"
					inputMode="text"
					className="field w-28"
					placeholder="9:30, 14:00, 2pm"
					aria-describedby="job-days-said"
					value={at}
					onChange={(event) => onAt(event.target.value)}
					onBlur={() => {
						const parsed = parseTime(at);
						if (parsed !== null) onAt(timeText(parsed.hours, parsed.minutes));
					}}
				/>
				<p id="job-days-said" className="hint" aria-live="polite">
					{said}
				</p>
			</div>
		</>
	);
}

/** A typed time as a calendar keeps it, `09:00` on a 24-hour clock, or null when it is not a time. */
function calendarAt(raw: string): string | null {
	const parsed = parseTime(raw);
	if (parsed === null) return null;
	return `${String(parsed.hours).padStart(2, "0")}:${String(parsed.minutes).padStart(2, "0")}`;
}

function capitalized(words: string): string {
	return words.charAt(0).toUpperCase() + words.slice(1);
}

/** An hour from now, on the minute. */
function defaultWhen(): { day: string; time: string } {
	const at = new Date(Date.now() + HOUR);
	return { day: dayKey(at), time: timeText(at.getHours(), at.getMinutes()) };
}

/** A local calendar day as `YYYY-MM-DD`. */
function dayKey(at: Date): string {
	const pad = (n: number) => String(n).padStart(2, "0");
	return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
}

/** Today and the 30 days after it: as far as a one-shot can reach. */
function dayChoices(): { id: string; name: string; detail?: string }[] {
	const today = new Date();
	const choices = [];
	for (let offset = 0; offset <= 30; offset++) {
		const at = new Date(today.getFullYear(), today.getMonth(), today.getDate() + offset);
		const date = at.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
		const named = offset === 0 ? "Today" : offset === 1 ? "Tomorrow" : null;
		choices.push(named === null ? { id: dayKey(at), name: date } : { id: dayKey(at), name: named, detail: date });
	}
	return choices;
}

/** The time as this machine's clock writes it: `2:30 PM` or `14:30`. */
function timeText(hours: number, minutes: number): string {
	const local = new Date(2000, 0, 1, hours, minutes).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
	const read = parseTime(local);
	if (read?.hours === hours && read.minutes === minutes) return local;
	// A clock the field cannot read back (`오후 2:30`, Arabic digits) is
	// written as `14:30`, which it always can.
	return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}`;
}

/**
 * A typed time, read the way people write one: `9`, `09:30`, `930`,
 * `14.05`, `2pm`, `2:30 PM`, `2:30 p.m.`. Null when it is not a time of day.
 */
function parseTime(raw: string): { hours: number; minutes: number } | null {
	const text = raw.trim().toLowerCase().replace(/\s+/g, "").replace(/([ap])\.?m\.?$/, "$1m");
	const match = /^(\d{1,2})(?:[:.h]?(\d{2}))?(am|pm|a|p)?$/.exec(text);
	if (!match) return null;
	let hours = Number(match[1]);
	const minutes = match[2] === undefined ? 0 : Number(match[2]);
	const half = match[3]?.[0];
	if (minutes > 59) return null;
	if (half !== undefined) {
		if (hours < 1 || hours > 12) return null;
		hours = (hours % 12) + (half === "p" ? 12 : 0);
	} else if (hours > 23) {
		return null;
	}
	return { hours, minutes };
}

function useNow(interval = 15_000): number {
	const [now, setNow] = useState(Date.now);
	useEffect(() => {
		const tick = window.setInterval(() => setNow(Date.now()), interval);
		return () => window.clearInterval(tick);
	}, [interval]);
	return now;
}
