import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { AccountChoice } from "../src/components/accounts/fits";
import type { AccountsList, CredentialAccount } from "../src/generated/contract";
import { installDom } from "./dom";

let act: typeof import("react").act;
let root: Root;
let card: HTMLDivElement;
let dom: Window;
let teardown: () => Promise<void>;
let picked: string[];

beforeEach(async () => {
	({ dom, teardown } = installDom());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	card = document.createElement("div");
	document.body.append(card);
	root = createRoot(card);
	picked = [];
});

afterEach(async () => {
	await act(async () => root.unmount());
	await teardown();
});

function choice(id: string, name: string, extra: Partial<AccountChoice> = {}): AccountChoice {
	return { id, name, section: "subscription", detail: "Signed in", marks: [], payPerUse: false, isDefault: false, signedIn: true, ...extra };
}

const key = (id: string, name: string) => choice(id, name, { section: "api_key", detail: "API key", payPerUse: true });

async function mount(choices: AccountChoice[], selected: string, adds?: { subscription?: () => void; apiKey?: () => void }) {
	const { AccountPicker } = await import("../src/components/accounts/AccountPicker");
	await act(async () => {
		root.render(<AccountPicker choices={choices} selected={selected} label="Continue on" onSelect={(id) => picked.push(id)} {...(adds ? { adds } : {})} />);
	});
}

const button = () => card.querySelector<HTMLButtonElement>('[aria-label="Continue on"]')!;
const listbox = () => document.querySelector<HTMLElement>('[role="listbox"]');
const options = () => [...document.querySelectorAll<HTMLElement>('[role="option"]')];
const optionNames = () => options().map((one) => one.querySelector(".menu-item-text")?.firstChild?.textContent);
const search = () => document.querySelector<HTMLInputElement>('input[type="search"]');
const adds = () => [...document.querySelectorAll<HTMLButtonElement>(".account-picker-add")];

async function toggle() {
	await act(async () => button().click());
}

async function press(key: string, extra: { shiftKey?: boolean } = {}) {
	const target = document.activeElement as HTMLElement;
	await act(async () => {
		target.dispatchEvent(new window.KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...extra }));
	});
}

async function type(input: HTMLInputElement, value: string) {
	await act(async () => {
		Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, "value")!.set!.call(input, value);
		input.dispatchEvent(new window.Event("input", { bubbles: true }));
	});
}

