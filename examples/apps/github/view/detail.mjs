// One pull request as GitHub shows it: title and state, the Conversation, Commits, Checks and Files changed tabs.
import { act, notice } from './act.mjs';
import { forget, load } from './api.mjs';
import { composer } from './compose.mjs';
import { filesTab } from './files.mjs';
import { h, icon, clear } from './dom.mjs';
import { markdown } from './markdown-render.mjs';
import { mergeBox } from './merge.mjs';
import { avatar, blankslate, counter, stateBadge, userLink } from './parts.mjs';
import { sidebar } from './sidebar.mjs';
import { ago, duration } from './time.mjs';
import { describeFailure } from './list.mjs';

const TABS = [['conversation', 'Conversation', 'comment-discussion'], ['commits', 'Commits', 'git-commit'], ['checks', 'Checks', 'check-circle'], ['files', 'Files changed', 'file-diff']];

const REVIEW_TEXT = { APPROVED: ['approved these changes', 'check', 'color-fg-success'], CHANGES_REQUESTED: ['requested changes', 'file-diff', 'color-fg-danger'], COMMENTED: ['reviewed', 'eye', 'color-fg-muted'], DISMISSED: ['had their review dismissed', 'x', 'color-fg-muted'] };

function commentBox({ user, body, createdAt, association, edited }, verb = 'commented') {
  const roles = { OWNER: 'Owner', MEMBER: 'Member', COLLABORATOR: 'Collaborator', CONTRIBUTOR: 'Contributor', FIRST_TIME_CONTRIBUTOR: 'First-time contributor' };
  return h('div.d-flex.mb-3', {}, h('span.mr-2.flex-shrink-0', {}, avatar(user, 40)),
    h('div.Box.flex-auto.min-width-0', {},
      h('div.Box-header.d-flex.flex-items-center.py-2', {}, h('span.flex-auto', {}, userLink(user), h('span.color-fg-muted', {}, ` ${verb} ${ago(createdAt)}`)), roles[association] ? h('span.Label.Label--secondary', {}, roles[association]) : null),
      h('div.Box-body', {}, body ? markdown(body) : h('em.color-fg-muted', {}, 'No description provided.'))));
}

function lineThread(comments) {
  const first = comments[0];
  return h('div.Box.mt-2.mb-2', {},
    h('div.Box-header.py-2.text-mono.text-small', {}, first.path),
    first.diffHunk ? h('pre.diff-hunk-context.m-0.px-3.py-2.text-mono.text-small', {}, first.diffHunk.split('\n').slice(-4).join('\n')) : null,
    comments.map(comment => h('div.Box-body.border-top', {}, h('div.d-flex.flex-items-center.mb-2', {}, avatar(comment.user, 20), h('span.ml-2', {}, userLink(comment.user), h('span.color-fg-muted', {}, ` ${ago(comment.createdAt)}`))), markdown(comment.body))));
}

/** The conversation, oldest first: description, comments, reviews with the line comments of that review, commits. */
function timeline(pull, discussion, reviews, commits) {
  const description = commentBox({ user: pull.author, body: pull.body, createdAt: pull.createdAt, association: pull.authorAssociation });
  const items = [];
  for (const comment of discussion.comments) items.push({ at: comment.createdAt, node: commentBox({ user: comment.user, body: comment.body, createdAt: comment.createdAt, association: comment.authorAssociation }) });
  const threads = new Map();
  for (const comment of discussion.lineComments) threads.set(comment.reviewId, [...(threads.get(comment.reviewId) ?? []), comment]);
  for (const review of reviews.reviews) {
    const [text, iconName, color] = REVIEW_TEXT[review.state] ?? REVIEW_TEXT.COMMENTED;
    const attached = threads.get(review.id) ?? [];
    if (review.state === 'COMMENTED' && !review.body && !attached.length) continue;
    items.push({ at: review.submittedAt ?? pull.createdAt, node: h('div.TimelineItem', {}, h('div.TimelineItem-badge', {}, icon(iconName, { size: 16, className: color })),
      h('div.TimelineItem-body', {}, h('div.d-flex.flex-items-center', {}, avatar(review.user, 20), h('span.ml-2', {}, userLink(review.user), h('span.color-fg-muted', {}, ` ${text} ${ago(review.submittedAt)}`))),
        review.body ? h('div.Box.mt-2', {}, h('div.Box-body', {}, markdown(review.body))) : null,
        attached.length ? lineThread(attached) : null)) });
  }
  for (const commit of commits.commits) {
    items.push({ at: commit.authoredAt, node: h('div.TimelineItem', {}, h('div.TimelineItem-badge', {}, icon('git-commit', { size: 16 })),
      h('div.TimelineItem-body.d-flex.flex-items-center', {}, h('span.flex-auto.min-width-0.text-truncate', {}, h('span.text-bold.color-fg-default', {}, commit.message.split('\n', 1)[0])), h('code.text-small.color-fg-muted.ml-2', {}, commit.sha.slice(0, 7)))) });
  }
  items.sort((a, b) => Date.parse(a.at) - Date.parse(b.at));
  return h('div', {}, description, items.map(item => item.node));
}

