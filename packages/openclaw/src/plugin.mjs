import {
  getAcpRuntimeBackend,
  registerAcpRuntimeBackend,
  unregisterAcpRuntimeBackend,
  tryDispatchAcpReplyHook,
} from 'openclaw/plugin-sdk/acp-runtime-backend';
import { createRoderPlugin } from './index.mjs';

export default createRoderPlugin({
  getAcpRuntimeBackend,
  registerAcpRuntimeBackend,
  unregisterAcpRuntimeBackend,
  tryDispatchAcpReplyHook,
});
