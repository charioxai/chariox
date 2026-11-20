// The form that creates or edits one subscription. It sends what was typed to `create_subscription` or
// `update_subscription`; the App's backend checks it and answers in words, which show above the buttons.
import { act, notice } from './act.mjs';
import { h } from './dom.mjs';
import { eventText } from './event-text.mjs';

const HOUR = 3600_000;
const split = text => [...new Set(text.split(/[\s,]+/).map(part => part.trim()).filter(Boolean))];
const join = list => (list ?? []).join(', ');

export function targetText(target) {
  if (target.kind === 'person') return 'You';
  if (target.kind === 'agent') return `Agent ${target.agentId}`;
  if (target.kind === 'room') return `Room ${target.roomId ?? ''} of agent ${target.agentId}`.replace('  ', ' ');
  return `Workflow ${target.publicationRef}`;
}

const field = (label, control, hint) => h('div.form-group.mb-3', {}, h('div.form-group-header', {}, h('label', {}, label)), h('div.form-group-body', {}, control, hint ? h('p.note', {}, hint) : null));
const input = (name, value = '', props = {}) => h('input.form-control.input-sm', { type: 'text', name, value, autocomplete: 'off', ...props });
const select = (name, options, value) => {
  const element = h('select.form-select.select-sm', { name }, options.map(([key, text]) => h('option', { value: key }, text)));
  element.value = value;
  return element;
};

