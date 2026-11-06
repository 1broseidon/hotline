import { expect, spyOn, test } from "bun:test";
import { installDom } from "./dom";

/** The person's "Always allow" answers sit under Permissions, a row each, and Remove takes one back. */
test("a teammate's local addresses are listed under Permissions, folded, and Remove takes one back", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { LocalAddresses } = await import("../src/components/teammate/LocalAddresses");
	const { wire } = await import("../src/wire");
	let origins = ["localhost:5173", "nas.local:8080"];
	const asked: unknown[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: { origin?: string }) => {
		asked.push({ cmd, params });
		if (cmd === "browser.local.remove") origins = origins.filter((one) => one !== params.origin);
		return origins;
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => root.render(<LocalAddresses personaId="kit" disabled={false} />));
		expect(container.textContent).toContain("Local addresses · 2");
		expect(container.textContent).not.toContain("localhost:5173");
		await act(async () => container.querySelector<HTMLButtonElement>("[aria-expanded]")!.click());
		expect(container.textContent).toContain("localhost:5173");
		const remove = [...container.querySelectorAll("button")].find((button) => button.textContent === "Remove")!;
		await act(async () => remove.click());
		expect(asked.at(-1)).toEqual({ cmd: "browser.local.remove", params: { personaId: "kit", origin: "localhost:5173" } });
		expect(container.textContent).not.toContain("localhost:5173");
		expect(container.textContent).toContain("nas.local:8080");
		origins = [];
		await act(async () => root.render(<LocalAddresses personaId="ada" disabled={false} />));
		expect(container.textContent, "a teammate with none shows nothing").toBe("");
	} finally {
		await act(async () => root.unmount());
		command.mockRestore();
		await teardown();
	}
});
