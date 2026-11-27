import { isAbsolute, resolve } from 'node:path';

const MiB = 1024 * 1024;
const withDefault = (value, fallback) => value === undefined ? fallback : value;
const stringSchema = { type: 'string', minLength: 1, pattern: '^(?!.*\\u0000)[\\s\\S]*\\S[\\s\\S]*$' };
const argumentSchema = { type: 'string', pattern: '^[^\\u0000]*$' };
const pairsSchema = {
  type: 'array',
  items: {
    type: 'object', additionalProperties: false, required: ['name', 'value'],
    properties: { name: stringSchema, value: argumentSchema },
  },
};

export const configSchema = {
  type: 'object',
  additionalProperties: false,
  properties: {
    command: { ...stringSchema, default: 'roder' },
    args: { type: 'array', items: argumentSchema, default: ['acp'] },
    cwd: { ...stringSchema, description: 'Absolute working directory; defaults to the gateway working directory.' },
    permissionMode: { type: 'string', enum: ['deny-all', 'approve-reads', 'approve-all'], default: 'deny-all' },
    requestTimeoutMs: { type: 'integer', minimum: 100, maximum: 300000, default: 30000 },
    cancelTimeoutMs: { type: 'integer', minimum: 100, maximum: 60000, default: 5000 },
    maxMessageBytes: { type: 'integer', minimum: 1024, maximum: 64 * MiB, default: 16 * MiB },
    mcpServers: {
      type: 'array', default: [],
      items: {
        oneOf: [
          {
            type: 'object', additionalProperties: false, required: ['name', 'command'],
            properties: {
              type: { const: 'stdio' }, name: stringSchema, command: stringSchema,
              args: { type: 'array', items: argumentSchema }, env: pairsSchema,
            },
          },
          {
            type: 'object', additionalProperties: false, required: ['type', 'name', 'url'],
            properties: {
              type: { const: 'http' }, name: stringSchema,
              url: { ...stringSchema, pattern: '^https?://[^\\s\\u0000]+$' }, headers: pairsSchema,
            },
          },
        ],
      },
    },
  },
};

function object(value, label, keys) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new TypeError(`${label} must be an object`);
  }
  for (const key of Object.keys(value)) {
    if (!keys.includes(key)) throw new TypeError(`${label}: unknown option ${key}`);
  }
}

function string(value, label, allowEmpty = false) {
  if (typeof value !== 'string' || value.includes('\0') || (!allowEmpty && !value.trim())) {
    throw new TypeError(`${label} must be ${allowEmpty ? 'a' : 'a nonblank'} string without NUL bytes`);
  }
  return value;
}

function argumentsArray(value, label) {
  if (!Array.isArray(value)) throw new TypeError(`${label} must be an array`);
  return value.map((item, index) => string(item, `${label}[${index}]`, true));
}

function pairs(value, label) {
  if (!Array.isArray(value)) throw new TypeError(`${label} must be an array`);
  const seen = new Set();
  return value.map((item, index) => {
    const path = `${label}[${index}]`;
    object(item, path, ['name', 'value']);
    const name = string(item.name, `${path}.name`);
    if (seen.has(name)) throw new TypeError(`${label}: duplicate name ${name}`);
    seen.add(name);
    return { name, value: string(item.value, `${path}.value`, true) };
  });
}

function mcpServers(value) {
  if (!Array.isArray(value)) throw new TypeError('mcpServers must be an array');
  const seen = new Set();
  return value.map((server, index) => {
    const label = `mcpServers[${index}]`;
    if (!server || typeof server !== 'object') throw new TypeError(`${label} must be an object`);
    const type = withDefault(server.type, 'stdio');
    object(server, label, type === 'stdio'
      ? ['type', 'name', 'command', 'args', 'env']
      : ['type', 'name', 'url', 'headers']);
    const name = string(server.name, `${label}.name`);
    if (seen.has(name)) throw new TypeError(`mcpServers: duplicate name ${name}`);
    seen.add(name);
    if (type === 'stdio') {
      return {
        name, command: string(server.command, `${label}.command`),
        args: argumentsArray(withDefault(server.args, []), `${label}.args`),
        env: pairs(withDefault(server.env, []), `${label}.env`),
      };
    }
    if (type !== 'http') throw new TypeError(`${label}.type must be stdio or http; SSE is not supported`);
    const url = string(server.url, `${label}.url`);
    let parsed;
    try { parsed = new URL(url); } catch { throw new TypeError(`${label}.url must be an absolute HTTP(S) URL`); }
    if (!/^https?:\/\//i.test(url) || !['http:', 'https:'].includes(parsed.protocol) || !parsed.hostname ||
        parsed.username || parsed.password || /[\s\x00-\x1f]/.test(url)) {
      throw new TypeError(`${label}.url must be an absolute HTTP(S) URL without credentials or whitespace`);
    }
    return { type: 'http', name, url, headers: pairs(withDefault(server.headers, []), `${label}.headers`) };
  });
}

export function parseConfig(raw = {}) {
  object(raw, 'Roder config', Object.keys(configSchema.properties));
  const config = {
    command: string(withDefault(raw.command, 'roder'), 'command'),
    args: argumentsArray(withDefault(raw.args, ['acp']), 'args'),
    cwd: string(withDefault(raw.cwd, resolve(process.cwd())), 'cwd'),
    permissionMode: withDefault(raw.permissionMode, 'deny-all'),
    requestTimeoutMs: withDefault(raw.requestTimeoutMs, 30000),
    cancelTimeoutMs: withDefault(raw.cancelTimeoutMs, 5000),
    maxMessageBytes: withDefault(raw.maxMessageBytes, 16 * MiB),
    mcpServers: mcpServers(withDefault(raw.mcpServers, [])),
  };
  if (!isAbsolute(config.cwd)) throw new TypeError('cwd must be an absolute path');
  if (!configSchema.properties.permissionMode.enum.includes(config.permissionMode)) {
    throw new TypeError('permissionMode must be deny-all, approve-reads, or approve-all');
  }
  for (const key of ['requestTimeoutMs', 'cancelTimeoutMs', 'maxMessageBytes']) {
    const { minimum, maximum } = configSchema.properties[key];
    if (!Number.isInteger(config[key]) || config[key] < minimum || config[key] > maximum) {
      throw new TypeError(`${key} must be an integer between ${minimum} and ${maximum}`);
    }
  }
  return config;
}
