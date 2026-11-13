import { describe, expect, test } from "bun:test";
import type { TranscriptEvent } from "../src/generated/contract";
import { pathGroups } from "../src/outcomes";
import type { Step } from "../src/scheduledRuns";
import { toBlocks } from "../src/transcriptFolds";

// task-449 (George, 2026-11-12): a helper or a command an ACP agent starts is
// said once in the path. The desk says the agent's own start in its words
// ("Started a helper", "Started a command"), whatever the agent called its
// tool, and the run's own line stands for a start that went through; a
// failed one stays. The fixtures are the dev desk's real tapes of
// 2026-11-12: Quill (Claude Code, claude-agent-acp 0.106) in
// transcripts/1e615299…/1.jsonl, the four-helper turn at lines 804-843 and
// the web server turn at 794-802, and Goldie (Hotline Agent) in
// transcripts/79d12f56…/1.jsonl at 2935-2943. The steps the desk now writes
// for a failed launch, a backgrounded command and Codex's calls carry the
// kinds and words its core gives them (see the ACP driver's
// `a_call_is_kept_under_the_desks_kind_by_the_tool_behind_it`).

type Said = { key: string; one: string; many: string; doing: string };
const said = (key: string, one: string, many = one): Said => ({ key, one, many, doing: one });
const tool = (id: string, ts: number, title: string, toolKind: string, words: Said, status: "completed" | "failed" | "in_progress" = "completed"): TranscriptEvent => ({
	kind: "tool",
	id: `tool:${id}`,
	ts,
	endedTs: ts + 500,
	toolCallId: id,
	title,
	toolKind,
	status,
	said: words,
});
const helper = (thread: string, ts: number, title: string, more: Record<string, unknown> = {}): TranscriptEvent =>
	({ kind: "link", id: `link:run:${thread}`, ts, thread, threadKind: "run", title, state: "closed", end: "done", personaId: "1e615299", elapsedMs: 116_744, ...more }) as TranscriptEvent;
const activity = (id: string, ts: number, message: string): TranscriptEvent => ({ kind: "activity", id, ts, type: "searching", message });
const user = (id: string, ts: number, text: string): TranscriptEvent => ({ kind: "user", id, ts, text });
const reply = (id: string, ts: number, text: string): TranscriptEvent => ({ kind: "agent", id, ts, text });
const end = (id: string, ts: number): TranscriptEvent => ({ kind: "turn", id, ts, stopReason: "end_turn" });

const declaredActivity = (id: string, ts: number) => tool(id, ts, "mcp__hotline__activity", "mcp__hotline__activity", said("activity", "Said what it is on"));
const declaredOutcome = (id: string, ts: number) => tool(id, ts, "mcp__hotline__outcome", "mcp__hotline__outcome", said("outcome", "Said what came of it"));
const ownCommand = (id: string, ts: number, title: string, words: string) => tool(id, ts, title, "execute", said("own:execute", words, "Ran {n} commands"));

/** Quill's four-helper turn, as its tape has it: Claude's adapter never shows the launches, only the four announcements. */
const fourHelpers: TranscriptEvent[] = [
	user("0b7457b7", 1794521359920, "help me identify open source desktop agent tools similar to hotline and let's make a comparison chart"),
	reply("7de6302c", 1794521362952, "I'll scan the field and build the chart. This will take a few minutes."),
	tool("toolu_018SqHqMWZWXYAK1HxpJ86Z9", 1794521362953, "ToolSearch", "other", said("tools", "Looked up its tools")),
	ownCommand("toolu_01SkCQw23PtbGqfVvQynmGzY", 1794521403837, "for r in iOfficeAI/AionUi Orkas-AI/Orkas traycerai/traycer…", "Look up license, stars and activity for candidate repos"),
	declaredActivity("toolu_01NdUJc9fnzhTay6TEygqB9Z", 1794521441797),
	activity("53352ebb", 1794521464512, "Checking each tool's features at the source"),
	helper("3ddb7284", 1794521464528, "Profile agent tools group A"),
	helper("4cb8275c", 1794521464531, "Profile agent tools group B"),
	helper("b9f0d42d", 1794521464533, "Profile agent tools group C"),
	helper("65125220", 1794521464536, "Profile agent tools group D"),
	ownCommand("toolu_01Lz9mnJkGDZVxVrVPRq3jNC", 1794521466201, "cd /Users/george/Projects/personal/hotline && git tag --sort=-creatordate | head -2…", "Check Hotline release and docs for its own row"),
	declaredOutcome("toolu_012WrgRKgx1iys2LLvEN3iGX", 1794521776722),
	{ kind: "outcome", id: "9a334e01", ts: 1794521778164, type: "made", message: "Comparison chart of 16 agent tools vs Hotline", key: "9a334e01" },
	reply("836", 1794521778155, "Comparison chart: 16 open source agent tools next to Hotline."),
	end("a30ffd66", 1794521783405),
];

/** A turn's path, a line per entry: a line of steps by its words, a helper or command line as the pane says it. */
function path(events: TranscriptEvent[]): string[][] {
	const steps = toBlocks(events).flatMap((block) => (block.kind === "steps" ? [block.items] : []));
	expect(steps.length).toBe(1);
	return pathGroups(steps[0] as Step[], false).map((group) => [
		`# ${group.heading?.message ?? ""}`,
		...group.entries.map((entry) =>
			entry.kind === "line"
				? `${entry.line.text}${entry.line.failed ? " (failed)" : ""}`
				: entry.kind === "helpers"
					? entry.links.length === 1
						? `Started a helper: ${entry.links[0]!.title}`
						: `Started ${entry.links.length} helpers`
					: entry.kind === "command"
						? `Started a command: ${entry.link.title}`
						: `outcome: ${entry.outcome.message}`,
		),
	]);
}

