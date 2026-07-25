/**
 * Envelope attr parsing (design §3.1) — the inverse of the Go side's
 * `internal/harness/channel.go` RenderChannel.
 *
 * The run child pre-renders every inbound turn into a `<channel …>` envelope
 * whose attributes are HTML-escaped in a stable order (source and chat_id
 * first). We forward that content to the SDK VERBATIM; this parse is a
 * read-only side channel that lets the ledger classify a turn (operator vs
 * schedule/notify/fleet) and route the M1 fallback and M2 last-operator
 * persistence. Parsing never mutates the content string.
 *
 * Content that does not begin with `<channel` (RenderChannel returns bare
 * content when there is no meta) parses to null.
 */

export interface Envelope {
  source?: string;
  chat_id?: string;
  kind?: string;
  message_id?: string;
}

/**
 * Reverse the small subset of HTML entities `html.EscapeString` (Go) emits:
 * `& < > " '` → `&amp; &lt; &gt; &#34; &#39;`. `&#34;`/`&quot;` and
 * `&#39;`/`&apos;` are both accepted so a future escaper change can't silently
 * corrupt a value. `&amp;` is unescaped LAST so an input like `&amp;lt;` round
 * trips to `&lt;`, not `<`.
 */
export function htmlUnescape(s: string): string {
  return s
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&#34;/g, '"')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, "&");
}

/**
 * Parse the leading `<channel …>` tag's attributes out of a pre-rendered
 * inbound content string. Returns the well-known routing attrs, or null when
 * the content carries no channel tag.
 */
export function parseEnvelope(content: string): Envelope | null {
  if (typeof content !== "string") return null;
  const trimmed = content.replace(/^\s+/, "");
  if (!trimmed.startsWith("<channel")) return null;
  // Attribute values are HTML-escaped, so no unescaped `>` can appear inside
  // them — the first `>` is always the tag close.
  const close = trimmed.indexOf(">");
  if (close < 0) return null;
  const attrStr = trimmed.slice("<channel".length, close);

  const env: Envelope = {};
  const re = /([A-Za-z_][A-Za-z0-9_-]*)="([^"]*)"/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(attrStr)) !== null) {
    const key = m[1];
    const val = htmlUnescape(m[2]);
    switch (key) {
      case "source":
        env.source = val;
        break;
      case "chat_id":
        env.chat_id = val;
        break;
      case "kind":
        env.kind = val;
        break;
      case "message_id":
        env.message_id = val;
        break;
      // other attrs (user, ts, attachment_*, …) are carried by the content the
      // model reads; the ledger only needs the routing/classification quartet.
      default:
        break;
    }
  }
  return env;
}
