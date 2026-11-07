import { expect, spyOn, test } from "bun:test";
import type { Provider } from "../src/generated/contract";
import { installDom } from "./dom";

const oauthOnly: Provider = { id: "acme", name: "Acme", credentialKinds: ["oauth"], modelDiscovery: false };

test("a sign-in flow is one state, and every refusal lands somewhere with a way forward", async () => {
	const { signInStep } = await import("../src/signIn");
	const login = { loginId: "l1", userCode: "", verificationUri: "https://x" };
	let flow = signInStep({ k: "choosing" }, { type: "begin" });
	expect(flow).toEqual({ k: "preparing" });
	flow = signInStep(flow, { type: "prompted", login });
	expect(flow).toEqual({ k: "waiting", login });
	const refusal = { message: "Sign-in failed" };
	for (const from of [{ k: "preparing" }, { k: "waiting", login }, { k: "finishing", form: { k: "choosing" } }] as const) {
		const out = signInStep(from, { type: "refused", refusal, back: { k: "choosing" } });
		expect(out).toEqual({ k: "refused", refusal, back: { k: "choosing" } });
	}
	// Choosing again from a refusal starts over; a key refusal goes back to its field.
	expect(signInStep({ k: "refused", refusal, back: { k: "choosing" } }, { type: "begin" })).toEqual({ k: "preparing" });
	expect(signInStep({ k: "refused", refusal, back: { k: "entering", kind: "api_key" } }, { type: "saving", form: { k: "entering", kind: "api_key" } })).toEqual({ k: "finishing", form: { k: "entering", kind: "api_key" } });
	// A late prompt for a sign-in nobody is waiting on changes nothing.
	expect(signInStep({ k: "choosing" }, { type: "prompted", login })).toEqual({ k: "choosing" });
	expect(signInStep({ k: "waiting", login }, { type: "reset" })).toEqual({ k: "choosing" });
});

test("a status poll that fails leaves the pane with a way to sign in again, not stuck preparing", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { ConnectProvider } = await import("../src/components/ConnectProvider");
	const { wire } = await import("../src/wire");
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string) => {
		if (cmd === "credential.login") return { loginId: "l1", userCode: "", verificationUri: "https://x" };
		if (cmd === "credential.login_status") throw new Error("network blip");
		return null;
	}) as typeof wire.command);
	const realTimeout = globalThis.setTimeout;
	const timeout = spyOn(globalThis, "setTimeout").mockImplementation(((fn: () => void, ms?: number) =>
		realTimeout(fn, ms === 2000 ? 0 : ms)) as typeof setTimeout);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => root.render(<ConnectProvider provider={oauthOnly} onConnected={() => {}} onCancel={() => {}} />));
		await act(async () => {
			await new Promise((resolve) => realTimeout(resolve, 30));
		});
		expect(container.textContent).not.toContain("Preparing sign-in");
		expect(container.textContent).toContain("Sign-in failed");
		expect([...container.querySelectorAll("button")].map((b) => b.textContent)).toContain("Sign in");
	} finally {
		await act(async () => root.unmount());
		timeout.mockRestore();
		command.mockRestore();
		await teardown();
	}
});

const chatgpt: Provider = { id: "openai-codex", name: "ChatGPT", credentialKinds: ["oauth"], modelDiscovery: true };
const REDIRECT = "http://127.0.0.1:1455/auth/callback";

/** ChatGPT's pane, on a wire that answers `answer` and records every call. */
async function chatgptPane(answer: (cmd: string, params: Record<string, unknown>) => unknown) {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { ConnectProvider } = await import("../src/components/ConnectProvider");
	const { wire } = await import("../src/wire");
	const calls: { cmd: string; params: Record<string, unknown> }[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push({ cmd, params });
		return answer(cmd, params);
	}) as typeof wire.command);
	const opened: string[] = [];
	const open = spyOn(window, "open").mockImplementation(((href: string) => {
		opened.push(href);
		return null;
	}) as typeof window.open);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	await act(async () => root.render(<ConnectProvider provider={chatgpt} onConnected={() => {}} onCancel={() => {}} />));
	const click = async (label: string) => {
		const button = [...container.querySelectorAll("button")].find((b) => b.textContent === label);
		if (!button) throw new Error(`No button ${label}: ${container.textContent}`);
		await act(async () => button.click());
	};
	const close = async () => {
		await act(async () => root.unmount());
		open.mockRestore();
		command.mockRestore();
		await teardown();
	};
	return { act, container, calls, opened, click, close };
}

test("Sign in with ChatGPT opens the browser and takes the address it ends on", async () => {
	const pane = await chatgptPane((cmd, params) => {
		if (cmd === "credential.login") return { loginId: "l1", userCode: "", verificationUri: "https://auth.openai.com/api/accounts/authorize?x", redirectUri: REDIRECT };
		if (cmd === "credential.login_redirect" && String(params["redirectUrl"]).includes("stale")) throw new Error("This page belongs to another sign-in attempt.");
		if (cmd === "credential.login_redirect") return null;
		if (cmd === "credential.login_status") return { state: "pending" };
		return null;
	});
	try {
		const choices = [...pane.container.querySelectorAll("button")].map((b) => b.textContent);
		expect(choices).toContain("Sign in with ChatGPT");
		expect(choices).toContain("Use a device code");
		await pane.click("Sign in with ChatGPT");
		expect(pane.calls[0]).toEqual({ cmd: "credential.login", params: { providerId: "openai-codex", browser: true } });
		expect(pane.opened).toEqual(["https://auth.openai.com/api/accounts/authorize?x"]);
		expect(pane.container.textContent).toContain("Open ChatGPT sign-in");

		const field = pane.container.querySelector("textarea");
		if (!field) throw new Error("no paste field");
		expect(field.placeholder).toBe(`${REDIRECT}?…`);
		const type = async (text: string) => {
			const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(field), "value")?.set;
			await pane.act(async () => {
				setter?.call(field, text);
				field.dispatchEvent(new window.Event("input", { bubbles: true }));
			});
		};
		await type(`${REDIRECT}?code=c&state=stale`);
		await pane.click("Finish sign-in");
		expect(pane.container.textContent).toContain("That address didn't finish the sign-in.");

		await type(`  ${REDIRECT}?code=c&state=s&client_id=oaiapp  `);
		await pane.click("Finish sign-in");
		expect(pane.calls.at(-1)).toEqual({ cmd: "credential.login_redirect", params: { loginId: "l1", redirectUrl: `${REDIRECT}?code=c&state=s&client_id=oaiapp` } });
		expect(pane.container.textContent).not.toContain("That address didn't finish the sign-in.");
	} finally {
		await pane.close();
	}
});

test("a device code stays the other way into ChatGPT, with nothing to paste", async () => {
	const pane = await chatgptPane((cmd) => {
		if (cmd === "credential.login") return { loginId: "l2", userCode: "ABCD-1234", verificationUri: "https://auth.openai.com/codex/device" };
		if (cmd === "credential.login_status") return { state: "pending" };
		return null;
	});
	try {
		await pane.click("Use a device code");
		expect(pane.calls[0]).toEqual({ cmd: "credential.login", params: { providerId: "openai-codex" } });
		expect(pane.opened).toEqual([]);
		expect(pane.container.textContent).toContain("ABCD-1234");
		expect(pane.container.querySelector("textarea")).toBeNull();
	} finally {
		await pane.close();
	}
});
