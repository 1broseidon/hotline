/**
 * Inbound extraction: notifications/claude/channel params → envelope text.
 *
 * The hotline child's injected-harness branch pre-renders the full
 * `<channel …>` envelope server-side into params.content — chat_id, source,
 * attachments all ride inside the text. So the extraction is trivial and
 * pure; each harness wraps the string into its own turn type (pi:
 * sendUserMessage, claude-sdk: SDKUserMessage).
 */

/**
 * Extract a channel notification's pre-rendered envelope text, or null when
 * the params carry no usable content (caller logs and drops).
 */
export function channelContent(params: unknown): string | null {
  const content = (params as { content?: unknown } | null | undefined)?.content;
  if (typeof content !== "string" || content === "") return null;
  return content;
}
