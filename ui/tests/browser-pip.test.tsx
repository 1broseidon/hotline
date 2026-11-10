import { expect, spyOn, test } from "bun:test";
import type { BrowserView } from "../src/generated/contract";
import { installDom } from "./dom";

test("The PIP stays up and marks it while the browser needs the person, and leaves when it is long idle", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { BrowserPip } = await import("../src/components/BrowserPip");
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const idle: BrowserView = { phase: "teammate", url: "https://expired.badssl.com/", title: "", activeAt: Date.now() - 600_000 };
	try {
		await act(async () => root.render(<BrowserPip personaId="quill" name="Quill" view={idle} />));
		expect(container.querySelector(".browser-pip")).toBeNull();

		await act(async () => root.render(<BrowserPip personaId="quill" name="Quill" view={{ ...idle, needs: "Certificate expired" }} />));
		const pip = container.querySelector(".browser-pip");
		expect(pip?.classList.contains("needs")).toBe(true);
		expect(container.querySelector(".browser-pip-needs")?.textContent).toBe("Certificate expired");
		expect(container.textContent).toContain("Quill needs you");
		expect(container.textContent).toContain("expired.badssl.com");
		expect(container.querySelector("button")?.getAttribute("aria-label")).toBe("Open Quill's browser: Certificate expired");
	} finally {
		await act(async () => root.unmount());
		container.remove();
		teardown();
	}
});

test("The PIP takes the page's shape and names its size, and Hide sits on its status line", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { BrowserPip, sizeName } = await import("../src/components/BrowserPip");
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const busy: BrowserView = { phase: "teammate", url: "https://example.com/", title: "", activeAt: Date.now() };
	try {
		await act(async () => root.render(<BrowserPip personaId="quill" name="Quill" view={busy} />));
		expect(container.querySelector(".browser-pip-size")).toBeNull();
		expect(container.querySelector(".browser-pip-stage")?.classList.contains("upright")).toBe(false);

		await act(async () => root.render(<BrowserPip personaId="quill" name="Quill" view={{ ...busy, viewport: { width: 390, height: 844 } }} />));
		expect(container.querySelector(".browser-pip-size")?.textContent).toBe("Phone");
		expect(container.querySelector(".browser-pip-stage")?.classList.contains("upright")).toBe(true);
		expect(container.querySelector<HTMLElement>(".browser-pip-page")?.style.aspectRatio).toBe("390 / 844");

		const hide = container.querySelector<HTMLButtonElement>(".browser-pip-hide");
		expect(hide?.getAttribute("aria-label")).toBe("Hide");
		await act(async () => hide!.click());
		expect(container.querySelector(".browser-pip")).toBeNull();
	} finally {
		await act(async () => root.unmount());
		container.remove();
		teardown();
	}
	expect([sizeName({ width: 820, height: 1180 }), sizeName({ width: 1920, height: 1080 }), sizeName({ width: 1024, height: 768 }), sizeName(undefined)]).toEqual([
		"Tablet",
		"Wide",
		"1024 × 768",
		null,
	]);
});

test("The PIP sets no timer while there is no browser, and one that ends when the teammate's last move is 45 s old", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { BrowserPip } = await import("../src/components/BrowserPip");
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	// The clock is the test's, and every timer set from the PIP's own code is noted as it is set.
	let clock = 1_000_000;
	spyOn(Date, "now").mockImplementation(() => clock);
	const set: { kind: "once" | "repeating"; after: number; run(): void }[] = [];
	const fromThePip = () => (new Error().stack ?? "").includes("BrowserPip");
	const realTimeout = globalThis.setTimeout;
	const realInterval = globalThis.setInterval;
	spyOn(globalThis, "setTimeout").mockImplementation(((run: () => void, after?: number, ...rest: unknown[]) => {
		if (fromThePip()) set.push({ kind: "once", after: after ?? 0, run });
		return realTimeout(run, after, ...rest);
	}) as typeof setTimeout);
	spyOn(globalThis, "setInterval").mockImplementation(((run: () => void, after?: number, ...rest: unknown[]) => {
		if (fromThePip()) set.push({ kind: "repeating", after: after ?? 0, run });
		return realInterval(run, after, ...rest);
	}) as typeof setInterval);
	const draw = (view: BrowserView | undefined) => act(async () => root.render(<BrowserPip personaId="quill" name="Quill" view={view} />));
	const shown = () => container.querySelector(".browser-pip") !== null;
	try {
		// No browser: the window is not woken for one.
		await draw(undefined);
		expect(set).toEqual([]);

		// A browser long idle when it is first heard of: nothing to wait for.
		await draw({ phase: "teammate", url: "https://example.com/", title: "", activeAt: clock - 600_000 });
		expect(shown()).toBe(false);
		expect(set).toEqual([]);

		// The teammate moved ten seconds ago: up, with one timer for the 35 s left, however often the roster says so again.
		const moved: BrowserView = { phase: "teammate", url: "https://example.com/", title: "", activeAt: clock - 10_000 };
		await draw(moved);
		await draw({ ...moved });
		expect(shown()).toBe(true);
		expect(set.map(({ kind, after }) => ({ kind, after }))).toEqual([{ kind: "once", after: 35_000 }]);

		// The moment comes: the PIP leaves, and nothing is set to ask again.
		clock += 35_000;
		await act(async () => set[0]!.run());
		expect(shown()).toBe(false);
		expect(set.length).toBe(1);

		// The teammate moves again: up again, for 45 s from that move.
		await draw({ ...moved, activeAt: clock });
		expect(shown()).toBe(true);
		expect(set.map(({ kind, after }) => ({ kind, after }))).toEqual([
			{ kind: "once", after: 35_000 },
			{ kind: "once", after: 45_000 },
		]);
	} finally {
		await act(async () => root.unmount());
		container.remove();
		teardown();
	}
});
