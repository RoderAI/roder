import assert from "node:assert/strict";
import test from "node:test";
import { EventQueue, promptBlocks, runtimeEvent } from "../src/events.mjs";

test("queued events drain in order and an empty waiter wakes on end", async () => {
  const queue = new EventQueue();
  queue.push({ type: "status", text: "first" });
  queue.push({ type: "status", text: "second" });
  queue.end();
  assert.deepEqual((await Array.fromAsync(queue)).map((event) => event.text), ["first", "second"]);
  const empty = new EventQueue();
  const result = Array.fromAsync(empty);
  empty.end();
  assert.deepEqual(await result, []);
});

test("discarding a queue releases pending output and rejects unbounded consumers", async () => {
  const queue = new EventQueue();
  for (let i = 0; i < 4096; i++) queue.push({ type: "status", text: "queued" });
  assert.throws(() => queue.push({ type: "status", text: "overflow" }), /not keeping up/);
  queue.end(true);
  queue.push({ type: "status", text: "ignored" });
  assert.deepEqual(await Array.fromAsync(queue), []);
  const bytes = new EventQueue();
  assert.throws(() => bytes.push({ text: "x".repeat(16 * 1024 * 1024) }), /not keeping up/);
  bytes.end(true);
});

test("maps tool content without embedding image bytes and ignores user-history replay", () => {
  const event = runtimeEvent({
    sessionUpdate: "tool_call_update", toolCallId: "tool", status: "completed",
    content: [
      { type: "content", content: { type: "text", text: "stdout" } },
      { type: "diff", path: "src/test.txt", newText: "replacement" },
      { type: "content", content: { type: "image", data: "never-inline-image-data" } },
    ],
  });
  assert.equal(event.toolCallId, "tool");
  assert.match(event.text, /stdout\nsrc\/test.txt\nreplacement/);
  assert.doesNotMatch(event.text, /never-inline/);
  assert.equal(runtimeEvent({ sessionUpdate: "user_message_chunk" }), undefined);
  assert.equal(runtimeEvent({ sessionUpdate: "agent_message_chunk", content: { type: "image" } }), undefined);
  assert.equal(runtimeEvent({ sessionUpdate: "unknown" }), undefined);
});

test("preserves commands and session information as tagged status events", () => {
  assert.deepEqual(runtimeEvent({
    sessionUpdate: "available_commands_update",
    availableCommands: [{ name: "help", description: "Show help" }],
  }), { type: "status", tag: "available_commands_update", text: "/help: Show help" });
  assert.deepEqual(runtimeEvent({ sessionUpdate: "session_info_update", title: "Test session" }), {
    type: "status", tag: "session_info_update", text: "Test session",
  });
});

test("prompt conversion preserves text and canonical image bytes but rejects unsupported inputs", () => {
  assert.deepEqual(promptBlocks("hello", [{ mediaType: "image/png", data: "aGVsbG8=" }]), [
    { type: "text", text: "hello" },
    { type: "image", mimeType: "image/png", data: "aGVsbG8=" },
  ]);
  for (const attachment of [
    { mediaType: "audio/wav", data: "aGVsbG8=" },
    { mediaType: "image/png", data: "aGVsbG8" },
    { mediaType: "image/png", data: "data:image/png;base64,aGVsbG8=" },
    { mediaType: "image/png", data: "" },
  ]) assert.throws(() => promptBlocks("hello", [attachment]), /Attachments/);
  assert.throws(() => promptBlocks(null), /needs text/);
});
