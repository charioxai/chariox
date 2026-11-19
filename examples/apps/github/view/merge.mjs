// The merge box as GitHub draws it: what is true of the branch, then what can be done with the pull request:
// mark it ready, bring the branch up to date, merge it with a method (one confirmation step, as on GitHub),
// turn auto-merge on or off, delete its branch after the merge, or reopen it.
import { act, notice } from './act.mjs';
import { append, h, icon } from './dom.mjs';
import { overlay } from './menu.mjs';
import { ago } from './time.mjs';

const METHODS = {
  merge: { button: 'Merge pull request', confirm: 'Confirm merge', title: 'Create a merge commit', text: () => 'All commits from this branch will be added to the base branch via a merge commit.' },
  squash: { button: 'Squash and merge', confirm: 'Confirm squash and merge', title: 'Squash and merge', text: n => `The ${n === 1 ? '1 commit' : `${n} commits`} from this branch will be combined into one commit in the base branch.` },
  rebase: { button: 'Rebase and merge', confirm: 'Confirm rebase and merge', title: 'Rebase and merge', text: n => `The ${n === 1 ? '1 commit' : `${n} commits`} from this branch will be rebased and added to the base branch.` },
};

const row = (iconName, color, title, text, ...extra) => h('div.Box-row.d-flex.flex-items-start', {},
  icon(iconName, { size: 24, className: `${color} mr-3 flex-shrink-0` }), h('div.flex-auto', {}, h('div.text-bold', {}, title), text ? h('div.color-fg-muted', {}, text) : null), ...extra);

/** GitHub's default commit title and message for a method. */
const commitText = (method, pull) => method === 'merge'
  ? { title: `Merge pull request #${pull.number} from ${pull.head.label ?? pull.head.ref}`, message: pull.title }
  : method === 'squash' ? { title: `${pull.title} (#${pull.number})`, message: '' } : null;

function statusRows(pull, checks, merge, update) {
  const rows = [];
  if (pull.state === 'draft') rows.push(row('git-pull-request-draft', 'color-fg-muted', 'This pull request is still a work in progress', 'Draft pull requests cannot be merged.'));
  if (checks.state === 'failure') rows.push(row('x-circle-fill', 'color-fg-danger', 'Some checks were not successful', `${checks.failed} failing and ${checks.total - checks.failed - checks.pending} successful checks`));
  else if (checks.state === 'pending') rows.push(row('dot-fill', 'color-fg-attention', 'Some checks haven\'t completed yet', `${checks.pending} pending and ${checks.total - checks.pending} completed checks`));
  else if (checks.state === 'success') rows.push(row('check-circle-fill', 'color-fg-success', 'All checks have passed', `${checks.total} successful checks`));
  if (merge.mergeable === false || merge.mergeableState === 'dirty') rows.push(row('alert-fill', 'color-fg-danger', 'This branch has conflicts that must be resolved', 'Use the web editor or the command line to resolve conflicts.'));
  else if (merge.mergeable === null) rows.push(row('sync', 'color-fg-attention', 'Checking for ability to merge automatically…', 'GitHub is working out whether this branch can be merged.'));
  else if (merge.behindBy > 0) rows.push(row('alert', 'color-fg-attention', 'This branch is out-of-date with the base branch', `Merge the latest changes from ${pull.base.ref} into this branch (${merge.behindBy} commit${merge.behindBy === 1 ? '' : 's'} behind).`, pull.state === 'draft' ? null : update));
  else rows.push(row('check-circle-fill', 'color-fg-success', 'This branch has no conflicts with the base branch', 'Merging can be performed automatically.'));
  if (pull.autoMerge) rows.push(row('git-merge', 'color-fg-done', 'Auto-merge enabled', `${pull.autoMerge.enabledBy?.login ?? 'Someone'} will merge this pull request (${{ MERGE: 'merge commit', SQUASH: 'squash', REBASE: 'rebase' }[pull.autoMerge.method] ?? pull.autoMerge.method}) when the requirements are met.`));
  const protection = merge.protection;
  if (protection.state === 'protected') {
    const needs = [protection.requiredApprovals ? `${protection.requiredApprovals} approving review${protection.requiredApprovals === 1 ? '' : 's'}` : null, protection.requiredChecks.length ? `status checks: ${protection.requiredChecks.join(', ')}` : null, protection.requireUpToDate ? 'a branch up to date with the base' : null].filter(Boolean);
    rows.push(row('shield', 'color-fg-muted', 'Branch protection', needs.length ? `Required: ${needs.join('; ')}.` : 'This branch is protected.'));
  } else if (protection.state === 'unknown') rows.push(row('shield', 'color-fg-muted', 'Branch protection', 'The rules of the base branch could not be read with this account.'));
  return rows;
}

