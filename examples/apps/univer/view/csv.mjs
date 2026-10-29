// RFC 4180 CSV, with bounded input and literal text cells (including '=...').
export const MAX_FILE_BYTES = 192 * 1024;
export const MAX_ROWS = 10000;
export const MAX_COLUMNS = 256;

export function parseCsv(text) {
  if (typeof text !== 'string' || new TextEncoder().encode(text).length > MAX_FILE_BYTES) {
    throw new Error(`CSV must be UTF-8 text of at most ${MAX_FILE_BYTES} bytes`);
  }
  text = text.replace(/^\uFEFF/, '');
  if (!text.length) return [['']];
  const rows = [];
  let row = [], field = '', quoted = false, closed = false;
  const cell = () => {
    row.push(field); field = ''; closed = false;
    if (row.length > MAX_COLUMNS) throw new Error(`CSV exceeds ${MAX_COLUMNS} columns`);
  };
  const line = () => {
    cell(); rows.push(row); row = [];
    if (rows.length > MAX_ROWS) throw new Error(`CSV exceeds ${MAX_ROWS} rows`);
  };
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"' && text[i + 1] === '"') { field += '"'; i++; }
      else if (c === '"') { quoted = false; closed = true; }
      else field += c;
    } else if (c === ',') cell();
    else if (c === '\r' || c === '\n') {
      line(); if (c === '\r' && text[i + 1] === '\n') i++;
    } else if (c === '"' && !field && !closed) quoted = true;
    else {
      if (closed || c === '"') throw new Error('CSV has an unexpected character beside a quote');
      field += c;
    }
  }
  if (quoted) throw new Error('CSV has an unterminated quoted field');
  if (field || row.length || closed || !/[\r\n]$/.test(text)) line();
  const width = Math.max(...rows.map(r => r.length));
  return rows.map(r => [...r, ...Array(width - r.length).fill('')]);
}

export function csvCell(text) {
  // Retain identifiers such as 00123, whitespace, dates and formula-looking text.
  const value = Number(text);
  if (/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(text) && Number.isFinite(value) &&
      (!Number.isInteger(value) || Number.isSafeInteger(value))) {
    return { v: value, t: 2 };
  }
  return { v: text, t: 1 };
}

export function encodeCsv(rows) {
  return rows.map(row => row.map(value => {
    const text = value == null ? '' : String(value);
    return /[",\r\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
  }).join(',')).join('\r\n') + '\r\n';
}
