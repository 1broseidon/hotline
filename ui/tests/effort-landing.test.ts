import { expect, test } from "bun:test";
import { landsOn } from "../src/effortLanding";

const CLAUDE = ["default", "low", "medium", "high", "xhigh", "max"];
const SONNET = ["default", "low", "medium", "high", "max"];
const CODEX = ["low", "medium", "high", "xhigh", "ultra"];

// The core's table (models/effort.rs), so the dimmed guess is the answer that follows.
test("a switch lands as the core lands it", () => {
	const cases: [string, Parameters<typeof landsOn>[0], string | undefined][] = [
		["the teammate's own first", { levels: CLAUDE, default: "default", teammate: "low", desk: "max", current: "high", currentLevels: CLAUDE }, "low"],
		["else the desk's", { levels: CLAUDE, default: "default", desk: "xhigh", current: "low", currentLevels: CLAUDE }, "xhigh"],
		["else the current, where offered", { levels: CLAUDE, default: "default", current: "medium", currentLevels: CLAUDE }, "medium"],
		["else a step down, never up", { levels: SONNET, default: "default", current: "xhigh", currentLevels: CLAUDE }, "high"],
		["ultra onto Opus by the ladder", { levels: CLAUDE, default: "default", current: "ultra", currentLevels: CODEX }, "max"],
		["Auto carries over to Auto", { levels: SONNET, default: "default", current: "default", currentLevels: CLAUDE }, "default"],
		["Auto onto a model without it is its default", { levels: CODEX, default: "medium", current: "default", currentLevels: CLAUDE }, "medium"],
		["below everything is the default", { levels: ["medium", "high"], default: "high", current: "low", currentLevels: CODEX }, "high"],
		["no default is the first level", { levels: CODEX, currentLevels: [] }, "low"],
		["no levels, nothing", { levels: [], teammate: "high", current: "high", currentLevels: CLAUDE }, undefined],
	];
	for (const [why, switched, lands] of cases) expect([why, landsOn(switched)]).toEqual([why, lands]);
});
