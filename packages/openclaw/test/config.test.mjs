import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { isAbsolute } from 'node:path';
import test from 'node:test';
import { configSchema, parseConfig } from '../src/config.mjs';

test('safe defaults and fresh mutable containers', () => {
  const config = parseConfig();
  assert.deepEqual(config, {
    command: 'roder', args: ['acp'], cwd: process.cwd(), permissionMode: 'deny-all',
    requestTimeoutMs: 30000, cancelTimeoutMs: 5000, maxMessageBytes: 16 * 1024 * 1024, mcpServers: [],
  });
  assert.ok(isAbsolute(config.cwd));
  config.args.push('bad');
  config.mcpServers.push({});
  assert.deepEqual(parseConfig().args, ['acp']);
  assert.deepEqual(parseConfig().mcpServers, []);
});

test('manifest schema matches runtime schema exactly', async () => {
  const manifest = JSON.parse(await readFile(new URL('../openclaw.plugin.json', import.meta.url)));
  assert.equal(manifest.id, 'roder');
  assert.deepEqual(manifest.configSchema, configSchema);
});

test('valid command override and boundary values', () => {
  const config = parseConfig({
    command: '/opt/bin/roder', args: ['acp', '--literal=$(id)', ''], cwd: '/tmp',
    permissionMode: 'approve-reads', requestTimeoutMs: 100, cancelTimeoutMs: 60000, maxMessageBytes: 1024,
  });
  assert.equal(config.command, '/opt/bin/roder');
  assert.deepEqual(config.args, ['acp', '--literal=$(id)', '']);
  assert.equal(parseConfig({ permissionMode: 'approve-all' }).permissionMode, 'approve-all');
});

test('reject unknown options and malformed values', () => {
  for (const value of [null, [], 'bad', 42]) assert.throws(() => parseConfig(value), TypeError);
  for (const [key, values] of Object.entries({
    command: ['', '  ', 'a\0b', 1, null],
    args: ['acp', [1], ['a\0b'], null],
    cwd: ['relative', '', ' /tmp', '/tmp\0x', null],
    permissionMode: ['auto', true, null],
    requestTimeoutMs: [99, 300001, 1.5, '500', null],
    cancelTimeoutMs: [99, 60001, NaN, null],
    maxMessageBytes: [1023, 67108865, Infinity, null],
    mcpServers: [null, {}],
  })) {
    for (const value of values) assert.throws(() => parseConfig({ [key]: value }), TypeError, `${key}: ${value}`);
  }
  for (const key of ['env', 'shell', 'stateDir', 'requestTimeout']) {
    assert.throws(() => parseConfig({ [key]: true }), /unknown option/);
  }
});

test('normalizes valid MCP stdio and HTTP configs without mutating input', () => {
  const input = {
    mcpServers: [
      { type: 'stdio', name: 'local', command: '/bin/mcp', env: [{ name: 'TOKEN', value: 'secret' }] },
      { type: 'http', name: 'remote', url: 'https://example.test/mcp', headers: [{ name: 'Authorization', value: 'Bearer secret' }] },
    ],
  };
  const config = parseConfig(input);
  assert.deepEqual(config.mcpServers[0], {
    name: 'local', command: '/bin/mcp', args: [], env: [{ name: 'TOKEN', value: 'secret' }],
  });
  assert.deepEqual(config.mcpServers[1], input.mcpServers[1]);
  config.mcpServers[0].env[0].value = 'changed';
  assert.equal(input.mcpServers[0].env[0].value, 'secret');
});

test('reject malformed MCP definitions, duplicate names, unsafe URLs, and SSE', () => {
  const malformed = [
    null, [], {}, { name: '', command: 'mcp' }, { name: 'x', command: ' ' },
    { name: 'x', command: 'mcp', args: null },
    { name: 'x', command: 'mcp', env: { KEY: 'value' } },
    { name: 'x', command: 'mcp', env: [{ name: 'K', value: 'v', unknown: true }] },
    { name: 'x', command: 'mcp', env: [{ name: 'K', value: 'v\0' }] },
    { name: 'x', command: 'mcp', env: [{ name: 'K', value: 'v' }, { name: 'K', value: 'v' }] },
    { type: 'sse', name: 'x', url: 'https://example.test' },
    { type: 'http', name: 'x', url: '/mcp' },
    { type: 'http', name: 'x', url: 'file:///etc/passwd' },
    { type: 'http', name: 'x', url: 'https:example.test' },
    { type: 'http', name: 'x', url: 'https://user:pass@example.test' },
    { type: 'http', name: 'x', url: 'https://example.test/\nfoo' },
    { type: 'http', name: 'x', url: 'https://example.test', headers: null },
    { type: 'http', name: 'x', url: 'https://example.test', command: 'mcp' },
  ];
  for (const value of malformed) assert.throws(() => parseConfig({ mcpServers: [value] }), TypeError);
  assert.throws(() => parseConfig({ mcpServers: [{ name: 'x', command: 'mcp' }, { name: 'x', command: 'other' }] }), /duplicate name/);
});
