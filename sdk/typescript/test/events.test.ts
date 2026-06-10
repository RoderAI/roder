import assert from "node:assert/strict";
import test from "node:test";
import { normalizeNotification } from "../src/index.js";
import type { JsonRpcNotification, RoderSdkEvent } from "../src/index.js";

/**
 * Verbatim app-server notification lines captured from live Anthropic runs
 * (host-app migration spike transcripts, stage 0 and stage 2). They pin the
 * typed event layer to the real wire format.
 */
const TRANSCRIPT = {
  threadStarted:
    '{"jsonrpc":"2.0","method":"thread/started","params":{"thread":{"id":"2f210784-8681-4299-b110-9ae3eceded32","preview":"Untitled thread","modelProvider":"anthropic","model":"claude-haiku-4-5-20251001","createdAt":1781027441,"updatedAt":1781027441,"status":{"type":"idle","activeTurnId":null,"activeFlags":[]},"cwd":"/private/tmp/roder-stage0","workspaceId":"ws_16fe3403bee295cf","rootId":"root_16fe3403bee295cf","messageCount":0}}}',
  turnStarted:
    '{"jsonrpc":"2.0","method":"turn/started","params":{"threadId":"2f210784-8681-4299-b110-9ae3eceded32","turn":{"id":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1","items":[],"itemsView":"default","status":"inProgress","startedAt":1781027441}}}',
  statusChanged:
    '{"jsonrpc":"2.0","method":"thread/status/changed","params":{"threadId":"2f210784-8681-4299-b110-9ae3eceded32","status":{"type":"running","activeTurnId":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1","activeFlags":[]}}}',
  userMessageCompleted:
    '{"jsonrpc":"2.0","method":"item/completed","params":{"seq":1,"eventId":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1-item-event-1","threadId":"2f210784-8681-4299-b110-9ae3eceded32","turnId":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1","timestamp":"2026-06-09T17:50:41.419253Z","event":{"type":"itemCompleted","item":{"type":"userMessage","id":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1-user","text":"Reply with exactly the string RODER-STAGE0-OK followed by the value of 2+2.","status":"completed"}}}}',
  agentMessageStarted:
    '{"jsonrpc":"2.0","method":"item/started","params":{"seq":2,"eventId":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1-item-event-2","threadId":"2f210784-8681-4299-b110-9ae3eceded32","turnId":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1","timestamp":"2026-06-09T17:50:42.958491Z","event":{"type":"itemStarted","item":{"type":"agentMessage","id":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1-agent-final_answer","text":"","status":"inProgress"}}}}',
  agentMessageDelta:
    '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"seq":3,"eventId":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1-item-event-3","threadId":"2f210784-8681-4299-b110-9ae3eceded32","turnId":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1","timestamp":"2026-06-09T17:50:42.958491Z","event":{"type":"itemDelta","itemId":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1-agent-final_answer","delta":{"type":"agentMessageText","delta":"RODER-STAGE0-OK4"}}}}',
  rawItemCompleted:
    '{"jsonrpc":"2.0","method":"item/completed","params":{"seq":5,"eventId":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1-item-event-5","threadId":"2f210784-8681-4299-b110-9ae3eceded32","turnId":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1","timestamp":"2026-06-09T17:50:42.959585Z","event":{"type":"itemCompleted","item":{"type":"raw","id":"7ca424cb-61f8-4cc1-a427-dfe96a5081e1-item-2","payload":{"ProviderMetadata":{"kind":"model_profile_segment","segment":"assistant","provider":"anthropic","model":"claude-haiku-4-5-20251001"}},"status":"completed"}}}}',
  toolExecutionStarted:
    '{"jsonrpc":"2.0","method":"item/started","params":{"seq":3,"eventId":"7167bd43-7b61-4a52-9b11-93f58f621638-item-event-3","threadId":"8edbd7d6-5b07-438f-a319-ad9abe99ac5c","turnId":"7167bd43-7b61-4a52-9b11-93f58f621638","timestamp":"2026-06-09T19:04:14.074917Z","event":{"type":"itemStarted","item":{"type":"toolExecution","id":"toolu_01W2dtQxyZxqpxXeqWc26Qia","toolCallId":"toolu_01W2dtQxyZxqpxXeqWc26Qia","toolName":"read_file","status":"inProgress","input":{"path":"notes.md"}}}}}',
  toolExecutionCompleted:
    '{"jsonrpc":"2.0","method":"item/completed","params":{"seq":5,"eventId":"7167bd43-7b61-4a52-9b11-93f58f621638-item-event-5","threadId":"8edbd7d6-5b07-438f-a319-ad9abe99ac5c","turnId":"7167bd43-7b61-4a52-9b11-93f58f621638","timestamp":"2026-06-09T19:04:14.077314Z","event":{"type":"itemCompleted","item":{"type":"toolExecution","id":"toolu_01W2dtQxyZxqpxXeqWc26Qia","toolCallId":"toolu_01W2dtQxyZxqpxXeqWc26Qia","toolName":"read_file","status":"completed","input":{"path":"notes.md","shown":3,"total_lines":3,"truncated":false},"output":"    1: # Probe Notes\\n    2: The magic word is PINEAPPLE-42.\\n    3: Second line for context."}}}}',
  turnCompleted:
    '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"8edbd7d6-5b07-438f-a319-ad9abe99ac5c","turn":{"id":"7167bd43-7b61-4a52-9b11-93f58f621638","items":[],"itemsView":"default","status":"completed","completedAt":1781031855,"usage":{"prompt_tokens":24889,"completion_tokens":72,"total_tokens":24961,"cached_prompt_tokens":12392,"cache_hit_rate":0.49789063441681064}}}}',
};

