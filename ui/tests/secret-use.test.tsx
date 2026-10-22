import { expect, spyOn, test } from "bun:test";
import type { SharedSecret, TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

test("Settings shows Automatic or Ask me for each key and saves the selected policy", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { SecretsSection } = await import("../src/components/Secrets");
	const { wire } = await import("../src/wire");
	let stored: SharedSecret[] = [{ name: "LOGIN", kind: "login", updatedAt: 1, use: "auto" }, { name: "KEY", kind: "passkey", updatedAt: 1, use: "ask" }, { name: "TOKEN", kind: "variable", updatedAt: 1, use: "auto" }];
	const commands: unknown[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: { name: string; use: "auto" | "ask" }) => {
		if (cmd === "secrets.list") return stored;
		if (cmd === "computer.cookies.blocked") return [];
		commands.push({ cmd, params });
		const result = { ...stored.find(s => s.name === params.name)!, use: params.use };
		stored = stored.map(s => s.name === params.name ? result : s);
		return result;
	}) as typeof wire.command);
	const container = document.createElement("div"); document.body.append(container);
	const root = createRoot(container);
	const choice = (name: string, label: string) => Array.from(container.querySelectorAll<HTMLButtonElement>(`fieldset[aria-label="Use ${name}"] button`)).find(b => b.textContent === label)!;
	try {
		await act(async () => { root.render(<SecretsSection />); });
		expect(choice("LOGIN", "Automatic").getAttribute("aria-pressed")).toBe("true");
		expect(choice("KEY", "Ask me").getAttribute("aria-pressed")).toBe("true");
		expect(container.querySelector('[aria-label="Use TOKEN"]')).toBeNull();
		await act(async () => { choice("LOGIN", "Ask me").click(); });
		await act(async () => { choice("KEY", "Automatic").click(); });
		expect(commands).toEqual([{ cmd: "secrets.use.set", params: { name: "LOGIN", use: "ask" } }, { cmd: "secrets.use.set", params: { name: "KEY", use: "auto" } }]);
		await act(async () => { root.render(null); });
		await act(async () => { root.render(<SecretsSection />); });
		expect(choice("LOGIN", "Ask me").getAttribute("aria-pressed")).toBe("true");
		expect(choice("KEY", "Automatic").getAttribute("aria-pressed")).toBe("true");
	} finally { await act(async () => root.unmount()); command.mockRestore(); await teardown(); }
});

test("Secret use card sends Approve, Allow 10 min and Deny once and settles or expires without buttons", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { SecretUseAsk } = await import("../src/components/transcript/SecretUseAsk");
	const { wire } = await import("../src/wire");
	const calls: unknown[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => { calls.push({ cmd, params }); return null; }) as typeof wire.command);
	const container = document.createElement("div"); document.body.append(container);
	const root = createRoot(container);
	const pending: Extract<TranscriptEvent, { kind: "secret_use_ask" }> = { kind: "secret_use_ask", id: "card", ts: 1, useId: "opaque", secret: "GH_PASSKEY", secretKind: "passkey", site: "github.com", path: "/login", teammate: "ada", askedAt: Date.now(), expiresAt: Date.now() + 300_000, status: "pending" };
	try {
		for (const [label, answer] of [["Approve", "approve"], ["Allow 10 min", "allow"], ["Deny", "deny"]]) {
			await act(async () => { root.render(<SecretUseAsk key={label} event={pending} teammate="Pork Chop" />); });
			expect(container.textContent).toContain("github.com/login");
			expect(container.textContent).toContain("Pork Chop");
			const button = Array.from(container.querySelectorAll("button")).find(b => b.textContent === label)!;
			await act(async () => button.click());
			await act(async () => button.click());
			expect(calls.at(-1)).toEqual({ cmd: "secrets.use.answer", params: { personaId: "ada", useId: "opaque", answer } });
		}
		expect(calls.length).toBe(3);
		for (const status of ["approved", "allowed", "denied", "expired"] as const) {
			await act(async () => { root.render(<SecretUseAsk key={status} event={{ ...pending, status }} teammate="Pork Chop" />); });
			expect(container.querySelectorAll("button").length).toBe(0);
		}
		await act(async () => { root.render(<SecretUseAsk key="expired-locally" event={{ ...pending, expiresAt: Date.now() - 1 }} teammate="Pork Chop" />); });
		expect(container.textContent).toContain("Expired");
		expect(container.querySelectorAll("button").length).toBe(0);
	} finally { await act(async () => root.unmount()); command.mockRestore(); await teardown(); }
});