/** The box under the conversation: a comment, and closing or reopening the pull request with it. */
function commentForm(ctx, { pull, input, reload }) {
  const where = notice();
  const box = composer(ctx, `comment:${input.repo}#${input.number}`);
  const comment = h('button.btn.btn-primary', { type: 'button', disabled: true }, 'Comment');
  const merged = pull.state === 'merged';
  const state = h('button.btn.mr-2', { type: 'button', hidden: merged ? true : undefined }, pull.state === 'closed' ? 'Reopen pull request' : 'Close pull request');
  const label = () => {
    const typed = box.value().length > 0;
    comment.disabled = !typed;
    state.textContent = pull.state === 'closed' ? (typed ? 'Reopen with comment' : 'Reopen pull request') : (typed ? 'Close with comment' : 'Close pull request');
  };
  box.area.addEventListener('input', label);
  label();
  const finish = () => { box.clear(); return reload(); };
  comment.addEventListener('click', () => act('comment', { ...input, body: box.value() }, { buttons: [comment, state], where, after: finish }));
  state.addEventListener('click', async () => {
    const text = box.value();
    // The comment goes first; if closing is refused the comment still stands and the reason is shown.
    if (text && !await act('comment', { ...input, body: text }, { buttons: [comment, state], where })) return;
    if (text) box.clear();
    await act(pull.state === 'closed' ? 'reopen_pull_request' : 'close_pull_request', input, { buttons: [comment, state], where, after: reload });
  });
  return h('div.d-flex.mb-3', {}, h('span.mr-2.flex-shrink-0', {}, avatar(ctx.account, 40)),
    h('div.Box.flex-auto.min-width-0', {}, h('div.Box-header.py-2', {}, h('span.text-bold', {}, 'Add a comment')),
      h('div.Box-body', {}, where.element, box.element, h('div.d-flex.flex-justify-end.mt-2', {}, state, comment))));
}

function commitsTab(commits) {
  return h('div.Box', {}, commits.commits.map(commit => h('div.Box-row.d-flex.flex-items-center', {},
    avatar(commit.author, 24), h('div.flex-auto.ml-3.min-width-0', {}, h('div.text-bold.text-truncate', {}, commit.message.split('\n', 1)[0]), h('div.color-fg-muted.text-small', {}, `${commit.author?.login ?? 'ghost'} committed ${ago(commit.authoredAt)}`)),
    h('code.color-fg-muted.text-small', {}, commit.sha.slice(0, 7)))));
}

