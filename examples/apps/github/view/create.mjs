// "Open a pull request": GitHub's compare page. Pick the repository, the base branch and the branch to compare, read
// what the comparison holds, give it a title and a description, and create the pull request (or a draft).
import { act, notice } from './act.mjs';
import { load } from './api.mjs';
import { composer } from './compose.mjs';
import { h, icon, clear } from './dom.mjs';
import { selectMenu } from './menu.mjs';
import { blankslate } from './parts.mjs';
import { describeFailure } from './list.mjs';

/** GitHub's title for a branch nobody described: its name in words. */
const titleOf = branch => {
  const words = branch.replace(/[-_]+/g, ' ').replace(/\//g, ': ').trim();
  return words ? words[0].toUpperCase() + words.slice(1) : '';
};

/** Every branch of the repository (up to 1,000, ten pages of a hundred), so the filter finds the one that is wanted. */
async function allBranches(repo) {
  const names = [];
  for (let page = 1; page <= 10; page++) {
    const result = await load('list_branches', { repo, page, perPage: 100 }).catch(() => ({ branches: [], hasNext: false }));
    names.push(...result.branches.map(branch => branch.name));
    if (!result.hasNext) break;
  }
  return names;
}

export function newPullPage(ctx, route) {
  const root = h('div.px-3.py-3');
  let disposed = false;
  const back = h('a.Link--muted.d-inline-block.mb-2', { href: '#', on: { click: event => { event.preventDefault(); ctx.back(); } } }, icon('arrow-left', { size: 16, className: 'mr-1' }), 'Pull requests');
  const chosen = { repo: route.repo ?? ctx.drafts.get('new:repo') ?? null, base: null, head: null };
  const where = notice();
  const compare = h('div.mb-3');
  const form = h('div');
  const key = name => `new:${chosen.repo}:${chosen.head}:${name}`;
  let repos = [], branches = [], comparison = null, loadingBranches = false;

  const menu = (label, title, current, entries, onSelect, filter) => selectMenu({
    summary: h('span', {}, h('span.color-fg-muted', {}, `${label}: `), h('span.text-bold', {}, current ?? 'choose')),
    title, filter, empty: 'Nothing found',
    items: query => entries().filter(name => !query || name.toLowerCase().includes(query)).slice(0, 100).map(name => ({ key: name, text: name, selected: name === current, onSelect: () => onSelect(name) })),
  });

  async function choose(patch) {
    Object.assign(chosen, patch);
    if (patch.repo) { chosen.base = repos.find(repo => repo.fullName === chosen.repo)?.defaultBranch ?? 'main'; chosen.head = null; ctx.drafts.set('new:repo', chosen.repo); branches = []; }
    comparison = null;
    draw();
    if (patch.repo) { loadingBranches = true; draw(); branches = await allBranches(chosen.repo); loadingBranches = false; draw(); }
    if (chosen.head && chosen.head !== chosen.base) {
      try { comparison = await load('get_compare', { repo: chosen.repo, base: chosen.base, head: chosen.head }); } catch (error) { where.show(error); }
      if (!disposed) draw();
    }
  }

  function summary() {
    clear(compare);
    if (!chosen.head) return compare.append(h('div.color-fg-muted', {}, 'Choose the branch with your changes to compare it with the base branch.'));
    if (chosen.head === chosen.base) return compare.append(h('div.flash.flash-warn', {}, icon('alert', { size: 16, className: 'mr-2' }), `There isn't anything to compare. ${chosen.base} is the same branch.`));
    if (!comparison) return compare.append(h('div.color-fg-muted', {}, 'Comparing…'));
    if (comparison.aheadBy === 0) return compare.append(h('div.flash.flash-warn', {}, icon('alert', { size: 16, className: 'mr-2' }), `There isn't anything to compare. ${chosen.base} and ${chosen.head} have no differences.`));
    compare.append(h('div.flash.flash-success.mb-2', {}, icon('check', { size: 16, className: 'mr-2' }), h('span.text-bold', {}, 'Able to create a pull request. '),
      `${comparison.aheadBy} commit${comparison.aheadBy === 1 ? '' : 's'} and ${comparison.filesChanged} changed file${comparison.filesChanged === 1 ? '' : 's'}.`));
    compare.append(h('div.Box', {}, comparison.commits.map(commit => h('div.Box-row.d-flex.flex-items-center', {}, h('span.flex-auto.text-bold.text-truncate', {}, commit.message), h('code.color-fg-muted.text-small.ml-2', {}, commit.sha.slice(0, 7))))));
  }

  function draw() {
    if (disposed) return;
    const picker = h('div.d-flex.flex-items-center.flex-wrap.mb-3', {},
      h('span.mr-2', {}, menu('Repository', 'Choose a repository', chosen.repo, () => repos.map(repo => repo.fullName), name => choose({ repo: name }), 'Filter repositories').element),
      chosen.repo && loadingBranches ? h('span.color-fg-muted', {}, 'Loading branches…') : null,
      chosen.repo && !loadingBranches ? [h('span.mr-2', {}, menu('base', 'Choose a base branch', chosen.base, () => branches, name => choose({ base: name }), 'Filter branches').element),
        icon('arrow-left', { size: 16, className: 'mr-2 color-fg-muted' }),
        h('span', {}, menu('compare', 'Choose a branch to compare', chosen.head, () => branches.filter(name => name !== chosen.base), name => choose({ head: name }), 'Filter branches').element)] : null);
    summary();
    clear(form);
    const ready = chosen.repo && chosen.head && chosen.head !== chosen.base && comparison && comparison.aheadBy > 0;
    if (ready) {
      const title = h('input.form-control.width-full.mb-3', { type: 'text', 'aria-label': 'Add a title', placeholder: 'Title', maxlength: 256, autocomplete: 'off' });
      title.value = ctx.drafts.get(key('title')) ?? (comparison.totalCommits === 1 ? comparison.commits[0].message : titleOf(chosen.head));
      title.addEventListener('input', () => { ctx.drafts.set(key('title'), title.value); submit.disabled = draft.disabled = !title.value.trim(); });
      const body = composer(ctx, key('body'), { placeholder: 'Add a description', rows: 8 });
      const make = isDraft => async () => {
        const created = await act('open_pull_request', { repo: chosen.repo, title: title.value.trim(), head: chosen.head, base: chosen.base, ...(body.value() ? { body: body.value() } : {}), draft: isDraft },
          { buttons: [submit, draft], where });
        if (created) { for (const name of ['title', 'body']) ctx.drafts.delete(key(name)); ctx.go({ name: 'pr', repo: chosen.repo, number: created.pull.number, tab: 'conversation' }); }
      };
      const submit = h('button.btn.btn-primary', { type: 'button', on: { click: make(false) } }, 'Create pull request');
      const draft = h('button.btn.mr-2', { type: 'button', on: { click: make(true) } }, 'Create draft pull request');
      submit.disabled = draft.disabled = !title.value.trim();
      form.append(title, body.element, h('div.d-flex.flex-justify-end.mt-3', {}, draft, submit));
    }
    clear(root).append(back, h('h1.h2.text-normal.mb-1', {}, 'Open a pull request'),
      h('p.color-fg-muted.mb-3', {}, 'Create a new pull request by comparing changes across two branches.'), where.element, picker, compare, form);
  }

  root.append(back, h('div.color-fg-muted', {}, 'Loading repositories…'));
  load('list_repositories', {}).then(result => {
    repos = result.repos.filter(repo => !repo.archived);
    if (!repos.length) { clear(root).append(back, blankslate('repo', 'No repositories', 'This account sees no repository to open a pull request in.')); return; }
    if (chosen.repo && !repos.some(repo => repo.fullName === chosen.repo)) chosen.repo = null;
    return choose({ repo: chosen.repo ?? repos[0].fullName });
  }).catch(error => { if (!disposed) clear(root).append(back, describeFailure(error)); ctx.noteFailure(error); });
  return { element: root, dispose: () => { disposed = true; }, refresh: async () => {} };
}
