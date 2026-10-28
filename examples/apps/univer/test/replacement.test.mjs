import test from 'node:test';
import assert from 'node:assert/strict';
import { attachWorkbook } from '../view/actions.mjs';
import { emptyWorkbook, workbookFile } from '../view/workbook.mjs';

test('an editor failure while opening a validated file restores the previous workbook', async () => {
  let active = null;
  const original = emptyWorkbook(); original.name = 'Unsaved budget';
  const broken = emptyWorkbook(); broken.name = 'Broken engine';
  const facade = data => ({
    getId: () => data.id, save: () => structuredClone(data),
    onCommandExecuted: () => ({ dispose() {} }), getActiveRange: () => null,
    getActiveSheet: () => ({
      getSheetName: () => 'Sheet1', getMaxRows: () => 1000, getMaxColumns: () => 26,
      getRange: () => ({ getA1Notation: () => 'A1:H10', getValues: () => [[]], getFormulas: () => [[]] }),
    }),
  });
  const api = {
    getActiveWorkbook: () => active,
    createWorkbook(data) {
      if (data.name === 'Broken engine') throw new Error('renderer rejected the workbook');
      active = facade(data); return active;
    },
    disposeUnit() { active = null; },
    getFormula: () => ({ onCalculationResultApplied: async () => {} }),
  };
  const bridge = {
    view: { state() {}, changed() {} }, action: (_, handler) => input => handler(input, bridge),
    async call(name) {
      if (name === 'load_workbook') return { text: JSON.stringify(original), version: '7', done: true };
      if (name === 'read_file') return { text: workbookFile(broken), done: true };
      throw new Error(name);
    },
  };
  const { actions, summary } = await attachWorkbook(api, bridge);
  await assert.rejects(actions.open_workbook({ name: 'Broken.univer.json' }), /previous workbook restored/);
  assert.equal(summary().name, 'Unsaved budget');
  assert.equal(summary().version, '7');
  assert.equal(summary().dirty, false);
  assert.deepEqual(active.save(), original);
});
