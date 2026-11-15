// "Files changed": GitHub's diff, unified or split, with the review comments under their lines.
import { call, load } from './api.mjs';
import { emptyNote, parsePatch, splitRows } from './diff.mjs';
import { h, icon, clear } from './dom.mjs';
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

function commentBox(comment) {
  return h('div.Box.m-2.diff-comment', {}, h('div.Box-header.d-flex.flex-items-center.py-2', {}, avatar(comment.user, 20), h('span.ml-2', {}, userLink(comment.user), h('span.color-fg-muted', {}, ` commented ${ago(comment.createdAt)}`))), h('div.Box-body', {}, markdown(comment.body)));
}

function codeCell(row, side) {
  const cell = h('td.diff-code', { class: row ? `diff-${row.kind}` : 'diff-empty' });
  if (row) cell.append(h('span.diff-marker', {}, row.kind === 'add' ? '+' : row.kind === 'del' ? '−' : ' '), h('span.diff-text', {}, row.text));
  return cell;
}

function numCell(row, side) {
  return h('td.diff-num', { class: row ? `diff-${row.kind}` : 'diff-empty' }, row?.[side] ?? '');
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

export function diffTable(patch, comments, mode) {
  const rows = parsePatch(patch);
  const index = commentIndex(comments);
  const table = h('table.diff-table', { class: mode === 'split' ? 'diff-split' : 'diff-unified' });
  const span = mode === 'split' ? 4 : 3;
  const notes = (row, side) => {
    const found = [];
    if (row?.new !== null && row?.new !== undefined && (row.kind === 'add' || row.kind === 'context') && side !== 'left') found.push(...(index.get(`RIGHT:${row.new}`) ?? []));
    if (row?.old !== null && row?.old !== undefined && row.kind === 'del') found.push(...(index.get(`LEFT:${row.old}`) ?? []));
    return found;
  };
  const noteRow = found => found.length ? h('tr.diff-comments', {}, h('td', { colspan: span }, found.map(commentBox))) : null;
  if (mode === 'split') {
    for (const row of splitRows(rows)) {
      if (row.kind === 'hunk') table.append(h('tr.diff-hunk-row', {}, h('td.diff-num.diff-hunk', { colspan: 1 }, icon('fold', { size: 16 })), h('td.diff-code.diff-hunk', { colspan: 3 }, row.text)));
      else if (row.kind === 'note') table.append(h('tr', {}, h('td', { colspan: 4, class: 'diff-note' }, row.text)));
      else {
        table.append(h('tr', {}, numCell(row.left, 'old'), codeCell(row.left), numCell(row.right, 'new'), codeCell(row.right)));
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
        table.append(h('tr', {}, numCell(row, 'old'), numCell(row, 'new'), codeCell(row)));
        const extra = noteRow(notes(row));
        if (extra) table.append(extra);
      }
    }
  }
  return table;
}

function fileBox(ctx, route, file, comments, mode) {
  const body = h('div.diff-body');
  let collapsed = false;
  const toggle = h('button.btn-octicon', { type: 'button', 'aria-label': `Toggle ${file.path}`, 'aria-expanded': 'true' }, icon('chevron-down', { size: 16 }));
  const show = patch => clear(body).append(diffTable(patch, comments, mode));
  const load = h('button.btn.btn-sm', { type: 'button' }, 'Load diff');
  load.addEventListener('click', async () => {
    load.disabled = true;
    try {
      const result = await call('get_pull_request_diff', { repo: route.repo, number: route.number, path: file.path });
      if (result.tooLarge || (!result.diff && result.truncated === false && result.bytes === 0)) {
        clear(body).append(h('div.p-4.text-center.color-fg-muted', {}, 'This diff is too large to display.'));
      } else {
        const at = result.diff.indexOf('\n@@');
        show(at < 0 ? '' : result.diff.slice(at + 1));
      }
    } catch (error) { clear(body).append(h('div.p-4.text-center.color-fg-danger', {}, error.message)); }
  });
  if (file.patch) show(file.patch);
  else body.append(h('div.p-4.text-center.color-fg-muted', {}, emptyNote(file) ?? [h('p.mb-2', {}, 'Large diffs are not rendered by default.'), load]));
  toggle.addEventListener('click', () => { collapsed = !collapsed; body.hidden = collapsed; toggle.setAttribute('aria-expanded', String(!collapsed)); clear(toggle).append(icon(collapsed ? 'chevron-right' : 'chevron-down', { size: 16 })); });
  const name = file.previousPath ? `${file.previousPath} → ${file.path}` : file.path;
  return h('div.Box.mb-3.diff-file', { 'data-file': file.path },
    h('div.Box-header.d-flex.flex-items-center.py-2', {}, toggle,
      h('span.ml-2.text-mono.text-small.flex-auto.min-width-0.text-bold', {}, name),
      h('span.color-fg-muted.text-small', {}, `${file.changes} changes`), blocks(file)),
    body);
}

export function filesTab(ctx, route, pull) {
  let mode = 'unified';
  const root = h('div');
  const list = h('div');
  let files = [], comments = [], more = null;

  const drawFiles = () => {
    clear(list);
    for (const file of files) list.append(fileBox(ctx, route, file, comments.filter(comment => comment.path === file.path), mode));
    if (more) list.append(h('div.text-center.mb-3', {}, h('button.btn', { type: 'button', on: { click: () => loadMore() } }, 'Load more files')));
  };
  const modeButton = (value, text) => h('button.btn.btn-sm.BtnGroup-item', { type: 'button', 'aria-pressed': String(mode === value), class: mode === value ? 'selected' : '', on: { click: () => { mode = value; drawBar(); drawFiles(); } } }, text);
  const bar = h('div.d-flex.flex-items-center.mb-3');
  const drawBar = () => {
    clear(bar).append(h('span.color-fg-muted.flex-auto', {}, `Showing ${pull.changedFiles} changed file${pull.changedFiles === 1 ? '' : 's'} with ${Number(pull.additions).toLocaleString('en-US')} additions and ${Number(pull.deletions).toLocaleString('en-US')} deletions.`),
      h('div.BtnGroup', {}, modeButton('unified', 'Unified'), modeButton('split', 'Split')));
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
  root.append(bar, list, h('div.color-fg-muted', {}, 'Loading files…'));
  Promise.all([load('get_pull_request_files', { repo: route.repo, number: route.number, page: 1, perPage: 100 }), load('list_comments', { repo: route.repo, number: route.number })]).then(([result, discussion]) => {
    files = result.files; comments = discussion.lineComments; more = result.hasNext ? 2 : null;
    root.lastChild.remove();
    drawFiles();
  }, error => { clear(root).append(h('div.flash.flash-error', {}, error.message)); });
  return root;
}
