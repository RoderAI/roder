import { brotliDecompressSync, gunzipSync } from "node:zlib";

const CONNECT_COMPRESSED_FLAG = 1;
const CONNECT_END_STREAM_FLAG = 2;
const encoder = new TextEncoder();
const decoder = new TextDecoder();

export function encodeAgentClientMessage(prompt, modelId, conversationId) {
  return protoMessage([protoField(1, 2, encodeAgentRunRequest(prompt, modelId, conversationId))]);
}

function encodeAgentRunRequest(prompt, modelId, conversationId) {
  return protoMessage([
    protoField(1, 2, new Uint8Array(0)),
    protoField(2, 2, encodeConversationAction(prompt)),
    protoField(4, 2, new Uint8Array(0)),
    protoField(5, 2, conversationId),
    protoField(9, 2, encodeRequestedModel(modelId)),
    protoField(12, 0, 0),
    protoField(16, 2, conversationId)
  ]);
}

function encodeConversationAction(prompt) {
  return protoMessage([protoField(1, 2, encodeUserMessageAction(prompt))]);
}

function encodeUserMessageAction(prompt) {
  return protoMessage([protoField(1, 2, encodeUserMessage(prompt, crypto.randomUUID()))]);
}

function encodeUserMessage(prompt, messageId) {
  return protoMessage([
    protoField(1, 2, prompt),
    protoField(2, 2, messageId),
    protoField(3, 2, new Uint8Array(0)),
    protoField(4, 0, 2)
  ]);
}

function encodeRequestedModel(modelId) {
  return protoMessage([
    protoField(1, 2, modelId),
    protoField(3, 2, protoMessage([protoField(1, 2, "fast"), protoField(2, 2, "false")]))
  ]);
}

export function encodeConnectFrame(payload) {
  const frame = new Uint8Array(5 + payload.length);
  frame[0] = 0;
  new DataView(frame.buffer).setUint32(1, payload.length, false);
  frame.set(payload, 5);
  return frame;
}

export function encodeCliStreamControlFrames() {
  return [
    "2a020a00",
    "1a021a00",
    "1a0408011a00",
    "1a0408021a00",
    "1a0408031a00",
    "1a0408051a00",
    "1a0408041a00",
    "1a0408061a00",
    "1a0408071a00"
  ].map((hex) => encodeConnectFrame(new Uint8Array(Buffer.from(hex, "hex"))));
}

export function encodeMinimalContextFrame() {
  const contextText = "Minimal direct AgentService inference proof context.";
  const contextItem = protoMessage([
    protoField(1, 2, "cursor-api-poc-context.txt"),
    protoField(2, 2, contextText),
    protoField(3, 2, protoMessage([
      protoField(3, 2, protoMessage([protoField(1, 2, contextText)]))
    ]))
  ]);
  const requestContext = protoMessage([
    protoField(2, 2, contextItem),
    protoField(4, 2, encodeRequestContextEnv()),
    protoField(17, 0, 0),
    protoField(24, 0, 0),
    protoField(32, 0, 1),
    protoField(33, 0, 1),
    protoField(35, 0, 0),
    protoField(36, 0, 1),
    protoField(39, 0, 1),
    protoField(40, 0, 1),
    protoField(41, 0, 1),
    protoField(42, 0, 1),
    protoField(43, 0, 1),
    protoField(44, 0, 1),
    protoField(45, 0, 1)
  ]);
  return encodeConnectFrame(protoMessage([
    protoField(2, 2, protoMessage([
      protoField(10, 2, protoMessage([
        protoField(1, 2, protoMessage([protoField(1, 2, requestContext)]))
      ]))
    ]))
  ]));
}

function encodeRequestContextEnv() {
  const cwd = process.cwd();
  return protoMessage([
    protoField(1, 2, `${process.platform} ${process.arch}`),
    protoField(2, 2, cwd),
    protoField(3, 2, process.env.SHELL || ""),
    protoField(5, 0, 0),
    protoField(10, 2, process.env.TZ || "UTC"),
    protoField(11, 2, cwd),
    protoField(18, 0, 1),
    protoField(20, 0, cwd === process.env.HOME ? 1 : 0),
    protoField(21, 2, cwd),
    protoField(22, 0, 0)
  ]);
}

