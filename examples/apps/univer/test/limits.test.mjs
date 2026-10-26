import test from 'node:test';
import assert from 'node:assert/strict';
import { attachWorkbook } from '../view/actions.mjs';
import { emptyWorkbook, MAX_WORKBOOK_BYTES, workbookFromCsv, workbookFile } from '../view/workbook.mjs';

function atBytes(bytes) {
  const book = emptyWorkbook();
  book.sheets.sheet1.cellData = { 99: { 0: { v: '', t: 1 } } };
  const encoded = () => JSON.stringify({ format: 'chariox-univer-workbook', version: 1, workbook: book });
  book.sheets.sheet1.cellData[99][0].v = 'x'.repeat(bytes - Buffer.byteLength(encoded()));
  assert.equal(Buffer.byteLength(encoded()), bytes);
  return book;
}

function editor() {
  let active, source, saved = 0;
  const api = {
    getActiveWorkbook: () => active,
    createWorkbook(data) {
      active = {
        getId: () => data.id, save: () => structuredClone(data),
        onCommandExecuted: () => ({ dispose() {} }), getActiveRange: () => null,
        getActiveSheet: () => ({ getSheetName: () => 'Sheet1', getMaxRows: () => 1000, getMaxColumns: () => 26,
          getRange: () => ({ getA1Notation: () => 'A1:H10', getValues: () => [[]], getFormulas: () => [[]] }) }),
      };
      return active;
    },
    disposeUnit() { active = null; },
    getFormula: () => ({ onCalculationResultApplied: async () => {} }),
  };
  const bridge = {
    view: { state() {}, changed() {} }, action: (_, handler) => handler,
    async call(name) {
      if (name === 'load_workbook') return { text: null, version: null };
      if (name === 'read_file') return { text: source, done: true };
      if (name === 'write_file') return { bytes: 0 };
      if (name === 'save_workbook') { saved++; return { version: String(saved) }; }
      throw new Error(name);
    },
  };
  return { api, bridge, source: text => { source = text; }, saves: () => saved };
}

test('native import and save accept just under the snapshot cap and refuse just over before changing the editor or saving', async () => {
  const e = editor(), { actions, summary } = await attachWorkbook(e.api, e.bridge);
  const below = atBytes(MAX_WORKBOOK_BYTES - 1), above = atBytes(MAX_WORKBOOK_BYTES + 1);
  e.source(workbookFile(below));
  await actions.open_workbook({ name: 'boundary.univer.json' });
  await actions.save_workbook({});
  assert.equal(e.saves(), 1);
  e.source(JSON.stringify({ format: 'chariox-univer-workbook', version: 1, workbook: above }));
  await assert.rejects(actions.open_workbook({ name: 'boundary.univer.json' }), /512 KiB/);
  assert.equal(summary().version, '1');
  assert.equal(e.api.getActiveWorkbook().save().sheets.sheet1.cellData[99][0].v.length, below.sheets.sheet1.cellData[99][0].v.length);
  e.api.createWorkbook(above);
  await assert.rejects(actions.save_workbook({}), /512 KiB/);
  assert.equal(e.saves(), 1);
});

test('a 2000 by 10 numeric CSV fits the native snapshot cap', () => {
  const book = workbookFromCsv(('2,2,2,2,2,2,2,2,2,2\n').repeat(2000));
  assert.equal(book.sheets.sheet1.rowCount, 2000);
  assert.equal(Object.keys(book.sheets.sheet1.cellData).length, 2000);
  assert.equal(book.sheets.sheet1.cellData[1999][9].v, 2);
});
