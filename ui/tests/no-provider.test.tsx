import { expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { ErrorCard } = await import("../src/components/ErrorCard");

function notice(kind: string): string {
	return `Hotline Agent unavailable: Connect a provider in Settings › Providers to run a model.\n\n${JSON.stringify({
		hotlineFailure: { kind, title: "Hotline Agent unavailable", summary: "Connect a provider in Settings › Providers to run a model.", details: "No provider is connected on this desk." },
	})}`;
}

test("Hotline Agent with no provider links to Settings › Providers, and nothing else does", () => {
	const html = renderToStaticMarkup(<ErrorCard text={notice("no_provider")} onProviders={() => {}} />);
	expect(html).toContain("Hotline Agent unavailable");
	expect(html).toContain(">Settings › Providers</button>");
	// Not a failed turn: there is nothing to try again.
	expect(html).not.toContain("Try again");
	expect(renderToStaticMarkup(<ErrorCard text={notice("auth")} onProviders={() => {}} />)).not.toContain("</button>");
	// A thread's card, with nowhere to send the person, says it in words only.
	expect(renderToStaticMarkup(<ErrorCard text={notice("no_provider")} />)).not.toContain("</button>");
});
