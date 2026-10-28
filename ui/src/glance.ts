import type { Step } from "./scheduledRuns";
import { stepTitle } from "./stepText";
import type { ToolStep } from "./turnFacts";

/*
 * A turn's steps at a glance: what the teammate did in a few words, never the
 * command it ran. The words are the desk core's (`said` on each step, from
 * hotline-core's glance.rs and its manifest of Hotline's own tools, or the
 * agent's own description of a step), so the desk and the phone say the same
 * thing. Here a run of steps of one kind becomes one line with its count
 * ("Read 3 files"), and the step running now says what is happening. A
 * thought is not a step: it rides with the line after it, to be read when
 * that line is opened, and is a line of its own only while the turn is
 * thinking now.
 */

/** One line of the glance: the steps it stands for, the first of them its id, and the thinking before them. */
export type Glance = { id: string; ids: string[]; steps: ToolStep[]; thoughts: string[]; text: string; live: boolean; failed: boolean };

type Said = { key: string; one: string; many: string; doing: string };

/** A step's words: the core's, or, for a step written before it said them, a few words of its title. */
function saidOf(step: ToolStep): Said {
	if (step.said !== undefined) return step.said;
	const words = stepTitle(step.title).split(/\s+/).slice(0, 4).join(" ");
	return { key: `title:${words}`, one: words, many: `${words} ({n})`, doing: words };
}

/**
 * The steps as lines a person reads at a glance. `running` is whether the
 * turn is still at work: its last line then says what is happening now.
 */
export function glance(items: Step[], running: boolean): Glance[] {
	const lines: (Glance & { said: Said })[] = [];
	let thoughts: string[] = [];
	for (const step of items) {
		if (step.kind === "thought") {
			if (step.text.trim() !== "") thoughts.push(step.text);
			continue;
		}
		const said = saidOf(step);
		const failed = step.status === "failed";
		const last = lines[lines.length - 1];
		if (last !== undefined && last.said.key === said.key) {
			last.ids.push(step.id);
			last.thoughts.push(...thoughts);
			thoughts = [];
			last.steps.push(step);
			last.failed ||= failed;
			// The newest of the run says what it is doing now ("Running cargo test").
			last.said = said;
			last.text = said.many.replace("{n}", String(last.ids.length));
		} else {
			lines.push({ id: step.id, ids: [step.id], steps: [step], thoughts, text: said.one, live: false, failed, said });
			thoughts = [];
		}
	}
	const newest = items[items.length - 1];
	if (running && newest?.kind === "thought") {
		lines.push({ id: newest.id, ids: [newest.id], steps: [], thoughts, text: "Thinking", live: true, failed: false, said: { key: "think", one: "", many: "", doing: "Thinking" } });
	} else if (lines.length > 0) {
		const last = lines[lines.length - 1]!;
		// Thinking after the last step, once the turn is over, rides with that step.
		last.thoughts.push(...thoughts);
		if (running) {
			last.live = true;
			last.text = last.said.doing;
		}
	}
	return lines.map(({ said: _said, ...line }) => line);
}
