import { useEffect, useState } from "react";
import type { RunningSubagent, ThreadEnd, TranscriptEvent } from "../../generated/contract";
import { ArrowLeftIcon } from "../../icons";
import { clockWords, threadFamily } from "../../threadKinds";
import { Band } from "../../ui/Band";
import { PaneClose } from "../../ui/PaneClose";
import { useProcessOutput } from "../Work";
import { BandKind, LiveStream, PaneHead } from "./Pane";
import { SectionRule, SpineFoot, StateLine } from "./Spine";

type Link = Extract<TranscriptEvent, { kind: "link" }>;

/** The run's own line at the head of its stream: what it is and how it ended. */
export function runLink(events: readonly TranscriptEvent[]): Link | undefined {
	return events.find((event): event is Link => event.kind === "link" && event.threadKind === "run");
}

/** The output a command's run kept when it ended: its one step's, the newest written. */
function keptOutput(events: readonly TranscriptEvent[]): string {
	for (const event of [...events].reverse()) {
		if (event.kind !== "tool") continue;
		const text = event.output?.flatMap((one) => (one.type === "text" ? [one.text] : [])).join("\n") ?? "";
		if (text !== "") return text;
	}
	return "";
}

/** How an ended command's band says it ended. */
const ENDED: Partial<Record<ThreadEnd, string>> = { done: "Done", failed: "Failed", cancelled: "Stopped", stopped: "Stopped" };

/** A length of time as a clock reads it: 1:03, 1:02:03. */
function clockOf(ms: number): string {
	const total = Math.max(0, Math.floor(ms / 1000));
	const hours = Math.floor(total / 3600);
	const minutes = Math.floor((total % 3600) / 60);
	const seconds = String(total % 60).padStart(2, "0");
	return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}` : `${minutes}:${seconds}`;
}

/** How many lines a command has written, as Output's count says it: a closing newline ends a line, it starts none. */
function linesOf(output: string): string {
	const lines = output.replace(/\n$/, "").split("\n").length;
	return lines === 1 ? "1 line" : `${lines} lines`;
}

/** The time now, moved on each second while `running`. */
function useClock(running: boolean): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		if (!running) return;
		const tick = window.setInterval(() => setNow(Date.now()), 1000);
		return () => window.clearInterval(tick);
	}, [running]);
	return now;
}

/**
 * A command a teammate's harness left running, open in the pane
 * (docs/design/thread-panes.html, Command; desk-threads, George 2026-11-12:
 * activity, not a chat): the command, how it stands, where and when it
 * started, then its Output, what it writes in a terminal as it writes it,
 * read the way a command Hotline Agent left running is read
 * (`useProcessOutput`). Once it ends the state line says how, and the
 * terminal keeps the last of its output: the last read while the pane was
 * open, else what the run kept. The foot says whose it is, with Stop where
 * the desk can stop it.
 */
export function CommandPane({
	personaId,
	runId,
	owner,
	cwd,
	run,
	events,
	onStop,
	onBack,
	onClose,
}: {
	/** The teammate whose command it is. */
	personaId: string;
	runId: string;
	owner: string;
	/** Where the teammate works, which is where its commands run. */
	cwd: string | undefined;
	/** The run as the roster lists it; none once it has ended. */
	run: RunningSubagent | undefined;
	/** The run's stream, its link included. */
	events: TranscriptEvent[];
	/** The person's Stop (`thread.cancel`), offered while the roster says the desk can stop it. */
	onStop(): Promise<unknown>;
	onBack(): void;
	onClose(): void;
}) {
	const running = run !== undefined;
	const read = useProcessOutput(personaId, runId, running);
	const link = runLink(events);
	const now = useClock(running);
	const command = run?.command ?? read?.command ?? link?.title ?? "";
	const startedAt = run?.startedAt ?? link?.ts;
	const output = read !== null && read.tail !== "" ? read.tail : running ? "" : keptOutput(events);
	const state = running ? "Running" : ((link?.end !== undefined ? ENDED[link.end] : undefined) ?? "Ended");
	const took = running ? (startedAt === undefined ? undefined : now - startedAt) : link?.elapsedMs;
	const dir = cwd?.split(/[\\/]/).filter((part) => part !== "").at(-1);
	const [refused, setRefused] = useState<string | null>(null);
	const [stopping, setStopping] = useState(false);
	const stop = () => {
		setRefused(null);
		setStopping(true);
		void onStop().catch((error: unknown) => {
			setStopping(false);
			setRefused(error instanceof Error ? error.message : String(error));
		});
	};
	return (
		<section className="dock-page" aria-label={`Command: ${command}`}>
			<Band>
				<button type="button" className="control btn-icon pane-back" title="Back" aria-label="Back" onClick={onBack}>
					<ArrowLeftIcon />
				</button>
				<BandKind kind={threadFamily("run", undefined, "shell")} state={state.toLowerCase()} />
				<PaneClose onClose={onClose} />
			</Band>
			<PaneHead
				title={command}
				line={<StateLine word={running ? "Running now" : state} time={took === undefined ? undefined : clockOf(took)} />}
				facts={[
					["In", dir],
					["Started", startedAt !== undefined ? clockWords(startedAt) : undefined],
				]}
			/>
			<SectionRule title="Output" status={running ? "live" : state === "Failed" ? "failed" : "stopped"} count={output === "" ? undefined : linesOf(output)} />
			<LiveStream following={runId} count={output.length} content={output} live={running}>
				<pre className="pane-terminal selectable">
					<span className="pane-terminal-command">
						<span aria-hidden="true" className="pane-command-prompt">
							$
						</span>
						{command}
					</span>
					{"\n"}
					{output !== "" ? output : <span className="text-ink-3">{running ? (read === null ? "Reading its output." : "Nothing written yet.") : "No output was kept."}</span>}
				</pre>
			</LiveStream>
			{refused !== null && <p className="dock-note selectable" style={{ color: "var(--warn)" }}>{refused}</p>}
			<SpineFoot
				action={
					running &&
					run.stoppable === true && (
						<button type="button" className="control btn btn-sm" title="Stop this command" disabled={stopping} onClick={stop}>
							Stop
						</button>
					)
				}
			>
				Left running by {owner}.
			</SpineFoot>
		</section>
	);
}
