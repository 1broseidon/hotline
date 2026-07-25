#!/usr/bin/env node
/**
 * Stamp dist/.hotline-harness after every build. `hotline up`'s claude-sdk
 * preflight reads this marker to enforce the Go↔TS lockstep rule (PARITY.md):
 * a dist whose child-harness disagrees with what the Go binary reserves would
 * otherwise surface as a confusing ownership refusal at claim time; the
 * marker turns it into a one-line "rebuild" error at launch.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const dist = path.resolve(here, "..", "dist");
fs.mkdirSync(dist, { recursive: true });
fs.writeFileSync(path.join(dist, ".hotline-harness"), "harness=claude-sdk\nchild-harness=claude-sdk\n");
