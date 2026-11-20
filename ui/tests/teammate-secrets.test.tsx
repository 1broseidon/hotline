import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Persona, SessionInfo, SharedSecret } from "../src/generated/contract";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Teammate } = await import("../src/components/Teammate");
const { wire } = await import("../src/wire");
const { replaceDesks } = await import("../src/desks");

const persona: Persona = {
	id: "ada", name: "Ada", goal: "", backendId: "hotline", cwd: "/work/ada",
	mcpPolicy: { mode: "none", serverIds: [] }, skillPolicy: { mode: "none", names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 1, updatedAt: 1,
	computer: { enabled: true },
	secrets: ["GITHUB_TOKEN"],
};
const session: SessionInfo = {
	personaId: "ada", state: "idle", contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
	capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
};
const github = { service: "github", origins: ["https://api.github.com", "https://github.com", "https://uploads.github.com"], header: "Authorization", schemes: ["Bearer", "token", "Basic"] };
const stored: SharedSecret[] = [
	{ name: "GITHUB_TOKEN", kind: "variable", use: "auto", updatedAt: 1, usedWith: github },
	{ name: "NPM_TOKEN", kind: "variable", use: "auto", updatedAt: 1 },
];
const services = [{ id: "github", name: "GitHub", prefixes: ["ghp_"], ...github, refuse: [] }];

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Call = { cmd: string; params: Record<string, unknown> };
let calls: Call[];
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let teardown: () => Promise<void>;
let status: Record<string, unknown>;

beforeEach(async () => {
	restores = [];
	({ teardown } = installDom({ ResizeObserver: NoResize }));
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	calls = [];
	status = { state: "absent" };
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push({ cmd, params });
		if (cmd === "persona.update") return persona;
		if (cmd === "secrets.list") return stored;
		if (cmd === "secrets.services") return services;
		if (cmd === "computer.status") return status;
		if (cmd === "computer.migration.answer") return null;
		throw new Error(`Not in this test: ${cmd}`);
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	replaceDesks([]);
	for (const restore of restores.reverse()) restore();
	await teardown();
});

async function mount(who: Persona) {
	await act(async () => {
		root.render(<Teammate persona={who} session={session} jobs={[]} roster={[]} focusSchedules={false} onClose={() => {}} onDeleted={() => {}} />);
	});
}

const section = (heading: string) => [...container.querySelectorAll("section")].find((node) => node.querySelector("h3")?.textContent === heading);

test("Secrets is the teammate's own section, not a fold under Computer, and a tick saves on the teammate", async () => {
	await mount(persona);
	const computer = section("Computer");
	const secrets = section("Secrets");
	if (!computer || !secrets) throw new Error("No Computer or Secrets section");
	expect(computer.textContent).not.toContain("Secrets");
	expect(computer.contains(secrets)).toBe(false);
	expect(secrets.textContent).toContain("Logins and passkeys · 1 given");

	const fold = secrets.querySelector<HTMLButtonElement>("[aria-expanded]");
	await act(async () => fold!.click());
	const npm = [...secrets.querySelectorAll("label")].find((node) => node.textContent?.includes("NPM_TOKEN"))?.querySelector("input");
	if (!npm) throw new Error("No NPM_TOKEN tick");
	await act(async () => npm.click());
	const give = [...secrets.querySelectorAll("button")].find((node) => node.textContent === "Give");
	await act(async () => give!.click());
	expect(calls.at(-1)).toEqual({ cmd: "persona.update", params: { id: "ada", patch: { secretGrants: [{ name: "GITHUB_TOKEN" }, { name: "NPM_TOKEN", readable: true }] } } });
});

test("a plain variable is granted only as readable", async () => {
	await mount({ ...persona, secrets: [], secretGrants: [] });
	const secrets = section("Secrets")!;
	await act(async () => secrets.querySelector<HTMLButtonElement>("[aria-expanded]")!.click());
	const tick = (name: string) => [...secrets.querySelectorAll("label")].find((node) => node.textContent?.includes(name))!.querySelector("input")!;
	const button = (label: string) => [...secrets.querySelectorAll("button")].find((node) => node.textContent === label);

	// A key Used with a service is given at once, and protected.
	await act(async () => tick("GITHUB_TOKEN").click());
	expect(calls.at(-1)?.params).toEqual({ id: "ada", patch: { secretGrants: [{ name: "GITHUB_TOKEN" }] } });
	expect(secrets.textContent).toContain("Key used with GitHub");

	// A plain variable asks first, in the words its grant will carry.
	const before = calls.length;
	await act(async () => tick("NPM_TOKEN").click());
	expect(calls.length, "nothing is granted until the person says so").toBe(before);
	expect(secrets.textContent).toContain("Readable by this teammate's programs");
	await act(async () => button("Cancel")!.click());
	expect(calls.length).toBe(before);
	expect(button("Give")).toBeUndefined();
	await act(async () => tick("NPM_TOKEN").click());
	await act(async () => button("Give")!.click());
	expect(calls.at(-1)?.params).toEqual({ id: "ada", patch: { secretGrants: [{ name: "NPM_TOKEN", readable: true }] } });

	// Granted readable, the row says so.
	await act(async () => root.render(null));
	await mount({ ...persona, secrets: ["NPM_TOKEN"], secretGrants: [{ name: "NPM_TOKEN", readable: true }] });
	const again = section("Secrets")!;
	await act(async () => again.querySelector<HTMLButtonElement>("[aria-expanded]")!.click());
	const row = [...again.querySelectorAll("label")].find((node) => node.textContent?.includes("NPM_TOKEN"))!;
	expect(row.textContent).toContain("Readable by this teammate's programs");
});

test("a teammate with a computer is offered its computer and its shell for a variable", async () => {
	await mount({ ...persona, secretGrants: [{ name: "GITHUB_TOKEN" }] });
	const secrets = section("Secrets")!;
	await act(async () => secrets.querySelector<HTMLButtonElement>("[aria-expanded]")!.click());
	const places = secrets.querySelector('fieldset[aria-label="Where GITHUB_TOKEN goes"]');
	expect(places?.textContent).toBe("ComputerShell");
});

test("a grant with no place named is its computer's, and adding the shell names both", async () => {
	const { TeammateSecrets } = await import("../src/components/Secrets");
	let saved: unknown = null;
	await act(async () => {
		root.render(<TeammateSecrets grants={[{ name: "GITHUB_TOKEN" }]} places={["computer", "desk"]} asPerson={false} disabled={false} onChange={(grants) => { saved = grants; }} />);
	});
	const places = container.querySelector('fieldset[aria-label="Where GITHUB_TOKEN goes"]');
	if (!places) throw new Error("No surfaces choice");
	const chip = (label: string) => [...places.querySelectorAll("button")].find((node) => node.textContent === label)!;
	expect(chip("Computer").getAttribute("aria-pressed")).toBe("true");
	expect(chip("Shell").getAttribute("aria-pressed")).toBe("false");
	await act(async () => chip("Shell").click());
	expect(saved).toEqual([{ name: "GITHUB_TOKEN", on: ["computer", "desk"] }]);
	await act(async () => {
		root.render(<TeammateSecrets grants={[{ name: "GITHUB_TOKEN", on: ["computer", "desk"] }]} places={["computer", "desk"]} asPerson={false} disabled={false} onChange={(grants) => { saved = grants; }} />);
	});
	await act(async () => chip("Shell").click());
	expect(saved, "its computer alone is said by saying nothing").toEqual([{ name: "GITHUB_TOKEN" }]);
});

test("a whole-machine teammate is given every key readable, and every grant says so", async () => {
	await mount({ ...persona, reach: "machine", secrets: ["GITHUB_TOKEN"], secretGrants: [{ name: "GITHUB_TOKEN", readable: true }] });
	const secrets = section("Secrets")!;
	await act(async () => secrets.querySelector<HTMLButtonElement>("[aria-expanded]")!.click());
	const row = (name: string) => [...secrets.querySelectorAll("label")].find((node) => node.textContent?.includes(name))!;
	expect(row("GITHUB_TOKEN").textContent).toContain("Readable by this teammate's programs");
	await act(async () => root.render(null));
	await mount({ ...persona, backendId: "claude-code", secrets: [], secretGrants: [] });
	const harness = section("Secrets")!;
	await act(async () => harness.querySelector<HTMLButtonElement>("[aria-expanded]")!.click());
	const before = calls.length;
	const tick = [...harness.querySelectorAll("label")].find((node) => node.textContent?.includes("GITHUB_TOKEN"))!.querySelector("input")!;
	await act(async () => tick.click());
	expect(calls.length, "even a key Used with a service asks first").toBe(before);
	const give = [...harness.querySelectorAll("button")].find((node) => node.textContent === "Give")!;
	await act(async () => give.click());
	expect(calls.at(-1)?.params).toEqual({ id: "ada", patch: { secretGrants: [{ name: "GITHUB_TOKEN", readable: true }] } });
});

test("one notice says keys wait for an update, with Update when there is one", async () => {
	status = { state: "running", release: "0.11.0", available: "0.13.0", protection: "needsUpdate" };
	await mount(persona);
	const computer = section("Computer")!;
	expect(computer.textContent).toContain("Keys waiting · until 0.13.0");
	expect(computer.textContent?.match(/Keys waiting/g)?.length).toBe(1);
	expect([...computer.querySelectorAll("button")].some((node) => node.textContent === "Update")).toBe(true);
	await act(async () => root.render(null));
	status = { state: "running", release: "0.13.0", protection: "protected" };
	await mount(persona);
	expect(section("Computer")?.textContent).not.toContain("Keys waiting");
});

test("a teammate with no computer gives a variable to its shell, and is told so", async () => {
	await mount({ ...persona, computer: { enabled: false }, secrets: [], secretGrants: [] });
	const secrets = section("Secrets")!;
	await act(async () => secrets.querySelector<HTMLButtonElement>("[aria-expanded]")!.click());
	expect(secrets.querySelector('fieldset[aria-label^="Where"]')).toBeNull();
	const tick = [...secrets.querySelectorAll("label")].find((node) => node.textContent?.includes("GITHUB_TOKEN"))!.querySelector("input")!;
	await act(async () => tick.click());
	expect(calls.at(-1)?.params).toEqual({ id: "ada", patch: { secretGrants: [{ name: "GITHUB_TOKEN", on: ["desk"] }] } });
	await act(async () => root.render(null));
	await mount({ ...persona, computer: { enabled: false } });
	expect(section("Secrets")?.textContent).toContain("Its shell gets the variables.");
	await act(async () => root.render(null));
	await mount(persona);
	expect(section("Secrets")?.textContent).not.toContain("Its shell gets the variables.");
});

test("on the Mac app, a teammate with no computer arms its desk browser for a passkey under Secrets", async () => {
	const shell = window as unknown as { __hotlineDesk?: { platform: string } };
	shell.__hotlineDesk = { platform: "macos" };
	restores.push(() => delete shell.__hotlineDesk);
	await mount({ ...persona, computer: { enabled: false } });
	const secrets = section("Secrets");
	if (!secrets) throw new Error("No Secrets section");
	expect(secrets.textContent).toContain("signs in with these passkeys");
	expect(secrets.textContent).not.toContain("no browser for a login");
	const add = [...secrets.querySelectorAll("button")].find((node) => node.textContent?.includes("Add a passkey"));
	if (!add) throw new Error("No Add a passkey");
	await act(async () => add.click());
	expect(secrets.textContent).toContain("Arm the browser");
	expect(secrets.textContent).not.toContain("starts Ada's computer");

	await act(async () => root.render(null));
	await mount(persona);
	expect(
		[...(section("Secrets")?.querySelectorAll("button") ?? [])].some((node) => node.textContent?.includes("Add a passkey")),
		"a teammate with a computer arms its computer's browser under Secrets too",
	).toBe(true);
});

test("on Linux and Windows the desk browser signs with no passkey, so none is armed there", async () => {
	for (const platform of ["linux", "windows"]) {
		const shell = window as unknown as { __hotlineDesk?: { platform: string } };
		shell.__hotlineDesk = { platform };
		await mount({ ...persona, computer: { enabled: false } });
		const secrets = section("Secrets")!;
		expect([...secrets.querySelectorAll("button")].some((node) => node.textContent?.includes("Add a passkey")), platform).toBe(false);
		expect(secrets.textContent).toContain("Its browser types these logins.");
		expect(secrets.textContent).not.toContain("passkeys.");
		await act(async () => root.render(null));
		delete shell.__hotlineDesk;
	}
});

test("after the update the one notice asks what becomes of the old snapshots, and sends the answer", async () => {
	status = { state: "running", release: "0.13.0", protection: "protected", migration: { snapshots: [{ name: "before", bytes: 10 }, { name: "older", bytes: 20 }] } };
	await mount(persona);
	const computer = section("Computer")!;
	expect(computer.textContent).toContain("Old snapshots · 2 on hold");
	expect(computer.textContent).toContain("Passwords and keys this computer had before may have been read: change them.");
	const labels = [...computer.querySelectorAll("button")].map((node) => node.textContent);
	expect(labels).toEqual(expect.arrayContaining(["Clean", "Delete", "Keep for now"]));
	status = { state: "running", release: "0.13.0", protection: "protected" };
	const keep = [...computer.querySelectorAll("button")].find((node) => node.textContent === "Keep for now")!;
	await act(async () => keep.click());
	expect(calls.find((call) => call.cmd === "computer.migration.answer")?.params).toEqual({ personaId: "ada", snapshots: "keep" });
	expect(section("Computer")?.textContent).not.toContain("Old snapshots");
});

const TRUSTS =
	"Granting a secret to this computer trusts the teammate and its programs: they can read and send out granted variables, passwords, sign-in codes and sessions, and can use a granted passkey to sign in.";
const login: SharedSecret = { name: "GITHUB_LOGIN", kind: "login", use: "auto", updatedAt: 1, sites: ["https://github.com"], username: "ada" };
const passkey: SharedSecret = { name: "GITHUB_PASSKEY", kind: "passkey", use: "auto", updatedAt: 1, rpId: "github.com" };

async function openSecrets(who: Persona) {
	await mount(who);
	const secrets = section("Secrets")!;
	await act(async () => secrets.querySelector<HTMLButtonElement>("[aria-expanded]")!.click());
	return secrets;
}

const rowOf = (within: Element, name: string) => [...within.querySelectorAll("label")].find((node) => node.textContent?.includes(name))!;

test("on a computer that has not shown it keeps them from its programs, logins and passkeys are readable, said above the list and on the row, and given on the tick", async () => {
	stored.push(login, passkey);
	restores.push(() => stored.splice(2));
	for (const protection of [undefined, "legacy", "needsUpdate"]) {
		status = { state: "running", ...(protection !== undefined ? { protection } : {}) };
		const secrets = await openSecrets({ ...persona, secrets: [], secretGrants: [] });
		expect(secrets.textContent, String(protection)).toContain(TRUSTS);
		// Told, not asked: the line above the list says it, and the tick gives
		// it as it did before, with no second press on an older computer.
		await act(async () => rowOf(secrets, "GITHUB_LOGIN").querySelector("input")!.click());
		expect([...secrets.querySelectorAll("button")].some((node) => node.textContent === "Give")).toBe(false);
		expect(calls.at(-1)?.params).toEqual({ id: "ada", patch: { secretGrants: [{ name: "GITHUB_LOGIN" }] } });
		await act(async () => root.render(null));
		const granted = await openSecrets({ ...persona, secrets: ["GITHUB_LOGIN", "GITHUB_PASSKEY"], secretGrants: [{ name: "GITHUB_LOGIN" }, { name: "GITHUB_PASSKEY" }] });
		expect(rowOf(granted, "GITHUB_LOGIN").textContent).toContain("Readable by this teammate's programs");
		expect(rowOf(granted, "GITHUB_PASSKEY").textContent).toContain("Readable by this teammate's programs");
		await act(async () => root.render(null));
	}
	status = { state: "stopped", protection: "protected" };
	const shown = await openSecrets({ ...persona, secrets: ["GITHUB_LOGIN"], secretGrants: [{ name: "GITHUB_LOGIN" }] });
	expect(shown.textContent).not.toContain(TRUSTS);
	expect(rowOf(shown, "GITHUB_LOGIN").textContent).not.toContain("Readable by this teammate's programs");
});

test("a login or passkey for this machine, and a key used with Other, are readable wherever they go", async () => {
	const local: SharedSecret = { name: "DEV_LOGIN", kind: "login", use: "auto", updatedAt: 1, sites: ["http://127.0.0.1:3000"], username: "ada" };
	const devPasskey: SharedSecret = { name: "DEV_PASSKEY", kind: "passkey", use: "auto", updatedAt: 1, rpId: "app.localhost" };
	const other: SharedSecret = { name: "API_KEY", kind: "variable", use: "auto", updatedAt: 1, usedWith: { service: "other", origins: ["https://api.example.com"], header: "Authorization", schemes: ["Bearer"] } };
	stored.push(local, devPasskey, other);
	restores.push(() => stored.splice(2));
	status = { state: "running", protection: "protected" };
	const secrets = await openSecrets({ ...persona, secrets: ["DEV_LOGIN", "DEV_PASSKEY", "API_KEY", "GITHUB_TOKEN"], secretGrants: [{ name: "DEV_LOGIN" }, { name: "DEV_PASSKEY" }, { name: "API_KEY" }, { name: "GITHUB_TOKEN" }] });
	for (const name of ["DEV_LOGIN", "DEV_PASSKEY", "API_KEY"]) expect(rowOf(secrets, name).textContent, name).toContain("Readable by this teammate's programs");
	expect(rowOf(secrets, "GITHUB_TOKEN").textContent).not.toContain("Readable by this teammate's programs");
	await act(async () => root.render(null));

	// Ticked, a key used with Other asks first, and stays a key.
	const ticking = await openSecrets({ ...persona, secrets: [], secretGrants: [] });
	const before = calls.length;
	await act(async () => rowOf(ticking, "API_KEY").querySelector("input")!.click());
	expect(calls.length).toBe(before);
	await act(async () => [...ticking.querySelectorAll("button")].find((node) => node.textContent === "Give")!.click());
	expect(calls.at(-1)?.params).toEqual({ id: "ada", patch: { secretGrants: [{ name: "API_KEY" }] } });
});

test("a site on this machine is one whatever its spelling, and a public one is not", async () => {
	const { loopbackSite } = await import("../src/components/Secrets");
	for (const site of ["http://localhost:8080", "localhost", "https://app.localhost", "http://127.0.0.1:3000", "http://127.9.9.9", "http://[::1]:8080", "http://[::ffff:127.0.0.1]"]) expect(loopbackSite(site), site).toBe(true);
	for (const site of ["https://github.com", "https://localhost.example.com", "http://128.0.0.1", "http://10.0.0.1", "not a site"]) expect(loopbackSite(site), site).toBe(false);
});
