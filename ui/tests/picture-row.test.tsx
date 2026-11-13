import { expect, spyOn, test } from "bun:test";
import type { Persona } from "../src/generated/contract";
import { installDom } from "./dom";

const persona = {
	id: "ada",
	name: "Ada",
	goal: "",
	backendId: "hotline",
	cwd: "/work",
	mcpPolicy: { mode: "all", serverIds: [] },
	skillPolicy: { mode: "all", names: [] },
	backgroundWork: false,
	allowedSenders: [],
	sessionCheckpoints: [],
	createdAt: 0,
	updatedAt: 0,
} as unknown as Persona;

test("The Picture row offers the desk's drawings as images, picks one, and draws more", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { PictureRow } = await import("../src/components/teammate/PictureRow");
	const { wire } = await import("../src/wire");
	const sent: [string, unknown][] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: { round?: number }) => {
		sent.push([cmd, params]);
		if (cmd === "avatar.candidates") {
			const seed = `ada:${params.round}`;
			return [
				{ style: "lorelei", seed, svg: `<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>` },
				{ style: "open-peeps", seed, svg: `<svg xmlns="http://www.w3.org/2000/svg"/>` },
			];
		}
		return { hash: "a".repeat(64), by: "person", updatedAt: "now" };
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => {
			root.render(<PictureRow persona={persona} disabled={false} onRefusal={() => {}} />);
		});
		expect(container.textContent).toContain("Its initial");
		expect(sent).toEqual([]);

		await act(async () => {
			container.querySelector<HTMLButtonElement>("button[aria-expanded]")!.click();
		});
		const choices = () => [...container.querySelectorAll<HTMLButtonElement>(".picture-choice")];
		expect(choices().map((one) => one.title)).toEqual(["Lorelei", "Open Peeps"]);
		// The drawing is an image's source, never markup in the page.
		expect(container.querySelector("script")).toBeNull();
		expect(choices()[0]!.querySelector("img")!.getAttribute("src")).toStartWith("data:image/svg+xml,%3Csvg");

		await act(async () => {
			choices()[1]!.click();
		});
		expect(sent.at(-1)).toEqual(["avatar.pick", { personaId: "ada", style: "open-peeps", seed: "ada:0" }]);

		const more = [...container.querySelectorAll("button")].find((one) => one.textContent === "More")!;
		await act(async () => {
			more.click();
		});
		expect(sent.at(-1)).toEqual(["avatar.candidates", { personaId: "ada", round: 1 }]);
	} finally {
		await act(async () => {
			root.unmount();
		});
		command.mockRestore();
		await teardown();
	}
});
