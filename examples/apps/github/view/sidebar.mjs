// The column beside the conversation: who reviews, who is assigned, the labels. Reviewers and labels are changed
// from the gear of their section, as on GitHub: the list shows what can be chosen and a click adds or removes it.
import { act, notice } from './act.mjs';
import { load } from './api.mjs';
import { h, icon } from './dom.mjs';
import { selectMenu } from './menu.mjs';
import { avatar, labelPill, userLink } from './parts.mjs';

const section = (title, gear, ...content) => h('div.py-3.border-bottom', {},
  h('div.d-flex.flex-items-center.mb-2', {}, h('span.text-bold.color-fg-muted.text-small.flex-auto', {}, title), gear ?? null), ...content);
const none = text => h('span.color-fg-muted', {}, text);

/** A gear that lists what can be chosen once it is opened; `entries(choices)` says what a click does. */
function picker({ title, filter, fetch, entries, where, readOnly }) {
  if (readOnly) return null;
  let choices = null;
  return selectMenu({
    summary: icon('gear', { size: 16 }), plain: true, title, align: 'SelectMenu--right', filter, empty: 'Nothing to choose',
    items: query => choices && entries(choices).filter(entry => !query || entry.text.toLowerCase().includes(query)),
    onOpen: async refill => {
      try { choices = await fetch(); } catch (error) { choices = []; where.show(error); }
      refill();
    },
  });
}

export function sidebar(ctx, { pull, reviews, input, reload }) {
  const where = notice();
  const editable = pull.state !== 'merged';
  const reviewers = new Map();
  for (const review of reviews.reviews) if (review.user && review.state !== 'COMMENTED') reviewers.set(review.user.login, { user: review.user, state: review.state });
  for (const user of reviews.requestedReviewers) if (!reviewers.has(user.login) || reviewers.get(user.login).state === 'DISMISSED') reviewers.set(user.login, { user, state: 'REQUESTED' });
  const mark = { APPROVED: ['check', 'color-fg-success'], CHANGES_REQUESTED: ['file-diff', 'color-fg-danger'], REQUESTED: ['dot-fill', 'color-fg-attention'], DISMISSED: ['x', 'color-fg-muted'] };
  const requested = new Set(reviews.requestedReviewers.map(user => user.login));
  const labelled = new Set(pull.labels.map(label => label.name));

  const reviewerGear = picker({
    title: 'Request up to 15 reviewers', filter: 'Type or choose a user', readOnly: !editable,
    fetch: async () => (await load('list_assignees', { repo: input.repo })).people.filter(person => person.login !== pull.author?.login),
    entries: people => people.map(person => ({
      key: person.login, text: person.login, selected: requested.has(person.login),
      onSelect: () => act(requested.has(person.login) ? 'remove_reviewers' : 'request_reviewers', { ...input, reviewers: [person.login] }, { where, after: reload }),
    })),
    where,
  });
  const labelGear = picker({
    title: 'Apply labels to this pull request', filter: 'Filter labels', readOnly: !editable,
    fetch: async () => (await load('list_labels', { repo: input.repo })).labels,
    entries: labels => labels.map(label => ({
      key: label.name, text: label.name, meta: label.description ?? '', selected: labelled.has(label.name),
      onSelect: () => act(labelled.has(label.name) ? 'remove_labels' : 'add_labels', { ...input, labels: [label.name] }, { where, after: reload }),
    })),
    where,
  });

  return h('aside.flex-shrink-0.pl-4.diff-sidebar', { 'aria-label': 'Pull request details' }, where.element,
    section('Reviewers', reviewerGear?.element, reviewers.size ? [...reviewers.values()].map(({ user, state }) => h('div.d-flex.flex-items-center.mb-1', {}, avatar(user, 20), h('span.ml-2.flex-auto', {}, userLink(user)), mark[state] ? icon(mark[state][0], { size: 16, className: mark[state][1] }) : null)) : none('No reviews')),
    section('Assignees', null, pull.assignees.length ? pull.assignees.map(user => h('div.d-flex.flex-items-center.mb-1', {}, avatar(user, 20), h('span.ml-2', {}, userLink(user)))) : none('No one assigned')),
    section('Labels', labelGear?.element, pull.labels.length ? h('div', {}, pull.labels.map(label => [labelPill(label), ' '])) : none('None yet')),
    section('Milestone', null, pull.milestone ? pull.milestone.title : none('No milestone')));
}
