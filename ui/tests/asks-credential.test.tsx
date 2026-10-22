import { expect, spyOn, test } from "bun:test";
import type { TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

type Ask = Extract<TranscriptEvent, { kind: "credential_ask" }>;
const pending: Ask = { kind: "credential_ask", id: "credential:1", actionId: "credential:1", ts: 1, site: "https://github.com", reason: "Sign in to the repository", credentialKind: "login", name: "GITHUB_LOGIN", status: "pending", expiresAt: Date.now() + 1_800_000, existing: [], armed: false };

test("Asks capture sends one command and clears all typed fields on Save, decline, settlement and dismiss", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { CredentialAsk } = await import("../src/components/transcript/CredentialAsk");
	const { wire } = await import("../src/wire");
	const commands: { cmd: string; params: unknown }[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => { commands.push({ cmd, params }); return null; }) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const render = async (event: Ask, key = "same") => act(async () => { root.render(<CredentialAsk key={key} event={event} teammate="Pork Chop" />); });
	const field = (name: string) => container.querySelector<HTMLInputElement>(`[name="${name}"]`)!;
	const fill = () => {
		field("username").value = "private-user";
		field("password").value = "private-password";
		field("totp").value = "JBSWY3DPEHPK3PXP";
		return [field("username"), field("password"), field("totp")];
	};
	const click = async (label: string) => act(async () => { Array.from(container.querySelectorAll("button")).find(b => b.textContent === label)!.click(); });
	try {
		await render(pending);
		expect(container.textContent).toContain("https://github.com");
		expect(field("ask").checked).toBe(false);
		const saved = fill();
		field("name").value = "CHOSEN_LOGIN";
		await act(async () => { container.querySelector("form")!.dispatchEvent(new window.Event("submit", { bubbles: true, cancelable: true })); });
		expect(commands).toEqual([{ cmd: "secrets.capture.answer", params: { actionId: "credential:1", name: "CHOSEN_LOGIN", ask: false, values: { username: "private-user", password: "private-password", totp: "JBSWY3DPEHPK3PXP" } } }]);
		expect(saved.map(f => f.value)).toEqual(["", "", ""]);
		await render({ ...pending, status: "stored", storedAs: "CHOSEN_LOGIN" });
		expect(container.textContent).toContain("Stored as CHOSEN_LOGIN");
		expect(container.textContent).toContain("Pork Chop");
		expect(container.textContent).not.toContain(pending.reason);
		expect(container.querySelectorAll("input, button").length).toBe(0);
		for (const status of ["declined", "expired", "withdrawn"] as const) {
			await render(pending, status);
			const typed = fill();
			const before = commands.length;
			await render({ ...pending, status }, status);
			expect(typed.map(f => f.value)).toEqual(["", "", ""]);
			expect(commands.length).toBe(before);
			expect(container.querySelectorAll("input, button").length).toBe(0);
		}
		await render(pending, "decline-button");
		const declined = fill();
		await click("Decline");
		expect(commands.at(-1)).toEqual({ cmd: "secrets.capture.answer", params: { actionId: "credential:1", ask: false } });
		expect(declined.map(f => f.value)).toEqual(["", "", ""]);
		await render(pending, "local-expiry");
		const locallyExpired = fill();
		const beforeExpiry = commands.length;
		await render({ ...pending, expiresAt: Date.now() - 1 }, "local-expiry");
		await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
		expect(locallyExpired.map(f => f.value)).toEqual(["", "", ""]);
		expect(commands.length).toBe(beforeExpiry);
		expect(container.textContent).toContain("Expired");
		await render(pending, "dismiss");
		const dismissed = fill();
		await act(async () => { root.render(null); });
		expect(dismissed.map(f => f.value)).toEqual(["", "", ""]);
		await render({ ...pending, existing: ["OLD_LOGIN"] }, "existing");
		fill();
		await click("Grant OLD_LOGIN instead");
		expect(commands.at(-1)).toEqual({ cmd: "secrets.capture.answer", params: { actionId: "credential:1", name: "OLD_LOGIN", ask: false } });
		await render({ ...pending, credentialKind: "passkey", name: "GH_PASSKEY" }, "passkey");
		expect(container.querySelector('[name="password"]')).toBeNull();
		expect(container.querySelector('[name="username"]')).toBeNull();
		field("ask").checked = true;
		await act(async () => { container.querySelector("form")!.dispatchEvent(new window.Event("submit", { bubbles: true, cancelable: true })); });
		expect(commands.at(-1)).toEqual({ cmd: "secrets.capture.answer", params: { actionId: "credential:1", name: "GH_PASSKEY", ask: true } });
	} finally {
		await act(async () => { root.unmount(); });
		command.mockRestore();
		await teardown();
	}
});
