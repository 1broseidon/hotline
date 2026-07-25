/**
 * The per-turn contract reminder (design §2.3) must name the reply tool and the
 * <channel> tag so it can't drift from the Go-side profile, and must NOT
 * advertise the fallback lane (design §2.2 / §11.4 as tightened for this build).
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { CONTRACT_REMINDER } from "../dist/contract.js";

test("reminder names mcp__hotline__reply and <channel>", () => {
  assert.match(CONTRACT_REMINDER, /mcp__hotline__reply/);
  assert.match(CONTRACT_REMINDER, /<channel>/);
});

test("reminder does not advertise the safety net", () => {
  assert.doesNotMatch(CONTRACT_REMINDER, /lane|fallback|safety net|forward/i);
});
