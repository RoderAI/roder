#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { randomBytes, randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
import http2 from "node:http2";

import {
  bestEffortText,
  decodeAgentServerMessage,
  encodeAgentClientMessage,
  encodeCliStreamControlFrames,
  encodeConnectFrame,
  encodeMinimalContextFrame,
  isContextFrameHex,
  isProofText,
  normalizeComposerText,
  proofLine,
  takeConnectFrame
} from "./cursor-agentservice-wire.mjs";

const DEFAULT_ENDPOINT = "https://agentn.global.api5.cursor.sh";
const DEFAULT_PATH = "/agent.v1.AgentService/Run";
const DEFAULT_MODEL = "composer-2.5";
const DEFAULT_PROOF = "CURSOR_COMPOSER_25_API_POC_PROOF";
const DEFAULT_CLIENT_VERSION = "cli-2026.05.24-dda726e";

class PocError extends Error {
  constructor(message, details = {}) {
    super(message);
    this.name = "PocError";
    this.details = details;
  }
}

const proof = process.env.CURSOR_COMPOSER_PROOF || DEFAULT_PROOF;
const expectProof = process.env.CURSOR_EXPECT_PROOF !== "0";
const model = process.env.CURSOR_COMPOSER_MODEL || DEFAULT_MODEL;
const endpoint = process.env.CURSOR_AGENT_SERVICE_URL || DEFAULT_ENDPOINT;
const servicePath = process.env.CURSOR_AGENT_SERVICE_PATH || DEFAULT_PATH;
const prompt = process.argv.slice(2).join(" ").trim() || `Reply with exactly this token and nothing else: ${proof}`;
const timeoutMs = Number(process.env.CURSOR_POC_TIMEOUT_MS || 60000);
const freeformIdleMs = Number(process.env.CURSOR_FREEFORM_IDLE_MS || 2500);

try {
  const access = await resolveAccessToken();
  const requestId = randomUUID();
  const conversationId = randomUUID();
  const output = await runCursorAgentService({
    accessToken: access.token,
    endpoint,
    servicePath,
    prompt,
    model,
    requestId,
    conversationId,
    proof,
    expectProof,
    timeoutMs,
    freeformIdleMs
  });
  const responseText = normalizeComposerText(output.proofText || output.text || bestEffortText(output.strings, proof));
  const found = expectProof
    ? responseText === proof || output.strings.some((value) => isProofText(value, proof))
    : responseText.length > 0;

  console.log(JSON.stringify({
    ok: found,
    transport: "cursor-agentservice-http2-connect-proto",
    authSource: access.source,
    endpoint: `${endpoint}${servicePath}`,
    model,
    ...(expectProof ? { proof } : {}),
    requestId,
    conversationId,
    httpStatus: output.status,
    text: responseText,
    usage: output.usage
  }, null, 2));

  if (!found) {
    throw new PocError("Cursor Composer proof token was not present in the response.", {
      requestId,
      status: output.status
    });
  }
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  const details = error instanceof PocError ? error.details : {};
  console.error(JSON.stringify({ ok: false, error: message, ...redactDetails(details) }, null, 2));
  process.exit(1);
}

async function resolveAccessToken() {
  const directToken = firstEnv("CURSOR_ACCESS_TOKEN", "CURSOR_AUTH_TOKEN");
  if (directToken) return { token: directToken, source: "env-access-token" };

  const apiKey = firstEnv("RODER_CURSOR_API_KEY", "CURSOR_API_KEY");
  if (apiKey) {
    return { token: await exchangeCursorApiKey(apiKey), source: "api-key-exchange" };
  }

  const keychainToken = readCursorKeychainToken();
  if (keychainToken) return { token: keychainToken, source: "macos-keychain" };

  throw new PocError(
    "No Cursor auth found. Set CURSOR_ACCESS_TOKEN, set CURSOR_API_KEY/RODER_CURSOR_API_KEY, or log into Cursor so cursor-access-token exists in macOS Keychain."
  );
}

