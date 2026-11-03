import { afterEach, beforeEach, expect, mock, spyOn, test } from "bun:test";
import { act } from "react";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

// React decides at load whether this DOM has `input` events, so it loads with
// one in place; on its own, with no DOM yet, it would never see the typing.
const boot = installDom();
// The section reads the room only to know when the model list changed; no desk is needed for the name.
const real = await import("../src/room");
mock.module("../src/room", () => ({ ...real, useModelsRevision: () => 0 }));
const { createRoot } = await import("react-dom/client");
const { GeneralSection } = await import("../src/components/settings/General");
const { wire } = await import("../src/wire");
await boot.teardown();

let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let saved: string[];

beforeEach(() => {
	({ teardown } = installDom());
	spyOn(wire, "command").mockImplementation((async () => []) as typeof wire.command);
	saved = [];
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await teardown();
});

async function mount(personName: string) {
	await act(async () => {
		root.render(
			<GeneralSection
				personName={personName}
				onPersonName={(name) => saved.push(name)}
				idleHours={24}
				defaultBackendId="hotline"
				defaultModelId={null}
				onIdleHours={() => {}}
				onBackend={() => {}}
				onDefaultModel={() => {}}
				onProviders={undefined}
			/>,
		);
	});
}

function type(field: HTMLInputElement, text: string) {
	const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(field), "value")!.set!;
	setter.call(field, text);
	field.dispatchEvent(new window.Event("input", { bubbles: true }));
}

test("a name typed and left by closing Settings, without a blur, is saved", async () => {
	await mount("");
	const field = container.querySelector<HTMLInputElement>("#setting-person-name")!;
	await act(async () => type(field, "  Ada "));
	expect(saved).toEqual([]);
	await act(async () => root.unmount());
	expect(saved).toEqual(["Ada"]);
});

test("leaving with the name unchanged saves nothing", async () => {
	await mount("Ada");
	await act(async () => root.unmount());
	expect(saved).toEqual([]);
});
