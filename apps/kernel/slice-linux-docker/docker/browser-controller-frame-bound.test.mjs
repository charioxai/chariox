import assert from 'node:assert/strict';
import test from 'node:test';
import { withBrowserFrames } from './browser-controller-frames.mjs';
import { observeBrowserNote, NOTE_OBSERVER_EXPRESSION } from './browser-controller-notes.mjs';

// MP-08/MP-10/MP-11: the frame bound, enumerated instead of exemplified. A scenario is a top frame with `top`
// children c0..; `isolated` lists children that live in their own renderer (a placeholder in the top tree, a
// renderer tree with `size` descendants); `nested` makes the first descendant isolated again (one frame below);
// `reverse` reverses the order the browser lists its targets in. As in the browser, an iframe target is named
// after the frame it renders.
const MAX_FRAMES = 64;
const frame = (id, parent) => ({ id, ...(parent ? { parentId: parent } : {}), loaderId: 'doc-' + id, url: 'https://example.test/' + id });
const node = (id, parent, childFrames = []) => ({ frame: frame(id, parent), childFrames });

function build({ top, isolated, nested, reverse }) {
  const sessionOf = new Map([['top', 'page']]);
  const trees = new Map([['page', node('top', undefined, Array.from({ length: top }, (_, i) => node('c' + i, 'top')))]]);
  const targets = [];
  for (let i = 0; i < top; i++) sessionOf.set('c' + i, 'page');
  const renderer = (id, parent, descendants) => {
    const session = 's-' + id;
    trees.set(session, node(id, parent, descendants));
    targets.push({ type: 'iframe', targetId: id, parentFrameId: parent });
    for (const child of descendants) if (!sessionOf.has(child.frame.id)) sessionOf.set(child.frame.id, session);
    sessionOf.set(id, session);
    return session;
  };
  for (const { at, size } of isolated) {
    const id = 'c' + at;
    const own = Array.from({ length: size }, (_, j) => node(`r${at}_${j}`, id));
    renderer(id, 'top', own);
    if (nested && size > 0) renderer(own[0].frame.id, id, [node(`n${at}`, own[0].frame.id)]);
  }
  targets.push({ type: 'iframe', targetId: 'foreign', parentFrameId: 'elsewhere' });
  if (reverse) targets.reverse();
  return { trees, targets, sessionOf };
}

// What the bound promises, whatever the order: the top tree is admitted first, every retained frame is read through
// the renderer that owns it, and nothing beyond the bound is returned or observed.
function check(scenario, limit, owners, label) {
  const { top, isolated, nested } = scenario;
  const topKept = Math.min(top + 1, limit);
  const all = build(scenario).sessionOf;
  assert.ok(owners.size <= limit, `${label}: ${owners.size} frames kept, bound ${limit}`);
  for (const [id, session] of owners) assert.ok(session === all.get(id), `${label}: ${id} read through ${session}, owned by ${all.get(id)}`);
  assert.ok(owners.has('top') || limit === 0, `${label}: the top frame is kept`);
  for (let i = 0; i < top; i++) assert.ok(owners.has('c' + i) === (i < topKept - 1), `${label}: child c${i} ${i < topKept - 1 ? 'is retained' : 'is beyond the bound'}`);
  const reachable = topKept + isolated.filter(({ at }) => at < topKept - 1).reduce((sum, { size }) => sum + size, 0);
  if (nested) assert.ok(owners.size >= topKept, `${label}: the top tree is kept`);
  else assert.equal(owners.size, Math.min(limit, reachable), `${label}: the bound is filled from the frames reachable through retained parents`);
}

// The work promises the same: one attachment and one tree read per retained renderer, never one per omitted target.
function boundedWork(connection, owners, label) {
  const { attachedTargets, trees } = connection.calls;
  for (const target of attachedTargets) assert.ok(owners.has(target), `${label}: ${target} was opened although it is beyond the bound`);
  assert.ok(new Set(attachedTargets).size === attachedTargets.length, `${label}: a renderer was opened twice`);
  assert.ok(trees.filter((session) => session !== 'page').length === attachedTargets.length, `${label}: more than one tree read per renderer`);
}

