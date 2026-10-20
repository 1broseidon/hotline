import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { UpdateStatus } from "../src/native";
import { installDom } from "./dom";

// Loading the window's modules reads the platform, and the theme, before any test has a DOM.
Object.assign(globalThis, {
	window: { matchMedia: () => ({ matches: false, addEventListener() {} }) },
	localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
});
const { wire } = await import("../src/wire");
const { replaceDesks } = await import("../src/desks");
const native = await import("../src/native");
const { ErrorCard } = await import("../src/components/ErrorCard");
const { SecretsSection } = await import("../src/components/Secrets");
const { SkillsSection } = await import("../src/components/Skills");
const { UpdatesSection } = await import("../src/components/UpdatesSection");
Reflect.deleteProperty(globalThis, "localStorage");

// The settings pages scroll; nothing here lays out.
class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

let root: Root | null;
let container: HTMLDivElement;
let act: typeof import("react").act;
let restores: (() => void)[];
let teardown: () => Promise<void>;

beforeEach(async () => {
	restores = [];
	({ teardown } = installDom({ ResizeObserver: NoResize }));
	// A desk with no endpoint: the room's wire never opens, so a page reads no settings.
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	// React reads the DOM when it loads, so it loads once the DOM is here (as search.test.tsx does).
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => {
		root?.unmount();
		root = null;
	});
	replaceDesks([]);
	for (const restore of restores.reverse()) restore();
	await teardown();
});

async function mount(node: React.ReactNode) {
	await act(async () => {
		root!.render(node);
	});
}
const text = () => container.textContent ?? "";
function button(label: string) {
	const found = [...container.querySelectorAll("button")].find((node) => node.textContent === label);
	if (!found) throw new Error(`Missing button ${label}: ${text()}`);
	return found as HTMLButtonElement;
}
async function click(label: string) {
	await act(async () => {
		button(label).click();
	});
}
const detail = () => container.querySelector(".refusal-detail")?.textContent ?? null;
function refuse(cmd: string, reason: string) {
	const command = spyOn(wire, "command").mockImplementation((async (asked: string) => {
		if (asked === cmd) throw new Error(reason);
		return [];
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
}

/** The label is ours and short; what refused is behind Details, which opens and closes. */
async function expectDisclosed(label: string, reason: string) {
	expect(container.querySelector(".refusal-message")?.textContent).toBe(label);
	expect(text()).not.toContain(reason);
	await click("Details");
	expect(detail()).toBe(reason);
	await click("Details");
	expect(detail()).toBeNull();
}

describe("a refusal in settings is a few words, with the detail behind a disclosure", () => {
	test("the update check", async () => {
		const status: UpdateStatus = {
			current: "0.40.0", available: null, checkedAt: null, phase: "idle", downloaded: 0, total: null, error: null, disabledReason: null,
		};
		const spies = [
			spyOn(native, "watchUpdates").mockImplementation((onChange) => { onChange(status); return () => {}; }),
			spyOn(native, "updateStatus").mockResolvedValue({ ...status, error: { message: "Update check failed", detail: "dns error: no such host" } }),
			// The desktop's command refuses with the label and the detail already apart.
			spyOn(native, "checkUpdate").mockRejectedValue({ message: "Update check failed", detail: "dns error: no such host" }),
		];
		restores.push(() => { for (const spy of spies) spy.mockRestore(); });
		await mount(<UpdatesSection />);
		await click("Check now");
		await expectDisclosed("Update check failed", "dns error: no such host");
		expect(button("Check now").disabled).toBe(false);
	});

	test("the skills list", async () => {
		refuse("skills.list", "skills home /srv/private is not a directory");
		await mount(<SkillsSection />);
		await expectDisclosed("Couldn't list skills", "skills home /srv/private is not a directory");
	});

	test("the secrets list", async () => {
		refuse("secrets.list", "keychain daemon refused: errSecInteractionNotAllowed");
		await mount(<SecretsSection />);
		await expectDisclosed("Couldn't read the keychain", "keychain daemon refused: errSecInteractionNotAllowed");
	});

	test("another try at a failed turn keeps its button", async () => {
		const failure = JSON.stringify({ hotlineFailure: { kind: "transport", title: "Provider connection interrupted", summary: "The request could not complete.", details: "dns error" } });
		await mount(<ErrorCard text={failure} onRunAgain={() => Promise.reject(new Error("The connection to Hotline dropped."))} />);
		await click("Try again");
		await expectDisclosed("Couldn't try again", "The connection to Hotline dropped.");
		expect(button("Try again").disabled).toBe(false);
	});

	test("a keychain failure can be retried after unlocking", async () => {
		let attempts = 0;
		const failure = JSON.stringify({ hotlineFailure: { kind: "keychain", title: "Keychain locked", summary: "Unlock your keychain, then send another message.", details: "token store failed" } });
		await mount(<ErrorCard text={failure} onRunAgain={async () => { attempts += 1; }} />);
		await click("Try again");
		expect(attempts).toBe(1);
	});
});
