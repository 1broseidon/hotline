import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { ReactNode } from "react";
import type { Root } from "react-dom/client";
import type { Window } from "happy-dom";
import type { Desk } from "../src/desks";
import type { BrowserSnapshot, BrowserView, TranscriptEvent } from "../src/generated/contract";
import type { RosterEntry } from "../src/wire";
import { installDom } from "./dom";

/*
 * A teammate's desk browser on another desk, opened from this window: the PIP
 * and a cookie ask's Open Browser bring up the window's viewer rather than the
 * native pane, and the viewer drives the browser over the wire as far as the
 * desk's hello lets this seat.
 */

type Call = { cmd: string; params: Record<string, unknown> };
type Ask = Extract<TranscriptEvent, { kind: "cookie_ask" }>;

const remote: Desk = { id: "grizzly", name: "Grizzly", kind: "remote", origin: "http://127.0.0.1:1", token: "t", state: "open" };
const local: Desk = { id: "local", name: "This computer", kind: "local", origin: "http://127.0.0.1:2", token: "t" };
const browsing: BrowserView = { phase: "teammate", url: "https://example.com/", title: "Example", activeAt: Date.now(), snapshotAt: 1 };

let dom: Window;
let teardown: () => Promise<void>;
let root: Root;
let container: HTMLDivElement;
let act: typeof import("react").act;
let modules: {
	BrowserPip: typeof import("../src/components/BrowserPip").BrowserPip;
	RemoteBrowser: typeof import("../src/components/RemoteBrowser").RemoteBrowser;
	CookieAskCard: typeof import("../src/components/transcript/CookieAskCard").CookieAskCard;
	desks: typeof import("../src/desks");
};
/** What the desk was asked, and what the shell was. */
let calls: Call[];
let shell: Call[];
/** What the desk's hello granted this seat. */
let granted: string[];
/** The browser as the teammate's session last said it. */
let shown: BrowserView;
let taken = 10;

beforeEach(async () => {
	({ dom, teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } }));
	({ act } = await import("react"));
	const { createRoot } = await import("react-dom/client");
	modules = {
		BrowserPip: (await import("../src/components/BrowserPip")).BrowserPip,
		RemoteBrowser: (await import("../src/components/RemoteBrowser")).RemoteBrowser,
		CookieAskCard: (await import("../src/components/transcript/CookieAskCard")).CookieAskCard,
		desks: await import("../src/desks"),
	};
	calls = [];
	shell = [];
	granted = [];
	// The window runs in the shell: the pane is a command away, on whichever desk.
	Object.assign(dom, {
		__hotlineDesk: { platform: "macos", origin: "", token: "" },
		__TAURI_INTERNALS__: {
			invoke: async (cmd: string, args: Record<string, unknown>) => {
				shell.push({ cmd, params: args });
				return null;
			},
		},
	});
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	modules.desks.replaceDesks([]);
	await teardown();
});

/** Every picture is newer than the last, as the desk's are. */
function picture(): BrowserSnapshot {
	taken += 1;
	return { jpeg: "AAAA", at: taken, url: "https://example.com/", title: "Example", size: { width: 1280, height: 800 } };
}

/** The window on `desk`, whose core answers the browser's commands as a desk does. */
function onDesk(desk: Desk) {
	modules.desks.replaceDesks([desk]);
	const line = modules.desks.wireFor(desk.id);
	spyOn(line, "can").mockImplementation((name: string) => granted.includes(name));
	spyOn(line, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push({ cmd, params });
		if (cmd === "browser.take_over") return { ...shown, phase: "person" };
		if (cmd === "browser.hand_back") return { ...shown, phase: "teammate" };
		if (cmd.startsWith("browser.")) return picture();
		return null;
	}) as typeof line.command);
}

const settle = () => act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
const wait = (ms: number) => act(async () => new Promise((resolve) => setTimeout(resolve, ms)));
const viewer = () => document.body.querySelector<HTMLElement>(".remote-browser");
const pageOf = () => document.body.querySelector<HTMLElement>(".remote-browser-page")!;
const button = (label: string) =>
	Array.from(document.body.querySelectorAll("button")).find((one) => one.textContent === label || one.getAttribute("aria-label") === label) ?? null;
const press = (label: string) => act(async () => button(label)!.click());
const sent = (cmd: string) => calls.filter((call) => call.cmd === cmd).map((call) => call.params);
const inputs = () => sent("browser.input").map((params) => params["input"]);

/** The teammate's conversation as far as the browser goes: its PIP, and the window's one viewer. */
async function render(view: BrowserView, extra?: ReactNode) {
	shown = view;
	const entry = { persona: { id: "quill", name: "Quill" }, session: { browser: view } } as unknown as RosterEntry;
	await act(async () =>
		root.render(
			<>
				<modules.BrowserPip personaId="quill" name="Quill" view={view} />
				{extra}
				<modules.RemoteBrowser roster={[entry]} />
			</>,
		),
	);
	await settle();
}

