import declarations from '../schemas/ui-actions.json';
import { MAX_ROWS, MAX_COLUMNS, encodeCsv } from './csv.mjs';
import { emptyWorkbook, validateWorkbook, workbookFromCsv, workbookFile, parseWorkbookFile, usedBounds } from './workbook.mjs';

export async function attachWorkbook(api, bridge) {
  let version = null, dirty = false, revision = 0, subscription;
  let latestRange = null;
  const book = () => {
    const current = api.getActiveWorkbook();
    if (!current) throw new Error('No open workbook');
    return current;
  };
  const sheet = name => {
    const result = name ? book().getSheetByName(name) : book().getActiveSheet();
    if (!result) throw new Error(`Worksheet not found: ${name}`);
    return result;
  };
  const changed = delta => bridge.view.changed(delta);
  function replace(data) {
    validateWorkbook(data);
    const previous = api.getActiveWorkbook();
    subscription?.dispose();
    if (previous) api.disposeUnit(previous.getId());
    const current = api.createWorkbook(data);
    latestRange = null;
    subscription = current.onCommandExecuted(command => {
      // Mutations include human edits, undo/redo, sort and formula results.
      if (command.type === 2) { dirty = true; revision++; changed({ workbookChanged: command.id }); }
    });
    dirty = true; revision++;
    changed({ opened: data.name });
  }
  const calculated = () => api.getFormula().onCalculationResultApplied(10000);

  function range(input) {
    if (!/^[A-Z]{1,2}[1-9]\d{0,4}(?::[A-Z]{1,2}[1-9]\d{0,4})?$/.test(input.range)) throw new Error('Use an A1 range such as A1:C10');
    const current = sheet(input.sheet), selected = current.getRange(input.range);
    const { startRow, endRow, startColumn, endColumn } = selected.getRange();
    if (startRow < 0 || startColumn < 0 || endRow < startRow || endColumn < startColumn ||
        endRow >= current.getMaxRows() || endColumn >= current.getMaxColumns() ||
        (endRow - startRow + 1) * (endColumn - startColumn + 1) > 1000) throw new Error('Choose at most 1000 cells within the worksheet');
    return selected;
  }
  const read = selected => ({ values: selected.getValues(), formulas: selected.getFormulas() });

  function summary() {
    const snapshot = book().save(), active = book().getActiveSheet();
    const clip = value => typeof value === 'string' ? value.slice(0, 256) : value;
    const preview = active.getRange(0, 0, Math.min(10, active.getMaxRows()), Math.min(8, active.getMaxColumns()));
    return {
      name: snapshot.name, version, dirty,
      sheets: snapshot.sheetOrder.map(id => ({ id, name: snapshot.sheets[id].name, ...usedBounds(snapshot.sheets[id]) })),
      activeSheet: active.getSheetName(), selection: book().getActiveRange()?.getA1Notation() ?? null,
      preview: { range: preview.getA1Notation(), values: preview.getValues().map(row => row.map(clip)), formulas: preview.getFormulas().map(row => row.map(clip)) },
      lastReadRange: latestRange,
    };
  }

  const handlers = {
    workbook_summary: summary,
    async read_range(input) {
      await calculated(); const selected = range(input);
      latestRange = { sheet: sheet(input.sheet).getSheetName(), range: input.range };
      return read(selected);
    },
    async set_range(input) {
      const selected = range(input), bounds = selected.getRange();
      if (!Array.isArray(input.values) || input.values.length !== bounds.endRow - bounds.startRow + 1 ||
          input.values.some(row => !Array.isArray(row) || row.length !== bounds.endColumn - bounds.startColumn + 1 ||
            row.some(v => v !== null && !['string', 'number', 'boolean'].includes(typeof v)))) throw new Error('Values must match the rectangular range');
      selected.setValues(input.values);
      await calculated(); return read(selected);
    },
    async edit_dimension(input) {
      const current = sheet(input.sheet), rows = input.axis === 'rows';
      if (!['rows', 'columns'].includes(input.axis) || !['insert', 'delete'].includes(input.operation) ||
          !Number.isInteger(input.index) || !Number.isInteger(input.count) || input.index < 0 || input.count < 1) throw new Error('Invalid dimension edit');
      const size = rows ? current.getMaxRows() : current.getMaxColumns();
      if (input.index >= size || (input.operation === 'delete' && (input.index + input.count > size || input.count === size)) ||
          (input.operation === 'insert' && size + input.count > (rows ? MAX_ROWS : MAX_COLUMNS))) throw new Error('Dimension edit exceeds worksheet bounds');
      const method = (input.operation === 'insert' ? 'insert' : 'delete') + (rows ? 'Rows' : 'Columns');
      current[method](input.index, input.count);
      await calculated(); return summary();
    },
    async sort_range(input) {
      const selected = range(input), bounds = selected.getRange();
      if (!Number.isInteger(input.column) || input.column < bounds.startColumn || input.column > bounds.endColumn || typeof input.ascending !== 'boolean') throw new Error('Sort column must be inside the range');
      selected.sort({ column: input.column, ascending: input.ascending });
      await calculated(); return read(selected);
    },
    async import_csv(input) {
      if (!input.name?.endsWith('.csv')) throw new Error('Choose a .csv filename');
      const openingRevision = revision;
      const { text } = await bridge.call('read_file', input);
      if (revision !== openingRevision) throw new Error('Workbook changed while opening the file; save your edits and try again');
      replace(workbookFromCsv(text)); await calculated(); return summary();
    },
    async export_csv(input) {
      if (!input.name?.endsWith('.csv')) throw new Error('Use a .csv filename');
      await calculated(); const current = sheet(input.sheet);
      const bounds = usedBounds(book().save().sheets[current.getSheetId()]);
      if (!input.range && bounds.rows * bounds.columns > 1000) throw new Error('Select an export range of at most 1000 cells');
      const values = input.range ? range(input).getValues()
        : bounds.rows && bounds.columns ? current.getRange(0, 0, bounds.rows, bounds.columns).getValues() : [['']];
      return bridge.call('write_file', { name: input.name, text: encodeCsv(values), offer: input.offer ?? false });
    },
    async save_workbook() {
      await calculated(); const data = validateWorkbook(book().save()), savingRevision = revision;
      const saved = await bridge.call('save_workbook', { workbook: data, version });
      version = saved.version; dirty = revision !== savingRevision;
      changed({ saved: version, dirty }); return { version, dirty };
    },
    async open_workbook(input) {
      if (!input.name?.endsWith('.univer.json')) throw new Error('Choose a .univer.json filename');
      const openingRevision = revision;
      const { text } = await bridge.call('read_file', input);
      if (revision !== openingRevision) throw new Error('Workbook changed while opening the file; save your edits and try again');
      replace(parseWorkbookFile(text)); await calculated(); return summary();
    },
    async export_workbook(input) {
      if (!input.name?.endsWith('.univer.json')) throw new Error('Use a .univer.json filename');
      await calculated(); return bridge.call('write_file', { name: input.name, text: workbookFile(book().save()), offer: input.offer ?? false });
    },
    pick_file: input => bridge.call('pick_file', input),
  };

  const loaded = await bridge.call('load_workbook', {});
  version = loaded.version;
  replace(loaded.workbook ?? emptyWorkbook());
  await calculated(); dirty = false;
  bridge.view.state(summary);
  // Human menu callbacks and agent calls share the same workbook operations.
  let pending = Promise.resolve();
  const actions = Object.fromEntries(declarations.uiActions.map(declaration => [
    declaration.name, bridge.action(declaration, input => {
      const result = pending.then(() => handlers[declaration.name](input));
      pending = result.catch(() => {});
      return result;
    }),
  ]));
  return { actions, summary };
}
