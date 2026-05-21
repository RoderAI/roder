import { RoderAgent } from "../../typescript/src/index.js";

if (process.env.RODER_SDK_LIVE !== "1") {
  console.log("Set RODER_SDK_LIVE=1, RODER_REMOTE_URL, and RODER_REMOTE_TOKEN to run this example.");
  process.exit(0);
}

const url = process.env.RODER_REMOTE_URL;
const token = process.env.RODER_REMOTE_TOKEN;
if (!url || !token) {
  throw new Error("RODER_REMOTE_URL and RODER_REMOTE_TOKEN are required");
}

const agent = await RoderAgent.create({ remote: { url, token } });
console.log(await agent.listProviders());
await agent.close();
