import { expect, spyOn, test } from "bun:test";
import type { Credential, Provider } from "../src/generated/contract";
import { installDom } from "./dom";

const credential = (id: string, label: string): Credential => ({
	id,
	providerId: "anthropic",
	credentialKind: "api_key",
	label,
	revoked: false,
	createdAt: 1,
	updatedAt: 1,
});
const providers: Provider[] = [
	{ id: "anthropic", name: "Anthropic", credentialKinds: ["api_key"], modelDiscovery: false, does: ["chat"] },
	{ id: "openai", name: "OpenAI", credentialKinds: ["api_key"], modelDiscovery: false, does: ["chat"] },
];

test("accounts are named once a provider has two, and the first name of a new one is numbered", async () => {
	const { accountName, nextAccountLabel } = await import("../src/accounts");
	expect(accountName("Anthropic", "Anthropic", false)).toBe("Anthropic");
	expect(accountName("Anthropic", "Work", false)).toBe("Anthropic");
	expect(accountName("Anthropic", "Work", true)).toBe("Anthropic · Work");
	expect(accountName("Anthropic", "Anthropic 2", true)).toBe("Anthropic 2");
	expect(nextAccountLabel("Anthropic", 0)).toBe("Anthropic");
	expect(nextAccountLabel("Anthropic", 1)).toBe("Anthropic 2");
});

test("a connected provider can be added again, each account is its own row, and its page renames it", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { ProvidersSection } = await import("../src/components/settings/Providers");
	const { wire } = await import("../src/wire");
	const { replaceDesks, setActiveDesk, LOCAL_DESK } = await import("../src/desks");
	replaceDesks([{ id: LOCAL_DESK, name: "This computer", kind: "local", origin: "http://127.0.0.1:1", token: "token" }]);
	setActiveDesk(LOCAL_DESK);
	const calls: { cmd: string; params: unknown }[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push({ cmd, params });
		if (cmd === "credential.list") return [credential("one", "Anthropic"), credential("two", "Work")];
		if (cmd === "providers.list") return providers;
		if (cmd === "models.catalog") return [];
		if (cmd === "credential.rename") return credential(String(params["id"]), String(params["label"]));
		throw new Error("not in this test");
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const titles = () => [...container.querySelectorAll(".group-row-title")].map((title) => title.textContent);
	try {
		await act(async () => root.render(<ProvidersSection enabledModels={{}} />));
		expect(titles()).toEqual(["Anthropic", "Anthropic · Work"]);
		const add = [...container.querySelectorAll("button")].find((button) => button.textContent?.includes("Add provider"))!;
		await act(async () => add.click());
		expect(container.textContent).toContain("Anthropic");
		expect(container.textContent).not.toContain("Every provider Hotline knows is already here.");
		const row = [...container.querySelectorAll<HTMLButtonElement>(".group-row-choice")].find((button) => button.textContent?.includes("Anthropic · Work"))!;
		await act(async () => row.click());
		const name = container.querySelector<HTMLInputElement>('input[aria-label="Account name"]')!;
		expect(name.value).toBe("Work");
		await act(async () => {
			const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
			setter.call(name, "Personal");
			name.dispatchEvent(new window.Event("input", { bubbles: true }));
		});
		await act(async () => {
			name.dispatchEvent(new window.FocusEvent("focusout", { bubbles: true }));
		});
		expect(calls.filter((call) => call.cmd === "credential.rename")).toEqual([{ cmd: "credential.rename", params: { id: "two", label: "Personal" } }]);
	} finally {
		await act(async () => root.unmount());
		replaceDesks([]);
		command.mockRestore();
		await teardown();
	}
});

test("an agent with accounts is added from the same list with no form, named after the one on this computer, and its page removes it", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { ProvidersSection } = await import("../src/components/settings/Providers");
	const { wire } = await import("../src/wire");
	const { replaceDesks, setActiveDesk, LOCAL_DESK } = await import("../src/desks");
	replaceDesks([{ id: LOCAL_DESK, name: "This computer", kind: "local", origin: "http://127.0.0.1:1", token: "token" }]);
	setActiveDesk(LOCAL_DESK);
	const made: Credential = { ...credential("acct", "Claude Code 2"), providerId: "claude-acp", credentialKind: "agent" };
	const calls: { cmd: string; params: unknown }[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push({ cmd, params });
		if (cmd === "credential.list") return [credential("one", "Anthropic")];
		if (cmd === "providers.list") return providers;
		if (cmd === "backends.list")
			return [
				{ id: "hotline", name: "Hotline Agent", description: "" },
				{ id: "claude-acp", name: "Claude Code", description: "", accounts: true },
				{ id: "cursor", name: "Cursor", description: "" },
			];
		if (cmd === "credential.create") return made;
		if (cmd === "credential.delete") return null;
		throw new Error("not in this test");
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const buttons = () => [...container.querySelectorAll<HTMLButtonElement>("button")];
	try {
		await act(async () => root.render(<ProvidersSection enabledModels={{}} />));
		await act(async () => buttons().find((button) => button.textContent?.includes("Add provider"))!.click());
		const rows = buttons().filter((button) => button.textContent?.includes("Another account"));
		expect(rows.map((row) => row.querySelector(".group-row-title")?.textContent)).toEqual(["Claude Code"]);
		await act(async () => rows[0]!.click());
		expect(calls.find((call) => call.cmd === "credential.create")?.params).toEqual({ providerId: "claude-acp", label: "Claude Code 2", secret: "" });
		// Its page, opened at once: no models to filter and no sign-in to start here.
		expect(container.querySelector("h2")?.textContent).toBe("Claude Code 2");
		expect(container.textContent).toContain("Not signed in yet");
		expect(container.textContent).not.toContain("Models in the picker");
		await act(async () => buttons().find((button) => button.textContent === "Sign out and remove")!.click());
		expect(calls.find((call) => call.cmd === "credential.delete")?.params).toEqual({ id: "acct" });
		expect([...container.querySelectorAll(".group-row-title")].map((title) => title.textContent)).toEqual(["Anthropic"]);
	} finally {
		await act(async () => root.unmount());
		replaceDesks([]);
		command.mockRestore();
		await teardown();
	}
});
