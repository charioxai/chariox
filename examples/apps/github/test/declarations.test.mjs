import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import test from 'node:test';
import { outputs } from '../scripts/generate.mjs';
import { createApp } from '../src/app.mjs';
import { tools as catalog } from '../src/catalog.mjs';
import { FAILING_CHECK, fakeGitHub, heads, memoryState, REPO } from './support.mjs';
import { kernelRouting } from './subscription-support.mjs';

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

test('every GitHub call the read tools make is a declared read route, and nothing declared as a read is changed', async () => {
  const manifest = read('app.json');
  const [network] = manifest.capabilities.network;
  assert.deepEqual([network.origin, network.credential], ['https://api.github.com', 'github']);
  assert.deepEqual(network.methods, [...new Set(network.routes.map(route => route.method))].sort(), 'the methods are the methods of the routes');
  const double = fakeGitHub();
  const definition = createApp({ state: memoryState(), events: {}, routing: kernelRouting({ resources: [REPO] }) });
  const ctx = { fetch: double.fetch };
  const pull = { repo: REPO, number: 2 };
  const inputs = {
    list_repositories: {}, list_pull_requests: { scope: 'mine' }, get_pull_request: pull, get_pull_request_summary: pull,
    get_pull_request_files: pull, get_pull_request_diff: pull, list_commits: pull, get_checks: pull, get_check_output: { repo: REPO, check_run_id: FAILING_CHECK }, list_reviews: pull,
    list_comments: pull, get_mergeability: pull, list_branches: { repo: REPO }, get_compare: { repo: REPO, base: 'main', head: heads[2] },
    list_accounts: {}, get_status: {}, list_labels: { repo: REPO }, list_assignees: { repo: REPO },
    list_subscriptions: {}, get_subscription: { id: 'none' }, list_deliveries: {}, get_event_sources: {}, list_connection_repositories: {},
  };
  const readTools = catalog.filter(t => t.effect === 'read' && !t.name.startsWith('preview_')).map(t => t.name);
  assert.deepEqual(Object.keys(inputs).sort(), readTools.sort(), 'every read tool is exercised');
  for (const [name, input] of Object.entries(inputs)) await definition.tools[name](input, ctx).catch(error => assert.equal(name, 'get_subscription', `${name}: ${error.message}`));
  await definition.tools.list_repositories({ org: 'charioxai' }, ctx);
  assert.ok(double.calls.every(call => call.method === 'GET'), 'a read tool changes nothing');
  const reads = network.routes.filter(route => route.safety === 'read');
  assert.ok(reads.every(route => route.method === 'GET'));
  const matches = (route, path) => { const wanted = route.path.split('/'), got = path.split('/'); return wanted.length === got.length && wanted.every((part, i) => /^\{\w+\}$/.test(part) ? got[i] !== '' : part === got[i]); };
  for (const call of double.calls) assert.ok(reads.some(route => matches(route, call.path)), `${call.path} is not a declared read`);
});

test('the routes the write tools use are declared with the class of what they do, and no other route changes anything', () => {
  const [network] = read('app.json').capabilities.network;
  const safety = Object.fromEntries(network.routes.map(route => [`${route.method} ${route.path}`, route.safety]));
  const expected = {
    'POST /repos/{owner}/{repo}/issues/{number}/comments': 'write', 'POST /repos/{owner}/{repo}/pulls/{number}/reviews': 'write',
    'PUT /repos/{owner}/{repo}/pulls/{number}/merge': 'destructive', 'PATCH /repos/{owner}/{repo}/pulls/{number}': 'destructive',
    'DELETE /repos/{owner}/{repo}/git/refs/heads/{branch}': 'destructive', 'POST /graphql': 'write',
  };
  for (const [route, level] of Object.entries(expected)) assert.equal(safety[route], level, route);
  for (const route of network.routes) assert.equal(route.method === 'GET', route.safety === 'read', `${route.method} ${route.path} is read only when it reads`);
  assert.ok(network.routes.length <= 128, 'the kernel allows 128 routes');
});

