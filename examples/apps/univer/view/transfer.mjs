// Keep each App call below its existing byte/node budgets, independently of
// the native snapshot's size. Never split a Unicode scalar between calls.
const CHARS = 16000;
export function fileTransfer(bridge) {
  async function read(method, input) {
    let text = '', offset = 0, version;
    for (;;) {
      const result = await bridge.call(method, { ...input, offset });
      if (result.text === null) return { workbook: null, version: result.version };
      if (offset && result.version !== version) throw new Error('Saved workbook changed while loading; reopen the view');
      version = result.version;
      text += result.text;
      if (result.done) return { text, version };
      if (!Number.isSafeInteger(result.next) || result.next <= offset) throw new Error('File transfer made no progress; retry');
      offset = result.next;
      input = { ...input }; delete input.grantId;
    }
  }
  async function write(input) {
    let offset = 0, result;
    for (let start = 0; start < input.text.length || start === 0;) {
      let end = Math.min(start + CHARS, input.text.length);
      const last = input.text.charCodeAt(end - 1);
      if (end < input.text.length && last >= 0xd800 && last <= 0xdbff) end--;
      const text = input.text.slice(start, end);
      result = await bridge.call('write_file', { ...input, text, offset, final: end === input.text.length });
      offset += new TextEncoder().encode(text).length;
      if (end === input.text.length) return result;
      start = end;
    }
  }
  return { readFile: input => read('read_file', input), load: () => read('load_workbook', {}), write };
}
