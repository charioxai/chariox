// GitHub's pull request list: who it is for, the filters, Open and Closed counts and one row per pull request.
import { call, describe, load, loadRepositories, peek } from './api.mjs';
import { h, icon, clear } from './dom.mjs';
import { selectMenu } from './menu.mjs';
import { blankslate, checkIcon, flash, labelPill, reviewBadge, stateIcon } from './parts.mjs';
import { ago } from './time.mjs';

export const defaultList = { name: 'list', scope: 'mine', state: 'open', repos: [], org: '', page: 1, review: '', draft: 'any', label: '' };
const SCOPES = [['mine', 'Created', 'git-pull-request'], ['review_requested', 'Review requests', 'eye'], ['all', 'Involved', 'people']];
const PER_PAGE = 25;

const input = route => ({
  scope: route.scope, state: route.state, repos: route.repos, draft: route.draft, ...(route.org ? { org: route.org } : {}),
  ...(route.review ? { review: route.review } : {}), ...(route.label ? { labels: [route.label] } : {}), page: route.page, perPage: PER_PAGE,
});

/** The pull request list. `ctx.go` navigates; `ctx.fail` shows a failure of the whole window. */
export function listPage(ctx, route) {
  const body = h('div.Box');
  const root = h('div.px-3.py-3', {}, body);
  let disposed = false;
  let counts = {};

  // Another list starts from its open pull requests, as GitHub's tabs do.
  const go = patch => ctx.go({ ...route, ...(patch.scope && patch.scope !== route.scope ? { repos: [], org: '', state: 'open' } : {}), ...patch, page: patch.page ?? 1 });

  const tabs = h('nav.UnderlineNav-body.mb-3', { 'aria-label': 'Pull request lists' }, SCOPES.map(([scope, text, name]) => h('a.UnderlineNav-item', {
    href: '#', class: route.scope === scope ? 'selected' : '', 'aria-current': route.scope === scope ? 'page' : undefined,
    on: { click: event => { event.preventDefault(); go({ scope }); } },
  }, icon(name, { size: 16, className: 'UnderlineNav-octicon' }), text)));

  // Every pull request of one repository, or of one organization: the owners of the repositories the account sees.
  const known = () => peek('list_repositories', {})?.repos ?? [];
  const repoMenu = selectMenu({
    summary: h('span', {}, icon('repo', { size: 16, className: 'mr-1' }), route.scope === 'repo' && route.repos.length ? route.repos[0] : 'Repository'),
    title: 'Pull requests of a repository', align: 'SelectMenu--right', filter: 'Filter repositories', empty: 'No repositories found',
    items: query => known().filter(repo => !query || repo.fullName.toLowerCase().includes(query)).slice(0, 50)
      .map(repo => ({ key: repo.fullName, text: repo.fullName, selected: route.scope === 'repo' && route.repos[0] === repo.fullName, onSelect: () => go({ scope: 'repo', repos: [repo.fullName], org: '' }) })),
  });
  const orgMenu = selectMenu({
    summary: h('span', {}, icon('organization', { size: 16, className: 'mr-1' }), route.scope === 'org' ? route.org : 'Organization'),
    title: 'Pull requests of an organization', align: 'SelectMenu--right', empty: 'No organizations found',
    items: () => [...new Set(known().map(repo => repo.fullName.split('/')[0]))].sort()
      .map(org => ({ key: org, text: org, selected: route.scope === 'org' && route.org === org, onSelect: () => go({ scope: 'org', org, repos: [] }) })),
  });
  loadRepositories().then(() => { repoMenu.refill(); orgMenu.refill(); }, () => {});

  const create = h('a.btn.btn-primary.btn-sm.ml-2', { href: '#', on: { click: event => { event.preventDefault(); ctx.go({ name: 'new', repo: route.scope === 'repo' && route.repos.length ? route.repos[0] : null }); } } }, 'New pull request');
  const toolbar = h('div.d-flex.flex-items-center.flex-wrap.mb-3', {}, tabs, h('span.flex-1'), h('span.mr-2', {}, orgMenu.element), repoMenu.element, create);

  const stateLink = (state, text, iconName) => h('a.btn-link.mr-3', {
    href: '#', class: route.state === state ? 'text-bold color-fg-default' : 'color-fg-muted',
    on: { click: event => { event.preventDefault(); go({ state }); } },
  }, icon(iconName, { size: 16, className: 'mr-1' }), `${counts[state] === undefined ? '' : Number(counts[state]).toLocaleString('en-US') + ' '}${text}`);

  function header(labels = []) {
    const select = (value, options, on, label) => {
      const el = h('select.form-select.select-sm.ml-2', { 'aria-label': label }, options.map(([v, t]) => h('option', { value: v, selected: v === value ? true : undefined }, t)));
      el.addEventListener('change', () => on(el.value));
      return el;
    };
    return h('div.Box-header.d-flex.flex-items-center.flex-wrap', {},
      stateLink('open', 'Open', 'git-pull-request'), stateLink('closed', 'Closed', 'check'), h('span.flex-1'),
      select(route.label, [['', 'Label'], ...labels.map(name => [name, name])], label => go({ label }), 'Label'),
      select(route.review, [['', 'Review'], ['approved', 'Approved'], ['changes_requested', 'Changes requested'], ['required', 'Review required'], ['none', 'No reviews']], review => go({ review }), 'Review'),
      select(route.draft, [['any', 'Draft'], ['only', 'Drafts only'], ['hide', 'Hide drafts']], draft => go({ draft }), 'Draft'));
  }

  function row(pull) {
    const status = h('span.d-flex.flex-items-center.flex-shrink-0.ml-3.text-small.color-fg-muted', { 'data-summary': `${pull.repo}#${pull.number}` });
    const comments = pull.comments ? h('span.d-inline-flex.flex-items-center.ml-3.color-fg-muted', { title: `${pull.comments} comments` }, icon('comment', { size: 16, className: 'mr-1' }), String(pull.comments)) : null;
    const meta = [`${route.scope === 'repo' ? '' : pull.repo + ' '}#${pull.number}`, pull.state === 'merged' ? `by ${pull.author?.login ?? 'ghost'} was merged ${ago(pull.closedAt)}` : pull.state === 'closed' ? `by ${pull.author?.login ?? 'ghost'} was closed ${ago(pull.closedAt)}` : `opened ${ago(pull.createdAt)} by ${pull.author?.login ?? 'ghost'}`];
    const el = h('div.Box-row.Box-row--hover-gray.d-flex.flex-items-start.px-3.py-2', { role: 'link', tabindex: '0', 'data-pull': `${pull.repo}#${pull.number}`,
      on: { click: () => ctx.go({ name: 'pr', repo: pull.repo, number: pull.number, tab: 'conversation' }), keydown: event => { if (event.key === 'Enter') ctx.go({ name: 'pr', repo: pull.repo, number: pull.number, tab: 'conversation' }); } } },
      h('span.flex-shrink-0.mr-2.mt-1', {}, stateIcon(pull.state)),
      h('div.flex-auto.min-width-0', {},
        h('div', {}, h('a.Link--primary.text-bold.h4', { href: '#', on: { click: event => event.preventDefault() } }, pull.title), ' ', pull.labels.map(label => [labelPill(label), ' '])),
        h('div.color-fg-muted.text-small.mt-1', {}, meta.join(' '))),
      h('span.d-flex.flex-items-center.flex-shrink-0', {}, status, comments));
    return el;
  }

  function fillSummaries(pulls) {
    let next = 0;
    const worker = async () => {
      while (!disposed && next < pulls.length) {
        const pull = pulls[next++];
        const slot = root.querySelector(`[data-summary="${pull.repo}#${pull.number}"]`);
        try {
          const summary = await load('get_pull_request_summary', { repo: pull.repo, number: pull.number });
          if (disposed || !slot) continue;
          clear(slot).append(...[reviewBadge(summary.review), checkIcon(summary.checks.state, summary.checks)].filter(Boolean).flatMap((el, i) => i ? [h('span.ml-2'), el] : [el]));
          if (pull.state !== 'merged' && pull.state !== 'closed') slot.append(h('span.ml-3.color-fg-muted', {}, '+', String(summary.additions), ' −', String(summary.deletions)));
        } catch { /* a row without its extras is still a row */ }
      }
    };
    for (let i = 0; i < 2; i++) worker();
  }

  function paginate(result) {
    if (route.page === 1 && !result.hasNext) return null;
    const step = page => h('a.btn.btn-sm', { href: '#', on: { click: event => { event.preventDefault(); ctx.go({ ...route, page }); } } }, page < route.page ? 'Previous' : 'Next');
    return h('div.paginate-container.d-flex.flex-justify-center.p-3', {}, h('div.BtnGroup', {}, route.page > 1 ? step(route.page - 1) : null, result.hasNext ? step(route.page + 1) : null));
  }

  async function refresh() {
    const wanted = input(route);
    const other = route.state === 'open' ? 'closed' : 'open';
    const [result, otherCount] = await Promise.all([
      load('list_pull_requests', wanted),
      call('list_pull_requests', { ...wanted, state: other, page: 1, perPage: 1 }).then(r => r.total, () => undefined),
    ]);
    if (disposed) return;
    counts = { [route.state]: result.total, [other]: otherCount };
    const labels = [...new Set(result.pulls.flatMap(pull => pull.labels.map(label => label.name)))];
    clear(body);
    body.append(header(labels));
    if (!result.pulls.length) body.append(blankslate('git-pull-request', route.state === 'open' ? 'No open pull requests' : 'No closed pull requests', route.scope === 'review_requested' ? 'Nobody is waiting for your review.' : 'Nothing matches these filters.'));
    for (const pull of result.pulls) body.append(row(pull));
    const pages = paginate(result);
    if (pages) body.append(pages);
    if (result.stale) root.prepend(flash('warn', 'GitHub\'s rate limit is used up; this is the list as it was last loaded.'));
    fillSummaries(result.pulls);
  }

  body.append(h('div.Box-body.color-fg-muted', {}, 'Loading pull requests…'));
  root.prepend(toolbar);
  refresh().catch(error => { if (!disposed) { clear(body).append(describeFailure(error)); } ctx.noteFailure(error); });
  return { element: root, dispose: () => { disposed = true; }, refresh };
}

function describeFailure(error) {
  const failure = describe(error);
  return h('div.blankslate', {}, icon(failure.code === 'RATE_LIMITED' ? 'alert' : 'mark-github', { size: 24, className: 'blankslate-icon' }), h('h3.blankslate-heading', {}, failure.title), h('p', {}, failure.text));
}

export { describeFailure };
