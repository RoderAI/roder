/** Offline tests for the dispatcher and task executor services: event
 * ordering, terminal shapes, error mapping, cancellation, and redaction —
 * all against the fake SDK through the injectable seam. */

import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

import { CursorCloudDispatcher, dispatcherDefinitions } from "../src/dispatcher.js";
import { createFakeSdkModule, resetFakeSdk } from "../src/fake.js";
import type { RpcWriter, SubagentEvent, TaskEvent } from "../src/protocol.js";
import { setSdkModule } from "../src/sdk.js";
import { CursorCloudTaskExecutor, taskSpec } from "../src/tasks.js";

class RecordingWriter implements RpcWriter {
  readonly notifications: Array<{ method: string; params: Record<string, unknown> }> = [];
  readonly replies: Array<{ id: unknown; result: unknown }> = [];

  reply(id: unknown, result: unknown): void {
    this.replies.push({ id, result });
  }

  replyError(_id: unknown, _message: string): void {}

  notify(method: string, params: unknown): void {
    this.notifications.push({ method, params: params as Record<string, unknown> });
  }

  subagentEvents(): SubagentEvent[] {
    return this.notifications
      .filter((n) => n.method === "subagents/event")
      .map((n) => n.params.event as SubagentEvent);
  }

  taskEvents(): TaskEvent[] {
    return this.notifications
      .filter((n) => n.method === "tasks/event")
      .map((n) => n.params.event as TaskEvent);
  }
}

beforeEach(() => {
  resetFakeSdk();
  setSdkModule(createFakeSdkModule());
  process.env.CURSOR_API_KEY = "crsr_test_key_for_offline_tests";
});

afterEach(() => {
  setSdkModule(null);
});

function dispatchParams(
  prompt: string,
  inputs: Record<string, unknown>,
): Parameters<CursorCloudDispatcher["dispatch"]>[0] {
  return {
    dispatcherId: "cursor-cloud",
    dispatchId: "dispatch-1",
    parentThreadId: "thread-1",
    parentTurnId: "turn-1",
    request: {
      description: "remote work",
      prompt,
      subagent_type: "cursor-cloud",
      model: "composer-2.5",
      inputs,
      timeout_seconds: 60,
    },
  };
}

test("dispatcher streams status events then a completed result", async () => {
  const writer = new RecordingWriter();
  const dispatcher = new CursorCloudDispatcher();
  await dispatcher.dispatch(
    dispatchParams("ship it", {
      repoUrl: "https://github.com/example-org/example-repo",
      autoCreatePr: true,
    }),
    writer,
  );

  const events = writer.subagentEvents();
  const statuses = events.filter((event) => event.type === "status");
  assert.ok(statuses.length >= 3, `expected progress statuses, got ${JSON.stringify(events)}`);
  assert.equal(statuses[0]?.status, "SUBMITTED");

  const terminal = events.at(-1);
  assert.ok(terminal?.type === "completed", JSON.stringify(terminal));
  const result = terminal.result;
  assert.match(result.thread_id, /^bc-fake-/);
  assert.equal(result.agent_type, "cursor-cloud");
  assert.equal(result.exit_reason, "completed");
  assert.equal(result.final_message, "Fake cloud agent completed: ship it");
  assert.deepEqual(result.metadata.prUrls, [
    "https://github.com/example-org/example-repo/pull/7",
  ]);
  assert.equal(result.metadata.resumed, false);
});

test("dispatcher maps validation and SDK errors to redacted failed events", async () => {
  const writer = new RecordingWriter();
  const dispatcher = new CursorCloudDispatcher();

  // Validation failure: no repoUrl.
  await dispatcher.dispatch(dispatchParams("go", {}), writer);
  let terminal = writer.subagentEvents().at(-1);
  assert.ok(terminal?.type === "failed");
  assert.match(terminal.error, /repoUrl is required/);

  // SDK throw with embedded secrets: redacted.
  const throwing = new RecordingWriter();
  await dispatcher.dispatch(
    dispatchParams("FAKE_SDK_THROW", {
      repoUrl: "https://github.com/example-org/example-repo",
    }),
    throwing,
  );
  terminal = throwing.subagentEvents().at(-1);
  assert.ok(terminal?.type === "failed");
  assert.ok(!terminal.error.includes("crsr_supersecretvalue1234"), terminal.error);
  assert.match(terminal.error, /crsr_\[REDACTED\]/);

  // Run finishing with status error: failed, not completed.
  const erroring = new RecordingWriter();
  await dispatcher.dispatch(
    dispatchParams("FAKE_SDK_RUN_ERROR", {
      repoUrl: "https://github.com/example-org/example-repo",
    }),
    erroring,
  );
  terminal = erroring.subagentEvents().at(-1);
  assert.ok(terminal?.type === "failed");
  assert.match(terminal.error, /finished with status error/);
});

