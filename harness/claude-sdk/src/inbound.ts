/**
 * Inbound mapping: notifications/claude/channel params → SDKUserMessage.
 * Extraction is harness-core's channelContent; this wrapper only adds the
 * Agent-SDK message shape (core must stay SDK-type-free).
 */

import type { SDKUserMessage } from "@anthropic-ai/claude-agent-sdk";
import { channelContent } from "@1broseidon/hotline-harness-core/inbound";

/**
 * Map a channel notification's params to an SDKUserMessage, or null when the
 * params carry no usable content (caller logs and drops).
 */
export function toUserMessage(params: unknown): SDKUserMessage | null {
  const content = channelContent(params);
  if (content === null) return null;
  return {
    type: "user",
    message: { role: "user", content },
    parent_tool_use_id: null,
  };
}