function parseLine(line: string, mode: "strict" | "permissive" = "permissive"): RoderSdkEvent | undefined {
  return normalizeNotification(JSON.parse(line) as JsonRpcNotification, mode);
}

test("thread/started maps to a typed thread", () => {
  const event = parseLine(TRANSCRIPT.threadStarted);
  assert.ok(event?.type === "thread.started");
  assert.equal(event.thread.id, "2f210784-8681-4299-b110-9ae3eceded32");
  assert.equal(event.thread.modelProvider, "anthropic");
  assert.equal(event.thread.model, "claude-haiku-4-5-20251001");
  assert.equal(event.thread.status.type, "idle");
  assert.equal(event.thread.status.activeTurnId, null);
  assert.equal(event.raw.method, "thread/started");
});

test("turn/started maps to a typed turn", () => {
  const event = parseLine(TRANSCRIPT.turnStarted);
  assert.ok(event?.type === "turn.started");
  assert.equal(event.threadId, "2f210784-8681-4299-b110-9ae3eceded32");
  assert.equal(event.turn.id, "7ca424cb-61f8-4cc1-a427-dfe96a5081e1");
  assert.equal(event.turn.status, "inProgress");
  assert.equal(event.turn.startedAt, 1781027441);
});

test("thread/status/changed maps to a typed status", () => {
  const event = parseLine(TRANSCRIPT.statusChanged);
  assert.ok(event?.type === "thread.status.changed");
  assert.equal(event.threadId, "2f210784-8681-4299-b110-9ae3eceded32");
  assert.equal(event.status.type, "running");
  assert.equal(event.status.activeTurnId, "7ca424cb-61f8-4cc1-a427-dfe96a5081e1");
});

test("item/completed maps user message items", () => {
  const event = parseLine(TRANSCRIPT.userMessageCompleted);
  assert.ok(event?.type === "item.completed");
  assert.equal(event.seq, 1);
  assert.equal(event.eventId, "7ca424cb-61f8-4cc1-a427-dfe96a5081e1-item-event-1");
  assert.equal(event.turnId, "7ca424cb-61f8-4cc1-a427-dfe96a5081e1");
  assert.equal(event.timestamp, "2026-06-09T17:50:41.419253Z");
  assert.ok(event.item.type === "userMessage");
  assert.match(event.item.text, /RODER-STAGE0-OK/);
});

test("item/started maps agent message items", () => {
  const event = parseLine(TRANSCRIPT.agentMessageStarted);
  assert.ok(event?.type === "item.started");
  assert.ok(event.item.type === "agentMessage");
  assert.equal(event.item.id, "7ca424cb-61f8-4cc1-a427-dfe96a5081e1-agent-final_answer");
  assert.equal(event.item.status, "inProgress");
});

test("item/agentMessage/delta maps to a typed text delta", () => {
  const event = parseLine(TRANSCRIPT.agentMessageDelta);
  assert.ok(event?.type === "item.delta");
  assert.equal(event.itemId, "7ca424cb-61f8-4cc1-a427-dfe96a5081e1-agent-final_answer");
  assert.ok(event.delta.type === "agentMessageText");
  assert.equal(event.delta.delta, "RODER-STAGE0-OK4");
});

test("item/completed keeps raw items with their payload", () => {
  const event = parseLine(TRANSCRIPT.rawItemCompleted);
  assert.ok(event?.type === "item.completed");
  assert.ok(event.item.type === "raw");
  assert.deepEqual(event.item.payload, {
    ProviderMetadata: {
      kind: "model_profile_segment",
      segment: "assistant",
      provider: "anthropic",
      model: "claude-haiku-4-5-20251001",
    },
  });
});

