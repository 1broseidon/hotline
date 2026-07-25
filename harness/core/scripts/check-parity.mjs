#!/usr/bin/env node
/**
 * Mechanical drift guard for the harness parity ledger (harness/PARITY.md):
 *
 *   1. The pi copies of the shared modules must NOT reappear — pi imports
 *      @1broseidon/hotline-harness-core now; a resurrected harness/pi/src/
 *      child.ts / jsonrpc.ts / log.ts would silently fork the plumbing again.
 *   2. Every module core exports must have a row in PARITY.md (matched by
 *      normalized substring), so a new core module cannot ship without its
 *      parity story.
 *
 * Runs from core's `npm test` (`check:parity`). Exits non-zero with a named
 * reason on any violation.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const coreRoot = path.resolve(here, "..");
const harnessRoot = path.resolve(coreRoot, "..");

let failures = 0;
const fail = (msg) => {
  failures++;
  console.error(`check-parity: FAIL ${msg}`);
};

// 1. Forked copies must not come back.
for (const forbidden of ["child.ts", "jsonrpc.ts", "log.ts"]) {
  const p = path.join(harnessRoot, "pi", "src", forbidden);
  if (fs.existsSync(p)) {
    fail(`${p} exists — pi must import @1broseidon/hotline-harness-core, not fork it`);
  }
}

// 2. Every exported core module needs a PARITY.md row.
const parityPath = path.join(harnessRoot, "PARITY.md");
if (!fs.existsSync(parityPath)) {
  fail(`${parityPath} missing`);
} else {
  const normalize = (s) => s.toLowerCase().replace(/[^a-z0-9]/g, "");
  const parity = normalize(fs.readFileSync(parityPath, "utf8"));
  const modules = fs
    .readdirSync(path.join(coreRoot, "src"))
    .filter((f) => f.endsWith(".ts"))
    .map((f) => f.replace(/\.ts$/, ""));
  for (const mod of modules) {
    if (!parity.includes(normalize(mod))) {
      fail(`PARITY.md has no row mentioning core module "${mod}"`);
    }
  }
}

if (failures > 0) process.exit(1);
console.log("check-parity: ok");
