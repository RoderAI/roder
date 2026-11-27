import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { randomBytes, randomUUID } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import test from 'node:test';

const exec = promisify(execFile);
const hostPackage = process.env.OPENCLAW_TEST_PACKAGE;
const binary = process.env.RODER_TEST_BINARY;
const enabled = process.env.RODER_TEST_GATEWAY === '1';
const packageRoot = fileURLToPath(new URL('../..', import.meta.url));

async function unusedPort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const port = server.address().port;
  await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
  return port;
}

function assistantTexts(history) {
  return (history.messages ?? []).filter((message) => message.role === 'assistant').map((message) =>
    typeof message.content === 'string' ? message.content
      : (message.content ?? []).filter((item) => item.type === 'text').map((item) => item.text).join(''));
}

test('isolated real Gateway dispatches ACP chat prompts to native Roder', {
  skip: !(enabled && hostPackage && binary)
    && 'set RODER_TEST_GATEWAY=1, OPENCLAW_TEST_PACKAGE, and RODER_TEST_BINARY',
  timeout: 180_000,
}, async (t) => {
  assert.ok(path.isAbsolute(hostPackage), 'host package path must be absolute');
  assert.ok(path.isAbsolute(binary), 'Roder binary path must be absolute');
  const hostManifest = JSON.parse(await readFile(path.join(hostPackage, 'package.json'), 'utf8'));
  assert.equal(hostManifest.name, 'openclaw');
  assert.equal(hostManifest.version, '2026.10.9');
  const root = await mkdtemp(path.join(os.tmpdir(), 'roder-openclaw-gateway-test-'));
  const home = path.join(root, 'home');
  const stage = path.join(root, 'consumer');
  const workspace = path.join(root, 'workspace');
  const roderConfig = path.join(root, 'roder-config');
  const state = path.join(home, 'state');
  await Promise.all([mkdir(home), mkdir(stage), mkdir(workspace), mkdir(roderConfig)]);
  await mkdir(state);
  await writeFile(path.join(roderConfig, 'config.toml'), 'provider = "mock"\nmodel = "mock"\n', { mode: 0o600 });
  const port = await unusedPort();
  const token = randomBytes(32).toString('hex');
  let nativeInferenceAttempts = 0;
  const nativeInferenceGuard = http.createServer((_request, response) => {
    nativeInferenceAttempts += 1;
    response.writeHead(503);
    response.end('This test only permits Roder ACP inference.');
  });
  await new Promise((resolve, reject) => {
    nativeInferenceGuard.once('error', reject);
    nativeInferenceGuard.listen(0, '127.0.0.1', resolve);
  });
  const guardPort = nativeInferenceGuard.address().port;
  // Deliberately do not forward ambient API keys, channels, NODE_OPTIONS, or
  // another gateway's credentials/configuration to either owned child.
  const env = {
    PATH: `${path.dirname(process.execPath)}${path.delimiter}${process.env.PATH ?? ''}`,
    HOME: home, USERPROFILE: home,
    ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
    LANG: 'C.UTF-8', NO_COLOR: '1',
    TMPDIR: root, TEMP: root, TMP: root,
    XDG_CONFIG_HOME: path.join(home, 'config'),
    XDG_CACHE_HOME: path.join(home, 'cache'),
    XDG_DATA_HOME: path.join(home, 'data'),
    OPENCLAW_HOME: home,
    OPENCLAW_STATE_DIR: state,
    OPENCLAW_CONFIG_PATH: path.join(state, 'openclaw.json'),
    OPENCLAW_COMPILE_CACHE_DIR: path.join(home, 'compile-cache'),
    OPENCLAW_LOG_DIR: path.join(home, 'logs'),
    RODER_CONFIG_DIR: roderConfig, RODER_DATA_DIR: path.join(root, 'roder-data'),
    RODER_PROVIDER: 'mock', RODER_MODEL: 'mock',
  };
  let gateway;
  let exited;
  let log = '';
  let spawnError;
  t.after(async () => {
    if (gateway?.pid) {
      const signal = (name) => {
        try {
          if (process.platform === 'win32') gateway.kill(name);
          else process.kill(-gateway.pid, name);
        } catch (error) { if (error.code !== 'ESRCH') throw error; }
      };
      if (gateway.exitCode === null && gateway.signalCode === null) {
        signal('SIGTERM');
        if (!await Promise.race([exited.then(() => true), delay(5000, undefined, { ref: false }).then(() => false)])) {
          signal('SIGKILL');
          await Promise.race([exited, delay(3000, undefined, { ref: false })]);
        }
      }
      // Only this detached, test-owned process group is eligible for cleanup.
      // Remove straggling gateway workers even when the main child exited first.
      signal('SIGKILL');
    }
    nativeInferenceGuard.closeAllConnections();
    await new Promise((resolve) => nativeInferenceGuard.close(resolve));
    await rm(root, { recursive: true, force: true });
  });
  const run = (file, args, cwd = root) => exec(file, args, {
    cwd, env, timeout: 30_000, maxBuffer: 8 * 1024 * 1024,
  });
  const cli = async (args) => (await run(process.execPath,
    [path.join(hostPackage, 'openclaw.mjs'), ...args])).stdout;
  // The generic CLI call only asks for chat.send's write scope, whereas /acp
  // management on the internal channel requires admin. Use the host's public
  // client with explicit scopes and the token minted for this owned gateway.
  const rpcScript = `
    import { createRequire } from 'node:module';
    import { pathToFileURL } from 'node:url';
    const require = createRequire(process.argv[1]);
    const { callGatewayFromCli } = await import(pathToFileURL(require.resolve('openclaw/plugin-sdk/gateway-runtime')));
    const result = await callGatewayFromCli(process.argv[2], JSON.parse(process.argv[3]), JSON.parse(process.argv[4]),
      { scopes: ['operator.admin', 'operator.read', 'operator.write'] });
    console.log(JSON.stringify(result));
  `;
  const rpc = async (method, params = {}) => JSON.parse((await run(process.execPath, [
    '--input-type=module', '--eval', rpcScript, path.join(stage, 'consumer.cjs'), method,
    JSON.stringify({ url: `ws://127.0.0.1:${port}`, token, timeout: '10000' }), JSON.stringify(params),
  ])).stdout);
  const poll = async (description, read, accept, timeoutMs = 35_000) => {
    const deadline = Date.now() + timeoutMs;
    let last;
    do {
      if (spawnError) throw spawnError;
      if (nativeInferenceAttempts) throw new Error('Gateway attempted native provider inference instead of ACP dispatch');
      if (gateway && (gateway.exitCode !== null || gateway.signalCode !== null)) {
        throw new Error(`Owned gateway exited during ${description}: ${log}`);
      }
      try {
        last = await read();
        if (accept(last)) return last;
      } catch (error) {
        last = error.message;
      }
      await delay(300);
    } while (Date.now() < deadline);
    throw new Error(`${description} timed out; last result: ${JSON.stringify(last)}\n${log}`);
  };
  try {
    const packed = JSON.parse((await run('npm', [
      'pack', '--ignore-scripts', '--json', '--pack-destination', root,
    ], packageRoot)).stdout);
    await run('npm', ['install', '--offline', '--ignore-scripts', '--omit=optional',
      '--no-audit', '--no-fund', path.join(root, packed[0].filename)], stage);
    await symlink(hostPackage, path.join(stage, 'node_modules', 'openclaw'), 'dir');
    const installedRoot = path.join(stage, 'node_modules', '@roderai', 'openclaw');
    await writeFile(env.OPENCLAW_CONFIG_PATH, JSON.stringify({
      gateway: {
        mode: 'local', bind: 'loopback', port, auth: { mode: 'token', token },
        controlUi: { enabled: false },
      },
      agents: {
        ownership: 'explicit',
        defaults: { workspace, heartbeat: { every: '0m' }, model: { primary: 'mock/mock' } },
        entries: { main: { workspace }, roder: { workspace } },
      },
      models: {
        mode: 'replace', catalogRefresh: { enabled: false },
        // Host admission needs a known model even for ACP. If dispatch ever
        // falls through to native inference, only this owned loopback guard is reachable.
        providers: {
          mock: {
            baseUrl: `http://127.0.0.1:${guardPort}/v1`, apiKey: 'local-test-only',
            api: 'openai-completions',
            models: [{
              id: 'mock', name: 'Offline ACP admission model', reasoning: false, input: ['text'],
              cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
              contextWindow: 128000, maxTokens: 4096,
            }],
          },
        },
      },
      acp: { enabled: true, dispatch: { enabled: true }, backend: 'roder', allowedAgents: ['roder'], defaultAgent: 'roder' },
      plugins: {
        allow: ['roder'],
        load: { paths: [installedRoot] },
        entries: { roder: { enabled: true, config: { command: binary, cwd: workspace, permissionMode: 'deny-all' } } },
      },
      discovery: { mdns: { mode: 'off' } },
      browser: { enabled: false },
      cron: { enabled: false },
      update: { checkOnStart: false },
      logging: { file: path.join(root, 'gateway.log') },
    }), { mode: 0o600 });
    const validation = JSON.parse(await cli(['config', 'validate', '--json']));
    assert.equal(validation.valid, true, JSON.stringify(validation));
    gateway = spawn(process.execPath, [
      path.join(hostPackage, 'openclaw.mjs'), 'gateway', 'run',
      '--bind', 'loopback', '--port', String(port), '--auth', 'token', '--token', token,
    ], { cwd: root, env, detached: process.platform !== 'win32', stdio: ['ignore', 'pipe', 'pipe'] });
    exited = new Promise((resolve) => gateway.once('exit', resolve));
    gateway.once('error', (error) => { spawnError = error; });
    const appendLog = (data) => { log = `${log}${data}`.replaceAll(token, '[local-test-token]').slice(-24_000); };
    gateway.stdout.on('data', appendLog);
    gateway.stderr.on('data', appendLog);
    await poll('gateway readiness', () => rpc('health'), (health) => health.ok === true);
    const main = 'agent:main:main';
    await rpc('chat.send', {
      sessionKey: main, idempotencyKey: randomUUID(),
      message: `/acp spawn roder --mode persistent --thread off --cwd ${workspace}`,
    });
    const spawnHistory = await poll('ACP spawn command response',
      () => rpc('chat.history', { sessionKey: main, limit: 100 }),
      (result) => assistantTexts(result).length > 0);
    t.diagnostic(`ACP spawn response: ${JSON.stringify(assistantTexts(spawnHistory))}`);
    assert.ok(assistantTexts(spawnHistory).some((text) => /Spawned ACP session/.test(text)),
      `ACP spawn failed: ${JSON.stringify(assistantTexts(spawnHistory))}`);
    const sessions = await poll('ACP session creation', () => rpc('sessions.list', { limit: 100 }),
      (result) => result.sessions?.some((session) => /:acp:/.test(session.key)));
    const target = sessions.sessions.find((session) => /:acp:/.test(session.key)).key;
    t.diagnostic(`Gateway created ACP target ${target}`);
    for (let turn = 1; turn <= 2; turn += 1) {
      const response = await rpc('chat.send', {
        sessionKey: target, idempotencyKey: randomUUID(), message: `Gateway offline integration prompt ${turn}.`,
      });
      assert.ok(response.runId, JSON.stringify(response));
      const history = await poll(`ACP prompt ${turn} transcript`, () => rpc('chat.history', { sessionKey: target, limit: 100 }),
        (result) => assistantTexts(result).filter((text) => text === 'hello from roder').length >= turn);
      assert.equal(assistantTexts(history).filter((text) => text === 'hello from roder').length, turn);
      t.diagnostic(`ACP prompt ${turn}: persisted exact assistant reply "hello from roder"`);
    }
    await rpc('chat.send', { sessionKey: target, idempotencyKey: randomUUID(), message: '/acp status' });
    const status = await poll('ACP status transcript', () => rpc('chat.history', { sessionKey: target, limit: 100 }),
      (result) => assistantTexts(result).some((text) => /^backend: roder$/m.test(text) && /^state: idle$/m.test(text)));
    const statusText = assistantTexts(status).find((text) => /^backend: roder$/m.test(text));
    assert.match(statusText, /^backend: roder$/m);
    assert.match(statusText, /^state: idle$/m);
    assert.ok(statusText.includes(`session: ${target}`));
    t.diagnostic(`ACP status: ${JSON.stringify(statusText.split('\n').slice(0, 10).join('\n'))}`);
    assert.equal(nativeInferenceAttempts, 0, 'Gateway must dispatch to ACP, not native inference');
    t.diagnostic('Native provider guard received zero inference requests');
  } catch (error) {
    t.diagnostic(`Owned gateway log tail:\n${log}`);
    throw error;
  }
});
