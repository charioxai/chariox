import './local-storage.mjs';
import test from 'node:test';
import assert from 'node:assert/strict';
import { attachWorkbook } from '../view/actions.mjs';
import { emptyWorkbook } from '../view/workbook.mjs';

// An update reloads the view after the kernel switched generations: the old
// page cannot save through the App any more, so its unsaved workbook has to
// come back with the new page.
function page(saved, version) {
  let active = null, onCommand;
  const facade = data => ({
    getId: () => data.id, save: () => structuredClone(data),
    onCommandExecuted: handler => { onCommand = handler; return { dispose() {} }; }, getActiveRange: () => null,
    getActiveSheet: () => ({
      getSheetName: () => 'Sheet1', getMaxRows: () => 1000, getMaxColumns: () => 26,
      getRange: () => ({ getA1Notation: () => 'A1:H10', getValues: () => [[]], getFormulas: () => [[]] }),
    }),
  });
  const api = {
    getActiveWorkbook: () => active,
    createWorkbook(data) { active = facade(data); return active; },
    disposeUnit() { active = null; },
    getFormula: () => ({ onCalculationResultApplied: async () => {} }),
  };
  const updateHandlers = [];
  const bridge = {
    view: { state() {}, changed() {}, beforeUpdate: handler => updateHandlers.push(handler) },
    action: (_, handler) => input => handler(input, bridge),
    async call(name) {
      if (name === 'load_workbook') return { text: saved ? JSON.stringify(saved) : null, version, done: true };
      throw new Error(name);
    },
  };
  return { api, bridge, updateHandlers, edit: () => onCommand({ type: 2, id: 'edit' }), active: () => active };
}

test('unsaved edits reach the page the update loads', async () => {
  const before = page(null, null);
  const { summary } = await attachWorkbook(before.api, before.bridge);
  before.edit();
  assert.equal(summary().dirty, true);
  await Promise.all(before.updateHandlers.map(handler => handler()));
  const after = page(null, null);
  const reopened = await attachWorkbook(after.api, after.bridge);
  assert.equal(reopened.summary().dirty, true, 'the restored workbook still has unsaved edits');
  assert.deepEqual(after.active().save(), before.active().save());
});

test('a workbook with nothing unsaved leaves no draft', async () => {
  const before = page(null, null);
  await attachWorkbook(before.api, before.bridge);
  await Promise.all(before.updateHandlers.map(handler => handler()));
  const after = page(null, null);
  assert.equal((await attachWorkbook(after.api, after.bridge)).summary().dirty, false);
});

test('a draft is dropped when the App saved a newer workbook meanwhile', async () => {
  const before = page(null, null);
  await attachWorkbook(before.api, before.bridge);
  before.edit();
  await Promise.all(before.updateHandlers.map(handler => handler()));
  const newer = emptyWorkbook(); newer.name = 'Saved elsewhere';
  const after = page(newer, '2');
  const reopened = await attachWorkbook(after.api, after.bridge);
  assert.equal(reopened.summary().name, 'Saved elsewhere');
  assert.equal(reopened.summary().dirty, false);
});