test('every tool has a handler, and the gating of each action is declared: a card names what it shows, a standing class names its repository input', () => {
  const definition = createApp({ state: memoryState(), events: {} });
  for (const tool of catalog) assert.equal(typeof definition.tools[tool.name], 'function', tool.name);
  const actions = read('bundle/schemas/ui-actions.json').uiActions;
  const byName = Object.fromEntries(actions.map(action => [action.name, action]));
  const tier = effect => actions.filter(action => action.effect === effect).map(action => action.name).sort();
  assert.deepEqual(tier('person_confirmed'), ['close_pull_request', 'delete_branch', 'merge_pull_request']);
  assert.deepEqual(tier('approval'), ['approve_pull_request', 'convert_to_draft', 'create_subscription_for_others', 'disable_auto_merge', 'enable_auto_merge', 'mark_ready_for_review', 'open_pull_request', 'reopen_pull_request', 'request_changes', 'update_branch', 'update_subscription_for_others']);
  assert.deepEqual(tier('write').filter(name => !/account/.test(name)), ['add_labels', 'comment', 'connect_github_events', 'create_subscription', 'delete_subscription', 'pause_subscription', 'remove_labels', 'remove_reviewers', 'reply_to_review_comment', 'request_reviewers', 'rerun_failed_checks', 'resume_subscription', 'submit_review', 'update_subscription', 'watch_pull_request']);
  for (const action of actions.filter(a => a.effect === 'person_confirmed')) {
    const preview = byName[action.confirmationView];
    assert.equal(preview?.effect, 'read', `${action.name} shows ${action.confirmationView}, a read`);
    assert.deepEqual(preview.inputSchema, action.inputSchema, `${action.name}: the preview takes exactly the input of the action`);
  }
  for (const action of actions.filter(a => a.effect === 'approval')) assert.ok(action.approvalReason, `${action.name} says why it asks`);
  const standing = Object.fromEntries(actions.filter(a => a.standing).map(a => [a.name, a.standing]));
  assert.deepEqual(standing, {
    approve_pull_request: { class: 'review', repo: 'repo', account: 'account' }, request_changes: { class: 'review', repo: 'repo', account: 'account' },
    merge_pull_request: { class: 'merge', repo: 'repo', account: 'account' }, close_pull_request: { class: 'close', repo: 'repo', account: 'account' }, delete_branch: { class: 'delete_branch', repo: 'repo', account: 'account' },
  });
  for (const action of actions.filter(a => a.standing)) {
    assert.ok(action.inputSchema.required.includes(action.standing.repo), `${action.name} requires the repository input`);
    assert.equal(action.inputSchema.properties[action.standing.account].type, 'string', `${action.name} takes the account it runs as`);
  }
  assert.ok(!('force_push' in byName), 'a force push is not an action');
  const merge = byName.merge_pull_request.inputSchema;
  assert.ok(merge.required.includes('sha') && merge.required.includes('method'), 'a merge names the method and the head it was shown');
});

test('the manifest needs the features the App uses', () => {
  const manifest = read('app.json');
  assert.deepEqual(manifest.requiredFeatures, ['wasm-component', 'ui-actions', 'directed-events', 'credentialed-http', 'event-routing'], 'the features the App uses, in the kernel\'s order');
  assert.equal(manifest.appId, 'com.chariox.github');
  assert.equal(manifest.displayName, 'GitHub');
});

test('the backend sources use only what the App\'s QuickJS runtime has: no structuredClone, URL, Intl, timers beyond setTimeout, Node or browser globals', () => {
  // The runtime provides the language and the SDK's TextEncoder/TextDecoder, AbortController, Headers, setTimeout, console, crypto.getRandomValues.
  const forbidden = /\b(structuredClone|URL|URLSearchParams|Intl|setInterval|setImmediate|process|Buffer|btoa|atob|window|document|localStorage|require)\b/;
  for (const file of readdirSync(new URL('../src/', import.meta.url), { recursive: true }).filter(name => name.endsWith('.mjs'))) {
    const source = readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '').replace(/'(?:[^'\\\n]|\\.)*'|`(?:[^`\\]|\\.)*`/g, "''");
    assert.doesNotMatch(source, forbidden, `${file} uses a global the App runtime lacks`);
  }
});