function* scenarios(limit, { tops, positions, sizes }) {
  for (const top of tops) {
    const at = positions(top);
    const choices = [[], ...at.map((a) => [a]), ...at.flatMap((a, i) => at.slice(i + 1).map((b) => [a, b]))];
    for (const chosen of choices) {
      const options = chosen.reduce((acc) => acc.flatMap((rest) => sizes.map((size) => [...rest, size])), [[]]);
      for (const given of options) for (const nested of [false, true]) for (const reverse of [false, true]) {
        yield { top, isolated: chosen.map((position, i) => ({ at: position, size: given[i] })), nested, reverse };
      }
    }
  }
}
const describe = (s) => `top=${s.top} isolated=${JSON.stringify(s.isolated)} nested=${s.nested} reverse=${s.reverse}`;

// `unattachable` names renderers that are gone by the time the browser is asked to open them.
function cdp(scenario, unattachable = () => false) {
  const { trees, targets } = build(scenario);
  const calls = { attached: new Set(), detached: new Set(), trees: [], attachedTargets: [] };
  const connection = { calls, async send(method, params, sessionId) {
    if (method === 'Page.getFrameTree') { calls.trees.push(sessionId); return { frameTree: trees.get(sessionId) }; }
    if (method === 'Target.getTargets') return { targetInfos: targets };
    if (method === 'Target.attachToTarget') {
      if (unattachable(params.targetId)) throw new Error('No target with given id found');
      calls.attachedTargets.push(params.targetId);
      const session = 's-' + params.targetId; calls.attached.add(session); return { sessionId: session }; }
    if (method === 'Target.detachFromTarget') { calls.detached.add(params.sessionId); return {}; }
    return {};
  } };
  return connection;
}

test('MP-08/MP-10/MP-11 withBrowserFrames keeps at most the bound and reads every retained frame through its owner (bounded exhaustive, bound 4)', async () => {
  const limit = 4;
  let cases = 0;
  for (const scenario of scenarios(limit, { tops: [0, 1, 2, 3, 4, 5, 6], positions: (top) => Array.from({ length: top }, (_, i) => i), sizes: [0, 1, limit + 2] })) {
    const connection = cdp(scenario);
    const owners = new Map();
    const info = await withBrowserFrames(connection, 'page', 'tab', 'doc-top', async (entries, state) => {
      const ids = new Set();
      for (const entry of entries) {
        const visit = (tree) => { owners.set(tree.frame.id, entry.sessionId); ids.add(tree.frame.id); (tree.childFrames ?? []).forEach(visit); };
        visit(entry.tree);
      }
      assert.ok(ids.size <= limit, `${describe(scenario)}: ${ids.size} frames returned`);
      return state;
    }, { limit });
    if (!scenario.nested) assert.equal(info.truncated, scenario.top + 1 + scenario.isolated.reduce((sum, { size }) => sum + size, 0) > limit, `${describe(scenario)}: truncation is reported exactly when frames were left out`);
    else if (scenario.top + 1 > limit) assert.ok(info.truncated, `${describe(scenario)}: truncation is reported`);
    check(scenario, limit, owners, describe(scenario));
    boundedWork(connection, owners, describe(scenario));
    assert.deepEqual([...connection.calls.attached].sort(), [...connection.calls.detached].sort(), `${describe(scenario)}: every attached renderer is detached`);
    cases++;
  }
  assert.ok(cases > 1000, `${cases} scenarios`);
});

// The same scenarios through the note observer at the real bound: the frames it polls are the frames it keeps.
function noteBrowser(scenario, unattachable) {
  const connection = cdp(scenario, unattachable);
  const worlds = new Map(), polled = new Map(), ownerReads = new Set();
  let contexts = 100;
  const send = connection.send.bind(connection);
  connection.send = async (method, params, sessionId) => {
    if (method === 'DOM.getFrameOwner') { ownerReads.add(params.frameId); return { backendNodeId: 1 }; }
    if (method === 'Page.createIsolatedWorld') { worlds.set(++contexts, { frameId: params.frameId, sessionId }); return { executionContextId: contexts }; }
    if (method === 'DOM.resolveNode') return { object: { objectId: 'owner' } };
    if (method === 'Runtime.callFunctionOn') return { result: { value: false } };
    if (method === 'Runtime.releaseObject') return {};
    if (method === 'Runtime.evaluate') {
      if (params.expression === NOTE_OBSERVER_EXPRESSION) return { result: { value: true } };
      polled.set(params.contextId, sessionId);
      return { result: { value: null } };
    }
    return send(method, params, sessionId);
  };
  const browser = { ensureConnection: async () => connection, ensureTargetSession: async () => 'page', ensureFocusWorld: async () => { worlds.set(42, { frameId: 'top', sessionId: 'page' }); return { contextId: 42 }; } };
  return { browser, connection, worlds, polled, ownerReads };
}

