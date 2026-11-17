// Subscriptions: the one place a person or an agent says "tell me about this". The matcher decides, the targets
// deliver, the ring records, and the routes follow the subscriptions. Events reach it through the kernel's existing
// inbox routes; no other path exists.
import { finishesChecks, kinds as eventKinds } from '../event-types.mjs';
import { checkState, NO_CHECKS_MS } from './ci.mjs';
import { conclusionReason, filterReason, matchedKinds, needsPaths, normalize, pathsMatch, inScope } from './matcher.mjs';
import { FAILURES_BEFORE_DISABLED, assertRoom, parseSubscription } from './model.mjs';
import { reconcile } from './routes.mjs';
import { createStore } from './store.mjs';
import { createTargets } from './targets.mjs';

const fail = (code, message) => { throw { code, message, retryable: false }; };
const DAY = 24 * 3600_000;
const COMMENT_WAKES = 10;
const COMMENT_KINDS = new Set(['pull_request.comment', 'pull_request.review_submitted']);
/** 64-bit FNV-1a as 16 hex digits. */
function fnv1a(text) {
  let hash = 0xcbf29ce484222325n;
  for (let index = 0; index < text.length; index += 1) hash = BigInt.asUintN(64, (hash ^ BigInt(text.charCodeAt(index))) * 0x100000001b3n);
  return hash.toString(16).padStart(16, '0');
}
const wakeIds = { end: id => `end:${id}`, none: id => `none:${id}`, daily: 'reconcile' };

