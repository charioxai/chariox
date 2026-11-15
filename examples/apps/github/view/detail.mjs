// One pull request as GitHub shows it: title and state, the Conversation, Commits, Checks and Files changed tabs.
import { load } from './api.mjs';
import { filesTab } from './files.mjs';
import { h, icon, clear } from './dom.mjs';
import { markdown } from './markdown-render.mjs';
import { avatar, blankslate, counter, labelPill, stateBadge, userLink } from './parts.mjs';
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

function mergeBox(pull, checks, merge) {
  const row = (iconName, color, title, text) => h('div.Box-row.d-flex.flex-items-start', {}, icon(iconName, { size: 24, className: `${color} mr-3 flex-shrink-0` }), h('div', {}, h('div.text-bold', {}, title), text ? h('div.color-fg-muted', {}, text) : null));
  if (pull.state === 'merged') return h('div.Box.mb-3', {}, row('git-merge', 'color-fg-done', 'Pull request successfully merged and closed', `${pull.mergedBy?.login ?? 'Someone'} merged this ${ago(pull.mergedAt)}.`));
  if (pull.state === 'closed') return h('div.Box.mb-3', {}, row('git-pull-request-closed', 'color-fg-closed', 'Closed with unmerged commits', 'This pull request is closed.'));
  const rows = [];
  if (pull.state === 'draft') rows.push(row('git-pull-request-draft', 'color-fg-muted', 'This pull request is still a work in progress', 'Draft pull requests cannot be merged.'));
  if (checks.state === 'failure') rows.push(row('x-circle-fill', 'color-fg-danger', 'Some checks were not successful', `${checks.failed} failing and ${checks.total - checks.failed - checks.pending} successful checks`));
  else if (checks.state === 'pending') rows.push(row('dot-fill', 'color-fg-attention', 'Some checks haven\'t completed yet', `${checks.pending} pending and ${checks.total - checks.pending} completed checks`));
  else if (checks.state === 'success') rows.push(row('check-circle-fill', 'color-fg-success', 'All checks have passed', `${checks.total} successful checks`));
  if (merge.mergeable === false || merge.mergeableState === 'dirty') rows.push(row('alert-fill', 'color-fg-danger', 'This branch has conflicts that must be resolved', 'Use the web editor or the command line to resolve conflicts.'));
  else if (merge.mergeable === null) rows.push(row('sync', 'color-fg-attention', 'Checking for ability to merge automatically…', 'GitHub is working out whether this branch can be merged.'));
  else if (merge.behindBy > 0) rows.push(row('alert', 'color-fg-attention', 'This branch is out-of-date with the base branch', `Merge the latest changes from ${pull.base.ref} into this branch (${merge.behindBy} commit${merge.behindBy === 1 ? '' : 's'} behind).`));
  else rows.push(row('check-circle-fill', 'color-fg-success', 'This branch has no conflicts with the base branch', 'Merging can be performed automatically.'));
  if (pull.autoMerge) rows.push(row('git-merge', 'color-fg-done', 'Auto-merge enabled', `${pull.autoMerge.enabledBy?.login ?? 'Someone'} will merge this pull request (${{ MERGE: 'merge commit', SQUASH: 'squash', REBASE: 'rebase' }[pull.autoMerge.method] ?? pull.autoMerge.method}) when the requirements are met.`));
  const protection = merge.protection;
  if (protection.state === 'protected') {
    const needs = [protection.requiredApprovals ? `${protection.requiredApprovals} approving review${protection.requiredApprovals === 1 ? '' : 's'}` : null, protection.requiredChecks.length ? `status checks: ${protection.requiredChecks.join(', ')}` : null, protection.requireUpToDate ? 'a branch up to date with the base' : null].filter(Boolean);
    rows.push(row('shield', 'color-fg-muted', 'Branch protection', needs.length ? `Required: ${needs.join('; ')}.` : 'This branch is protected.'));
  } else if (protection.state === 'unknown') rows.push(row('shield', 'color-fg-muted', 'Branch protection', 'The rules of the base branch could not be read with this account.'));
  return h('div.Box.mb-3', {}, rows);
}

function sidebarSection(title, ...content) {
  return h('div.py-3.border-bottom', {}, h('div.text-bold.color-fg-muted.text-small.mb-2', {}, title), ...content);
}

function sidebar(pull, reviews) {
  const reviewers = new Map();
  for (const review of reviews.reviews) if (review.user && review.state !== 'COMMENTED') reviewers.set(review.user.login, { user: review.user, state: review.state });
  for (const user of reviews.requestedReviewers) if (!reviewers.has(user.login) || reviewers.get(user.login).state === 'DISMISSED') reviewers.set(user.login, { user, state: 'REQUESTED' });
  const mark = { APPROVED: ['check', 'color-fg-success'], CHANGES_REQUESTED: ['file-diff', 'color-fg-danger'], REQUESTED: ['dot-fill', 'color-fg-attention'], DISMISSED: ['x', 'color-fg-muted'] };
  const none = text => h('span.color-fg-muted', {}, text);
  return h('aside.flex-shrink-0.pl-4.diff-sidebar', { 'aria-label': 'Pull request details' },
    sidebarSection('Reviewers', reviewers.size ? [...reviewers.values()].map(({ user, state }) => h('div.d-flex.flex-items-center.mb-1', {}, avatar(user, 20), h('span.ml-2.flex-auto', {}, userLink(user)), mark[state] ? icon(mark[state][0], { size: 16, className: mark[state][1] }) : null)) : none('No reviews')),
    sidebarSection('Assignees', pull.assignees.length ? pull.assignees.map(user => h('div.d-flex.flex-items-center.mb-1', {}, avatar(user, 20), h('span.ml-2', {}, userLink(user)))) : none('No one assigned')),
    sidebarSection('Labels', pull.labels.length ? h('div', {}, pull.labels.map(label => [labelPill(label), ' '])) : none('None yet')),
    sidebarSection('Milestone', pull.milestone ? pull.milestone.title : none('No milestone')));
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
      const output = await load('get_check_output', { ...input, check_run_id: run.id });
      clear(detail);
      if (output.title) detail.append(h('div.text-bold.color-fg-default', {}, output.title));
      for (const note of output.annotations) detail.append(h('div.d-flex.mt-1', {}, icon(note.level === 'failure' ? 'x-circle-fill' : 'alert', { size: 16, className: `${note.level === 'failure' ? 'color-fg-danger' : 'color-fg-attention'} mr-2 flex-shrink-0` }), h('span', {}, h('span.text-mono', {}, `${note.path}:${note.line ?? ''}`), ' ', note.message)));
      if (!output.title && !output.annotations.length) detail.append('This check reported no details. Its log is on GitHub.');
    } catch (error) { clear(detail).append(error.message); }
  });
  return row;
}

