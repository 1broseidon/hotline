import type { Step } from "./scheduledRuns";
import { stepTitle } from "./stepText";

/*
 * A turn's steps at a glance: what the teammate did in a few words, never the
 * command it ran. Steps of one kind in a row are one line ("Read 3 files",
 * "Ran 2 commands", "Used Linear 4 times"); the pane and the phone's work
 * sheet still show each step as it was run. The words are what the peek's
 * lines and the work card say, on the desk and on the phone alike.
 */

/** One line of the glance: the steps it stands for, the first of them its id. */
export type Glance = { id: string; ids: string[]; text: string; live: boolean; failed: boolean };

/** How a kind of step is said: once, several times, and while it runs. */
type Family = { key: string; one: string; many: (n: number) => string; doing: string };

const SERVICE_NAMES: Record<string, string> = {
	github: "GitHub",
	gmail: "Gmail",
	linear: "Linear",
	calendar: "Calendar",
	drive: "Drive",
	brainfile: "the board",
	hotline: "Hotline",
	runprism: "Prism",
	slack: "Slack",
	notion: "Notion",
	figma: "Figma",
};

/** "claude_ai_Prism__linear__list_issues" → "Linear"; "brainfile__get_task" → "the board". */
function serviceOf(rest: string): string {
	const parts = rest.split("__").filter((part) => part !== "");
	let server = (parts[0] ?? "").replace(/^claude_ai_/i, "");
	// A gateway's tools are named for the service behind it: Prism's linear__list_issues is Linear.
	if (/^prism$/i.test(server) && parts.length > 2) server = parts[1]!;
	const plain = server.toLowerCase().replace(/[^a-z0-9]/g, "");
	return SERVICE_NAMES[plain] ?? server.replace(/[_-]+/g, " ").replace(/^\w/, (c) => c.toUpperCase());
}

const times = (n: number) => (n === 2 ? "twice" : `${n} times`);

/** The last part of a path, for "Read design.md". */
function fileOf(step: Extract<Step, { kind: "tool" }>): string | undefined {
	const path = step.locations?.[0] ?? /^(?:read|write|edit)\s+(\S+)$/i.exec(step.title.trim())?.[1];
	return path?.split("/").filter(Boolean).pop();
}

/** The program a command runs, said as a person would: "git status", "a Python script". */
function programOf(title: string): string {
	const command = stepTitle(title)
		.replace(/^(?:Run|shell)\s+/i, "")
		.replace(/^(?:sudo|env\s+\S+=\S+|\S+=\S+)\s+/, "")
		.trim();
	const words = command.split(/\s+/);
	const program = (words[0] ?? "").split("/").pop() ?? "";
	if (/^python\d*(?:\.\d+)?$/.test(program)) return "a Python script";
	if (/^(?:node|bun|deno|tsx|ts-node)$/.test(program) && !/^(?:install|add|test|run)$/.test(words[1] ?? "")) return "a script";
	if (/^(?:git|gh|npm|npx|bun|cargo|make|docker|brainfile|pnpm|yarn|go|uv|pip|brew|kubectl)$/.test(program)) {
		const sub = words.slice(1).find((word) => /^[a-z][a-z-]*$/.test(word));
		return sub !== undefined ? `${program} ${sub}` : program;
	}
	return program === "" ? "a command" : program;
}

