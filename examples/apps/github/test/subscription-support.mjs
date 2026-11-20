// The App as the kernel runs it, with the kernel's event-routing, host and schedule imports replaced by in-memory
// doubles that enforce what the real ones enforce (declared events, one route per id, one automation per agent).
import assert from 'node:assert/strict';
import { createApp } from '../src/app.mjs';
import { tools as declaredTools } from '../src/catalog.mjs';
import { incoming } from '../src/event-types.mjs';
import { fakeGitHub, memoryState, REPO } from './support.mjs';

export const NOW = Date.parse('2026-11-15T12:00:00Z');
export const SHA = '271de153c2b8c196681165cc32ff2688634e1238';
export const AGENT = { kind: 'agent', id: 'agent-1' };
export const PERSON = { kind: 'human', id: 'owner' };

const refused = () => { throw { code: 'NOT_ALLOWED', message: 'not allowed' }; };
/** Whom the kernel says a routing call acts for (`Acting::of`): from the callers of the calls in progress, never from the App. */
function acting(callers) {
  const agents = new Map();
  let person = false;
  let background = false;
  for (const caller of callers) {
    if (caller.kind === 'agent') agents.set(caller.id, (agents.get(caller.id) ?? true) && caller.approved === true);
    if (caller.kind === 'human') person = true;
    if (caller.kind === 'background') background = true;
  }
  if (agents.size > 1) return refused();
  if (agents.size === 1) { const [[id, approved]] = agents; return { kind: approved ? 'approved' : 'agent', id }; }
  if (person) return { kind: 'person' };
  return background ? { kind: 'background' } : refused();
}

export function kernelRouting({ resources = [], maxRoutes = 200, outside = [] } = {}) {
  const routes = new Map();
  const targets = new Map();
  const log = [];
  const api = {
    routes, targets, log, callers: [], failUnbind: 0,
    ensureRoute: async spec => {
      assert.ok(Object.values(incoming).includes(spec.sourceEventType), `an undeclared type ${spec.sourceEventType}`);
      assert.ok(Object.hasOwn(incoming, spec.event), `an undeclared event ${spec.event}`);
      if (outside.includes(spec.repository)) throw { code: 'REPOSITORY_NOT_IN_CONNECTION', message: 'outside' };
      if (routes.has(spec.routeId)) throw { code: 'ROUTE_EXISTS', message: spec.routeId };
      routes.set(spec.routeId, { ...spec, active: true, pending: 0n, delivered: 0n, failed: 0n, expired: 0n });
      log.push(['ensure', spec.routeId]);
    },
    removeRoute: async id => { routes.delete(id); log.push(['remove', id]); },
    setRouteActive: async (id, active) => { routes.get(id).active = active; },
    listRoutes: async () => [...routes.values()],
    connectionStatus: async () => ({ generatorId: 'dev.chariox.github', connectionId: 'c', status: 'ready', maxRoutes, routes: routes.size }),
    connectionResources: async () => JSON.stringify({ page: { resources: resources.map(name => ({ id: name, name, kind: 'repository', connectionScope: name })), nextCursor: null } }),
    startAuthorization: async () => ({ authorizationId: 'a1', url: 'https://github.com/apps/x/installations/new' }),
    bindAgent: async (event, agentId) => {
      assert.equal(event, 'github_notification');
      const who = acting(api.callers);
      if (who.kind === 'background' || (who.kind === 'agent' && who.id !== agentId)) refused();
      const id = `auto-${agentId}`;
      targets.set(id, { automationId: id, event, kind: 'agent', agentId, state: 'active' });
      return targets.get(id);
    },
    bindWorkflow: async (event, sessionId, publicationRef, queueRef) => {
      if (!['person', 'approved'].includes(acting(api.callers).kind)) refused();
      const id = `auto-wf-${publicationRef}`;
      targets.set(id, { automationId: id, event, kind: 'workflow', agentId: null, state: 'active', sessionId, queueRef });
      return targets.get(id);
    },
    unbindTarget: async id => {
      const who = acting(api.callers);
      if (who.kind === 'agent' && targets.get(id)?.agentId !== who.id) refused();
      if (api.failUnbind > 0) { api.failUnbind--; throw { code: 'STORAGE_UNAVAILABLE', message: 'try again' }; }
      if (!targets.delete(id)) throw { code: 'TARGET_NOT_FOUND', message: id };
    },
    listTargets: async () => [...targets.values()],
  };
  return api;
}

