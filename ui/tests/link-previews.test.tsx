import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { LinkPreview } from "../src/generated/contract";
import type { Handlers } from "../src/wire";
import { installDom } from "./dom";

const observer = { ResizeObserver: class { observe() {} unobserve() {} disconnect() {} } };
const boot = installDom(observer);
const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { LinkCard } = await import("../src/components/LinkCard");
const { Favicon, Favicons } = await import("../src/components/Favicon");
const { Settings } = await import("../src/components/Settings");
const { replaceDesks } = await import("../src/desks");
const { fetchPreview, forgetPreviews } = await import("../src/linkPreview");
const { Wire, wire } = await import("../src/wire");
await boot.teardown();

const PAGE = "https://qdrant.tech/documentation/quickstart/";
const PICTURE = "data:image/jpeg;base64,/9j/4AAQ";
const ICON = "data:image/x-icon;base64,AAABAA==";

let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let desks = 0;
/** The room's `linkPreviews` setting as the desk has it; undefined is never set. */
let previews: boolean | undefined;
let card: LinkPreview | null;
let icon: string | null;
let asked: { cmd: string; params: unknown }[];
let room: Handlers<unknown, unknown> | undefined;

beforeEach(() => {
	({ teardown } = installDom(observer));
	forgetPreviews();
	previews = undefined;
	card = null;
	icon = null;
	asked = [];
	spyOn(Wire.prototype, "subscribe").mockImplementation(((target: unknown, handlers: Handlers<unknown, unknown>) => {
		if (target === "room") {
			room = handlers;
			handlers.snapshot(previews === undefined ? [] : [{ kind: "setting", id: "linkPreviews", value: previews }]);
		}
		return () => {};
	}) as InstanceType<typeof Wire>["subscribe"]);
	spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		asked.push({ cmd, params });
		if (cmd === "link.preview") return card;
		if (cmd === "link.icon") return icon;
		if (cmd === "settings.update") {
			const value = (params as { patch: { linkPreviews: boolean | null } }).patch.linkPreviews;
			room?.event({ kind: "setting", id: "linkPreviews", value: value ?? true });
			return {};
		}
		return [];
	}) as typeof wire.command);
	// A desk of its own for each test, so each starts from its own room.
	replaceDesks([{ id: `link-previews-${++desks}`, kind: "local", name: "Test", origin: "ws://localhost", token: "test" }]);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await teardown();
});

async function draw(node: React.ReactNode) {
	await act(async () => root.render(node));
}

const linkAsks = () => asked.filter((one) => one.cmd.startsWith("link."));
const sources = () => [...container.querySelectorAll("img")].map((img) => img.getAttribute("src"));

test("a card without a picture draws no tile: site, title and description only", async () => {
	card = { url: PAGE, site: "qdrant.tech · Documentation", title: "Local Quickstart", description: "Run Qdrant locally with Docker." };
	await fetchPreview(PAGE);
	await draw(<LinkCard url={PAGE} />);
	const shown = container.querySelector(".link-card")!;
	expect([...shown.querySelectorAll(".link-card-words > span")].map((line) => [line.className, line.textContent])).toEqual([
		["link-card-site", "qdrant.tech · Documentation"],
		["link-card-title", "Local Quickstart"],
		["link-card-description", "Run Qdrant locally with Docker."],
	]);
	expect(shown.classList.contains("link-card-pictured")).toBe(false);
	expect(container.querySelector(".link-card-picture")).toBeNull();
	expect(sources()).toEqual([]);
});

test("a card's picture is drawn only from the bytes the desk handed over, never from the site", async () => {
	card = { url: PAGE, site: "qdrant.tech", title: "Local Quickstart", image: "https://qdrant.tech/og.png" };
	await fetchPreview(PAGE);
	await draw(<LinkCard url={PAGE} />);
	expect(sources()).toEqual([]);
	expect(container.querySelector(".link-card-pictured")).toBeNull();

	forgetPreviews();
	card = { url: PAGE, site: "qdrant.tech", title: "Local Quickstart", image: PICTURE };
	await fetchPreview(PAGE);
	await draw(<LinkCard key="again" url={PAGE} />);
	expect(sources()).toEqual([PICTURE]);
	expect(container.querySelector(".link-card-pictured")).not.toBeNull();
});

test("a site the desk has no icon for shows nothing: no tile, no initial, no +N alone", async () => {
	icon = null;
	await draw(
		<>
			<Favicon site="github.com" />
			<Favicons sites={["github.com", "example.com"]} total={4} />
		</>,
	);
	expect(container.innerHTML).toBe("");
	expect(linkAsks().map((one) => one.params)).toEqual([{ site: "github.com" }, { site: "example.com" }]);
});

test("an icon asked twice is asked of the desk once and drawn from its bytes", async () => {
	icon = ICON;
	await draw(
		<>
			<Favicon site="GitHub.com" />
			<Favicon site="github.com" size={14} />
		</>,
	);
	await draw(<Favicons sites={["github.com"]} total={3} />);
	expect(linkAsks()).toEqual([{ cmd: "link.icon", params: { site: "github.com" } }]);
	expect(sources()).toEqual([ICON]);
	expect(container.querySelector(".favicons-more")?.textContent).toBe("+2");
});

test("with link previews off there is no card, picture or icon, and the desk is asked for none", async () => {
	previews = false;
	card = { url: PAGE, site: "qdrant.tech", title: "Local Quickstart", image: PICTURE };
	icon = ICON;
	await draw(
		<>
			<LinkCard url={PAGE} always />
			<Favicon site="github.com" />
			<Favicons sites={["github.com"]} />
		</>,
	);
	// Past the card's settle, when it would have asked.
	await act(async () => new Promise((done) => setTimeout(done, 700)));
	expect(container.innerHTML).toBe("");
	expect(linkAsks()).toEqual([]);
});

test("Settings › General has a Link previews switch, on by default, that turns them off and back on", async () => {
	await draw(<Settings section="general" />);
	const toggle = () => container.querySelector<HTMLInputElement>('input[role="switch"][aria-label="Link previews"]')!;
	expect(toggle().checked).toBe(true);
	expect(toggle().closest("label")?.getAttribute("title")).toBe("Cards, pictures, icons");
	expect(toggle().closest("label")?.textContent).toBe("Link previews");
	await act(async () => toggle().click());
	expect(asked.filter((one) => one.cmd === "settings.update").map((one) => one.params)).toEqual([{ patch: { linkPreviews: false } }]);
	expect(toggle().checked).toBe(false);
	await act(async () => toggle().click());
	expect(asked.filter((one) => one.cmd === "settings.update").at(-1)?.params).toEqual({ patch: { linkPreviews: null } });
	expect(toggle().checked).toBe(true);
});