describe("the account picker", () => {
	// happy-dom lays nothing out, so no height can be read: the proof is that
	// the menu floats on the window outside the card, and nothing in the card
	// is added or replaced while it is open.
	test("opens and closes without changing the parent container's height", async () => {
		await mount([choice("w", "Work"), choice("p", "Personal"), choice("", "This computer", { signedIn: false })], "p");
		const before = [...card.querySelectorAll("*")];
		const unchanged = () => {
			const now = [...card.querySelectorAll("*")];
			expect(now).toHaveLength(before.length);
			for (const [at, element] of now.entries()) expect(element).toBe(before[at]!);
		};
		expect(button().getAttribute("aria-expanded")).toBe("false");
		expect(button().textContent).toContain("Personal");

		await toggle();
		expect(button().getAttribute("aria-expanded")).toBe("true");
		const menu = listbox()!.closest<HTMLElement>(".account-picker-menu")!;
		expect(menu.parentElement).toBe(document.body);
		expect(menu.style.position).toBe("fixed");
		expect(card.contains(menu)).toBe(false);
		unchanged();
		expect(optionNames()).toEqual(["Work", "Personal", "This computer"]);
		expect(options().map((one) => one.getAttribute("aria-selected"))).toEqual(["false", "true", "false"]);

		await press("Escape");
		expect(listbox()).toBeNull();
		expect(document.activeElement).toBe(button());
		unchanged();
		expect(picked).toEqual([]);
	});

	test("a selection that is none of its accounts reads Choose an account", async () => {
		await mount([key("a", "Key A"), key("b", "Key B")], "");
		expect(button().textContent).toBe("Choose an account");
		await toggle();
		expect(options().map((one) => one.getAttribute("aria-selected"))).toEqual(["false", "false"]);
	});

	test("a click outside closes it", async () => {
		await mount([choice("w", "Work"), choice("p", "Personal")], "w");
		await toggle();
		await act(async () => {
			document.body.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true }));
		});
		expect(listbox()).toBeNull();
	});

	test("Escape closes it and never reaches the window", async () => {
		const heard: string[] = [];
		window.addEventListener("keydown", (event) => heard.push(event.key));
		await mount([choice("w", "Work"), choice("p", "Personal")], "w");
		await toggle();
		await press("a");
		await press("Escape");
		expect(listbox()).toBeNull();
		expect(heard).toEqual(["a"]);
	});

	test("arrows move through the accounts and Enter picks one", async () => {
		await mount([choice("w", "Work"), choice("p", "Personal"), key("k", "Prod key")], "w");
		await toggle();
		await press("ArrowDown");
		await press("ArrowDown");
		expect(listbox()!.getAttribute("aria-activedescendant")).toBe(options()[2]!.id);
		await press("Enter");
		expect(picked).toEqual(["k"]);
		expect(listbox()).toBeNull();
		expect(document.activeElement).toBe(button());
	});

	test("ArrowUp from the first account wraps to the last", async () => {
		await mount([choice("w", "Work"), choice("p", "Personal"), key("k", "Prod key")], "w");
		await toggle();
		expect(listbox()!.getAttribute("aria-activedescendant")).toBe(options()[0]!.id);
		await press("ArrowUp");
		expect(listbox()!.getAttribute("aria-activedescendant")).toBe(options()[2]!.id);
	});

	test("a search appears past eight accounts and filters them by name", async () => {
		const many = Array.from({ length: 9 }, (_, at) => choice(`a${at}`, at === 4 ? "Agency" : `Account ${at + 1}`));
		await mount(many.slice(0, 8), "a0");
		await toggle();
		expect(search()).toBeNull();
		await toggle();

		await mount(many, "a0");
		await toggle();
		expect(search()!.placeholder).toBe("Search 9 accounts");
		expect(document.activeElement).toBe(search()!);
		expect(search()!.getAttribute("role")).toBe("combobox");
		expect(search()!.getAttribute("aria-controls")).toBe(listbox()!.id);
		await type(search()!, "agen");
		expect(optionNames()).toEqual(["Agency"]);
		await press("Enter");
		expect(picked).toEqual(["a4"]);
	});

	test("arrows walk only what the search left", async () => {
		const names = ["Work", "Agency", "Personal", "Team", "Agents", "Spare", "Old", "Lab", "Home"];
		await mount(
			names.map((name, at) => choice(`a${at}`, name)),
			"a0",
		);
		await toggle();
		await type(search()!, "agen");
		expect(optionNames()).toEqual(["Agency", "Agents"]);
		expect(search()!.getAttribute("aria-activedescendant")).toBe(options()[0]!.id);
		await press("ArrowDown");
		expect(search()!.getAttribute("aria-activedescendant")).toBe(options()[1]!.id);
		await press("ArrowDown");
		expect(search()!.getAttribute("aria-activedescendant")).toBe(options()[0]!.id);
		await press("ArrowUp");
		await press("Enter");
		expect(picked).toEqual(["a4"]);
	});

	test("each section is a group named by its heading, and keys are marked pay per use", async () => {
		await mount([choice("x", "SuperGrok"), key("k", "xAI key"), choice("o", "Ollama Local", { section: "server", detail: "http://localhost:11434" })], "k");
		expect(button().textContent).toContain("xAI key · API key · pay per use");
		await toggle();
		const groups = [...listbox()!.querySelectorAll('[role="group"]')].map((group) => [
			document.getElementById(group.getAttribute("aria-labelledby")!)?.textContent,
			...[...group.querySelectorAll(".menu-item-text")].map((one) => one.firstChild?.textContent),
		]);
		expect(groups).toEqual([
			["Subscriptions", "SuperGrok"],
			["API keys · pay per use", "xAI key"],
			["Servers", "Ollama Local"],
		]);
	});

	test("Tab walks to the add rows, and closes the menu where it would leave it", async () => {
		await mount([choice("w", "Work"), choice("", "This computer")], "", { subscription: () => {}, apiKey: () => {} });
		await toggle();
		expect(document.activeElement).toBe(listbox()!);
		await press("Tab");
		expect(document.activeElement).toBe(adds()[0]!);
		await press("Tab");
		expect(document.activeElement).toBe(adds()[1]!);
		await press("Tab", { shiftKey: true });
		expect(document.activeElement).toBe(adds()[0]!);
		await press("Tab", { shiftKey: true });
		expect(document.activeElement).toBe(listbox()!);
		await press("Tab", { shiftKey: true });
		expect(listbox()).toBeNull();
		expect(document.activeElement).toBe(button());

		await toggle();
		await press("Tab");
		await press("Tab");
		await press("Tab");
		expect(listbox()).toBeNull();
		expect(document.activeElement).toBe(button());
	});

	test("two keys with one name are told apart by the day each was added, and the default says so", async () => {
		const { providerAccounts } = await import("../src/components/accounts/fits");
		const at = 1_800_000_000_000;
		const later = at + 3 * 86_400_000;
		const day = (when: number) => new Date(when).toLocaleDateString(undefined, { month: "short", day: "numeric" });
		const credential = (id: string, label: string, createdAt: number, isDefault = false): CredentialAccount => ({
			credential: { id, providerId: "anthropic", credentialKind: "api_key", label, revoked: false, createdAt, updatedAt: createdAt },
			section: "api_key",
			usedBy: [],
			default: isDefault,
		});
		const list: AccountsList = {
			agents: [],
			accounts: [],
			credentials: [credential("k1", "Anthropic", at, true), credential("k2", "anthropic", later), credential("k3", "Spare", at)],
		};
		await mount(providerAccounts(list, "anthropic"), "k2");
		expect(button().textContent).toBe(`anthropic · API key · pay per use · Added ${day(later)}`);
		await toggle();
		const details = options().map((one) => one.querySelector(".menu-item-detail")?.textContent);
		expect(details).toEqual([`API key · Added ${day(at)} · Default`, `API key · Added ${day(later)}`, "API key"]);
	});

	test("the add rows call what adds brings, and only those", async () => {
		const added: string[] = [];
		await mount([choice("w", "Work"), choice("", "This computer")], "", { subscription: () => added.push("subscription") });
		await toggle();
		expect(adds().map((one) => one.textContent)).toEqual(["Add subscription"]);
		await act(async () => adds()[0]!.click());
		expect(added).toEqual(["subscription"]);
		expect(listbox()).toBeNull();

		await mount([key("a", "Prod key"), key("b", "Personal key")], "a", { subscription: () => added.push("subscription"), apiKey: () => added.push("key") });
		await toggle();
		expect(adds().map((one) => one.textContent)).toEqual(["Add subscription", "Add API key"]);
		await act(async () => adds()[1]!.click());
		expect(added).toEqual(["subscription", "key"]);
		expect(picked).toEqual([]);
	});
});
