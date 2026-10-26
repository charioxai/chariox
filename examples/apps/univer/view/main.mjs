import { createUniver, LocaleType, mergeLocales } from '@univerjs/presets';
import { UniverSheetsCorePreset } from '@univerjs/preset-sheets-core';
import coreEnUS from '@univerjs/preset-sheets-core/locales/en-US';
import { UniverSheetsSortPreset } from '@univerjs/preset-sheets-sort';
import sortEnUS from '@univerjs/preset-sheets-sort/locales/en-US';
import '@univerjs/preset-sheets-core/lib/index.css';
import '@univerjs/preset-sheets-sort/lib/index.css';
import { attachWorkbook } from './actions.mjs';

async function start() {
  if (!globalThis.chariox?.action || !globalThis.chariox?.call) throw new Error('Open Univer through its installed Chariox App view');
  const { univerAPI } = createUniver({
    locale: LocaleType.EN_US,
    locales: { [LocaleType.EN_US]: mergeLocales(coreEnUS, sortEnUS) },
    presets: [UniverSheetsCorePreset({ container: 'app' }), UniverSheetsSortPreset()],
  });
  const { actions, summary } = await attachWorkbook(univerAPI, chariox);
  const run = task => async () => {
    try { await task(); univerAPI.showMessage({ type: 'success', content: 'Done', duration: 2000 }); }
    catch (error) { univerAPI.showMessage({ type: 'error', content: error.message ?? String(error), duration: 10000 }); }
  };
  const open = async format => {
    if (summary().dirty && !confirm('Replace the workbook? Save or export your changes first.')) return;
    const { grants } = await actions.pick_file({ format });
    if (!grants.length) return;
    await actions[format === 'csv' ? 'import_csv' : 'open_workbook']({ name: format === 'csv' ? 'import.csv' : 'import.univer.json', grantId: grants[0] });
  };
  const exportFile = async format => {
    const name = prompt('Export filename', format === 'csv' ? 'Workbook.csv' : 'Workbook.univer.json');
    if (!name) return;
    const range = format === 'csv' ? prompt('CSV range (blank for used cells; at most 1,000 cells)', '') : '';
    if (range === null) return;
    await actions[format === 'csv' ? 'export_csv' : 'export_workbook']({ name, offer: true, ...(range ? { range: range.trim().toUpperCase() } : {}) });
  };
  const file = univerAPI.createSubmenu({ id: 'chariox.files', title: 'Files', order: -1 });
  file.appendTo('ribbon.start.history');
  for (const [id, title, task] of [
    ['save', 'Save workbook', () => actions.save_workbook({})],
    ['open_csv', 'Open CSV…', () => open('csv')],
    ['open_workbook', 'Open workbook…', () => open('workbook')],
    ['export_csv', 'Export CSV…', () => exportFile('csv')],
    ['export_workbook', 'Export workbook…', () => exportFile('workbook')],
  ]) {
    // Toolbar dropdowns consume direct menu items; grouped submenu children
    // are currently omitted by Univer's desktop toolbar renderer.
    univerAPI.createMenu({ id: `chariox.${id}`, title, action: run(task) }).appendTo('chariox.files');
  }
  window.addEventListener('beforeunload', event => {
    if (summary().dirty) { event.preventDefault(); event.returnValue = ''; }
  });
}

start().catch(error => {
  document.getElementById('app').textContent = `Univer could not open: ${error.message}. Reopen the App or restore an exported workbook.`;
  console.error('Univer startup failed:', error.message);
});