/** Opens the viewer from the PIP. */
async function open(view: BrowserView = browsing) {
	await render(view);
	await act(async () => container.querySelector<HTMLButtonElement>(".browser-pip button")!.click());
	await settle();
}

/** Opens the viewer and takes control. */
async function holding(view: BrowserView = browsing) {
	await open(view);
	await press("Take control");
	await settle();
	// The picture answers a box 800 by 400 on the screen.
	pageOf().getBoundingClientRect = () => ({ left: 0, top: 0, width: 800, height: 400, right: 800, bottom: 400, x: 0, y: 0, toJSON() {} }) as DOMRect;
}

const key = (target: Element, name: string, extra: Record<string, unknown> = {}) =>
	target.dispatchEvent(new dom.KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true, ...extra }) as unknown as Event);

test("On a remote desk the PIP opens the window's viewer of the browser, never this computer's pane", async () => {
	onDesk(remote);
	granted = ["browserDrive"];
	await open();
	expect(viewer()?.getAttribute("aria-label")).toBe("Quill's browser");
	expect(viewer()?.textContent).toContain("Quill is driving");
	expect(document.body.querySelector<HTMLInputElement>('.remote-browser input[aria-label="Address"]')?.value).toBe("https://example.com/");
	expect(document.body.querySelector(".remote-browser .lock")?.classList.contains("secure")).toBe(true);
	expect(pageOf().querySelector("img")?.getAttribute("src")).toBe("data:image/jpeg;base64,AAAA");
	// While the teammate drives, the picture is its last move's, never a fresh one.
	expect(sent("browser.snapshot").every((params) => params["fresh"] === undefined)).toBe(true);
	expect(shell).toEqual([]);
});

test("On this computer's desk the PIP opens the pane, as it always has", async () => {
	onDesk(local);
	await open();
	expect(shell).toEqual([{ cmd: "browser_pane_open", params: { personaId: "quill", name: "Quill" } }]);
	expect(viewer()).toBeNull();
});

test("On a remote desk a cookie ask's Open Browser answers, then opens the viewer", async () => {
	onDesk(remote);
	const ask: Ask = {
		kind: "cookie_ask",
		id: "cookie-ask:remote",
		ts: 1,
		site: "github.com",
		origin: "https://github.com",
		reason: "To read the failed check.",
		status: "pending",
		sources: [],
		name: "GITHUB_COM",
		deskBrowser: true,
		expiresAt: Date.now() + 7_200_000,
	};
	await render(browsing, <modules.CookieAskCard event={ask} personaId="quill" teammate="Quill" />);
	await press("Open Browser");
	await settle();
	expect(sent("computer.cookies.answer")).toEqual([{ personaId: "quill", id: ask.id, answer: "open_browser" }]);
	expect(viewer()?.getAttribute("aria-label")).toBe("Quill's browser");
	expect(shell).toEqual([]);
});

test("A seat the desk does not let drive only watches: no Take control, nothing it could not do, and nothing sent", async () => {
	onDesk(remote);
	granted = ["browserVisitAnyway"];
	await open({ ...browsing, needs: "Certificate expired" });
	expect(button("Take control")).toBeNull();
	expect(button("Back")).toBeNull();
	expect(button("Visit anyway")).toBeNull();
	expect(viewer()?.textContent).toContain("Certificate expired");
	expect(button("Close")).not.toBeNull();

	// Someone with control elsewhere: the picture follows, but this seat's keys and clicks go nowhere.
	await act(async () => root.unmount());
	root = (await import("react-dom/client")).createRoot(container);
	await open({ ...browsing, phase: "person" });
	expect(sent("browser.snapshot").some((params) => params["fresh"] === true)).toBe(true);
	await act(async () => {
		key(pageOf(), "a");
		pageOf().click();
	});
	await settle();
	expect(sent("browser.input")).toEqual([]);
	expect(sent("browser.take_over")).toEqual([]);
});

test("Take control and Give it back go to the desk, and only while held is the page pictured afresh", async () => {
	onDesk(remote);
	granted = ["browserDrive"];
	await open();
	expect(button("Back")?.disabled).toBe(true);
	await press("Take control");
	await settle();
	expect(sent("browser.take_over")).toEqual([{ personaId: "quill" }]);
	expect(viewer()?.textContent).toContain("You have control");
	expect(document.body.querySelector(".remote-browser .browser-bar")?.classList.contains("yours")).toBe(true);
	expect(pageOf().classList.contains("yours")).toBe(true);
	expect(button("Back")?.disabled).toBe(false);

	// A poll at a time, again and again.
	await wait(500);
	const fresh = () => sent("browser.snapshot").filter((params) => params["fresh"] === true).length;
	expect(fresh()).toBeGreaterThanOrEqual(2);

	await press("Give it back");
	await settle();
	expect(sent("browser.hand_back")).toEqual([{ personaId: "quill" }]);
	expect(viewer()?.textContent).toContain("Quill is driving");
	const stopped = fresh();
	await wait(500);
	expect(fresh()).toBe(stopped);
});

