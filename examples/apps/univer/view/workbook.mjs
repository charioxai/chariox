import { MAX_ROWS, MAX_COLUMNS, parseCsv, csvCell } from './csv.mjs';

export const MAX_WORKBOOK_BYTES = 512 * 1024;

export function emptyWorkbook() {
  return {
    id: 'workbook', name: 'Workbook', appVersion: '1.2.0', locale: 'enUS',
    sheetOrder: ['sheet1'], styles: {},
    sheets: { sheet1: { id: 'sheet1', name: 'Sheet1', rowCount: 1000, columnCount: 26, cellData: {} } },
  };
}

export function validateWorkbook(value) {
  if (!value || typeof value !== 'object' || typeof value.id !== 'string' || !value.id || typeof value.name !== 'string' ||
      !Array.isArray(value.sheetOrder) || !value.sheetOrder.length || value.sheetOrder.length > 16 ||
      new Set(value.sheetOrder).size !== value.sheetOrder.length || !value.sheets ||
      Object.keys(value.sheets).length !== value.sheetOrder.length) throw new Error('Invalid Univer workbook');
  if (new TextEncoder().encode(JSON.stringify({ format: 'chariox-univer-workbook', version: 1, workbook: value })).length > MAX_WORKBOOK_BYTES) throw new Error('Workbook exceeds 512 KiB (524288 bytes), including file metadata; export or split the workbook');
  for (const id of value.sheetOrder) {
    const sheet = value.sheets[id];
    if (!sheet || sheet.id !== id || typeof sheet.name !== 'string' ||
        !Number.isInteger(sheet.rowCount) || sheet.rowCount < 1 || sheet.rowCount > MAX_ROWS ||
        !Number.isInteger(sheet.columnCount) || sheet.columnCount < 1 || sheet.columnCount > MAX_COLUMNS ||
        !sheet.cellData || typeof sheet.cellData !== 'object') throw new Error('Invalid worksheet dimensions');
    for (const [r, cells] of Object.entries(sheet.cellData)) {
      if (!/^(0|[1-9]\d*)$/.test(r) || Number(r) >= sheet.rowCount || !cells || typeof cells !== 'object') throw new Error('Invalid workbook row');
      for (const [c, cell] of Object.entries(cells)) {
        if (!/^(0|[1-9]\d*)$/.test(c) || Number(c) >= sheet.columnCount || (cell !== null && typeof cell !== 'object')) throw new Error('Invalid workbook cell');
      }
    }
  }
  return value;
}

export function workbookFromCsv(text) {
  const rows = parseCsv(text), workbook = emptyWorkbook(), sheet = workbook.sheets.sheet1;
  sheet.rowCount = Math.max(1000, rows.length);
  sheet.columnCount = Math.max(26, rows[0].length);
  sheet.cellData = Object.fromEntries(rows.map((row, r) => [r, Object.fromEntries(row.map((v, c) => [c, csvCell(v)]))]));
  return validateWorkbook(workbook);
}

export function workbookFile(workbook) {
  return JSON.stringify({ format: 'chariox-univer-workbook', version: 1, workbook: validateWorkbook(workbook) });
}

export function parseWorkbookFile(text) {
  if (new TextEncoder().encode(text).length > MAX_WORKBOOK_BYTES) throw new Error('Workbook file exceeds 512 KiB (524288 bytes), including file metadata');
  const file = JSON.parse(text);
  if (file.format !== 'chariox-univer-workbook' || file.version !== 1) throw new Error('Open a Chariox Univer JSON workbook');
  return validateWorkbook(file.workbook);
}

export function usedBounds(sheet) {
  let rows = 0, columns = 0;
  for (const [r, cells] of Object.entries(sheet.cellData ?? {})) for (const [c, cell] of Object.entries(cells)) {
    if (cell && (cell.v != null || cell.f || cell.p)) {
      rows = Math.max(rows, Number(r) + 1); columns = Math.max(columns, Number(c) + 1);
    }
  }
  return { rows, columns };
}
