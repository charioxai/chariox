import test from 'node:test';
import assert from 'node:assert/strict';
import { parseCsv, encodeCsv, csvCell, MAX_FILE_BYTES } from '../view/csv.mjs';
import { emptyWorkbook, workbookFromCsv, workbookFile, parseWorkbookFile, validateWorkbook, usedBounds } from '../view/workbook.mjs';

test('CSV preserves quoted commas, escaped quotes, multiline Unicode and empty final fields', () => {
  const rows = [['Name', 'Note', ''], ['Zoë', 'one, two', 'a "quote"'], ['李', 'line\r\nbreak', '']];
  assert.deepEqual(parseCsv('\uFEFF' + encodeCsv(rows)), rows);
  assert.deepEqual(parseCsv('a,b\n1,2\n'), [['a', 'b'], ['1', '2']]);
  assert.deepEqual(parseCsv('a,b\n1'), [['a', 'b'], ['1', '']]);
  assert.deepEqual(parseCsv('""'), [['']]);
  assert.deepEqual(parseCsv('a,'), [['a', '']]);
  assert.deepEqual(parseCsv('a\r\n\r\n'), [['a'], ['']]);
});

test('CSV rejects malformed quotes and bounded input before changing a workbook', () => {
  for (const text of ['"open', 'a"b', '"x"z', '"x" "y"', 'a,'.repeat(257)]) assert.throws(() => parseCsv(text));
  assert.throws(() => parseCsv('x'.repeat(MAX_FILE_BYTES + 1)), /at most/);
});

test('CSV numerics calculate; identifiers and formula-looking text remain literal', () => {
  for (const text of ['12', '-0.5', '1e3']) assert.equal(csvCell(text).t, 2);
  for (const text of ['0012', '=SUM(A1)', '+1', ' 2', '2026-10-26', '12345678901234567890']) assert.deepEqual(csvCell(text), { v: text, t: 1 });
  const data = workbookFromCsv('Item,Qty\nTea,2\nCoffee,3');
  assert.deepEqual(usedBounds(data.sheets.sheet1), { rows: 3, columns: 2 });
  assert.equal(data.sheets.sheet1.cellData[1][1].v, 2);
});

test('JSON workbook round trip retains formulas, calculated values, styles and dimensions', () => {
  const data = emptyWorkbook();
  data.sheets.sheet1.cellData = { 0: { 0: { f: '=SUM(B1:C1)', v: 5, s: 'bold' }, 1: { v: 2 }, 2: { v: 3 } } };
  data.styles.bold = { bl: 1 };
  assert.deepEqual(parseWorkbookFile(workbookFile(data)), data);
  assert.throws(() => parseWorkbookFile('{"format":"xlsx","version":1}'), /Chariox Univer/);
  const broken = structuredClone(data); broken.sheets.sheet1.cellData[-1] = {};
  assert.throws(() => validateWorkbook(broken), /row/);
  broken.sheetOrder.push('missing');
  assert.throws(() => validateWorkbook(broken));
});
