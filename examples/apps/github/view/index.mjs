// The GitHub window. It talks only to the App's backend (window.chariox.call) and follows its change feed.
import { call, describe, forget, load } from './api.mjs';
import { newPullPage } from './create.mjs';
import { pullPage } from './detail.mjs';
import { h, clear } from './dom.mjs';
import { defaultList, listPage, describeFailure } from './list.mjs';
import { shell } from './shell.mjs';
import { blankslate } from './parts.mjs';

const REFRESH_MS = 60_000;
const FEED_MS = 5_000;

document.documentElement.setAttribute('data-color-mode', 'auto');
document.documentElement.setAttribute('data-light-theme', 'light');
document.documentElement.setAttribute('data-dark-theme', 'dark');

const root = document.getElementById('github');
const ctx = {
  accounts: null, account: null, limited: null, route: defaultList, history: [],
  // What the person is writing (comments, a review in progress), kept while the page behind it reloads.
  drafts: new Map(), deletedBranches: new Set(), mergeMethod: null,
  writing: () => writing(),
  go(route) { this.history.push(this.route); this.route = route; draw(); },
  back() { this.route = this.history.pop() ?? defaultList; draw(); },
  async useAccount(account) { await call('set_default_account', { account: account.label }); await follow(); },
  async addAccount(label) { await call('add_account', { account: label }); await follow(); },
  noteFailure(error) {
    if (error?.code === 'RATE_LIMITED') { this.limited = { resetAt: error.resetAt ?? Date.now() + 60_000 }; drawHeader(); }
  },
};

let page = null;
let frame = null;

/**
 * A page is not reloaded under someone who is in the middle of something: typing, a menu or a popover open, the merge
 * confirmation, a comment on a line being written, or text left in a comment box.
 */
const writing = () => {
  const active = document.activeElement;
  if (root.contains(active) && ['TEXTAREA', 'INPUT'].includes(active.tagName)) return true;
  return Boolean(root.querySelector('details[open], .diff-composer, [data-transient]')) || [...root.querySelectorAll('textarea')].some(area => area.value.trim());
};

function drawHeader() {
  if (!frame) return;
  const { header, banner } = shell(ctx);
  frame.header.replaceWith(header);
  frame.header = header;
  frame.banner.replaceWith(banner ?? h('div'));
  frame.banner = banner ?? frame.banner;
}

function draw() {
  page?.dispose();
  const { header, banner } = shell(ctx);
  const body = h('main.flex-auto.overflow-auto');
  frame = { header, banner: banner ?? h('div'), body };
  clear(root).append(h('div.d-flex.flex-column.height-full', {}, header, frame.banner, body));
  const unbound = ctx.accounts && ctx.account && ctx.account.status === 'unbound';
  if (!ctx.accounts) { body.append(h('div.p-4.color-fg-muted', {}, 'Loading…')); page = null; return; }
  if (!ctx.account || unbound) {
    page = null;
    body.append(h('div.p-4', {}, blankslate('mark-github', 'Connect a GitHub account', 'This App has no GitHub account yet.', 'The owner of this kernel binds one in the App\'s permissions; this window picks it up by itself.')));
    return;
  }
  page = ctx.route.name === 'pr' ? pullPage(ctx, ctx.route) : ctx.route.name === 'new' ? newPullPage(ctx, ctx.route) : listPage(ctx, ctx.route);
  body.append(page.element);
}

async function loadAccounts() {
  try {
    const { accounts } = await call('list_accounts');
    ctx.accounts = accounts;
    ctx.account = accounts.find(account => account.isDefault) ?? accounts[0] ?? null;
    const core = ctx.account?.rate?.core;
    ctx.limited = core && core.remaining < 100 && core.resetAt > Date.now() ? { resetAt: core.resetAt } : null;
  } catch (error) {
    clear(root).append(h('div.p-4', {}, describeFailure(error)));
    throw error;
  }
}

// The App's own change feed: another window or an agent changed the accounts.
let since = 0;
async function follow() {
  try {
    const feed = await call('view_changes', { since });
    const accountsChanged = feed.dropped || feed.changes.some(change => change.delta?.kind === 'accounts');
    // A GitHub event arrived for a pull request: reload the page when it shows that pull request or a list.
    const events = feed.changes.filter(change => change.delta?.kind === 'event').map(change => change.delta);
    const about = ctx.route.name === 'pr' ? events.some(event => event.repo === ctx.route.repo && (event.number === null || event.number === ctx.route.number)) : events.length > 0;
    since = feed.latest;
    if (accountsChanged && ctx.accounts) { forget(); await loadAccounts(); draw(); } else if (about && !writing()) { forget(); await page?.refresh?.(); }
  } catch { /* the next tick tries again */ }
}

await loadAccounts().catch(() => {});
if (ctx.accounts) {
  since = (await call('view_changes', { since: 0 }).catch(() => ({ latest: 0 }))).latest;
  draw();
  setInterval(follow, FEED_MS);
  setInterval(() => { if (!document.hidden && !writing()) page?.refresh?.().catch(() => {}); }, REFRESH_MS);
}
