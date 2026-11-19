// What a person (or an agent) may ask a subscription to be, checked once at the boundary.
import { kinds } from '../event-types.mjs';
import { LIMITS } from './store.mjs';

const fail = (code, message) => { throw { code, message, retryable: false }; };
export const HOUR = 3600_000;
export const DEFAULT_ONCE_MS = 4 * HOUR;
export const MAX_ONCE_MS = 7 * 24 * HOUR;
export const TARGET_KINDS = ['person', 'agent', 'room', 'workflow'];
export const FAILURES_BEFORE_DISABLED = 5;
const REPO = /^[A-Za-z0-9_.-]{1,100}\/[A-Za-z0-9_.-]{1,100}$/;
const OWNER = /^[A-Za-z0-9_.-]{1,100}$/;

const list = (value, what, max = 20) => {
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.length > max || value.some(item => typeof item !== 'string' || !item || item.length > 200)) {
    fail('INVALID_ARGUMENT', `${what} is a list of at most ${max} short names`);
  }
  return [...new Set(value)];
};

function scopeOf(input) {
  const scope = input ?? {};
  if (scope.kind === 'all') return { kind: 'all' };
  if (scope.kind === 'org') {
    if (!OWNER.test(scope.org ?? '')) fail('INVALID_ARGUMENT', 'An organization scope names the organization');
    return { kind: 'org', org: scope.org };
  }
  const repos = list(scope.repos, 'repos', 50);
  if (repos.length === 0 || repos.some(repo => !REPO.test(repo))) fail('INVALID_ARGUMENT', 'Name at least one repository as owner/name');
  return { kind: 'repos', repos };
}

function filtersOf(input = {}) {
  const filters = {};
  for (const key of ['branches', 'authors', 'labels', 'paths', 'conclusion']) if (input[key]?.length) filters[key] = list(input[key], key);
  if (input.draft !== undefined && input.draft !== 'any') {
    if (!['only', 'hide'].includes(input.draft)) fail('INVALID_ARGUMENT', 'draft is any, only or hide');
    filters.draft = input.draft;
  }
  if (input.excludeSelf) filters.excludeSelf = true;
  return filters;
}

function targetOf(input, context) {
  const target = input ?? { kind: 'agent' };
  if (!TARGET_KINDS.includes(target.kind)) fail('INVALID_ARGUMENT', `A target is one of ${TARGET_KINDS.join(', ')}`);
  if (target.kind === 'person') return { kind: 'person' };
  if (target.kind === 'workflow') {
    if (!target.sessionId || !target.publicationRef) fail('INVALID_ARGUMENT', 'A workflow target names its session and publication');
    return { kind: 'workflow', sessionId: target.sessionId, publicationRef: target.publicationRef, ...(target.queueRef ? { queueRef: target.queueRef } : {}) };
  }
  const agentId = target.agentId ?? context.agentId;
  if (!agentId) fail('INVALID_ARGUMENT', 'An agent target names the agent');
  return { kind: target.kind, agentId, ...(target.kind === 'room' ? { roomId: target.roomId ?? context.roomId } : {}) };
}

/** `once` is the one-shot: it ends after its first delivery or at `expiresAt`, whichever comes first. */
function onceOf(input, now) {
  if (!input) return undefined;
  const pr = input.pr ?? null;
  if (pr && (!REPO.test(pr.repo ?? '') || !Number.isSafeInteger(pr.number) || pr.number < 1)) fail('INVALID_ARGUMENT', 'once.pr names a repository and a pull request number');
  const expiresAt = input.expiresAt === undefined ? now + DEFAULT_ONCE_MS : typeof input.expiresAt === 'number' ? input.expiresAt : Date.parse(input.expiresAt);
  if (!Number.isFinite(expiresAt) || expiresAt <= now) fail('INVALID_ARGUMENT', 'once.expiresAt is a time in the future');
  if (expiresAt > now + MAX_ONCE_MS) fail('INVALID_ARGUMENT', 'A one-shot subscription ends within 7 days');
  return { pr, expiresAt };
}

export function parseSubscription(input, { now, actor, context, account }) {
  const events = list(input.events, 'events', 30);
  if (events.length === 0 || events.some(kind => !Object.hasOwn(kinds, kind))) fail('INVALID_ARGUMENT', `events are chosen from ${Object.keys(kinds).join(', ')}`);
  const once = onceOf(input.once, now);
  const scope = once?.pr ? { kind: 'repos', repos: [once.pr.repo] } : scopeOf(input.scope);
  return {
    name: input.name === undefined ? undefined : String(input.name).slice(0, 100),
    enabled: true, createdBy: { kind: actor?.kind ?? 'human', id: actor?.id ?? '' }, createdAt: now,
    account, scope, events, filters: filtersOf(input.filters), target: targetOf(input.target, context),
    ...(once ? { once } : {}), failures: 0,
  };
}

export const assertRoom = count => { if (count >= LIMITS.subscriptions) fail('LIMIT', `At most ${LIMITS.subscriptions} subscriptions: delete one first`); };
