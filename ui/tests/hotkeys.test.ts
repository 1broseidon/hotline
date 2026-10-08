import { describe, expect, test } from "bun:test";

// hotkeys.ts asks the shell's globals which system it is on when it loads; outside the shell there are none.
if (typeof window === "undefined") Object.assign(globalThis, { window: {} });
const { HotkeyRegistrar, hotkeyFromPress, hotkeyLabel } = await import("../src/hotkeys");
type ShortcutPlugin = import("../src/hotkeys").ShortcutPlugin;

const press = (code: string, held: { ctrl?: boolean; alt?: boolean; shift?: boolean; meta?: boolean } = {}) => ({
	code,
	ctrlKey: held.ctrl ?? false,
	altKey: held.alt ?? false,
	shiftKey: held.shift ?? false,
	metaKey: held.meta ?? false,
});

/** A system that takes any keys but those another app holds. */
function fakeSystem(taken: string[] = []) {
	const calls: string[] = [];
	const handlers = new Map<string, () => void>();
	const plugin: ShortcutPlugin = {
		register: async (accelerator, onPress) => {
			calls.push(`register ${accelerator}`);
			if (taken.includes(accelerator)) throw "FailedToRegister";
			handlers.set(accelerator, onPress);
		},
		unregister: async (accelerator) => {
			calls.push(`unregister ${accelerator}`);
			handlers.delete(accelerator);
		},
		unregisterAll: async () => {
			calls.push("unregister all");
			handlers.clear();
		},
	};
	return { plugin, calls, press: (accelerator: string) => handlers.get(accelerator)?.() };
}

describe("global shortcuts", () => {
	test("the recorder binds a modifier and a key, and waits while only modifiers are held", () => {
		expect(hotkeyFromPress(press("KeyH", { ctrl: true, alt: true }))).toBe("Control+Alt+KeyH");
		expect(hotkeyFromPress(press("Digit5", { meta: true, shift: true }))).toBe("Shift+Super+Digit5");
		expect(hotkeyFromPress(press("AltLeft", { alt: true }))).toBeNull();
		expect(hotkeyFromPress(press("KeyH"))).toBeNull();
		// Shift and a letter is typing.
		expect(hotkeyFromPress(press("KeyH", { shift: true }))).toBeNull();
		// A key the system cannot bind.
		expect(hotkeyFromPress(press("Lang1", { ctrl: true }))).toBeNull();
	});

	test("keys read the way each system writes them", () => {
		expect(hotkeyLabel("Control+Alt+KeyH", true)).toBe("⌃⌥H");
		expect(hotkeyLabel("Control+Alt+KeyH", false)).toBe("Ctrl+Alt+H");
		expect(hotkeyLabel("Shift+Super+Comma", true)).toBe("⇧⌘,");
		expect(hotkeyLabel("", true)).toBe("");
		expect(hotkeyLabel("Nonsense", false)).toBe("");
	});

	test("lets the last page's shortcuts go, then registers, re-registers on change and lets go on removal", async () => {
		const pressed: string[] = [];
		const system = fakeSystem();
		const registrar = new HotkeyRegistrar(system.plugin, (id) => pressed.push(id));
		await registrar.sync({ dictate: "Control+Alt+KeyH" });
		system.press("Control+Alt+KeyH");
		expect(pressed).toEqual(["dictate"]);

		await registrar.sync({ dictate: "Control+Alt+KeyJ", call: "Control+Alt+KeyK" });
		await registrar.sync({ dictate: "Control+Alt+KeyJ" });
		expect(system.calls).toEqual([
			"unregister all",
			"register Control+Alt+KeyH",
			"unregister Control+Alt+KeyH",
			"register Control+Alt+KeyJ",
			"register Control+Alt+KeyK",
			"unregister Control+Alt+KeyK",
		]);
		system.press("Control+Alt+KeyH");
		system.press("Control+Alt+KeyK");
		system.press("Control+Alt+KeyJ");
		expect(pressed).toEqual(["dictate", "dictate"]);
	});

	test("keys the system refused are said, asked for again, and forgotten when turned off", async () => {
		const system = fakeSystem(["Control+Alt+KeyH"]);
		const registrar = new HotkeyRegistrar(system.plugin, () => {});
		await registrar.sync({ dictate: "Control+Alt+KeyH" });
		expect(registrar.refused.dictate).toContain("Another app may be using it");
		await registrar.sync({ dictate: "Control+Alt+KeyH" });
		expect(system.calls.filter((call) => call === "register Control+Alt+KeyH")).toHaveLength(2);
		await registrar.sync({});
		expect(registrar.refused.dictate).toBeUndefined();
	});
});