async function observed(scenario, unattachable) {
  const { browser, connection, worlds, polled, ownerReads } = noteBrowser(scenario, unattachable);
  await observeBrowserNote(browser, { target_id: 'tab', document_id: 'doc-top' });
  const owners = new Map();
  for (const [context, session] of polled) {
    const world = worlds.get(context);
    assert.ok(world.sessionId === session, `world of ${world.frameId} polled through ${session}`);
    owners.set(world.frameId, session);
  }
  return { owners, connection, ownerReads };
}

test('MP-08/MP-10/MP-11 a note is observed only in retained frames, each through its owner (bounded exhaustive at the real bound)', async () => {
  const limit = MAX_FRAMES;
  let cases = 0;
  const positions = (top) => [...new Set([0, limit - 3, limit - 2, limit - 1, top - 1])].filter((at) => at >= 0 && at < top);
  for (const scenario of scenarios(limit, { tops: [limit - 2, limit - 1, limit, limit + 1, limit + 6], positions, sizes: [0, 1, limit + 6] })) {
    const { owners, connection, ownerReads } = await observed(scenario);
    check(scenario, limit, owners, describe(scenario));
    for (const id of ownerReads) assert.ok(owners.has(id), `${describe(scenario)}: the owner of ${id} was read beyond the bound`);
    boundedWork(connection, owners, describe(scenario));
    assert.deepEqual([...connection.calls.attached].sort(), [...connection.calls.detached].sort(), `${describe(scenario)}: every attached renderer is detached`);
    cases++;
  }
  assert.ok(cases > 1000, `${cases} scenarios`);
});

// The review's case: 64 children keep f0..f62 under the bound and f0 is isolated.
test('MP-08/MP-10/MP-11 review case: a retained isolated child of a page with one frame too many is read through its renderer', async () => {
  const scenario = { top: MAX_FRAMES, isolated: [{ at: 0, size: 0 }], nested: false, reverse: false };
  const { owners } = await observed(scenario);
  assert.equal(owners.get('c0'), 's-c0');
  assert.equal(owners.size, MAX_FRAMES);
  assert.ok(!owners.has('c' + (MAX_FRAMES - 1)), 'the first frame beyond the bound is not observed');
});

// The review's case: a thousand isolated placeholder children keep the top and 63 of them. The others are never
// opened, so a read costs 63 attachments however many advertising iframes the browser holds, and one that is gone
// by now cannot fail it.
const isolatedChildren = (top) => ({ top, isolated: Array.from({ length: top }, (_, at) => ({ at, size: 0 })), nested: false, reverse: false });
const beyondBound = (targetId) => Number(targetId.slice(1)) >= MAX_FRAMES - 1;

test('MP-08/MP-10/MP-11 a page with a thousand isolated children opens only the renderers it keeps, even when the others are gone', async () => {
  const scenario = isolatedChildren(1000);
  for (const unattachable of [() => false, beyondBound]) {
    const connection = cdp(scenario, unattachable);
    const owners = new Map();
    const info = await withBrowserFrames(connection, 'page', 'tab', 'doc-top', async (entries, state) => {
      for (const entry of entries) owners.set(entry.frame.id, entry.sessionId);
      return state;
    });
    assert.ok(info.truncated, 'truncation is reported');
    assert.equal(owners.size, MAX_FRAMES);
    assert.equal(connection.calls.attachedTargets.length, MAX_FRAMES - 1, 'one attachment per retained renderer');
    assert.equal(connection.calls.trees.filter((session) => session !== 'page').length, MAX_FRAMES - 1, 'one tree read per retained renderer');
    boundedWork(connection, owners, 'thousand children');
    assert.deepEqual([...connection.calls.attached].sort(), [...connection.calls.detached].sort(), 'every attached renderer is detached');
  }
});

test('MP-08/MP-10/MP-11 a note on a page with a thousand isolated children is observed in the retained frames although the others are gone', async () => {
  const { owners, connection } = await observed(isolatedChildren(1000), beyondBound);
  assert.equal(owners.size, MAX_FRAMES);
  assert.equal(connection.calls.attachedTargets.length, MAX_FRAMES - 1);
  boundedWork(connection, owners, 'thousand children');
});