export function createSubscriptions({ state, commit, routing, events, host, schedule, accounts, githubFor, now = () => Date.now() }) {
  const store = createStore({ state, commit });
  const targets = createTargets({ routing, events, host });
  const idOf = () => `sub-${now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  const person = ctx => ctx.actor?.kind !== 'agent';

  const find = async id => (await store.subscriptions()).find(subscription => subscription.id === id) ?? fail('NOT_FOUND', `There is no subscription ${id}`);
  /** What an agent may change: its own subscriptions. The person at the window may change any. */
  function mayChange(subscription, ctx) {
    if (!person(ctx) && !(subscription.createdBy.kind === 'agent' && subscription.createdBy.id === ctx.actor.id)) {
      fail('NOT_YOURS', 'This subscription was made by someone else. Ask the person to change it in the GitHub window.');
    }
  }
  const selfOnly = (target, ctx, others) => {
    if (person(ctx) || others) return;
    if (target.kind !== 'agent' || target.agentId !== ctx.agentId) fail('TARGET_NEEDS_APPROVAL', 'An agent subscribes itself. To notify the person, another agent, a Room or a workflow use create_subscription_for_others, which asks the person.');
  };

  // The kernel keeps a wake's revision as text and refuses a missing one; the due time is the revision.
  const arm = (id, dueAtMs, revision = String(dueAtMs)) => schedule.set({ id, dueAtMs: BigInt(dueAtMs), revision });
  // The "no checks" wake carries the revision it was armed for: the due time and a fixed-length digest of the pull request,
  // so a long repository name never outgrows the kernel's revision limit and a re-arm for the same pull request is still a new wait.
  const noChecksRevision = (once, dueAtMs) => `${dueAtMs}:${fnv1a(`${once.pr?.repo}#${once.pr?.number}`)}`;
  const armNoChecks = (subscription, since) => arm(wakeIds.none(subscription.id), since + NO_CHECKS_MS, noChecksRevision(subscription.once, since + NO_CHECKS_MS));
  async function sync(subscriptions) {
    await arm(wakeIds.daily, now() + DAY);
    return reconcile({ routing, subscriptions });
  }
  /** A one-shot's wakes follow its settings: it ends at its expiry, and "no checks" is said 15 minutes after `since`. */
  async function armOnce(subscription, since) {
    if (!subscription.once) return;
    await arm(wakeIds.end(subscription.id), subscription.once.expiresAt);
    if (subscription.events.includes('ci.finished')) await armNoChecks(subscription, since);
    else await schedule.cancel(wakeIds.none(subscription.id)).catch(() => {});
  }

  /**
   * Let go of automations no subscription uses any more. A release the kernel refuses stays on the list and is tried
   * again by the next removal and the daily reconcile, so a subscription never ends with its automation left behind.
   */
  async function settleReleases() {
    const pending = await store.releases();
    if (pending.length === 0) return;
    const subscriptions = await store.subscriptions();
    const done = [];
    for (const automationId of pending) {
      try { await targets.release(automationId, subscriptions); done.push(automationId); } catch { /* kept for the next try */ }
    }
    if (done.length) await store.updateReleases(items => { items.splice(0, items.length, ...items.filter(id => !done.includes(id))); });
  }
  async function release(automationIds) {
    const named = automationIds.filter(Boolean);
    if (named.length) await store.updateReleases(items => { items.push(...named.filter(id => !items.includes(id))); });
    await settleReleases();
  }

  /** Remove subscriptions by id and what only they held: automation, wakes and routes. */
  async function remove(ids) {
    const gone = await store.updateSubscriptions(items => {
      const out = items.filter(item => ids.includes(item.id));
      items.splice(0, items.length, ...items.filter(item => !ids.includes(item.id)));
      return out;
    });
    for (const subscription of gone) {
      await schedule.cancel(wakeIds.end(subscription.id)).catch(() => {});
      await schedule.cancel(wakeIds.none(subscription.id)).catch(() => {});
    }
    if (gone.length) {
      await release(gone.map(subscription => subscription.automationId));
      await sync(await store.subscriptions());
    }
    return gone;
  }

  async function create(input, ctx, { others = false, extra = {} } = {}) {
    const at = now();
    const account = await accounts.resolve({ account: input.account, repo: input.scope?.repos?.[0] ?? input.once?.pr?.repo });
    const subscription = { id: idOf(), ...parseSubscription(input, { now: at, actor: ctx.actor, context: ctx, account }), ...extra };
    selfOnly(subscription.target, ctx, others);
    assertRoom((await store.subscriptions()).length);
    const automationId = await targets.bind(subscription.target);
    if (automationId) subscription.automationId = automationId;
    await store.updateSubscriptions(items => { assertRoom(items.length); items.push(subscription); });
    try {
      await armOnce(subscription, at);
      return { subscription, sources: await sync(await store.subscriptions()) };
    } catch (error) {
      await remove([subscription.id]).catch(() => {});
      throw error;
    }
  }

  async function update(input, ctx, { others = false } = {}) {
    const current = await find(input.id);
    mayChange(current, ctx);
    const merged = { ...current, ...Object.fromEntries(['name', 'scope', 'events', 'filters', 'target'].filter(key => input[key] !== undefined).map(key => [key, input[key]])), ...(input.once ? { once: { ...current.once, ...input.once } } : {}) };
    const next = { ...current, ...parseSubscription(merged, { now: now(), actor: current.createdBy, context: ctx, account: current.account }), id: current.id, createdAt: current.createdAt, createdBy: current.createdBy, enabled: current.enabled, failures: 0 };
    selfOnly(next.target, ctx, others);
    const retarget = JSON.stringify(next.target) !== JSON.stringify(current.target);
    if (retarget) next.automationId = await targets.bind(next.target) ?? undefined;
    await store.updateSubscriptions(items => { items[items.findIndex(item => item.id === next.id)] = next; });
    if (retarget) await release([current.automationId]);
    const rescheduled = JSON.stringify(next.once) !== JSON.stringify(current.once) || next.events.includes('ci.finished') !== current.events.includes('ci.finished');
    if (rescheduled) await armOnce(next, now());
    return { subscription: next, sources: await sync(await store.subscriptions()) };
  }

  async function setEnabled({ id }, ctx, enabled) {
    mayChange(await find(id), ctx);
    const subscription = await store.updateSubscriptions(items => { const item = items.find(entry => entry.id === id); item.enabled = enabled; item.failures = 0; return item; });
    // A paused one-shot's "no checks" firing is skipped, so it hears the wait again from its resume.
    if (enabled && subscription.once && subscription.events.includes('ci.finished') && !(await schedule.list()).some(armed => armed.id === wakeIds.none(id))) await armNoChecks(subscription, now());
    return { subscription, sources: await sync(await store.subscriptions()) };
  }

  // ---- delivery ----

  function describe(subscription, kind, event, extra = {}) {
    const subject = event.number ? ` #${event.number}` : '';
    const line = extra.line ?? `${kind.replaceAll('_', ' ')} in ${event.repo}${subject}${event.title ? `: ${event.title}` : ''}${event.headSha ? `. Head ${event.headSha.slice(0, 7)}` : ''}${event.url ? `. ${event.url}` : ''}`;
    return {
      at: event.at, title: `${event.repo}${subject}`, text: extra.line ? line : `${line}\n\n${event.text ?? ''}`.trim(),
      data: {
        subscription: { id: subscription.id, ...(subscription.name ? { name: subscription.name } : {}) }, event: kind, repo: event.repo,
        number: event.number, title: event.title, url: event.url, conclusion: event.conclusion, headSha: event.headSha, ...extra.data,
      },
    };
  }

  /** One line of the delivery ring about `subject`, a notification's data or a delivered event. */
  const entry = (subscription, subject, kind, result, reason) => ({
    at: now(), subscription: subscription.id, name: subscription.name, repo: subject.repo, event: kind, subject: subject.number ? `#${subject.number}` : null,
    result, ...(reason ? { reason } : {}), target: subscription.target.kind,
  });

  async function logins(cache, github, account) {
    if (!cache.has(account)) cache.set(account, github.get(account, '/user', {}).then(({ data }) => [data.login], () => []));
    return cache.get(account);
  }

  /** Every subscription this delivery concerns: its deliveries to make, and the records of what it filtered. */
  async function plan(event, ctx) {
    const github = githubFor(ctx);
    const cache = new Map();
    const files = new Map();
    const picked = [];
    const records = [];
    const record = (subscription, kind, result, reason) => records.push(entry(subscription, event, kind, result, reason));
    for (const subscription of (await store.subscriptions()).filter(item => item.enabled)) {
      if (!inScope(subscription.scope, event.repo)) continue;
      const pinned = subscription.once?.pr ?? subscription.watch?.pr;
      const accountLogins = await logins(cache, github, subscription.account);
      const kinds = matchedKinds(subscription, event, { logins: accountLogins });
      const ciWatch = subscription.events.includes('ci.finished') && finishesChecks(event.sourceType) && event.headSha && event.prNumbers.length > 0
        && (!pinned || event.prNumbers.includes(pinned.number));
      if (kinds.length === 0 && !ciWatch) continue;
      if (pinned && !event.prNumbers.includes(pinned.number) && event.number !== pinned.number) continue;
      const why = filterReason(subscription, event, { logins: accountLogins });
      if (why) { for (const kind of kinds) record(subscription, kind, 'filtered', why); continue; }
      // The conclusion filter judges the event's own check here, and the commit's whole result for `ci.finished` below.
      const own = conclusionReason(subscription, event.conclusion);
      if (own) for (const kind of kinds) record(subscription, kind, 'filtered', own);
      const wanted = own ? [] : kinds;
      if (wanted.length === 0 && !ciWatch) continue;
      if (needsPaths(subscription)) {
        if (event.number === null) { for (const kind of wanted) record(subscription, kind, 'filtered', 'no changed files known'); continue; }
        const key = `${subscription.account}:${event.number}`;
        if (!files.has(key)) {
          const [owner, repo] = event.repo.split('/');
          files.set(key, github.listAll(subscription.account, '/repos/{owner}/{repo}/pulls/{number}/files', { owner, repo, number: event.number }, { items: page => page.map(file => file.filename) }));
        }
        if (!pathsMatch(subscription, await files.get(key))) { for (const kind of wanted) record(subscription, kind, 'filtered', 'no selected path changed'); continue; }
      }
      for (const kind of wanted) picked.push({ subscription, kind, notification: describe(subscription, kind, event) });
      if (ciWatch) {
        const checks = await checkState(github, subscription.account, event.repo, event.headSha);
        if (checks.finished) {
          const ciEvent = { ...event, conclusion: checks.conclusion, number: pinned?.number ?? event.prNumbers[0] };
          const filtered = subscription.filters?.conclusion?.length && !subscription.filters.conclusion.includes(checks.conclusion);
          if (filtered) record(subscription, 'ci.finished', 'filtered', `conclusion ${checks.conclusion} is not selected`);
          else picked.push({ subscription, kind: 'ci.finished', notification: describe(subscription, 'ci.finished', ciEvent, { line: `${ciEvent.repo} #${ciEvent.number}: ${checks.line}. Head ${event.headSha.slice(0, 7)}`, data: { checks: { total: checks.total, failed: checks.failed } } }) });
        }
      }
    }
    return { picked, records };
  }

  /** Deliver, record, and end what ended: a one-shot after its first delivery, a watch at its closing event. */
  async function carryOut(picked, records) {
    const ended = new Set();
    const failures = new Map();
    const used = new Set();
    for (const { subscription, kind, notification } of picked) {
      const key = `${kind}:${notification.at}:${notification.data.number}`;
      try {
        await targets.deliver(subscription, notification, key);
        records.push(entry(subscription, notification.data, kind, 'delivered'));
        used.add(subscription.id);
        if ((subscription.once && !subscription.watch) || subscription.endsOn?.includes(kind)) ended.add(subscription.id);
      } catch (error) {
        failures.set(subscription.id, (failures.get(subscription.id) ?? 0) + 1);
        records.push(entry(subscription, notification.data, kind, 'failed', String(error?.message ?? error?.payload?.message ?? error).slice(0, 300)));
      }
    }
    const disabled = [];
    if (used.size || failures.size) await store.updateSubscriptions(items => {
      for (const item of items) {
        if (used.has(item.id)) {
          item.failures = 0;
          if (item.watch && picked.some(({ subscription, kind }) => subscription.id === item.id && COMMENT_KINDS.has(kind))) {
            item.watch.commentWakes = (item.watch.commentWakes ?? 0) + 1;
            if (item.watch.commentWakes >= COMMENT_WAKES) { item.enabled = false; disabled.push(item.id); }
          }
        }
        if (failures.has(item.id)) {
          item.failures += failures.get(item.id);
          if (item.failures >= FAILURES_BEFORE_DISABLED && item.enabled) { item.enabled = false; disabled.push(item.id); }
        }
      }
    });
    await store.addDeliveries(records);
    if (ended.size) await remove([...ended]);
    else if (disabled.length) await sync(await store.subscriptions());
  }

  async function deliver(sourceType, payload, ctx) {
    const event = { ...normalize(sourceType, payload), at: Date.parse(payload.occurred_at) };
    const { picked, records } = await plan(event, ctx);
    await carryOut(picked, records);
  }

  /** A kernel-owned wake: a one-shot's expiry, its 15-minute "no checks", or the daily route reconcile. */
  async function wake({ id, revision }, ctx) {
    if (id === wakeIds.daily) {
      const subscriptions = await store.subscriptions();
      if (subscriptions.some(item => item.enabled)) await sync(subscriptions);
      await settleReleases();
      // The kernel consumes this wake once the handler returns, so a release that is still pending needs the next one.
      if (!subscriptions.some(item => item.enabled) && (await store.releases()).length > 0) await arm(wakeIds.daily, now() + DAY);
      return;
    }
    const [kind, subscriptionId] = [id.slice(0, id.indexOf(':')), id.slice(id.indexOf(':') + 1)];
    const subscription = (await store.subscriptions()).find(item => item.id === subscriptionId);
    // A firing of a wake the subscription has since moved is not its own: an expiry it changed, a pull request it left, a CI event it dropped.
    if (!subscription || (kind === 'end' && revision !== String(subscription.once?.expiresAt))) return;
    if (kind === 'none' && !(subscription.once && subscription.events.includes('ci.finished') && (await schedule.list()).some(armed => armed.id === id && armed.revision === revision))) return;
    const subject = { repo: subscription.once?.pr?.repo ?? '', number: subscription.once?.pr?.number ?? null };
    if (!subscription.enabled) {
      // Paused: nobody is told. An expiry still ends it; the "no checks" wait starts again when it resumes.
      if (kind === 'end') {
        await store.addDeliveries([entry(subscription, subject, 'expired', 'filtered', 'paused when it ended')]);
        await remove([subscription.id]);
      }
      return;
    }
    const event = { ...subject, title: null, url: null, conclusion: null, headSha: null, text: '', at: now() };
    if (kind === 'none') {
      const { repo, number } = subject;
      if (!repo) return;
      const github = githubFor(ctx);
      const [owner, name] = repo.split('/');
      const { data } = await github.get(subscription.account, '/repos/{owner}/{repo}/pulls/{number}', { owner, repo: name, number });
      if ((await checkState(github, subscription.account, repo, data.head.sha)).total > 0) return;
      await carryOut([{ subscription, kind: 'ci.finished', notification: describe(subscription, 'ci.finished', event, { line: `${repo} #${number}: no checks reported after 15 minutes`, data: { outcome: 'no_checks' } }) }], []);
      return;
    }
    // The expiry ends it whether or not the notice arrived: an unreachable target must not keep a watch alive.
    await carryOut([{ subscription, kind: 'expired', notification: describe(subscription, 'expired', event, { line: `Your subscription ${subscription.name ?? subscription.id} ended without a result${event.repo ? ` for ${event.repo} #${event.number}` : ''}.`, data: { outcome: 'expired' } }) }], []);
    await remove([subscription.id]);
  }

  // ---- tools ----

  const watchEvents = { checks_failed: ['ci.finished'], checks_passed: ['ci.finished'], comment: ['pull_request.comment'], review: ['pull_request.review_submitted'] };
  async function watch({ repo, number, account, on = ['checks_failed', 'checks_passed', 'comment', 'review'], until = 'merged_or_closed', expiresAt }, ctx) {
    const events = [...new Set([...on.flatMap(name => watchEvents[name] ?? fail('INVALID_ARGUMENT', `Watch ${Object.keys(watchEvents).join(', ')}`)), ...(until === 'merged_or_closed' ? ['pull_request.merged', 'pull_request.closed'] : [])])];
    const ci = ['checks_failed', 'checks_passed'].filter(name => on.includes(name));
    const filters = { ...(on.includes('comment') || on.includes('review') ? { excludeSelf: true } : {}), ...(ci.length === 1 ? { conclusion: [ci[0] === 'checks_failed' ? 'failure' : 'success'] } : {}) };
    return create({ name: `Watch ${repo} #${number}`, account, scope: { kind: 'repos', repos: [repo] }, events, filters, target: { kind: 'agent' }, ...(expiresAt ? { once: { pr: { repo, number }, expiresAt } } : {}) }, ctx,
      { extra: { watch: { pr: { repo, number }, on, commentWakes: 0 }, endsOn: ['pull_request.merged', 'pull_request.closed'] } });
  }

  return {
    deliver, wake,
    tools: {
      list_subscriptions: async () => ({ subscriptions: await store.subscriptions(), eventKinds: Object.keys(eventKinds) }),
      get_subscription: async ({ id }) => ({ subscription: await find(id) }),
      create_subscription: (input, ctx) => create(input, ctx),
      create_subscription_for_others: (input, ctx) => create(input, ctx, { others: true }),
      update_subscription: (input, ctx) => update(input, ctx),
      update_subscription_for_others: (input, ctx) => update(input, ctx, { others: true }),
      pause_subscription: (input, ctx) => setEnabled(input, ctx, false),
      resume_subscription: (input, ctx) => setEnabled(input, ctx, true),
      async delete_subscription({ id }, ctx) { mayChange(await find(id), ctx); await remove([id]); return { deleted: id }; },
      watch_pull_request: watch,
      async list_deliveries({ subscription, limit = 50 }) {
        const records = (await store.deliveries()).filter(record => subscription === undefined || record.subscription === subscription);
        return { deliveries: records.slice(-limit).reverse() };
      },
      async get_event_sources() {
        const subscriptions = (await store.subscriptions()).filter(item => item.enabled);
        const status = await routing.connectionStatus().catch(error => ({ status: (error?.payload ?? error)?.code ?? 'UNAVAILABLE' }));
        const routes = status.generatorId ? await routing.listRoutes() : [];
        const repos = [...new Set(subscriptions.flatMap(item => item.scope.kind === 'repos' ? item.scope.repos : []))];
        return { connection: status, repositories: repos.map(repo => ({ repo, source: routes.some(route => route.repository === repo && route.active) ? 'live' : 'none' })), routes: routes.length };
      },
      list_connection_repositories: async ({ query, cursor, limit = 50 }) => JSON.parse(await routing.connectionResources(query ?? null, cursor ?? null, limit)),
      connect_github_events: async ({ returnUrl }) => routing.startAuthorization(returnUrl ?? null),
    },
  };
}