/** What kind of step this is, and how to say it. */
function familyOf(step: Step): Family {
	if (step.kind === "thought") return { key: "think", one: "Thought it through", many: () => "Thought it through", doing: "Thinking" };
	const kind = (step.toolKind ?? "").toLowerCase();
	const title = step.title.trim();
	const name = (/^[A-Za-z0-9_-]+$/.test(title) ? title : kind).toLowerCase();
	const mcp = /^mcp__(.+)$/i.exec(title);
	if (mcp) {
		const service = serviceOf(mcp[1]!);
		return { key: `mcp:${service}`, one: `Used ${service}`, many: (n) => `Used ${service} ${times(n)}`, doing: `Using ${service}` };
	}
	if (name.startsWith("computer__")) {
		if (name === "computer__browser") return { key: "browser", one: "Used the browser", many: (n) => `Used the browser ${times(n)}`, doing: "Browsing" };
		if (name === "computer__capture") return { key: "screen", one: "Looked at the screen", many: (n) => `Looked at the screen ${times(n)}`, doing: "Looking at the screen" };
		return { key: "computer", one: "Used the computer", many: (n) => `Used the computer ${times(n)}`, doing: "Using the computer" };
	}
	const words: Record<string, Family> = {
		post_to_chat: { key: "post", one: "Posted to the chat", many: (n) => `Posted to the chat ${times(n)}`, doing: "Posting to the chat" },
		send_file: { key: "send", one: "Sent a file", many: (n) => `Sent ${n} files`, doing: "Sending a file" },
		generate_image: { key: "image", one: "Made an image", many: (n) => `Made ${n} images`, doing: "Making an image" },
		web_search: { key: "web", one: "Searched the web", many: (n) => `Searched the web ${times(n)}`, doing: "Searching the web" },
		search_thread: { key: "history", one: "Searched the conversation", many: (n) => `Searched the conversation ${times(n)}`, doing: "Searching the conversation" },
		read_thread: { key: "history", one: "Read back the conversation", many: () => "Read back the conversation", doing: "Reading back" },
		message_teammate: { key: "colleague", one: "Messaged a teammate", many: (n) => `Messaged teammates ${times(n)}`, doing: "Messaging a teammate" },
		list_teammates: { key: "colleague", one: "Checked the team", many: () => "Checked the team", doing: "Checking the team" },
		request_human: { key: "ask", one: "Asked you", many: (n) => `Asked you ${times(n)}`, doing: "Asking you" },
		ask_question: { key: "ask", one: "Asked you", many: (n) => `Asked you ${times(n)}`, doing: "Asking you" },
		request_credential: { key: "ask", one: "Asked you to sign in", many: (n) => `Asked you ${times(n)}`, doing: "Asking you to sign in" },
		wait_jobs: { key: "wait", one: "Waited on a job", many: () => "Waited on jobs", doing: "Waiting on a job" },
		toolsearch: { key: "tools", one: "Looked up its tools", many: () => "Looked up its tools", doing: "Looking up its tools" },
		tool_schema: { key: "tools", one: "Looked up its tools", many: () => "Looked up its tools", doing: "Looking up its tools" },
	};
	const named = words[name];
	if (named !== undefined) return named;
	if (/schedule|^loop$/.test(name)) return { key: "schedule", one: "Changed a schedule", many: () => "Changed schedules", doing: "Changing a schedule" };
	if (kind === "read") {
		const file = fileOf(step);
		return { key: "read", one: file !== undefined ? `Read ${file}` : "Read a file", many: (n) => `Read ${n} files`, doing: "Reading" };
	}
	if (kind === "edit" || kind === "write" || kind === "delete" || kind === "move") {
		const file = fileOf(step);
		return { key: "edit", one: file !== undefined ? `Edited ${file}` : "Edited a file", many: (n) => `Edited ${n} files`, doing: "Editing" };
	}
	if (kind === "search" || kind === "grep" || kind === "glob") return { key: "search", one: "Searched the files", many: (n) => `Searched ${times(n)}`, doing: "Searching" };
	if (kind === "fetch" || /^web_fetch/i.test(title)) return { key: "fetch", one: "Read a web page", many: (n) => `Read ${n} web pages`, doing: "Reading a web page" };
	if (kind === "execute" || /^(?:shell|Run)\s/i.test(stepTitle(title)) || /^(?:ba|z)?sh\s/.test(title)) {
		const program = programOf(title);
		return { key: "execute", one: `Ran ${program}`, many: (n) => `Ran ${n} commands`, doing: `Running ${program}` };
	}
	if (kind === "think") return { key: "think", one: "Thought it through", many: () => "Thought it through", doing: "Thinking" };
	// Anything else by its own name, a few words of it.
	const said = stepTitle(title).split(/\s+/).slice(0, 4).join(" ");
	return { key: `other:${said}`, one: said, many: (n) => `${said} (${n})`, doing: said };
}

/**
 * The steps as lines a person reads at a glance. `running` is whether the
 * turn is still at work: its last line then says what is happening now.
 */
export function glance(items: Step[], running: boolean): Glance[] {
	const lines: (Glance & { family: Family })[] = [];
	for (const step of items) {
		const family = familyOf(step);
		const failed = step.kind === "tool" && step.status === "failed";
		const last = lines[lines.length - 1];
		if (last !== undefined && last.family.key === family.key) {
			last.ids.push(step.id);
			// The newest of the run says what it is doing now ("Running cargo test").
			last.family = family;
			last.failed ||= failed;
			// A run of one kind says its count, unless it is one thing said once.
			last.text = family.key === "think" ? family.one : last.family.many(last.ids.length);
		} else {
			lines.push({ id: step.id, ids: [step.id], text: family.one, live: false, failed, family });
		}
	}
	const last = lines[lines.length - 1];
	if (running && last !== undefined) {
		last.live = true;
		last.text = last.family.doing;
	}
	return lines.map(({ family: _family, ...line }) => line);
}
