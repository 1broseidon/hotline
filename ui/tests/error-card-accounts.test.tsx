import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { AccountView } from "../src/generated/contract";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false, addEventListener() {} }) } });
const { wire } = await import("../src/wire");
const { replaceDesks } = await import("../src/desks");
const { ErrorCard } = await import("../src/components/ErrorCard");

const agents = [{ backendId: "claude-acp", name: "Claude Code", supportsAccounts: true, takesEmail: true }];
function account(id: string, name: string, extra: Partial<AccountView> = {}): AccountView {
	return {
		account: { id, backendId: "claude-acp", name, createdAt: 1 },
		status: { state: "signed_out", checkedAt: 1 },
		usedBy: [],
		folder: `/data/agent-accounts/${id}`,
		canSignInHere: true,
		...extra,
	};
}
const a1 = (usedBy = ["ada"]) => account("a1", "Account 1", { status: { state: "signed_in", checkedAt: 1, email: "dev@example.com", plan: "max" }, usedBy });
const a2 = () => account("a2", "Account 2", { status: { state: "signed_in", checkedAt: 1, email: "two@example.com", plan: "pro" } });
const a3 = () => account("a3", "Account 3");
const local = { id: "local", name: "This computer", kind: "local" as const, origin: "", token: "" };
const remote = { id: "server-1", name: "Studio", kind: "remote" as const, origin: "", token: "" };

function notice(failure: Record<string, unknown>): string {
	const full = { title: "Turn failed", summary: "It did not finish.", details: "diagnostic", phase: "acp", ...failure };
	return `${full.title}: ${full.summary}\n\n${JSON.stringify({ hotlineFailure: full })}`;
}
const limit = (extra: Record<string, unknown> = {}) =>
	notice({ kind: "usage_limit", title: "Claude Code usage limit reached", summary: "You've hit your limit · resets 5pm", ...extra });
const signIn = { harnessName: "Claude Code", methods: [{ id: "claude-ai", name: "Claude account" }, { id: "console", name: "Anthropic Console" }] };

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Call = [string, Record<string, unknown>];
let calls: Call[];
let accounts: AccountView[];
let poll: { state: "running" | "succeeded" | "failed"; output: string };
let chosen: number;
let retried: number;
let root: Root | null;
let container: HTMLDivElement;
let dom: Window;
let act: typeof import("react").act;
let restores: (() => void)[];
let teardown: () => Promise<void>;

function respond(cmd: string, params: Record<string, unknown>): unknown {
	if (cmd === "accounts.list") return { agents, accounts };
	if (cmd === "accounts.status") return accounts.find((one) => one.account.id === params["accountId"])?.status;
	if (cmd === "persona.account") return { running: params["accountId"] ?? undefined };
	if (cmd === "accounts.login.start") return { id: "l1" };
	if (cmd === "accounts.login.poll") return poll;
	if (cmd === "accounts.login.cancel") return null;
	throw new Error(`Unexpected command ${cmd}`);
}

beforeEach(async () => {
	restores = [];
	({ dom, teardown } = installDom({ ResizeObserver: NoResize }));
	(dom as unknown as { __hotlineDesk: unknown }).__hotlineDesk = { platform: "windows" };
	replaceDesks([local]);
	calls = [];
	accounts = [a1(), a2(), a3()];
	poll = { state: "running", output: "" };
	chosen = 0;
	retried = 0;
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push([cmd, params]);
		return respond(cmd, params);
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await unmount();
	replaceDesks([]);
	for (const restore of restores.reverse()) restore();
	await teardown();
});

async function mount(text: string, latest = true) {
	await act(async () => {
		root!.render(
			<ErrorCard
				text={text}
				personaId="ada"
				teammate="Ada"
				backendId="claude-acp"
				onChooseAccount={() => chosen++}
				onRetry={() => retried++}
				{...(latest ? { onRunAgain: async () => null } : {})}
			/>,
		);
	});
	await settle();
}
async function unmount() {
	await act(async () => {
		root?.unmount();
		root = null;
	});
}
async function remount(text: string, latest = true) {
	await unmount();
	await act(async () => {
		root = (await import("react-dom/client")).createRoot(container);
	});
	await mount(text, latest);
}
async function settle() {
	for (let round = 0; round < 6; round++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
	}
}
const text = () => container.textContent ?? "";
const buttons = () => [...container.querySelectorAll("button")].map((node) => node.textContent);
function button(label: string): HTMLButtonElement {
	const found = [...container.querySelectorAll("button")].find((node) => node.textContent === label);
	if (!found) throw new Error(`Missing button ${label}: ${text()}`);
	return found as HTMLButtonElement;
}
async function click(label: string) {
	const found = button(label);
	await act(async () => {
		found.click();
	});
	await settle();
}

