import { test } from "node:test";
import assert from "node:assert/strict";
import { resolveAuth } from "../dist/auth.js";

test("precedence table", () => {
  assert.equal(
    resolveAuth({ ANTHROPIC_API_KEY: "k", CLAUDE_CODE_OAUTH_TOKEN: "t", ANTHROPIC_AUTH_TOKEN: "a" }).source,
    "api-key",
  );
  assert.equal(resolveAuth({ CLAUDE_CODE_OAUTH_TOKEN: "t", ANTHROPIC_AUTH_TOKEN: "a" }).source, "oauth-token-env");
  assert.equal(resolveAuth({ ANTHROPIC_AUTH_TOKEN: "a" }).source, "auth-token");
  assert.equal(resolveAuth({}).source, "stored-login");
});

test("blank values do not count as set", () => {
  assert.equal(resolveAuth({ ANTHROPIC_API_KEY: "  " }).source, "stored-login");
  assert.equal(resolveAuth({ ANTHROPIC_API_KEY: "" , CLAUDE_CODE_OAUTH_TOKEN: "t"}).source, "oauth-token-env");
});

test("note never contains the credential value", () => {
  const { note } = resolveAuth({ ANTHROPIC_API_KEY: "sk-ant-secret" });
  assert.ok(!note.includes("sk-ant-secret"));
});
