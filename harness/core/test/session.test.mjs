import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { sessionFilePath, loadSessionId, saveSessionId, clearSessionId } from "../dist/session.js";

function tmpFile() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sdk-session-"));
  return path.join(dir, "claude-sdk-session.json");
}

test("save/load round-trip", () => {
  const f = tmpFile();
  saveSessionId(f, "abc-123");
  assert.equal(loadSessionId(f), "abc-123");
  const parsed = JSON.parse(fs.readFileSync(f, "utf8"));
  assert.equal(parsed.session_id, "abc-123");
  assert.ok(parsed.saved_at);
});

test("missing file → null", () => {
  assert.equal(loadSessionId(tmpFile()), null);
});

test("corrupt JSON → null, no throw", () => {
  const f = tmpFile();
  fs.writeFileSync(f, "{not json");
  assert.equal(loadSessionId(f), null);
});

test("non-string session_id → null", () => {
  const f = tmpFile();
  fs.writeFileSync(f, JSON.stringify({ session_id: 7 }));
  assert.equal(loadSessionId(f), null);
});

test("atomic write leaves no partial tmp file", () => {
  const f = tmpFile();
  saveSessionId(f, "xyz");
  assert.equal(fs.existsSync(`${f}.tmp`), false);
});

test("clearSessionId removes and is idempotent", () => {
  const f = tmpFile();
  saveSessionId(f, "gone");
  clearSessionId(f);
  assert.equal(loadSessionId(f), null);
  assert.doesNotThrow(() => clearSessionId(f));
});

test("sessionFilePath prefers HOTLINE_SUPERVISOR_DIR", () => {
  assert.equal(
    sessionFilePath("claude-sdk-session.json", { HOTLINE_SUPERVISOR_DIR: "/sup/dir" }, "/work"),
    path.join("/sup/dir", "claude-sdk-session.json"),
  );
  assert.equal(
    sessionFilePath("claude-sdk-session.json", {}, "/work"),
    path.join("/work", ".hotline-claude-sdk-session.json"),
  );
  assert.equal(
    sessionFilePath("claude-sdk-session.json", { HOTLINE_SUPERVISOR_DIR: "  " }, "/work"),
    path.join("/work", ".hotline-claude-sdk-session.json"),
  );
});
