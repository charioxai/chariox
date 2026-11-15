import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { outputs } from '../scripts/generate.mjs';
import { createApp } from '../src/app.mjs';
import { tools as catalog } from '../src/catalog.mjs';
import { fakeGitHub, heads, memoryState, REPO } from './support.mjs';

const read = path => JSON.parse(readFileSync(new URL(`../${path}`, import.meta.url), 'utf8'));

test('the committed manifest routes, tools and UI actions are what the catalog generates', async () => {
  for (const [file, text] of Object.entries(await outputs())) {
    assert.equal(readFileSync(new URL(`../${file}`, import.meta.url), 'utf8'), text, `${file}: run node scripts/generate.mjs`);
  }
});

test('the backend registers exactly the signed tools and the signed incoming events', () => {
  const definition = createApp({ state: memoryState(), events: {} });
  assert.deepEqual(Object.keys(definition.tools).sort(), read('bundle/schemas/tools.json').tools.map(t => t.name).sort());
  const incoming = read('bundle/schemas/events.json').events.filter(e => e.direction === 'incoming').map(e => e.name);
  assert.deepEqual(Object.keys(definition.events).sort(), incoming.sort());
  assert.deepEqual(read('bundle/schemas/ui-actions.json').uiActions.map(a => a.name), catalog.map(t => t.name));
});

test('every GitHub call the tools make is a route the signed manifest declares, as a read', async () => {
  const manifest = read('app.json');
  const [network] = manifest.capabilities.network;
  assert.deepEqual([network.origin, network.credential, network.methods], ['https://api.github.com', 'github', ['GET']]);
  const double = fakeGitHub();
  const definition = createApp({ state: memoryState(), events: {} });
  const ctx = { fetch: double.fetch };
  const pull = { repo: REPO, number: 2 };
  const inputs = {
    list_repositories: {}, list_pull_requests: { scope: 'mine' }, get_pull_request: pull, get_pull_request_summary: pull,
    get_pull_request_files: pull, get_pull_request_diff: pull, list_commits: pull, get_checks: pull, list_reviews: pull,
    list_comments: pull, get_mergeability: pull, list_branches: { repo: REPO }, get_compare: { repo: REPO, base: 'main', head: heads[2] },
    list_accounts: {}, get_status: {},
  };
  assert.deepEqual(Object.keys(inputs).sort(), catalog.filter(t => t.effect === 'read').map(t => t.name).sort(), 'every read tool is exercised');
  for (const [name, input] of Object.entries(inputs)) await definition.tools[name](input, ctx);
  await definition.tools.list_repositories({ org: 'charioxai' }, ctx);
  const declared = network.routes.map(route => route.path.split('/'));
  for (const call of double.calls) {
    const segments = call.path.split('/');
    assert.ok(declared.some(route => route.length === segments.length && route.every((part, i) => /^\{\w+\}$/.test(part) ? segments[i] !== '' : part === segments[i])), `${call.path} is not declared`);
  }
  assert.ok(network.routes.every(route => route.safety === 'read' && route.method === 'GET'));
  // no authority beyond what a tool uses
  for (const route of declared) {
    assert.ok(double.calls.some(call => { const segments = call.path.split('/'); return route.length === segments.length && route.every((part, i) => /^\{\w+\}$/.test(part) || part === segments[i]); }), `${route.join('/')} is declared but no tool calls it`);
  }
});

test('the manifest needs the features the App uses', () => {
  const manifest = read('app.json');
  assert.deepEqual(manifest.requiredFeatures, ['wasm-component', 'ui-actions', 'directed-events', 'credentialed-http'], 'the features the App uses, in the kernel\'s order');
  assert.equal(manifest.appId, 'com.chariox.github');
  assert.equal(manifest.displayName, 'GitHub');
});
