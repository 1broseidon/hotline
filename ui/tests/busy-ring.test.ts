import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";

/**
 * A busy face's ring turns as a square box, whose corners stand past the
 * face at a diagonal. That made the rail scrollable sideways for a step, and
 * Windows drew a scrollbar each time. The ring is clipped just outside
 * itself where the platform can, and the rail never scrolls sideways.
 */
test("a busy face's turning ring never makes its list scroll sideways", () => {
	const css = readFileSync(new URL("../src/index.css", import.meta.url), "utf8");
	const clipped = css.match(/@supports \(overflow-clip-margin: 4px\) \{\s*\.avatar-busy \{([^}]*)\}/)?.[1] ?? "";
	expect(clipped).toMatch(/overflow:\s*clip/);
	expect(clipped).toMatch(/overflow-clip-margin:\s*4px/);
	const ring = css.match(/\.avatar-ring \{([^}]*)\}/)?.[1] ?? "";
	expect(ring).toMatch(/inset:\s*-3px/);
	const rail = readFileSync(new URL("../src/components/Rail.tsx", import.meta.url), "utf8");
	expect(rail).toContain("overflow-y-auto overflow-x-hidden");
});
