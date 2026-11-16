// "Files changed": GitHub's diff, unified or split, with the review comments under their lines.
import { act, notice } from './act.mjs';
import { call, load } from './api.mjs';
import { composer } from './compose.mjs';
import { emptyNote, parsePatch, splitRows } from './diff.mjs';
import { h, icon, clear } from './dom.mjs';
import { overlay } from './menu.mjs';
import { avatar, userLink } from './parts.mjs';
import { markdown } from './markdown-render.mjs';
import { ago } from './time.mjs';

const FIVE = 5;

/** The row of five squares beside a file's counts: green for additions, red for deletions. */
function blocks(file) {
  const total = file.additions + file.deletions;
  const greens = total ? Math.round(FIVE * file.additions / total) : 0;
  const reds = total ? Math.min(FIVE - greens, Math.round(FIVE * file.deletions / total)) : 0;
  return h('span.diffstat-blocks.ml-2', { 'aria-hidden': 'true' }, Array.from({ length: FIVE }, (_, i) => h('span.diffstat-block', { class: i < greens ? 'diffstat-add' : i < greens + reds ? 'diffstat-del' : 'diffstat-neutral' })));
}

function commentBox(comment, forget) {
  const when = comment.pending ? h('span.Label.Label--attention.ml-2', {}, 'Pending') : h('span.color-fg-muted', {}, ` commented ${ago(comment.createdAt)}`);
  return h('div.Box.m-2.diff-comment', { 'data-pending': comment.pending ? '' : undefined }, h('div.Box-header.d-flex.flex-items-center.py-2', {}, avatar(comment.user, 20),
    h('span.ml-2.flex-auto', {}, userLink(comment.user), when), comment.pending ? h('button.btn-link.color-fg-muted', { type: 'button', on: { click: forget } }, 'Delete') : null), h('div.Box-body', {}, markdown(comment.body)));
}

function codeCell(row) {
  const cell = h('td.diff-code', { class: row ? `diff-${row.kind}` : 'diff-empty' });
  if (row) cell.append(h('span.diff-marker', {}, row.kind === 'add' ? '+' : row.kind === 'del' ? '−' : ' '), h('span.diff-text', {}, row.text));
  return cell;
}

/** A line number with GitHub's blue "+" that opens a comment on that line (`add({line, side}, row)`) when the diff can be commented on. */
function numCell(row, side, add, tr) {
  const cell = h('td.diff-num', { class: row ? `diff-${row.kind}` : 'diff-empty' });
  const line = row?.[side];
  if (add && line) {
    const where = { line, side: side === 'old' ? 'LEFT' : 'RIGHT' };
    cell.append(h('button.diff-line-comment', { type: 'button', 'aria-label': `Add a comment on line ${line}`, on: { click: () => add(where, tr()) } }, icon('plus', { size: 12 })));
  }
  cell.append(String(line ?? ''));
  return cell;
}

/** Comments of the file keyed by `RIGHT:n` / `LEFT:n`. */
function commentIndex(comments) {
  const index = new Map();
  for (const comment of comments) {
    if (comment.line === null) continue;
    const key = `${comment.side === 'LEFT' ? 'LEFT' : 'RIGHT'}:${comment.line}`;
    index.set(key, [...(index.get(key) ?? []), comment]);
  }
  return index;
}

