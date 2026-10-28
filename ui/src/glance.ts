import type { Step } from "./scheduledRuns";
import { stepTitle } from "./stepText";

/*
 * A turn's steps at a glance: what the teammate did in a few words, never the
 * command it ran. The words are the desk core's (`said` on each step, from
 * hotline-core's glance.rs and its manifest of Hotline's own tools, or the
 * agent's own description of a step), so the desk and the phone say the same
 * thing. Here a run of steps of one kind becomes one line with its count
 * ("Read 3 files"), and the step running now says what is happening. The pane
 * and the phone's work sheet still show each step as it was run.
 */

/** One line of the glance: the steps it stands for, the first of them its id. */
export type Glance = { id: string; ids: string[]; text: string; live: boolean; failed: boolean };

type Said = { key: string; one: string; many: string; doing: string };

/** A thought is said by the window: it carries no words of the core's. */
const THOUGHT: Said = { key: "think", one: "Thought it through", many: "Thought it through", doing: "Thinking" };

/** A step's words: the core's, or, for a step written before it said them, a few words of its title. */
function saidOf(step: Step): Said {
	if (step.kind === "thought") return THOUGHT;
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
	for (const step of items) {
		const said = saidOf(step);
		const failed = step.kind === "tool" && step.status === "failed";
		const last = lines[lines.length - 1];
		if (last !== undefined && last.said.key === said.key) {
			last.ids.push(step.id);
			last.failed ||= failed;
			// The newest of the run says what it is doing now ("Running cargo test").
			last.said = said;
			last.text = said.many.replace("{n}", String(last.ids.length));
		} else {
			lines.push({ id: step.id, ids: [step.id], text: said.one, live: false, failed, said });
		}
	}
	const last = lines[lines.length - 1];
	if (running && last !== undefined) {
		last.live = true;
		last.text = last.said.doing;
	}
	return lines.map(({ said: _said, ...line }) => line);
}
