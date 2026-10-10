// MD-N2/N5 / MP-08/MP-10/MP-11: production isolated observer and controller
// registry replay over synthetic DOM/CDP, never real credentials or providers.
import assert from 'node:assert/strict';
import test from 'node:test';
import { createContext, runInContext } from 'node:vm';
import { TextEncoder } from 'node:util';
import { observeBrowserNote } from './browser-controller-notes.mjs';
import { handleBrowserControllerRequest } from './browser-controller.mjs';

const secret = 'synthetic-vault-secret';

function fixture(text, start, end, { splitAt, shadow = false, child = false, protectedAttribute } = {}) {
  class ShadowRoot {}
  const root = shadow ? new ShadowRoot() : {};
  const protectedElement = protectedAttribute ? {
    closest: selector => selector.includes(`[${protectedAttribute}]`) ? {} : null,
    getRootNode: () => ({})
  } : null;
  if (shadow) root.host = protectedElement;
  root.nodes = (splitAt == null ? [text] : [text.slice(0, splitAt), text.slice(splitAt)]).map(data => ({
    data, length: data.length, getRootNode: () => root, parentElement: shadow ? null : protectedElement,
  }));
  const body = shadow ? { nodes: [] } : root;
  const locate = offset => {
    for (const node of root.nodes) {
      if (offset <= node.length) return [node, offset];
      offset -= node.length;
    }
    throw new Error('MD-N5: invalid fixture offset');
  };
  const [startContainer, startOffset] = locate(start);
  const [endContainer, endOffset] = locate(end);
  const selection = { isCollapsed: false, rangeCount: 1, getRangeAt: () => ({
    startContainer, startOffset, endContainer, endOffset,
    getBoundingClientRect: () => ({ x: 10, y: 20, width: 30, height: 10 }),
  }) };
  const context = createContext({
    ShadowRoot, TextEncoder,
    document: {
      body, documentElement: body,
      getSelection: () => selection,
      addEventListener() {},
      createTreeWalker(current, kind) {
        const nodes = kind === 1 ? (shadow && current === body ? [{ shadowRoot: root }] : []) : current.nodes;
        let i = 0;
        return { nextNode: () => nodes[i++] ?? null };
      },
      createRange() {
        let at = 0, end = 0;
        const offset = node => root.nodes.slice(0, root.nodes.indexOf(node)).reduce((n, item) => n + item.length, 0);
        return {
          selectNodeContents(node) { at = offset(node); end = at + node.length; },
          setStart(node, value) { at = offset(node) + value; },
          collapse(start) { if (!start) at = end; },
          compareBoundaryPoints(_, other) { return at - other.at(); },
          at: () => at,
          getClientRects: () => [{}],
        };
      },
    },
    NodeFilter: { SHOW_ELEMENT: 1, SHOW_TEXT: 4 },
    Range: { START_TO_START: 0 },
    MutationObserver: class { observe() {} },
  });
  const tree = { frame: { id: 'top', loaderId: 'doc', url: 'https://fixture.test/' } };
  if (child) tree.childFrames = [{ frame: { id: 'child', parentId: 'top', loaderId: 'child-doc', url: 'https://child.test/' } }];
  const calls = [];
  const connection = { async send(method, params = {}) {
    calls.push({ method, contextId: params.contextId }); // Do not retain protected evaluator arguments.
    if (method === 'Target.getTargets') return { targetInfos: [] };
    if (method === 'Page.getFrameTree') return { frameTree: structuredClone(tree) };
    if (method === 'Page.createIsolatedWorld') return { executionContextId: 43 };
    if (method === 'DOM.getFrameOwner') return { backendNodeId: 1 };
    if (method === 'DOM.getBoxModel') return { model: { content: [0,0,100,0,100,100,0,100] } };
    if (method === 'Runtime.evaluate') {
      assert.ok(Number.isSafeInteger(params.contextId) && params.contextId > 0);
      if (params.expression === '({width:innerWidth,height:innerHeight})') return { result: { value: { width: 100, height: 100 } } };
      const selected = !child || params.contextId === 43;
      context.document.getSelection = () => selected ? selection : null;
      const value = runInContext(params.expression, context);
      return { result: { value: value == null ? value : JSON.parse(JSON.stringify(value)) } };
    }
    throw new Error(`MD-N5: unexpected CDP method ${method}`);
  } };
  const world = { contextId: 42 };
  const browser = {
    protectedValues: new Set(),
    ensureConnection: async () => connection,
    ensureTargetSession: async () => 'top',
    ensureFocusWorld: async () => world,
    observeNote(request) { return observeBrowserNote(this, request); },
  };
  let id = 0;
  return {
    calls, browser,
    capture: (values = []) => handleBrowserControllerRequest({
      id: ++id, method: 'browser.notes.observe', protected_values: values,
      params: { target_id: 'target', document_id: 'doc' },
    }, { browser }),
  };
}

