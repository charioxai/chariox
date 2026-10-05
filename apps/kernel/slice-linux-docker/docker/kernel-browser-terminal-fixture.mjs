// MD-N5 / MP-08/MP-10/MP-11: real host adapter over synthetic Chromium/CDP.
// Used only by the isolated Rust local-transport regression; no provider data.
import { BrowserControllerStdioServer } from './browser-controller.mjs';

// Import the host class without starting its standalone CLI. The fixture then
// serves the same adapter with its synthetic browser factory through stdio.
const command = process.argv[2];
process.argv[2] = 'MD-N5-fixture-import';
const { KernelBrowserHost } = await import('./kernel-browser-host.mjs');
process.argv[2] = command;

const pages = new Map();
let next = 0, revision = 0;
const chromium = {
  child: null,
  async start() { this.child = { exitCode: null, signalCode: null }; return 'http://127.0.0.1:1'; },
  async stop() { this.child = null; },
};
const connection = {
  async send(method, params = {}, session) {
    if (method === 'Target.getTargets') return { targetInfos: [...pages].map(([targetId, tab]) => ({ targetId, type: 'page', ...tab })) };
    if (method === 'Target.createTarget') {
      const targetId = `target-${++next}`;
      pages.set(targetId, { url: params.url, document_id: `doc-${++revision}` });
      return { targetId };
    }
    if (method === 'Target.closeTarget') { pages.delete(params.targetId); return {}; }
    if (method === 'Page.getFrameTree') return { frameTree: { frame: { id: 'frame', loaderId: pages.get(session)?.document_id } } };
    if (method === 'Page.createIsolatedWorld') return { executionContextId: 42 };
    if (method === 'Runtime.evaluate') return { result: { value: false } };
    if (method === 'Input.insertText') return {};
    throw new Error(`MD-N5: unexpected fixture CDP method ${method}`);
  },
};
const host = new KernelBrowserHost(process.argv[3], {
  chromium,
  browserFactory: () => ({
    connection, ensureConnection: async () => connection, close: async () => {},
    reconcile: async () => ({ tabs: [...pages].map(([target_id, tab]) => ({ target_id, title: 'MD-N5 fixture', ...tab })) }),
    navigate: async ({ target_id, url }) => pages.set(target_id, { url, document_id: `doc-${++revision}` }),
    resolvePageTarget: async target => ({ connection, sessionId: target }),
    inputCapture: { run: async (_, __, operation) => operation() },
  }),
});
await new BrowserControllerStdioServer({ handleRequest: (request, options) => host.handle(request, options) }).run().finally(() => host.stop());