/** `add(where, tr)` makes the "+" appear; `forget(comment)` deletes a pending comment of this review. */
export function diffTable(patch, comments, mode, { add, forget } = {}) {
  const rows = parsePatch(patch);
  const index = commentIndex(comments);
  const table = h('table.diff-table', { class: mode === 'split' ? 'diff-split' : 'diff-unified' });
  const span = mode === 'split' ? 4 : 3;
  // table-layout: fixed takes its widths from these, not from whichever row comes first
  table.append(h('colgroup', {}, (mode === 'split' ? ['num', 'code', 'num', 'code'] : ['num', 'num', 'code']).map(kind => h('col', { class: `diff-col-${kind}` }))));
  const notes = (row, side) => {
    const found = [];
    if (row?.new !== null && row?.new !== undefined && (row.kind === 'add' || row.kind === 'context') && side !== 'left') found.push(...(index.get(`RIGHT:${row.new}`) ?? []));
    if (row?.old !== null && row?.old !== undefined && row.kind === 'del') found.push(...(index.get(`LEFT:${row.old}`) ?? []));
    return found;
  };
  const noteRow = found => found.length ? h('tr.diff-comments', {}, h('td', { colspan: span }, found.map(comment => commentBox(comment, () => forget(comment))))) : null;
  // a line takes a comment in one place only: the new number of an added or unchanged line, the old number of a removed one
  const onOld = row => (row?.kind === 'del' ? add : null);
  const onNew = row => (row?.kind === 'del' ? null : add);
  if (mode === 'split') {
    for (const row of splitRows(rows)) {
      if (row.kind === 'hunk') table.append(h('tr.diff-hunk-row', {}, h('td.diff-num.diff-hunk', { colspan: 1 }, icon('fold', { size: 16 })), h('td.diff-code.diff-hunk', { colspan: 3 }, row.text)));
      else if (row.kind === 'note') table.append(h('tr', {}, h('td', { colspan: 4, class: 'diff-note' }, row.text)));
      else {
        const tr = h('tr');
        tr.append(numCell(row.left, 'old', onOld(row.left), () => tr), codeCell(row.left), numCell(row.right, 'new', onNew(row.right), () => tr), codeCell(row.right));
        table.append(tr);
        const found = [...notes(row.left?.kind === 'del' ? row.left : null), ...notes(row.right?.kind === 'del' ? null : row.right)];
        const extra = noteRow(found);
        if (extra) table.append(extra);
      }
    }
  } else {
    for (const row of rows) {
      if (row.kind === 'hunk') table.append(h('tr.diff-hunk-row', {}, h('td.diff-num.diff-hunk', { colspan: 2 }, icon('fold', { size: 16 })), h('td.diff-code.diff-hunk', {}, row.text)));
      else if (row.kind === 'note') table.append(h('tr', {}, h('td', { colspan: 3, class: 'diff-note' }, row.text)));
      else {
        const tr = h('tr');
        tr.append(numCell(row, 'old', onOld(row), () => tr), numCell(row, 'new', onNew(row), () => tr), codeCell(row));
        table.append(tr);
        const extra = noteRow(notes(row));
        if (extra) table.append(extra);
      }
    }
  }
  return table;
}

/** One file of the diff: its header and its table, or why there is none. `open` shows the composer under a line. */
function fileBox(file, comments, mode, { loaded, tools }) {
  const body = h('div.diff-body');
  let collapsed = false;
  const toggle = h('button.btn-octicon', { type: 'button', 'aria-label': `Toggle ${file.path}`, 'aria-expanded': 'true' }, icon('chevron-down', { size: 16 }));
  const show = patch => clear(body).append(diffTable(patch, comments, mode, tools));
  const more = h('button.btn.btn-sm', { type: 'button' }, 'Load diff');
  more.addEventListener('click', async () => {
    more.disabled = true;
    try {
      const result = await call('get_pull_request_diff', { repo: tools.repo, number: tools.number, path: file.path });
      if (result.tooLarge || (!result.diff && result.truncated === false && result.bytes === 0)) {
        clear(body).append(h('div.p-4.text-center.color-fg-muted', {}, 'This diff is too large to display.'));
      } else {
        const at = result.diff.indexOf('\n@@');
        const patch = at < 0 ? '' : result.diff.slice(at + 1);
        loaded.set(file.path, patch);
        show(patch);
      }
    } catch (error) { clear(body).append(h('div.p-4.text-center.color-fg-danger', {}, error.message)); }
  });
  const patch = file.patch ?? loaded.get(file.path);
  if (patch) show(patch);
  else body.append(h('div.p-4.text-center.color-fg-muted', {}, emptyNote(file) ?? [h('p.mb-2', {}, 'Large diffs are not rendered by default.'), more]));
  toggle.addEventListener('click', () => { collapsed = !collapsed; body.hidden = collapsed; toggle.setAttribute('aria-expanded', String(!collapsed)); clear(toggle).append(icon(collapsed ? 'chevron-right' : 'chevron-down', { size: 16 })); });
  const name = file.previousPath ? `${file.previousPath} → ${file.path}` : file.path;
  return h('div.Box.mb-3.diff-file', { 'data-file': file.path },
    h('div.Box-header.d-flex.flex-items-center.py-2', {}, toggle,
      h('span.ml-2.text-mono.text-small.flex-auto.min-width-0.text-bold', {}, name),
      h('span.color-fg-muted.text-small', {}, `${file.changes} changes`), blocks(file)),
    body);
}

const VERDICTS = [
  ['comment', 'Comment', 'Submit general feedback without explicit approval.'],
  ['approve', 'Approve', 'Submit feedback and approve merging these changes.'],
  ['request_changes', 'Request changes', 'Submit feedback that must be addressed before merging.'],
];