function firstEnv(...names) {
  for (const name of names) {
    const value = process.env[name]?.trim();
    if (value) return value;
  }
  return "";
}

function readCursorKeychainToken() {
  try {
    return execFileSync(
      "security",
      ["find-generic-password", "-a", "cursor-user", "-s", "cursor-access-token", "-w"],
      { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }
    ).trim();
  } catch {
    return "";
  }
}

async function exchangeCursorApiKey(apiKey) {
  const base = process.env.CURSOR_BACKEND_BASE_URL || "https://api2.cursor.sh";
  const url = `${base.replace(/\/$/, "")}/auth/exchange_user_api_key`;
  const response = await fetch(url, {
    method: "POST",
    headers: {
      authorization: `Bearer ${apiKey}`,
      "content-type": "application/json"
    },
    body: "{}"
  });
  if (!response.ok) {
    throw new PocError(`Cursor API key exchange failed with HTTP ${response.status}`, { status: response.status });
  }
  const payload = await response.json().catch(() => ({}));
  const token = payload.accessToken || payload.access_token || payload.token;
  if (typeof token !== "string" || !token.trim()) {
    throw new PocError("Cursor API key exchange did not return an access token.", { status: response.status });
  }
  return token.trim();
}

async function runCursorAgentService(input) {
  const authority = new URL(input.endpoint);
  const traceId = randomBytes(16).toString("hex");
  const spanId = randomBytes(8).toString("hex");
  const traceparent = `00-${traceId}-${spanId}-01`;
  const runFrame = Buffer.from(encodeConnectFrame(
    encodeAgentClientMessage(input.prompt, input.model, input.conversationId)
  ));

  return new Promise((resolve, reject) => {
    const session = http2.connect(authority.origin);
    let settled = false;
    let headers = {};
    let buffer = new Uint8Array(0);
    let text = "";
    let thinking = "";
    let usage = null;
    let request;
    let timer;
    let idleTimer;
    const strings = [];

    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      clearTimeout(idleTimer);
      try {
        request?.close();
      } catch {}
      session.destroy();
      if (error) reject(error);
      else resolve(value);
    };

    const scheduleFreeformIdleFinish = () => {
      if (input.expectProof || !input.freeformIdleMs || input.freeformIdleMs <= 0) return;
      clearTimeout(idleTimer);
      idleTimer = setTimeout(() => {
        finish(null, {
          status: Number(headers[":status"] || 0),
          proofText: "",
          text,
          usage,
          strings
        });
      }, input.freeformIdleMs);
    };

    timer = setTimeout(() => {
      finish(new PocError("Timed out waiting for Cursor AgentService response.", { timeoutMs: input.timeoutMs }));
    }, input.timeoutMs);

    session.on("error", (error) => finish(error));
    request = session.request(requestHeaders(input, traceparent));

    request.on("response", (responseHeaders) => {
      headers = responseHeaders;
    });
    request.on("data", (chunk) => {
      buffer = concatBytes(buffer, new Uint8Array(chunk));
      for (;;) {
        const next = takeConnectFrame(buffer, headers);
        if (!next) break;
        buffer = next.rest;
        if (next.endStreamError) {
          finish(new PocError(next.endStreamError, { status: headers[":status"] }));
          return;
        }
        if (!next.payload) continue;
        const decoded = decodeAgentServerMessage(next.payload);
        if (process.env.CURSOR_DEBUG_FRAMES === "1") {
          console.error(JSON.stringify({
            frame: decoded.topFields,
            textLen: decoded.text.length,
            thinkingLen: decoded.thinking.length,
            turnEnded: decoded.turnEnded,
            strings: decoded.strings.slice(0, 5).map((value) => value.slice(0, 160))
          }));
        }
        text += decoded.text;
        thinking += decoded.thinking;
        strings.push(...decoded.strings);
        if (decoded.usage) usage = decoded.usage;
        const proofText = input.expectProof
          ? [decoded.text, ...decoded.strings].find((value) => isProofText(value, input.proof))
          : "";
        if (proofText || (!input.expectProof && decoded.turnEnded)) {
          finish(null, {
            status: Number(headers[":status"] || 0),
            proofText: proofText ? proofLine(proofText, input.proof) : "",
            text,
            usage,
            strings
          });
          return;
        }
        if (!input.expectProof && decoded.text) scheduleFreeformIdleFinish();
      }
    });
    request.on("error", (error) => finish(error));
    request.on("end", () => {
      const status = Number(headers[":status"] || 0);
      if (status >= 400) {
        finish(new PocError(`Cursor AgentService returned HTTP ${status}`, { status }));
        return;
      }
      finish(null, { status, text: text || finalTextFromThinking(thinking), usage, strings });
    });

    request.write(runFrame);
    for (const contextFrame of loadContextFrames()) request.write(Buffer.from(contextFrame));
    for (const controlFrame of encodeCliStreamControlFrames()) request.write(Buffer.from(controlFrame));
    if (process.env.CURSOR_HALF_CLOSE === "1") request.end();
  });
}

