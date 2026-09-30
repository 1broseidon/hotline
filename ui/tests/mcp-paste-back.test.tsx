import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";

// Loading the window's modules reads the platform, and the theme, before any test has a DOM.
Object.assign(globalThis, {
	window: { matchMedia: () => ({ matches: false, addEventListener() {} }) },
	localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
});
const { wire } = await import("../src/wire");
const { replaceDesks, setActiveDesk } = await import("../src/desks");
const { McpOAuthControls } = await import("../src/components/Settings");
Reflect.deleteProperty(globalThis, "localStorage");

const server = { id: "linear", type: "http" as const, name: "Linear", url: "https://mcp.example/mcp", auth: { mode: "oauth" } };
const local = { id: "local", name: "This computer", kind: "local" as const, origin: "", token: "" };
const remote = { id: "server-1", name: "Studio", kind: "remote" as const, origin: "", token: "" };
const CALLBACK = "http://127.0.0.1:41777/oauth/callback/linear?code=secret-code&state=the-state";

type Call = { cmd: string; params: Record<string, unknown> };
let calls: Call[];
let answer: (call: Call) => unknown;
let root: Root | null;
let container: HTMLDivElement;
let dom: Window;
let act: typeof import("react").act;
let restores: (() => void)[];

const pending = { serverId: "linear", status: "pending", loginId: "login-1", authorizationUrl: "https://provider.example/authorize", redirectUri: "http://127.0.0.1:41777/oauth/callback/linear" };

beforeEach(async () => {
	restores = [];
	dom = new Window();
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, IS_REACT_ACT_ENVIRONMENT: true })) {
		const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
		restores.push(() => {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		});
	}
	calls = [];
	answer = ({ cmd }) => {
		if (cmd === "mcp.auth_status") return pending;
		throw new Error(`Unexpected command ${cmd}`);
	};
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		const call = { cmd, params };
		calls.push(call);
		return answer(call);
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
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
	await dom.happyDOM.close();
});

async function mount(desk: "local" | "server") {
	replaceDesks([local, remote]);
	setActiveDesk(desk === "server" ? remote.id : local.id);
	await act(async () => {
		root!.render(<McpOAuthControls server={server} />);
	});
}
const text = () => container.textContent ?? "";
const field = () => container.querySelector("textarea") as HTMLTextAreaElement | null;
function button(label: string) {
	const found = [...container.querySelectorAll("button")].find((node) => node.textContent === label);
	if (!found) throw new Error(`Missing button ${label}: ${text()}`);
	return found as HTMLButtonElement;
}
async function paste(value: string) {
	await act(async () => {
		Object.getOwnPropertyDescriptor(dom.HTMLTextAreaElement.prototype, "value")!.set!.call(field()!, value);
		field()!.dispatchEvent(new dom.Event("input", { bubbles: true }));
	});
}
async function click(label: string) {
	await act(async () => {
		button(label).click();
	});
}
const callbacks = () => calls.filter(({ cmd }) => cmd === "mcp.auth_callback");

describe("MCP sign-in on a desk on a server", () => {
	test("asks for the address the browser lands on, in one sentence, while the sign-in is pending", async () => {
		await mount("server");
		expect(text()).toContain("Approve in your browser, then paste the address of the page you land on (it won't load, and that's expected).");
		expect(field()).not.toBeNull();
		expect(button("Finish sign-in").disabled).toBe(true);
		await paste("   ");
		expect(button("Finish sign-in").disabled).toBe(true);
	});

	test("finishes with the loginId and the pasted address, shows the returned status, and forgets the address", async () => {
		answer = ({ cmd }) => (cmd === "mcp.auth_status" ? pending : { ...pending, status: "signed_in" });
		await mount("server");
		await paste(`  ${CALLBACK}\n`);
		await click("Finish sign-in");
		expect(callbacks()).toEqual([{ cmd: "mcp.auth_callback", params: { loginId: "login-1", callbackUrl: CALLBACK } }]);
		expect(text()).toContain("Signed in");
		expect(field()).toBeNull();
		expect(text()).not.toContain("secret-code");
	});

	test("a refusal is one sentence of ours with the desk's words behind Details, and never shows the address", async () => {
		answer = ({ cmd }) => {
			if (cmd === "mcp.auth_status") return pending;
			throw new Error("MCP OAuth callback state did not match this sign-in.");
		};
		await mount("server");
		await paste(CALLBACK);
		await click("Finish sign-in");
		expect(text()).toContain("That address didn't finish the sign-in.");
		expect(text()).not.toContain("state did not match");
		await click("Details");
		expect(container.querySelector("pre")?.textContent).toBe("MCP OAuth callback state did not match this sign-in.");
		expect(container.querySelector(".refusal")?.textContent).not.toContain("secret-code");
		// The sign-in is still pending, so the person can try the right address.
		expect(field()!.value).toBe(CALLBACK);
		expect(button("Finish sign-in").disabled).toBe(false);
	});

	test("offers nothing to paste unless a sign-in is pending", async () => {
		answer = ({ cmd }) => ({ serverId: "linear", status: cmd === "mcp.auth_status" ? "signed_out" : "failed" });
		await mount("server");
		expect(field()).toBeNull();
		expect(text()).not.toContain("Approve in your browser");
	});
});

describe("MCP sign-in on this computer's own desk", () => {
	test("is as it was: the provider's redirect arrives on this machine, so there is nothing to paste", async () => {
		await mount("local");
		expect(text()).toContain("Signing in…");
		expect(text()).toContain("Waiting for consent in your browser…");
		expect(field()).toBeNull();
		expect(text()).not.toContain("Approve in your browser");
		expect(callbacks()).toEqual([]);
	});
});
