import { expect, spyOn, test } from "bun:test";
import type { CookieOffer } from "../src/generated/contract";
import { installDom } from "./dom";

test("A lend into the desk browser is a row under the teammate's secrets, in sight without opening them, and Remove takes it back", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { SecretRows } = await import("../src/components/teammate/SecretRows");
	const { wire } = await import("../src/wire");
	const lend: CookieOffer = {
		id: "desk",
		ts: 1,
		teammate: "ada",
		site: "porkbun.com",
		browserName: "Firefox",
		profileName: "Default",
		login: "lent-firefox-default-desk",
		status: "lent",
		deskBrowser: true,
	};
	const { deskBrowser: _, ...computerLend } = { ...lend, id: "computer", site: "github.com" };
	let offers = [lend, computerLend];
	const calls: unknown[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		if (cmd === "computer.cookies.offers") return offers;
		if (cmd === "secrets.list") return [];
		calls.push({ cmd, params });
		offers = [{ ...lend, status: "removed" }, computerLend];
		return null;
	}) as typeof wire.command);
	let refresh = () => {};
	const subscribe = spyOn(wire, "subscribe").mockImplementation(((_target: unknown, handlers: { snapshot(items: unknown[]): void }) => {
		refresh = () => handlers.snapshot([]);
		return () => {};
	}) as typeof wire.subscribe);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => root.render(<SecretRows personaId="ada" teammate="Ada" granted={[]} computer={false} disabled={false} onChange={() => {}} />));
		// In sight without opening the fold: a row with the site, whose and from where.
		expect(container.textContent).toContain("porkbun.com");
		expect(container.textContent).toContain("Signed in as you, from Firefox");
		expect(container.textContent).not.toContain("github.com");
		await act(async () => Array.from(container.querySelectorAll("button")).find((button) => button.textContent === "Remove")!.click());
		expect(calls).toEqual([{ cmd: "computer.cookies.remove", params: { personaId: "ada", id: "desk" } }]);
		await act(async () => refresh());
		expect(container.textContent).not.toContain("porkbun.com");
	} finally {
		await act(async () => root.unmount());
		command.mockRestore();
		subscribe.mockRestore();
		await teardown();
	}
});
