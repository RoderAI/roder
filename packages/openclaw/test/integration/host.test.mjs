import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import test from 'node:test';

const hostPackage = process.env.OPENCLAW_TEST_PACKAGE;
const binary = process.env.RODER_TEST_BINARY;
const packageRoot = fileURLToPath(new URL('../..', import.meta.url));

// Explicit opt-in: this installs only the local tarball, uses the real host SDK,
// and gives the CLI an entirely temporary HOME/state/config. No gateway starts.
test('packed plugin with real OpenClaw SDK and isolated CLI discovery', {
  skip: !hostPackage && 'set OPENCLAW_TEST_PACKAGE to an installed openclaw package directory',
  timeout: 120_000,
}, async (t) => {
  assert.ok(path.isAbsolute(hostPackage), 'OPENCLAW_TEST_PACKAGE must be absolute');
  const hostManifest = JSON.parse(await readFile(path.join(hostPackage, 'package.json'), 'utf8'));
  assert.equal(hostManifest.name, 'openclaw');
  assert.equal(hostManifest.version, '2026.10.9', 'only the pinned host version is qualified');
  const root = await mkdtemp(path.join(os.tmpdir(), 'roder-openclaw-host-test-'));
  let service;
  t.after(async () => {
    try { await service?.stop(); }
    finally { await rm(root, { recursive: true, force: true }); }
  });
  const home = path.join(root, 'home');
  const stage = path.join(root, 'consumer');
  const workspace = path.join(root, 'workspace');
  await Promise.all([mkdir(home), mkdir(stage), mkdir(workspace)]);
  const env = {
    ...process.env,
    PATH: `${path.dirname(process.execPath)}${path.delimiter}${process.env.PATH ?? ''}`,
    HOME: home,
    USERPROFILE: home,
    XDG_CONFIG_HOME: path.join(home, 'config'),
    XDG_CACHE_HOME: path.join(home, 'cache'),
    XDG_DATA_HOME: path.join(home, 'data'),
    OPENCLAW_HOME: home,
    OPENCLAW_STATE_DIR: path.join(home, 'state'),
    OPENCLAW_CONFIG_PATH: path.join(home, 'state', 'openclaw.json'),
    OPENCLAW_COMPILE_CACHE_DIR: path.join(home, 'compile-cache'),
    OPENCLAW_LOG_DIR: path.join(home, 'logs'),
  };
  const npm = (args, cwd) => execFileSync('npm', args, {
    cwd, env, encoding: 'utf8', timeout: 45_000, stdio: ['ignore', 'pipe', 'pipe'],
  });
  const packed = JSON.parse(npm([
    'pack', '--ignore-scripts', '--json', '--pack-destination', root,
  ], packageRoot));
  assert.equal(packed.length, 1);
  // Omit the optional peer: its real, separately installed instance is linked below.
  npm(['install', '--offline', '--ignore-scripts', '--omit=optional', '--no-audit',
    '--no-fund', path.join(root, packed[0].filename)], stage);
  await symlink(hostPackage, path.join(stage, 'node_modules', 'openclaw'), 'dir');
  const requireFromConsumer = createRequire(path.join(stage, 'consumer.cjs'));
  const installedRoot = path.dirname(path.dirname(requireFromConsumer.resolve('@roderai/openclaw')));
  const installedManifest = JSON.parse(await readFile(path.join(installedRoot, 'package.json'), 'utf8'));
  assert.deepEqual(installedManifest.openclaw.extensions, ['./src/plugin.mjs']);
  const sdk = await import(pathToFileURL(requireFromConsumer.resolve('openclaw/plugin-sdk/acp-runtime-backend')));
  const { default: plugin } = await import(pathToFileURL(requireFromConsumer.resolve('@roderai/openclaw')));
  const { RoderRuntime } = await import(pathToFileURL(requireFromConsumer.resolve('@roderai/openclaw/runtime')));
  await t.test('native packed entry registers an actual runtime in the real SDK registry', async () => {
    assert.equal(sdk.getAcpRuntimeBackend('roder'), null);
    plugin.register({
      pluginConfig: { command: binary ? path.resolve(binary) : 'roder', cwd: workspace },
      on(event, handler, options) {
        assert.equal(event, 'reply_dispatch');
        assert.equal(handler, sdk.tryDispatchAcpReplyHook);
        assert.deepEqual(options, { eligibleDispatchKinds: ['acp'] });
      },
      registerService(value) {
        assert.equal(service, undefined);
        service = value;
      },
    });
    assert.equal(service.id, 'roder-acp');
    await service.start({ stateDir: path.join(root, 'service-state') });
    const registered = sdk.getAcpRuntimeBackend('roder');
    assert.ok(registered.runtime instanceof RoderRuntime);
    assert.equal(registered.runtime.config.stateDir, path.join(root, 'service-state', 'roder-acp'));
    assert.equal(sdk.requireAcpRuntimeBackend('roder').runtime, registered.runtime);
  });

  await t.test('optional real binary doctor uses isolated offline mock configuration', {
    skip: !binary && 'set RODER_TEST_BINARY to exercise the real ACP binary',
  }, async () => {
    const configDir = path.join(root, 'roder-config');
    await mkdir(configDir);
    await writeFile(path.join(configDir, 'config.toml'), 'provider = "mock"\nmodel = "mock"\n', { mode: 0o600 });
    const isolated = {
      RODER_CONFIG_DIR: configDir, RODER_DATA_DIR: path.join(root, 'roder-data'),
      RODER_PROVIDER: 'mock', RODER_MODEL: 'mock',
    };
    const previous = new Map(Object.keys(isolated).map((key) => [key, process.env[key]]));
    Object.assign(process.env, isolated);
    try {
      const result = await sdk.requireAcpRuntimeBackend('roder').runtime.doctor();
      assert.equal(result.ok, true, JSON.stringify(result));
    } finally {
      for (const [key, value] of previous) {
        if (value === undefined) delete process.env[key];
        else process.env[key] = value;
      }
    }
  });

  await t.test('stop and restart clean the actual host registry', async () => {
    await Promise.all([service.stop(), service.stop()]);
    assert.equal(sdk.getAcpRuntimeBackend('roder'), null);
    await service.start({ stateDir: path.join(root, 'service-state') });
    assert.ok(sdk.requireAcpRuntimeBackend('roder').runtime instanceof RoderRuntime);
    await service.stop();
    assert.equal(sdk.getAcpRuntimeBackend('roder'), null);
  });

  const cli = (args) => spawnSync(process.execPath, [path.join(hostPackage, 'openclaw.mjs'), ...args], {
    env, cwd: root, encoding: 'utf8', timeout: 45_000,
  });
  const successfulCli = (args) => {
    const result = cli(args);
    assert.equal(result.status, 0, `${args.join(' ')}\n${result.stdout}\n${result.stderr}\n${result.error ?? ''}`);
    return result.stdout;
  };

  await t.test('isolated real CLI installs and discovers the packed plugin', () => {
    successfulCli([
      'plugins', 'install', '--link', installedRoot, '--force',
      '--acknowledge-install-policy-warning', '--accept-capabilities',
    ]);
    const listed = JSON.parse(successfulCli(['plugins', 'list', '--json']));
    const discovered = listed.plugins.find((item) => item.id === 'roder');
    assert.ok(discovered, 'real plugin discovery must find roder');
    assert.equal(discovered.status, 'loaded', JSON.stringify(discovered));
    assert.equal(discovered.enabled, true);
    assert.equal(discovered.configSchema, true);
    assert.equal(discovered.dependencyStatus.requiredInstalled, true);
  });

  await t.test('real CLI accepts the backend config and rejects unknown plugin keys', async () => {
    const config = JSON.parse(await readFile(env.OPENCLAW_CONFIG_PATH, 'utf8'));
    config.acp = {
      enabled: true, dispatch: { enabled: true }, backend: 'roder',
      allowedAgents: ['roder'], defaultAgent: 'roder',
    };
    config.plugins.entries.roder.config = { command: binary ? path.resolve(binary) : 'roder', cwd: workspace };
    await writeFile(env.OPENCLAW_CONFIG_PATH, JSON.stringify(config));
    const valid = JSON.parse(successfulCli(['config', 'validate', '--json']));
    assert.equal(valid.valid, true, JSON.stringify(valid));
    config.plugins.entries.roder.config.unknownOption = true;
    await writeFile(env.OPENCLAW_CONFIG_PATH, JSON.stringify(config));
    const rejected = cli(['config', 'validate', '--json']);
    assert.notEqual(rejected.status, 0);
    const invalid = JSON.parse(rejected.stdout);
    assert.equal(invalid.valid, false, JSON.stringify(invalid));
  });
});
