// Hosted multi-tenant service example (roadmap phase 72): connect with a
// bearer credential, read hosted/whoami, start a thread, stream a run, and
// close. Gated behind RODER_HOSTED_SDK_LIVE=1 because it needs a running
// hosted gateway (see docs/roder-hosted-service.md).

import { HostedClient } from "../../typescript/src/index.js";

if (process.env.RODER_HOSTED_SDK_LIVE !== "1") {
  console.log(
    "Set RODER_HOSTED_SDK_LIVE=1, RODER_HOSTED_URL, and RODER_HOSTED_TOKEN to run this example.",
  );
  process.exit(0);
}

const url = process.env.RODER_HOSTED_URL;
const token = process.env.RODER_HOSTED_TOKEN;
if (!url || !token) {
  throw new Error("RODER_HOSTED_URL and RODER_HOSTED_TOKEN are required");
}

const hosted = await HostedClient.connect({ url, token });

const whoami = await hosted.whoami();
console.log(`tenant ${whoami.tenant.tenantId} as ${JSON.stringify(whoami.principal)}`);

// Hosted profiles require a configured runner destination for execution;
// thread creation uses the tenant's own workspace surface.
const workspaces = await hosted.client.call("workspace/list", {});
console.log("workspaces:", workspaces);

const started = (await hosted.client.call("thread/start", {
  workspaceId: process.env.RODER_HOSTED_WORKSPACE_ID,
  model: "mock",
})) as { thread: { id: string } };
const threadId = started.thread.id;
const turn = await hosted.client.call("turn/start", {
  threadId,
  prompt: "Say hello from the hosted SDK example.",
});
console.log("turn:", turn);

// Stream tenant-scoped notifications until the turn completes.
for await (const notification of hosted.notifications()) {
  console.log("notification:", notification.method);
  if (notification.method === "turn/completed" || notification.method === "turn/failed") {
    break;
  }
}

await hosted.close();
