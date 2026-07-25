import { test } from "node:test";
import assert from "node:assert/strict";
import { classifySpawnError, isActiveConflict } from "../dist/child.js";

test("classifySpawnError table", () => {
  assert.equal(classifySpawnError("ENOENT"), "missing");
  assert.equal(classifySpawnError("EACCES"), "denied");
  assert.equal(classifySpawnError("EPERM"), "denied");
  assert.equal(classifySpawnError("EMFILE"), "transient");
  assert.equal(classifySpawnError("EAGAIN"), "transient");
  assert.equal(classifySpawnError(undefined), "transient");
});

test("isActiveConflict marker matching", () => {
  assert.equal(isActiveConflict("fatal: state ownership conflict detected"), true);
  assert.equal(isActiveConflict("error CLAIMING POLLER SLOT for bot"), true);
  assert.equal(isActiveConflict("poller slot busy"), true);
  assert.equal(isActiveConflict("some ordinary crash"), false);
  assert.equal(isActiveConflict(""), false);
});
