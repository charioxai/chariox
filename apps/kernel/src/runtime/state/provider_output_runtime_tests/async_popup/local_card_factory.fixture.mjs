// Regression transport fixture: production host and geometry, deterministic external CDP.
// This does not replace D3 live browser/provider acceptance.
import { runInNewContext } from 'node:vm';
import { appendFileSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
const root = process.argv[2], modules = process.argv[3];
const { KernelBrowserHost } = await import(pathToFileURL(path.join(modules, 'kernel-browser-host.mjs')).href);
const { BrowserControllerStdioServer } = await import(pathToFileURL(path.join(modules, 'browser-controller.mjs')).href);
appendFileSync(path.join(root, 'factory-pids.log'), `${process.pid} node-factory-fixture\n`);
const host = new KernelBrowserHost(root), document = {};
const state = { document_id: 'doc', width: 800, height: 600, box: { left: 80, top: 90, right: 180, bottom: 140 } };
const node = { isConnected: true, ownerDocument: document, innerText: '', getBoundingClientRect: () => state.box };
const tab = { tab_id: 'one', target_id: 'target', document_id: 'doc', url: 'https://example.com/card' };
const connection = { subscribe: () => () => {}, async send(method, params) {
  if (method === 'Page.getFrameTree') return { frameTree: { frame: { id: 'frame', loaderId: state.document_id } } };
  if (method === 'DOM.resolveNode') return { object: { objectId: 'element' } };
  if (method === 'Runtime.callFunctionOn' && params.functionDeclaration.includes('function trackCardElement')) return {result:{objectId:'resolver'}};
  if (method === 'Runtime.callFunctionOn' && params.objectId === 'resolver') return {result:(state.replacement ?? node).isConnected ? {objectId:'element'} : {subtype:'null',value:null}};
  if (method === 'Runtime.callFunctionOn' && params.functionDeclaration.includes('function bindVerificationInput')) return {result:{objectId:'cleanup'}};
  if (method === 'Runtime.callFunctionOn') {
    const context = { document, innerWidth: state.width, innerHeight: state.height, setTimeout, clearTimeout,
      requestAnimationFrame: callback => setTimeout(callback, 5), cancelAnimationFrame: clearTimeout };
    const result = await runInNewContext(`(${params.functionDeclaration})`, context).call(state.replacement ?? node, params.arguments[0].value, params.arguments[1]?.value ?? null, params.arguments[2]?.objectId === 'resolver' ? () => state.replacement ?? (node.isConnected ? node : null) : undefined);
    return { result: { value: JSON.parse(JSON.stringify(result)) } };
  }
  return {};
} };
host.generation = 1; host.start = async () => {};
host.tabs.set(tab.tab_id, tab);
host.target = async command => ({ ...tab, tab_id: command.tab_id });
host.displayTarget = async command => ({ ...tab, tab_id: command.tab_id, document_id: state.document_id });
host.browser = { close: async () => {}, reconcile: async () => ({ tabs: [{ ...tab }], active_target_id: tab.target_id }), ensureConnection: async () => connection, ensureTargetSession: async () => 'session', ensureFocusWorld: async () => ({ contextId: 7 }),
  snapshot: async () => ({ document_id: state.document_id, dom_nodes: [{ node_ref: 'backend:42', document_index: 0 }], dom_documents: [] }) };

const record = label => appendFileSync(path.join(root, 'factory-operations.log'), `${label}\n`);
const send = connection.send.bind(connection);
connection.send = async (method, params) => { record(method); return send(method, params); };
const server = new BrowserControllerStdioServer({ handleRequest: request => {
record(`${request.method}:${request.params?.op ?? ''}`);
return host.handle(request);
} });
await server.run();
process.exit(0);
