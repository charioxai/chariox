// A unified patch as the rows a diff table shows.
const HUNK = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@ ?(.*)$/;

/** Rows: { kind: 'hunk', text } | { kind: 'context'|'add'|'del', old, new, text } | { kind: 'note', text }. */
export function parsePatch(patch) {
  const rows = [];
  let oldLine = 0, newLine = 0;
  for (const line of String(patch ?? '').split('\n')) {
    const hunk = HUNK.exec(line);
    if (hunk) { oldLine = Number(hunk[1]); newLine = Number(hunk[3]); rows.push({ kind: 'hunk', text: line }); continue; }
    if (line.startsWith('\\')) { rows.push({ kind: 'note', text: line.slice(2) }); continue; }
    const sign = line[0];
    if (sign === '+') rows.push({ kind: 'add', old: null, new: newLine++, text: line.slice(1) });
    else if (sign === '-') rows.push({ kind: 'del', old: oldLine++, new: null, text: line.slice(1) });
    else if (sign === ' ') rows.push({ kind: 'context', old: oldLine++, new: newLine++, text: line.slice(1) });
  }
  return rows;
}

/** Side by side: a run of deletions meets the additions that follow it, line by line. */
export function splitRows(rows) {
  const out = [];
  for (let i = 0; i < rows.length;) {
    const row = rows[i];
    if (row.kind !== 'del') {
      out.push(row.kind === 'add' ? { kind: 'pair', left: null, right: row } : row.kind === 'context' ? { kind: 'pair', left: row, right: row } : row);
      i++;
      continue;
    }
    const deleted = [], added = [];
    while (rows[i]?.kind === 'del') deleted.push(rows[i++]);
    while (rows[i]?.kind === 'add') added.push(rows[i++]);
    for (let k = 0; k < Math.max(deleted.length, added.length); k++) out.push({ kind: 'pair', left: deleted[k] ?? null, right: added[k] ?? null });
  }
  return out;
}

/** GitHub's file status line: "Binary file not shown", "File renamed without changes" ... */
export function emptyNote(file) {
  if (file.status === 'removed' && !file.patch) return 'This file was deleted.';
  if (file.status === 'renamed' && file.changes === 0) return 'File renamed without changes.';
  if (file.changes === 0) return 'Binary file not shown.';
  return null;
}