test("Held, a click lands on the page through its size, typing goes as text and Enter as a key, and the wheel scrolls", async () => {
	onDesk(remote);
	granted = ["browserDrive"];
	await holding();
	// A 1280 × 800 page in the 800 × 400 box is drawn 640 wide from 80 in: the box's middle is the page's.
	await act(async () => pageOf().dispatchEvent(new dom.MouseEvent("click", { bubbles: true, clientX: 400, clientY: 200 }) as unknown as Event));
	await settle();
	expect(inputs()).toEqual([{ kind: "tap", x: 640, y: 400 }]);
	// Beside the picture is not the page.
	await act(async () => pageOf().dispatchEvent(new dom.MouseEvent("click", { bubbles: true, clientX: 20, clientY: 200 }) as unknown as Event));
	await settle();
	expect(inputs()).toHaveLength(1);

	calls = [];
	await act(async () => {
		for (const one of ["h", "i", "!", "Enter"]) key(pageOf(), one);
	});
	await settle();
	await settle();
	expect(inputs()).toEqual([
		{ kind: "text", text: "h" },
		{ kind: "text", text: "i!" },
		{ kind: "key", key: "Enter" },
	]);

	calls = [];
	await act(async () => {
		for (const deltaY of [100, 50, 30]) {
			const wheel = new dom.WheelEvent("wheel", { bubbles: true, deltaY });
			// happy-dom's wheel is no mouse event, so where the pointer is is said here.
			Object.defineProperties(wheel, { clientX: { value: 400 }, clientY: { value: 200 } });
			pageOf().dispatchEvent(wheel as unknown as Event);
		}
	});
	await settle();
	await settle();
	expect(inputs()).toEqual([
		{ kind: "scroll", dx: 0, dy: 100, x: 640, y: 400 },
		{ kind: "scroll", dx: 0, dy: 80, x: 640, y: 400 },
	]);
});

test("Held, ‹ › ↻ step through the page's history and the address goes where it says", async () => {
	onDesk(remote);
	granted = ["browserDrive"];
	await holding();
	await press("Back");
	await press("Forward");
	await press("Reload");
	expect(sent("browser.step")).toEqual([
		{ personaId: "quill", step: "back" },
		{ personaId: "quill", step: "forward" },
		{ personaId: "quill", step: "reload" },
	]);

	const field = document.body.querySelector<HTMLInputElement>('.remote-browser input[aria-label="Address"]')!;
	expect(field.readOnly).toBe(false);
	await act(async () => {
		Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, "value")!.set!.call(field, " example.org ");
		field.dispatchEvent(new dom.Event("input", { bubbles: true }) as unknown as Event);
	});
	await act(async () => field.form!.dispatchEvent(new dom.Event("submit", { bubbles: true, cancelable: true }) as unknown as Event));
	await settle();
	expect(sent("browser.go")).toEqual([{ personaId: "quill", url: "example.org" }]);
});

test("Visit anyway is there only where the desk takes it, and lets the certificate through only with control", async () => {
	onDesk(remote);
	granted = ["browserDrive"];
	await open({ ...browsing, needs: "Certificate expired" });
	expect(viewer()?.textContent).toContain("Certificate expired");
	expect(button("Visit anyway")).toBeNull();

	await act(async () => root.unmount());
	root = (await import("react-dom/client")).createRoot(container);
	granted = ["browserDrive", "browserVisitAnyway"];
	await open({ ...browsing, needs: "Certificate expired" });
	const visit = button("Visit anyway") as HTMLButtonElement;
	expect(visit.disabled).toBe(true);
	expect(visit.title).toBe("Take control first");
	await act(async () => visit.click());
	expect(sent("browser.visit_anyway")).toEqual([]);

	await press("Take control");
	await settle();
	expect((button("Visit anyway") as HTMLButtonElement).disabled).toBe(false);
	await press("Visit anyway");
	await settle();
	expect(sent("browser.visit_anyway")).toEqual([{ personaId: "quill" }]);
});

test("Escape on the picture is the page's while held, and closing hands control back and stops the pictures", async () => {
	onDesk(remote);
	granted = ["browserDrive"];
	await holding();
	pageOf().focus();
	calls = [];
	await act(async () => key(pageOf(), "Escape"));
	await settle();
	expect(inputs()).toEqual([{ kind: "key", key: "Escape" }]);
	expect(viewer()).not.toBeNull();

	await press("Close");
	await settle();
	expect(viewer()).toBeNull();
	expect(sent("browser.hand_back")).toEqual([{ personaId: "quill" }]);
	const polls = sent("browser.snapshot").length;
	await wait(500);
	expect(sent("browser.snapshot")).toHaveLength(polls);
});

test("Watching, Escape closes the viewer and hands nothing back", async () => {
	onDesk(remote);
	granted = ["browserDrive"];
	await open();
	await act(async () => key(pageOf(), "Escape"));
	expect(viewer()).toBeNull();
	expect(sent("browser.hand_back")).toEqual([]);
	expect(sent("browser.input")).toEqual([]);
});