/** Merge pull request / Squash and merge / Rebase and merge: the button, its method menu and the confirmation step. */
function mergeControl(ctx, { pull, merge, input, reload }, where) {
  const allowed = merge.methods.length ? merge.methods : ['merge'];
  let method = ctx.mergeMethod && allowed.includes(ctx.mergeMethod) ? ctx.mergeMethod : allowed[0];
  const holder = h('div.d-flex.flex-items-center');
  const blocked = pull.state === 'draft' || merge.mergeable === false;

  function chooser() {
    const menu = overlay(h('details.details-reset.details-overlay.BtnGroup-parent.position-relative'));
    const list = h('div.SelectMenu-list', {}, allowed.map(key => h('button.SelectMenu-item.flex-column.flex-items-start', { type: 'button', role: 'menuitemradio', 'aria-checked': String(key === method),
      on: { click: () => { method = key; ctx.mergeMethod = key; menu.removeAttribute('open'); draw(); } } },
      h('span.d-flex.flex-items-center.width-full', {}, icon('check', { size: 16, className: 'SelectMenu-icon SelectMenu-icon--check' }), h('span.text-bold', {}, METHODS[key].title)),
      h('span.color-fg-muted.text-small.pl-4', {}, METHODS[key].text(pull.commits)))));
    menu.append(h('summary.btn.btn-primary.BtnGroup-item.px-2', { 'aria-haspopup': 'menu', 'aria-label': 'Select merge method', disabled: blocked ? true : undefined }, h('span.dropdown-caret')),
      h('div.SelectMenu', {}, h('div.SelectMenu-modal', {}, h('header.SelectMenu-header', {}, h('span.SelectMenu-title', {}, 'Select merge method')), list)));
    return menu;
  }

  function draw() {
    holder.replaceChildren();
    const start = h('button.btn.btn-primary.BtnGroup-item', { type: 'button', disabled: blocked ? true : undefined, on: { click: () => confirm() } }, METHODS[method].button);
    holder.append(h('div.BtnGroup', {}, start, allowed.length > 1 ? chooser() : null));
  }

  function confirm() {
    holder.replaceChildren();
    const text = commitText(method, pull);
    const title = text && h('input.form-control.width-full.mb-2', { type: 'text', 'aria-label': 'Commit message', value: text.title });
    const body = text && h('textarea.form-control.width-full.mb-2', { rows: 3, 'aria-label': 'Extended description', placeholder: 'Add an optional extended description…' });
    if (body) body.value = text.message;
    const go = h('button.btn.btn-primary', { type: 'button' }, METHODS[method].confirm);
    const cancel = h('button.btn.ml-2', { type: 'button', on: { click: draw } }, 'Cancel');
    go.addEventListener('click', async () => {
      // The head the person was shown is what is merged; GitHub refuses it when the branch moved meanwhile.
      const done = await act('merge_pull_request', { ...input, method, sha: pull.head.sha, ...(title?.value.trim() ? { commit_title: title.value.trim() } : {}), ...(body?.value.trim() ? { commit_message: body.value.trim() } : {}) },
        { buttons: [go, cancel], where, after: reload });
      if (done === undefined) { go.disabled = false; }
    });
    holder.append(h('div.width-full', { 'data-transient': '' }, title, body, go, cancel));
  }
  draw();
  return holder;
}

function autoMergeControl(ctx, { pull, merge, input, reload }, where) {
  if (pull.autoMerge) {
    const off = h('button.btn', { type: 'button' }, 'Disable auto-merge');
    off.addEventListener('click', () => act('disable_auto_merge', input, { buttons: [off], where, after: reload }));
    return off;
  }
  if (!merge.autoMergeAllowed) return null;
  const method = ctx.mergeMethod && merge.methods.includes(ctx.mergeMethod) ? ctx.mergeMethod : merge.methods[0] ?? 'merge';
  const on = h('button.btn', { type: 'button' }, 'Enable auto-merge');
  on.addEventListener('click', () => act('enable_auto_merge', { ...input, method }, { buttons: [on], where, after: reload }));
  return on;
}

export function mergeBox(ctx, { pull, checks, merge, input, reload }) {
  const where = notice();
  const wrap = body => h('div.mb-3', {}, where.element, h('div.Box', {}, body));
  if (pull.state === 'merged') {
    const done = row('git-merge', 'color-fg-done', 'Pull request successfully merged and closed', `${pull.mergedBy?.login ?? 'Someone'} merged this ${ago(pull.mergedAt)}.`);
    // A branch of the same repository that is not its default branch can be deleted once merged, unless GitHub does it itself.
    const deletable = pull.head.repo === pull.base.repo && pull.head.ref !== pull.base.defaultBranch && !merge.deleteBranchOnMerge;
    const key = `${pull.base.repo}:${pull.head.ref}`;
    if (!deletable) return wrap(done);
    if (ctx.deletedBranches.has(key)) return wrap([done, h('div.Box-row.color-fg-muted', {}, `The ${pull.head.ref} branch was deleted.`)]);
    const del = h('button.btn', { type: 'button' }, 'Delete branch');
    del.addEventListener('click', async () => {
      if (await act('delete_branch', { repo: input.repo, branch: pull.head.ref }, { buttons: [del], where })) { ctx.deletedBranches.add(key); await reload(); }
    });
    return wrap([done, h('div.Box-row.d-flex.flex-items-center', {}, h('div.flex-auto.color-fg-muted', {}, `You're all set — the ${pull.head.ref} branch can be safely deleted.`), del)]);
  }
  if (pull.state === 'closed') return wrap(row('git-pull-request-closed', 'color-fg-closed', 'Closed with unmerged commits', 'This pull request is closed.'));

  const update = h('button.btn.btn-sm.ml-3', { type: 'button' }, 'Update branch');
  update.addEventListener('click', () => act('update_branch', { ...input, sha: pull.head.sha }, { buttons: [update], where, after: reload }));
  const rows = statusRows(pull, checks, merge, update);
  const actions = h('div.Box-row.d-flex.flex-items-center.flex-wrap');
  if (pull.state === 'draft') {
    const ready = h('button.btn.btn-primary', { type: 'button' }, 'Ready for review');
    ready.addEventListener('click', () => act('mark_ready_for_review', input, { buttons: [ready], where, after: reload }));
    actions.append(ready);
  } else {
    const auto = autoMergeControl(ctx, { pull, merge, input, reload }, where);
    append(actions, [mergeControl(ctx, { pull, merge, input, reload }, where), auto ? h('span.ml-2', {}, auto) : null]);
  }
  rows.push(actions);
  return wrap(rows);
}
