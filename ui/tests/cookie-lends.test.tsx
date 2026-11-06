import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { CookieLend, CookieOfferSites } from "../src/generated/contract";
import { installDom } from "./dom";

const DAY = 24 * 60 * 60 * 1000;

let asked: { cmd: string; params: Record<string, unknown> }[];
let lends: CookieLend[];
let lendable: CookieOfferSites;
/** The desk is a server, which shares no cookies. */
let served: boolean;
let root: Root;
let container: HTMLDivElement;
let act: typeof import("react").act;
let teardown: () => Promise<void>;
let restores: (() => void)[];
let desks = 0;

const github: CookieLend = { id: "gh", teammate: "kit", site: "github.com", browserName: "Brave", profileName: "George", lentAt: 1 };

beforeEach(async () => {
	({ teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } }));
	act = (await import("react")).act;
	const { Wire, wire } = await import("../src/wire");
	const { replaceDesks } = await import("../src/desks");
	// A desk of its own for each test, so each starts from its own room.
	replaceDesks([{ id: `cookie-lends-${++desks}`, kind: "local", name: "Test", origin: "ws://localhost", token: "test" }]);
	asked = [];
	served = false;
	lends = [github];
	lendable = {
		sites: [
			{ site: "github.com", sources: [{ browserName: "Brave", profileName: "George" }] },
			{ site: "linear.app", sources: [{ browserName: "Brave", profileName: "George" }, { browserName: "Chrome", profileName: "Work" }, { browserName: "Brave", profileName: "Work" }] },
		],
		unavailable: [],
	};
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		asked.push({ cmd, params });
		switch (cmd) {
			case "computer.cookies.lends":
				return lends;
			case "computer.cookies.lendable":
				return lendable;
			case "computer.cookies.lend": {
				const lend = { id: "ln", teammate: "kit", site: String(params["site"]), browserName: "Brave", profileName: "George", lentAt: 2 };
				lends = [...lends, lend];
				return lend;
			}
			case "computer.cookies.lend.set": {
				const was = lends.find((one) => one.id === params["id"])!;
				const next: CookieLend = { ...was };
				if (params["keepFresh"] !== undefined) next.keepFresh = params["keepFresh"] === true;
				if (params["ends"] === "day") next.endsAt = Date.now() + DAY;
				if (params["ends"] === "never") delete next.endsAt;
				lends = lends.map((one) => (one.id === next.id ? next : one));
				return next;
			}
			case "computer.cookies.remove":
				lends = lends.filter((one) => one.id !== params["id"]);
				return null;
			case "computer.cookies.holders":
				return [{ teammate: "kit", name: "Kit", sites: ["github.com", "linear.app"] }];
			case "computer.cookies.sources":
				if (served) return { sources: [], blocked: [], off: true };
				return { sources: [{ browserId: "brave", browserName: "Brave", profileId: "Default", profileName: "George", on: true }], blocked: [] };
			case "computer.cookies.source.set":
				return { sources: [{ browserId: "brave", browserName: "Brave", profileId: "Default", profileName: "George", on: params["on"] === true }], blocked: [] };
			case "secrets.list":
				return [];
			default:
				throw new Error(`Unexpected command ${cmd}`);
		}
	}) as typeof wire.command);
	const subscribe = spyOn(Wire.prototype, "subscribe").mockImplementation((() => () => {}) as InstanceType<typeof Wire>["subscribe"]);
	restores = [() => command.mockRestore(), () => subscribe.mockRestore(), () => replaceDesks([])];
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	for (const restore of restores) restore();
	await teardown();
});

const text = () => container.textContent ?? "";
const button = (words: string) => Array.from(container.querySelectorAll("button")).find((one) => one.textContent?.includes(words))!;
const commands = () => asked.map((one) => one.cmd);

/** A teammate with a computer, or with the Mac app's desk browser instead. */
async function mountPane(computer: boolean) {
	if (!computer) {
		const shell = window as unknown as { __hotlineDesk?: { platform: string } };
		shell.__hotlineDesk = { platform: "macos" };
		restores.push(() => delete shell.__hotlineDesk);
	}
	const { SecretRows } = await import("../src/components/teammate/SecretRows");
	await act(async () => root.render(<SecretRows personaId="kit" teammate="Kit" granted={[]} computer={computer} disabled={false} onChange={() => {}} />));
}

test("Cookies sits under the teammate's secrets, with Add Cookies at its head, and the pane reads no browser", async () => {
	await mountPane(true);
	expect(text()).toContain("Cookies");
	expect(text()).toContain("Add Cookies");
	expect(text()).toContain("github.com");
	expect(text()).toContain("From Brave");
	expect(commands()).not.toContain("computer.cookies.lendable");
	for (const word of ["lend", "Lend", "lent", "Lent"]) expect(text()).not.toContain(word);
});

