import { isAbsolute, join } from 'node:path';
import { configSchema, parseConfig } from './config.mjs';

/**
 * The registry is the real OpenClaw ACP SDK in production. Injection keeps unit
 * tests independent of the host without pretending to test host integration.
 */
export function createRoderPlugin(registry, { Runtime } = {}) {
  return {
    id: 'roder',
    name: 'Roder ACP Runtime',
    description: 'Run Roder as an OpenClaw ACP runtime backend.',
    configSchema,
    register(api) {
      const config = parseConfig(api.pluginConfig);
      // Backend registration handles ACP lifecycle commands. Chat dispatch is
      // a separate host hook; without it ACP messages fall through to the
      // gateway's native agent instead of reaching this backend.
      api.on('reply_dispatch', registry.tryDispatchAcpReplyHook, { eligibleDispatchKinds: ['acp'] });
      let runtime = null;
      let revision = 0;
      let queue = Promise.resolve();
      const enqueue = (operation) => {
        const result = queue.then(operation);
        queue = result.catch(() => {});
        return result;
      };
      const retract = (owned) => {
        if (registry.getAcpRuntimeBackend('roder')?.runtime === owned) {
          registry.unregisterAcpRuntimeBackend('roder');
        }
      };
      api.registerService({
        id: 'roder-acp',
        start(context) {
          const expectedRevision = revision;
          return enqueue(async () => {
            if (revision !== expectedRevision || runtime) return;
            if (typeof context.stateDir !== 'string' || !isAbsolute(context.stateDir) || context.stateDir.includes('\0')) {
              throw new TypeError('Roder service requires an absolute host stateDir');
            }
            const RuntimeClass = Runtime ?? (await import('./runtime.mjs')).RoderRuntime;
            if (revision !== expectedRevision) return;
            const candidate = new RuntimeClass({ ...config, stateDir: join(context.stateDir, 'roder-acp') });
            try {
              registry.registerAcpRuntimeBackend({ id: 'roder', runtime: candidate });
              runtime = candidate;
            } catch (error) {
              // A registry may throw after writing; retract only our own value.
              retract(candidate);
              await candidate.stop();
              throw error;
            }
          });
        },
        stop() {
          revision += 1;
          return enqueue(async () => {
            const owned = runtime;
            runtime = null;
            if (!owned) return;
            retract(owned);
            await owned.stop();
          });
        },
      });
    },
  };
}