/** "Review changes": GitHub's popover that finishes a review with a verdict and the comments collected on the lines. */
function reviewMenu(ctx, route, pull, { pendingOf, headOf, forgetAll }, { reload }) {
  const where = notice();
  const box = composer(ctx, `review:${route.repo}#${route.number}`, { placeholder: 'Leave a comment', rows: 4 });
  const own = pull.author?.login && ctx.account?.login && pull.author.login.toLowerCase() === ctx.account.login.toLowerCase();
  let verdict = 'comment';
  const submit = h('button.btn.btn-primary', { type: 'button' }, 'Submit review');
  const radios = VERDICTS.map(([value, title, text]) => {
    const blocked = own && value !== 'comment';
    const input = h('input', { type: 'radio', name: 'verdict', value, checked: value === verdict ? true : undefined, disabled: blocked ? true : undefined });
    input.addEventListener('change', () => { verdict = value; check(); });
    return h('label.d-block.mb-2', {}, input, h('span.text-bold.ml-2', {}, title), h('div.color-fg-muted.text-small.ml-4', {},
      blocked ? `Pull request authors can't ${value === 'approve' ? 'approve their own pull request' : 'request changes on their own pull request'}.` : text));
  });
  const count = h('span.Counter.ml-1');
  const summary = h('summary.btn.btn-sm.btn-primary', { 'aria-haspopup': 'dialog' }, 'Review changes', count, h('span.dropdown-caret.ml-1'));
  function check() {
    const pending = pendingOf().length;
    count.textContent = pending ? String(pending) : '';
    count.hidden = !pending;
    submit.disabled = verdict === 'request_changes' ? !box.value() : verdict === 'comment' ? !box.value() && !pending : false;
  }
  box.area.addEventListener('input', check);
  submit.addEventListener('click', async () => {
    const comments = pendingOf().map(({ path, line, side, body }) => ({ path, line, side, body }));
    const input = { repo: route.repo, number: route.number, ...(box.value() ? { body: box.value() } : {}), ...(comments.length ? { comments } : {}) };
    const name = { comment: 'submit_review', approve: 'approve_pull_request', request_changes: 'request_changes' }[verdict];
    // The head the comments were written against is the one reviewed; GitHub would otherwise attach them to a newer commit.
    const done = await act(name, { ...input, sha: headOf() }, { buttons: [submit], where, after: () => { box.clear(); forgetAll(); return reload(); } });
    if (done) details.removeAttribute('open');
  });
  const details = overlay(h('details.details-reset.details-overlay.d-inline-block.position-relative', {}, summary,
    h('div.Popover.mt-2.right-0', {}, h('div.Popover-message.Popover-message--top-right.p-3.text-left.Box.color-shadow-large', {},
      h('h4.mb-2', {}, 'Finish your review'), where.element, box.element, h('div.mt-3', {}, radios), h('div.d-flex.flex-justify-end', {}, submit)))));
  details.addEventListener('toggle', check);
  check();
  return { element: details, check };
}

const pendingKey = route => `pending:${route.repo}#${route.number}`;
const pendingHeadKey = route => `pending-head:${route.repo}#${route.number}`;

