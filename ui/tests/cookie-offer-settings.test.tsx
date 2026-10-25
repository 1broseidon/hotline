import { expect, spyOn, test } from "bun:test";
import type { CookieOfferSettings as OfferSettings } from "../src/generated/contract";
import { installDom } from "./dom";

test("Computer has an offer switch and All sites / 1 site / N sites without discovery", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { ComputerSection } = await import("../src/components/settings/Computer");
	const { wire } = await import("../src/wire");
	const calls: { cmd: string; params: unknown }[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		calls.push({ cmd, params });
		if (cmd === "computer.runtimes") return [];
		if (cmd === "computer.releases") return { floor: "", repository: "", releases: [] };
		return {};
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	let opened = 0;
	try {
		for (const [settings, words] of [
			[{ enabled: true }, "All sites"],
			[{ enabled: false, sites: ["porkbun.com"] }, "1 site"],
			[{ enabled: true, sites: ["porkbun.com", "google.com"] }, "2 sites"],
		] satisfies [OfferSettings, string][]) {
			await act(async () => root.render(<ComputerSection runtime={null} image={null} onRuntime={() => {}} onImage={() => {}} cookieOffers={settings} onSites={() => { opened++; }} />));
			const toggle = container.querySelector<HTMLInputElement>('[role="switch"]')!;
			expect(toggle.checked).toBe(settings.enabled);
			expect(container.textContent).toContain("Offer your browser cookies");
			const row = Array.from(container.querySelectorAll("button")).find((button) => button.textContent === `Sites: ${words}`)!;
			expect(row).toBeDefined();
			await act(async () => toggle.click());
			expect(calls.at(-1)).toEqual({ cmd: "settings.update", params: { patch: { cookieOffers: { ...settings, enabled: !settings.enabled } } } });
			await act(async () => row.click());
		}
		expect(opened).toBe(3);
		expect(calls.every((call) => ["computer.runtimes", "computer.releases", "settings.update"].includes(call.cmd))).toBe(true);
	} finally {
		await act(async () => root.unmount());
		command.mockRestore();
		await teardown();
	}
});

const inventory = {
	sites: [
		{ site: "google.com", sources: [{ browserName: "Brave", profileName: "Work" }, { browserName: "Firefox", profileName: "Default" }] },
		{ site: "porkbun.com", sources: [{ browserName: "Firefox", profileName: "Default" }] },
	],
	unavailable: ["Could not read cookie sites from Chrome · Personal."],
};

async function pageTest(settings: OfferSettings, work: (view: {
	container: HTMLDivElement;
	click(label: string): Promise<void>;
	search(text: string): Promise<void>;
	calls: { cmd: string; params: unknown }[];
	backs(): number;
}) => Promise<void>) {
	const { dom, teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { CookieSitesPage } = await import("../src/components/settings/CookieOfferSettings");
	const { wire } = await import("../src/wire");
	const calls: { cmd: string; params: unknown }[] = [];
	let backs = 0;
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		calls.push({ cmd, params });
		if (cmd === "computer.cookies.sites") return inventory;
		if (cmd === "computer.cookies.blocked") return [];
		return {};
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => root.render(<CookieSitesPage settings={settings} onBack={() => { backs++; }} />));
		await work({
			container, calls, backs: () => backs,
			click: async (label) => {
				const control = container.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`) ?? Array.from(container.querySelectorAll("button")).find((button) => button.textContent === label || button.getAttribute("aria-label") === label)!;
				await act(async () => control.click());
			},
			search: async (text) => {
				const input = container.querySelector<HTMLInputElement>('[aria-label="Search sites"]')!;
				await act(async () => {
					Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, "value")!.set!.call(input, text);
					input.dispatchEvent(new dom.Event("input", { bubbles: true }) as unknown as Event);
				});
			},
		});
	} finally {
		await act(async () => root.unmount());
		command.mockRestore();
		await teardown();
	}
}

test("cookie sites page searches browser profiles, saves selected sites, and explains whole accounts", async () => {
	await pageTest({ enabled: false }, async ({ container, calls, click, search, backs }) => {
		expect(calls.map((call) => call.cmd)).toEqual(["computer.cookies.sites", "computer.cookies.blocked"]);
		expect(container.textContent).toContain("Brave · Work; Firefox · Default");
		expect(container.textContent).toContain("google.com, apple.com and microsoft.com lend the whole account");
		expect(container.textContent).toContain(inventory.unavailable[0]!);
		await search("work");
		expect(container.querySelector('[aria-label="google.com"]')).not.toBeNull();
		expect(container.querySelector('[aria-label="porkbun.com"]')).toBeNull();
		await click("google.com");
		await click("Save");
		expect(calls.at(-1)).toEqual({ cmd: "settings.update", params: { patch: { cookieOffers: { enabled: false, sites: ["porkbun.com"] } } } });
		expect(backs()).toBe(1);
	});
});

test("ticking every cookie site saves no list, even when search shows only one", async () => {
	await pageTest({ enabled: true, sites: ["porkbun.com"] }, async ({ calls, click, search }) => {
		await search("google");
		await click("google.com");
		await click("Save");
		expect(calls.at(-1)).toEqual({ cmd: "settings.update", params: { patch: { cookieOffers: { enabled: true } } } });
	});
});

test("Back discards cookie site edits and saved sites missing from discovery remain editable", async () => {
	await pageTest({ enabled: true, sites: ["missing.com"] }, async ({ container, calls, click, backs }) => {
		expect(container.textContent).toContain("Not found in browsers on this desk");
		expect(container.querySelector<HTMLInputElement>('[aria-label="missing.com"]')!.checked).toBe(true);
		await click("missing.com");
		await click("Back to the team");
		expect(backs()).toBe(1);
		expect(calls.some((call) => call.cmd === "settings.update")).toBe(false);
	});
});
