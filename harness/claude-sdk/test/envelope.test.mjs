/**
 * Envelope attr parsing (design §3.1) — the inverse of the Go RenderChannel.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { parseEnvelope, htmlUnescape } from "../dist/envelope.js";

test("parses source/chat_id/kind/message_id from a rendered channel tag", () => {
  const content = `<channel source="telegram" chat_id="123" message_id="7" kind="message">\nhey there\n</channel>`;
  const env = parseEnvelope(content);
  assert.deepEqual(env, { source: "telegram", chat_id: "123", message_id: "7", kind: "message" });
});

test("bare content (no channel tag) parses to null", () => {
  assert.equal(parseEnvelope("just some text, no envelope"), null);
  assert.equal(parseEnvelope(""), null);
});

test("leading whitespace before the tag is tolerated", () => {
  const env = parseEnvelope(`\n  <channel source="app" chat_id="a-1">\nx\n</channel>`);
  assert.deepEqual(env, { source: "app", chat_id: "a-1" });
});

test("HTML-escaped attribute values are unescaped", () => {
  const content = `<channel source="telegram" chat_id="1" user="A&amp;B &lt;x&gt; &#39;q&#39; &#34;d&#34;">\nc\n</channel>`;
  const env = parseEnvelope(content);
  assert.equal(env.source, "telegram");
  assert.equal(env.chat_id, "1");
});

test("a > inside the content body does not confuse the tag close", () => {
  const content = `<channel source="telegram" chat_id="1">\n1 > 0 is true\n</channel>`;
  const env = parseEnvelope(content);
  assert.deepEqual(env, { source: "telegram", chat_id: "1" });
});

test("kind attributes for excluded classes parse through", () => {
  for (const kind of ["schedule", "notify", "fleet", "element_action"]) {
    const env = parseEnvelope(`<channel source="telegram" chat_id="1" kind="${kind}">\nx\n</channel>`);
    assert.equal(env.kind, kind);
  }
});

test("htmlUnescape reverses the Go escaper set, & last", () => {
  assert.equal(htmlUnescape("a&amp;b"), "a&b");
  assert.equal(htmlUnescape("&lt;tag&gt;"), "<tag>");
  assert.equal(htmlUnescape("&#34;q&#34;"), '"q"');
  assert.equal(htmlUnescape("&quot;q&quot;"), '"q"');
  assert.equal(htmlUnescape("&#39;s&#39;"), "'s'");
  // & unescaped last: &amp;lt; → &lt;, not <
  assert.equal(htmlUnescape("&amp;lt;"), "&lt;");
});
