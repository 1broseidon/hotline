import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { ScheduleEntry, ScheduledRun, TranscriptEvent } from "../src/generated/contract";
import { countdown, jobRuns, postingWords, scheduleWords } from "../src/jobRuns";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });

let act: typeof import("react").act;
let JobPage: typeof import("../src/components/thread/JobPane").JobPage;
let Transcript: typeof import("../src/components/Transcript").Transcript;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

beforeEach(async () => {
	const installed = installDom({ requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	Object.assign(globalThis, { ResizeObserver: installed.dom.ResizeObserver });
	act = (await import("react")).act;
	({ JobPage } = await import("../src/components/thread/JobPane"));
	({ Transcript } = await import("../src/components/Transcript"));
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await teardown();
});

const NOW = new Date(2026, 10, 7, 19, 56, 1).getTime();
const at = (hours: number, minutes: number) => new Date(2026, 10, 7, hours, minutes).getTime();

const watch: ScheduleEntry = {
	id: "ci",
	personaId: "pork",
	name: "Watch CI on release/0.49.0",
	kind: "loop",
	prompt: "Watch CI on release/0.49.0 and tell me when a job changes.",
	every: 300_000,
	nextAt: NOW + 299_000,
	lastPost: { at: at(19, 41), text: "Linux green\nAll 14 jobs passed." },
};
const firing: ScheduledRun = { jobId: "ci", kind: "loop", name: watch.name, operatorCreated: true };
const said = (key: string, one: string) => ({ key, one, many: `${one} ({n})`, doing: one });

/** Run 1 posted its report, run 2 was stopped, run 3 is working now. */
const thread: TranscriptEvent[] = [
	{ kind: "user", id: "f1", ts: at(19, 46), text: "Fire.", scheduled: firing },
	{ kind: "tool", id: "t1", ts: at(19, 46), toolCallId: "tc1", title: "gh run view 4218", status: "completed", said: said("run", "Ran gh run view") },
	{ kind: "agent", id: "a1", ts: at(19, 47), text: "Linux passed (14m)." },
	{ kind: "turn", id: "e1", ts: at(19, 47), stopReason: "end_turn" },
	{ kind: "user", id: "f2", ts: at(19, 51), text: "Fire.", scheduled: firing },
	{ kind: "notice", id: "n2", ts: at(19, 52), level: "info", text: "Stopped. The job keeps its schedule and runs again at its next time." },
	{ kind: "turn", id: "e2", ts: at(19, 52), stopReason: "cancelled" },
	{ kind: "user", id: "f3", ts: at(19, 56), text: "Fire.", scheduled: firing },
	{ kind: "tool", id: "t3", ts: at(19, 56), toolCallId: "tc3", title: "gh run list --branch release/0.49.0", status: "in_progress", said: { key: "run", one: "Ran gh run list", many: "Ran gh run list ({n})", doing: "Running gh run list" } },
];

type Props = Partial<Parameters<typeof JobPage>[0]>;
let pressed: string[];

async function draw(props: Props = {}) {
	pressed = [];
	await act(async () =>
		root.render(
			<JobPage
				title={watch.name}
				name="Pork Chop"
				job={watch}
				ended={false}
				events={thread}
				working
				now={NOW}
				onStop={() => pressed.push("stop")}
				onEnd={() => pressed.push("end")}
				onRemove={() => pressed.push("remove")}
				onBack={() => {}}
				onClose={() => {}}
				composer={<textarea aria-label="Message" />}
				{...props}
			/>,
		),
	);
}

const text = (selector: string) => [...container.querySelectorAll(selector)].map((el) => el.textContent);
const button = (words: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find((one) => one.textContent === words);

describe("a job's pane", () => {
	test("says ↻ Job and Loop or Once, its title, and its teammate, schedule and posting rule", async () => {
		await draw();
		expect(container.querySelector(".band-kind")?.textContent).toBe("↻ Job");
		expect(container.querySelector(".pane-title")?.textContent).toBe("Watch CI on release/0.49.0");
		expect(container.querySelector(".pane-meta")?.textContent).toBe("Loop · Pork Chop · every 5 min · posts each run");
		const { every: _every, ...once } = watch;
		await draw({ job: { ...once, kind: "schedule", when: at(21, 0), quiet: true } });
		expect(container.querySelector(".pane-meta")?.textContent).toMatch(/^Once · Pork Chop · once at 9:00\s?PM · posts only what is worth posting$/);
	});

	test("says when it runs next, counting down, and what it last posted", async () => {
		await draw({ working: false });
		expect(text(".job-facts dt")).toEqual(["Next run", "Last posted"]);
		expect(text(".job-facts dd")[0]).toBe("4:59");
		expect(text(".job-facts dd")[1]).toMatch(/^7:41\s?PM · Linux green$/);
		await draw({ working: false, now: NOW + 1000 });
		expect(text(".job-facts dd")[0]).toBe("4:58");
		const { lastPost: _lastPost, ...unposted } = watch;
		await draw({ working: false, job: unposted });
		expect(text(".job-facts dd")[1]).toBe("Nothing yet");
	});

	test("each run is a collapsible section, newest first, with its state; the running one is open and live", async () => {
		await draw();
		const heads = [...container.querySelectorAll<HTMLButtonElement>(".job-run-head")];
		expect(heads.map((head) => head.textContent?.replace(/\s+/g, " "))).toEqual([
			"Run 3 · workingnow",
			expect.stringMatching(/^Run 2 · stopped7:51\s?PM$/),
			expect.stringMatching(/^Run 1 · posted7:46\s?PM$/),
		]);
		expect(heads.map((head) => head.getAttribute("aria-expanded"))).toEqual(["true", "false", "false"]);
		const running = container.querySelector(".job-run")!;
		expect(running.querySelector(".job-run-dot")?.classList.contains("beat")).toBe(true);
		expect(running.querySelector(".trail")?.textContent).toContain("Running gh run list");
	});

	test("a run opens to what it found and its steps, and folds again", async () => {
		await draw();
		const first = [...container.querySelectorAll<HTMLButtonElement>(".job-run-head")][2]!;
		await act(async () => first.click());
		const section = first.closest(".job-run")!;
		expect(section.querySelector(".job-run-found")?.textContent).toBe("Linux passed (14m).");
		expect(section.querySelector(".trail")?.textContent).toContain("Ran gh run view");
		await act(async () => first.click());
		expect(section.querySelector(".job-run-body")).toBeNull();
		const stopped = [...container.querySelectorAll<HTMLButtonElement>(".job-run-head")][1]!;
		await act(async () => stopped.click());
		expect(stopped.closest(".job-run")?.querySelector(".job-run-notice")?.textContent).toContain("Stopped.");
	});

	test("the band has End job while the job lives and Stop while a run runs, and no Run now, which the desk cannot do", async () => {
		await draw();
		expect(container.querySelector(".band h2")?.getAttribute("aria-label")).toBe("Job, running");
		expect(container.querySelector(".band .instrument")?.textContent).toBe("every 5 min");
		expect(button("Run now")).toBeUndefined();
		await act(async () => button("Stop")?.click());
		await act(async () => button("End job")?.click());
		expect(pressed).toEqual(["stop", "end"]);
		await draw({ working: false });
		expect(button("Stop")).toBeUndefined();
		expect(button("End job")).toBeDefined();
	});

	test("the composer tells the job something while it lives, and an ended job has none, nor End job", async () => {
		await draw();
		expect(container.querySelector('[aria-label="Message"]')).not.toBeNull();
		await draw({ ended: true, working: false });
		expect(container.querySelector(".band h2")?.getAttribute("aria-label")).toBe("Job, ended");
		expect(container.querySelector('[aria-label="Message"]')).toBeNull();
		expect(button("End job")).toBeUndefined();
		expect(text(".job-facts dd")[0]).toBe("None");
	});

	test("an ended job has Remove, which takes it off the list, and a live one has none", async () => {
		await draw();
		expect(button("Remove")).toBeUndefined();
		await draw({ ended: true, working: false });
		await act(async () => button("Remove")?.click());
		expect(pressed).toEqual(["remove"]);
	});

	test("with older lines on the desk its runs go unnumbered, and Earlier runs loads them", async () => {
		let asked = 0;
		await draw({ more: true, onEarlier: async () => {
			asked += 1;
		} });
		expect(container.querySelector(".job-run-head")?.textContent).toBe("Run · workingnow");
		await act(async () => button("Earlier runs")?.click());
		expect(asked).toBe(1);
	});
});

describe("a job's runs, from its thread", () => {
	test("a quiet run posted only when a step of its own posted", () => {
		const quiet = { ...firing, quiet: true };
		const runs = jobRuns(
			[
				{ kind: "user", id: "q1", ts: 1, text: "Fire.", scheduled: quiet },
				{ kind: "agent", id: "w1", ts: 2, text: "Nothing new." },
				{ kind: "turn", id: "e1", ts: 3, stopReason: "end_turn" },
				{ kind: "user", id: "q2", ts: 4, text: "Fire.", scheduled: quiet },
				{ kind: "tool", id: "p2", ts: 5, toolCallId: "p2", title: "post_to_chat", status: "completed", said: said("post", "Posted to the chat") },
				{ kind: "agent", id: "w2", ts: 6, text: "Posted the red build." },
				{ kind: "turn", id: "e2", ts: 7, stopReason: "end_turn" },
			],
			false,
		);
		expect(runs.map((run) => [run.number, run.state, run.found])).toEqual([
			[1, "done", "Nothing new."],
			[2, "posted", "Posted the red build."],
		]);
	});

	test("a line the person said is a run of its own that posts nothing, and what it found is the words after its last step", () => {
		const runs = jobRuns(
			[
				{ kind: "user", id: "m1", ts: 1, text: "Watch macOS too." },
				{ kind: "agent", id: "w1", ts: 2, text: "Checking." },
				{ kind: "tool", id: "s1", ts: 3, toolCallId: "s1", title: "gh run list", status: "completed" },
				{ kind: "agent", id: "w2", ts: 4, text: "Will do from the next run." },
				{ kind: "turn", id: "e1", ts: 5, stopReason: "end_turn" },
			],
			false,
		);
		expect(runs).toEqual([expect.objectContaining({ asked: "Watch macOS too.", state: "done", found: "Will do from the next run." })]);
		expect(runs[0]?.number).toBeUndefined();
	});

	test("a run whose turn never ended is working only while the job's turn runs", () => {
		const open: TranscriptEvent[] = [{ kind: "user", id: "f", ts: 1, text: "Fire.", scheduled: firing }];
		expect(jobRuns(open, true)[0]?.state).toBe("working");
		expect(jobRuns(open, false)[0]?.state).toBe("done");
	});

	test("its words: a countdown, a schedule and a posting rule", () => {
		expect(countdown(299_000)).toBe("4:59");
		expect(countdown(3_899_000)).toBe("1:04:59");
		expect(countdown(-5)).toBe("0:00");
		expect(countdown(3 * 86_400_000)).toBe("3d");
		expect(scheduleWords({ ...watch, every: 7_200_000 })).toBe("every 2 h");
		expect(scheduleWords({ ...watch, every: 86_400_000 })).toBe("every 1 day");
		expect(postingWords({ ...watch, quiet: true })).toBe("posts only what is worth posting");
	});
});

describe("a job's run that posts", () => {
	test("leaves a link line in the person's chat, Job · its title · posted, that opens the job", async () => {
		const opened: import("../src/components/Transcript").ThreadRef[] = [];
		const post: TranscriptEvent = { kind: "agent", id: "p1", ts: 1, text: "Linux passed (14m).", scheduled: firing, from: { kind: "job", thread: "ci", title: watch.name } };
		await act(async () =>
			root.render(<Transcript personaId="pork" name="Pork Chop" events={[post]} streaming={[]} live={false} focus={null} onOpenThread={(open) => opened.push(open)} />),
		);
		const line = container.querySelector<HTMLButtonElement>('.link-row[data-kind="job"]')!;
		expect(line.getAttribute("aria-label")).toBe("Job · Watch CI on release/0.49.0 · posted. Open it");
		expect([".link-row-title", ".link-row-outcome"].map((part) => line.querySelector(part)?.textContent)).toEqual([watch.name, "· posted"]);
		expect(line.textContent).not.toContain("Job");
		// The words posted are still the bubble under it.
		expect(container.querySelector(".speech")?.textContent).toContain("Linux passed (14m).");
		await act(async () => line.click());
		expect(opened).toEqual([{ thread: { kind: "job", key: "ci" }, title: watch.name }]);
	});
});