test("Add Cookies reads the sites only when opened, and a press shares one that lands as a row", async () => {
	await mountPane(false);
	await act(async () => button("Add Cookies").click());
	expect(commands().filter((cmd) => cmd === "computer.cookies.lendable")).toHaveLength(1);
	const search = container.querySelector<HTMLInputElement>("input[placeholder='Search sites']");
	expect(search).not.toBeNull();
	expect(text()).toContain("Brave, Chrome");
	expect(text()).toContain("Shared");
	const linear = Array.from(container.querySelectorAll("button")).find((one) => one.textContent?.startsWith("linear.app"))!;
	await act(async () => linear.click());
	expect(asked.find((one) => one.cmd === "computer.cookies.lend")?.params).toEqual({ personaId: "kit", site: "linear.app" });
	const rows = Array.from(container.querySelectorAll("button[aria-expanded]")).map((one) => one.textContent);
	expect(rows.some((row) => row?.startsWith("linear.appFrom Brave"))).toBe(true);
});

test("a row opens to Keep fresh, Ends and Remove, and its title names only what is not the default", async () => {
	await mountPane(false);
	const row = () => Array.from(container.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")).find((one) => one.textContent?.startsWith("github.com"))!;
	expect(text()).not.toContain("Remove");
	await act(async () => row().click());
	expect(text()).toContain("Keep fresh");
	expect(text()).toContain("Ends");
	expect(text()).toContain("1 day");
	expect(text()).toContain("Never");
	expect(button("Never").getAttribute("aria-pressed")).toBe("true");
	expect(row().textContent).toBe("github.comFrom Brave");

	const fresh = Array.from(container.querySelectorAll<HTMLInputElement>("input.switch"))[0]!;
	await act(async () => fresh.click());
	expect(asked.at(-1)).toEqual({ cmd: "computer.cookies.lend.set", params: { personaId: "kit", id: "gh", keepFresh: true } });
	expect(row().textContent).toContain("github.com · Keep fresh");

	await act(async () => button("1 day").click());
	expect(asked.at(-1)?.params).toEqual({ personaId: "kit", id: "gh", ends: "day" });
	expect(row().textContent).toContain("Keep fresh · Ends tomorrow");

	await act(async () => button("Remove").click());
	expect(asked.at(-1)).toEqual({ cmd: "computer.cookies.remove", params: { personaId: "kit", id: "gh" } });
	expect(text()).not.toContain("github.com");
});

test("Settings › Secrets has Cookie Sources, a switch each, and In Use, which opens the holder's pane", async () => {
	const { SecretsSection } = await import("../src/components/Secrets");
	const opened: string[] = [];
	await act(async () => root.render(<SecretsSection team={[{ id: "kit" }]} onOpenTeammate={(id) => opened.push(id)} />));
	expect(text()).toContain("Cookie Sources");
	expect(text()).toContain("Brave · George");
	const toggle = container.querySelector<HTMLInputElement>("input[aria-label='Brave · George']")!;
	expect(toggle.checked).toBe(true);
	await act(async () => toggle.click());
	expect(asked.at(-1)).toEqual({ cmd: "computer.cookies.source.set", params: { browserId: "brave", profileId: "Default", on: false } });
	expect(container.querySelector<HTMLInputElement>("input[aria-label='Brave · George']")!.checked).toBe(false);
	expect(text()).toContain("In Use");
	expect(text()).toContain("Kit · github.com, linear.app");
	await act(async () => button("Kit").click());
	expect(opened).toEqual(["kit"]);
	expect(commands()).not.toContain("computer.cookies.lendable");
});

test("A server's Settings › Secrets has no Cookie Sources, since it shares no cookies", async () => {
	served = true;
	const { SecretsSection } = await import("../src/components/Secrets");
	await act(async () => root.render(<SecretsSection team={[{ id: "kit" }]} />));
	expect(commands()).toContain("computer.cookies.sources");
	expect(text()).not.toContain("Cookie Sources");
	expect(text()).not.toContain("Looking for browsers");
});

test("an end reads as today, tomorrow or its date", async () => {
	const { endsWords } = await import("../src/components/teammate/Cookies");
	const noon = new Date(2026, 10, 5, 12).getTime();
	expect(endsWords(noon + 60 * 60 * 1000, noon)).toBe("Ends today");
	expect(endsWords(noon + DAY, noon)).toBe("Ends tomorrow");
	expect(endsWords(noon + 3 * DAY, noon)).toMatch(/^Ends Nov 8$/);
});
