/**
 * Logging for hotline's TS harnesses.
 *
 * HARD RULE: every byte a harness emits goes to stderr or a file, NEVER
 * stdout. In `pi --mode rpc` stdout is the JSONL event stream and in the
 * claude-sdk harness stdout is reserved on principle; a single stray line
 * corrupts the RPC stream for its client. So a Logger only ever writes to
 * `process.stderr` and, optionally, an append-only file. Under `hotline up`
 * the supervisor captures stderr into harness.log either way.
 *
 * Each harness creates its own tagged instance:
 *   createLog("hotline-pi", "HOTLINE_PI_LOG")
 *   createLog("hotline-sdk", "HOTLINE_SDK_LOG")
 * The optional second argument names an env var holding an absolute path to
 * the opt-in log file (read once, at creation).
 */

import * as fs from "node:fs";

export interface Logger {
  info(msg: string): void;
  warn(msg: string): void;
  error(msg: string): void;
}

export function createLog(tag: string, fileEnvVar?: string): Logger {
  const logFile = fileEnvVar ? process.env[fileEnvVar] : undefined;

  function write(level: string, msg: string): void {
    const line = `[${tag} ${new Date().toISOString()}] ${level} ${msg}\n`;
    // NEVER process.stdout.
    process.stderr.write(line);
    if (logFile) {
      try {
        fs.appendFileSync(logFile, line);
      } catch {
        // A broken log file must never take down the harness.
      }
    }
  }

  return {
    info: (msg: string) => write("INFO", msg),
    warn: (msg: string) => write("WARN", msg),
    error: (msg: string) => write("ERROR", msg),
  };
}

/**
 * Stderr-only fallback for core-internal diagnostics on modules whose public
 * API takes no Logger (queue, session). Rare paths only; harness-tagged
 * loggers are preferred wherever the API allows injection.
 */
export const defaultLog: Logger = createLog("hotline-core");
