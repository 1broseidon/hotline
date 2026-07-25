/**
 * AsyncQueue<T>: an unbounded FIFO bridging push-style producers (the child's
 * notification handler) to a pull-style consumer (e.g. the async generator
 * feeding the Agent SDK's streaming input).
 *
 * Contract:
 *   - push() never throws. A push after close() is a logged drop, not an error
 *     (the notification handler runs inside the child's stdout data event; a
 *     throw there must never take the transport down — same posture as pi's
 *     deliver.ts).
 *   - close() lets the iterator drain everything already buffered, then end.
 *   - One ACTIVE consumer at a time; FIFO order is preserved across interleaved
 *     producers, and across a consumer being cancelled and replaced.
 *
 * LEASES (sol review #5). Iterating the queue directly is lossy whenever the
 * consumer can fail: `next()` removes the item, so if the thing consuming it
 * dies before doing anything with it, the item is simply gone. That is not
 * hypothetical on the Agent SDK path — a resume against a wiped session throws
 * only AFTER the SDK has pulled turns from the generator, and runAgent's retry
 * then builds a SECOND consumer over the same queue. The turns the first one
 * pulled (replayCatchup can push an unbounded number at boot) were lost, and
 * the dead consumer's waiter was left holding a promise nobody would ever
 * resolve.
 *
 * So a consumer LEASES: pulled items are held aside until ack() says they are
 * safely in the sink, and cancel() puts every unacked lease back at the FRONT
 * in original order and releases the waiter. A failed attempt loses nothing and
 * strands nothing.
 */

import { defaultLog as log } from "./log.js";

/**
 * One consumption attempt over a queue. Iterate it exactly as you would the
 * queue itself; the difference is what happens when the attempt fails.
 */
export interface QueueConsumer<T> extends AsyncIterable<T> {
  /**
   * Release leases that reached their destination. With no argument, release
   * ALL outstanding leases (the original behavior; the honest ack point for a
   * query attempt is the first message the session produces). With a count,
   * release the OLDEST `n` leases and keep the rest held — the delivery-evidence
   * ack the claude-sdk 0.2.0 ledger uses to close the pre-pulled-but-unprocessed
   * crash window (design §7.3): a turn the SDK pulled but has not yet echoed
   * stays leased and requeues on the next attempt's cancel(). `n<=0` is a no-op;
   * `n>=leased` releases all.
   */
  ack(n?: number): void;
  /**
   * Abandon this attempt. Every unacked lease is requeued at the FRONT in
   * original order, and a parked waiter is released so the dead iterator's
   * promise cannot strand. Idempotent.
   */
  cancel(): void;
  /** Items pulled but not yet acked. Diagnostics and tests. */
  readonly leased: number;
}

export class AsyncQueue<T> implements AsyncIterable<T> {
  private items: T[] = [];
  private waiter: ((v: IteratorResult<T>) => void) | null = null;
  private closed = false;
  /** The consumer currently allowed to park a waiter. A cancelled consumer's
   * late resolve must never be mistaken for the live one's. */
  private active: object | null = null;

  /** Enqueue one item. Never throws; drops (loudly) after close(). */
  push(item: T): void {
    try {
      if (this.closed) {
        log.warn("queue: push after close dropped");
        return;
      }
      if (this.waiter) {
        const w = this.waiter;
        this.waiter = null;
        w({ value: item, done: false });
        return;
      }
      this.items.push(item);
    } catch (err) {
      // Belt and braces: nothing above should throw, but the contract is
      // absolute — a producer must never see an exception.
      log.error(`queue: push failed: ${(err as Error).message}`);
    }
  }

  /** End the stream. Buffered items are still delivered first. Idempotent. */
  close(): void {
    if (this.closed) return;
    this.closed = true;
    if (this.waiter && this.items.length === 0) {
      const w = this.waiter;
      this.waiter = null;
      w({ value: undefined as unknown as T, done: true });
    }
  }

  /** Items buffered and not yet handed to a consumer. Diagnostics and tests. */
  get size(): number {
    return this.items.length;
  }

  /**
   * Start a leased consumption attempt. Creating one supersedes any previous
   * consumer: the old one's waiter is released so it cannot strand, but its
   * unacked leases are NOT auto-requeued — call cancel() on it first if you
   * want them back. (runAgent always does; making it implicit would hide the
   * one decision that matters on the retry path.)
   */
  consumer(): QueueConsumer<T> {
    const queue = this;
    const token = {};
    const leases: T[] = [];
    let cancelled = false;

    // Release a previous consumer's parked waiter — it belongs to an iterator
    // nobody is driving any more.
    if (queue.waiter) {
      const w = queue.waiter;
      queue.waiter = null;
      w({ value: undefined as unknown as T, done: true });
    }
    queue.active = token;

    return {
      get leased(): number {
        return leases.length;
      },
      ack(n?: number): void {
        if (n === undefined || n >= leases.length) {
          leases.length = 0;
          return;
        }
        if (n <= 0) return;
        leases.splice(0, n);
      },
      cancel(): void {
        if (cancelled) return;
        cancelled = true;
        // Front of the line, original order: these turns arrived before
        // everything currently buffered.
        if (leases.length > 0) {
          queue.items.unshift(...leases);
          log.warn(`queue: requeued ${leases.length} unacknowledged item(s) from a cancelled consumer`);
          leases.length = 0;
        }
        if (queue.active === token && queue.waiter) {
          const w = queue.waiter;
          queue.waiter = null;
          w({ value: undefined as unknown as T, done: true });
        }
        if (queue.active === token) queue.active = null;
      },
      [Symbol.asyncIterator](): AsyncIterator<T> {
        return {
          next: (): Promise<IteratorResult<T>> => {
            if (cancelled) {
              return Promise.resolve({ value: undefined as unknown as T, done: true });
            }
            if (queue.items.length > 0) {
              const value = queue.items.shift() as T;
              leases.push(value);
              return Promise.resolve({ value, done: false });
            }
            if (queue.closed) {
              return Promise.resolve({ value: undefined as unknown as T, done: true });
            }
            return new Promise<IteratorResult<T>>((resolve) => {
              queue.waiter = (result) => {
                // A cancelled consumer must not take delivery: the item would
                // vanish with it. Put it back and end this iterator instead.
                if (cancelled) {
                  if (!result.done) queue.items.unshift(result.value);
                  resolve({ value: undefined as unknown as T, done: true });
                  return;
                }
                if (!result.done) leases.push(result.value);
                resolve(result);
              };
            });
          },
        };
      },
    };
  }

  /** Direct iteration: an unleased consumer, for callers that cannot lose an
   * item because they cannot fail (and for the queue's own tests). Prefer
   * consumer() anywhere a failed attempt is retried. */
  [Symbol.asyncIterator](): AsyncIterator<T> {
    return this.consumer()[Symbol.asyncIterator]();
  }
}
