import { expect, spyOn, test } from "bun:test";
import type { ConfigChoice } from "../src/generated/contract";
import { installDom } from "./dom";

const notice = `Account no longer connected: This teammate's account was removed or signed out.\n\n${JSON.stringify({
	hotlineFailure: {
		kind: "account_gone",
		title: "Account no longer connected",
		summary: "This teammate's account was removed or signed out. Pick another account for it; it won't move to one on its own.",
		details: "This teammate's Anthropic account is no longer connected.",
		model: "anthropic/claude-x",
	},
})}`;

const listed: ConfigChoice[] = [
	{ id: "anthropic/claude-x", name: "Claude X", group: "Anthropic", account: { id: "work", label: "Work", kind: "api_key", shared: false } },
	{ id: "anthropic/claude-y", name: "Claude Y", group: "Anthropic", account: { id: "work", label: "Work", kind: "api_key", shared: false } },
	{ id: "openai/gpt-6", name: "GPT-6", group: "OpenAI", account: { id: "oai", label: "OpenAI", kind: "api_key", shared: false } },
];

test("a teammate on a vanished account is offered the accounts serving its model, and only a press moves it", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { ErrorCard } = await import("../src/components/ErrorCard");
	const { wire } = await import("../src/wire");
	const calls: { cmd: string; params: unknown }[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		calls.push({ cmd, params });
		if (cmd === "models.list") return listed;
		return null;
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	let again = 0;
	try {
		await act(async () => root.render(<ErrorCard text={notice} personaId="huck" onRunAgain={async () => { again++; }} />));
		expect(container.textContent).toContain("Account no longer connected");
		const buttons = [...container.querySelectorAll("button")].filter((button) => button.textContent?.startsWith("Use "));
		expect(buttons.map((button) => button.textContent)).toEqual(["Use Anthropic"]);
		expect(calls.map((call) => call.cmd)).toEqual(["models.list"]);
		expect(again).toBe(0);
		await act(async () => buttons[0]!.click());
		expect(calls.at(-1)).toEqual({ cmd: "session.set_model", params: { personaId: "huck", modelId: "anthropic/claude-x" } });
		expect(again).toBe(1);
	} finally {
		await act(async () => root.unmount());
		command.mockRestore();
		await teardown();
	}
});
