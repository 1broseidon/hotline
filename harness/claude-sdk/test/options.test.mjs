import { test } from "node:test";
import assert from "node:assert/strict";
import {
  EFFORT_TO_MAX_THINKING_TOKENS,
  effortToMaxThinkingTokens,
  effortToSdkApply,
  parsePositiveInt,
  describeEffort,
  parseSettingSources,
} from "../dist/options.js";

test("effort table carries the five calibrated names", () => {
  assert.deepEqual(EFFORT_TO_MAX_THINKING_TOKENS, {
    low: 4096,
    medium: 8192,
    high: 16384,
    xhigh: 32768,
    max: 63999,
  });
});

test("named efforts map through the table, case-insensitively", () => {
  assert.equal(effortToMaxThinkingTokens("low"), 4096);
  assert.equal(effortToMaxThinkingTokens("XHIGH"), 32768);
  assert.equal(effortToMaxThinkingTokens(" max "), 63999);
});

test("numeric effort passes through raw", () => {
  assert.equal(effortToMaxThinkingTokens("12000"), 12000);
});

test("unset/empty effort → undefined (SDK default)", () => {
  assert.equal(effortToMaxThinkingTokens(undefined), undefined);
  assert.equal(effortToMaxThinkingTokens(""), undefined);
  assert.equal(effortToMaxThinkingTokens("   "), undefined);
});

test("unknown effort warns and returns undefined — never throws", () => {
  assert.equal(effortToMaxThinkingTokens("xtreme"), undefined);
  assert.equal(effortToMaxThinkingTokens("-5"), undefined);
  assert.equal(effortToMaxThinkingTokens("0"), undefined);
});

test("effortToSdkApply: symbolic names → effortLevel (the modern control)", () => {
  assert.deepEqual(effortToSdkApply("high"), { kind: "effortLevel", level: "high" });
  assert.deepEqual(effortToSdkApply("XHIGH"), { kind: "effortLevel", level: "xhigh" });
  // max is session-scoped but valid for the live applyFlagSettings control.
  assert.deepEqual(effortToSdkApply(" max "), { kind: "effortLevel", level: "max" });
});

test("effortToSdkApply: positive integer → maxThinkingTokens (raw budget)", () => {
  assert.deepEqual(effortToSdkApply("12000"), { kind: "maxThinkingTokens", tokens: 12000 });
});

test("effortToSdkApply: unset/empty → clear", () => {
  assert.deepEqual(effortToSdkApply(undefined), { kind: "clear" });
  assert.deepEqual(effortToSdkApply(""), { kind: "clear" });
  assert.deepEqual(effortToSdkApply("  "), { kind: "clear" });
});

test("effortToSdkApply: junk / non-positive → invalid (never silently applied)", () => {
  assert.deepEqual(effortToSdkApply("turbo"), { kind: "invalid", raw: "turbo" });
  assert.deepEqual(effortToSdkApply("0"), { kind: "invalid", raw: "0" });
  assert.deepEqual(effortToSdkApply("-5"), { kind: "invalid", raw: "-5" });
});

test("parsePositiveInt accepts positive integers only", () => {
  assert.equal(parsePositiveInt("40"), 40);
  assert.equal(parsePositiveInt(" 7 "), 7);
  assert.equal(parsePositiveInt("0"), undefined);
  assert.equal(parsePositiveInt("-3"), undefined);
  assert.equal(parsePositiveInt("1.5"), undefined);
  assert.equal(parsePositiveInt("many"), undefined);
  assert.equal(parsePositiveInt(""), undefined);
  assert.equal(parsePositiveInt(undefined), undefined);
});

test("parseSettingSources: unset/empty → [] (hermetic default)", () => {
  assert.deepEqual(parseSettingSources(undefined), []);
  assert.deepEqual(parseSettingSources(""), []);
  assert.deepEqual(parseSettingSources("   "), []);
  assert.deepEqual(parseSettingSources(" , , "), []);
});

test("parseSettingSources: parses/normalizes valid tokens, case-insensitive, order preserved", () => {
  assert.deepEqual(parseSettingSources("project"), ["project"]);
  assert.deepEqual(parseSettingSources("Project, USER ,local"), ["project", "user", "local"]);
  assert.deepEqual(parseSettingSources("user,user,project"), ["user", "project"]);
});

test("parseSettingSources: invalid token throws LOUD naming the var and valid values", () => {
  assert.throws(() => parseSettingSources("projekt"), /HOTLINE_SDK_SETTING_SOURCES.*user\|project\|local/s);
  assert.throws(() => parseSettingSources("project,global"), /"global"/);
  assert.throws(() => parseSettingSources("managed"), /HOTLINE_SDK_SETTING_SOURCES/);
});

test("describeEffort renders the boot-log forms", () => {
  assert.equal(describeEffort(undefined), "unset");
  assert.equal(describeEffort(""), "unset");
  assert.equal(describeEffort("xhigh"), "xhigh (32768 thinking tokens)");
  assert.equal(describeEffort("9000"), "9000 thinking tokens");
  assert.equal(describeEffort("xtreme"), "xtreme (unknown; SDK default)");
});

// ---- Integer-knob parity with the Go side (sol review #12) ------------------
//
// The MIRROR of internal/config/sdk_test.go's TestIntegerKnobParityWithTheHarness.
// Go validates these knobs at `up` and at the app channel's set_sdk_config; this
// module applies them. When the two rules differ the failure is silent in both
// directions — Go accepted "+5" and effort quietly fell back to the SDK default;
// Go accepted values above 2^53 and maxTurns quietly became unlimited.
//
// Keep the two tables in step, case for case.
const INTEGER_KNOB_CASES = [
  ["1", true, "the smallest positive budget"],
  ["32000", true, "an ordinary budget"],
  ["9007199254740991", true, "exactly Number.MAX_SAFE_INTEGER"],
  ["+5", false, "a leading plus: Go's strconv.Atoi took it, digits-only does not"],
  ["-5", false, "negative"],
  ["0", false, "zero is not a positive budget"],
  ["9007199254740992", false, "one past MAX_SAFE_INTEGER"],
  ["99999999999999999999", false, "far past exact representation"],
  ["5.0", false, "not an integer"],
  ["1e3", false, "exponent form"],
  ["", false, "empty"],
  ["abc", false, "junk"],
];

test("parity: parsePositiveInt matches the Go integer-knob rule case for case", () => {
  for (const [input, valid, why] of INTEGER_KNOB_CASES) {
    const got = parsePositiveInt(input);
    assert.equal(
      got !== undefined,
      valid,
      `parsePositiveInt(${JSON.stringify(input)}) = ${got}, want ${valid ? "a number" : "undefined"} — ${why}`,
    );
  }
});

test("parity: a numeric effort follows the same rule", () => {
  for (const [input, valid, why] of INTEGER_KNOB_CASES) {
    const plan = effortToSdkApply(input);
    // "" is the clear escape on both sides, not an integer; every other
    // invalid case must be refused rather than silently defaulted.
    if (input === "") {
      assert.equal(plan.kind, "clear");
      continue;
    }
    assert.equal(
      plan.kind === "maxThinkingTokens",
      valid,
      `effortToSdkApply(${JSON.stringify(input)}).kind = ${plan.kind}, want ${valid ? "maxThinkingTokens" : "invalid"} — ${why}`,
    );
  }
});
