/**
 * The harness's tagged logger instance. All plumbing lives in harness-core;
 * this module only binds the tag and the opt-in file-sink env var
 * (HOTLINE_SDK_LOG, an absolute path). Stderr-only invariant documented in
 * the core module.
 */

import { createLog } from "@1broseidon/hotline-harness-core/log";

export const log = createLog("hotline-sdk", "HOTLINE_SDK_LOG");
