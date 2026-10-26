// MP-08/MP-10/MP-11: production controller and App bridge with an external
// renderer in a separate realm. The kernel and backend are exercised by Rust.
import vm from 'node:vm';
import { randomUUID } from 'node:crypto';
import { appendFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
const [directory, root] = process.argv.slice(2);
const { AppTabs } = await import(pathToFileURL(join(directory, 'browser-controller-apps.mjs')));
let releaseClose, retirement;
const originalAgent = AppTabs.prototype.agent;
AppTabs.prototype.agent = async function (params) {
  if (params.call === 'registrations' && existsSync(join(root, 'registration-error'))) {
    throw new Error('registration-vault-value');
  }
  if (!retirement && (existsSync(join(root, 'fail-idle-close')) || existsSync(join(root, 'successful-idle-close')))) {
    const app = [...this.apps.values()].find(app => app.targetId === params.target_id);
    app.lastActivity = this.now() - this.idleMs;
    retirement = this.retireIdle();
    await new Promise(resolve => setImmediate(resolve));
  }
  return originalAgent.call(this, params);
};
const originalOpen = AppTabs.prototype.open;
AppTabs.prototype.open = function (params) {
  appendFileSync(join(root, 'app-opens'), 'open\n');
  releaseClose?.();
  return originalOpen.call(this, params);
};
const { BrowserControllerStdioServer } = await import(pathToFileURL(join(directory, 'browser-controller.mjs')));
let listener, bridge, context, targets = 0;
const resume = () => new Promise(resolve => {
  const wait = () => existsSync(join(root, 'resume-handler')) ? resolve() : setTimeout(wait, 10);
  wait();
});
const connection = {
  isOpen: () => true,
  subscribe(fn) { listener = fn; return () => { listener = undefined; }; },
  async send(method, params = {}, sessionId) {
    if (method === 'Target.getTargets') return { targetInfos: [] };
    if (method === 'Target.createTarget') return { targetId: ++targets === 1 ? 'actor-target' : 'actor-target-2' };
    if (method === 'Browser.getWindowForTarget') return { windowId: 1 };
    if (method === 'Browser.getWindowBounds') return { bounds: { windowState: 'fullscreen' } };
    if (method === 'Page.getFrameTree') return { frameTree: { frame: { loaderId: 'actor-document' } } };
    if (method === 'Page.addScriptToEvaluateOnNewDocument') bridge = params.source;
    if (method === 'Page.navigate') {
      context = vm.createContext({ crypto: { randomUUID }, TextEncoder, setTimeout, clearTimeout, resume,
        __charioxAppCall(payload) { listener?.({ method: 'Runtime.bindingCalled', sessionId, params: { name: '__charioxAppCall', payload } }); },
      });
      vm.runInContext(bridge, context);
      vm.runInContext(`chariox.view.state(() => ({ready:true})); chariox.action({name:'roundtrip',description:'Backend round trip',effect:'write',inputSchema:{type:'object'}}, async ({method}) => {
        await chariox.call(method, {text:'before revoke'});
        await resume();
        return chariox.call(method, {text:'after revoke'});
      });`, context);
      listener?.({ method: 'Page.frameNavigated', sessionId, params: { frame: { id: 'actor-frame', loaderId: 'actor-document' } } });
    }
    if (method === 'Runtime.evaluate') {
      let value = vm.runInContext(params.expression, context);
      if (params.awaitPromise) value = await value;
      return { result: { value } };
    }
    return {};
  },
};
const browser = {
  ensureConnection: async () => connection,
  ensureTargetSession: async () => 'actor-session',
  closePageTarget: async () => {
    if (existsSync(join(root, 'fail-idle-close'))) {
      await new Promise((_, reject) => { releaseClose = () => reject(new Error('CDP close unavailable')); });
    } else if (existsSync(join(root, 'successful-idle-close'))) {
      await new Promise(resolve => { releaseClose = resolve; });
    }
  },
  close: async () => {},
};
await new BrowserControllerStdioServer({ browser }).run();