export function takeConnectFrame(buffer, headers) {
  if (buffer.length < 5) return null;
  const flags = buffer[0];
  const length = new DataView(buffer.buffer, buffer.byteOffset + 1, 4).getUint32(0, false);
  if (buffer.length < 5 + length) return null;
  let payload = buffer.slice(5, 5 + length);
  const rest = buffer.slice(5 + length);

  if ((flags & CONNECT_COMPRESSED_FLAG) === CONNECT_COMPRESSED_FLAG) {
    payload = decompressConnectPayload(payload, headers);
  }
  if ((flags & CONNECT_END_STREAM_FLAG) === CONNECT_END_STREAM_FLAG) {
    return { rest, endStreamError: parseEndStreamError(payload) };
  }
  return { rest, payload };
}

function decompressConnectPayload(payload, headers) {
  const encoding = String(headers["connect-content-encoding"] || headers["grpc-encoding"] || "").toLowerCase();
  if (encoding === "gzip") return new Uint8Array(gunzipSync(payload));
  if (encoding === "br") return new Uint8Array(brotliDecompressSync(payload));
  throw new Error(`Cursor returned compressed Connect payload with unsupported encoding '${encoding || "unknown"}'.`);
}

function parseEndStreamError(payload) {
  if (!payload.length) return "";
  const text = decoder.decode(payload).trim();
  if (!text || text === "{}") return "";
  try {
    const parsed = JSON.parse(text);
    const message = parsed?.error?.message || parsed?.message;
    return typeof message === "string" ? message : text;
  } catch {
    return text;
  }
}

export function decodeAgentServerMessage(payload) {
  const result = {
    text: "",
    thinking: "",
    usage: null,
    turnEnded: false,
    topFields: describeFields(payload),
    strings: collectUtf8Strings(payload)
  };
  for (const field of decodeProtobufFields(payload)) {
    if (field.wt !== 2 || !(field.value instanceof Uint8Array)) continue;
    if (field.no === 1) mergeInteractionUpdate(result, field.value);
    if (field.no === 2) mergeLegacyComposerUpdate(result, field.value);
  }
  return result;
}

function describeFields(bytes) {
  return decodeProtobufFieldsSafe(bytes).map((field) => ({
    no: field.no,
    wt: field.wt,
    len: field.value instanceof Uint8Array ? field.value.length : undefined,
    value: typeof field.value === "number" ? field.value : undefined
  }));
}

function mergeInteractionUpdate(result, bytes) {
  for (const field of decodeProtobufFields(bytes)) {
    if (field.wt !== 2 || !(field.value instanceof Uint8Array)) continue;
    if (field.no === 1) mergeTextField(result, field.value, "text");
    if (field.no === 4) mergeTextField(result, field.value, "thinking");
    if (field.no === 14) {
      result.usage = decodeUsage(field.value);
      result.turnEnded = true;
    }
  }
}

function mergeLegacyComposerUpdate(result, bytes) {
  for (const field of decodeProtobufFields(bytes)) {
    if (field.no === 1 && field.wt === 2 && field.value instanceof Uint8Array) {
      result.text += decoder.decode(field.value);
    }
    if (field.no === 25 && field.wt === 2 && field.value instanceof Uint8Array) {
      mergeTextField(result, field.value, "thinking");
    }
  }
}

function mergeTextField(result, bytes, key) {
  for (const field of decodeProtobufFields(bytes)) {
    if (field.no === 1 && field.wt === 2 && field.value instanceof Uint8Array) {
      result[key] += decoder.decode(field.value);
    }
  }
}

function decodeUsage(bytes) {
  const usage = {};
  for (const field of decodeProtobufFields(bytes)) {
    if (field.wt === 0 && typeof field.value === "number") usage[`field_${field.no}`] = field.value;
  }
  return Object.keys(usage).length ? usage : null;
}

