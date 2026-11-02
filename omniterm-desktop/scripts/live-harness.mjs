// Turn handling for test-agent-live.mjs, apart from it so the logic can be tested offline.
const TURN_TIMEOUT_MS = 240_000;
const SETTLE_TIMEOUT_MS = 20_000;
const POLL_MS = 100;

export const operationsIn = (events) => events.filter(({ type }) => type === "operation").map(({ operation }) => operation);

// Events are append-only, so an operation stays `pendingApproval` in history after it is decided.
// Each turn may only act on what happened during that turn.
export const pendingOperations = (events, start) => operationsIn(events.slice(start)).filter(({ status }) => status === "pendingApproval");

// Leaves no turn active: a turn that outlives its check would fail every check after it.
export async function settleTurn({ agent, events, until, start, timeoutMs = SETTLE_TIMEOUT_MS }) {
  if (!agent.conversation?.turnId) return;
  await agent.cancel().catch(() => {});
  try {
    await until(() => !agent.conversation?.turnId || events.slice(start).some(({ type }) => type === "turn-completed"), "the failed turn must stop", timeoutMs);
  } catch { await agent.reset(); }
}

export async function runTurn({ agent, events, until, report, request, onOperation = async () => {}, timeoutMs = TURN_TIMEOUT_MS, settleTimeoutMs = SETTLE_TIMEOUT_MS }) {
  const start = events.length;
  const handled = new Set();
  const pump = setInterval(() => {
    for (const operation of pendingOperations(events, start)) {
      if (handled.has(operation.operationId)) continue;
      handled.add(operation.operationId);
      onOperation(operation).catch((error) => report({ type: "error", message: error.message }));
    }
  }, POLL_MS);
  const settle = () => settleTurn({ agent, events, until, start, timeoutMs: settleTimeoutMs });
  try {
    await agent.send(request);
    const finished = await until(() => events.slice(start).find(({ type }) => type === "turn-completed" || type === "error"), "the turn must finish", timeoutMs);
    if (finished.type === "error") await settle();
    return { finished, events: events.slice(start) };
  } catch (error) {
    await settle();
    throw error;
  } finally { clearInterval(pump); }
}
