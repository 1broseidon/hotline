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

test("a sign-in code has a Copy that puts the code on the clipboard", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { ConnectProvider } = await import("../src/components/ConnectProvider");
	const { wire } = await import("../src/wire");
	const native = await import("../src/native");
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string) => {
		if (cmd === "credential.login") return { loginId: "l1", userCode: "WXYZ-1234", verificationUri: "https://x" };
		if (cmd === "credential.login_status") return { state: "pending" };
		return null;
	}) as typeof wire.command);
	const copied: string[] = [];
	const clipboard = spyOn(native, "writeClipboard").mockImplementation(async (text: string) => {
		copied.push(text);
	});
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => root.render(<ConnectProvider provider={oauthOnly} onConnected={() => {}} onCancel={() => {}} />));
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 30));
		});
		const copy = container.querySelector<HTMLButtonElement>('button[aria-label="Copy sign-in code"]');
		expect(copy).not.toBeNull();
		await act(async () => copy!.click());
		expect(copied).toEqual(["WXYZ-1234"]);
		expect(copy!.textContent).toBe("Copied");
	} finally {
		await act(async () => root.unmount());
		clipboard.mockRestore();
		command.mockRestore();
		await teardown();
	}
});