test("task executor waits, logs progress, and returns the structured payload", async () => {
  const writer = new RecordingWriter();
  const executor = new CursorCloudTaskExecutor();
  await executor.execute(
    {
      executorId: "cursor-cloud-agent",
      executionId: "execution-1",
      taskId: "task-1",
      input: {
        prompt: "open a pr",
        repoUrl: "https://github.com/example-org/example-repo",
        autoCreatePr: true,
        model: "composer-2.5",
      },
    },
    writer,
  );

  const events = writer.taskEvents();
  const outputs = events.filter((event) => event.type === "output");
  assert.ok(
    outputs.some((event) => event.type === "output" && event.chunk.includes("created cloud agent bc-fake-")),
    JSON.stringify(outputs),
  );
  assert.ok(
    outputs.some((event) => event.type === "output" && event.chunk.includes("status: RUNNING")),
    JSON.stringify(outputs),
  );

  const terminal = events.at(-1);
  assert.ok(terminal?.type === "completed", JSON.stringify(terminal));
  const payload = terminal.result.payload;
  assert.match(String(payload.agentId), /^bc-fake-/);
  assert.equal(payload.status, "finished");
  assert.equal(payload.waited, true);
  assert.equal(payload.result, "Fake cloud agent completed: open a pr");
  assert.deepEqual(payload.prUrls, ["https://github.com/example-org/example-repo/pull/7"]);
});

test("task executor dispatch-and-return then resume by persisted agentId", async () => {
  const executor = new CursorCloudTaskExecutor();

  const first = new RecordingWriter();
  await executor.execute(
    {
      executorId: "cursor-cloud-agent",
      executionId: "execution-1",
      taskId: "task-1",
      input: {
        prompt: "long running work",
        repoUrl: "https://github.com/example-org/example-repo",
        wait: false,
      },
    },
    first,
  );
  const submitted = first.taskEvents().at(-1);
  assert.ok(submitted?.type === "completed");
  const agentId = String(submitted.result.payload.agentId);
  assert.match(agentId, /^bc-fake-/);
  assert.equal(submitted.result.payload.status, "running");
  assert.equal(submitted.result.payload.waited, false);

  const second = new RecordingWriter();
  await executor.execute(
    {
      executorId: "cursor-cloud-agent",
      executionId: "execution-2",
      taskId: "task-2",
      input: { prompt: "summarize what you did", agentId },
    },
    second,
  );
  const resumed = second.taskEvents().at(-1);
  assert.ok(resumed?.type === "completed", JSON.stringify(resumed));
  assert.equal(resumed.result.payload.agentId, agentId);
  assert.equal(resumed.result.payload.resumed, true);
  assert.equal(resumed.result.payload.status, "finished");
});

test("cancel stops an in-flight run and the result reports cancelled", async () => {
  const writer = new RecordingWriter();
  const dispatcher = new CursorCloudDispatcher();
  const pending = dispatcher.dispatch(
    dispatchParams("FAKE_SDK_SLOW work", {
      repoUrl: "https://github.com/example-org/example-repo",
    }),
    writer,
  );
  // Cancel once the run is registered (after the SUBMITTED status).
  for (let i = 0; i < 200; i += 1) {
    if (writer.subagentEvents().some((event) => event.type === "status")) {
      break;
    }
    await new Promise((resolve) => setTimeout(resolve, 1));
  }
  const cancelled = await dispatcher.cancel("dispatch-1");
  assert.equal(cancelled, true);
  await pending;

  const terminal = writer.subagentEvents().at(-1);
  assert.ok(terminal?.type === "completed", JSON.stringify(terminal));
  assert.equal(terminal.result.exit_reason, "cancelled");
});

test("definitions and task spec describe the services", () => {
  const definitions = dispatcherDefinitions();
  assert.equal(definitions.length, 1);
  assert.equal(definitions[0]?.agent_type, "cursor-cloud");

  const spec = taskSpec();
  assert.equal(spec.kind, "cursor-cloud-agent");
  assert.equal(spec.default_timeout_seconds, 1800);
  const schema = spec.input_schema as { required?: string[] };
  assert.deepEqual(schema.required, ["prompt"]);
});