function collectUtf8Strings(bytes, depth = 0) {
  if (depth > 5) return [];
  const values = [];
  for (const field of decodeProtobufFieldsSafe(bytes)) {
    if (field.wt !== 2 || !(field.value instanceof Uint8Array) || !field.value.length) continue;
    const text = decoder.decode(field.value);
    if (looksLikeText(text)) values.push(text);
    values.push(...collectUtf8Strings(field.value, depth + 1));
  }
  return values;
}

export function bestEffortText(strings, requiredToken) {
  const exact = strings.find((value) => isProofText(value, requiredToken));
  if (exact) return exact;
  return strings.filter((value) => /[A-Za-z]{3}/.test(value)).at(-1) || "";
}

export function isProofText(value, proof) {
  return proofLine(value, proof) === proof;
}

export function proofLine(value, proof) {
  return normalizeComposerText(value)
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => line === proof) || "";
}

export function normalizeComposerText(value) {
  return value
    .replace(/<\/think>/g, "")
    .replace(/<\s*[|]\s*final\s*[|]\s*>/g, "")
    .trim();
}

function looksLikeText(value) {
  const trimmed = value.trim();
  if (!trimmed) return false;
  const printable = [...trimmed].filter((char) => {
    const code = char.codePointAt(0);
    return code === 9 || code === 10 || code === 13 || (code >= 32 && code !== 127);
  }).length;
  return printable / trimmed.length > 0.9;
}

export function isContextFrameHex(hex) {
  return decodeProtobufFieldsSafe(connectPayloadFromHex(hex))
    .some((field) => field.no === 2 && field.wt === 2);
}

function connectPayloadFromHex(hex) {
  const frame = Buffer.from(hex, "hex");
  if (frame.length < 5) return new Uint8Array(0);
  return new Uint8Array(frame.subarray(5, 5 + frame.readUInt32BE(1)));
}

function protoMessage(parts) {
  return concatBytes(...parts);
}

function protoField(fieldNumber, wireType, value) {
  const tag = encodeVarint((fieldNumber << 3) | wireType);
  if (wireType === 0) return concatBytes(tag, encodeVarint(value));
  const bytes = typeof value === "string" ? encoder.encode(value) : value instanceof Uint8Array ? value : encodeVarint(value);
  return concatBytes(tag, encodeVarint(bytes.length), bytes);
}

function encodeVarint(value) {
  const bytes = [];
  let current = value >>> 0;
  while (current >= 0x80) {
    bytes.push((current & 0x7f) | 0x80);
    current >>>= 7;
  }
  bytes.push(current);
  return new Uint8Array(bytes);
}

function decodeProtobufFieldsSafe(bytes) {
  try {
    return decodeProtobufFields(bytes);
  } catch {
    return [];
  }
}

function decodeProtobufFields(bytes) {
  const fields = [];
  let offset = 0;
  while (offset < bytes.length) {
    const tag = readVarint(bytes, offset);
    offset = tag.offset;
    const no = tag.value >> 3;
    const wt = tag.value & 7;
    if (wt === 0) {
      const value = readVarint(bytes, offset);
      offset = value.offset;
      fields.push({ no, wt, value: value.value });
    } else if (wt === 2) {
      const length = readVarint(bytes, offset);
      offset = length.offset;
      if (offset + length.value > bytes.length) throw new Error("Length-delimited field exceeds payload size");
      fields.push({ no, wt, value: bytes.slice(offset, offset + length.value) });
      offset += length.value;
    } else if (wt === 1) {
      offset += 8;
    } else if (wt === 5) {
      offset += 4;
    } else {
      throw new Error(`Unsupported protobuf wire type ${wt}`);
    }
  }
  return fields;
}

function readVarint(bytes, offset) {
  let value = 0;
  let shift = 0;
  while (offset < bytes.length) {
    const byte = bytes[offset++];
    value += (byte & 0x7f) * 2 ** shift;
    if ((byte & 0x80) === 0) return { value, offset };
    shift += 7;
  }
  throw new Error("Unexpected end of protobuf varint");
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
