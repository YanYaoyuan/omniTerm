import test from "node:test";
import assert from "node:assert/strict";
import { operationsIn, pendingOperations, runTurn, settleTurn } from "../scripts/live-harness.mjs";

// A scripted agent over the same event list the live script uses.
function fixture(turns = []) {
  const events = [];
  const waiters = new Set();
  const notify = () => { for (const check of [...waiters]) check(); };
  const report = (event) => { events.push(event); notify(); };
  const until = (predicate, label, timeoutMs = 1000) => new Promise((resolve, reject) => {
    const check = () => { const value = predicate(); if (value) { clearTimeout(timer); waiters.delete(check); resolve(value); } };
    const timer = setTimeout(() => { waiters.delete(check); reject(new Error(`Timed out: ${label}`)); }, timeoutMs);
    waiters.add(check);
    check();
  });
  const agent = {
    conversation: null, cancels: 0, resets: 0,
    async send(request) {
      if (this.conversation?.turnId) throw new Error("The current turn is still running.");
      this.conversation = { turnId: "turn" };
      await turns.shift()?.(this, request);
    },
    finish(error = null) { this.conversation.turnId = null; report({ type: "turn-completed", error }); },
    async cancel() { this.cancels++; if (this.conversation?.stopsOnCancel !== false) this.finish(); },
    async reset() { this.resets++; this.conversation = null; },
  };
  return { events, report, until, agent };
}

const operation = (operationId, status) => ({ type: "operation", operation: { operationId, status } });

test("history of earlier turns is invisible to the approval pump", () => {
  const events = [operation("old", "pendingApproval"), operation("old", "denied"), operation("new", "pendingApproval")];
  assert.deepEqual(pendingOperations(events, 0).map(({ operationId }) => operationId), ["old", "new"]);
  assert.deepEqual(pendingOperations(events, 2).map(({ operationId }) => operationId), ["new"]);
  assert.deepEqual(operationsIn(events).map(({ status }) => status), ["pendingApproval", "denied", "pendingApproval"]);
});

test("a denied operation is not approved again by the next turn", async () => {
  const context = fixture([
    (agent) => { context.report(operation("denied-op", "pendingApproval")); setTimeout(() => { context.report(operation("denied-op", "denied")); agent.finish(); }, 150); },
    (agent) => { context.report(operation("next-op", "pendingApproval")); setTimeout(() => agent.finish(), 150); },
  ]);
  const seen = [];
  const onOperation = async ({ operationId }) => { seen.push(operationId); };
  const first = await runTurn({ ...context, request: {}, onOperation });
  const second = await runTurn({ ...context, request: {}, onOperation });
  assert.deepEqual(seen, ["denied-op", "next-op"]);
  assert.equal(first.finished.error, null);
  assert.equal(second.finished.error, null);
  assert.deepEqual(operationsIn(second.events).map(({ operationId }) => operationId), ["next-op"]);
});

test("a rejected approval stops that turn and the next one can still start", async () => {
  const context = fixture([
    () => { context.report(operation("op", "pendingApproval")); },
    (agent) => { agent.finish(); },
  ]);
  const { finished } = await runTurn({ ...context, request: {}, onOperation: async () => { throw new Error("This operation is no longer waiting for approval."); } });
  assert.equal(finished.type, "error");
  assert.equal(context.agent.cancels, 1);
  assert.equal((await runTurn({ ...context, request: {} })).finished.type, "turn-completed");
});

test("a turn that times out is cancelled before the failure is reported", async () => {
  const context = fixture([() => {}, (agent) => { agent.finish(); }]);
  await assert.rejects(runTurn({ ...context, request: {}, timeoutMs: 50 }), /Timed out/);
  assert.equal(context.agent.cancels, 1);
  assert.equal((await runTurn({ ...context, request: {} })).finished.type, "turn-completed");
});

test("a turn that ignores cancellation is torn down so later checks can run", async () => {
  const context = fixture([(agent) => { agent.conversation.stopsOnCancel = false; }]);
  await assert.rejects(runTurn({ ...context, request: {}, timeoutMs: 50, settleTimeoutMs: 50 }), /Timed out/);
  assert.equal(context.agent.resets, 1);
  assert.equal(context.agent.conversation, null);
});

test("a send that fails leaves nothing to clean up", async () => {
  const context = fixture();
  await settleTurn({ ...context, start: 0 });
  assert.equal(context.agent.cancels, 0);
  context.agent.send = async () => { throw new Error("no key"); };
  await assert.rejects(runTurn({ ...context, request: {} }), /no key/);
  assert.equal(context.agent.cancels, 0);
});