function requestHeaders(input, traceparent) {
  return {
    ":method": "POST",
    ":path": input.servicePath,
    authorization: `Bearer ${input.accessToken}`,
    "backend-traceparent": traceparent,
    "connect-accept-encoding": "gzip,br",
    "connect-protocol-version": "1",
    "content-type": "application/connect+proto",
    traceparent,
    "user-agent": "connect-es/1.6.1",
    "x-cursor-client-type": process.env.CURSOR_CLIENT_TYPE || "cli",
    "x-cursor-client-version": process.env.CURSOR_CLIENT_VERSION || DEFAULT_CLIENT_VERSION,
    "x-ghost-mode": "true",
    "x-original-request-id": input.requestId,
    "x-request-id": input.requestId
  };
}

function loadContextFrames() {
  const frames = [];
  const explicitHex = process.env.CURSOR_CONTEXT_FRAME_HEX?.trim();
  if (explicitHex) frames.push(hexToFrame(explicitHex));

  const hexFile = process.env.CURSOR_CONTEXT_FRAME_HEX_FILE?.trim();
  if (hexFile) {
    for (const line of readFileSync(hexFile, "utf8").split(/\r?\n/)) {
      const hex = line.trim();
      if (hex) frames.push(hexToFrame(hex));
    }
  }

  const traceFile = process.env.CURSOR_CONTEXT_TRACE_JSONL?.trim();
  if (traceFile) frames.push(...contextFramesFromTrace(traceFile));

  if (frames.length) return frames;
  if (process.env.CURSOR_USE_MINIMAL_CONTEXT === "1") return [encodeMinimalContextFrame()];
  throw new PocError(
    "Cursor AgentService Run requires a serialized RequestContextResult frame. Set CURSOR_CONTEXT_TRACE_JSONL, CURSOR_CONTEXT_FRAME_HEX_FILE, or CURSOR_USE_MINIMAL_CONTEXT=1 for the experimental synthetic context."
  );
}

function contextFramesFromTrace(path) {
  return readFileSync(path, "utf8")
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line) => JSON.parse(line))
    .filter((event) => event.type === "http2.request.write" && event.headers?.[":path"] === DEFAULT_PATH)
    .map((event) => event.chunkHex)
    .filter((hex) => isContextFrameHex(hex))
    .map(hexToFrame);
}

function hexToFrame(hex) {
  return new Uint8Array(Buffer.from(hex, "hex"));
}

function finalTextFromThinking(thinking) {
  const normalized = normalizeComposerText(thinking);
  return normalized.includes("</think>") ? normalized.split("</think>").at(-1).trim() : normalized;
}

function concatBytes(...parts) {
  const total = parts.reduce((sum, part) => sum + part.length, 0);
  const output = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    output.set(part, offset);
    offset += part.length;
  }
  return output;
}

function redactDetails(details) {
  const redacted = {};
  for (const [key, value] of Object.entries(details || {})) {
    redacted[key] = typeof value === "string" ? value.replace(/crsr_[A-Za-z0-9]+/g, "crsr_<redacted>") : value;
  }
  return redacted;
}
