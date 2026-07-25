/**
 * Auth seam: a pure precedence REPORT, not a resolver. The consuming harness's
 * runtime (the Agent SDK for claude-sdk) does the actual credential
 * resolution; we only log which source will win so harness.log tells the
 * truth. Pluggable and dumb by design — official programmatic Max auth is
 * postponed upstream; swapping auth = changing env.
 */

export type AuthSource = "api-key" | "oauth-token-env" | "auth-token" | "stored-login";

export function resolveAuth(env: NodeJS.ProcessEnv): { source: AuthSource; note: string } {
  const has = (k: string): boolean => typeof env[k] === "string" && env[k]!.trim() !== "";
  if (has("ANTHROPIC_API_KEY")) {
    return { source: "api-key", note: "ANTHROPIC_API_KEY is set; the SDK will bill the API key" };
  }
  if (has("CLAUDE_CODE_OAUTH_TOKEN")) {
    return { source: "oauth-token-env", note: "CLAUDE_CODE_OAUTH_TOKEN is set (claude setup-token); experimental" };
  }
  if (has("ANTHROPIC_AUTH_TOKEN")) {
    return { source: "auth-token", note: "ANTHROPIC_AUTH_TOKEN is set (custom gateway/bearer)" };
  }
  return {
    source: "stored-login",
    note: "no auth env vars set; relying on a stored `claude login` under HOME (errors surface on the first turn)",
  };
}
