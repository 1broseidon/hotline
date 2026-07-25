/**
 * M2 auth containment (design §4): classification table; consecutive counter +
 * notify-once persistence across simulated respawns; marker at n=3; marker +
 * counter cleared on init; last-operator persistence.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import {
  classifyAuthMessage,
  classifyAuthThrow,
  createAuthWatch,
  saveLastOperator,
  loadLastOperator,
  AUTH_FATAL_MARKER,
  AUTH_STATE_FILE,
  EXIT_AUTH_FATAL,
} from "../dist/authwatch.js";

const nolog = { info() {}, warn() {}, error() {} };
const tmpDir = () => fs.mkdtempSync(path.join(os.tmpdir(), "hotline-authwatch-"));

test("classifyAuthMessage: auth_status with error, and fatal assistant errors", () => {
  assert.match(classifyAuthMessage({ type: "auth_status", error: "invalid_api_key" }), /invalid_api_key/);
  assert.equal(classifyAuthMessage({ type: "auth_status", isAuthenticating: true }), null); // progress
  assert.equal(classifyAuthMessage({ type: "assistant", error: "authentication_failed" }), "authentication_failed");
  assert.equal(classifyAuthMessage({ type: "assistant", error: "oauth_org_not_allowed" }), "oauth_org_not_allowed");
  assert.equal(classifyAuthMessage({ type: "assistant", error: "billing_error" }), null); // self-heals
  assert.equal(classifyAuthMessage({ type: "assistant", error: "rate_limit" }), null);
  assert.equal(classifyAuthMessage({ type: "result", subtype: "success" }), null);
});

test("classifyAuthThrow matches 401/invalid key/oauth-expired/authentication, else null", () => {
  assert.match(classifyAuthThrow(new Error("HTTP 401 Unauthorized")), /401/);
  assert.match(classifyAuthThrow(new Error("invalid api key")), /invalid api key/);
  assert.match(classifyAuthThrow(new Error("OAuth token expired")), /OAuth/);
  assert.match(classifyAuthThrow(new Error("authentication error")), /authentication/);
  assert.equal(classifyAuthThrow(new Error("connection reset")), null);
});

test("failures 1..2 exit 1; the 3rd notifies once, writes the marker, exits 5", async () => {
  const dir = tmpDir();
  let notifyCount = 0;
  const mk = () =>
    createAuthWatch({ supervisorDir: dir, log: nolog, notify: async () => (notifyCount++, true) });

  // Simulate three consecutive respawns, each a fresh AuthWatch over the same dir.
  assert.equal(await mk().onAuthFailure("bad creds"), 1);
  assert.equal(await mk().onAuthFailure("bad creds"), 1);
  assert.equal(await mk().onAuthFailure("bad creds"), EXIT_AUTH_FATAL);

  assert.equal(notifyCount, 1, "notify-once");
  assert.ok(fs.existsSync(path.join(dir, AUTH_FATAL_MARKER)), "marker written at n=3");

  // A 4th failure still exits 5 but does NOT notify again.
  assert.equal(await mk().onAuthFailure("bad creds"), EXIT_AUTH_FATAL);
  assert.equal(notifyCount, 1, "still notify-once across respawns");
});

test("a successful init clears the counter and the marker", async () => {
  const dir = tmpDir();
  const mk = () => createAuthWatch({ supervisorDir: dir, log: nolog, notify: async () => true });
  await mk().onAuthFailure("x");
  await mk().onAuthFailure("x");
  await mk().onAuthFailure("x"); // marker + state exist now
  assert.ok(fs.existsSync(path.join(dir, AUTH_STATE_FILE)));
  assert.ok(fs.existsSync(path.join(dir, AUTH_FATAL_MARKER)));

  mk().onInit(); // recovery
  assert.ok(!fs.existsSync(path.join(dir, AUTH_STATE_FILE)), "state cleared");
  assert.ok(!fs.existsSync(path.join(dir, AUTH_FATAL_MARKER)), "marker cleared");

  // Counter restarts from zero: the next failure is a transient exit-1 again.
  assert.equal(await mk().onAuthFailure("x"), 1);
});

test("notify-once holds even when the send itself fails", async () => {
  const dir = tmpDir();
  let attempts = 0;
  const mk = () =>
    createAuthWatch({ supervisorDir: dir, log: nolog, notify: async () => (attempts++, false) });
  await mk().onAuthFailure("x");
  await mk().onAuthFailure("x");
  assert.equal(await mk().onAuthFailure("x"), EXIT_AUTH_FATAL);
  assert.equal(await mk().onAuthFailure("x"), EXIT_AUTH_FATAL);
  assert.equal(attempts, 1, "one notify attempt, even though it returned false");
});

test("unsupervised (no supervisor dir) never escalates: always exit 1", async () => {
  const w = createAuthWatch({ supervisorDir: undefined, log: nolog, notify: async () => true });
  assert.equal(await w.onAuthFailure("x"), 1);
  assert.equal(await w.onAuthFailure("x"), 1);
  assert.equal(await w.onAuthFailure("x"), 1);
});

test("last-operator round-trips (source, chat_id)", () => {
  const dir = tmpDir();
  assert.equal(loadLastOperator(dir), null);
  saveLastOperator(dir, "telegram", "123");
  assert.deepEqual(loadLastOperator(dir), { source: "telegram", chat_id: "123" });
  saveLastOperator(dir, "discord", "456"); // overwrite
  assert.deepEqual(loadLastOperator(dir), { source: "discord", chat_id: "456" });
  saveLastOperator(dir, undefined, ""); // empty chat_id ignored
  assert.deepEqual(loadLastOperator(dir), { source: "discord", chat_id: "456" });
});