function checkRow(run, input) {
  const [name, color] = run.status !== 'completed' ? ['dot-fill', 'color-fg-attention'] : { success: ['check-circle-fill', 'color-fg-success'], failure: ['x-circle-fill', 'color-fg-danger'], timed_out: ['x-circle-fill', 'color-fg-danger'], cancelled: ['stop', 'color-fg-muted'], skipped: ['skip', 'color-fg-muted'], neutral: ['check-circle', 'color-fg-muted'] }[run.conclusion] ?? ['alert', 'color-fg-attention'];
  const result = run.status !== 'completed' ? (run.status === 'queued' ? 'Queued' : 'In progress') : `${{ success: 'Successful', failure: 'Failing', timed_out: 'Timed out', cancelled: 'Cancelled', skipped: 'Skipped', neutral: 'Neutral' }[run.conclusion] ?? run.conclusion} ${run.startedAt ? 'in ' + duration(run.startedAt, run.completedAt) : ''}`.trim();
  const detail = h('div.mt-2.text-small.color-fg-muted', { hidden: true });
  const failed = run.status === 'completed' && run.conclusion !== 'success' && run.conclusion !== 'skipped' && run.conclusion !== 'neutral';
  const row = h('div.Box-row', { 'data-check': run.name }, h('div.d-flex.flex-items-center', {}, icon(name, { size: 16, className: `${color} mr-3` }), h('div.flex-auto', {}, h('span.text-bold', {}, run.name), run.app ? h('span.color-fg-muted.ml-2', {}, run.app) : null), h('span.color-fg-muted.text-small', {}, result),
    failed ? h('button.btn-octicon.ml-2', { type: 'button', 'aria-label': `Details of ${run.name}`, 'aria-expanded': 'false' }, icon('chevron-down', { size: 16 })) : null), detail);
  const toggle = row.querySelector('button');
  if (toggle) toggle.addEventListener('click', async () => {
    const open = detail.hidden;
    detail.hidden = !open;
    toggle.setAttribute('aria-expanded', String(open));
    if (!open || detail.dataset.loaded) return;
    detail.dataset.loaded = '1';
    clear(detail).append('Loading…');
    try {
      const output = await load('get_check_output', { repo: input.repo, check_run_id: run.id });
      clear(detail);
      if (output.title) detail.append(h('div.text-bold.color-fg-default', {}, output.title));
      for (const note of output.annotations) detail.append(h('div.d-flex.mt-1', {}, icon(note.level === 'failure' ? 'x-circle-fill' : 'alert', { size: 16, className: `${note.level === 'failure' ? 'color-fg-danger' : 'color-fg-attention'} mr-2 flex-shrink-0` }), h('span', {}, h('span.text-mono', {}, `${note.path}:${note.line ?? ''}`), ' ', note.message)));
      const result = { success: ['check-circle-fill', 'color-fg-success'], failure: ['x-circle-fill', 'color-fg-danger'], skipped: ['skip', 'color-fg-muted'], cancelled: ['stop', 'color-fg-muted'] };
      for (const step of output.steps) {
        const [mark, color] = step.status !== 'completed' ? ['dot-fill', 'color-fg-attention'] : result[step.conclusion] ?? ['alert', 'color-fg-attention'];
        detail.append(h('div.d-flex.flex-items-center.mt-1', { 'data-step': step.name }, icon(mark, { size: 16, className: `${color} mr-2 flex-shrink-0` }), h('span.flex-auto.text-mono', {}, step.name), step.startedAt && step.completedAt ? h('span.color-fg-muted.ml-2', {}, duration(step.startedAt, step.completedAt)) : null));
      }
      if (!output.title && !output.annotations.length && !output.steps.length) detail.append('This check reported no details. Its log is on GitHub.');
    } catch (error) { clear(detail).append(error.message); }
  });
  return row;
}

/** "Re-run failed jobs", as on a GitHub workflow run: only offered when something failed. */
function rerunButton(checks, input, reload, where) {
  if (!checks.failed) return null;
  const button = h('button.btn.btn-sm', { type: 'button' }, icon('sync', { size: 16, className: 'mr-1' }), 'Re-run failed jobs');
  button.addEventListener('click', () => act('rerun_failed_checks', input, { buttons: [button], where, after: reload }));
  return button;
}

function checksTab(checks, input, reload) {
  if (!checks.checks.length && !checks.statuses.length) return blankslate('check-circle', 'No checks have run', 'Checks that run on this pull request appear here.');
  const where = notice();
  return h('div', {}, where.element, h('div.Box', {}, h('div.Box-header.d-flex.flex-items-center', {}, h('span.text-bold', {}, checks.state === 'failure' ? 'Some checks were not successful' : checks.state === 'pending' ? 'Checks are running' : 'All checks have passed'), h('span.color-fg-muted.ml-2.flex-auto', {}, `${checks.failed} failing, ${checks.pending} pending, ${checks.total - checks.failed - checks.pending} successful`), rerunButton(checks, input, reload, where)),
    checks.checks.map(run => checkRow(run, input)),
    checks.statuses.map(status => h('div.Box-row.d-flex.flex-items-center', {}, icon(status.state === 'success' ? 'check-circle-fill' : status.state === 'pending' ? 'dot-fill' : 'x-circle-fill', { size: 16, className: `${status.state === 'success' ? 'color-fg-success' : status.state === 'pending' ? 'color-fg-attention' : 'color-fg-danger'} mr-3` }), h('span.text-bold.flex-auto', {}, status.context), h('span.color-fg-muted.text-small', {}, status.description ?? '')))));
}

