// MD-N2 / MD-N5 / MP-08 / MP-10 / MP-11: real isolated observer code,
// synthetic DOM/transport only; no live browser or provider acceptance claim.
import assert from 'node:assert/strict';
import test from 'node:test';
import { createContext, runInContext } from 'node:vm';
import { NOTE_OBSERVER_EXPRESSION, observeBrowserNote } from './browser-controller-notes.mjs';

const quote = { exact: 'selected', prefix: 'before ', suffix: ' after' };
const contextual = 'before selected after';
const unrelated = 'other selected surroundings';

// TreeWalker never crosses shadow roots. Distinct root boxes let assertions
// verify which occurrence won, in addition to the returned anchor state.
function observer(texts) {
  class ShadowRoot {}
  const roots = texts.map((text, i) => ({
    nodes: [{ data: text, length: text.length }],
    box: { x: 10 + i * 100, y: 20, width: 30, height: 10 },
  }));
  for (const root of roots) for (const node of root.nodes) node.root = root;
  const body = roots[0];
  const context = createContext({
    ShadowRoot,
    document: {
      body, documentElement: body,
      getSelection: () => null,
      addEventListener() {},
      createTreeWalker(root, kind) {
        const nodes = kind === 1
          ? (root === body ? roots.slice(1).map(shadowRoot => ({ shadowRoot })) : [])
          : root.nodes;
        let i = 0;
        return { nextNode: () => nodes[i++] ?? null };
      },
      createRange() {
        let root;
        return {
          selectNodeContents(node) { root = node.root; },
          setStart(node) { root = node.root; },
          setEnd() {},
          getClientRects: () => [root.box],
          getBoundingClientRect: () => root.box,
        };
      },
    },
    NodeFilter: { SHOW_ELEMENT: 1, SHOW_TEXT: 4 },
    MutationObserver: class { observe() {} },
  });
  return { run: expression => runInContext(expression, context) };
}

function fixture(frameTexts, { oopif = false, navigateOnFallback = false } = {}) {
  const calls = [];
  const worlds = frameTexts.map(observer);
  const children = frameTexts.slice(1).map((_, i) => ({
    frame: { id: `child-${i + 1}`, parentId: 'top', loaderId: `child-doc-${i + 1}`, url: `https://child-${i + 1}.test/` },
  }));
  const tree = { frame: { id: 'top', loaderId: 'doc', url: 'https://fixture.test/' }, childFrames: children };
  const connection = { async send(method, params, session) {
    calls.push({ method, params, session });
    if (method === 'Target.getTargets') return { targetInfos: oopif ? children.map(c => ({ type: 'iframe', targetId: c.frame.id })) : [] };
    if (method === 'Target.attachToTarget') return { sessionId: params.targetId };
    if (method === 'Target.detachFromTarget') return {};
    if (method === 'Page.getFrameTree') return { frameTree: structuredClone(session === 'top' ? tree : children.find(c => c.frame.id === session)) };
    if (method === 'Page.createIsolatedWorld') return { executionContextId: Number(params.frameId.split('-')[1]) + 42 };
    if (method === 'DOM.getFrameOwner') return { backendNodeId: 1 };
    if (method === 'DOM.getBoxModel') return { model: { content: [200, 300, 300, 300, 300, 400, 200, 400] } };
    if (method === 'Runtime.evaluate') {
      assert.ok(Number.isSafeInteger(params.contextId) && params.contextId >= 42);
      if (params.expression === '({width:innerWidth,height:innerHeight})') return { result: { value: { width: 100, height: 100 } } };
      const value = worlds[params.contextId - 42].run(params.expression);
      if (navigateOnFallback && params.expression.endsWith(',true)')) children[0].frame.loaderId = 'replaced';
      return { result: { value: value == null ? value : JSON.parse(JSON.stringify(value)) } };
    }
    throw new Error(`Unexpected CDP method: ${method}`);
  } };
  return {
    calls,
    read: (savedQuote = quote) => observeBrowserNote({
      ensureConnection: async () => connection,
      ensureTargetSession: async () => 'top',
      ensureFocusWorld: async () => ({ contextId: 42 }),
    }, { target_id: 'target', document_id: 'doc', quote: savedQuote }),
  };
}

