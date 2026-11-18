// The frame around every page: GitHub's header row with the pull requests tab and the account menu.
import { h, icon } from './dom.mjs';
import { avatar, flash } from './parts.mjs';
import { defaultList } from './list.mjs';
import { selectMenu } from './menu.mjs';
import { clock } from './time.mjs';

const count = (n, noun) => `${Number(n).toLocaleString('en-US')} ${noun}`;

function accountEntry(account, onUse) {
  const bits = [];
  if (account.status === 'ok') {
    const left = account.rate?.core;
    if (left) bits.push(`${count(left.remaining, 'request')} left`);
  } else bits.push({ unbound: 'not bound', unauthorized: 'not accepted', rate_limited: 'rate limited' }[account.status] ?? 'unavailable');
  return { key: account.label, text: account.login ? `${account.login}` : account.label, meta: bits.join(' · '), selected: account.isDefault, onSelect: () => onUse(account) };
}

/** The account menu: who the App acts as, what is left of the rate limit, adding another binding. */
function accountMenu(ctx) {
  const label = ctx.account;
  const login = label?.login ?? label?.label ?? 'Account';
  const input = h('input.form-control.input-sm', { type: 'text', placeholder: 'Account name', maxlength: 32, 'aria-label': 'Account name', autocomplete: 'off' });
  const message = h('div.color-fg-danger.text-small.px-3.pb-2');
  const add = h('button.btn.btn-sm', { type: 'button', on: { click: async () => {
    message.textContent = '';
    try { await ctx.addAccount(input.value.trim().toLowerCase()); input.value = ''; } catch (error) { message.textContent = error.code === 'ACCOUNT_NOT_BOUND' ? 'No such account is bound in the kernel.' : String(error.message); }
  } } }, 'Add');
  const menu = selectMenu({
    summary: h('span.d-inline-flex.flex-items-center', {}, avatar(label ?? { login: '?' }, 18), h('span.ml-2.text-bold', {}, login)),
    title: 'Accounts', align: 'SelectMenu--right',
    items: () => (ctx.accounts ?? []).map(account => accountEntry(account, () => ctx.useAccount(account))),
    empty: 'No account', footer: h('div.d-flex.flex-column', {}, h('div.d-flex.px-3.pt-2.pb-2', {}, input, h('span.ml-2', {}, add)), message),
  });
  return menu.element;
}

export function shell(ctx) {
  const limited = ctx.limited;
  const header = h('header.d-flex.flex-items-center.px-3.py-2.color-bg-subtle.border-bottom', {},
    icon('mark-github', { size: 24, className: 'mr-3' }),
    h('nav.UnderlineNav-body', { 'aria-label': 'GitHub' },
      h('a.UnderlineNav-item.selected', { href: '#', on: { click: event => { event.preventDefault(); ctx.go(defaultList); } } }, icon('git-pull-request', { size: 16, className: 'UnderlineNav-octicon' }), 'Pull requests')),
    h('span.flex-1'),
    ctx.refreshing ? h('span.color-fg-muted.text-small.mr-3', {}, 'Updating…') : null,
    accountMenu(ctx));
  const banner = limited ? flash('warn', icon('alert', { size: 16, className: 'mr-2' }), `API rate limit exceeded. It resets at ${clock(limited.resetAt)} UTC.`) : null;
  return { header, banner };
}
