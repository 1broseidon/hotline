/**
 * FB13 job cards for claude-sdk (spec workstream 1) — port of pi's
 * run-unit.mjs Part 6 onto the SDK's task_started/task_updated stream
 * messages instead of pi's extension bus. An injected `spawn` fake stands in
 * for `hotline job`; no real CLI runs.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  titleFrom,
  stateForStatus,
  startArgs,
  doneArgs,
  updateArgs,
  createJobCards,
  autoJobsEnabled,
} from "../dist/jobcards.js";

const silent = { info() {}, warn() {} };

function harness() {
  const spawned = [];
  const fakeSpawn = (command, args) => {
    spawned.push({ command, args });
    return { on() {}, unref() {} };
  };
  const jobCards = createJobCards({ binary: "hotline", spawn: fakeSpawn, log: silent });
  return { spawned, jobCards };
}

function taskStarted(over = {}) {
  return {
    type: "system",
    subtype: "task_started",
    task_id: "t1",
    description: "look into X",
    subagent_type: "researcher",
    session_id: "sess-42",
    uuid: "u1",
    ...over,
  };
}

function taskUpdated(over = {}) {
  return {
    type: "system",
    subtype: "task_updated",
    task_id: "t1",
    session_id: "sess-42",
    patch: {},
    uuid: "u2",
    ...over,
  };
}

// ---- Pure mappers ----------------------------------------------------------

test("title uses description", () => {
  assert.equal(titleFrom("do the thing", "researcher"), "do the thing");
});
test("title falls back to subagent_type", () => {
  assert.equal(titleFrom("", "researcher"), "researcher");
});
test("title falls back to a default", () => {
  assert.equal(titleFrom(undefined, undefined), "subagent");
});
test("title truncates to <=80 with ellipsis", () => {
  const t = titleFrom("x".repeat(200), "researcher");
  assert.equal(t.length, 80);
  assert.ok(t.endsWith("…"));
});

test("stateForStatus maps completed/failed/killed", () => {
  assert.equal(stateForStatus("completed"), "ok");
  assert.equal(stateForStatus("failed"), "err");
  assert.equal(stateForStatus("killed"), "cancelled");
});
test("stateForStatus maps non-terminal statuses to null", () => {
  assert.equal(stateForStatus("pending"), null);
  assert.equal(stateForStatus("running"), null);
  assert.equal(stateForStatus("paused"), null);
  assert.equal(stateForStatus(undefined), null);
});

test("startArgs shape", () => {
  assert.deepEqual(startArgs("c1", "b1", "T"), ["job", "start", "--cookie", "c1", "--batch", "b1", "--title", "T"]);
});
test("doneArgs always carries --batch (mis-card bug guard)", () => {
  assert.ok(doneArgs("c1", "b1", "ok").includes("--batch"));
  assert.ok(doneArgs("c1", "b1", "ok").includes("b1"));
});
test("updateArgs shape", () => {
  assert.deepEqual(updateArgs("c1", "d"), ["job", "update", "--cookie", "c1", "--detail", "d"]);
});

// ---- HOTLINE_AUTO_JOBS opt-out ---------------------------------------------

test("autoJobsEnabled defaults to true", () => {
  assert.equal(autoJobsEnabled({}), true);
});
for (const v of ["0", "false", "off", "no", "FALSE", "Off"]) {
  test(`autoJobsEnabled(${v}) opts out`, () => {
    assert.equal(autoJobsEnabled({ HOTLINE_AUTO_JOBS: v }), false);
  });
}
test("autoJobsEnabled(1) stays enabled", () => {
  assert.equal(autoJobsEnabled({ HOTLINE_AUTO_JOBS: "1" }), true);
});

// ---- Wiring: task_started/task_updated -> hotline job argv -----------------

test("task_started (subagent) -> job start", () => {
  const { spawned, jobCards } = harness();
  jobCards.onMessage(taskStarted());
  assert.equal(spawned.length, 1);
  assert.equal(spawned[0].args[1], "start");
  assert.ok(spawned[0].args.includes("t1"));
  assert.equal(spawned[0].args[spawned[0].args.indexOf("--batch") + 1], "sess-42");
  assert.equal(spawned[0].args[spawned[0].args.indexOf("--title") + 1], "look into X");
});

test("task_started without subagent_type (ambient/housekeeping) is not carded", () => {
  const { spawned, jobCards } = harness();
  jobCards.onMessage(taskStarted({ subagent_type: undefined }));
  assert.equal(spawned.length, 0);
});

test("task_started with skip_transcript is not carded", () => {
  const { spawned, jobCards } = harness();
  jobCards.onMessage(taskStarted({ skip_transcript: true }));
  assert.equal(spawned.length, 0);
});

test("task_started with task_type local_workflow is not carded", () => {
  const { spawned, jobCards } = harness();
  jobCards.onMessage(taskStarted({ task_type: "local_workflow" }));
  assert.equal(spawned.length, 0);
});

test("task_started with no task_id is ignored", () => {
  const { spawned, jobCards } = harness();
  jobCards.onMessage(taskStarted({ task_id: undefined }));
  assert.equal(spawned.length, 0);
});

test("task_updated completed -> job done ok", () => {
  const { spawned, jobCards } = harness();
  jobCards.onMessage(taskStarted());
  jobCards.onMessage(taskUpdated({ patch: { status: "completed" } }));
  assert.equal(spawned.length, 2);
  assert.ok(spawned[1].args.includes("done"));
  assert.ok(spawned[1].args.includes("ok"));
  assert.equal(spawned[1].args[spawned[1].args.indexOf("--batch") + 1], "sess-42");
});

test("task_updated failed -> job done err", () => {
  const { spawned, jobCards } = harness();
  jobCards.onMessage(taskStarted());
  jobCards.onMessage(taskUpdated({ patch: { status: "failed" } }));
  assert.ok(spawned[1].args.includes("done"));
  assert.ok(spawned[1].args.includes("err"));
});

test("task_updated killed -> job done cancelled", () => {
  const { spawned, jobCards } = harness();
  jobCards.onMessage(taskStarted());
  jobCards.onMessage(taskUpdated({ patch: { status: "killed" } }));
  assert.ok(spawned[1].args.includes("done"));
  assert.ok(spawned[1].args.includes("cancelled"));
});

test("task_updated description change -> job update", () => {
  const { spawned, jobCards } = harness();
  jobCards.onMessage(taskStarted());
  jobCards.onMessage(taskUpdated({ patch: { description: "still working" } }));
  assert.equal(spawned.length, 2);
  assert.ok(spawned[1].args.includes("update"));
  assert.ok(spawned[1].args.includes("--detail"));
  assert.equal(spawned[1].args[spawned[1].args.indexOf("--detail") + 1], "still working");
});

test("task_updated paused alone is skipped as noisy", () => {
  const { spawned, jobCards } = harness();
  jobCards.onMessage(taskStarted());
  jobCards.onMessage(taskUpdated({ patch: { status: "paused" } }));
  assert.equal(spawned.length, 1); // only the start
});

test("task_updated for an uncarded (unknown) cookie is ignored", () => {
  const { spawned, jobCards } = harness();
  // No task_started for t1 first.
  jobCards.onMessage(taskUpdated({ patch: { status: "completed" } }));
  assert.equal(spawned.length, 0);
});

test("a terminal transition fires done at most once", () => {
  const { spawned, jobCards } = harness();
  jobCards.onMessage(taskStarted());
  jobCards.onMessage(taskUpdated({ patch: { status: "completed" } }));
  jobCards.onMessage(taskUpdated({ patch: { status: "completed" } })); // duplicate/late
  jobCards.onMessage(taskUpdated({ patch: { status: "failed" } })); // also late
  assert.equal(spawned.length, 2, "the duplicate/late terminal transitions must not re-fire done");
});

test("non-system / non-task messages are ignored", () => {
  const { spawned, jobCards } = harness();
  jobCards.onMessage({ type: "assistant", message: { content: [] } });
  jobCards.onMessage({ type: "system", subtype: "init" });
  assert.equal(spawned.length, 0);
});

test("spawn throws are swallowed (best-effort contract)", () => {
  const jobCards = createJobCards({
    binary: "hotline",
    spawn: () => {
      throw new Error("spawn boom");
    },
    log: silent,
  });
  assert.doesNotThrow(() => jobCards.onMessage(taskStarted()));
});

test("an onMessage handler that throws internally never escapes", () => {
  const jobCards = createJobCards({ binary: "hotline", spawn: () => ({ on() {}, unref() {} }), log: silent });
  // A message shaped to blow up field access (patch is a string, not an
  // object) must not throw out of onMessage.
  assert.doesNotThrow(() => jobCards.onMessage(taskUpdated({ patch: "not-an-object" })));
});

test("dispose clears the started-cookie tracking (a later update reads as unknown)", () => {
  const { spawned, jobCards } = harness();
  jobCards.onMessage(taskStarted());
  jobCards.dispose();
  jobCards.onMessage(taskUpdated({ patch: { status: "completed" } }));
  assert.equal(spawned.length, 1, "only the start; dispose dropped tracking so the update is now unknown");
});