test('MD-N2 / MP-10: unique contextual match wins over exact text in other shadow roots', async () => {
  for (const texts of [[contextual, unrelated], [contextual, `${unrelated} ${unrelated}`], [unrelated, contextual]]) {
    const result = await fixture([texts]).read();
    assert.equal(result.anchoring.anchor_state, 'attached');
    assert.equal(result.anchoring.box_css.x, 10 + texts.indexOf(contextual) * 100);
  }
});

test('MD-N2 / MP-10: contextual match wins across owned frames and shadow roots in either order', async () => {
  for (const oopif of [false, true]) {
    for (const frameTexts of [
      [[contextual, unrelated], [unrelated, `${unrelated} ${unrelated}`]],
      [[unrelated, `${unrelated} ${unrelated}`], [unrelated, contextual]],
    ]) {
      const f = fixture(frameTexts, { oopif });
      const result = await f.read();
      assert.equal(result.anchoring.anchor_state, 'attached');
      const childMatch = frameTexts[1].includes(contextual);
      assert.equal(result.url, childMatch ? 'https://child-1.test/' : 'https://fixture.test/');
      assert.equal(result.anchoring.box_css.x, childMatch ? 310 : 10);
      assert.equal(JSON.parse(result.anchoring.hint).frame_id, childMatch ? 'child-1' : 'top');
      if (oopif) assert.equal(f.calls.filter(c => c.method === 'Target.detachFromTarget').length, 1);
    }
  }
});

test('MD-N2 / MP-10: unique context wins over a single exact-only occurrence in another frame', async () => {
  for (const oopif of [false, true]) {
    for (const frameTexts of [[[contextual], [unrelated]], [[unrelated], [contextual]]]) {
      const result = await fixture(frameTexts, { oopif }).read();
      const childMatch = frameTexts[1][0] === contextual;
      assert.equal(result.anchoring.anchor_state, 'attached');
      assert.equal(result.anchoring.box_css.x, childMatch ? 210 : 10);
      assert.equal(JSON.parse(result.anchoring.hint).frame_id, childMatch ? 'child-1' : 'top');
    }
  }
});

test('MD-N2 / MP-10: exact-only fallback requires one occurrence across all roots and frames', async () => {
  for (const frames of [[[unrelated, 'empty']], [['empty'], [unrelated]]]) {
    assert.equal((await fixture(frames).read()).anchoring.anchor_state, 'attached');
  }
  for (const frames of [[[unrelated, unrelated]], [[unrelated], [unrelated]], [[`${unrelated} ${unrelated}`]]]) {
    const result = await fixture(frames).read();
    assert.equal(result.anchoring.anchor_state, 'ambiguous');
    assert.equal(result.anchoring.box_css, null);
  }
  const result = await fixture([['empty'], ['also empty']]).read();
  assert.equal(result.anchoring.anchor_state, 'missing');
  assert.equal(result.anchoring.box_css, null);
});

test('MD-N2 / MP-10: multiple contextual matches remain ambiguous across roots and frames', async () => {
  for (const frames of [[[contextual, contextual]], [[contextual], [contextual]], [[`${contextual} ${contextual}`]]]) {
    const result = await fixture(frames).read();
    assert.equal(result.anchoring.anchor_state, 'ambiguous');
    assert.equal(result.anchoring.box_css, null);
  }
});

test('MD-N2 / MP-11: fallback still fences frame navigation before returning an anchor', async () => {
  const f = fixture([['empty'], [unrelated]], { oopif: true, navigateOnFallback: true });
  await assert.rejects(f.read(), /frame changed during capture/);
  assert.equal(f.calls.filter(c => c.method === 'Target.detachFromTarget').length, 1);
});
