import { expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import type { CapabilityOptions } from "../src/generated/contract";

test("ChatGPT images require a picker selection, Automatic clears it, and with nothing to hear you Voice points at the free models", async () => {
	const dom = new Window();
	const restores: (() => void)[] = [];
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, IS_REACT_ACT_ENVIRONMENT: true })) {
		const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
		restores.push(() => {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		});
	}
	// React probes event support at module load; install the DOM before loading it.
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { UseFor } = await import("../src/components/UseFor");
	const { wire } = await import("../src/wire");
	const calls: { cmd: string; params: unknown }[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		calls.push({ cmd, params });
		return {};
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const provider = { providerId: "openai-codex", providerName: "Codex (ChatGPT subscription)" };
	const options: CapabilityOptions = {
		images: { options: [{ ...provider, models: [{ id: "gpt-image-2" }] }] },
		stt: { options: [] },
		tts: { options: [] },
		dispatcher: { options: [] },
		spending: { dayUsd: 2, monthUsd: 20, spentDayUsd: 0, spentMonthUsd: 0 },
	};
	let changes = 0;
	const render = () => act(async () => { root.render(<UseFor options={options} voice={undefined} onChanged={() => { changes += 1; }} />); });
	const pick = async (name: string) => {
		await act(async () => { (container.querySelector('[aria-label="Model for images"]') as HTMLButtonElement).click(); });
		expect(document.body.textContent).toContain("Codex (ChatGPT subscription)");
		const item = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((node) => node.textContent === name);
		expect(item).toBeDefined();
		await act(async () => { item!.click(); });
	};
	try {
		await render();
		expect(calls).toEqual([]);
		expect(container.textContent).toContain("Choose a provider for images");
		await pick("gpt-image-2");
		expect(calls).toEqual([{ cmd: "settings.update", params: { patch: { images: { provider: "openai-codex", model: "gpt-image-2" } } } }]);
		options.images.selected = { ...provider, modelId: "gpt-image-2" };
		await render();
		expect(container.textContent).toContain("Codex (ChatGPT subscription) · gpt-image-2");
		await pick("Automatic");
		expect(calls.at(-1)).toEqual({ cmd: "settings.update", params: { patch: { images: null } } });
		expect(changes).toBe(2);
		// Nothing hears yet, so the folded free models are pointed at; once the desk hears, the hint goes.
		expect(container.textContent).toContain("To talk without a key, download a free speech model.");
		options.stt.automatic = { providerId: "local", providerName: "On the desk" };
		await render();
		expect(container.textContent).not.toContain("download a free speech model");
	} finally {
		await act(async () => { root.unmount(); });
		command.mockRestore();
		for (const restore of restores.reverse()) restore();
		await dom.happyDOM.close();
	}
});
