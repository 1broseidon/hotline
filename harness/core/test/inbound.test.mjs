import { test } from "node:test";
import assert from "node:assert/strict";
import { channelContent } from "../dist/inbound.js";

const ENVELOPE = '<channel source="telegram" chat_id="1" message_id="2" user="g">\nhi\n</channel>';

test("extracts envelope content verbatim", () => {
  assert.equal(channelContent({ content: ENVELOPE, meta: null }), ENVELOPE);
});

test("empty content → null", () => {
  assert.equal(channelContent({ content: "" }), null);
});

test("missing content → null", () => {
  assert.equal(channelContent({}), null);
});

test("non-string content → null", () => {
  assert.equal(channelContent({ content: 42 }), null);
});

test("null / undefined params → null", () => {
  assert.equal(channelContent(null), null);
  assert.equal(channelContent(undefined), null);
});
