import { expect, spyOn, test } from "bun:test";
import type { SharedSecret, TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

test("Settings shows Automatic or Ask me for each key and saves the selected policy", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { SecretsSection } = await import("../src/components/Secrets");
	const { wire } = await import("../src/wire");
	let stored: SharedSecret[] = [{ name: "LOGIN", kind: "login", updatedAt: 1, use: "auto" }, { name: "KEY", kind: "passkey", updatedAt: 1, use: "ask" }, { name: "TOKEN", kind: "variable", updatedAt: 1, use: "auto" }];
	const commands: unknown[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: { name: string; use: "auto" | "ask" }) => {
		if (cmd === "secrets.list") return stored;
		if (cmd === "secrets.services") return [];
		if (cmd === "computer.cookies.sources") return { sources: [], blocked: [] };
		if (cmd === "computer.cookies.holders") return [];
		commands.push({ cmd, params });
		const result = { ...stored.find(s => s.name === params.name)!, use: params.use };
		stored = stored.map(s => s.name === params.name ? result : s);
		return result;
	}) as typeof wire.command);
	const container = document.createElement("div"); document.body.append(container);
	const root = createRoot(container);
	const choice = (name: string, label: string) => Array.from(container.querySelectorAll<HTMLButtonElement>(`fieldset[aria-label="Use ${name}"] button`)).find(b => b.textContent === label)!;
	try {
		await act(async () => { root.render(<SecretsSection />); });
		expect(choice("LOGIN", "Automatic").getAttribute("aria-pressed")).toBe("true");
		expect(choice("KEY", "Ask me").getAttribute("aria-pressed")).toBe("true");
		expect(container.querySelector('[aria-label="Use TOKEN"]')).toBeNull();
		await act(async () => { choice("LOGIN", "Ask me").click(); });
		await act(async () => { choice("KEY", "Automatic").click(); });
		expect(commands).toEqual([{ cmd: "secrets.use.set", params: { name: "LOGIN", use: "ask" } }, { cmd: "secrets.use.set", params: { name: "KEY", use: "auto" } }]);
		await act(async () => { root.render(null); });
		await act(async () => { root.render(<SecretsSection />); });
		expect(choice("LOGIN", "Ask me").getAttribute("aria-pressed")).toBe("true");
		expect(choice("KEY", "Automatic").getAttribute("aria-pressed")).toBe("true");
	} finally { await act(async () => root.unmount()); command.mockRestore(); await teardown(); }
});

test("Secret use card sends Approve, Allow 10 min and Deny once and settles or expires without buttons", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { SecretUseAsk } = await import("../src/components/transcript/SecretUseAsk");
	const { wire } = await import("../src/wire");
	const calls: unknown[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => { calls.push({ cmd, params }); return null; }) as typeof wire.command);
	const container = document.createElement("div"); document.body.append(container);
	const root = createRoot(container);
	const pending: Extract<TranscriptEvent, { kind: "secret_use_ask" }> = { kind: "secret_use_ask", id: "card", ts: 1, useId: "opaque", secret: "GH_PASSKEY", secretKind: "passkey", site: "github.com", path: "/login", teammate: "ada", askedAt: Date.now(), expiresAt: Date.now() + 300_000, status: "pending", operation: { kind: "passkey", origin: "https://github.com", account: "", rpId: "github.com" }, attested: true };
	try {
		for (const [label, answer] of [["Approve", "approve"], ["Allow 10 min", "allow"], ["Deny", "deny"]]) {
			await act(async () => { root.render(<SecretUseAsk key={label} event={{ ...pending, id: `card:${label}` }} teammate="Pork Chop" />); });
			expect(container.textContent).toContain("github.com/login");
			expect(container.textContent).toContain("Pork Chop");
			const button = Array.from(container.querySelectorAll("button")).find(b => b.textContent === label)!;
			await act(async () => button.click());
			await act(async () => button.click());
			expect(calls.at(-1)).toEqual({ cmd: "secrets.use.answer", params: { personaId: "ada", useId: "opaque", answer } });
		}
		expect(calls.length).toBe(3);
		for (const status of ["approved", "allowed", "denied", "expired"] as const) {
			await act(async () => { root.render(<SecretUseAsk key={status} event={{ ...pending, status }} teammate="Pork Chop" />); });
			expect(container.querySelectorAll("button").length).toBe(0);
		}
		await act(async () => { root.render(<SecretUseAsk key="expired-locally" event={{ ...pending, expiresAt: Date.now() - 1 }} teammate="Pork Chop" />); });
		expect(container.textContent).toContain("Expired");
		expect(container.querySelectorAll("button").length).toBe(0);
	} finally { await act(async () => root.unmount()); command.mockRestore(); await teardown(); }
});

