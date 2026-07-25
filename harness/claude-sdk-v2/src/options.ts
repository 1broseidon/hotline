/**
 * Pure knob mapping for the SDK session options (spec §3): the HOTLINE_SDK_*
 * env family → Agent SDK Options fields. The Go side (`hotline up`,
 * config.LoadSDK) validates loudly at launch and re-exports RESOLVED values
 * into this process's env (including the deprecated HOTLINE_CLAUDE_SDK_MODEL
 * fallback — resolved there, in one place); the guards here cover
 * direct/unsupervised runs, where a knob typo must never crash the box.
 */

import { log } from "./log.js";

/**
 * Effort-name → maxThinkingTokens calibration. Initial values — tune here,
 * one exported const, never inline elsewhere.
 */
export const EFFORT_TO_MAX_THINKING_TOKENS: Readonly<Record<string, number>> = {
  low: 4096,
  medium: 8192,
  high: 16384,
  xhigh: 32768,
  max: 63999,
};

/**
 * Map HOTLINE_SDK_EFFORT to Options.maxThinkingTokens: a known name uses the
 * table, a positive integer string passes through raw, unset/empty means
 * undefined (SDK default). An unknown name logs a warning and returns
 * undefined — never a crash at runtime (the `up` path already failed loudly).
 */
export function effortToMaxThinkingTokens(effort: string | undefined): number | undefined {
  if (effort === undefined) return undefined;
  const key = effort.trim().toLowerCase();
  if (key === "") return undefined;
  const mapped = EFFORT_TO_MAX_THINKING_TOKENS[key];
  if (mapped !== undefined) return mapped;
  if (/^[0-9]+$/.test(key)) {
    const n = Number(key);
    if (Number.isSafeInteger(n) && n > 0) return n;
  }
  log.warn(`unknown HOTLINE_SDK_EFFORT ${JSON.stringify(effort)} (want low|medium|high|xhigh|max or a positive integer); using the SDK default`);
  return undefined;
}

/** The SDK's symbolic effort ladder — the Query.applyFlagSettings({effortLevel})
 * / Options.effort value set. `max` is session-scoped (applyFlagSettings accepts
 * it; the persisted Settings.effortLevel excludes it), which is exactly the hot
 * live-session case, so we keep it. */
export type SdkEffortLevel = "low" | "medium" | "high" | "xhigh" | "max";

const SDK_EFFORT_LEVELS: readonly SdkEffortLevel[] = ["low", "medium", "high", "xhigh", "max"];

/**
 * How a HOTLINE_SDK_EFFORT value maps onto the SDK's effort controls — the
 * ONE place model boot (Options) and hot apply (applyFlagSettings /
 * setMaxThinkingTokens) agree, so the two paths can never drift:
 *
 *   - a symbolic name → `effortLevel` (Options.effort at boot,
 *     applyFlagSettings({effortLevel}) live). This is the modern control that
 *     actually steers reasoning depth on current models — unlike the
 *     deprecated maxThinkingTokens, which several models treat as mere on/off.
 *   - a positive integer → `maxThinkingTokens` (Options.maxThinkingTokens at
 *     boot, setMaxThinkingTokens(n) live) — the raw-budget escape hatch.
 *   - unset/empty → `clear` (boot: leave the SDK default; live: clear the
 *     flag layer / the thinking budget back to default).
 *   - anything else → `invalid` (a knob typo; boot ignores with a warning,
 *     the hot path validates and refuses before touching the session).
 */
export type EffortApply =
  | { kind: "effortLevel"; level: SdkEffortLevel }
  | { kind: "maxThinkingTokens"; tokens: number }
  | { kind: "clear" }
  | { kind: "invalid"; raw: string };

export function effortToSdkApply(effort: string | undefined): EffortApply {
  if (effort === undefined) return { kind: "clear" };
  const key = effort.trim().toLowerCase();
  if (key === "") return { kind: "clear" };
  if ((SDK_EFFORT_LEVELS as readonly string[]).includes(key)) {
    return { kind: "effortLevel", level: key as SdkEffortLevel };
  }
  if (/^[0-9]+$/.test(key)) {
    const n = Number(key);
    if (Number.isSafeInteger(n) && n > 0) return { kind: "maxThinkingTokens", tokens: n };
  }
  return { kind: "invalid", raw: effort };
}

/**
 * The SDK's filesystem settings-source tiers — the Options.settingSources
 * literal union (@anthropic-ai/claude-agent-sdk sdk.d.ts: `SettingSource =
 * 'user' | 'project' | 'local'`). Kept in lockstep with the Go accept list
 * (internal/config/sdk.go sdkSettingSources) — change both together.
 */
export type SdkSettingSource = "user" | "project" | "local";

const SDK_SETTING_SOURCES: readonly SdkSettingSource[] = ["user", "project", "local"];

/**
 * Parse HOTLINE_SDK_SETTING_SOURCES (comma-separated) into Options.settingSources.
 * Unset/empty → [] (the Umibozu-safe hermetic default, unchanged behavior:
 * no user/project/local settings bleed). Tokens are trimmed and lowercased,
 * duplicates collapsed, order preserved. An unknown token throws LOUD naming
 * the var and the valid values — a knob typo must fail at harness boot, not be
 * silently dropped (mirrors LoadSDK's typo-fails-loudly stance on the Go side).
 */
export function parseSettingSources(raw: string | undefined): SdkSettingSource[] {
  if (raw === undefined) return [];
  const trimmed = raw.trim();
  if (trimmed === "") return [];
  const out: SdkSettingSource[] = [];
  for (const part of trimmed.split(",")) {
    const tok = part.trim().toLowerCase();
    if (tok === "") continue;
    if (!(SDK_SETTING_SOURCES as readonly string[]).includes(tok)) {
      throw new Error(
        `invalid HOTLINE_SDK_SETTING_SOURCES token ${JSON.stringify(part.trim())} ` +
          `(want a comma-separated list of ${SDK_SETTING_SOURCES.join("|")}; unset/empty = none)`,
      );
    }
    const src = tok as SdkSettingSource;
    if (!out.includes(src)) out.push(src);
  }
  return out;
}

/**
 * Parse a positive integer env value (HOTLINE_SDK_MAX_TURNS); anything else
 * — unset, empty, zero, negative, junk — is undefined (unlimited).
 */
export function parsePositiveInt(raw: string | undefined): number | undefined {
  if (raw === undefined) return undefined;
  const s = raw.trim();
  if (!/^[0-9]+$/.test(s)) return undefined;
  const n = Number(s);
  return Number.isSafeInteger(n) && n > 0 ? n : undefined;
}

/** Human description of the effort knob for the boot log line. */
export function describeEffort(effort: string | undefined): string {
  if (effort === undefined || effort.trim() === "") return "unset";
  const tokens = effortToMaxThinkingTokens(effort);
  if (tokens === undefined) return `${effort.trim()} (unknown; SDK default)`;
  const key = effort.trim().toLowerCase();
  return key in EFFORT_TO_MAX_THINKING_TOKENS ? `${key} (${tokens} thinking tokens)` : `${tokens} thinking tokens`;
}
