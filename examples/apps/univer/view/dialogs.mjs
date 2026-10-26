import { createElement as h } from 'react';
import { Input } from '@univerjs/design';

// Use Univer's existing dialogs: native prompt/confirm are disabled by the
// App sandbox, and the App must not weaken that sandbox to collect input.
export function fileDialogs(confirm) {
  return {
    replace: () => confirm({
      id: 'chariox-replace', title: { title: 'Replace workbook?' },
      children: { title: 'Save or export your changes before replacing the workbook.' },
      confirmText: 'Replace', cancelText: 'Cancel', width: 440,
    }),
    async export(format) {
      let name = format === 'csv' ? 'Workbook.csv' : 'Workbook.univer.json', range = '';
      const fields = [h('label', { key: 'name' }, 'Export filename',
        h(Input, { 'aria-label': 'Export filename', defaultValue: name, onChange: value => { name = value; } }))];
      if (format === 'csv') fields.push(h('label', { key: 'range' }, 'CSV range (blank for used cells; at most 1,000 cells)',
        h(Input, { 'aria-label': 'CSV range', defaultValue: '', onChange: value => { range = value; } })));
      const accepted = await confirm({
        id: 'chariox-export', title: { title: format === 'csv' ? 'Export CSV' : 'Export workbook' },
        children: { title: h('div', null, ...fields) }, confirmText: 'Export', cancelText: 'Cancel', width: 480,
      });
      return accepted ? { name, offer: true, ...(range.trim() ? { range: range.trim().toUpperCase() } : {}) } : null;
    },
  };
}