function checksTab(checks, input) {
  if (!checks.checks.length && !checks.statuses.length) return blankslate('check-circle', 'No checks have run', 'Checks that run on this pull request appear here.');
  return h('div.Box', {}, h('div.Box-header', {}, h('span.text-bold', {}, checks.state === 'failure' ? 'Some checks were not successful' : checks.state === 'pending' ? 'Checks are running' : 'All checks have passed'), h('span.color-fg-muted.ml-2', {}, `${checks.failed} failing, ${checks.pending} pending, ${checks.total - checks.failed - checks.pending} successful`)),
    checks.checks.map(run => checkRow(run, input)),
    checks.statuses.map(status => h('div.Box-row.d-flex.flex-items-center', {}, icon(status.state === 'success' ? 'check-circle-fill' : status.state === 'pending' ? 'dot-fill' : 'x-circle-fill', { size: 16, className: `${status.state === 'success' ? 'color-fg-success' : status.state === 'pending' ? 'color-fg-attention' : 'color-fg-danger'} mr-3` }), h('span.text-bold.flex-auto', {}, status.context), h('span.color-fg-muted.text-small', {}, status.description ?? ''))));
}

export function pullPage(ctx, route) {
  const root = h('div.px-3.py-3');
  const content = h('div');
  let disposed = false;
  const input = { repo: route.repo, number: route.number };

  const go = tab => ctx.go({ ...route, tab });
  const back = h('a.Link--muted.d-inline-block.mb-2', { href: '#', on: { click: event => { event.preventDefault(); ctx.back(); } } }, icon('arrow-left', { size: 16, className: 'mr-1' }), 'Pull requests');

  async function show() {
    const { pull } = await load('get_pull_request', input);
    if (disposed) return;
    const [{ commits }, checks] = await Promise.all([load('list_commits', input), load('get_checks', input)]);
    if (disposed) return;
    const counts = { conversation: pull.conversationComments + pull.reviewComments, commits: pull.commits, checks: checks.total, files: pull.changedFiles };
    const nav = h('nav.UnderlineNav-body.mb-3.border-bottom', { 'aria-label': 'Pull request tabs' }, TABS.map(([tab, text, name]) => h('a.UnderlineNav-item', {
      href: '#', class: route.tab === tab ? 'selected' : '', 'aria-current': route.tab === tab ? 'page' : undefined,
      on: { click: event => { event.preventDefault(); go(tab); } },
    }, icon(name, { size: 16, className: 'UnderlineNav-octicon' }), text, ' ', counter(counts[tab] ?? 0))));
    clear(root).append(back,
      h('h1.h1.text-normal.mb-2', {}, pull.title, h('span.color-fg-muted.ml-2', {}, `#${pull.number}`)),
      h('div.d-flex.flex-items-center.flex-wrap.mb-3', {}, stateBadge(pull.state), h('span.ml-2.color-fg-muted', {}, h('span.text-bold', {}, pull.author?.login ?? 'ghost'), pull.merged ? ` merged ${pull.commits} commit${pull.commits === 1 ? '' : 's'} into` : ` wants to merge ${pull.commits} commit${pull.commits === 1 ? '' : 's'} into`), h('span.branch-name.ml-1', {}, pull.base.ref), h('span.color-fg-muted.mx-1', {}, 'from'), h('span.branch-name', {}, pull.head.ref)),
      nav, content);
    clear(content);
    if (route.tab === 'commits') content.append(commitsTab({ commits }));
    else if (route.tab === 'checks') content.append(checksTab(checks, input));
    else if (route.tab === 'files') content.append(filesTab(ctx, route, pull));
    else {
      const [discussion, reviews, merge] = await Promise.all([load('list_comments', input), load('list_reviews', input), load('get_mergeability', input)]);
      if (disposed) return;
      clear(content).append(h('div.d-flex.flex-wrap', {}, h('div.flex-auto.min-width-0', {}, timeline(pull, discussion, reviews, { commits }), mergeBox(pull, checks, merge)), sidebar(pull, reviews)));
    }
  }
  root.append(back, h('div.color-fg-muted', {}, 'Loading pull request…'));
  show().catch(error => { if (!disposed) clear(root).append(back, describeFailure(error)); ctx.noteFailure(error); });
  return { element: root, dispose: () => { disposed = true; }, refresh: show };
}
