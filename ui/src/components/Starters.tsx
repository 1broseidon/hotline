import type { Persona } from "../generated/contract";
import { dataDirectory } from "../native";

/**
 * The first conversation's opening: two or three things to ask, fitted to
 * the folder the teammate works in, and one sentence on chapters. A folder
 * Toad made for them is empty, so the prompts make something; a folder the
 * person picked is a project, so the prompts read it. A prompt fills the
 * composer and does not send: the words are theirs to change.
 *
 * The card reads the tape, not a flag: it is there while nothing has been
 * said, and gone for good once a user line is on the tape.
 */
export function Starters({ persona, onPick }: { persona: Persona; onPick(text: string): void }) {
	const own = isOwnFolder(persona.cwd);
	const prompts = own
		? [
				"Make a small command-line game in this folder and run it once to show me it works.",
				"Write a README here that says what this folder is for, from your goal.",
				"What can you do from here? Give me three concrete things.",
			]
		: [
				"Tell me how this project is built and where to start reading.",
				"Find something small worth fixing here, and fix it.",
				"What would you change first in this project, and why?",
			];
	return (
		<div className="mx-auto mb-3 w-full max-w-[46rem]" role="group" aria-label="Things to try">
			<div className="grouped">
				<div className="group-row">
					<p className="group-row-text text-sm text-ink-3" style={{ whiteSpace: "normal" }}>
						A conversation runs in chapters: one closes after a quiet stretch with a handoff note, and {persona.name} remembers across them.
					</p>
				</div>
				{prompts.map((prompt) => (
					<button key={prompt} type="button" className="group-row group-row-choice w-full text-left" onClick={() => onPick(prompt)}>
						<span className="group-row-text text-sm text-ink" style={{ whiteSpace: "normal" }}>
							{prompt}
						</span>
					</button>
				))}
			</div>
		</div>
	);
}

/** A folder under the data directory is one Toad made for the teammate. */
function isOwnFolder(cwd: string): boolean {
	const data = dataDirectory();
	return data !== "" && (cwd === data || cwd.startsWith(data.endsWith("/") || data.endsWith("\\") ? data : `${data}/`) || cwd.startsWith(`${data}\\`));
}
