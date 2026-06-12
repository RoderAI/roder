import assert from "node:assert/strict";
import test from "node:test";
import { createPartTransformer } from "../src/index.js";
import type { AgentPart, RoderSdkEvent } from "../src/index.js";
import type { JsonRpcNotification } from "../src/index.js";
import type { ThreadItem, ThreadItemDelta } from "../src/index.js";

const raw: JsonRpcNotification = { jsonrpc: "2.0", method: "fixture" };

function delta(itemId: string, body: ThreadItemDelta): RoderSdkEvent {
  return {
    type: "item.delta",
    seq: 0,
    eventId: "e",
    threadId: "t",
    turnId: "u",
    timestamp: "1970-01-01T00:00:00Z",
    itemId,
    delta: body,
    raw,
  };
}

function started(item: ThreadItem): RoderSdkEvent {
  return {
    type: "item.started",
    seq: 0,
    eventId: "e",
    threadId: "t",
    turnId: "u",
    timestamp: "1970-01-01T00:00:00Z",
    item,
    raw,
  };
}

function completed(item: ThreadItem): RoderSdkEvent {
  return {
    type: "item.completed",
    seq: 0,
    eventId: "e",
    threadId: "t",
    turnId: "u",
    timestamp: "1970-01-01T00:00:00Z",
    item,
    raw,
  };
}

function run(events: RoderSdkEvent[]): AgentPart[] {
  const transformer = createPartTransformer();
  const parts = events.flatMap((event) => transformer.push(event));
  return [...parts, ...transformer.flush()];
}

test("agent-message text reusing one item id across a tool splits into two parts", () => {
  const A = "u-agent-final_answer";
  const tool = { type: "toolExecution", id: "u-tool-1", toolCallId: "call-1", toolName: "write_file", status: "completed" } as const;

  const parts = run([
    delta(A, { type: "agentMessageText", delta: "Starting now." }),
    started(tool),
    completed(tool),
    delta(A, { type: "agentMessageText", delta: "Done." }),
    completed({ type: "agentMessage", id: A, text: "Starting now.Done.", status: "completed" }),
  ]);

  // The tool renders BETWEEN the two text runs, each text run a distinct part.
  assert.deepEqual(
    parts.map((p) => [p.type, p.id]),
    [
      ["text-start", A],
      ["text-delta", A],
      ["text-end", A],
      ["tool-start", "u-tool-1"],
      ["tool-end", "u-tool-1"],
      ["text-start", `${A}__seg1`],
      ["text-delta", `${A}__seg1`],
      ["text-end", `${A}__seg1`],
    ],
  );
});

test("reasoning passes through on its own server ids, not __seg-split", () => {
  const tool = { type: "toolExecution", id: "u-tool-1", toolCallId: "call-1", toolName: "bash", status: "completed" } as const;

  const parts = run([
    delta("u-reasoning-1", { type: "reasoningText", delta: "think a", contentIndex: 0 }),
    started(tool),
    completed(tool),
    delta("u-reasoning-2", { type: "reasoningText", delta: "think b", contentIndex: 0 }),
    completed({ type: "reasoning", id: "u-reasoning-2", status: "completed" }),
  ]);

  assert.deepEqual(
    parts.map((p) => [p.type, p.id]),
    [
      ["reasoning-start", "u-reasoning-1"],
      ["reasoning-delta", "u-reasoning-1"],
      ["reasoning-end", "u-reasoning-1"],
      ["tool-start", "u-tool-1"],
      ["tool-end", "u-tool-1"],
      ["reasoning-start", "u-reasoning-2"],
      ["reasoning-delta", "u-reasoning-2"],
      ["reasoning-end", "u-reasoning-2"],
    ],
  );
});

test("flush closes a text part left open by an unterminated stream", () => {
  const A = "u-agent-final_answer";
  const parts = run([delta(A, { type: "agentMessageText", delta: "partial" })]);

  assert.deepEqual(
    parts.map((p) => p.type),
    ["text-start", "text-delta", "text-end"],
  );
});

test("tool-start carries the tool name and input", () => {
  const transformer = createPartTransformer();
  const out = transformer.push(
    started({
      type: "toolExecution",
      id: "u-tool-1",
      toolCallId: "call-1",
      toolName: "write_file",
      status: "inProgress",
      input: { path: "a.md" },
    }),
  );
  assert.deepEqual(out, [
    { type: "tool-start", id: "u-tool-1", toolCallId: "call-1", toolName: "write_file", input: { path: "a.md" } },
  ]);
});