/** `subscription` is null when creating. `saved` runs once the App's backend accepted it. */
export function subscriptionForm({ subscription, kinds, saved, cancel }) {
  const editing = Boolean(subscription);
  const scope = subscription?.scope ?? { kind: 'repos', repos: [] };
  const filters = subscription?.filters ?? {};
  const target = subscription?.target ?? { kind: 'person' };
  const chosen = new Set(subscription?.events ?? ['pull_request.opened']);
  const here = notice();

  const name = input('name', subscription?.name ?? '', { maxlength: 100, 'aria-label': 'Name', placeholder: 'For example: Releases of the app' });
  const scopeKind = select('scope', [['repos', 'Repositories'], ['org', 'An organization'], ['all', 'All repositories']], scope.kind);
  const repos = input('repos', join(scope.repos), { 'aria-label': 'Repositories', placeholder: 'owner/name, owner/other' });
  const org = input('org', scope.org ?? '', { 'aria-label': 'Organization', placeholder: 'organization' });
  const scopeFields = h('div', {}, field('Repositories', repos, 'Separate with commas.'), field('Organization', org));
  const showScope = () => {
    const kind = scopeKind.value;
    scopeFields.children[0].hidden = kind !== 'repos';
    scopeFields.children[1].hidden = kind !== 'org';
  };
  scopeKind.addEventListener('change', showScope);
  showScope();

  const boxes = kinds.map(kind => {
    const box = h('input', { type: 'checkbox', name: 'event', value: kind, 'data-event': kind });
    box.checked = chosen.has(kind);
    return h('label.d-block.text-normal.mb-1', {}, box, ' ', eventText(kind));
  });

  const branches = input('branches', join(filters.branches), { 'aria-label': 'Branches', placeholder: 'main, release/*' });
  const authors = input('authors', join(filters.authors), { 'aria-label': 'Authors', placeholder: 'octocat' });
  const labels = input('labels', join(filters.labels), { 'aria-label': 'Labels', placeholder: 'bug, !wip' });
  const paths = input('paths', join(filters.paths), { 'aria-label': 'Paths', placeholder: 'src/**' });
  const conclusion = input('conclusion', join(filters.conclusion), { 'aria-label': 'Conclusion', placeholder: 'failure' });
  const draft = select('draft', [['any', 'Drafts and ready'], ['only', 'Drafts only'], ['hide', 'Not drafts']], filters.draft ?? 'any');
  const self = h('input', { type: 'checkbox', name: 'excludeSelf' });
  self.checked = Boolean(filters.excludeSelf);

  const targetKind = select('target', [['person', 'You'], ['agent', 'An agent'], ['room', 'A Room'], ['workflow', 'A workflow']], target.kind);
  const agentId = input('agentId', target.agentId ?? '', { 'aria-label': 'Agent' });
  const roomId = input('roomId', target.roomId ?? '', { 'aria-label': 'Room' });
  const session = input('sessionId', target.sessionId ?? '', { 'aria-label': 'Workflow session' });
  const publication = input('publicationRef', target.publicationRef ?? '', { 'aria-label': 'Workflow publication' });
  const queue = input('queueRef', target.queueRef ?? '', { 'aria-label': 'Workflow queue' });
  const targetFields = h('div', {}, field('Agent', agentId), field('Room', roomId), field('Workflow session', session), field('Workflow publication', publication), field('Workflow queue (optional)', queue));
  const showTarget = () => {
    const kind = targetKind.value;
    const shown = { agent: [0], room: [0, 1], workflow: [2, 3, 4], person: [] }[kind];
    [...targetFields.children].forEach((element, index) => { element.hidden = !shown.includes(index); });
  };
  targetKind.addEventListener('change', showTarget);
  showTarget();

  const oncePr = input('oncePr', '', { 'aria-label': 'Pull request', placeholder: 'owner/name#12' });
  const onceHours = input('onceHours', '4', { 'aria-label': 'Hours until it ends', type: 'number', min: '1', max: '168' });

  const submit = h('button.btn.btn-primary.btn-sm.mr-2', { type: 'submit', 'data-save': '' }, editing ? 'Save' : 'Create subscription');
  const form = h('form.Box.mb-4', { 'data-transient': '', 'data-form': editing ? 'edit' : 'create', novalidate: '' },
    h('div.Box-header', {}, h('h2.Box-title', {}, editing ? 'Edit subscription' : 'New subscription')),
    h('div.Box-body', {}, here.element,
      field('Name', name), field('Where', scopeKind), scopeFields,
      h('div.form-group.mb-3', {}, h('div.form-group-header', {}, h('label', {}, 'Events')), h('div.form-group-body', {}, boxes)),
      h('details.mb-3', {}, h('summary.text-bold', { style: 'cursor: pointer' }, 'Only when'),
        h('div.pt-2', {}, field('Branches', branches, 'Names or patterns such as release/*.'), field('Authors', authors), field('Labels', labels, 'Not case sensitive; start a name with ! to leave those out.'), field('Paths', paths, 'Patterns such as src/**.'),
          field('Conclusion', conclusion, 'For checks: success, failure, cancelled…'), field('Drafts', draft),
          h('label.d-block.text-normal', {}, self, ' Not my own activity'))),
      field('Tell', targetKind), targetFields,
      editing ? null : h('details.mb-3', {}, h('summary.text-bold', { style: 'cursor: pointer' }, 'Only once, for one pull request'),
        h('div.pt-2', {}, field('Pull request', oncePr, 'Leave empty for a subscription that stays.'), field('Ends after (hours)', onceHours, 'At most 168.')))),
    h('div.Box-footer', {}, submit, h('button.btn.btn-sm', { type: 'button', 'data-cancel': '', on: { click: cancel } }, 'Cancel')));

  const pullRequest = () => {
    const text = oncePr.value.trim();
    if (!text) return null;
    const match = /^([^\s#]+\/[^\s#]+)#(\d+)$/.exec(text);
    if (!match) throw new Error('Name the pull request as owner/name#12');
    return { repo: match[1], number: Number(match[2]) };
  };

  const build = () => {
    const kind = targetKind.value;
    const tellers = {
      person: { kind },
      agent: { kind, agentId: agentId.value.trim() },
      room: { kind, agentId: agentId.value.trim(), roomId: roomId.value.trim() },
      workflow: { kind, sessionId: session.value.trim(), publicationRef: publication.value.trim(), ...(queue.value.trim() ? { queueRef: queue.value.trim() } : {}) },
    };
    const only = {
      branches: split(branches.value), authors: split(authors.value), labels: split(labels.value), paths: split(paths.value), conclusion: split(conclusion.value),
      draft: draft.value, excludeSelf: self.checked,
    };
    const body = {
      name: name.value.trim() || undefined,
      scope: scopeKind.value === 'all' ? { kind: 'all' } : scopeKind.value === 'org' ? { kind: 'org', org: org.value.trim() } : { kind: 'repos', repos: split(repos.value) },
      events: boxes.map(label => label.firstChild).filter(box => box.checked).map(box => box.value),
      filters: only, target: tellers[kind],
    };
    if (editing) return { id: subscription.id, ...body };
    const pr = pullRequest();
    return pr ? { ...body, once: { pr, expiresAt: new Date(Date.now() + Math.min(168, Math.max(1, Number(onceHours.value) || 4)) * HOUR).toISOString() } } : body;
  };

  form.addEventListener('submit', async event => {
    event.preventDefault();
    let body;
    try { body = build(); } catch (error) { here.show(error); return; }
    await act(editing ? 'update_subscription' : 'create_subscription', body, { buttons: [submit], where: here, after: saved });
  });
  return form;
}
