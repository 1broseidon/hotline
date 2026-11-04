import { expect, test } from "bun:test";
import type { BrowserView } from "../src/generated/contract";
import { installDom } from "./dom";

test("The PIP stays up and marks it while the browser needs the person, and leaves when it is long idle", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { BrowserPip } = await import("../src/components/BrowserPip");
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const idle: BrowserView = { phase: "teammate", url: "https://expired.badssl.com/", title: "", activeAt: Date.now() - 600_000 };
	try {
		await act(async () => root.render(<BrowserPip personaId="quill" name="Quill" view={idle} />));
		expect(container.querySelector(".browser-pip")).toBeNull();

		await act(async () => root.render(<BrowserPip personaId="quill" name="Quill" view={{ ...idle, needs: "Certificate expired" }} />));
		const pip = container.querySelector(".browser-pip");
		expect(pip?.classList.contains("needs")).toBe(true);
		expect(container.querySelector(".browser-pip-needs")?.textContent).toBe("Certificate expired");
		expect(container.textContent).toContain("Quill needs you");
		expect(container.textContent).toContain("expired.badssl.com");
		expect(container.querySelector("button")?.getAttribute("aria-label")).toBe("Open Quill's browser: Certificate expired");
	} finally {
		await act(async () => root.unmount());
		container.remove();
		teardown();
	}
});