/** The kernel's imports as doubles, with what they were asked to do kept for the test to look at. */
export function kernelImports({ routing = kernelRouting() } = {}) {
  const emitted = [];
  const notified = [];
  const wakes = new Map();
  const rejecting = new Set();
  return {
    routing, emitted, notified, wakes, rejecting,
    events: {
      occurrenceId: (key, at) => `${key}@${at}`,
      emit: async occurrence => {
        if (rejecting.has(occurrence.automationId)) throw { code: 'NOT_FOUND', message: 'the agent is gone' };
        emitted.push(occurrence);
        return { receiptId: 'r', state: 'queued' };
      },
    },
    host: { notify: async (title, body) => { notified.push({ title, body }); return 'n1'; } },
    schedule: {
      // The kernel's wake keeps its revision as text of at most 128 bytes and refuses a missing one.
      set: async wake => {
        if (typeof wake.revision !== 'string' || new TextEncoder().encode(wake.revision).length > 128) throw { code: 'INVALID_ARGUMENT', message: 'Invalid App storage request', retryable: false };
        wakes.set(wake.id, wake); return [...wakes.values()];
      },
      cancel: async id => { wakes.delete(id); return [...wakes.values()]; },
      list: async () => [...wakes.values()],
    },
  };
}

export function harness({ override, routing, state = memoryState(), clock = { now: NOW } } = {}) {
  const double = fakeGitHub({ override });
  const imports = kernelImports({ routing });
  const { emitted, notified, wakes, rejecting } = imports;
  const definition = createApp({ state, routing: imports.routing, now: () => clock.now, events: imports.events, host: imports.host, schedule: imports.schedule });
  const context = actor => ({ fetch: double.fetch, signal: new AbortController().signal, actor, agentId: actor?.kind === 'agent' ? actor.id : null, roomId: 'room-1' });
  /** The kernel attributes a call to its actor, and says it was approved when the person approved its effect. */
  const within = async (caller, run) => {
    imports.routing.callers = [caller];
    try { return await run(); } finally { imports.routing.callers = []; }
  };
  const call = (name, input = {}, actor = PERSON) => {
    const approved = ['approval', 'person_confirmed'].includes(declaredTools.find(tool => tool.name === name)?.effect);
    return within({ ...actor, approved }, () => definition.tools[name](input, context(actor)));
  };
  const background = id => ({ kind: 'background', id });
  const deliver = (name, payload) => within(background('inbox:route'), () => definition.events[name](payload, { ...context(background('inbox:route')), occurrenceId: `o-${Math.random()}` }));
  /** A due wake as the kernel fires it: the due time and revision the App armed. */
  const fire = armed => within(background('schedule'), () => definition.wake({ id: armed.id, dueAtMs: armed.dueAtMs ?? 0n, revision: armed.revision ?? null, overdue: false }, context(background('schedule'))));
  const wake = id => fire({ id, ...wakes.get(id) });
  return { ...double, state, routing: imports.routing, call, deliver, wake, fire, emitted, notified, wakes, rejecting, clock, definition };
}

export const payload = (name, metadata, text = 'GitHub says so') => ({
  source: { generator_id: 'dev.chariox.github', connection_id: 'c', event_type: incoming[name], event_type_version: 1 },
  occurred_at: '2026-11-15T12:00:00.000Z', text, metadata,
});
const repository = full_name => ({ full_name, default_branch: 'main' });
/** A pull request event as the generator normalizes it: GitHub's payload with `metadata.chariox` derived from it. */
export const pullRequest = ({ repo = REPO, number = 2, labels = [], branch = 'main', head = 'feature', author = 'octocat', sender = author, draft = false, merged = false, mentions = [], sha = SHA } = {}) => ({
  repository: repository(repo), sender: { login: sender },
  pull_request: { number, title: 'Add a feature', html_url: `https://github.com/${repo}/pull/${number}`, draft, merged, user: { login: author }, base: { ref: branch }, head: { ref: head, sha } },
  chariox: { labels, merged, pr_numbers: [number], body_mentions: mentions, branch, base_ref: branch, head_sha: sha, sender_login: sender },
});
export const checkRun = ({ repo = REPO, numbers = [2], conclusion = 'failure', sha = SHA, name = 'failing', sender = 'github-actions[bot]' } = {}) => ({
  repository: repository(repo), sender: { login: sender },
  check_run: { id: 1, name, conclusion, status: 'completed', head_sha: sha, html_url: `https://github.com/${repo}/runs/1`, pull_requests: numbers.map(number => ({ number })) },
  chariox: { labels: [], merged: false, pr_numbers: numbers, body_mentions: [], head_sha: sha, sender_login: sender, branch: 'feature' },
});
export const comment = ({ repo = REPO, number = 2, body = 'looks good', sender = 'reviewer', mentions = [], onPull = true } = {}) => ({
  repository: repository(repo), sender: { login: sender },
  issue: { number, title: 'Add a feature', html_url: `https://github.com/${repo}/pull/${number}`, user: { login: 'octocat' }, ...(onPull ? { pull_request: {} } : {}) },
  comment: { body },
  chariox: { labels: [], merged: false, pr_numbers: onPull ? [number] : [], body_mentions: mentions, sender_login: sender },
});

/** The recording answers `/user` with the sandbox account; checks of the head commit are `runs` instead of the recording. */
export const checksOverride = runs => url => url.pathname.endsWith('/check-runs')
  ? { status: 200, headers: {}, body: JSON.stringify({ total_count: runs.length, check_runs: runs.map(([name, status, conclusion]) => ({ name, status, conclusion })) }) }
  : undefined;
