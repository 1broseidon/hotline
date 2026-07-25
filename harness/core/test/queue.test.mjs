import { test } from "node:test";
import assert from "node:assert/strict";
import { AsyncQueue } from "../dist/queue.js";

test("FIFO order preserved", async () => {
  const q = new AsyncQueue();
  q.push(1);
  q.push(2);
  q.push(3);
  q.close();
  const got = [];
  for await (const v of q) got.push(v);
  assert.deepEqual(got, [1, 2, 3]);
});

test("consumer waiting before push is woken", async () => {
  const q = new AsyncQueue();
  const it = q[Symbol.asyncIterator]();
  const p = it.next();
  q.push("hello");
  assert.deepEqual(await p, { value: "hello", done: false });
});

test("close drains buffered items first, then terminates", async () => {
  const q = new AsyncQueue();
  q.push("a");
  q.close();
  const it = q[Symbol.asyncIterator]();
  assert.deepEqual(await it.next(), { value: "a", done: false });
  const end = await it.next();
  assert.equal(end.done, true);
});

test("push after close is a logged drop, not a throw", async () => {
  const q = new AsyncQueue();
  q.close();
  assert.doesNotThrow(() => q.push("late"));
  const it = q[Symbol.asyncIterator]();
  assert.equal((await it.next()).done, true);
});

test("close while a consumer is waiting ends the iterator", async () => {
  const q = new AsyncQueue();
  const it = q[Symbol.asyncIterator]();
  const p = it.next();
  q.close();
  assert.equal((await p).done, true);
});

test("interleaved async producers keep order", async () => {
  const q = new AsyncQueue();
  setTimeout(() => q.push("x"), 5);
  setTimeout(() => q.push("y"), 10);
  setTimeout(() => q.close(), 20);
  const got = [];
  for await (const v of q) got.push(v);
  assert.deepEqual(got, ["x", "y"]);
});

// ---- Leases: a failed consumer must not eat the queue (sol review #5) -------
//
// Iterating the queue directly is lossy whenever the consumer can fail: next()
// REMOVES the item, so a consumer that dies before doing anything with it takes
// it with it. On the Agent SDK path that is a real, unbounded loss — a resume
// against a wiped session throws only after the SDK has pulled turns, and
// replayCatchup can push any number of them at boot.

test("a cancelled consumer returns every unacked item, in order, at the front", async () => {
  const q = new AsyncQueue();
  q.push("turn-1");
  q.push("turn-2");
  q.push("turn-3");

  const first = q.consumer();
  const it = first[Symbol.asyncIterator]();
  assert.equal((await it.next()).value, "turn-1");
  assert.equal((await it.next()).value, "turn-2");
  assert.equal(first.leased, 2);

  // The attempt fails before the session produced anything.
  first.cancel();

  const second = q.consumer();
  const got = [];
  const it2 = second[Symbol.asyncIterator]();
  for (let i = 0; i < 3; i++) got.push((await it2.next()).value);
  assert.deepEqual(
    got,
    ["turn-1", "turn-2", "turn-3"],
    "turns pulled by the failed attempt were lost, or reordered behind later ones",
  );
});

test("ack releases the leases so a later cancel requeues nothing", async () => {
  const q = new AsyncQueue();
  q.push("a");
  q.push("b");
  const c = q.consumer();
  const it = c[Symbol.asyncIterator]();
  await it.next();
  await it.next();
  assert.equal(c.leased, 2);
  c.ack();
  assert.equal(c.leased, 0);
  c.cancel();
  assert.equal(q.size, 0, "acked items were incorrectly returned to the queue");
});

// ack(n) — the additive partial-ack primitive (design §7.3): release the oldest
// n leases, keep the rest. Default (no arg) stays "release all"; the claude-sdk
// 0.2.0 ledger uses the counted form to close the pre-pulled-but-unprocessed
// crash window.
test("ack(n) releases the OLDEST n leases and keeps the rest held", async () => {
  const q = new AsyncQueue();
  for (const x of ["a", "b", "c"]) q.push(x);
  const c = q.consumer();
  const it = c[Symbol.asyncIterator]();
  await it.next();
  await it.next();
  await it.next();
  assert.equal(c.leased, 3);
  c.ack(2); // a, b safe; c still owed
  assert.equal(c.leased, 1);
  c.cancel();
  assert.deepEqual([q.size], [1], "the one unacked lease was requeued");
  const it2 = q.consumer()[Symbol.asyncIterator]();
  assert.equal((await it2.next()).value, "c", "the held lease requeues at the front, in order");
});

test("ack(0) is a no-op; ack(n>=leased) releases all", async () => {
  const q = new AsyncQueue();
  for (const x of ["a", "b"]) q.push(x);
  const c = q.consumer();
  const it = c[Symbol.asyncIterator]();
  await it.next();
  await it.next();
  c.ack(0);
  assert.equal(c.leased, 2, "ack(0) released nothing");
  c.ack(99);
  assert.equal(c.leased, 0, "ack(n>=leased) released everything");
});

test("cancel releases a parked waiter instead of orphaning it", async () => {
  const q = new AsyncQueue();
  const first = q.consumer();
  const it = first[Symbol.asyncIterator]();
  const parked = it.next(); // no items: this waits

  first.cancel();
  const settled = await parked;
  assert.equal(settled.done, true, "the dead consumer's waiter never settled — it would hang forever");

  // And the queue still works for the replacement.
  const second = q.consumer();
  q.push("after");
  assert.equal((await second[Symbol.asyncIterator]().next()).value, "after");
});

test("an item pushed to a cancelled consumer's waiter is kept, not swallowed", async () => {
  const q = new AsyncQueue();
  const first = q.consumer();
  const it = first[Symbol.asyncIterator]();
  const parked = it.next();
  first.cancel();
  // A producer racing the cancel: the item must survive for the next consumer.
  q.push("raced");
  assert.equal((await parked).done, true);

  const second = q.consumer();
  assert.equal(
    (await second[Symbol.asyncIterator]().next()).value,
    "raced",
    "an item delivered to a cancelled waiter vanished",
  );
});

test("starting a new consumer releases the old one's waiter", async () => {
  const q = new AsyncQueue();
  const first = q.consumer();
  const parked = first[Symbol.asyncIterator]().next();
  q.consumer(); // supersede without an explicit cancel
  assert.equal((await parked).done, true, "the superseded consumer's waiter stranded");
});
