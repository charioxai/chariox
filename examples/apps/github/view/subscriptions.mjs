// The Subscriptions tab: what GitHub tells whom, and the last deliveries of each. Everything is one tool call of the
// App's backend; the window keeps no copy of the rules.
import { act, notice } from './act.mjs';
import { call } from './api.mjs';
import { clear, h, icon } from './dom.mjs';
import { eventText } from './event-text.mjs';
import { subscriptionForm, targetText } from './subscription-form.mjs';
import { ago, clock } from './time.mjs';
import { blankslate, counter, flash } from './parts.mjs';

export const subscriptionsRoute = { name: 'subscriptions' };

const FAILURES_BEFORE_DISABLED = 5;
const ROW_DELIVERIES = 5;
const RECENT_DELIVERIES = 20;


const scopeText = scope => scope.kind === 'all' ? 'All repositories' : scope.kind === 'org' ? `Organization ${scope.org}` : scope.repos.length <= 2 ? scope.repos.join(', ') : `${scope.repos.slice(0, 2).join(', ')} and ${scope.repos.length - 2} more`;
const stateOf = subscription => subscription.enabled ? { text: 'Active', badge: 'State--open' } : subscription.failures >= FAILURES_BEFORE_DISABLED ? { text: 'Disabled after failures', badge: 'State--closed' } : { text: 'Paused', badge: 'State--draft' };
const whenText = ms => ago(new Date(ms).toISOString());
const RESULTS = { delivered: ['check', 'color-fg-success', 'Delivered'], filtered: ['dot-fill', 'color-fg-muted', 'Filtered'], failed: ['x', 'color-fg-danger', 'Failed'], expired: ['clock', 'color-fg-attention', 'Expired'] };

function deliveryRow(record) {
  const [name, color, label] = RESULTS[record.result] ?? RESULTS.filtered;
  return h('div.d-flex.flex-items-center.py-1.text-small', { 'data-delivery': record.result }, icon(name, { size: 16, className: `${color} mr-2` }),
    h('span.flex-auto.text-truncate', {}, `${eventText(record.event)} · ${record.repo}${record.subject ? ` ${record.subject}` : ''}`,
      record.reason ? h('span.color-fg-muted', {}, ` — ${record.reason}`) : null),
    h('span.color-fg-muted.ml-2', { title: label }, label), h('span.color-fg-muted.ml-3.flex-shrink-0', {}, whenText(record.at)));
}

function deliveries(records, empty) {
  return records.length ? h('div', {}, records.map(deliveryRow)) : h('div.color-fg-muted.text-small.py-1', {}, empty);
}

