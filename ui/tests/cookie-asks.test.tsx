import { expect, spyOn, test } from "bun:test";
import type { TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

type Ask = Extract<TranscriptEvent, { kind: "cookie_ask" }>;
const pending: Ask = {
	kind: "cookie_ask",
	id: "cookie-ask:1",
	ts: 1,
	site: "github.com",
	origin: "https://github.com",
	reason: "To open your PR #203 and read why the Linux check failed.",
	status: "pending",
	sources: [{ browserName: "Brave", profileName: "Default" }],
	name: "GITHUB_COM",
	expiresAt: Date.now() + 7_200_000,
};

/** Every card the file renders is its own: answers in flight are kept by card across the file. */
let key = 0;

async function rig() {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { CookieAskCard } = await import("../src/components/transcript/CookieAskCard");
	const { wire } = await import("../src/wire");
	const calls: { cmd: string; params: unknown }[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		calls.push({ cmd, params });
		return null;
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	let screens = 0;
	// Each render is a card of its own, as a new ask is.
	const render = async (event: Ask) => {
		key += 1;
		const id = `${event.id}:${key}`;
		last = id;
		await act(async () => root.render(<CookieAskCard key={id} event={{ ...event, id }} personaId="kit" teammate="Kit" onOpenScreen={() => screens++} />));
		// A card whose time is up folds on its timer's first tick.
		await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
	};
	const buttons = () => Array.from(container.querySelectorAll("button")).map((button) => button.textContent ?? "");
	const click = async (label: string) =>
		act(async () => Array.from(container.querySelectorAll("button")).find((button) => button.textContent === label || button.getAttribute("aria-label") === label)!.click());
	let last = "";
	const done = async () => {
		await act(async () => root.unmount());
		command.mockRestore();
		await teardown();
	};
	return { container, calls, render, buttons, click, screens: () => screens, done, act, id: () => last };
}

test("The ask is a lock, the site and the teammate's reason, with Use Cookies, Sign In and Open Browser, and a close that is Not now", async () => {
	const view = await rig();
	try {
		await view.render(pending);
		expect(view.container.querySelector(".sign-in-lock")).not.toBeNull();
		expect(view.container.querySelector(".sign-in-site")?.textContent).toBe("github.com");
		expect(view.container.textContent).toContain(pending.reason);
		expect(view.buttons().filter((label) => label !== "")).toEqual(["Use Cookies", "Sign In", "Open Browser"]);
		expect(view.container.querySelector(".btn-primary")?.textContent).toBe("Use Cookies");
		expect(view.container.textContent).toContain("From Brave");
		const close = view.container.querySelector<HTMLButtonElement>('button[aria-label="Not now"]')!;
		expect(close.title).toBe("Not now");
		expect(view.container.textContent).not.toMatch(/Not now|lend|lent|existing cookie|signed in as you|Give it a login|Sign in myself/i);

		const used = view.id();
		await view.click("Use Cookies");
		await view.render(pending);
		await view.click("Not now");
		expect(view.calls).toEqual([
			{ cmd: "computer.cookies.answer", params: { personaId: "kit", id: used, answer: "use", source: { browserName: "Brave", profileName: "Default" } } },
			{ cmd: "computer.cookies.answer", params: { personaId: "kit", id: view.id(), answer: "not_now" } },
		]);
	} finally {
		await view.done();
	}
});

test("With several browsers signed in, a chooser picks the one, the last used first", async () => {
	const view = await rig();
	try {
		await view.render({ ...pending, sources: [{ browserName: "Brave", profileName: "Default" }, { browserName: "Chrome", profileName: "Work" }] });
		const chooser = view.container.querySelector('[aria-label="Cookies from"]')!;
		expect(Array.from(chooser.querySelectorAll("button")).map((button) => [button.textContent, button.getAttribute("aria-pressed")])).toEqual([
			["Brave", "true"],
			["Chrome · Work", "false"],
		]);
		await view.click("Chrome · Work");
		await view.click("Use Cookies");
		expect(view.calls).toEqual([{ cmd: "computer.cookies.answer", params: { personaId: "kit", id: view.id(), answer: "use", source: { browserName: "Chrome", profileName: "Work" } } }]);
	} finally {
		await view.done();
	}
});

test("With no browser signed in to the site there is no Use Cookies, and Sign In leads", async () => {
	const view = await rig();
	try {
		await view.render({ ...pending, sources: [] });
		expect(view.buttons().filter((label) => label !== "")).toEqual(["Sign In", "Open Browser"]);
		expect(view.container.querySelector(".btn-primary")?.textContent).toBe("Sign In");
		expect(view.container.textContent).not.toContain("From");
	} finally {
		await view.done();
	}
});

test("Open Browser answers, then opens the teammate's computer screen", async () => {
	const view = await rig();
	try {
		await view.render(pending);
		await view.click("Open Browser");
		expect(view.calls).toEqual([{ cmd: "computer.cookies.answer", params: { personaId: "kit", id: view.id(), answer: "open_browser" } }]);
		expect(view.screens()).toBe(1);
	} finally {
		await view.done();
	}
});

test("Sign In opens the secure fields in this same card, saves to Secrets by default, and clears what was typed", async () => {
	const view = await rig();
	const field = (name: string) => view.container.querySelector<HTMLInputElement>(`[name="${name}"]`);
	const submit = async () => view.act(async () => { view.container.querySelector("form")!.dispatchEvent(new window.Event("submit", { bubbles: true, cancelable: true })); });
	try {
		await view.render(pending);
		expect(field("password")).toBeNull();
		await view.click("Sign In");
		expect(view.container.querySelectorAll("form").length).toBe(1);
		expect(view.container.textContent).toContain(pending.reason);
		expect(field("save")!.checked).toBe(true);
		expect(field("name")!.value).toBe("GITHUB_COM");
		field("username")!.value = "private-user";
		field("password")!.value = "private-password";
		await submit();
		expect(view.calls.at(-1)).toEqual({
			cmd: "secrets.capture.answer",
			params: { actionId: view.id(), name: "GITHUB_COM", values: { username: "private-user", password: "private-password" }, ask: false, save: true },
		});
		expect(field("username")!.value).toBe("");
		expect(field("password")!.value).toBe("");

		await view.render(pending);
		await view.click("Sign In");
		await view.act(async () => field("save")!.click());
		expect(field("name")).toBeNull();
		field("username")!.value = "private-user";
		field("password")!.value = "private-password";
		await submit();
		expect(view.calls.at(-1)).toEqual({
			cmd: "secrets.capture.answer",
			params: { actionId: view.id(), name: "GITHUB_COM", values: { username: "private-user", password: "private-password" }, ask: false, save: false },
		});
	} finally {
		await view.done();
	}
});

test("Settled, the ask folds to one quiet line that says how", async () => {
	const view = await rig();
	try {
		for (const [settled, line] of [
			[{ status: "cookies", browserName: "Brave", profileName: "Default" }, "Cookies from Brave"],
			[{ status: "signed_in", storedAs: "GITHUB_COM" }, "Signed in"],
			[{ status: "opened" }, "Browser opened"],
			[{ status: "not_now" }, "Not now"],
			[{ status: "expired" }, "Expired"],
		] satisfies [Partial<Ask>, string][]) {
			await view.render({ ...pending, ...settled });
			expect(view.container.querySelector(".ask-settled .ask-settled-what")?.textContent).toBe("github.com");
			expect(view.container.querySelector(".ask-settled .ask-chip")?.textContent).toBe(line);
			expect(view.container.querySelectorAll("input, button").length).toBe(0);
			expect(view.container.textContent).not.toContain(pending.reason);
		}
		await view.render({ ...pending, expiresAt: Date.now() - 1 });
		expect(view.container.querySelector(".ask-settled .ask-chip")?.textContent).toBe("Expired");
	} finally {
		await view.done();
	}
});

test("Settings › Computer has no offers switch, Sites list or Never, and reads no browser", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { ComputerSection } = await import("../src/components/settings/Computer");
	const { wire } = await import("../src/wire");
	const calls: string[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string) => {
		calls.push(cmd);
		if (cmd === "computer.runtimes") return [];
		if (cmd === "computer.releases") return { floor: "", repository: "", releases: [] };
		return {};
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => root.render(<ComputerSection runtime={null} image={null} onRuntime={() => {}} onImage={() => {}} />));
		expect(container.textContent).not.toMatch(/Offer your browser cookies|sign-in offers|Sites:|Never for this site/);
		expect(calls.some((cmd) => cmd.startsWith("computer.cookies"))).toBe(false);
	} finally {
		await act(async () => root.unmount());
		command.mockRestore();
		await teardown();
	}
});
