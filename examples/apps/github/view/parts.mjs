// Pieces of GitHub's pages that both the list and the pull request page use.
import { h, icon } from './dom.mjs';
import { avatarColor, labelColors } from './labels.mjs';

const STATES = {
  open: { icon: 'git-pull-request', color: 'color-fg-open', label: 'Open', badge: 'State--open' },
  draft: { icon: 'git-pull-request-draft', color: 'color-fg-muted', label: 'Draft', badge: 'State--draft' },
  merged: { icon: 'git-merge', color: 'color-fg-done', label: 'Merged', badge: 'State--merged' },
  closed: { icon: 'git-pull-request-closed', color: 'color-fg-closed', label: 'Closed', badge: 'State--closed' },
};

export const stateIcon = (state, size = 16) => icon(STATES[state].icon, { size, className: STATES[state].color, label: STATES[state].label });
export const stateBadge = state => h('span', { class: `State ${STATES[state].badge}` }, icon(STATES[state].icon, { size: 16 }), ' ', STATES[state].label);

export function labelPill(label) {
  const colors = labelColors(label.color);
  const pill = h('span.IssueLabel.hx_IssueLabel', { title: label.description ?? undefined }, label.name);
  pill.style.backgroundColor = colors.background;
  pill.style.color = colors.color;
  pill.style.borderColor = colors.border;
  return pill;
}

/** Pictures are not loaded in this page (no network), so a person is a disc with an initial. */
export function avatar(user, size = 20) {
  const login = user?.login ?? 'ghost';
  const disc = h('span.avatar.avatar-user.avatar-initial', { 'aria-hidden': 'true' }, login[0].toUpperCase());
  disc.style.backgroundColor = avatarColor(login);
  disc.style.width = disc.style.height = `${size}px`;
  disc.style.fontSize = `${Math.round(size * 0.55)}px`;
  disc.style.lineHeight = `${size}px`;
  return disc;
}

export const userLink = user => h('a.Link--primary.text-bold', { on: { click: event => event.preventDefault() } }, user?.login ?? 'ghost');

export function checkIcon(state, summary) {
  if (state === 'none' || !state) return null;
  const parts = { success: ['check', 'color-fg-success', 'All checks have passed'], failure: ['x', 'color-fg-danger', `${summary?.failed ?? 'Some'} of ${summary?.total ?? ''} checks failed`], pending: ['dot-fill', 'color-fg-attention', 'Some checks haven\'t completed yet'] }[state];
  const wrap = h('span.d-inline-block', { title: parts[2] }, icon(parts[0], { size: 16, className: parts[1], label: parts[2] }));
  return wrap;
}

export function reviewBadge(decision) {
  const parts = { approved: ['check', 'color-fg-success', 'Approved'], changes_requested: ['file-diff', 'color-fg-danger', 'Changes requested'], review_required: ['eye', 'color-fg-muted', 'Review required'] }[decision];
  return parts ? h('span.d-inline-flex.flex-items-center.color-fg-muted.text-small', {}, icon(parts[0], { size: 16, className: parts[1] }), h('span.ml-1', {}, parts[2])) : null;
}

export const counter = value => h('span.Counter', {}, String(value));

export const flash = (kind, ...children) => h(`div.flash.flash-${kind}.mb-3`, { role: kind === 'error' ? 'alert' : 'status' }, ...children);

export function blankslate(iconName, title, ...text) {
  return h('div.blankslate', {}, icon(iconName, { size: 24, className: 'blankslate-icon color-fg-muted' }), h('h3.blankslate-heading', {}, title), ...text.map(line => h('p', {}, line)));
}