describe("a start is said once in the path", () => {
	test("four helpers Claude started together are one line, and its own steps and declarations are left as they are", () => {
		expect(path(fourHelpers)).toEqual([
			["# ", "Looked up its tools", "Look up license, stars and activity for candidate repos"],
			["# Checking each tool's features at the source", "Started 4 helpers", "Check Hotline release and docs for its own row", "outcome: Comparison chart of 16 agent tools vs Hotline"],
		]);
	});

	test("one helper's start is its line, by Hotline Agent's step as by the helper's line", () => {
		// Goldie, 2026-11-07 20:40: the `subagent` step and its run share an id.
		const goldie: TranscriptEvent[] = [
			user("u", 1, "launch a subagent that takes action like creating searching and editing some files"),
			reply("a", 2, "On it."),
			tool("6238848e", 3, "subagent QA subagent file activity (3–4 minutes)", "subagent", said("subagent", "Started a helper", "Started {n} helpers")),
			helper("6238848e", 4, "QA subagent file activity (3–4 minutes)"),
			reply("b", 5, "Launched."),
			end("e", 6),
		];
		expect(path(goldie)).toEqual([["# ", "Started a helper: QA subagent file activity (3–4 minutes)"]]);
	});

	test("a launch that failed started nothing and stays beside the four that went, in the desk's words", () => {
		// Claude's adapter shows a launch that became no helper, failed, titled by its task; Codex's is `spawnAgent`.
		const failed = tool("toolu_e", 1794521464540, "Profile agent tools group E", "subagent", said("subagent", "Started a helper", "Started {n} helpers"), "failed");
		const codex = tool("call_codex_spawn", 1794521464541, "spawnAgent", "subagent", said("subagent", "Started a helper", "Started {n} helpers"), "failed");
		const events = [...fourHelpers.slice(0, 10), failed, codex, ...fourHelpers.slice(10)];
		expect(path(events)[1]).toEqual([
			"# Checking each tool's features at the source",
			"Started 4 helpers",
			"Started 2 helpers (failed)",
			"Check Hotline release and docs for its own row",
			"outcome: Comparison chart of 16 agent tools vs Hotline",
		]);
	});

	test("a start no helper's line stands for stays, and an unrelated step is never taken for one", () => {
		const alone = tool("toolu_f", 2, "Agent", "subagent", said("subagent", "Started a helper", "Started {n} helpers"));
		expect(path([user("u", 1, "go"), alone, ownCommand("toolu_g", 3, "ls", "List the workspace"), end("e", 4)])).toEqual([["# ", "Started a helper", "List the workspace"]]);
	});

	test("a command left running is its own line, and the step that sent it there is not said again", () => {
		// Quill's web server turn, had Claude sent the server to the background with its own Bash: the step
		// the desk writes again as the start ("background"), and the command's run.
		const turn: TranscriptEvent[] = [
			user("8131e604", 1794520613452, "use 66"),
			ownCommand("toolu_01XAAoWBHCi3pCPbjqUSctSh", 1794520615748, 'lsof -nP -iTCP:8766 -sTCP:LISTEN; echo "exit=$?"', "Check whether port 8766 is free"),
			tool("toolu_bg", 1794520620390, "python3 -I serve_octopus.py", "background", said("command", "Started a command", "Started {n} commands")),
			helper("4f0c1d2e", 1794520620400, "python3 -I serve_octopus.py", { runKind: "shell", state: "live", end: undefined }),
			declaredOutcome("toolu_01Gn9rRhNJNmamGGbhteRstn", 1794520624050),
			{ kind: "outcome", id: "fd874cd9", ts: 1794520625541, type: "open", message: "octopus.md served at http://127.0.0.1:8766", key: "octopus-server" },
			reply("a6ef37bd", 1794520627380, "octopus.md is up at http://127.0.0.1:8766."),
			end("e26868d3", 1794520627381),
		];
		expect(path(turn)).toEqual([["# ", "Check whether port 8766 is free", "Started a command: python3 -I serve_octopus.py", "outcome: octopus.md served at http://127.0.0.1:8766"]]);
	});

	test("Codex's calls to activity and outcome are the lines they wrote, not steps", () => {
		// Codex titles a Hotline tool `mcp.hotline.<tool>`; the desk keeps it under the tool's own kind and words.
		const turn: TranscriptEvent[] = [
			user("u", 1, "check the docs"),
			tool("call_a", 2, "mcp.hotline.activity", "mcp__hotline__activity", said("activity", "Said what it is on")),
			activity("d", 3, "Reading the docs"),
			tool("call_r", 4, "Read README.md", "read", said("read", "Read README.md", "Read {n} files")),
			tool("call_o", 5, "mcp.hotline.outcome", "mcp__hotline__outcome", said("outcome", "Said what came of it")),
			{ kind: "outcome", id: "o", ts: 6, type: "made", message: "A summary of the docs", key: "o" },
			reply("a", 7, "Done."),
			end("e", 8),
		];
		expect(path(turn)).toEqual([["# Reading the docs", "Read README.md", "outcome: A summary of the docs"]]);
	});
});
