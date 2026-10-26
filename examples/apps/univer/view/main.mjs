import { createUniver, LocaleType, mergeLocales } from '@univerjs/presets';
import { CommandType, ICommandService, IConfirmService } from '@univerjs/core';
import { UniverSheetsCorePreset } from '@univerjs/preset-sheets-core';
import coreEnUS from '@univerjs/preset-sheets-core/locales/en-US';
import { UniverSheetsSortPreset } from '@univerjs/preset-sheets-sort';
import sortEnUS from '@univerjs/preset-sheets-sort/locales/en-US';
import '@univerjs/preset-sheets-core/lib/index.css';
import '@univerjs/preset-sheets-sort/lib/index.css';
import { attachWorkbook } from './actions.mjs';
import { fileDialogs } from './dialogs.mjs';
import { INotificationService } from '@univerjs/ui';

async function start() {
  if (!globalThis.chariox?.action || !globalThis.chariox?.call) throw new Error('Open Univer through its installed Chariox App view');
  const { univer, univerAPI } = createUniver({
    locale: LocaleType.EN_US,
    locales: { [LocaleType.EN_US]: mergeLocales(coreEnUS, sortEnUS) },
    presets: [UniverSheetsCorePreset({ container: 'app' }), UniverSheetsSortPreset()],
  });
  const { actions, summary } = await attachWorkbook(univerAPI, chariox);
  const dialogs = fileDialogs(options => univer.__getInjector().get(IConfirmService).confirm(options));
  const notifications = univer.__getInjector().get(INotificationService);
  const run = task => async () => {
    try { if (await task() === false) return; notifications.show({ type: 'success', title: 'Done', duration: 2000, position: 'bottom-right' }); }
    catch (error) { notifications.show({ type: 'error', title: error.message ?? String(error), duration: 10000, position: 'bottom-right' }); }
  };
  const open = async format => {
    if (summary().dirty && !await dialogs.replace()) return false;
    const { grants } = await actions.pick_file({ format });
    if (!grants.length) return false;
    await actions[format === 'csv' ? 'import_csv' : 'open_workbook']({ name: format === 'csv' ? 'import.csv' : 'import.univer.json', grantId: grants[0] });
  };
  const exportFile = async format => {
    const input = await dialogs.export(format);
    if (!input) return false;
    await actions[format === 'csv' ? 'export_csv' : 'export_workbook'](input);
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
    // The toolbar dispatches the item's id, ignoring the facade's generated
    // commandId. Register the same explicit id on both sides.
    const command = `chariox.${id}`;
    univer.__getInjector().get(ICommandService).registerCommand({ id: command, type: CommandType.COMMAND, handler: run(task) });
    univerAPI.createMenu({ id: command, title, action: command }).appendTo('chariox.files');
  }

}

start().catch(error => {
  document.getElementById('app').textContent = `Univer could not open: ${error.message}. Reopen the App or restore an exported workbook.`;
  console.error('Univer startup failed:', error.message);
});
