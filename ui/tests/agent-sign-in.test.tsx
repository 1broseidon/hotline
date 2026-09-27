import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { TranscriptEvent } from "../src/generated/contract";
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { ErrorCard, errorDetails } = await import("../src/components/ErrorCard");
const { retryForNotice } = await import("../src/components/Transcript");

const signIn = { harnessName: "Test harness", methods: [{ id: "login", name: "Personal account" }, { id: "work", name: "Work account" }] };
function notice(extra: Record<string, unknown> = {}): string {
	return JSON.stringify({ hotlineFailure: { kind: "agent_auth", title: "Agent sign-in expired", summary: "Sign in again", details: "auth_required", signIn, ...extra } });
}

describe("Harness sign-in", () => {
	test("a supported auth failure offers Sign in and the advertised methods", () => {
		const html = renderToStaticMarkup(<ErrorCard text={notice()} personaId="ada" />);
		expect(html).toContain(">Sign in</button>");
		expect(html).toContain("Personal account");
		expect(html).toContain("Work account");
		expect(html).toContain("Test harness");
		expect(html).not.toContain("Review failed message");
	});

	test("only agent auth and only a main conversation offer sign-in", () => {
		for (const text of ["Old plain error", notice({ kind: "quota" }), notice({ signIn: null }), notice({ signIn: { ...signIn, methods: [] } })]) {
			expect(renderToStaticMarkup(<ErrorCard text={text} personaId="ada" />)).not.toContain(">Sign in</button>");
		}
		expect(renderToStaticMarkup(<ErrorCard text={notice()} />)).not.toContain(">Sign in</button>");
	});

	test("malformed descriptors do not become actions or executable parameters", () => {
		expect(errorDetails(notice({ signIn: { ...signIn, methods: [null, {}, { id: 5, name: "bad" }] } })).signIn).toBeUndefined();
		const parsed = errorDetails(notice({ signIn: { ...signIn, command: "/bin/sh", methods: [{ id: "ok", name: "OK", args: ["secret"], env: { SECRET: "secret" } }] } }));
		expect(parsed.signIn).toEqual({ harnessName: "Test harness", methods: [{ id: "ok", name: "OK" }] });
	});

	test("retry is an explicit review of the original message, including files", () => {
		const message: Extract<TranscriptEvent, { kind: "user" }> = { kind: "user", id: "u", ts: 1, text: "Try this", attachments: [] };
		const failed: TranscriptEvent = { kind: "notice", id: "n", ts: 2, text: notice(), level: "error" };
		let reviewed: typeof message | null = null;
		const retry = retryForNotice([message, failed], "n", (one) => { reviewed = one; });
		expect(reviewed).toBeNull();
		retry?.();
		expect(reviewed).toBe(message);
		expect(retryForNotice([message, failed, { ...message, id: "later" }], "n", () => {})).toBeUndefined();
		expect(retryForNotice([message, { kind: "agent", id: "a", ts: 2, text: "I will check" }, failed], "n", () => {})).toBeFunction();
		expect(retryForNotice([message, failed], "missing", () => {})).toBeUndefined();
		expect(retryForNotice([message, { kind: "turn", id: "t", ts: 2, stopReason: "end_turn" }, failed], "n", () => {})).toBeUndefined();
	});
});