test("a split computer's card asks about one sign-in, and offers ten minutes only when its own browser asked", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { SecretUseAsk } = await import("../src/components/transcript/SecretUseAsk");
	const container = document.createElement("div"); document.body.append(container);
	const root = createRoot(container);
	const login: Extract<TranscriptEvent, { kind: "secret_use_ask" }> = { kind: "secret_use_ask", id: "card", ts: 1, useId: "opaque", secret: "GITHUB", secretKind: "login", site: "github.com", path: "/login", teammate: "ada", askedAt: Date.now(), expiresAt: Date.now() + 300_000, status: "pending", operation: { kind: "signin", origin: "https://github.com", account: "george" } };
	const labels = () => Array.from(container.querySelectorAll("button")).map((b) => b.textContent);
	try {
		await act(async () => { root.render(<SecretUseAsk event={login} teammate="Ada" />); });
		// The same words the desk browser's card asks (`use_question` in the core).
		expect(container.textContent).toContain("Sign in to github.com as george with GITHUB?");
		expect(labels()).toEqual(["Approve", "Deny"]);
		expect(container.textContent).toContain("for one use");
		await act(async () => { root.render(<SecretUseAsk event={{ ...login, attested: true }} teammate="Ada" />); });
		expect(labels()).toEqual(["Approve", "Allow 10 min", "Deny"]);
		expect(container.textContent).toContain("covers this sign-in only");
		// A computer from before split mode says no operation: its card is as it was.
		const { operation: _none, ...older } = login;
		await act(async () => { root.render(<SecretUseAsk event={older} teammate="Ada" />); });
		expect(container.textContent).toContain("Use GITHUB on github.com?");
		expect(labels()).toEqual(["Approve", "Allow 10 min", "Deny"]);
	} finally { await act(async () => root.unmount()); await teardown(); }
});

test("a key's start suggests what it is Used with, the person sees it picked, and Store sends it", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { SecretsSection } = await import("../src/components/Secrets");
	const { wire } = await import("../src/wire");
	const github = { origins: ["https://api.github.com"], header: "Authorization", schemes: ["Bearer"] };
	const services = [
		{ id: "github", name: "GitHub", prefixes: ["ghp_"], ...github, refuse: [] },
		{ id: "other", name: "Other", prefixes: [], origins: [], header: "Authorization", schemes: ["Bearer"], refuse: [] },
	];
	const calls: { cmd: string; params: unknown }[] = [];
	const stored: SharedSecret[] = [{ name: "OLD_KEY", kind: "variable", updatedAt: 1, use: "auto" }];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		calls.push({ cmd, params });
		if (cmd === "secrets.list") return stored;
		if (cmd === "secrets.services") return services;
		if (cmd === "computer.cookies.sources") return { sources: [], blocked: [] };
		if (cmd === "computer.cookies.holders") return [];
		return { ...stored[0], usedWith: github };
	}) as typeof wire.command);
	const container = document.createElement("div"); document.body.append(container);
	const root = createRoot(container);
	const type = async (field: HTMLInputElement, text: string) => act(async () => {
		Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!.call(field, text);
		field.dispatchEvent(new window.Event("input", { bubbles: true }));
	});
	const button = (label: string) => Array.from(container.querySelectorAll("button")).find((b) => b.textContent === label)!;
	const chip = (label: string) => Array.from(container.querySelectorAll<HTMLButtonElement>('fieldset[aria-label="Used with"] button')).find((b) => b.textContent === label)!;
	try {
		await act(async () => { root.render(<SecretsSection />); });
		await act(async () => button("Add a secret").click());
		await type(container.querySelector<HTMLInputElement>("#secret-name")!, "GITHUB_TOKEN");
		await type(container.querySelector<HTMLInputElement>("#secret-value")!, "ghp_notarealtoken0001");
		expect(chip("GitHub").getAttribute("aria-pressed")).toBe("true");
		expect(container.textContent).toContain("Picked from how the key starts.");
		await act(async () => button("Store").click());
		expect(calls.find((call) => call.cmd === "secrets.set")?.params).toEqual({ name: "GITHUB_TOKEN", value: "ghp_notarealtoken0001", usedWith: { service: "github" } });

		// A stored key is given its Used with without its value typed again.
		await act(async () => button("Used with").click());
		await act(async () => chip("Other").click());
		await type(container.querySelector<HTMLInputElement>("#secret-host")!, "api.example.com");
		await act(async () => button("Save").click());
		expect(calls.at(-1)).toEqual({ cmd: "secrets.used_with.set", params: { name: "OLD_KEY", usedWith: { service: "other", host: "api.example.com" } } });
	} finally { await act(async () => root.unmount()); command.mockRestore(); await teardown(); }
});