export function filesTab(ctx, route, pull, reload) {
  let mode = 'unified';
  const root = h('div');
  const list = h('div');
  const loaded = new Map();
  const boxes = new Map();
  let files = [], comments = [], more = null;
  const pendingOf = () => ctx.drafts.get(pendingKey(route)) ?? [];
  const where = notice();
  // the pending comments of this review sit under their lines beside the ones already on GitHub
  const commentsOf = path => [...comments, ...pendingOf().map(comment => ({ ...comment, pending: true, user: ctx.account }))].filter(comment => comment.path === path);
  // the head the first pending comment was written against: a page that reloads onto a newer head must not carry them over
  const headOf = () => (pendingOf().length && ctx.drafts.get(pendingHeadKey(route))) || pull.head.sha;
  const forgetAll = () => { ctx.drafts.delete(pendingKey(route)); ctx.drafts.delete(pendingHeadKey(route)); };
  const review = reviewMenu(ctx, route, pull, { pendingOf, headOf, forgetAll }, { reload });

  function redraw(path) {
    const file = files.find(candidate => candidate.path === path);
    const old = boxes.get(path);
    if (!file || !old) return;
    const next = buildBox(file);
    old.replaceWith(next);
    review.check();
  }

  const forget = comment => {
    ctx.drafts.set(pendingKey(route), pendingOf().filter(pending => pending !== comment && !(pending.path === comment.path && pending.line === comment.line && pending.side === comment.side && pending.body === comment.body)));
    if (!pendingOf().length) forgetAll();
    redraw(comment.path);
  };

  /** The composer of a line: what is typed here is a comment of the review, or one comment on its own. */
  function composeAt(file) {
    return ({ line, side }, tr) => {
      const next = tr.nextElementSibling;
      if (next?.classList.contains('diff-composer')) return next.querySelector('textarea').focus();
      const box = composer(ctx, `line:${route.repo}#${route.number}:${file.path}:${side}:${line}`, { placeholder: 'Leave a comment', rows: 3 });
      const fail = notice();
      const cancel = h('button.btn.btn-sm', { type: 'button' }, 'Cancel');
      const single = h('button.btn.btn-sm.mr-2', { type: 'button' }, 'Add single comment');
      const start = h('button.btn.btn-sm.btn-primary', { type: 'button' }, pendingOf().length ? 'Add review comment' : 'Start a review');
      const close = () => { ctx.drafts.delete(`line:${route.repo}#${route.number}:${file.path}:${side}:${line}`); row.remove(); };
      cancel.addEventListener('click', close);
      start.addEventListener('click', () => {
        if (!box.value()) return;
        ctx.drafts.set(pendingHeadKey(route), headOf());
        ctx.drafts.set(pendingKey(route), [...pendingOf(), { path: file.path, line, side, body: box.value() }]);
        close();
        redraw(file.path);
      });
      single.addEventListener('click', async () => {
        if (!box.value()) return;
        await act('submit_review', { repo: route.repo, number: route.number, sha: pull.head.sha, comments: [{ path: file.path, line, side, body: box.value() }] }, { buttons: [single, start, cancel], where: fail, after: () => { close(); return reload(); } });
      });
      const row = h('tr.diff-composer', {}, h('td', { colspan: tr.cells.length }, h('div.Box.m-2.p-3', {}, fail.element, box.element, h('div.d-flex.flex-justify-end.mt-2', {}, cancel, h('span.ml-2', {}, single), start))));
      tr.after(row);
      box.area.focus();
    };
  }

  function buildBox(file) {
    const element = fileBox(file, commentsOf(file.path), mode, { loaded, tools: { add: composeAt(file), forget, repo: route.repo, number: route.number } });
    boxes.set(file.path, element);
    return element;
  }

  const drawFiles = () => {
    clear(list);
    for (const file of files) list.append(buildBox(file));
    if (more) list.append(h('div.text-center.mb-3', {}, h('button.btn', { type: 'button', on: { click: () => loadMore() } }, 'Load more files')));
  };
  const modeButton = (value, text) => h('button.btn.btn-sm.BtnGroup-item', { type: 'button', 'aria-pressed': String(mode === value), class: mode === value ? 'selected' : '', on: { click: () => { mode = value; drawBar(); drawFiles(); } } }, text);
  const bar = h('div.d-flex.flex-items-center.mb-3');
  const drawBar = () => {
    clear(bar).append(h('span.color-fg-muted.flex-auto', {}, `Showing ${pull.changedFiles} changed file${pull.changedFiles === 1 ? '' : 's'} with ${Number(pull.additions).toLocaleString('en-US')} additions and ${Number(pull.deletions).toLocaleString('en-US')} deletions.`),
      h('div.BtnGroup.mr-2', {}, modeButton('unified', 'Unified'), modeButton('split', 'Split')),
      pull.state === 'merged' ? null : review.element);
  };
  async function loadMore() {
    const page = more;
    more = null;
    const result = await load('get_pull_request_files', { repo: route.repo, number: route.number, page, perPage: 100 });
    files = [...files, ...result.files];
    more = result.hasNext ? page + 1 : null;
    drawFiles();
  }
  drawBar();
  root.append(where.element, bar, list, h('div.color-fg-muted', {}, 'Loading files…'));
  Promise.all([load('get_pull_request_files', { repo: route.repo, number: route.number, page: 1, perPage: 100 }), load('list_comments', { repo: route.repo, number: route.number })]).then(([result, discussion]) => {
    files = result.files; comments = discussion.lineComments; more = result.hasNext ? 2 : null;
    root.lastChild.remove();
    drawFiles();
  }, error => { clear(root).append(h('div.flash.flash-error', {}, error.message)); });
  return root;
}