export function pullPage(ctx, route) {
  const root = h('div.px-3.py-3');
  const content = h('div');
  let disposed = false;
  const input = { repo: route.repo, number: route.number };

  const go = tab => ctx.go({ ...route, tab });
  const back = h('a.Link--muted.d-inline-block.mb-2', { href: '#', on: { click: event => { event.preventDefault(); ctx.back(); } } }, icon('arrow-left', { size: 16, className: 'mr-1' }), 'Pull requests');

  const reload = () => { forget(); return show(); };

  /** `background`: a reload nobody asked for (the timer, the change feed): it never replaces what the person is in the middle of. */
  async function show({ background = false } = {}) {
    const { pull } = await load('get_pull_request', input);
    if (disposed) return;
    const [{ commits }, checks] = await Promise.all([load('list_commits', input), load('get_checks', input)]);
    if (disposed) return;
    const conversation = route.tab === 'commits' || route.tab === 'checks' || route.tab === 'files' ? null
      : await Promise.all([load('list_comments', input), load('list_reviews', input), load('get_mergeability', input)]);
    // everything is read before anything is replaced, so a page is never left half drawn and never swapped under a click
    if (disposed || (background && ctx.writing())) return;
    const counts = { conversation: pull.conversationComments + pull.reviewComments, commits: pull.commits, checks: checks.total, files: pull.changedFiles };
    const nav = h('nav.UnderlineNav-body.mb-3.border-bottom', { 'aria-label': 'Pull request tabs' }, TABS.map(([tab, text, name]) => h('a.UnderlineNav-item', {
      href: '#', class: route.tab === tab ? 'selected' : '', 'aria-current': route.tab === tab ? 'page' : undefined,
      on: { click: event => { event.preventDefault(); go(tab); } },
    }, icon(name, { size: 16, className: 'UnderlineNav-octicon' }), text, ' ', counter(counts[tab] ?? 0))));
    clear(content);
    if (route.tab === 'commits') content.append(commitsTab({ commits }));
    else if (route.tab === 'checks') content.append(checksTab(checks, input, reload));
    else if (route.tab === 'files') content.append(filesTab(ctx, route, pull, reload));
    else {
      const [discussion, reviews, merge] = conversation;
      content.append(h('div.d-flex.flex-wrap', {}, h('div.flex-auto.min-width-0', {}, timeline(pull, discussion, reviews, { commits }), mergeBox(ctx, { pull, checks, merge, input, reload }), commentForm(ctx, { pull, input, reload })), sidebar(ctx, { pull, reviews, input, reload })));
    }
    clear(root).append(back,
      h('h1.h1.text-normal.mb-2', {}, pull.title, h('span.color-fg-muted.ml-2', {}, `#${pull.number}`)),
      h('div.d-flex.flex-items-center.flex-wrap.mb-3', {}, stateBadge(pull.state), h('span.ml-2.color-fg-muted', {}, h('span.text-bold', {}, pull.author?.login ?? 'ghost'), pull.merged ? ` merged ${pull.commits} commit${pull.commits === 1 ? '' : 's'} into` : ` wants to merge ${pull.commits} commit${pull.commits === 1 ? '' : 's'} into`), h('span.branch-name.ml-1', {}, pull.base.ref), h('span.color-fg-muted.mx-1', {}, 'from'), h('span.branch-name', {}, pull.head.ref)),
      nav, content);
  }
  root.append(back, h('div.color-fg-muted', {}, 'Loading pull request…'));
  show().catch(error => { if (!disposed) clear(root).append(back, describeFailure(error)); ctx.noteFailure(error); });
  return { element: root, dispose: () => { disposed = true; }, refresh: () => show({ background: true }) };
}
