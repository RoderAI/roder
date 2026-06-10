/**
 * Live integration smoke against a real Anthropic model. Env-gated: requires
 * ANTHROPIC_API_KEY and RODER_BIN (path to a roder binary, e.g.
 * target/release/roder). Run with `node live-smoke.ts` (Node >= 22 strips
 * types natively) after building the SDK (`pnpm build` in sdk/typescript).
 *
 * Scenarios:
 *   a. toolAllowlist shrinks the advertised toolset (prompt tokens drop).
 *   b. developerInstructions reach the model.
 *   c. external tool round-trip via thread/toolExecutionRequested + tools/resolve.
 *   d. turn/completed carries finishReason and cache-write usage.
 *   e. an unresolved external tool times out instead of hanging the turn.
 */
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  RoderAgent,
  type RoderAgentOptions,
  type RoderSdkEvent,
  type TurnCompletedEvent,
} from "../../typescript/dist/src/index.js";

const RODER_BIN = process.env.RODER_BIN;
const API_KEY = process.env.ANTHROPIC_API_KEY;
if (!RODER_BIN || !API_KEY) {
  console.error("live-smoke requires RODER_BIN and ANTHROPIC_API_KEY");
  process.exit(2);
}
const MODEL = process.env.RODER_SMOKE_MODEL ?? "claude-haiku-4-5-20251001";

interface TurnOutcome {
  completed: TurnCompletedEvent | undefined;
  agentText: string;
  events: RoderSdkEvent[];
  threadId: string;
}

function makeAgent(
  extra: Partial<RoderAgentOptions> = {},
  env: Record<string, string> = {},
): Promise<RoderAgent> {
  const workspace = mkdtempSync(join(tmpdir(), "roder-smoke-ws-"));
  const state = mkdtempSync(join(tmpdir(), "roder-smoke-state-"));
  return RoderAgent.create({
    local: {
      command: RODER_BIN,
      args: ["app-server", "--listen", "stdio://", "--yolo"],
      cwd: workspace,
      env: {
        ANTHROPIC_API_KEY: API_KEY,
        RODER_CONFIG_DIR: state,
        RODER_DATA_DIR: state,
        ...env,
      },
    },
    cwd: workspace,
    model: { provider: "anthropic", id: MODEL, reasoning: "none" },
    ...extra,
  });
}

/**
 * Hard deadline per turn: the regressions this smoke exists to catch (e.g. the
 * external-tool timeout path breaking) manifest as a hang, which would
 * otherwise block forever with an exit code that is neither pass nor fail.
 */
const TURN_DEADLINE_MS = 120_000;

