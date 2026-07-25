/**
 * Session-id persistence so a supervisor respawn resumes the same agent
 * session. The file lives under the supervisor dir when supervised, else a
 * dotfile in the cwd. Corrupt/missing files read as null; writes are
 * temp-then-rename so a crash never leaves a partial file.
 *
 * The file name is harness-scoped and passed by the caller (claude-sdk uses
 * "claude-sdk-session.json"); the unsupervised dotfile fallback derives from
 * it (".hotline-<fileName>").
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { defaultLog as log } from "./log.js";

export function sessionFilePath(
  fileName: string,
  env: NodeJS.ProcessEnv = process.env,
  cwd = process.cwd(),
): string {
  const supDir = env.HOTLINE_SUPERVISOR_DIR;
  if (supDir && supDir.trim() !== "") return path.join(supDir, fileName);
  return path.join(cwd, `.hotline-${fileName}`);
}

export function loadSessionId(file: string): string | null {
  try {
    const raw = fs.readFileSync(file, "utf8");
    const parsed = JSON.parse(raw) as { session_id?: unknown };
    const id = parsed?.session_id;
    if (typeof id === "string" && id !== "") return id;
    return null;
  } catch {
    return null; // missing or corrupt: start fresh
  }
}

export function saveSessionId(file: string, sessionId: string): void {
  try {
    const tmp = `${file}.tmp`;
    fs.writeFileSync(tmp, JSON.stringify({ session_id: sessionId, saved_at: new Date().toISOString() }) + "\n");
    fs.renameSync(tmp, file);
  } catch (err) {
    // Persistence is best-effort: losing it costs continuity, not correctness.
    log.warn(`session: save failed: ${(err as Error).message}`);
  }
}

export function clearSessionId(file: string): void {
  try {
    fs.unlinkSync(file);
  } catch {
    // already gone
  }
}
