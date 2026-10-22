import { expect, spyOn, test } from "bun:test";
import type { CookieOffer } from "../src/generated/contract";
import { installDom } from "./dom";

test("Cookie offer asks explicitly, warns of personal sign-in and removes the settled lend", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { CookieOfferCard } = await import(
		"../src/components/transcript/CookieOfferCard"
	);
	const { wire } = await import("../src/wire");
	const calls: unknown[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (
		cmd: string,
		params: unknown,
	) => {
		calls.push({ cmd, params });
		return null;
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const offer: CookieOffer = {
		id: "offer",
		ts: 1,
		teammate: "ada",
		site: "porkbun.com",
		browserName: "Brave",
		profileName: "Default",
		login: "lent-brave-default",
		status: "pending",
	};
	try {
		for (const [label, answer] of [
			["Use", "use"],
			["Not now", "not_now"],
			["Never for this site", "never"],
		]) {
			await act(async () =>
				root.render(
					<CookieOfferCard key={label} offer={offer} teammate="Ada" />,
				),
			);
			expect(container.textContent).toContain(
				"Use your existing cookie from Brave for porkbun.com?",
			);
			expect(container.textContent).toContain("Ada will be signed in as you.");
			const button = Array.from(container.querySelectorAll("button")).find(
				(b) => b.textContent === label,
			)!;
			await act(async () => button.click());
			await act(async () => button.click());
			expect(calls.at(-1)).toEqual({
				cmd: "computer.cookies.answer",
				params: { personaId: "ada", id: "offer", answer },
			});
		}
		expect(calls.length).toBe(3);
		await act(async () =>
			root.render(
				<CookieOfferCard offer={{ ...offer, status: "lent" }} teammate="Ada" />,
			),
		);
		expect(container.textContent).toContain(
			"Signed in as you on porkbun.com, from Brave",
		);
		await act(async () => container.querySelector("button")!.click());
		expect(calls.at(-1)).toEqual({
			cmd: "computer.cookies.remove",
			params: { personaId: "ada", id: "offer" },
		});
		await act(async () =>
			root.render(
				<CookieOfferCard
					offer={{ ...offer, status: "removed" }}
					teammate="Ada"
				/>,
			),
		);
		expect(container.querySelector("button")).toBeNull();
	} finally {
		await act(async () => root.unmount());
		command.mockRestore();
		await teardown();
	}
});

test("Never for this site can be undone for the named teammate in Settings", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { CookieSitesPage } = await import(
		"../src/components/settings/CookieOfferSettings"
	);
	const { wire } = await import("../src/wire");
	let blocked = [{ teammate: "ada", name: "Ada", site: "porkbun.com" }];
	const calls: unknown[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (
		cmd: string,
		params: unknown,
	) => {
		if (cmd === "computer.cookies.blocked") return blocked;
		if (cmd === "computer.cookies.sites") return { sites: [], unavailable: [] };
		calls.push({ cmd, params });
		blocked = [];
		return null;
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => root.render(<CookieSitesPage settings={{ enabled: true }} onBack={() => {}} />));
		expect(container.textContent).toContain("Ada · porkbun.com");
		await act(async () => Array.from(container.querySelectorAll("button")).find((button) => button.textContent === "Undo")!.click());
		expect(calls).toEqual([
			{
				cmd: "computer.cookies.reset",
				params: { personaId: "ada", site: "porkbun.com" },
			},
		]);
		expect(container.textContent).toContain("No sites hidden.");
	} finally {
		await act(async () => root.unmount());
		command.mockRestore();
		await teardown();
	}
});