describe("an account's failure card", () => {
	test("a usage limit offers Continue on each other signed-in account, System default and Try again", async () => {
		await mount(limit({ accountId: "a1" }));
		expect(text()).toContain("Account 1 hit its usage limit");
		expect(text()).toContain("You've hit your limit · resets 5pm");
		expect(calls).toContainEqual(["accounts.status", { accountId: "a2" }]);
		expect(calls).toContainEqual(["accounts.status", { accountId: "a3" }]);
		expect(calls).not.toContainEqual(["accounts.status", { accountId: "a1" }]);
		expect(buttons()).toContain("Continue on Account 2");
		expect(buttons()).toContain("Continue on System default");
		expect(buttons()).toContain("Try again");
		expect(buttons()).not.toContain("Continue on Account 3");
		expect(button("Continue on Account 2").className).toContain("btn-primary");
		expect(button("Continue on System default").className).not.toContain("btn-primary");
		expect(text()).toContain("Error details");

		await click("Continue on Account 2");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: "a2" }]);
		expect(text()).toContain(
			"Now on Account 2: a fresh Claude Code session that is handed this conversation. The failed turn may have done part of its work, so Try again tells it to check before repeating anything.",
		);
		expect(buttons()).toContain("Try again");
		expect(buttons()).not.toContain("Continue on Account 2");
	});

	test("a usage limit on System default still offers Try again", async () => {
		accounts = [a1([]), a2(), a3()];
		await mount(limit());
		expect(text()).toContain("System default hit its usage limit");
		expect(buttons()).toContain("Continue on Account 1");
		expect(buttons()).toContain("Continue on Account 2");
		expect(buttons()).not.toContain("Continue on System default");
		expect(buttons()).toContain("Try again");
		await click("Continue on Account 1");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: "a1" }]);

		const { renderToStaticMarkup } = await import("react-dom/server");
		const html = renderToStaticMarkup(<ErrorCard text={limit({ accountId: "a1" })} personaId="ada" onRunAgain={async () => null} />);
		expect(html).toContain("Claude Code usage limit reached");
		expect(html).toContain("Try again");
		expect(html).not.toContain("Continue on");
	});

	test("only the latest usage limit, with the teammate still on the failed account, asks for each status", async () => {
		const probes = () => calls.filter(([cmd]) => cmd === "accounts.status");
		await mount(limit({ accountId: "a1" }), false);
		expect(text()).toContain("Account 1 hit its usage limit");
		expect(probes()).toEqual([]);

		accounts = [a1([]), a2(), a3()];
		await remount(limit({ accountId: "a1" }));
		expect(text()).toContain("Now on System default");
		expect(probes()).toEqual([]);

		accounts = [a1(), a2(), a3()];
		await remount(limit({ accountId: "a1" }));
		expect(probes()).toEqual([
			["accounts.status", { accountId: "a2" }],
			["accounts.status", { accountId: "a3" }],
		]);
	});

	test("an account-bound sign-in failure offers Sign in and Choose account, not the harness methods", async () => {
		const failure = notice({ kind: "agent_auth", title: "Claude Code sign-in required", summary: "Sign in again", accountId: "a3", signIn });
		await mount(failure);
		expect(text()).toContain("Account 3 isn't signed in");
		expect(text()).toContain(
			"Ada runs on Account 3, and Claude Code says nobody is signed in there. Your message is kept; send it again once the account is ready.",
		);
		expect(buttons()).toContain("Sign in Account 3");
		expect(buttons()).toContain("Choose account");
		expect(text()).not.toContain("Sign-in method");
		expect(text()).not.toContain("Anthropic Console");
		await click("Choose account");
		expect(chosen).toBe(1);

		poll = { state: "succeeded", output: "" };
		await click("Sign in Account 3");
		expect(calls).toContainEqual(["accounts.login.start", { accountId: "a3" }]);
		expect(calls.some(([cmd]) => cmd.startsWith("agent.auth."))).toBe(false);
		expect(text()).toContain("Signed in. The teammate is ready for another message.");
		await click("Review failed message");
		expect(retried).toBe(1);

		accounts = [a1(), a2(), { ...a3(), canSignInHere: false }];
		await remount(failure);
		expect(buttons()).not.toContain("Sign in Account 3");
		expect(text()).toContain("Sign in from the desk itself");
		expect(buttons()).toContain("Choose account");
	});

	test("an unavailable account offers Show folder only for a listed folder on the Desk", async () => {
		const folder = notice({
			kind: "account_unavailable",
			code: "folder_unavailable",
			accountId: "a1",
			title: "Ada can't start",
			summary: "Can't open Account 1's folder (/data/agent-accounts/a1): permission denied. Fix the folder or pick another account.",
		});
		await mount(folder);
		expect(text()).toContain("Can't open Account 1's folder");
		expect(text()).toContain("permission denied. Fix the folder or pick another account.");
		expect(buttons()).toEqual(["Show folder", "Choose account"]);

		replaceDesks([remote]);
		await remount(folder);
		expect(buttons()).toEqual(["Choose account"]);

		replaceDesks([local]);
		await remount(
			notice({
				kind: "account_unavailable",
				code: "unknown_account",
				accountId: "gone",
				title: "Ada can't start",
				summary: "Pick a Claude Code account for Ada: the one it was set to is not on this desk.",
			}),
		);
		expect(text()).toContain("Pick a Claude Code account");
		expect(text()).toContain("Pick a Claude Code account for Ada: the one it was set to is not on this desk.");
		expect(buttons()).toEqual(["Choose account"]);
		await click("Choose account");
		expect(chosen).toBe(1);
	});

	test("a card with no account calls no wire command", async () => {
		await mount(notice({ kind: "agent_auth", title: "Claude Code sign-in required", summary: "Sign in again", signIn }));
		expect(text()).toContain("Sign-in method");
		await remount(notice({ kind: "quota", title: "Quota spent", summary: "No credit left" }));
		await remount("Plain diagnostic text");
		expect(calls).toEqual([]);
	});
});