test('MD-N2 / MP-10: withhold complete and partial protected selections before quote splitting', async t => {
  const text = `before ${secret} after`, at = text.indexOf(secret);
  for (const [a, b] of [[0, 10], [10, secret.length], [3, 16], [0, secret.length]]) await t.test(`MD-N2 / MP-10: selection offsets ${a}:${b}`, async () => {
    const f = fixture(text, at + a, at + b);
    const result = await f.capture([secret]);
    assert.equal(result.ok, true, JSON.stringify(result));
    assert.equal(result.result.selection, null);
  });
});

test('MD-N2 / MP-10: protect spans across prefix/suffix 64-code-point cutoffs', async () => {
  for (const secretText of [secret, `synthetic-${'x'.repeat(128)}-vault-secret`, `synthetic-${'x'.repeat(1024)}-vault-secret`]) {
    for (const text of [`${secretText}${'🙂'.repeat(60)}selected`, `selected${'🙂'.repeat(60)}${secretText}`]) {
      const at = text.indexOf('selected');
      assert.equal((await fixture(text, at, at + 8).capture([secretText])).result.selection, null);
    }
  }
});

test('MD-N2 / MP-11: raw protection covers text-node boundaries, shadow roots and child frames', async () => {
  const text = `before ${secret} after`, at = text.indexOf(secret);
  for (const options of [{ splitAt: at + 10 }, { shadow: true }, { child: true }]) {
    const f = fixture(text, at, at + 10, options);
    assert.equal((await f.capture([secret])).result.selection, null);
    assert.ok(f.calls.filter(c => c.method === 'Runtime.evaluate').every(c => c.contextId > 0));
  }
});

test('MD-N2 / MP-10: replayed retired values protect an already installed observer', async () => {
  const f = fixture(`before ${secret} after`, 7, 17);
  assert.equal((await f.capture()).result.selection.quote.exact, 'synthetic-');
  // Same kernel capture policy carries live and retired values on recovery.
  assert.equal((await f.capture([secret])).result.selection, null);
  assert.equal((await f.capture()).result.selection, null);
});

test('MD-N2 / MP-10: use the existing observation variants before splitting transformed echoes', async () => {
  const value = 'synthetic-Vault<&value';
  for (const transformed of [value.toUpperCase(), encodeURIComponent(value), Buffer.from(value).toString('base64'), Buffer.from(value).toString('hex')]) {
    assert.equal((await fixture(transformed, 1, 6).capture([value])).result.selection, null);
  }
});

test('MD-N2 / MP-10: benign quotes preserve raw DOM offsets and Unicode context', async () => {
  const text = `${secret}${'🙂'.repeat(65)}selected${'🙂'.repeat(65)}${secret}`;
  const at = text.indexOf('selected'), f = fixture(text, at, at + 8);
  const result = await f.capture([secret]);
  assert.deepEqual(result.result.selection.quote, { exact: 'selected', prefix: '🙂'.repeat(64), suffix: '🙂'.repeat(64) });
  const range = JSON.parse(JSON.parse(result.result.selection.hint).range);
  assert.deepEqual(range, { start: at, end: at + 8 });
});

for (const protectedAttribute of ['data-chariox-secret','data-chariox-observation-protected']) {
  for (const shadow of [false,true]) test(`Notes withholds ${protectedAttribute} selection in ${shadow?'shadow':'light'} DOM`, async () => {
    const result=await fixture('synthetic-private-text',0,22,{shadow,protectedAttribute}).capture();
    assert.equal(result.ok,true);
    assert.equal(result.result.selection,null);
  });
}
