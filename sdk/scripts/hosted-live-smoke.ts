#!/usr/bin/env node
// Hosted SDK live smoke (phase 72, Task 6). Gated: requires a running
// hosted gateway and a real credential.
//
//   RODER_HOSTED_SDK_LIVE=1 RODER_HOSTED_URL=ws://... RODER_HOSTED_TOKEN=rk_test_... \
//     node --experimental-strip-types sdk/scripts/hosted-live-smoke.ts

import { HostedClient } from "../typescript/src/index.js";

if (process.env.RODER_HOSTED_SDK_LIVE !== "1") {
  console.log("skipped: set RODER_HOSTED_SDK_LIVE=1 to run the hosted live smoke");
  process.exit(0);
}

const url = process.env.RODER_HOSTED_URL;
const token = process.env.RODER_HOSTED_TOKEN;
if (!url || !token) {
  throw new Error("RODER_HOSTED_URL and RODER_HOSTED_TOKEN are required");
}

const hosted = await HostedClient.connect({ url, token });
const whoami = await hosted.whoami();
console.log("hosted whoami ok:", whoami.tenant.tenantId, whoami.role);
await hosted.client.call("initialize", {});
console.log("initialize ok");
await hosted.close();
console.log("hosted live smoke passed");
