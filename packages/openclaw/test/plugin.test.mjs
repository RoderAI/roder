import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { createRoderPlugin } from '../src/index.mjs';

function fixture({ pluginConfig, stop, register } = {}) {
  let service;
  let backend;
  const hooks = [];
  const instances = [];
  const registry = {
    getAcpRuntimeBackend: () => backend,
    registerAcpRuntimeBackend(value) { backend = value; register?.(value); },
    unregisterAcpRuntimeBackend(id) { assert.equal(id, 'roder'); backend = undefined; },
    tryDispatchAcpReplyHook() {},
  };
  class Runtime {
    constructor(config) { this.config = config; this.stops = 0; instances.push(this); }
    async stop() { this.stops += 1; await stop?.(); }
  }
  const plugin = createRoderPlugin(registry, { Runtime });
  plugin.register({
    pluginConfig, registerService(value) { service = value; },
    on(...args) { hooks.push(args); },
  });
  return { plugin, service, registry, instances, hooks, get backend() { return backend; } };
}

const context = { stateDir: '/tmp/openclaw-test-state' };

test('native ESM entrypoint is declared and host peer version is bounded', async () => {
  const pkg = JSON.parse(await readFile(new URL('../package.json', import.meta.url)));
  assert.equal(pkg.type, 'module');
  assert.deepEqual(pkg.openclaw.extensions, ['./src/plugin.mjs']);
  assert.equal(pkg.exports['.'], './src/plugin.mjs');
  assert.equal(pkg.peerDependencies.openclaw, '2026.10.9');
  assert.equal(pkg.peerDependenciesMeta.openclaw.optional, true);
  assert.equal(pkg.dependencies, undefined);
  const entry = await readFile(new URL('../src/plugin.mjs', import.meta.url), 'utf8');
  assert.match(entry, /from 'openclaw\/plugin-sdk\/acp-runtime-backend'/);
});

test('registers one service and publishes configured backend on start only', async () => {
  const f = fixture({ pluginConfig: { command: '/bin/roder', cwd: '/tmp' } });
  assert.equal(f.plugin.id, 'roder');
  assert.equal(f.service.id, 'roder-acp');
  assert.equal(f.backend, undefined);
  assert.deepEqual(f.hooks, [
    ['reply_dispatch', f.registry.tryDispatchAcpReplyHook, { eligibleDispatchKinds: ['acp'] }],
  ]);
  await f.service.start(context);
  assert.equal(f.backend.id, 'roder');
  assert.equal(f.backend.runtime, f.instances[0]);
  assert.equal(f.instances[0].config.command, '/bin/roder');
  assert.equal(f.instances[0].config.stateDir, join(context.stateDir, 'roder-acp'));
  await f.service.stop(context);
  assert.equal(f.backend, undefined);
  assert.equal(f.instances[0].stops, 1);
});

test('duplicate start and stop are idempotent; service can restart', async () => {
  const f = fixture();
  await Promise.all([f.service.start(context), f.service.start(context)]);
  assert.equal(f.instances.length, 1);
  await Promise.all([f.service.stop(context), f.service.stop(context)]);
  assert.equal(f.instances[0].stops, 1);
  await f.service.start(context);
  assert.equal(f.instances.length, 2);
  await f.service.stop(context);
});

test('stop invalidates a queued startup without publishing a backend', async () => {
  const f = fixture();
  await Promise.all([f.service.start(context), f.service.stop(context)]);
  assert.equal(f.backend, undefined);
  assert.equal(f.instances.length, 0);
  await f.service.start(context);
  assert.equal(f.instances.length, 1);
  await f.service.stop(context);
});

test('stop preserves another plugin generation replacing its registry entry', async () => {
  const f = fixture();
  await f.service.start(context);
  const replacement = { id: 'roder', runtime: {} };
  f.registry.registerAcpRuntimeBackend(replacement);
  await f.service.stop(context);
  assert.equal(f.backend, replacement);
  assert.equal(f.instances[0].stops, 1);
});

test('restart waits for concurrent shutdown to complete', async () => {
  let release;
  const barrier = new Promise((resolve) => { release = resolve; });
  const f = fixture({ stop: () => barrier });
  await f.service.start(context);
  const stopping = f.service.stop(context);
  const starting = f.service.start(context);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(f.backend, undefined);
  assert.equal(f.instances.length, 1);
  release();
  await Promise.all([stopping, starting]);
  assert.equal(f.instances.length, 2);
  assert.equal(f.backend.runtime, f.instances[1]);
  await f.service.stop(context);
});

test('failed registration stops candidate and queue remains usable', async () => {
  let fail = true;
  const f = fixture({ register() { if (fail) throw new Error('registry failed'); } });
  await assert.rejects(f.service.start(context), /registry failed/);
  assert.equal(f.backend, undefined);
  assert.equal(f.instances[0].stops, 1);
  fail = false;
  await f.service.start(context);
  assert.equal(f.instances.length, 2);
  await f.service.stop(context);
});

test('shutdown failure does not leave a stale backend or poisoned lifecycle queue', async () => {
  let fail = true;
  const f = fixture({ stop() { if (fail) throw new Error('shutdown failed'); } });
  await f.service.start(context);
  await assert.rejects(f.service.stop(context), /shutdown failed/);
  assert.equal(f.backend, undefined);
  fail = false;
  await f.service.start(context);
  await f.service.stop(context);
  assert.equal(f.instances.length, 2);
});

test('bad plugin config and missing host state directory fail closed', async () => {
  assert.throws(() => fixture({ pluginConfig: { env: {} } }), /unknown option/);
  const f = fixture();
  for (const stateDir of [undefined, 'relative', '/tmp\0bad']) {
    await assert.rejects(f.service.start({ stateDir }), /absolute host stateDir/);
  }
  assert.equal(f.instances.length, 0);
});
