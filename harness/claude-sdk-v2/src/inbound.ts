/**
 * Inbound mapping: notifications/claude/channel params → SDKUserMessage + the
 * parsed envelope (design §3.1). Extraction is harness-core's channelContent;
 * this wrapper adds the Agent-SDK message shape (core stays SDK-type-free) and
 * attaches the side-channel envelope the ledger classifies on.
 *
 * The content string is forwarded to the SDK VERBATIM; the envelope is
 * read-only metadata parsed from a copy.
 */

import type { SDKUserMessage } from "@anthropic-ai/claude-agent-sdk";
import { randomUUID } from "node:crypto";
import { channelContent } from "@1broseidon/hotline-harness-core/inbound";
import { parseEnvelope, type Envelope } from "./envelope.js";

export interface InboundTurn {
  msg: SDKUserMessage;
  env: Envelope | null;
  /** The uuid stamped on the message — the ledger's attribution key. */
  uuid: string;
}

/**
 * Map a channel notification's params to a uuid-stamped SDKUserMessage plus its
 * parsed envelope, or null when the params carry no usable content (caller logs
 * and drops).
 */
export function toInboundTurn(params: unknown): InboundTurn | null {
  const content = channelContent(params);
  if (content === null) return null;
  const uuid = randomUUID();
  const msg = {
    type: "user",
    message: { role: "user", content },
    parent_tool_use_id: null,
    uuid,
  } as SDKUserMessage;
  return { msg, env: parseEnvelope(content), uuid };
}

/** Build a uuid-stamped internal (harness-injected) turn — mission handoff /
 * `/compact`. These carry no envelope; the ledger marks them internal. */
export function internalTurn(content: string): { msg: SDKUserMessage; uuid: string } {
  const uuid = randomUUID();
  const msg = {
    type: "user",
    message: { role: "user", content },
    parent_tool_use_id: null,
    uuid,
  } as SDKUserMessage;
  return { msg, uuid };
}
