/** Offline tests for input validation and the shared agent-driving logic
 * against the in-process fake SDK. No network, no real key. */

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import {
  awaitCloudAgent,
  parseCloudAgentInput,
  startCloudAgent,
} from "../src/agents.js";
import { createFakeSdkModule, resetFakeSdk } from "../src/fake.js";
import { redactSecrets } from "../src/protocol.js";

beforeEach(() => {
  resetFakeSdk();
  process.env.CURSOR_API_KEY = "crsr_test_key_for_offline_tests";
});

test("input validation rejects bad shapes before any SDK call", () => {
  assert.throws(
    () => parseCloudAgentInput({}, {}),
    /prompt is required/,
  );
  assert.throws(
    () => parseCloudAgentInput({ prompt: "go" }),
    /repoUrl is required/,
  );
  assert.throws(
    () => parseCloudAgentInput({ prompt: "go", repoUrl: "git@github.com:org/repo.git" }),
    /https repository URL/,
  );
  assert.throws(
    () => parseCloudAgentInput({ prompt: "go", agentId: "agent-local-1" }),
    /bc- prefix/,
  );
  assert.throws(
    () =>
      parseCloudAgentInput({
        prompt: "go",
        agentId: "bc-1",
        repoUrl: "https://github.com/org/repo",
      }),
    /omit them when resuming/,
  );
  assert.throws(
    () =>
      parseCloudAgentInput({
        prompt: "go",
        repoUrl: "https://github.com/org/repo",
        envVars: { SECRET: "x" },
      }),
    /unsupported input key "envVars"/,
  );

  const parsed = parseCloudAgentInput(
    { repoUrl: "https://github.com/org/repo", startingRef: "main", autoCreatePr: true },
    { prompt: "fix the bug", model: "composer-2.5" },
  );
  assert.equal(parsed.prompt, "fix the bug");
  assert.equal(parsed.model, "composer-2.5");
  assert.equal(parsed.autoCreatePr, true);
});

test("create + wait maps the run into a structured outcome with PR urls", async () => {
  const sdk = createFakeSdkModule();
  const started = await startCloudAgent(sdk, {
    prompt: "add structured logging",
    model: "composer-2.5",
    repoUrl: "https://github.com/example-org/example-repo",
    startingRef: "main",
    autoCreatePr: true,
  });
  assert.match(started.agentId, /^bc-fake-/);
  assert.equal(started.resumed, false);

  const progress: string[] = [];
  const outcome = await awaitCloudAgent(started, (status, detail) => {
    progress.push(detail ? `${status}:${detail}` : status);
  });

  assert.equal(outcome.status, "finished");
  assert.equal(outcome.agentId, started.agentId);
  assert.match(outcome.requestId, /^request-/);
  assert.equal(outcome.resultText, "Fake cloud agent completed: add structured logging");
  assert.equal(outcome.model, "composer-2.5");
  assert.deepEqual(outcome.prUrls, ["https://github.com/example-org/example-repo/pull/7"]);
  assert.deepEqual(progress, [
    "CREATING:provisioning VM",
    "RUNNING",
    "TOOL:read_file: completed",
    "FINISHED",
  ]);
  // Unstable tool args from the SDK never reach progress lines.
  assert.ok(!progress.join("|").includes("crsr_fake_args_should_never_leak"));
});

test("resume reattaches by bc- id and reports resumed", async () => {
  const sdk = createFakeSdkModule();
  const first = await startCloudAgent(sdk, {
    prompt: "start work",
    repoUrl: "https://github.com/example-org/example-repo",
    autoCreatePr: false,
  });
  await awaitCloudAgent(first, () => {});

  const second = await startCloudAgent(sdk, {
    prompt: "summarize what you did",
    agentId: first.agentId,
    autoCreatePr: false,
  });
  assert.equal(second.agentId, first.agentId);
  assert.equal(second.resumed, true);
  const outcome = await awaitCloudAgent(second, () => {});
  assert.equal(outcome.resumed, true);
  assert.equal(outcome.status, "finished");

  await assert.rejects(
    startCloudAgent(sdk, {
      prompt: "resume a stranger",
      agentId: "bc-does-not-exist",
      autoCreatePr: false,
    }),
    /unknown agent bc-does-not-exist/,
  );
});

test("missing api key fails closed before the SDK is touched", async () => {
  delete process.env.CURSOR_API_KEY;
  const sdk = createFakeSdkModule();
  await assert.rejects(
    startCloudAgent(sdk, {
      prompt: "go",
      repoUrl: "https://github.com/example-org/example-repo",
      autoCreatePr: false,
    }),
    /CURSOR_API_KEY is not configured/,
  );
});

test("redaction strips key material and bearer tokens", () => {
  process.env.CURSOR_API_KEY = "plain-key-value-123";
  const redacted = redactSecrets(
    "auth failed for crsr_abc123DEF456 with Bearer eyJ.tok.en and raw plain-key-value-123",
  );
  assert.ok(!redacted.includes("crsr_abc123DEF456"));
  assert.ok(!redacted.includes("eyJ.tok.en"));
  assert.ok(!redacted.includes("plain-key-value-123"));
  assert.match(redacted, /crsr_\[REDACTED\]/);
  assert.match(redacted, /Bearer \[REDACTED\]/);
});
