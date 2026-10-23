import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { AccountChoice } from "../src/components/accounts/fits";
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
	return { id, name, section: "subscription", detail: "Signed in", payPerUse: false, isDefault: false, signedIn: true, ...extra };
}

const key = (id: string, name: string) => choice(id, name, { section: "api_key", detail: "API key", payPerUse: true });

async function mount(choices: AccountChoice[], selected: string, adds?: { subscription?: () => void; apiKey?: () => void }) {
	const { AccountPicker } = await import("../src/components/accounts/AccountPicker");
	await act(async () => {
		root.render(<AccountPicker choices={choices} selected={selected} label="Continue on" onSelect={(id) => picked.push(id)} {...(adds ? { adds } : {})} />);
	});
}

const button = () => card.querySelector<HTMLButtonElement>('[aria-label="Continue on"]')!;
const listbox = () => card.querySelector<HTMLElement>('[role="listbox"]');
const options = () => [...card.querySelectorAll<HTMLElement>('[role="option"]')];
const optionNames = () => options().map((one) => one.querySelector(".menu-item-text")?.firstChild?.textContent);
const search = () => card.querySelector<HTMLInputElement>('input[type="search"]');

async function toggle() {
	await act(async () => button().click());
}

async function press(key: string) {
	const target = document.activeElement as HTMLElement;
	await act(async () => {
		target.dispatchEvent(new window.KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
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
	// the menu hangs out of the flow from a box anchored on the button, and the
	// card gains no child of its own while it is open.
	test("opens and closes without changing the parent container's height", async () => {
		await mount([choice("w", "Work"), choice("p", "Personal"), choice("", "This computer", { signedIn: false })], "p");
		const before = [...card.children];
		expect(button().getAttribute("aria-expanded")).toBe("false");
		expect(button().textContent).toContain("Personal");

		await toggle();
		expect(button().getAttribute("aria-expanded")).toBe("true");
		const menu = listbox()!.closest<HTMLElement>(".account-picker-menu")!;
		expect(menu.style.position).toBe("absolute");
		expect(menu.parentElement!.style.position).toBe("relative");
		expect(menu.parentElement!.contains(button())).toBe(true);
		expect([...card.children]).toEqual(before);
		expect(optionNames()).toEqual(["Work", "Personal", "This computer"]);
		expect(options().map((one) => one.getAttribute("aria-selected"))).toEqual(["false", "true", "false"]);

		await press("Escape");
		expect(listbox()).toBeNull();
		expect(document.activeElement).toBe(button());
		expect([...card.children]).toEqual(before);
		expect(picked).toEqual([]);
	});

	test("a click outside closes it", async () => {
		await mount([choice("w", "Work"), choice("p", "Personal")], "w");
		await toggle();
		await act(async () => {
			document.body.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true }) as unknown as Event);
		});
		expect(listbox()).toBeNull();
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

	test("a search appears past eight accounts and filters them by name", async () => {
		const many = Array.from({ length: 9 }, (_, at) => choice(`a${at}`, at === 4 ? "Agency" : `Account ${at + 1}`));
		await mount(many.slice(0, 8), "a0");
		await toggle();
		expect(search()).toBeNull();
		await toggle();

		await mount(many, "a0");
		await toggle();
		expect(search()!.placeholder).toBe("Search 9 accounts");
		await type(search()!, "agen");
		expect(optionNames()).toEqual(["Agency"]);
		await press("Enter");
		expect(picked).toEqual(["a4"]);
	});

	test("keys sit under their own heading, marked pay per use", async () => {
		await mount([choice("x", "SuperGrok"), key("k", "xAI key"), choice("o", "Ollama Local", { section: "server", detail: "http://localhost:11434" })], "k");
		expect(button().textContent).toContain("xAI key · API key · pay per use");
		await toggle();
		const rows = [...listbox()!.children].map((one) => (one.getAttribute("role") === "option" ? `· ${one.querySelector(".menu-item-text")?.firstChild?.textContent}` : one.textContent));
		expect(rows).toEqual(["Subscriptions", "· SuperGrok", "API keys · pay per use", "· xAI key", "Servers", "· Ollama Local"]);
	});

	test("the add rows call what adds brings, and only those", async () => {
		const added: string[] = [];
		await mount([choice("w", "Work"), choice("", "This computer")], "", { subscription: () => added.push("subscription") });
		await toggle();
		const adds = () => [...card.querySelectorAll<HTMLButtonElement>(".account-picker-add")].map((one) => one.textContent);
		expect(adds()).toEqual(["Add subscription"]);
		await act(async () => card.querySelector<HTMLButtonElement>(".account-picker-add")!.click());
		expect(added).toEqual(["subscription"]);
		expect(listbox()).toBeNull();

		await mount([key("a", "Prod key"), key("b", "Personal key")], "a", { subscription: () => added.push("subscription"), apiKey: () => added.push("key") });
		await toggle();
		expect(adds()).toEqual(["Add subscription", "Add API key"]);
		await act(async () => [...card.querySelectorAll<HTMLButtonElement>(".account-picker-add")][1]!.click());
		expect(added).toEqual(["subscription", "key"]);
		expect(picked).toEqual([]);
	});
});