export function subscriptionsPage(ctx) {
  const root = h('div.px-3.py-3.container-page');
  const where = notice();
  const list = h('div');
  const editor = h('div');
  let data = null, recent = [], sources = null, loadedAt = 0, disposed = false;

  const openForm = subscription => {
    clear(editor).append(subscriptionForm({
      subscription, kinds: data.eventKinds,
      saved: async () => { clear(editor); await refresh(); },
      cancel: () => clear(editor),
    }));
    editor.firstChild?.scrollIntoView?.({ block: 'nearest' });
  };

  const row = subscription => {
    const state = stateOf(subscription);
    const own = h('div');
    const rowNotice = notice();
    let confirming = false;
    const delivered = h('details.mt-2', {}, h('summary.color-fg-muted.text-small', { style: 'cursor: pointer' }, 'Last deliveries'));
    const body = h('div.pl-3.pt-1');
    delivered.append(body);
    delivered.addEventListener('toggle', async () => {
      if (!delivered.open) return;
      clear(body).append(h('div.color-fg-muted.text-small', {}, 'Loading…'));
      try { clear(body).append(deliveries((await call('list_deliveries', { subscription: subscription.id, limit: ROW_DELIVERIES })).deliveries, 'Nothing delivered or filtered yet.')); } catch (error) { rowNotice.show(error); clear(body); }
    });
    const buttons = () => {
      const toggle = h('button.btn.btn-sm.mr-1', { type: 'button', 'data-pause': subscription.id });
      toggle.textContent = subscription.enabled ? 'Pause' : 'Resume';
      toggle.addEventListener('click', () => act(subscription.enabled ? 'pause_subscription' : 'resume_subscription', { id: subscription.id }, { buttons: [toggle], where: rowNotice, after: refresh }));
      const edit = h('button.btn.btn-sm.mr-1', { type: 'button', 'data-edit': subscription.id, on: { click: () => openForm(subscription) } }, icon('pencil', { size: 16, className: 'mr-1' }), 'Edit');
      const remove = h('button.btn.btn-sm.btn-danger', { type: 'button', 'data-delete': subscription.id }, icon('trash', { size: 16, className: 'mr-1' }), 'Delete');
      remove.addEventListener('click', async () => {
        if (!confirming) { confirming = true; remove.lastChild.textContent = 'Delete this subscription?'; return; }
        await act('delete_subscription', { id: subscription.id }, { buttons: [remove], where: rowNotice, after: refresh });
      });
      return h('div.d-flex.flex-shrink-0', {}, toggle, edit, remove);
    };
    own.append(h('div.d-flex.flex-items-start', {},
      h('div.flex-auto.min-width-0', {},
        h('div.d-flex.flex-items-center.flex-wrap', {}, h('span.text-bold.mr-2', { 'data-subscription-name': '' }, subscription.name ?? scopeText(subscription.scope)), h('span', { class: `State ${state.badge}`, 'data-state': '' }, state.text),
          subscription.once ? h('span.Label.Label--secondary.ml-2', {}, `One-shot${subscription.once.pr ? ` · ${subscription.once.pr.repo} #${subscription.once.pr.number}` : ''} · ends ${clock(subscription.once.expiresAt)} UTC`) : null),
        h('div.color-fg-muted.text-small.mt-1', {}, `${scopeText(subscription.scope)} → ${targetText(subscription.target)}`),
        h('div.mt-1', {}, subscription.events.map(kind => h('span.Label.Label--secondary.mr-1', {}, eventText(kind)))),
        subscription.createdBy.kind === 'agent' ? h('div.color-fg-muted.text-small.mt-1', {}, 'Made by an agent') : null),
      buttons()), rowNotice.element, delivered);
    return h('div.Box-row', { 'data-subscription': subscription.id }, own);
  };

  function paint() {
    clear(list);
    if (!data) return;
    const connection = sources?.connection;
    // The kernel names the generator even when no connection was granted; the connection itself says whether one is.
    if (connection && !connection.connectionId) {
      list.append(flash('warn', icon('alert', { size: 16, className: 'mr-2' }), 'Live GitHub events are not connected, so nothing is delivered yet. ', h('a', { href: '#', 'data-connect': '', on: { click: async event => { event.preventDefault(); try { const { url } = await call('connect_github_events', {}); event.target.textContent = url; event.target.title = 'Open this address to approve on GitHub'; } catch (error) { where.show(error); } } } }, 'Connect GitHub events')));
    }
    list.append(h('div.Box.mb-4', {},
      h('div.Box-header.d-flex.flex-items-center', {}, h('h2.Box-title.flex-auto', {}, 'Subscriptions ', counter(data.subscriptions.length)),
        h('button.btn.btn-primary.btn-sm', { type: 'button', 'data-new-subscription': '', on: { click: () => openForm(null) } }, icon('plus', { size: 16, className: 'mr-1' }), 'New subscription')),
      data.subscriptions.length ? data.subscriptions.map(row) : h('div', {}, blankslate('bell', 'No subscriptions', 'A subscription tells you, an agent, a Room or a workflow when something happens on GitHub.'))),
      h('div.Box', {}, h('div.Box-header', {}, h('h2.Box-title', {}, 'Recent deliveries')), h('div.Box-body', { 'data-recent': '' }, deliveries(recent, 'Nothing has arrived yet.'))));
  }

  async function refresh() {
    const [subscriptions, latest, status] = await Promise.all([call('list_subscriptions'), call('list_deliveries', { limit: RECENT_DELIVERIES }), call('get_event_sources').catch(() => null)]);
    if (disposed) return;
    data = subscriptions; recent = latest.deliveries; sources = status; loadedAt = Date.now();
    paint();
  }

  root.append(h('a.Link--muted.d-inline-block.mb-2', { href: '#', on: { click: event => { event.preventDefault(); ctx.back(); } } }, icon('arrow-left', { size: 16, className: 'mr-1' }), 'Pull requests'), where.element, editor, list);
  refresh().catch(error => { clear(list).append(h('div.color-fg-muted', {}, String(error?.message ?? error))); });
  return { element: root, refresh, dispose: () => { disposed = true; }, get loadedAt() { return loadedAt; } };
}
