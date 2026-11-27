import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { chmod, mkdtemp, mkdir, readFile, readdir, rm, stat, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { ownerKey, SessionStore } from "../src/store.mjs";

function record(overrides = {}) {
  return {
    version: 1,
    sessionKey: "conversation",
    sessionId: "acp-session",
    cwd: process.cwd(),
    mode: "persistent",
    binding: "a".repeat(64),
    generation: "generation-1",
    ...overrides,
  };
}

async function fixture(t) {
  const root = await mkdtemp(path.join(os.tmpdir(), "roder-store-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const directory = path.join(root, "sessions");
  return { root, directory, store: new SessionStore(directory) };
}

test("owner keys are canonical and isolate agents", () => {
  assert.equal(ownerKey({ sessionKey: "x" }), '["","x"]');
  assert.notEqual(ownerKey({ sessionKey: "x", agentId: "a" }),
    ownerKey({ sessionKey: "x", agentId: "b" }));
  for (const input of [null, {}, { sessionKey: "" }, { sessionKey: 1 },
    { sessionKey: "x", agentId: "" }, { sessionKey: "x", agentId: null }]) {
    assert.throws(() => ownerKey(input));
  }
});

test("round trips private records, overwrites atomically, and deletes idempotently", async (t) => {
  const { directory, store } = await fixture(t);
  const value = record();
  const key = ownerKey(value);
  assert.equal(await store.read(key), undefined);
  await store.delete(key);
  await store.write(key, value);
  assert.deepEqual(await store.read(key), value);
  assert.equal((await stat(directory)).mode & 0o777, 0o700);
  const [filename] = await readdir(directory);
  assert.equal((await stat(path.join(directory, filename))).mode & 0o777, 0o600);
  const updated = record({ sessionId: "replacement", mode: "oneshot" });
  await store.write(key, updated);
  assert.deepEqual(await store.read(key), updated);
  assert.deepEqual(await readdir(directory), [filename]);
  await store.delete(key);
  await store.delete(key);
  assert.equal(await store.read(key), undefined);
});

test("hashes owner input instead of interpreting paths", async (t) => {
  const { directory, store } = await fixture(t);
  const value = record({ sessionKey: "../../escaped/file" });
  const key = ownerKey(value);
  await store.write(key, value);
  const expected = `${createHash("sha256").update(key).digest("hex")}.json`;
  assert.deepEqual(await readdir(directory), [expected]);
  assert.deepEqual(await store.read(key), value);
});

test("isolates owners and rejects records copied across owners", async (t) => {
  const { store } = await fixture(t);
  const first = record({ agentId: "first" });
  const second = record({ agentId: "second" });
  await store.write(ownerKey(first), first);
  assert.equal(await store.read(ownerKey(second)), undefined);
  await assert.rejects(store.write(ownerKey(second), first), /owner mismatch/);
  await writeFile(store.filename(ownerKey(second)), JSON.stringify(first), { mode: 0o600 });
  await assert.rejects(store.read(ownerKey(second)), /owner mismatch/);
});

test("corrupt and invalid records fail closed without changing stored bytes", async (t) => {
  const { store } = await fixture(t);
  const key = ownerKey(record());
  await store.write(key, record());
  const invalid = [
    { version: 2 }, { sessionId: "" }, { generation: "" }, { cwd: "relative" },
    { mode: "invalid" }, { binding: "not-a-hash" }, { unexpected: true },
  ];
  for (const override of invalid) {
    await assert.rejects(store.write(key, record(override)));
    assert.deepEqual(await store.read(key), record());
    const bytes = JSON.stringify(record(override));
    await writeFile(store.filename(key), bytes);
    await assert.rejects(store.read(key));
    assert.equal(await readFile(store.filename(key), "utf8"), bytes);
    await store.write(key, record());
  }
  await writeFile(store.filename(key), "{broken");
  await assert.rejects(store.read(key), SyntaxError);
  assert.equal(await readFile(store.filename(key), "utf8"), "{broken");
});

test("refuses symlink store directories", async (t) => {
  const { root, directory, store } = await fixture(t);
  const target = path.join(root, "target");
  await mkdir(target);
  await symlink(target, directory);
  const key = ownerKey(record());
  await assert.rejects(store.read(key), /symlink/);
  await assert.rejects(store.write(key, record()), /symlink/);
  await assert.rejects(store.delete(key), /symlink/);
  assert.deepEqual(await readdir(target), []);
});

test("refuses symlink session files without modifying targets", async (t) => {
  const { root, directory, store } = await fixture(t);
  await mkdir(directory);
  const target = path.join(root, "target");
  await writeFile(target, "untouched");
  const key = ownerKey(record());
  await symlink(target, store.filename(key));
  await assert.rejects(store.read(key), /symlink/);
  await assert.rejects(store.write(key, record()), /symlink/);
  await assert.rejects(store.delete(key), /symlink/);
  assert.equal(await readFile(target, "utf8"), "untouched");
  assert.equal((await readdir(directory)).length, 1);
});

test("tightens existing store directory permissions when writing", async (t) => {
  const { directory, store } = await fixture(t);
  await mkdir(directory);
  await chmod(directory, 0o755);
  await store.write(ownerKey(record()), record());
  assert.equal((await stat(directory)).mode & 0o777, 0o700);
});

test("concurrent writes leave one complete record and no temporary files", async (t) => {
  const { directory, store } = await fixture(t);
  const key = ownerKey(record());
  const values = Array.from({ length: 20 }, (_, index) =>
    record({ sessionId: `session-${index}` }));
  await Promise.all(values.map((value) => store.write(key, value)));
  const result = await store.read(key);
  assert.ok(values.some((value) => value.sessionId === result.sessionId));
  assert.equal((await readdir(directory)).length, 1);
});