test("item events map tool executions with input and output", () => {
  const started = parseLine(TRANSCRIPT.toolExecutionStarted);
  assert.ok(started?.type === "item.started");
  assert.ok(started.item.type === "toolExecution");
  assert.equal(started.item.toolCallId, "toolu_01W2dtQxyZxqpxXeqWc26Qia");
  assert.equal(started.item.toolName, "read_file");
  assert.deepEqual(started.item.input, { path: "notes.md" });

  const completed = parseLine(TRANSCRIPT.toolExecutionCompleted);
  assert.ok(completed?.type === "item.completed");
  assert.ok(completed.item.type === "toolExecution");
  assert.equal(completed.item.status, "completed");
  assert.match(completed.item.output ?? "", /PINEAPPLE-42/);
});

test("turn/completed maps usage from a live transcript", () => {
  const event = parseLine(TRANSCRIPT.turnCompleted);
  assert.ok(event?.type === "turn.completed");
  assert.equal(event.turn.status, "completed");
  assert.deepEqual(event.turn.usage, {
    prompt_tokens: 24889,
    completion_tokens: 72,
    total_tokens: 24961,
    cached_prompt_tokens: 12392,
    cache_hit_rate: 0.49789063441681064,
  });
});

test("turn/completed maps finishReason, cache-write usage, and failure error", () => {
  const event = normalizeNotification({
    jsonrpc: "2.0",
    method: "turn/completed",
    params: {
      threadId: "thread-1",
      turn: {
        id: "turn-1",
        items: [],
        itemsView: "default",
        status: "failed",
        completedAt: 1781031855,
        error: { message: "provider returned 500" },
        usage: {
          prompt_tokens: 100,
          completion_tokens: 10,
          total_tokens: 110,
          cached_prompt_tokens: 92,
          cache_creation_prompt_tokens: 5,
          cache_hit_rate: 0.92,
        },
        finishReason: "stop",
      },
    },
  });
  assert.ok(event?.type === "turn.completed");
  assert.equal(event.turn.finishReason, "stop");
  assert.equal(event.turn.usage?.cache_creation_prompt_tokens, 5);
  assert.equal(event.turn.error?.message, "provider returned 500");
});

test("item/reasoning deltas map to typed reasoning deltas", () => {
  const event = normalizeNotification({
    jsonrpc: "2.0",
    method: "item/reasoning/textDelta",
    params: {
      seq: 4,
      eventId: "turn-1-item-event-4",
      threadId: "thread-1",
      turnId: "turn-1",
      timestamp: "1970-01-01T00:00:00Z",
      event: {
        type: "itemDelta",
        itemId: "turn-1-agent-reasoning",
        delta: { type: "reasoningText", delta: "Inspecting", contentIndex: 0 },
      },
    },
  });
  assert.ok(event?.type === "item.delta");
  assert.ok(event.delta.type === "reasoningText");
  assert.equal(event.delta.delta, "Inspecting");
  assert.equal(event.delta.contentIndex, 0);
});

test("thread/toolExecutionRequested maps to a typed call", () => {
  const event = normalizeNotification({
    jsonrpc: "2.0",
    method: "thread/toolExecutionRequested",
    params: {
      threadId: "thread-external",
      turnId: "turn-external",
      requestId: "exttool-1",
      call: { id: "call-1", name: "acme_lookup", arguments: { query: "thread status" } },
    },
  });
  assert.ok(event?.type === "tool_execution.requested");
  assert.equal(event.requestId, "exttool-1");
  assert.deepEqual(event.call, {
    id: "call-1",
    name: "acme_lookup",
    arguments: { query: "thread status" },
  });
});

test("thread/toolExecutionResolved maps to a typed outcome", () => {
  const event = normalizeNotification({
    jsonrpc: "2.0",
    method: "thread/toolExecutionResolved",
    params: {
      threadId: "thread-external",
      turnId: "turn-external",
      requestId: "exttool-1",
      toolId: "call-1",
      toolName: "acme_lookup",
      outcome: "resolved",
      isError: false,
    },
  });
  assert.ok(event?.type === "tool_execution.resolved");
  assert.equal(event.outcome, "resolved");
  assert.equal(event.isError, false);
});

test("unknown methods flow as raw notifications in permissive mode only", () => {
  const notification: JsonRpcNotification = {
    jsonrpc: "2.0",
    method: "workspace/changeObserved",
    params: {},
  };
  assert.equal(normalizeNotification(notification)?.type, "raw.notification");
  assert.equal(normalizeNotification(notification, "strict"), undefined);
});

test("known methods with malformed payloads degrade like unknown methods", () => {
  const malformed: JsonRpcNotification = {
    jsonrpc: "2.0",
    method: "turn/completed",
    params: { threadId: "thread-1" },
  };
  assert.equal(normalizeNotification(malformed)?.type, "raw.notification");
  assert.equal(normalizeNotification(malformed, "strict"), undefined);
});