async function runTurn(agent: RoderAgent, prompt: string): Promise<TurnOutcome> {
  const run = await agent.send(prompt);
  const events: RoderSdkEvent[] = [];
  let completed: TurnCompletedEvent | undefined;
  let agentText = "";
  const consume = (async () => {
    for await (const event of run.stream()) {
      events.push(event);
      if (event.type === "item.completed" && event.item.type === "agentMessage") {
        agentText += event.item.text;
      }
      if (event.type === "turn.completed") {
        completed = event;
      }
    }
  })();
  let timer: NodeJS.Timeout | undefined;
  try {
    await Promise.race([
      consume,
      new Promise<never>((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`turn did not complete within ${TURN_DEADLINE_MS}ms`)),
          TURN_DEADLINE_MS,
        );
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
  return { completed, agentText, events, threadId: run.threadId };
}

const failures: string[] = [];
function check(scenario: string, condition: boolean, detail: string): void {
  const status = condition ? "PASS" : "FAIL";
  console.log(`[${status}] ${scenario}: ${detail}`);
  if (!condition) {
    failures.push(`${scenario}: ${detail}`);
  }
}

// --- a. allowlist: unrestricted baseline vs toolAllowlist ["read_file"] ---
{
  const baselineAgent = await makeAgent();
  const baseline = await runTurn(baselineAgent, "Reply with exactly the word OK.");
  await baselineAgent.close();
  const baselineTokens = baseline.completed?.turn.usage?.prompt_tokens ?? -1;

  const restrictedAgent = await makeAgent({ toolAllowlist: ["read_file"] });
  const restricted = await runTurn(restrictedAgent, "Reply with exactly the word OK.");
  const thread = (await restrictedAgent.readThread()) as {
    thread?: { toolAllowlist?: string[] };
  };
  await restrictedAgent.close();
  const restrictedTokens = restricted.completed?.turn.usage?.prompt_tokens ?? -1;

  check(
    "a.allowlist",
    baselineTokens > 0 && restrictedTokens > 0 && restrictedTokens < baselineTokens / 2,
    `baseline prompt_tokens=${baselineTokens}, allowlisted prompt_tokens=${restrictedTokens}`,
  );
  check(
    "a.allowlist.persisted",
    JSON.stringify(thread.thread?.toolAllowlist) === JSON.stringify(["read_file"]),
    `thread/read toolAllowlist=${JSON.stringify(thread.thread?.toolAllowlist)}`,
  );
}

// --- b. developerInstructions ---
{
  const agent = await makeAgent({
    instructions: "You must always begin replies with the word KUMQUAT.",
  });
  const outcome = await runTurn(agent, "Greet me in one short sentence.");
  await agent.close();
  check(
    "b.instructions",
    outcome.agentText.trimStart().startsWith("KUMQUAT"),
    `reply=${JSON.stringify(outcome.agentText.slice(0, 120))}`,
  );
}

/**
 * Inert filler pushing the cacheable prefix past the model's minimum cacheable
 * size (4096 tokens for Haiku-class models; below it cache writes are silently
 * 0 and the scenario-d assertion would be vacuous). ~30k chars ≈ 7.5k tokens.
 */
const CACHE_PRIMER = `Background reference material (ignore unless asked): ${"the acme workspace indexes threads, files, schedules, and artifacts for retrieval. ".repeat(350)}`;

// --- c. external tool round-trip ---
{
  const calls: Array<{ name: string; arguments: unknown }> = [];
  const agent = await makeAgent({
    instructions: CACHE_PRIMER,
    externalTools: [
      {
        name: "get_weather",
        description: "Get the current weather for a city",
        parameters: {
          type: "object",
          properties: { city: { type: "string" } },
          required: ["city"],
        },
      },
    ],
    onToolExecute(call) {
      calls.push({ name: call.name, arguments: call.arguments });
      return { output: "14C, raining" };
    },
  });
  const outcome = await runTurn(
    agent,
    "What is the weather in Oslo right now? Use the get_weather tool.",
  );
  const transcript = (await agent.client.call("thread/read", {
    threadId: outcome.threadId,
    includeTurns: true,
  })) as { thread?: { turns?: Array<{ items?: Array<Record<string, unknown>> }> } };
  await agent.close();

  const city =
    calls[0] && typeof calls[0].arguments === "object" && calls[0].arguments !== null
      ? String((calls[0].arguments as Record<string, unknown>).city ?? "")
      : "";
  check(
    "c.external_tool.callback",
    calls.length === 1 && calls[0]?.name === "get_weather" && /oslo/i.test(city),
    `calls=${JSON.stringify(calls)}`,
  );
  check(
    "c.external_tool.reply",
    /14\s*°?C/i.test(outcome.agentText) && /rain/i.test(outcome.agentText),
    `reply=${JSON.stringify(outcome.agentText.slice(0, 160))}`,
  );
  /**
   * The transcript projects ToolCall and ToolResult as separate toolExecution
   * items sharing one id (inProgress, then completed with the output); the
   * ToolResult is the completed one.
   */
  const toolResults = (transcript.thread?.turns ?? [])
    .flatMap((turn) => turn.items ?? [])
    .filter(
      (item) =>
        item.type === "toolExecution" &&
        item.toolName === "get_weather" &&
        item.status === "completed",
    );
  check(
    "c.external_tool.transcript",
    toolResults.length === 1 && String(toolResults[0]?.output ?? "").includes("14C, raining"),
    `completed toolExecution items=${JSON.stringify(toolResults)}`,
  );
  const sawRequested = outcome.events.some((event) => event.type === "tool_execution.requested");
  const sawResolved = outcome.events.some(
    (event) => event.type === "tool_execution.resolved" && event.outcome === "resolved",
  );
  check(
    "c.external_tool.events",
    sawRequested && sawResolved,
    `tool_execution.requested=${sawRequested}, tool_execution.resolved=${sawResolved}`,
  );

  // --- d. finishReason + cache-write usage on the same turn surface ---
  const usage = outcome.completed?.turn.usage;
  check(
    "d.finish_reason",
    outcome.completed?.turn.finishReason === "stop",
    `finishReason=${JSON.stringify(outcome.completed?.turn.finishReason)}`,
  );
  /**
   * Key presence alone is vacuous (the field serializes unconditionally,
   * defaulting to 0). The CACHE_PRIMER instructions exceed the model's
   * minimum cacheable prefix, so this multi-step turn must record an actual
   * cache write.
   */
  check(
    "d.cache_write_usage",
    (usage?.cache_creation_prompt_tokens ?? 0) > 0,
    `usage=${JSON.stringify(usage)}`,
  );
}

// --- e. unresolved external tool times out instead of hanging ---
{
  const agent = await makeAgent(
    {
      externalTools: [
        {
          name: "slow_lookup",
          description: "Look up a record. Takes a while.",
          parameters: {
            type: "object",
            properties: { key: { type: "string" } },
            required: ["key"],
          },
        },
      ],
      onToolExecute() {
        return new Promise(() => {});
      },
    },
    { RODER_EXTERNAL_TOOL_TIMEOUT_SECONDS: "5" },
  );
  const startedAt = Date.now();
  const outcome = await runTurn(
    agent,
    "Call the slow_lookup tool with key 'demo' and tell me what it returned.",
  );
  await agent.close();
  const elapsedMs = Date.now() - startedAt;
  const timedOut = outcome.events.some(
    (event) => event.type === "tool_execution.resolved" && event.outcome === "timedOut" && event.isError,
  );
  check(
    "e.timeout",
    timedOut && outcome.completed !== undefined && elapsedMs < 120_000,
    `timedOut=${timedOut}, turnStatus=${outcome.completed?.turn.status}, elapsedMs=${elapsedMs}`,
  );
}

if (failures.length > 0) {
  console.error(`\n${failures.length} scenario(s) failed`);
  process.exit(1);
}
console.log("\nall live smoke scenarios passed");
