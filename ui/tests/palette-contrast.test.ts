import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

// Reads both palettes straight from tokens.css, so a token change that drops a
// pairing below WCAG AA fails here before anyone has to see it on screen.

type Rgb = [number, number, number];

const css = readFileSync(new URL("../src/tokens.css", import.meta.url), "utf8");

function block(selector: string): Map<string, string> {
	const start = css.indexOf(`${selector} {`);
	if (start < 0) throw new Error(`no ${selector} block in tokens.css`);
	const body = css.slice(start, css.indexOf("\n}", start));
	const tokens = new Map<string, string>();
	for (const [, name, value] of body.matchAll(/--([a-z0-9-]+):\s*(oklch\([^)]*\))\s*;/g)) tokens.set(name!, value!);
	return tokens;
}

const dark = block(":root");
const light = new Map([...dark, ...block(':root[data-theme="light"]')]);

/** oklch(L% C h / a) to linear-free sRGB in 0..1, and its alpha. */
function parse(value: string): { rgb: Rgb; alpha: number } {
	const m = value.match(/oklch\(\s*([\d.]+)%\s+([\d.]+)\s+([\d.]+)\s*(?:\/\s*([\d.]+))?\s*\)/);
	if (!m) throw new Error(`not an oklch colour: ${value}`);
	const [L, C, h] = [Number(m[1]) / 100, Number(m[2]), (Number(m[3]) * Math.PI) / 180];
	const [a, b] = [C * Math.cos(h), C * Math.sin(h)];
	const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
	const mm = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
	const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
	const linear: Rgb = [
		4.0767416621 * l - 3.3077115913 * mm + 0.2309699292 * s,
		-1.2684380046 * l + 2.6097574011 * mm - 0.3413193965 * s,
		-0.0041960863 * l - 0.7034186147 * mm + 1.707614701 * s,
	];
	const gamma = (v: number) => {
		const c = Math.min(1, Math.max(0, v));
		return c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055;
	};
	return { rgb: linear.map(gamma) as Rgb, alpha: m[4] === undefined ? 1 : Number(m[4]) };
}

function colour(palette: Map<string, string>, name: string, under?: Rgb): Rgb {
	const value = palette.get(name);
	if (!value) throw new Error(`--${name} is not an oklch token`);
	const { rgb, alpha } = parse(value);
	if (alpha >= 1) return rgb;
	if (!under) throw new Error(`--${name} is translucent and needs a surface under it`);
	return rgb.map((v, i) => v * alpha + under[i]! * (1 - alpha)) as Rgb;
}

function luminance(rgb: Rgb): number {
	const [r, g, b] = rgb.map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
	return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}

function ratio(palette: Map<string, string>, ink: string, surface: string): number {
	const under = colour(palette, surface);
	const [x, y] = [luminance(colour(palette, ink, under)), luminance(under)].sort((m, n) => n - m);
	return (x! + 0.05) / (y! + 0.05);
}

const planes = ["well", "canvas", "raised", "pop"];

describe("light mode", () => {
	test("body, secondary and caption text hold 4.5:1 on every plane", () => {
		for (const ink of ["ink", "ink-2", "ink-3"])
			for (const plane of planes) expect({ ink, plane, ratio: ratio(light, ink, plane) >= 4.5 }).toEqual({ ink, plane, ratio: true });
	});

	test("ink-4's small timings and hints hold 4.5:1 where they sit", () => {
		for (const plane of ["canvas", "raised", "pop"]) expect(ratio(light, "ink-4", plane)).toBeGreaterThanOrEqual(4.5);
	});

	test("accent, warning and danger words hold 4.5:1 on the canvas and on a card", () => {
		for (const ink of ["accent-ink", "warn-ink", "danger"])
			for (const plane of ["canvas", "raised"]) expect(ratio(light, ink, plane)).toBeGreaterThanOrEqual(4.5);
	});

	test("words on the mint, the person's bubble and the other side's bubble hold 4.5:1", () => {
		expect(ratio(light, "on-accent", "accent")).toBeGreaterThanOrEqual(4.5);
		expect(ratio(light, "said-me-ink", "said-me")).toBeGreaterThanOrEqual(4.5);
		expect(ratio(light, "ink", "said-them")).toBeGreaterThanOrEqual(4.5);
	});

	test("the mint fill stands off white at 2:1 or more, so the bubble and keys read on the canvas", () => {
		expect(ratio(light, "accent", "canvas")).toBeGreaterThanOrEqual(2);
		expect(ratio(light, "said-me", "canvas")).toBeGreaterThanOrEqual(2);
	});

	test("a field's, a group's and the composer's edge shows on white", () => {
		expect(ratio(light, "line", "pop")).toBeGreaterThanOrEqual(1.35);
		expect(ratio(light, "line-strong", "pop")).toBeGreaterThanOrEqual(1.75);
	});
});

describe("dark mode", () => {
	test("body, secondary and caption text hold 4.5:1 on every plane", () => {
		for (const ink of ["ink", "ink-2", "ink-3"])
			for (const plane of planes) expect({ ink, plane, ratio: ratio(dark, ink, plane) >= 4.5 }).toEqual({ ink, plane, ratio: true });
	});

	test("words on the mint and on both bubbles hold 4.5:1", () => {
		expect(ratio(dark, "on-accent", "accent")).toBeGreaterThanOrEqual(4.5);
		expect(ratio(dark, "said-me-ink", "said-me")).toBeGreaterThanOrEqual(4.5);
		expect(ratio(dark, "ink", "said-them")).toBeGreaterThanOrEqual(4.5);
	});

	test("accent, warning and danger words hold 4.5:1 on the canvas and on a card", () => {
		for (const ink of ["accent-ink", "warn-ink", "danger"])
			for (const plane of ["canvas", "raised"]) expect(ratio(dark, ink, plane)).toBeGreaterThanOrEqual(4.5);
	});
});
