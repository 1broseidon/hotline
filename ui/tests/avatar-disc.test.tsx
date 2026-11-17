import { expect, test } from "bun:test";
import type { FileChunk } from "../src/generated/contract";
import { installDom } from "./dom";

/** The start of a PNG: its signature, its header with `colourType`, an optional transparency chunk, then image data. */
function png(colourType: number, transparency = false): Uint8Array {
	const chunk = (kind: string, data: number[]) => {
		const length = data.length;
		return [(length >>> 24) & 255, (length >>> 16) & 255, (length >>> 8) & 255, length & 255, ...[...kind].map((letter) => letter.charCodeAt(0)), ...data, 0, 0, 0, 0];
	};
	return new Uint8Array([
		0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
		...chunk("IHDR", [0, 0, 2, 0, 0, 0, 2, 0, 8, colourType, 0, 0, 0]),
		...(transparency ? chunk("tRNS", [0, 0, 0, 0, 0, 0]) : []),
		...chunk("IDAT", [1, 2, 3]),
	]);
}

const chunkOf = (bytes: Uint8Array): FileChunk => ({
	name: "a.png",
	mimeType: "image/png",
	size: bytes.length,
	offset: 0,
	data: btoa(String.fromCharCode(...bytes)),
});

test("Only a PNG with no transparency at all is read as filling its circle", async () => {
	Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
	const { opaquePng } = await import("../src/avatars");
	expect(opaquePng(png(2))).toBe(true);
	expect(opaquePng(png(0))).toBe(true);
	expect(opaquePng(png(6))).toBe(false);
	expect(opaquePng(png(4))).toBe(false);
	expect(opaquePng(png(2, true))).toBe(false);
	expect(opaquePng(new Uint8Array([1, 2, 3]))).toBe(false);
	expect(opaquePng(undefined)).toBe(false);
});

test("A face that fills its circle has no colour disc behind it; a transparent one keeps it", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { Avatar } = await import("../src/ui/Avatar");
	const { forgetAvatars } = await import("../src/avatars");
	forgetAvatars();
	const pictures: Record<string, Uint8Array> = { opaque: png(2), clear: png(6) };
	const read = async (_personaId: string, hash: string) => chunkOf(pictures[hash]!);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		const face = async (hash?: string) => {
			await act(async () => {
				root.render(<Avatar id="ada" name="Ada" size={28} {...(hash !== undefined ? { hash } : {})} read={read} />);
			});
			await act(async () => {
				await new Promise((resolve) => setTimeout(resolve, 0));
			});
			return container.querySelector<HTMLElement>(".avatar")!;
		};
		const initial = await face();
		expect(initial.textContent).toBe("A");
		expect(initial.hasAttribute("data-fills")).toBe(false);
		expect(initial.style.getPropertyValue("--face")).toContain("oklch(");

		const opaque = await face("opaque");
		expect(opaque.querySelector("img")).not.toBeNull();
		expect(opaque.hasAttribute("data-fills")).toBe(true);

		const clear = await face("clear");
		expect(clear.querySelector("img")).not.toBeNull();
		expect(clear.hasAttribute("data-fills")).toBe(false);
	} finally {
		await act(async () => {
			root.unmount();
		});
		await teardown();
	}
});
